// ===========================================================
// lib.rs
// Point d'entrée — assemble tous les composants
// Le UsbMonitor surveille le branchement du HX et déclenche
// automatiquement la séquence de connexion
// ===========================================================

mod helix;
mod stomp_layout;
mod preset_chain_params;

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
use serde_json::Value;

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

/// Retourne les slots du dernier preset lu, sous forme [catégorie, nom, module_hex].
#[tauri::command]
fn get_preset_slots(state: tauri::State<Arc<Mutex<AppState>>>) -> Option<Vec<[String; 3]>> {
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
fn get_active_preset_slots(state: tauri::State<Arc<Mutex<AppState>>>) -> Option<Vec<[String; 3]>> {
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

/// Version debug : coordonnées grille [x, y] + `module_hex`.
#[tauri::command]
fn get_active_preset_slots_debug(state: tauri::State<Arc<Mutex<AppState>>>) -> Option<Vec<[String; 5]>> {
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

/// Valeurs de chaîne lues dans le segment preset du slot **Kempline 0..15** (`read_params` Python).
/// `None` : pas de données preset actives, slot hors plage, segment vide `06`, ou parse impossible.
#[tauri::command]
fn get_active_preset_slot_chain_param_values(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
) -> Option<Vec<preset_chain_params::ChainParamValue>> {
    if slot_index >= 16 {
        return None;
    }
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
    let seg = kempline_assignable_segment_bytes(&s.preset_data, slot_index as usize)?;
    chain_param_values_for_assignable_segment(&seg)
}

/// Cab rattaché détecté dans un slot Amp+Cab, sous la forme `[module_hex, catégorie, nom, model_id]`.
#[tauri::command]
fn get_active_preset_slot_linked_cab(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
) -> Option<[String; 4]> {
    if slot_index >= 16 {
        return None;
    }
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
    let seg = kempline_assignable_segment_bytes(&s.preset_data, slot_index as usize)?;
    if !is_amp_cab_assignable_chunk(seg) {
        return None;
    }
    let ids = extract_module_ids_from_assignable_chunk(seg);
    let blocks = preset_chain_params::parse_assignable_segment_param_blocks(&seg)?;
    let cab_bi = catalog_cab_c219_block_index(&ids, blocks.len());
    cab_bi
        .and_then(|bi| block_chain_hex_for_c219(bi, &ids))
        .and_then(|h| cab_info_from_module_id(&h))
        .or_else(|| ids.iter().find_map(|id| cab_info_from_module_id(id)))
}

#[derive(Debug, Clone, Serialize)]
struct LinkedCabWithParams {
    cab: [String; 4],
    values: Vec<preset_chain_params::ChainParamValue>,
}

/// Cab rattaché + valeurs de ses paramètres décodées dans la chaîne Amp+Cab.
#[tauri::command]
fn get_active_preset_slot_linked_cab_with_params(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
) -> Option<LinkedCabWithParams> {
    if slot_index >= 16 {
        return None;
    }
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
    let seg = kempline_assignable_segment_bytes(&s.preset_data, slot_index as usize)?;
    if !is_amp_cab_assignable_chunk(seg) {
        return None;
    }

    let ids = extract_module_ids_from_assignable_chunk(seg);
    let blocks = preset_chain_params::parse_assignable_segment_param_blocks(seg)?;

    let cab_bi = catalog_cab_c219_block_index(&ids, blocks.len());

    let cab = cab_bi
        .and_then(|bi| block_chain_hex_for_c219(bi, &ids))
        .and_then(|h| cab_info_from_module_id(&h))
        .or_else(|| ids.iter().find_map(|id| cab_info_from_module_id(id)))?;

    let values = cab_bi
        .and_then(|bi| blocks.get(bi))
        .cloned()
        .or_else(|| blocks.last().cloned())?;
    Some(LinkedCabWithParams { cab, values })
}

/// Lecture d’un fichier JSON de définition de modèles (`resources/models/{file_base}.models`).
#[tauri::command]
fn read_models_definition_file(app: tauri::AppHandle, file_base: String) -> Result<String, String> {
    if !file_base
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("nom de fichier .models invalide".into());
    }
    let path = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("resources/models")
        .join(format!("{}.models", file_base));
    fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))
}

