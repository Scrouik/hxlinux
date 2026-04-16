// ===========================================================
// lib.rs
// Point d'entrée — assemble tous les composants
// Le UsbMonitor surveille le branchement du HX et déclenche
// automatiquement la séquence de connexion
// ===========================================================

mod helix;
mod stomp_layout;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::thread;
use tauri::{
    LogicalPosition, LogicalSize, Manager, PhysicalPosition, Position, Size, WindowEvent,
};
use serde::{Deserialize, Serialize};

use helix::HelixState;
use helix::ModeRequest;
use helix::KeepAliveCommand;
use helix::packet::OutPacket;
use helix::keep_alive::KeepAliveManager;
use helix::modes::connect::Connect;
use helix::modes::request_preset_name::RequestPresetName;
use helix::modes::request_preset_names::RequestPresetNames;
use helix::modes::standard::Standard;
use helix::modes::reconfigure_x1::ReconfigureX1;
use helix::modes::request_preset::RequestPreset;
use std::time::Duration;
use lazy_static::lazy_static;

const SUPPORTED_DEVICES: &[(&str, u16, u16)] = &[
    ("HX Stomp XL", 0x0e41, 0x4253),
    ("HX Stomp", 0x0e41, 0x4246),
    ("Helix Floor", 0x0e41, 0x4248),
    ("Helix LT", 0x0e41, 0x424a),
];
const EXPECTED_PRESET_COUNT: usize = 125;
const WINDOW_LAYOUT_FILE: &str = "window-layout.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedWindowGeometry {
    x: i32,
    y: i32,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SavedWindowLayout {
    main: Option<SavedWindowGeometry>,
    models: Option<SavedWindowGeometry>,
}

fn window_layout_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let mut dir = app.path().app_config_dir().ok()?;
    let _ = fs::create_dir_all(&dir);
    dir.push(WINDOW_LAYOUT_FILE);
    Some(dir)
}

fn read_window_layout(app: &tauri::AppHandle) -> Option<SavedWindowLayout> {
    let path = window_layout_path(app)?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<SavedWindowLayout>(&text).ok()
}

fn capture_window_geometry(window: &tauri::WebviewWindow) -> Option<SavedWindowGeometry> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(SavedWindowGeometry {
        x: pos.x,
        y: pos.y,
        width: size.width as f64,
        height: size.height as f64,
    })
}

fn apply_window_geometry(window: &tauri::WebviewWindow, geometry: &SavedWindowGeometry) {
    let _ = window.set_size(Size::Logical(LogicalSize::new(
        geometry.width,
        geometry.height,
    )));
    let _ = window.set_position(Position::Physical(PhysicalPosition::new(
        geometry.x,
        geometry.y,
    )));
}

fn save_window_layout(app: &tauri::AppHandle) {
    let layout = SavedWindowLayout {
        main: app
            .get_webview_window("main")
            .and_then(|w| capture_window_geometry(&w)),
        models: app
            .get_webview_window("models")
            .and_then(|w| capture_window_geometry(&w)),
    };

    if let Some(path) = window_layout_path(app) {
        if let Ok(text) = serde_json::to_string_pretty(&layout) {
            let _ = fs::write(path, text);
        }
    }
}

// ===========================================================
// État partagé entre Rust et Tauri
// ===========================================================
#[derive(Default)]
struct AppState {
    preset_names:  Vec<String>,
    active_preset: usize,
    connected_device_name: Option<String>,
    connection_issue_hint: Option<String>,
    // Accès au HelixState pour envoyer des commandes USB depuis Tauri
    helix_state:   Option<Arc<Mutex<HelixState>>>,
    // Handle USB pour les commandes directes (ex: MIDI Program Change sur 0x02)
    usb_handle:    Option<Arc<rusb::DeviceHandle<rusb::GlobalContext>>>,
}

enum UsbDiscovery {
    NoHxVisible,
    SupportedVisible(&'static str),
    UnsupportedHxVisible { vid: u16, pid: u16 },
}

fn discover_hx_usb() -> UsbDiscovery {
    let devices = match rusb::devices() {
        Ok(d) => d,
        Err(_) => return UsbDiscovery::NoHxVisible,
    };

    let mut saw_line6 = false;
    let mut first_line6_pid: Option<(u16, u16)> = None;

    for device in devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if desc.vendor_id() != 0x0e41 {
            continue;
        }
        saw_line6 = true;
        if first_line6_pid.is_none() {
            first_line6_pid = Some((desc.vendor_id(), desc.product_id()));
        }
        if let Some((name, _, _)) = SUPPORTED_DEVICES
            .iter()
            .find(|(_, vid, pid)| desc.vendor_id() == *vid && desc.product_id() == *pid)
        {
            return UsbDiscovery::SupportedVisible(*name);
        }
    }

    if saw_line6 {
        if let Some((vid, pid)) = first_line6_pid {
            UsbDiscovery::UnsupportedHxVisible { vid, pid }
        } else {
            UsbDiscovery::UnsupportedHxVisible {
                vid: 0x0e41,
                pid: 0x0000,
            }
        }
    } else {
        UsbDiscovery::NoHxVisible
    }
}

// ===========================================================
// Commandes Tauri
// ===========================================================

#[tauri::command]
fn get_preset_names(state: tauri::State<Arc<Mutex<AppState>>>) -> Vec<String> {
    // IMPORTANT pour le debug:
    // On ne fait aucun correctif "UX" ici: on renvoie la liste brute telle que
    // reconstruite par `RequestPresetNames`.
    // Les corrections seront réintroduites seulement une fois que le décodage
    // index->nom est stabilisé.
    state.lock().unwrap().preset_names.clone()
}

#[tauri::command]
fn get_active_preset(state: tauri::State<Arc<Mutex<AppState>>>) -> usize {
    state.lock().unwrap().active_preset
}

#[tauri::command]
fn get_connected_device_name(state: tauri::State<Arc<Mutex<AppState>>>) -> Option<String> {
    state.lock().unwrap().connected_device_name.clone()
}

#[tauri::command]
fn get_connection_hint_text(state: tauri::State<Arc<Mutex<AppState>>>) -> String {
    let (connected_name, issue_hint) = {
        let app = state.lock().unwrap();
        (app.connected_device_name.clone(), app.connection_issue_hint.clone())
    };
    if let Some(name) = connected_name {
        return format!("{name} connected");
    }
    match discover_hx_usb() {
        UsbDiscovery::NoHxVisible => "No HX detected (unplugged or powered off)".to_string(),
        UsbDiscovery::SupportedVisible(name) => issue_hint
            .unwrap_or_else(|| format!("{name} detected (initializing...)")),
        UsbDiscovery::UnsupportedHxVisible { vid, pid } => {
            format!("Unknown HX USB ID {vid:04x}:{pid:04x} (unsupported)")
        }
    }
}

/// Déclenche une lecture du nom du preset actif (via `RequestPresetName`).
/// Utile pour corriger les UI quand `get_preset_names()` est temporairement désaligné.
#[tauri::command]
fn request_active_preset_name(
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone().ok_or("HX non connecté")?
    };

    let s = helix_arc.lock().unwrap();
    s.switch_mode(ModeRequest::RequestPresetName);
    Ok(())
}

/// Renomme un preset sur le HX.
/// Traduction de set_preset_label_be_careful() de kempline.
#[tauri::command]
fn rename_preset(
    index: usize,
    name: String,
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    // Récupérer le HelixState
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;

    // Limiter à 16 caractères ASCII — limite produit utilisée par l'application.
    let text: Vec<u8> = name
        .chars()
        .filter(|c| c.is_ascii())
        .take(16)
        .map(|c| c as u8)
        .collect();

    let effective_name = String::from_utf8(text.clone()).unwrap_or_default();

    // Kempline : set_preset_label_be_careful(prog_no, text)
    let msg_size_byte      = 0x20u8 + text.len() as u8;
    let length_byte        = 0xa1u8 + text.len() as u8;
    let second_length_byte = msg_size_byte - 0x10;

    let mut s = helix_arc.lock().unwrap();
    let cnt = s.next_x1_cnt(); // "XX" → next_x1x10_packet_no()

    let mut data: Vec<u8> = vec![
        msg_size_byte, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt,  0x00, 0x04,
        0x77, 0x1e, 0x00, 0x00,
        0x01, 0x00, 0x02, 0x00,
        second_length_byte, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        0xed, 0x64, 0x06, 0x65,
        0x83, 0x6b, 0x00,
        0x6c, index as u8,
        0x6d, length_byte,
    ];

    // Ajouter les caractères du nom
    data.extend_from_slice(&text);

    // Padding — kempline : while len(data) < msg_size_byte + 9 + 2
    while data.len() < (msg_size_byte as usize) + 11 {
        data.push(0x00);
    }

    s.send(OutPacket::new(data));
    drop(s);

    // Mettre à jour AppState pour que le polling retourne le nouveau nom
    let mut app = state.lock().unwrap();
    if let Some(entry) = app.preset_names.get_mut(index) {
        *entry = effective_name;
    }

    Ok(())
}

/// Active un preset sur le HX via MIDI Program Change (endpoint 0x02).
/// Kempline : send_midi_program_change(program_no)
#[tauri::command]
fn activate_preset(
    index: usize,
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    if index > 125 {
        return Err(format!("Index hors limites : {}", index));
    }

    let handle = {
        let app = state.lock().unwrap();
        app.usb_handle.clone().ok_or("HX non connecté")?
    };

    // USB-MIDI Event Packet : CIN=0xC (Program Change), canal 0, program_no, padding
    let packet = [0x0Cu8, 0xC0, index as u8, 0x00];
    handle
        .write_bulk(0x02, &packet, Duration::from_millis(1000))
        .map_err(|e| format!("Erreur MIDI Program Change : {}", e))?;

    // Mettre à jour l'état local
    state.lock().unwrap().active_preset = index;

    Ok(())
}