/// Remplit `map` depuis `HX_ModelCatalog.json` : chaque `presetMeta.chainHex` (chaîne ou tableau)
/// → `[catégorie, nom modèle]` (`presetMeta.categoryName` + `name` si liste plate `models[]`, sinon
/// `categories[].name` + `models[].name` en format historique).
fn insert_chainhex_into_module_map(
    map: &mut HashMap<String, [String; 2]>,
    pair: &[String; 2],
    hex_v: &Value,
) {
    match hex_v {
        Value::String(s) => {
            let h = s.trim().to_lowercase();
            if !h.is_empty() {
                map.insert(h, pair.clone());
            }
        }
        Value::Array(a) => {
            for x in a {
                if let Some(s) = x.as_str() {
                    let h = s.trim().to_lowercase();
                    if !h.is_empty() {
                        map.insert(h, pair.clone());
                    }
                }
            }
        }
        _ => {}
    }
}

fn insert_hex_from_flat_models_list(map: &mut HashMap<String, [String; 2]>, models: &[Value]) {
    for m in models {
        let Some(obj) = m.as_object() else {
            continue;
        };
        let Some(model_name) = obj.get("name").and_then(|x| x.as_str()) else {
            continue;
        };
        let model_name = model_name.trim();
        if model_name.is_empty() {
            continue;
        };
        let Some(pm) = obj.get("presetMeta").and_then(|p| p.as_object()) else {
            continue;
        };
        let cat_name = pm
            .get("categoryName")
            .and_then(|n| n.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("Unknown");
        let Some(hex_v) = pm.get("chainHex") else {
            continue;
        };
        let pair = [cat_name.to_string(), model_name.to_string()];
        insert_chainhex_into_module_map(map, &pair, hex_v);
    }
}

fn insert_modules_from_hx_catalog(map: &mut HashMap<String, [String; 2]>, catalog: &Value) {
    if let Some(models) = catalog.get("models").and_then(|m| m.as_array()) {
        if !models.is_empty() {
            insert_hex_from_flat_models_list(map, models);
            return;
        }
    }
    let Some(categories) = catalog.get("categories").and_then(|c| c.as_array()) else {
        return;
    };
    for cat in categories {
        let Some(cat_name) = cat.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let cat_name = cat_name.trim();
        if cat_name.is_empty() {
            continue;
        }
        insert_hex_from_catalog_model_list(map, cat_name, cat.get("models"));
        if let Some(subs) = cat.get("subcategories").and_then(|s| s.as_array()) {
            for sub in subs {
                insert_hex_from_catalog_model_list(map, cat_name, sub.get("models"));
            }
        }
    }
}

fn insert_hex_from_catalog_model_list(
    map: &mut HashMap<String, [String; 2]>,
    cat_name: &str,
    models: Option<&Value>,
) {
    let Some(arr) = models.and_then(|m| m.as_array()) else {
        return;
    };
    for m in arr {
        let Some(obj) = m.as_object() else {
            continue;
        };
        let Some(model_name) = obj.get("name").and_then(|x| x.as_str()) else {
            continue;
        };
        let model_name = model_name.trim();
        if model_name.is_empty() {
            continue;
        };
        let Some(pm) = obj.get("presetMeta").and_then(|p| p.as_object()) else {
            continue;
        };
        let Some(hex_v) = pm.get("chainHex") else {
            continue;
        };
        let pair = [cat_name.to_string(), model_name.to_string()];
        insert_chainhex_into_module_map(map, &pair, hex_v);
    }
}

fn insert_model_id_hexes(map: &mut HashMap<String, String>, model_id: &str, hex_v: &Value) {
    match hex_v {
        Value::String(s) => {
            let h = s.trim().to_lowercase();
            if !h.is_empty() {
                map.insert(h, model_id.to_string());
            }
        }
        Value::Array(a) => {
            for x in a {
                if let Some(s) = x.as_str() {
                    let h = s.trim().to_lowercase();
                    if !h.is_empty() {
                        map.insert(h, model_id.to_string());
                    }
                }
            }
        }
        _ => {}
    }
}

fn insert_model_ids_from_flat_models(map: &mut HashMap<String, String>, models: &[Value]) {
    for m in models {
        let Some(obj) = m.as_object() else {
            continue;
        };
        let Some(model_id) = obj
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(pm) = obj.get("presetMeta").and_then(|p| p.as_object()) else {
            continue;
        };
        let Some(hex_v) = pm.get("chainHex") else {
            continue;
        };
        insert_model_id_hexes(map, &model_id, hex_v);
    }
}

/// Remplit `map` hex -> `id` modèle (catalogue), depuis `HX_ModelCatalog.json`.
fn insert_model_ids_from_hx_catalog(map: &mut HashMap<String, String>, catalog: &Value) {
    if let Some(models) = catalog.get("models").and_then(|m| m.as_array()) {
        if !models.is_empty() {
            insert_model_ids_from_flat_models(map, models);
            return;
        }
    }
    let Some(categories) = catalog.get("categories").and_then(|c| c.as_array()) else {
        return;
    };
    let mut insert_for_models = |models: Option<&Value>| {
        let Some(arr) = models.and_then(|m| m.as_array()) else {
            return;
        };
        for m in arr {
            let Some(obj) = m.as_object() else {
                continue;
            };
            let model_id = obj
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let Some(model_id) = model_id else {
                continue;
            };
            let Some(pm) = obj.get("presetMeta").and_then(|p| p.as_object()) else {
                continue;
            };
            let Some(hex_v) = pm.get("chainHex") else {
                continue;
            };
            insert_model_id_hexes(map, &model_id, hex_v);
        }
    };
    for cat in categories {
        let Some(cat_obj) = cat.as_object() else {
            continue;
        };
        insert_for_models(cat_obj.get("models"));
        if let Some(subs) = cat_obj.get("subcategories").and_then(|s| s.as_array()) {
            for sub in subs {
                if let Some(sub_obj) = sub.as_object() {
                    insert_for_models(sub_obj.get("models"));
                }
            }
        }
    }
}

lazy_static! {
    /// ID module (hex entre 0x19…0x1a) → `[catégorie, nom]`.
    /// Source unique : `HX_ModelCatalog.json` (`presetMeta.chainHex` chaîne ou tableau + nom court du modèle).
    static ref HX_CATALOG_MODULE_BY_HEX: HashMap<String, [String; 2]> = {
        const HX_CATALOG_JSON: &str = include_str!("../resources/HX_ModelCatalog.json");
        let catalog: Value =
            serde_json::from_str(HX_CATALOG_JSON).expect("HX_ModelCatalog.json invalide");
        let mut map = HashMap::new();
        insert_modules_from_hx_catalog(&mut map, &catalog);
        map
    };
    /// ID module hex -> ID modèle catalogue (`models[].id`), utile pour jointure stable vers `.models.symbolicID`.
    static ref MODEL_ID_BY_HEX: HashMap<String, String> = {
        const HX_CATALOG_JSON: &str = include_str!("../resources/HX_ModelCatalog.json");
        let catalog: Value =
            serde_json::from_str(HX_CATALOG_JSON).expect("HX_ModelCatalog.json invalide");
        let mut map = HashMap::new();
        insert_model_ids_from_hx_catalog(&mut map, &catalog);
        map
    };
}

/// Famille catalogue pour un `chainHex` : choix du bloc `c219` ampli vs cab en Amp+Cab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogSlotKind {
    AmpLike,
    CabLike,
    Other,
}

/// Candidats hex pour joindre le catalogue (comme côté TS : chaîne complète puis préfixe avant `1a`).
fn chain_hex_catalog_lookup_candidates(hex_norm: &str) -> Vec<String> {
    let h = hex_norm.trim().to_lowercase();
    if h.is_empty() {
        return Vec::new();
    }
    let mut out = vec![h.clone()];
    if let Some(i) = h.find("1a") {
        let prefix = h.get(..i).unwrap_or("").to_string();
        if !prefix.is_empty() && prefix != h {
            out.push(prefix);
        }
    }
    out
}

/// `pair[0]` = `presetMeta.categoryName` dans le catalogue plat.
fn catalog_slot_kind_from_category_name(cat: &str) -> CatalogSlotKind {
    let c = cat.trim().to_lowercase();
    if c == "amp" || c == "preamp" {
        CatalogSlotKind::AmpLike
    } else if c == "cab" || c == "ir" || c.contains("impulse") {
        CatalogSlotKind::CabLike
    } else {
        CatalogSlotKind::Other
    }
}

/// Repli si le hex n’est pas indexé : préfixes d’`id` catalogue Line 6.
fn catalog_slot_kind_from_model_id(id: &str) -> Option<CatalogSlotKind> {
    let lower = id.trim().to_ascii_lowercase();
    if lower.starts_with("hd2_amp") || lower.starts_with("hd2_preamp") {
        return Some(CatalogSlotKind::AmpLike);
    }
    if lower.starts_with("hd2_cab") {
        return Some(CatalogSlotKind::CabLike);
    }
    None
}