/// Retourne les données brutes du dernier preset lu en hex (debug).
#[tauri::command]
fn get_preset_data_hex(state: tauri::State<Arc<Mutex<AppState>>>) -> Option<String> {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()?
    };
    let s = helix_arc.lock().unwrap();
    if !s.preset_data_ready {
        return None;
    }
    let mut out = String::with_capacity(s.preset_data.len() * 2);
    for b in &s.preset_data {
        let _ = write!(&mut out, "{:02x}", b);
    }
    Some(out)
}

/// Déclenche la lecture du contenu du preset actif sur le HX.
#[tauri::command]
fn request_preset_content(state: tauri::State<Arc<Mutex<AppState>>>) -> Result<(), String> {
    let (active_preset, helix_arc) = {
        let app = state.lock().unwrap();
        (
            app.active_preset,
            app.helix_state.clone().ok_or("HX non connecté")?,
        )
    };
    let mut s = helix_arc.lock().unwrap();
    if s.preset_content_only && !s.preset_data_ready {
        eprintln!("[PresetDebug][request_preset_content] already in progress, skip");
        return Ok(());
    }
    // L'UI met à jour `active_preset` (ex. après `activate_preset` + MIDI PC) avant cette
    // commande, alors que `preset_index` côté Helix ne bouge qu'avec les paquets USB x2 ou
    // l'écoute MIDI — parfois après le dump, ou jamais si `RequestPresetName` est ignoré
    // pendant `preset_content_only`. Sans cette ligne, `get_active_preset_slots` reste à
    // None et la fenêtre models timeoute.
    if s.preset_index != active_preset {
        eprintln!(
            "[PresetDebug][request_preset_content] sync helix preset_index {} -> app active_preset {}",
            s.preset_index,
            active_preset
        );
    }
    s.preset_index = active_preset;
    s.preset_data_ready = false;
    s.preset_data.clear();
    s.preset_content_only = true;
    eprintln!(
        "[PresetDebug][request_preset_content] trigger preset_index={} mode=RequestPreset",
        s.preset_index
    );
    s.switch_mode(ModeRequest::RequestPreset);
    Ok(())
}

/// Retourne les slots du dernier preset lu, sous forme [catégorie, nom].
#[tauri::command]
fn get_preset_slots(state: tauri::State<Arc<Mutex<AppState>>>) -> Option<Vec<[String; 2]>> {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()?
    };
    let s = helix_arc.lock().unwrap();
    if !s.preset_data_ready {
        eprintln!("[PresetDebug][get_preset_slots] not ready yet");
        return None;
    }
    if s.preset_data.is_empty() {
        eprintln!("[PresetDebug][get_preset_slots] ready flag true but payload empty -> ignore");
        return None;
    }
    let slots = parse_preset_slots(&s.preset_data);
    eprintln!(
        "[PresetDebug][get_preset_slots] ready bytes={} slots={}",
        s.preset_data.len(),
        slots.len()
    );
    Some(slots)
}

/// Retourne les slots uniquement si les données correspondent au preset actif.
#[tauri::command]
fn get_active_preset_slots(state: tauri::State<Arc<Mutex<AppState>>>) -> Option<Vec<[String; 2]>> {
    let (active_preset, helix_arc) = {
        let app = state.lock().unwrap();
        (app.active_preset, app.helix_state.clone()?)
    };

    let s = helix_arc.lock().unwrap();
    if !s.preset_data_ready || s.preset_data.is_empty() {
        return None;
    }
    if s.preset_index != active_preset {
        eprintln!(
            "[PresetDebug][get_active_preset_slots] waiting: state.preset_index={} app.active_preset={}",
            s.preset_index,
            active_preset
        );
        return None;
    }
    Some(parse_preset_slots(&s.preset_data))
}

/// Version debug : retourne aussi les coordonnées brutes de grille [x, y].
#[tauri::command]
fn get_active_preset_slots_debug(state: tauri::State<Arc<Mutex<AppState>>>) -> Option<Vec<[String; 4]>> {
    let (active_preset, helix_arc) = {
        let app = state.lock().unwrap();
        (app.active_preset, app.helix_state.clone()?)
    };

    let s = helix_arc.lock().unwrap();
    if !s.preset_data_ready || s.preset_data.is_empty() {
        return None;
    }
    if s.preset_index != active_preset {
        return None;
    }
    Some(parse_preset_slots_debug(&s.preset_data))
}