fn catalog_slot_kind_for_chain_hex(hex: &str) -> CatalogSlotKind {
    let h = hex.trim().to_lowercase();
    if h.is_empty() {
        return CatalogSlotKind::Other;
    }
    for cand in chain_hex_catalog_lookup_candidates(&h) {
        if let Some(pair) = HX_CATALOG_MODULE_BY_HEX.get(&cand) {
            let k = catalog_slot_kind_from_category_name(&pair[0]);
            if k != CatalogSlotKind::Other {
                return k;
            }
        }
        if let Some(id) = MODEL_ID_BY_HEX.get(&cand) {
            if let Some(k) = catalog_slot_kind_from_model_id(id) {
                return k;
            }
        }
    }
    CatalogSlotKind::Other
}

/// Hex module (`presetMeta.chainHex` dans le catalogue) associé au bloc `c219` d’index `block_index`.
/// Plusieurs IDs `0x19…0x1a` : un bloc par ID. Un seul ID combiné `ampHex1acabHex` : bloc 0 / 1 = ampli / cab
/// (ordre inversé si le catalogue indique clairement cab puis ampli sur les deux parties).
fn block_chain_hex_for_c219(block_index: usize, ids: &[String]) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    if ids.len() == 1 {
        let h = ids[0].trim().to_lowercase();
        if let Some(sep) = h.find("1a") {
            let amp = h.get(..sep)?.to_string();
            let cab = h.get(sep + 2..)?.to_string();
            if amp.is_empty() || cab.is_empty() {
                return None;
            }
            let amp_k = catalog_slot_kind_for_chain_hex(&amp);
            let cab_k = catalog_slot_kind_for_chain_hex(&cab);
            let swap = matches!(amp_k, CatalogSlotKind::CabLike)
                && matches!(cab_k, CatalogSlotKind::AmpLike);
            let (first_hex, second_hex) = if swap { (cab, amp) } else { (amp, cab) };
            return match block_index {
                0 => Some(first_hex),
                1 => Some(second_hex),
                _ => None,
            };
        }
        return (block_index == 0).then_some(h);
    }
    ids.get(block_index)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
}

/// Index du bloc `c219` dont le `chainHex` (catalogue) est cab / IR, ou `None`.
fn catalog_cab_c219_block_index(ids: &[String], num_blocks: usize) -> Option<usize> {
    (0..num_blocks).find(|&bi| {
        block_chain_hex_for_c219(bi, ids)
            .map(|h| matches!(catalog_slot_kind_for_chain_hex(&h), CatalogSlotKind::CabLike))
            .unwrap_or(false)
    })
}