/// Marqueurs de routing synthétiques (Split / Merge) issus du parseur « flux »,
/// pour compléter l'affichage grille 16 cases où le split n'occupe pas un slot assignable.
#[tauri::command]
fn get_active_preset_routing_markers(
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Option<Vec<[String; 2]>> {
    let (active_preset, helix_arc) = {
        let app = state.lock().unwrap();
        (app.active_preset, app.helix_state.clone()?)
    };

    let s = helix_arc.lock().unwrap();
    if !s.preset_data_ready || s.preset_data.is_empty() {
        return None;
    }
    if s.preset_index != active_preset {
        return None;
    }
    let markers: Vec<[String; 2]> = parse_preset_slots_internal(&s.preset_data)
        .into_iter()
        .filter(|p| p.category == "Routing")
        .map(|p| [p.category, p.name])
        .collect();
    Some(markers)
}

/// Grille Kempline 16 cases + chaîne 10 entrées (8 colonnes + Split + Merge) et colonnes Split/Merge.
#[tauri::command]
fn get_active_preset_stomp_layout(
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Option<stomp_layout::ActivePresetStompLayout> {
    let (active_preset, helix_arc) = {
        let app = state.lock().unwrap();
        (app.active_preset, app.helix_state.clone()?)
    };

    let s = helix_arc.lock().unwrap();
    if !s.preset_data_ready || s.preset_data.is_empty() {
        return None;
    }
    if s.preset_index != active_preset {
        return None;
    }
    let grid = try_parse_preset_kempline_grid(&s.preset_data)?;
    let cells: Vec<stomp_layout::KemplineCell> = grid
        .into_iter()
        .map(|p| stomp_layout::KemplineCell {
            category: p.category,
            name: p.name,
            grid_x: p.grid_x,
            grid_y: p.grid_y,
        })
        .collect();
    Some(stomp_layout::compute_stomp_layout_from_kempline_grid_with_usb(
        &cells,
        &s.preset_data,
    ))
}

lazy_static! {
    static ref MODULES_BY_ID: HashMap<String, [String; 2]> = {
        let mut map = HashMap::new();
        let modules_py = include_str!("../../Kempline/modules.py");
        for line in modules_py.lines() {
            let parts: Vec<&str> = line.split('\'').collect();
            if parts.len() >= 6 {
                let key = parts[1].trim().to_lowercase();
                let category = parts[3].trim().to_string();
                let name = parts[5].trim().to_string();
                if !key.is_empty() && !category.is_empty() && !name.is_empty() {
                    map.insert(key, [category, name]);
                }
            }
        }
        map
    };
    static ref MODULE_FALLBACKS: HashMap<String, [String; 2]> = {
        let mut map = HashMap::new();
        // Missing from current local modules.py snapshot but observed on device.
        map.insert(
            "cd02a7".to_string(),
            [
                String::from("Distortion"),
                String::from("Pillars"),
            ],
        );
        map
    };
}

#[derive(Clone)]
struct ParsedSlot {
    category: String,
    name: String,
    grid_x: Option<u8>,
    grid_y: Option<u8>,
}

/// Découpe le flux preset aux marqueurs [0x82, 0x13] (équivalent Kempline `split('8213')` sur l'hex).
fn split_preset_by_8213(data: &[u8]) -> Vec<&[u8]> {
    let mut chunks: Vec<&[u8]> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + 1 < data.len() {
        if data[i] == 0x82 && data[i + 1] == 0x13 {
            if i > start {
                chunks.push(&data[start..i]);
            }
            start = i + 2;
            i += 2;
        } else {
            i += 1;
        }
    }
    if start < data.len() {
        chunks.push(&data[start..]);
    }
    chunks
}

/// Parse les slots d'un preset brut.
/// Kempline : split par [0x82, 0x13], extrait l'ID entre markers 0x19 et 0x1a.
fn parse_preset_slots_internal(data: &[u8]) -> Vec<ParsedSlot> {
    fn extract_grid_x(chunk: &[u8], scan_from: usize) -> Option<u8> {
        let end = chunk.len().min(scan_from.saturating_add(40));
        let mut i = scan_from;
        while i + 1 < end {
            if chunk[i] == 0x02 {
                return Some(chunk[i + 1]);
            }
            i += 1;
        }
        None
    }
    fn extract_grid_y(chunk: &[u8], scan_from: usize) -> Option<u8> {
        let end = chunk.len().min(scan_from.saturating_add(40));
        let mut i = scan_from;
        while i + 1 < end {
            if chunk[i] == 0x03 {
                return Some(chunk[i + 1]);
            }
            i += 1;
        }
        None
    }

    let chunks = split_preset_by_8213(data);

    let mut parsed_slots: Vec<ParsedSlot> = Vec::new();
    for chunk in chunks {
        let mut cursor = 0usize;
        while cursor < chunk.len() {
            if chunk[cursor] == 0x19 {
                let id_start = cursor + 1;
                if let Some(rel_end) = chunk[id_start..].iter().position(|&b| b == 0x1a) {
                    let id_bytes = &chunk[id_start..id_start + rel_end];
                    if !id_bytes.is_empty() {
                        let mut id_hex = String::with_capacity(id_bytes.len() * 2);
                        for b in id_bytes {
                            let _ = write!(&mut id_hex, "{:02x}", b);
                        }
                        let (category, name) = if let Some(entry) = MODULES_BY_ID
                            .get(&id_hex)
                            .or_else(|| MODULE_FALLBACKS.get(&id_hex))
                        {
                            (entry[0].clone(), entry[1].clone())
                        } else {
                            (String::from("Unknown"), id_hex)
                        };
                        parsed_slots.push(ParsedSlot {
                            category,
                            name,
                            grid_x: extract_grid_x(chunk, id_start + rel_end + 1),
                            grid_y: extract_grid_y(chunk, id_start + rel_end + 1),
                        });
                    }
                    cursor = id_start + rel_end + 1;
                    continue;
                }
            }
            cursor += 1;
        }
    }

    // Dédupliquer les doublons consécutifs (certains dumps peuvent contenir
    // une répétition du flux de blocs).
    let mut deduped: Vec<ParsedSlot> = Vec::with_capacity(parsed_slots.len());
    for slot in parsed_slots {
        let is_dup = deduped.last().map_or(false, |prev| {
            prev.category == slot.category && prev.name == slot.name
        });
        if !is_dup {
            deduped.push(slot);
        }
    }

    // Heuristique de routing:
    // si un bloc d'ampli est suivi plus tard d'un bloc avec une position
    // de grille plus basse, on injecte un marqueur de split avant l'ampli.
    let mut split_before_idx: Option<usize> = None;
    for (i, slot) in deduped.iter().enumerate() {
        let is_amp = slot.category == "Amp" || slot.category == "Preamp" || slot.category == "Amp+Cab";
        let amp_pos = slot.grid_x;
        if !is_amp || amp_pos.is_none() {
            continue;
        }
        let amp_pos = amp_pos.unwrap();
        if deduped
            .iter()
            .skip(i + 1)
            .any(|s| s.grid_x.map_or(false, |p| p < amp_pos))
        {
            split_before_idx = Some(i);
            break;
        }
    }

    // Merge/Mixer : le retour sur grid_x (premier slot avec x >= ancre après
    // une colonne plus basse) arrive trop tôt si l'ordre sérialisé mélange la
    // chaîne commune et la branche B (effets post-merge avant l'ampli B, etc.).
    // Helix liste souvent : ampli A, chaîne après merge, puis ampli B et la suite
    // sur la branche basse — le point de jonction visuel est donc juste après
    // le premier ampli (même indice que le split), pas au premier « rebond » de x.
    let merge_before_idx: Option<usize> = split_before_idx.map(|s| {
        let m = s.saturating_add(1);
        if m >= deduped.len() {
            deduped.len()
        } else {
            m
        }
    });

    let mut result = Vec::new();
    for (idx, slot) in deduped.iter().enumerate() {
        if split_before_idx == Some(idx) {
            result.push(ParsedSlot {
                category: String::from("Routing"),
                name: String::from("Split (branche parallèle)"),
                grid_x: None,
                grid_y: None,
            });
        }
        if merge_before_idx == Some(idx) {
            result.push(ParsedSlot {
                category: String::from("Routing"),
                name: String::from("Merge/Mixer"),
                grid_x: None,
                grid_y: None,
            });
        }
        result.push(slot.clone());
    }
    if merge_before_idx == Some(deduped.len()) {
        result.push(ParsedSlot {
            category: String::from("Routing"),
            name: String::from("Merge/Mixer"),
            grid_x: None,
            grid_y: None,
        });
    }

    result
}

/// Marqueur de slot vide (Kempline `request_preset.py` : `0814c0`).
const KEMPLINE_EMPTY_SLOT: [u8; 3] = [0x08, 0x14, 0xc0];

/// Indices des 16 blocs assignables dans la fenêtre de 20 segments (slots 1–8 puis 11–18).
const KEMPLINE_ASSIG_INDICES: [usize; 16] =
    [1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16, 17, 18];

fn extract_grid_xy_after_id(chunk: &[u8], scan_from: usize) -> (Option<u8>, Option<u8>) {
    let end = chunk.len().min(scan_from.saturating_add(40));
    let mut gx = None;
    let mut gy = None;
    let mut i = scan_from;
    while i + 1 < end {
        if chunk[i] == 0x02 && gx.is_none() {
            gx = Some(chunk[i + 1]);
        }
        if chunk[i] == 0x03 && gy.is_none() {
            gy = Some(chunk[i + 1]);
        }
        i += 1;
    }
    (gx, gy)
}

/// Premier module 0x19…0x1a dans un segment de slot assignable.
fn extract_first_module_from_assignable_chunk(chunk: &[u8]) -> ParsedSlot {
    let mut cursor = 0usize;
    while cursor < chunk.len() {
        if chunk[cursor] == 0x19 {
            let id_start = cursor + 1;
            if let Some(rel_end) = chunk[id_start..].iter().position(|&b| b == 0x1a) {
                let id_bytes = &chunk[id_start..id_start + rel_end];
                if !id_bytes.is_empty() {
                    let mut id_hex = String::with_capacity(id_bytes.len() * 2);
                    for b in id_bytes {
                        let _ = write!(&mut id_hex, "{:02x}", b);
                    }
                    let (category, name) = if let Some(entry) = MODULES_BY_ID
                        .get(&id_hex)
                        .or_else(|| MODULE_FALLBACKS.get(&id_hex))
                    {
                        (entry[0].clone(), entry[1].clone())
                    } else {
                        (String::from("Unknown"), id_hex)
                    };
                    let after = id_start + rel_end + 1;
                    let (grid_x, grid_y) = extract_grid_xy_after_id(chunk, after);
                    return ParsedSlot {
                        category,
                        name,
                        grid_x,
                        grid_y,
                    };
                }
                cursor = id_start + rel_end + 1;
                continue;
            }
        }
        cursor += 1;
    }
    ParsedSlot {
        category: String::from("Unknown"),
        name: String::from("(sans id)"),
        grid_x: None,
        grid_y: None,
    }
}

/// Grille fixe 8 + 8 emplacements (Kempline `preset_info_complete`) : None si le dump ne suit pas ce format.
fn try_parse_preset_kempline_grid(data: &[u8]) -> Option<Vec<ParsedSlot>> {
    const WIN: usize = 20;
    let segs = split_preset_by_8213(data);
    let slot1 = segs.iter().position(|seg| {
        seg.first()
            .map(|b| *b == 0x06 || *b == 0x08)
            .unwrap_or(false)
    })?;
    if slot1 == 0 {
        return None;
    }
    let start = slot1.checked_sub(1)?;
    let end = start.checked_add(WIN)?;
    if segs.len() < end {
        return None;
    }
    let w = &segs[start..end];
    if w[0].first().copied() != Some(0x00) {
        return None;
    }
    if w[9].first().copied() != Some(0x01) {
        return None;
    }
    if w[10].first().copied() != Some(0x02) {
        return None;
    }
    if w[19].first().copied() != Some(0x03) {
        return None;
    }
    for &idx in &KEMPLINE_ASSIG_INDICES {
        let fb = w[idx].first().copied()?;
        if fb != 0x06 && fb != 0x08 {
            return None;
        }
    }

    let mut out: Vec<ParsedSlot> = Vec::with_capacity(16);
    for &idx in &KEMPLINE_ASSIG_INDICES {
        let seg = w[idx];
        let cell = if seg.len() == 3 && seg == KEMPLINE_EMPTY_SLOT.as_slice() {
            ParsedSlot {
                category: String::new(),
                name: String::from("<empty>"),
                grid_x: None,
                grid_y: None,
            }
        } else {
            extract_first_module_from_assignable_chunk(seg)
        };
        out.push(cell);
    }
    eprintln!(
        "[PresetDebug][try_parse_preset_kempline_grid] ok: 16 assignable slots from {} segments",
        segs.len()
    );
    Some(out)
}

fn parse_preset_slots(data: &[u8]) -> Vec<[String; 2]> {
    if let Some(grid) = try_parse_preset_kempline_grid(data) {
        return grid.into_iter().map(|s| [s.category, s.name]).collect();
    }
    parse_preset_slots_internal(data)
        .into_iter()
        .map(|s| [s.category, s.name])
        .collect()
}

fn parse_preset_slots_debug(data: &[u8]) -> Vec<[String; 4]> {
    if let Some(grid) = try_parse_preset_kempline_grid(data) {
        return grid
            .into_iter()
            .map(|s| {
                [
                    s.category,
                    s.name,
                    s.grid_x.map(|v| v.to_string()).unwrap_or_default(),
                    s.grid_y.map(|v| v.to_string()).unwrap_or_default(),
                ]
            })
            .collect();
    }
    parse_preset_slots_internal(data)
        .into_iter()
        .map(|s| {
            [
                s.category,
                s.name,
                s.grid_x.map(|v| v.to_string()).unwrap_or_default(),
                s.grid_y.map(|v| v.to_string()).unwrap_or_default(),
            ]
        })
        .collect()
}

// ===========================================================
// Point d'entrée Tauri
// ===========================================================

pub fn run() {
    let app_state = Arc::new(Mutex::new(AppState::default()));
    let app_state_clone = Arc::clone(&app_state);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    save_window_layout(&window.app_handle());
                    window.app_handle().exit(0);
                } else {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_preset_names,
            get_active_preset,
            get_connected_device_name,
            get_connection_hint_text,
            request_active_preset_name,
            rename_preset,
            activate_preset,
            request_preset_content,
            get_preset_slots,
            get_active_preset_slots,
            get_active_preset_slots_debug,
            get_active_preset_routing_markers,
            get_active_preset_stomp_layout,
            get_preset_data_hex,
        ])
        .setup(move |app| {
            if let Some(main_window) = app.get_webview_window("main") {
                if let Some(saved_layout) = read_window_layout(&app.app_handle()) {
                    if let Some(main) = &saved_layout.main {
                        apply_window_geometry(&main_window, main);
                    }
                } else {
                    let target_height = 760.0;
                    let _ = main_window
                        .set_size(Size::Logical(LogicalSize::new(1280.0, target_height)));
                    let _ = main_window
                        .set_position(Position::Logical(LogicalPosition::new(80.0, 80.0)));
                }
                let _ = main_window.set_focus();
            }

            let state = app_state_clone;
            thread::spawn(move || {
                start_monitor(state);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ===========================================================
// Surveille le branchement USB et lance/arrête la connexion
// ===========================================================
fn start_monitor(app_state: Arc<Mutex<AppState>>) {
    let stop_monitor = Arc::new(AtomicBool::new(false));
    let state_for_connect = Arc::clone(&app_state);
    let state_for_lost = Arc::clone(&app_state);

    helix::usb_monitor::start_monitor(
        Arc::new(Mutex::new(HelixState::new())),
        Arc::clone(&stop_monitor),
        Arc::new(move || {
            let state = Arc::clone(&state_for_connect);
            {
                let mut app = state.lock().unwrap();
                app.connection_issue_hint = None;
            }
            thread::spawn(move || {
                start_helix(state);
            });
        }),
        Arc::new(move || {
            let mut app = state_for_lost.lock().unwrap();
            app.helix_state = None;
            app.usb_handle = None;
            app.connected_device_name = None;
            app.connection_issue_hint = None;
        }),
    );
}

// ===========================================================
// Connexion complète au HX et boucle de traitement
// ===========================================================
fn start_helix(app_state: Arc<Mutex<AppState>>) {
    // Ouvrir le premier device supporté trouvé (HX Stomp XL / Stomp / Floor / LT).
    let (device_name, handle) = match find_supported_device_handle() {
        Some(tuple) => tuple,
        None => {
            eprintln!("[Helix] Aucun device HX supporté trouvé");
            let mut app = app_state.lock().unwrap();
            app.connection_issue_hint = None;
            return;
        }
    };
    eprintln!("[Helix] Device connecté: {}", device_name);

    // Réclamer l'interface USB
    if let Err(e) = handle.claim_interface(0) {
        eprintln!("[Helix] erreur claim_interface : {}", e);
        let mut app = app_state.lock().unwrap();
        app.connected_device_name = Some(device_name.to_string());
        app.connection_issue_hint =
            Some("Protocol handshake failed (possible firmware mismatch)".to_string());
        return;
    }

    // Vider le buffer 0x81 — le HX peut avoir des données résiduelles
    let mut flush_buf = vec![0u8; 512];
    let flush_deadline = std::time::Instant::now() + Duration::from_millis(200);
    while std::time::Instant::now() < flush_deadline {
        match handle.read_bulk(0x81, &mut flush_buf, Duration::from_millis(50)) {
            Ok(n) if n > 0 => { /* paquet résiduel ignoré */ }
            _ => break,
        }
    }
    // Réclamer l'interface MIDI (interface 4)
    if let Err(_e) = handle.kernel_driver_active(4) {
        eprintln!("[Helix] kernel_driver_active(4) : {}", _e);
    }
    if handle.kernel_driver_active(4).unwrap_or(false) {
        let _ = handle.detach_kernel_driver(4);
    }
    if let Err(e) = handle.claim_interface(4) {
        eprintln!("[Helix] erreur claim_interface(4) : {}", e);
    }

    // Kempline : clear_feature ENDPOINT_HALT sur 0x01 et 0x81
    if let Err(e) = handle.clear_halt(0x01) {
        eprintln!("[Helix] clear_halt 0x01 : {}", e);
    }
    if let Err(e) = handle.clear_halt(0x81) {
        eprintln!("[Helix] clear_halt 0x81 : {}", e);
    }

    // -- Channels --
    let (usb_tx,  usb_rx)  = mpsc::channel::<OutPacket>();
    let (mode_tx, mode_rx) = mpsc::channel::<ModeRequest>();
    let (ka_tx,   ka_rx)   = mpsc::channel::<KeepAliveCommand>();

    // -- État partagé --
    let state = Arc::new(Mutex::new(HelixState::new()));
    {
        let mut s = state.lock().unwrap();
        s.tx           = Some(usb_tx);
        s.mode_tx      = Some(mode_tx.clone());
        s.keepalive_tx = Some(ka_tx);
        s.new_session_no();
    }

    // Stocker le HelixState et le handle USB dans AppState
    {
        let mut app = app_state.lock().unwrap();
        app.helix_state = Some(Arc::clone(&state));
        app.usb_handle  = Some(Arc::clone(&handle));
        app.connected_device_name = Some(device_name.to_string());
        app.connection_issue_hint = None;
    }

    // -- Démarrer usb_writer --
    helix::usb_writer::start_writer(Arc::clone(&handle), usb_rx);

    // -- Mode initial : Connect --
    let current_mode: Arc<Mutex<Box<dyn helix::Mode>>> =
        Arc::new(Mutex::new(Box::new(Connect::new())));

    {
        let mut s = state.lock().unwrap();
        let mut m = current_mode.lock().unwrap();
        m.start(&mut s);
    }

    // -- Démarrer usb_listener --
    let stop_listener = Arc::new(AtomicBool::new(false));
    helix::usb_listener::start_listener(
        Arc::clone(&handle),
        Arc::clone(&state),
        Arc::clone(&current_mode),
        Arc::clone(&stop_listener),
    );

    // -- KeepAlive manager --
    let ka_manager = Arc::new(KeepAliveManager::new());
    {
        let ka = Arc::clone(&ka_manager);
        let s  = Arc::clone(&state);
        thread::spawn(move || {
            loop {
                match ka_rx.recv() {
                    Ok(KeepAliveCommand::StartX1)  => ka.start_x1(Arc::clone(&s)),
                    Ok(KeepAliveCommand::StartX2)  => ka.start_x2(Arc::clone(&s)),
                    Ok(KeepAliveCommand::StartX80) => ka.start_x80(Arc::clone(&s)),
                    Ok(KeepAliveCommand::StopAll)  => {
                        ka.stop_all();
                        break;
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // -- Thread MIDI listener — endpoint 0x82 --
    {
        let handle_midi  = Arc::clone(&handle);
        let mode_tx_midi = mode_tx.clone();
        let state_midi   = Arc::clone(&state);
        thread::spawn(move || {
            let mut buf = vec![0u8; 64];
            loop {
                match handle_midi.read_bulk(0x82, &mut buf, Duration::from_millis(500)) {
                    Ok(n) if n >= 4 => {
                        if buf[0] == 0x0C && (buf[1] & 0xF0) == 0xC0 {
                            let preset_no = buf[2] as usize;
                            {
                                let mut s = state_midi.lock().unwrap();
                                s.preset_index = preset_no;
                            }
                            let _ = mode_tx_midi.send(ModeRequest::RequestPresetName);
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {}
                }
            }
        });
    }

    // -- Boucle principale : changements de mode --
    loop {
        match mode_rx.recv() {
            Ok(ModeRequest::RequestPreset) => {
                // Dédupliquer les RequestPreset consécutifs (course avec flux auto).
                let mut dropped = 0usize;
                loop {
                    match mode_rx.try_recv() {
                        Ok(ModeRequest::RequestPreset) => dropped += 1,
                        Ok(other) => {
                            // Remettre l'événement non-RequestPreset via un re-send local.
                            // Comme le receiver est unique et ce cas est rare, on le traite
                            // en repassant immédiatement par switch_mode.
                            let s = state.lock().unwrap();
                            s.switch_mode(other);
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => break,
                    }
                }
                if dropped > 0 {
                    eprintln!("[PresetDebug][ModeLoop] dropped duplicate RequestPreset x{}", dropped);
                }

                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(RequestPreset::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::RequestPresetName) => {
                let mut s = state.lock().unwrap();
                // Pendant une lecture content_only, ignorer les changements de
                // preset asynchrones pour ne pas interrompre RequestPreset.
                if s.preset_content_only {
                    eprintln!("[PresetDebug][ModeLoop] ignore RequestPresetName while content_only");
                    continue;
                }
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(RequestPresetName::new());
                {
                    let mut app = app_state.lock().unwrap();
                    app.active_preset = s.preset_index;
                }
                m.start(&mut s);
            }
            Ok(ModeRequest::RequestPresetNames) => {
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(RequestPresetNames::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::Standard) => {
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                {
                    let mut app = app_state.lock().unwrap();
                    // Ne jamais écraser la liste globale avec un état partiel.
                    if s.just_fetched_preset_names && s.preset_names.len() >= EXPECTED_PRESET_COUNT {
                        app.preset_names = s.preset_names.clone();
                    }
                    s.just_fetched_preset_names = false;
                    s.got_preset = false;
                    app.active_preset = s.preset_index;
                    app.connection_issue_hint = None;
                }
                *m = Box::new(Standard);
                m.start(&mut s);
            }
            Ok(ModeRequest::Connect) => {
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(Connect::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::ReconfigureX1) => {
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(ReconfigureX1::new());
                m.start(&mut s);
            }
            Err(_) => {
                let mut app = app_state.lock().unwrap();
                if app.connected_device_name.is_some() {
                    app.connection_issue_hint =
                        Some("Protocol handshake failed (possible firmware mismatch)".to_string());
                }
                break;
            }
        }
    }

    // -- Nettoyage --
    stop_listener.store(true, Ordering::SeqCst);
    ka_manager.stop_all();
    thread::sleep(Duration::from_millis(1100));
    let _ = handle.release_interface(0);
    if let Err(e) = handle.attach_kernel_driver(0) {
        eprintln!("[Helix] attach_kernel_driver : {}", e);
    }

    // Effacer le HelixState de l'AppState à la déconnexion
    {
        let mut app = app_state.lock().unwrap();
        app.helix_state = None;
        app.usb_handle = None;
        if app.connected_device_name.is_none() {
            app.connection_issue_hint = None;
        }
        app.connected_device_name = None;
    }
}

fn find_supported_device_handle() -> Option<(&'static str, Arc<rusb::DeviceHandle<rusb::GlobalContext>>)> {
    let devices = rusb::devices().ok()?;
    for device in devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if let Some((name, _, _)) = SUPPORTED_DEVICES
            .iter()
            .find(|(_, vid, pid)| desc.vendor_id() == *vid && desc.product_id() == *pid)
        {
            if let Ok(handle) = device.open() {
                return Some((*name, Arc::new(handle)));
            }
        }
    }
    None
}