/// Valeurs `read_params` du bloc **ampli** : en Amp+Cab (`c319`), le bloc est choisi via le catalogue
/// (`HX_CATALOG_MODULE_BY_HEX` / `chainHex`), pas l’index seul.
fn chain_param_values_for_assignable_segment(seg: &[u8]) -> Option<Vec<preset_chain_params::ChainParamValue>> {
    let blocks = preset_chain_params::parse_assignable_segment_param_blocks(seg)?;
    match blocks.len() {
        0 => None,
        1 => Some(blocks[0].clone()),
        _ => {
            if !is_amp_cab_assignable_chunk(seg) {
                return Some(blocks[0].clone());
            }
            let ids = extract_module_ids_from_assignable_chunk(seg);
            if ids.is_empty() {
                return blocks.iter().max_by_key(|b| b.len()).cloned();
            }
            for bi in 0..blocks.len() {
                if let Some(h) = block_chain_hex_for_c219(bi, &ids) {
                    if matches!(catalog_slot_kind_for_chain_hex(&h), CatalogSlotKind::AmpLike) {
                        return Some(blocks[bi].clone());
                    }
                }
            }
            blocks.iter().max_by_key(|b| b.len()).cloned()
        }
    }
}

#[derive(Clone)]
struct ParsedSlot {
    category: String,
    name: String,
    grid_x: Option<u8>,
    grid_y: Option<u8>,
    /// Hex entre 0x19…0x1a (minuscules), vide si inconnu / slot vide.
    module_hex: String,
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
/// Décodage "best effort" : on segmente sur [0x82, 0x13], puis on retient
/// au plus un module par segment (priorité à un ID connu dans HX_CATALOG_MODULE_BY_HEX).
/// Cela évite de sur-parser des séquences 0x19..0x1a parasites.
fn parse_preset_slots_internal(data: &[u8]) -> Vec<ParsedSlot> {
    fn category_with_chunk_hint(base_category: &str, chunk: &[u8]) -> String {
        if base_category.eq_ignore_ascii_case("amp") && is_amp_cab_assignable_chunk(chunk) {
            return String::from("Amp+Cab");
        }
        base_category.to_string()
    }

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
        if chunk.is_empty() {
            continue;
        }
        let is_assignable_chunk = matches!(chunk.first().copied(), Some(0x06 | 0x08));
        let mut best_unknown: Option<ParsedSlot> = None;
        let mut cursor = 0usize;

        while cursor < chunk.len() {
            if chunk[cursor] != 0x19 {
                cursor += 1;
                continue;
            }
            let id_start = cursor + 1;
            if let Some(rel_end) = chunk[id_start..].iter().position(|&b| b == 0x1a) {
                let id_bytes = &chunk[id_start..id_start + rel_end];
                let after_id = id_start + rel_end + 1;
                cursor = after_id;
                if id_bytes.is_empty() {
                    continue;
                }

                let mut id_hex = String::with_capacity(id_bytes.len() * 2);
                for b in id_bytes {
                    let _ = write!(&mut id_hex, "{:02x}", b);
                }

                if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(&id_hex) {
                    parsed_slots.push(ParsedSlot {
                        category: category_with_chunk_hint(&entry[0], chunk),
                        name: entry[1].clone(),
                        grid_x: extract_grid_x(chunk, after_id),
                        grid_y: extract_grid_y(chunk, after_id),
                        module_hex: id_hex,
                    });
                    // Un segment transporte au plus un module utile pour le fallback.
                    break;
                }

                // On garde un inconnu uniquement dans un segment assignable
                // (évite les faux positifs issus d'autres métadonnées).
                if is_assignable_chunk && best_unknown.is_none() {
                    best_unknown = Some(ParsedSlot {
                        category: String::from("Unknown"),
                        name: id_hex.clone(),
                        grid_x: extract_grid_x(chunk, after_id),
                        grid_y: extract_grid_y(chunk, after_id),
                        module_hex: id_hex,
                    });
                }
                continue;
            }
            cursor += 1;
        }

        if let Some(slot) = best_unknown {
            parsed_slots.push(slot);
        }
    }

    // Dédupliquer les doublons consécutifs (certains dumps peuvent contenir
    // une répétition du flux de blocs).
    let mut deduped: Vec<ParsedSlot> = Vec::with_capacity(parsed_slots.len());
    for slot in parsed_slots {
        let is_dup = deduped.last().map_or(false, |prev| {
            prev.category == slot.category
                && prev.name == slot.name
                && prev.module_hex == slot.module_hex
        });
        if !is_dup {
            deduped.push(slot);
        }
    }

    // Important: on ne doit pas inventer de blocs de routing dans ce parseur
    // de secours. Des marqueurs synthétiques peuvent décaler/altérer les données
    // remontées alors qu'on cherche ici un reflet brut des modules décodés.
    deduped
}

/// Marqueur de slot vide (Kempline `request_preset.py` : `0814c0`).
const KEMPLINE_EMPTY_SLOT: [u8; 3] = [0x08, 0x14, 0xc0];
/// Marqueur de bloc combiné Amp+Cab dans le segment assignable.
const AMP_CAB_MARKER: [u8; 6] = [0x85, 0x18, 0x83, 0x17, 0xc3, 0x19];

/// Indices des 16 blocs assignables dans la fenêtre de 20 segments (slots 1–8 puis 11–18).
const KEMPLINE_ASSIG_INDICES: [usize; 16] =
    [1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16, 17, 18];

fn is_amp_cab_assignable_chunk(chunk: &[u8]) -> bool {
    if chunk.first().copied() != Some(0x06) {
        return false;
    }
    chunk[1..]
        .windows(AMP_CAB_MARKER.len())
        .any(|w| w == AMP_CAB_MARKER)
}

/// Extrait tous les IDs module `0x19 <id...> 0x1a` trouvés dans un segment assignable.
fn extract_module_ids_from_assignable_chunk(chunk: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    while cursor < chunk.len() {
        if chunk[cursor] != 0x19 {
            cursor += 1;
            continue;
        }
        let id_start = cursor + 1;
        let Some(rel_end) = chunk[id_start..].iter().position(|&b| b == 0x1a) else {
            cursor += 1;
            continue;
        };
        let id_bytes = &chunk[id_start..id_start + rel_end];
        cursor = id_start + rel_end + 1;
        if id_bytes.is_empty() {
            continue;
        }
        let mut id_hex = String::with_capacity(id_bytes.len() * 2);
        for b in id_bytes {
            let _ = write!(&mut id_hex, "{:02x}", b);
        }
        out.push(id_hex);
    }
    out
}

/// Résout un module ID en info Cab:
/// - cas direct: `id_hex` est déjà un cab (`category == Cab`)
/// - cas combiné Amp+Cab: ID de forme `<amp_hex>1a<cab_hex>` ; on extrait alors la partie cab.
fn cab_info_from_module_id(id_hex: &str) -> Option<[String; 4]> {
    let entry = HX_CATALOG_MODULE_BY_HEX.get(id_hex)?;
    if entry[0].eq_ignore_ascii_case("cab") {
        let model_id = MODEL_ID_BY_HEX.get(id_hex).cloned().unwrap_or_default();
        return Some([
            id_hex.to_string(),
            entry[0].clone(),
            entry[1].clone(),
            model_id,
        ]);
    }
    if entry[0].eq_ignore_ascii_case("amp+cab") {
        let (_, cab_hex) = id_hex.rsplit_once("1a")?;
        let cab_hex = cab_hex.trim().to_lowercase();
        if cab_hex.is_empty() {
            return None;
        }
        let cab_entry = HX_CATALOG_MODULE_BY_HEX.get(&cab_hex)?;
        if !cab_entry[0].eq_ignore_ascii_case("cab") {
            return None;
        }
        let model_id = MODEL_ID_BY_HEX.get(&cab_hex).cloned().unwrap_or_default();
        return Some([
            cab_hex,
            cab_entry[0].clone(),
            cab_entry[1].clone(),
            model_id,
        ]);
    }
    None
}

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
                    let module_hex = id_hex.clone();
                    let (category, name) = if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(&id_hex) {
                        let cat =
                            if entry[0].eq_ignore_ascii_case("amp") && is_amp_cab_assignable_chunk(chunk) {
                                String::from("Amp+Cab")
                            } else {
                                entry[0].clone()
                            };
                        (cat, entry[1].clone())
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
                        module_hex,
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
        module_hex: String::new(),
    }
}

/// Indice de début (dans `split_preset_by_8213(data)`) de la fenêtre **20** segments grille Kempline, et nombre total de segments.
fn kempline_grid_window_start_and_seg_count(data: &[u8]) -> Option<(usize, usize)> {
    const WIN: usize = 20;
    let segs = split_preset_by_8213(data);
    let segs_len = segs.len();
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
    Some((start, segs_len))
}

/// Segment brut d’un slot assignable **0..16** (ordre grille : path1 puis path2), ou `None` si hors fenêtre / format.
fn kempline_assignable_segment_bytes(data: &[u8], slot_index: usize) -> Option<&[u8]> {
    if slot_index >= 16 {
        return None;
    }
    let (start, _) = kempline_grid_window_start_and_seg_count(data)?;
    let segs = split_preset_by_8213(data);
    let abs = start.checked_add(KEMPLINE_ASSIG_INDICES[slot_index])?;
    segs.get(abs).copied()
}

/// Grille fixe 8 + 8 emplacements (Kempline `preset_info_complete`) : None si le dump ne suit pas ce format.
fn try_parse_preset_kempline_grid(data: &[u8]) -> Option<Vec<ParsedSlot>> {
    let (start, segs_len) = kempline_grid_window_start_and_seg_count(data)?;
    let segs = split_preset_by_8213(data);
    let w = &segs[start..start + 20];
    let mut out: Vec<ParsedSlot> = Vec::with_capacity(16);
    for &idx in &KEMPLINE_ASSIG_INDICES {
        let seg = w[idx];
        let cell = if seg.len() == 3 && seg == KEMPLINE_EMPTY_SLOT.as_slice() {
            ParsedSlot {
                category: String::new(),
                name: String::from("<empty>"),
                grid_x: None,
                grid_y: None,
                module_hex: String::new(),
            }
        } else {
            extract_first_module_from_assignable_chunk(seg)
        };
        out.push(cell);
    }
    eprintln!(
        "[PresetDebug][try_parse_preset_kempline_grid] ok: 16 assignable slots from {} segments",
        segs_len
    );
    Some(out)
}

fn parse_preset_slots(data: &[u8]) -> Vec<[String; 3]> {
    if let Some(grid) = try_parse_preset_kempline_grid(data) {
        return grid
            .into_iter()
            .map(|s| [s.category, s.name, s.module_hex])
            .collect();
    }
    parse_preset_slots_internal(data)
        .into_iter()
        .map(|s| [s.category, s.name, s.module_hex])
        .collect()
}

fn parse_preset_slots_debug(data: &[u8]) -> Vec<[String; 5]> {
    if let Some(grid) = try_parse_preset_kempline_grid(data) {
        return grid
            .into_iter()
            .map(|s| {
                [
                    s.category,
                    s.name,
                    s.grid_x.map(|v| v.to_string()).unwrap_or_default(),
                    s.grid_y.map(|v| v.to_string()).unwrap_or_default(),
                    s.module_hex,
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
                s.module_hex,
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
            get_active_preset_slot_chain_param_values,
            get_active_preset_slot_linked_cab,
            get_active_preset_slot_linked_cab_with_params,
            get_preset_data_hex,
            read_models_definition_file,
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