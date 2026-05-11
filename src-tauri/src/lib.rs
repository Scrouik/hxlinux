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
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::{self, Write as IoWrite};
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
use helix::{
    kempline_index_to_slot_bus,
    preset_debug_verbose_enabled, set_preset_debug_verbose_enabled,
    set_usb_packet_trace_delta_only, set_usb_io_diag_enabled, set_usb_packet_trace_enabled,
    usb_packet_trace_delta_only, usb_packet_trace_enabled,
};
use helix::packet::OutPacket;
use helix::live_write::build_live_write_frames_from_state;
use helix::live_write_config::validate_usb_live_write_metadata;
use helix::edit_slot_model::{
    build_slot_model_probe_packets, change_model_hxedit_replace_test_bulk,
    resolve_catalog_model_chain_bytes, resolve_usb_assign_bulk, slot_probe_use_change_model_test_bulk,
    SlotModelProbeOp,
};
use helix::keep_alive::KeepAliveManager;
use helix::modes::connect::Connect;
use helix::modes::request_preset_name::RequestPresetName;
use helix::modes::request_preset_names::RequestPresetNames;
use helix::modes::standard::Standard;
use helix::modes::reconfigure_x1::ReconfigureX1;
use helix::modes::request_preset::RequestPreset;
use std::time::{Duration, Instant};
use lazy_static::lazy_static;

const SUPPORTED_DEVICES: &[(&str, u16, u16)] = &[
    ("HX Stomp XL", 0x0e41, 0x4253),
    ("HX Stomp", 0x0e41, 0x4246),
    ("Helix Floor", 0x0e41, 0x4248),
    ("Helix LT", 0x0e41, 0x424a),
];
const EXPECTED_PRESET_COUNT: usize = 125;
const WINDOW_LAYOUT_FILE: &str = "window-layout.json";
const PRESET_RECOVER_COOLDOWN_MS: u64 = 1_500;
const PRESET_RECOVER_MIN_REQUEST_AGE_MS: u64 = 700;
const PRESET_REQUEST_MIN_GAP_MS: u64 = 260;
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
    // Garde-fous backend anti-spam de recovery preset.
    preset_recover_in_flight: bool,
    last_preset_recover_at: Option<Instant>,
    last_preset_request_at: Option<Instant>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HardwareActiveSlotState {
    slot_index: Option<usize>,
    slot_bus: Option<u8>,
    sequence: u32,
}

#[tauri::command]
fn get_active_hardware_slot_state(
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> HardwareActiveSlotState {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let Some(helix_arc) = helix_arc else {
        return HardwareActiveSlotState {
            slot_index: None,
            slot_bus: None,
            sequence: 0,
        };
    };
    let s = helix_arc.lock().unwrap();
    HardwareActiveSlotState {
        slot_index: s.hw_active_slot_index,
        slot_bus: s.hw_active_slot_bus,
        sequence: s.hw_active_slot_sequence,
    }
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

    // Mettre à jour l'état local + signaler qu'on attend le x2 de confirmation
    // avant de lancer la lecture content_only (évite la race condition où cd:03
    // part avant que les x2 du hardware soient reçus et ACKés).
    let mut app = state.lock().unwrap();
    app.active_preset = index;
    if let Some(ref helix_arc) = app.helix_state {
        if let Ok(mut s) = helix_arc.lock() {
            s.want_content_only_after_x2 = true;
            // Le MIDI PC ne génère pas de paquet x2 avec 82 62 XX 1a → le slot actif
            // n'est jamais notifié. On remet à None pour éviter d'afficher le slot du
            // preset précédent pendant et après le chargement.
            s.hw_active_slot_index = None;
            s.hw_active_slot_bus = None;
            s.hw_active_slot_sequence = s.hw_active_slot_sequence.wrapping_add(1);
        }
    }

    Ok(())
}

/// Change le slot actif (focus UI matériel) via trame USB propriétaire.
/// Reverse-engineering depuis capture Windows "Slot1 to Slot2 from UI.json".
#[tauri::command]
fn switch_active_hardware_slot(
    slot_index: u32,
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;

    let mut s = helix_arc.lock().unwrap();
    // Garde-fou anti-embouteillage:
    // pendant un `request_preset_content`, le mode RequestPreset monopolise la pile.
    // Envoyer des commandes de switch slot dans cette fenêtre provoque des timeouts USB.
    if s.preset_content_only || !s.connected {
        return Err("switch slot ignoré pendant chargement preset".to_string());
    }
    let cnt = s.next_x80_cnt();
    let session = s.session_no;
    let double = s.preset_data_packet_double();
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or("slotIndex hors plage (0..15)")?;
    let packet = vec![
        0x1d, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt,  0x00, 0x04,
        session, double[0], double[1], 0x00,
        0x01, 0x00, 0x06, 0x00,
        0x0d, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        0xf9, 0x64, 0x4e, 0x65,
        0x82, 0x62, slot_bus, 0x1a,
        0x00, 0x00, 0x00, 0x00,
    ];
    s.send(OutPacket::new(packet.clone()));
    // Optimiste: met à jour l'état "slot actif hardware" local en attendant la notif IN.
    s.hw_active_slot_index = Some(slot_index as usize);
    s.hw_active_slot_bus = Some(slot_bus);
    s.hw_active_slot_sequence = s.hw_active_slot_sequence.wrapping_add(1);
    drop(s);

    eprintln!(
        "[HwSlotSwitch][sent] slot_index={} slot_bus={:02x} packet={}",
        slot_index,
        slot_bus,
        packet.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ")
    );
    Ok(())
}

/// Sonde expérimentale **focus slot** USB : même enveloppe `1d … 80:10:ed:03` que
/// [`switch_active_hardware_slot`], avec variante **`83:66:cd:04`** observée sur captures HX Edit
/// (`Slot1_to_slot2_PresetTest_HXEdit.json`, etc.) vs **`cd:03`** côté HXLinux actuel.
///
/// Ne met **pas** à jour `hw_active_slot_*` (évite de désynchroniser l’UI pendant l’essai).
/// Les réponses **IN** `0x81` sont consommées par `usb_listener` — pour les voir :  
/// `invoke("set_usb_trace_enabled", { enabled: true })` et/ou `set_usb_io_diag`.
///
/// Paramètres :
/// - `style` : `hx_edit_cd04` (défaut) ou `hxlinux_cd03` (équivalent commande switch).
/// - `tagByte` : optionnel ; octet après `cd` (captures HX : souvent `slot_bus` en `cd:04`, `0xf9` en `cd:03`).
/// - `includeChainFromDump` : si `true` (défaut), résume le parse `preset_chain_params` sur le segment
///   assignable du slot depuis **`preset_data`** déjà chargé (valide la chaîne parse, pas le trafic IN).
#[tauri::command]
fn probe_hardware_slot_focus_usb(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
    style: Option<String>,
    tag_byte: Option<u8>,
    include_chain_from_dump: Option<bool>,
) -> Result<Value, String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    let style_raw = style
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("hx_edit_cd04");
    let style_lc = style_raw.to_ascii_lowercase();
    let (cd_variant, style_key) = match style_lc.as_str() {
        "hx_edit" | "hx_edit_cd04" | "cd04" => (0x04u8, "hx_edit_cd04"),
        "hxlinux" | "hxlinux_cd03" | "cd03" | "linux" => (0x03u8, "hxlinux_cd03"),
        other => {
            return Err(format!(
                "style inconnu: {other} (attendu: hx_edit_cd04 | hxlinux_cd03)"
            ));
        }
    };

    let (active_preset, helix_arc) = {
        let app = state.lock().unwrap();
        (app.active_preset, app.helix_state.clone())
    };
    let helix_arc = helix_arc.ok_or_else(|| "HX non connecté".to_string())?;

    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or_else(|| "slotIndex invalide".to_string())?;

    let tag = tag_byte.unwrap_or(if cd_variant == 0x04 {
        slot_bus
    } else {
        0xf9
    });

    let mut s = helix_arc.lock().unwrap();
    if !s.connected {
        return Err("HX non connecté".to_string());
    }
    if s.preset_content_only {
        return Err("probe ignorée pendant lecture preset (preset_content_only)".to_string());
    }

    let cnt = s.next_x80_cnt();
    let session = s.session_no;
    let double = s.preset_data_packet_double();
    let packet = vec![
        0x1d, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt, 0x00, 0x04,
        session, double[0], double[1], 0x00,
        0x01, 0x00, 0x06, 0x00,
        0x0d, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, cd_variant,
        tag, 0x64, 0x4e, 0x65,
        0x82, 0x62, slot_bus, 0x1a,
        0x00, 0x00, 0x00, 0x00,
    ];

    let include_dump = include_chain_from_dump.unwrap_or(true);
    let chain_json = if include_dump {
        let ready = s.preset_data_ready && !s.preset_data.is_empty();
        let idx_ok = s.preset_index == active_preset;
        let mut snap = serde_json::json!({
            "presetDataReady": ready,
            "presetIndexMatchesActive": idx_ok,
            "segmentLen": serde_json::Value::Null,
            "chainParamCount": serde_json::Value::Null,
            "parseOk": false,
        });
        if ready && idx_ok {
            if let Some(seg) = kempline_assignable_segment_bytes(&s.preset_data, slot_index as usize)
            {
                snap["segmentLen"] = serde_json::json!(seg.len());
                if let Some(vals) = chain_param_values_for_assignable_segment(seg) {
                    snap["chainParamCount"] = serde_json::json!(vals.len());
                    snap["parseOk"] = serde_json::json!(true);
                }
            }
        }
        Some(snap)
    } else {
        None
    };

    s.send(OutPacket::new(packet.clone()));
    drop(s);

    let out_hex = packet
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!(
        "[SlotFocusProbe][sent] style={} slot_index={} slot_bus={:02x} cd={:02x} tag={:02x} len={} {}",
        style_key,
        slot_index,
        slot_bus,
        cd_variant,
        tag,
        packet.len(),
        out_hex
    );

    let mut root = serde_json::json!({
        "slotIndex": slot_index,
        "slotBus": slot_bus,
        "style": style_key,
        "cdVariant": cd_variant,
        "tagByte": tag,
        "outPacketLen": packet.len(),
        "outPacketHex": out_hex,
        "inTrafficHint": "Le flux IN 0x81 est lu par usb_listener : activer set_usb_trace_enabled et/ou set_usb_io_diag pour voir les réponses dans le terminal.",
    });
    if let Some(c) = chain_json {
        root["chainFromDump"] = c;
    }
    Ok(root)
}

/// **Étape 1** — lecture USB « focus slot » (même OUT que captures HX Edit `83:66:cd:04`) puis
/// collecte des IN `0x81` sur une fenêtre courte (remplie par `usb_listener`).
/// Ne modifie pas `preset_data` : sert à valider le trafic ; l’UI continue d’utiliser le dump preset
/// au chargement + merge RAM sur changement de slot.
#[tauri::command]
fn sync_hardware_slot_focus_usb(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
) -> Result<Value, String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state
            .clone()
            .ok_or_else(|| "HX non connecté".to_string())?
    };
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or_else(|| "slotIndex invalide".to_string())?;

    const CAPTURE_MS: u64 = 130;
    let t0 = std::time::Instant::now();
    let out_hex: String = {
        let mut s = helix_arc.lock().unwrap();
        if !s.connected {
            return Err("HX non connecté".to_string());
        }
        if s.preset_content_only {
            return Err("sync slot ignorée pendant lecture preset (preset_content_only)".to_string());
        }
        let cnt = s.next_x80_cnt();
        let session = s.session_no;
        let double = s.preset_data_packet_double();
        let packet = vec![
            0x1d, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, cnt, 0x00, 0x04,
            session, double[0], double[1], 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x0d, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x04,
            slot_bus, 0x64, 0x4e, 0x65,
            0x82, 0x62, slot_bus, 0x1a,
            0x00, 0x00, 0x00, 0x00,
        ];
        let out_hex = packet
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");
        s.usb_slot_focus_capture.clear();
        s.usb_slot_focus_capture_deadline =
            Some(std::time::Instant::now() + Duration::from_millis(CAPTURE_MS));
        s.send(OutPacket::new(packet));
        out_hex
    };

    thread::sleep(Duration::from_millis(CAPTURE_MS.saturating_add(20)));

    let mut frames: Vec<Vec<u8>> = Vec::new();
    {
        let mut s = helix_arc.lock().unwrap();
        s.usb_slot_focus_capture_deadline = None;
        std::mem::swap(&mut frames, &mut s.usb_slot_focus_capture);
    }

    let frames_hex: Vec<String> = frames
        .iter()
        .map(|f| {
            f.iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();

    if preset_debug_verbose_enabled() {
        eprintln!(
            "[SlotFocusSync] slot_index={} slot_bus={:02x} in_frames={} out={}",
            slot_index,
            slot_bus,
            frames_hex.len(),
            out_hex
        );
    }

    Ok(serde_json::json!({
        "slotIndex": slot_index,
        "slotBus": slot_bus,
        "outHex": out_hex,
        "captureWindowMs": CAPTURE_MS,
        "waitElapsedMs": t0.elapsed().as_millis() as u64,
        "inFrameCount": frames_hex.len(),
        "inFramesHex": frames_hex,
    }))
}

/// Sonde USB « assignation de modèle dans un slot » (captures HX Edit, best-effort).
/// `op` : `add` (slot vide, bulk 56 o ancienne capture) ou `replace` (slot occupé, bulk **48 o**
/// `83:66:cd:04` + courts `80:10`, voir Preset33 mai 2026 — `slot_bus` + octet voie `2*slotIndex`).
/// `catalogModelId` + `assignVariant` (`mono` | `stereo` | `legacy`) : si une entrée existe dans
/// `HX_ModelUsbAssign.json`, le bulk capturé est utilisé (courts ED alignés sur le bulk).
/// **Test historique** : `HX_SLOT_PROBE_USE_CHANGE_MODEL_TEST_BULK=1` force le bulk unique issu de
/// `src/Paquets Json/Change Model HXEdit.json` (même comportement que les premiers tests UI).
/// Sinon `catalogModelId` sert à la fusion **chainHex long** (`83 66 cd …`) comme avant.
/// Sans `catalogModelId` en **replace**, le bulk reste le template embarqué (sonde seule).
/// Avec `catalogModelId`, une entrée JSON est **requise** (plus de repli sur le
/// template unique type `cd01fe` pour tout le catalogue), sauf si la variable de test ci-dessus est active.
#[tauri::command]
fn probe_slot_model_usb(
    state: tauri::State<Arc<Mutex<AppState>>>,
    op: String,
    slot_index: u32,
    catalog_model_id: Option<String>,
    assign_variant: Option<String>,
) -> Result<String, String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or_else(|| "slotIndex invalide".to_string())?;
    let probe_op = match op.trim().to_ascii_lowercase().as_str() {
        "add" | "empty" => SlotModelProbeOp::AddToEmpty,
        "replace" | "change" => SlotModelProbeOp::ReplaceOccupied,
        "remove" | "delete" | "clear" => SlotModelProbeOp::RemoveFromOccupied,
        other => {
            return Err(format!("op inconnu: {other} (attendu: add | replace | remove)"));
        }
    };
    let id_for_log = catalog_model_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let variant_raw = assign_variant
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "mono".to_string());
    let variant_lc = variant_raw.to_ascii_lowercase();
    let use_change_model_test_bulk = matches!(probe_op, SlotModelProbeOp::ReplaceOccupied)
        && slot_probe_use_change_model_test_bulk();
    let usb_bulk_from_json: Option<Vec<u8>> = if use_change_model_test_bulk {
        Some(change_model_hxedit_replace_test_bulk())
    } else {
        id_for_log
            .as_deref()
            .and_then(|id| resolve_usb_assign_bulk(id, &variant_lc))
    };

    // Ne plus envoyer le template fixe (ex. `cd01fe`) pour tout le catalogue : si l’utilisateur
    // choisit un modèle (add/replace + id) sans entrée `HX_ModelUsbAssign.json`, on refuse.
    if id_for_log.is_some()
        && usb_bulk_from_json.is_none()
        && !use_change_model_test_bulk
    {
        return Err(format!(
            "Pas d'entrée dans HX_ModelUsbAssign.json pour id {:?} et variant {:?} — l'ancien template unique de test n'est plus envoyé.",
            id_for_log,
            variant_lc
        ));
    }

    let chain_bytes: Option<Vec<u8>> = if usb_bulk_from_json.is_some() {
        None
    } else {
        id_for_log
            .as_deref()
            .and_then(resolve_catalog_model_chain_bytes)
    };

    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    }
    .ok_or_else(|| "HX non connecté".to_string())?;

    let mut s = helix_arc.lock().unwrap();
    if !s.connected || s.preset_content_only {
        return Err(
            "probe_slot_model_usb ignoré (HX non prêt ou lecture preset en cours)".to_string(),
        );
    }
    let packs = build_slot_model_probe_packets(
        &mut s,
        probe_op,
        slot_index as usize,
        slot_bus,
        chain_bytes.as_deref(),
        usb_bulk_from_json.as_deref(),
    );
    let mono_0310_json_timing_strict = usb_bulk_from_json
        .as_deref()
        .is_some_and(|b| b.len() >= 8 && b[4] == 0x03 && b[5] == 0x10 && b[6] == 0xed && b[7] == 0x03);

    let mut lines: Vec<String> = Vec::new();
    for (i, p) in packs.iter().enumerate() {
        let delay = if mono_0310_json_timing_strict {
            0u64
        } else if i == 0 {
            0u64
        } else {
            8u64
        };
        s.send(OutPacket::with_delay(p.clone(), delay));
        let hx: String = p.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
        lines.push(format!("#{i} len={} delay_ms={} {}", p.len(), delay, hx));
    }
    drop(s);

    let summary = lines.join(" | ");
    eprintln!(
        "[SlotModelProbe] op={:?} slot_index={} slot_bus={:#04x} catalog_id={:?} variant={} usb_json={} change_model_test_bulk={} {}",
        probe_op,
        slot_index,
        slot_bus,
        id_for_log,
        variant_lc,
        usb_bulk_from_json.is_some(),
        use_change_model_test_bulk,
        summary
    );
    Ok(summary)
}

/// Envoie un Control Change MIDI USB (endpoint 0x02).
/// Nécessite un Controller Assign côté preset HX pour agir sur un paramètre.
#[tauri::command]
fn write_live_param_midi_cc(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
    param_index: u32,
    symbolic_id: String,
    display_type: Option<String>,
    raw_value: f64,
    midi_channel: u8,
    cc_number: u8,
) -> Result<(), String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    if !raw_value.is_finite() {
        return Err("rawValue invalide".to_string());
    }
    if midi_channel > 15 {
        return Err("midiChannel hors plage (0..15)".to_string());
    }
    if cc_number > 127 {
        return Err("ccNumber hors plage (0..127)".to_string());
    }

    let handle = {
        let app = state.lock().unwrap();
        app.usb_handle.clone().ok_or("HX non connecté")?
    };

    let sid = symbolic_id.trim();
    let raw = raw_value.clamp(0.0, 1.0);
    let cc_value = ((raw * 127.0).round() as i64).clamp(0, 127) as u8;
    // USB-MIDI Event Packet:
    // CIN=0xB (Control Change), status 0xBn, controller, value.
    let status = 0xB0u8 | midi_channel;
    let packet = [0x0Bu8, status, cc_number, cc_value];
    handle
        .write_bulk(0x02, &packet, Duration::from_millis(1000))
        .map_err(|e| format!("Erreur MIDI CC : {}", e))?;

    eprintln!(
        "[LiveWriteMidiCC][sent] slot={} param={} symbolicId={} displayType={} rawValue={} channel={} cc={} value={} packet={:02x} {:02x} {:02x} {:02x}",
        slot_index,
        param_index,
        sid,
        display_type.unwrap_or_default(),
        raw,
        midi_channel,
        cc_number,
        cc_value,
        packet[0],
        packet[1],
        packet[2],
        packet[3]
    );
    Ok(())
}

/// Sonde d'écriture live : **log uniquement**, aucun paquet USB (contraste avec `write_live_param`).
/// Déclenchée côté UI quand `models_live_write_probe === "1"` et write non activé.
#[tauri::command]
fn probe_live_param_write(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
    param_index: u32,
    symbolic_id: String,
    display_type: Option<String>,
    raw_value: f64,
) -> Result<(), String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    if !raw_value.is_finite() {
        return Err("rawValue invalide".to_string());
    }
    let has_hx = {
        let app = state.lock().unwrap();
        app.helix_state.is_some()
    };
    if !has_hx {
        return Err("HX non connecté".to_string());
    }
    eprintln!(
        "[LiveWriteProbe] slot={} param={} symbolicId={} displayType={} rawValue={}",
        slot_index,
        param_index,
        symbolic_id.trim(),
        display_type.unwrap_or_default(),
        raw_value
    );
    Ok(())
}

/// Écriture live USB paramètre : **seul** chemin d’envoi « valeur bloc » depuis le panneau models
/// (bool → trame `23`, float → `27` ; voir `helix/live_write.rs` + `HelixLiveWrite.json`).
/// Alternative : `write_live_param_midi_cc` si transport `midi_cc`. **Ne pas** dupliquer d’envoi ED03 ailleurs pour ce cas d’usage.
/// `value_type` : Line 6 `.models` (ex. 2 = bool) — route `23` vs `27`.
/// `chain_min` / `chain_max` : `.models` min/max ; 2ᵉ jambe float `27` = valeur physique (ex. dB) si les deux sont fournis et max > min.
#[tauri::command]
fn write_live_param(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
    param_index: u32,
    symbolic_id: String,
    display_type: Option<String>,
    value_type: Option<i32>,
    raw_value: f64,
    chain_min: Option<f64>,
    chain_max: Option<f64>,
) -> Result<(), String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    if !raw_value.is_finite() {
        return Err("rawValue invalide".to_string());
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;

    let sid = symbolic_id.trim();
    let dt = display_type.as_deref().map(str::trim);
    let vt = value_type;

    validate_usb_live_write_metadata(dt, vt)?;

    // Utilise la valeur réelle du slider, bornée dans l'intervalle machine attendu.
    let raw = raw_value.clamp(0.0, 1.0) as f32;
    // Trames calquées sur capture USBPcap HX Edit.
    // Cette construction est déléguée à `helix/live_write.rs` pour itérer
    // rapidement sur le protocole reverse-engineered sans gonfler `lib.rs`.
    let mut s = helix_arc.lock().unwrap();
    let frames = build_live_write_frames_from_state(
        &mut s,
        raw,
        slot_index,
        param_index,
        sid,
        dt,
        vt,
        chain_min,
        chain_max,
    );

    s.send(OutPacket::new(frames.pre_packet_x80.clone()));
    s.send(OutPacket::with_delay(frames.pre_packet_x2.clone(), 4));
    s.send(OutPacket::with_delay(frames.pre_packet_x80_sel.clone(), 8));
    s.send(OutPacket::with_delay(frames.packet_27.clone(), 12));
    // Deuxième jambe HX Edit (octet 11 = 0x0c), CTR/SEQ déjà avancés dans le builder.
    s.send(OutPacket::with_delay(frames.packet_27_b.clone(), 8));
    s.send(OutPacket::with_delay(frames.post_packet_x80_sel.clone(), 8));

    drop(s);

    let leg_b = match (chain_min, chain_max) {
        (Some(lo), Some(hi)) if hi > lo && lo.is_finite() && hi.is_finite() => {
            lo + f64::from(raw) * (hi - lo)
        }
        _ => f64::from(raw),
    };
    eprintln!(
        "[LiveWrite][sent] slot={} param={} symbolicId={} displayType={} valueType={:?} opcode={:02x} rawValue={} sentRaw={} legBChain={} chainMin={:?} chainMax={:?} slotBus={:02x} pp={:02x} ppSource={} pSel={:02x} pSelSource={} model_block={} frame_diff={} pre_x80={} pre_x2={} pre_x80_sel={} frame_a={} frame_b={} post_x80_sel={}",
        slot_index,
        param_index,
        sid,
        display_type.unwrap_or_default(),
        value_type,
        frames.primary_opcode,
        raw_value,
        raw,
        leg_b,
        chain_min,
        chain_max,
        frames.slot_bus,
        frames.pp,
        frames.pp_source,
        frames.param_selector,
        frames.param_selector_source,
        frames.model_block_kind,
        frames.frame27_diff_vs_static,
        frames.pre_packet_x80.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "),
        frames.pre_packet_x2.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "),
        frames.pre_packet_x80_sel.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "),
        frames.packet_27.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "),
        frames.packet_27_b.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "),
        frames.post_packet_x80_sel.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ")
    );
    Ok(())
}

#[tauri::command]
fn set_usb_trace_enabled(enabled: bool) -> Result<(), String> {
    set_usb_packet_trace_enabled(enabled);
    eprintln!(
        "[UsbTrace] packet tracing {} (delta_only={})",
        if enabled { "enabled" } else { "disabled" },
        usb_packet_trace_delta_only()
    );
    Ok(())
}

#[tauri::command]
fn set_usb_trace_delta_only(enabled: bool) -> Result<(), String> {
    set_usb_packet_trace_delta_only(enabled);
    eprintln!(
        "[UsbTrace] delta-only tracing {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

#[tauri::command]
fn set_preset_debug_verbose(enabled: bool) -> Result<(), String> {
    set_preset_debug_verbose_enabled(enabled);
    eprintln!(
        "[PresetDebug] verbose logging {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

#[tauri::command]
fn set_usb_io_diag(enabled: bool) -> Result<(), String> {
    set_usb_io_diag_enabled(enabled);
    eprintln!(
        "[UsbIODiag] {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

/// Relais de log frontend -> terminal Rust (`cargo tauri dev`, stderr).
#[tauri::command]
fn log_frontend_message(message: String) -> Result<(), String> {
    let m = message.trim();
    if m.is_empty() {
        return Ok(());
    }
    eprintln!("{m}");
    let _ = io::stderr().flush();
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
        let mut app = state.lock().unwrap();
        let now = Instant::now();
        if let Some(last) = app.last_preset_request_at {
            let elapsed_ms = now.duration_since(last).as_millis() as u64;
            if elapsed_ms < PRESET_REQUEST_MIN_GAP_MS {
                if elapsed_ms >= PRESET_RECOVER_MIN_REQUEST_AGE_MS {
                    app.preset_recover_in_flight = false;
                }
                return Ok(());
            }
        }
        (
            app.active_preset,
            app.helix_state.clone().ok_or("HX non connecté")?,
        )
    };
    let mut s = helix_arc.lock().unwrap();
    if s.preset_content_only {
        return Ok(());
    }
    // L'UI met à jour `active_preset` (ex. après `activate_preset` + MIDI PC) avant cette
    // commande, alors que `preset_index` côté Helix ne bouge qu'avec les paquets USB x2 ou
    // l'écoute MIDI — parfois après le dump, ou jamais si `RequestPresetName` est ignoré
    // pendant `preset_content_only`. Sans cette ligne, `get_active_preset_slots` reste à
    // None et la fenêtre models timeoute.
    s.preset_index = active_preset;
    s.preset_data_ready = false;
    s.preset_data.clear();
    // preset_content_only=true dans les deux cas : bloque les RequestPresetName
    // déclenchés par le MIDI listener pendant qu'on attend le x2 de confirmation.
    s.preset_content_only = true;
    if s.want_content_only_after_x2 {
        // activate_preset a envoyé le MIDI PC et posé le flag.
        // On attend le x2 du hardware pour lancer cd:03 — garantit que tous
        // les x2 sont ACKés avant la lecture (même séquence que HXEdit).
    } else {
        // Appel direct sans activate_preset (ex: force-recover) → déclenchement immédiat.
        s.switch_mode(ModeRequest::RequestPreset(true));
    }
    {
        let mut app = state.lock().unwrap();
        app.preset_recover_in_flight = false;
        app.last_preset_request_at = Some(Instant::now());
    }
    Ok(())
}

/// Récupération "hard" quand la lecture preset reste bloquée trop longtemps.
/// Force la sortie de `preset_content_only`, remet les compteurs de requête,
/// puis revient en mode Standard pour permettre une relance propre côté front.
#[tauri::command]
fn force_recover_preset_reader(state: tauri::State<Arc<Mutex<AppState>>>) -> Result<(), String> {
    let (helix_arc, skip_reason) = {
        let mut app = state.lock().unwrap();
        let now = Instant::now();
        let mut skip_reason: Option<&'static str> = None;
        if app.preset_recover_in_flight {
            skip_reason = Some("recover already in flight");
        } else if let Some(last) = app.last_preset_recover_at {
            let elapsed_ms = now.duration_since(last).as_millis() as u64;
            if elapsed_ms < PRESET_RECOVER_COOLDOWN_MS {
                skip_reason = Some("recover cooldown active");
            }
        }
        if skip_reason.is_none() {
            if let Some(last_req) = app.last_preset_request_at {
                let req_age_ms = now.duration_since(last_req).as_millis() as u64;
                if req_age_ms < PRESET_RECOVER_MIN_REQUEST_AGE_MS {
                    skip_reason = Some("request too recent");
                }
            }
        }
        if skip_reason.is_none() {
            app.preset_recover_in_flight = true;
            app.last_preset_recover_at = Some(now);
        }
        (
            app.helix_state.clone().ok_or("HX non connecté")?,
            skip_reason.map(str::to_string),
        )
    };
    if let Some(reason) = skip_reason {
        eprintln!("[PresetDebug][recover] ignored: {}", reason);
        return Ok(());
    }
    {
        let mut s = helix_arc.lock().unwrap();
        if !s.preset_content_only && s.preset_data_ready {
            drop(s);
            let mut app = state.lock().unwrap();
            app.preset_recover_in_flight = false;
            eprintln!("[PresetDebug][recover] ignored: no content_only session");
            return Ok(());
        }
        s.preset_content_only = false;
        s.preset_data_ready = false;
        s.preset_data.clear();
        // Re-synchronise l'état de requête (mêmes valeurs de base que RequestPreset::shutdown no-data).
        s.preset_pkt_counter = 0x001e;
        s.request_preset_session_id = 0xf4;
        s.new_session_no();
        s.switch_mode(ModeRequest::Standard);
        // Libérer le verrou helix AVANT de re-locker AppState pour éviter toute contention
        // avec la boucle de modes qui tient les deux verrous dans le même ordre.
    }
    {
        let mut app = state.lock().unwrap();
        app.preset_recover_in_flight = false;
    }
    eprintln!("[PresetDebug][recover] force_recover_preset_reader applied");
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
) -> Option<Vec<[String; 3]>> {
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
    let markers: Vec<[String; 3]> = parse_preset_slots_internal(&s.preset_data)
        .into_iter()
        .filter(|p| {
            let c = p.category.to_ascii_lowercase();
            let n = p.name.to_ascii_lowercase();
            c == "routing" || c == "split" || c == "merge" || n.contains("split") || n.contains("merge")
        })
        .map(|p| [p.category, p.name, p.module_hex])
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

/// Valeurs de chaîne des segments I/O Path 1 (Input/Output) dans la fenêtre Kempline.
/// `io_kind`: "input" | "output" (tolérance: "main", "main l/r", "mainlr").
#[tauri::command]
fn get_active_preset_path1_io_chain_param_values(
    state: tauri::State<Arc<Mutex<AppState>>>,
    io_kind: String,
) -> Option<Vec<preset_chain_params::ChainParamValue>> {
    let kind = io_kind.trim().to_ascii_lowercase();
    let seg_idx_in_window = if kind == "input" {
        0usize
    } else if kind == "output" || kind == "main" || kind == "main l/r" || kind == "mainlr" {
        9usize
    } else {
        return None;
    };
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
    let (start, _) = kempline_grid_window_start_and_seg_count(&s.preset_data)?;
    let segs = split_preset_by_8213(&s.preset_data);
    let abs = start.checked_add(seg_idx_in_window)?;
    let seg = segs.get(abs).copied()?;
    preset_chain_params::parse_flow_io_segment_params(seg)
}

/// Valeurs de chaîne des segments **flow** Kempline (Split / Merge) dans la fenêtre 20 segments.
/// `flow_kind`: `split` (segment `0x02`, index 10) ou `merge` / `mixer` (segment `0x03`, index 19).
#[tauri::command]
fn get_active_preset_kempline_flow_chain_param_values(
    state: tauri::State<Arc<Mutex<AppState>>>,
    flow_kind: String,
) -> Option<Vec<preset_chain_params::ChainParamValue>> {
    let kind = flow_kind.trim().to_ascii_lowercase();
    let seg_idx_in_window = if kind == "split" {
        10usize
    } else if kind == "merge" || kind == "mixer" || kind == "join" {
        19usize
    } else {
        return None;
    };
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
    let (start, _) = kempline_grid_window_start_and_seg_count(&s.preset_data)?;
    let segs = split_preset_by_8213(&s.preset_data);
    let abs = start.checked_add(seg_idx_in_window)?;
    let seg = segs.get(abs).copied()?;
    preset_chain_params::parse_flow_io_segment_params(seg)
}

/// JSON de debug pour un slot Kempline : segment assignable brut (hex) + décodage `read_params`
/// (même logique que `get_active_preset_slot_chain_param_values`) + métadonnées grille.
/// Utile pour analyser des amplis « riches » (`chainHex` ex. cd0207) sans sniffer le port USB réseau
/// (il n’y en a pas : le flux est bulk IN/OUT continu côté `rusb`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivePresetSlotAssignableUsbJson {
    slot_index: u32,
    slot_category: Option<String>,
    slot_name: Option<String>,
    module_hex: Option<String>,
    segment_byte_len: usize,
    segment_hex: String,
    /// Nombre de valeurs par bloc `c219` après `85188317` (plusieurs blocs = ampli / effet complexe).
    param_block_sizes: Option<Vec<usize>>,
    chain_param_values: Option<Vec<preset_chain_params::ChainParamValue>>,
}

#[tauri::command]
fn get_active_preset_slot_assignable_usb_json(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
) -> Option<ActivePresetSlotAssignableUsbJson> {
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
    let seg_owned = seg.to_vec();
    let mut segment_hex = String::with_capacity(seg_owned.len() * 2);
    for b in &seg_owned {
        let _ = write!(&mut segment_hex, "{:02x}", b);
    }
    let (slot_category, slot_name, module_hex) =
        try_parse_preset_kempline_grid(&s.preset_data).map_or((None, None, None), |grid| {
            grid.get(slot_index as usize)
                .map(|cell| {
                    (
                        Some(cell.category.clone()),
                        Some(cell.name.clone()),
                        Some(cell.module_hex.clone()).filter(|h| !h.is_empty()),
                    )
                })
                .unwrap_or((None, None, None))
        });
    let param_block_sizes = preset_chain_params::parse_assignable_segment_param_blocks(&seg_owned)
        .map(|blocks| blocks.iter().map(|b| b.len()).collect());
    let chain_param_values = chain_param_values_for_assignable_segment(&seg_owned);
    Some(ActivePresetSlotAssignableUsbJson {
        slot_index,
        slot_category,
        slot_name,
        module_hex,
        segment_byte_len: seg_owned.len(),
        segment_hex,
        param_block_sizes,
        chain_param_values,
    })
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
    let blocks = preset_chain_params::parse_assignable_segment_param_blocks(&seg)?;
    let ids = augmented_module_ids_for_assignable_chunk(seg, blocks.len());
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

    let blocks = preset_chain_params::parse_assignable_segment_param_blocks(seg)?;
    let ids = augmented_module_ids_for_assignable_chunk(seg, blocks.len());

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
                // Plusieurs `c219` sans combinaison Amp+Cab : concaténer les valeurs dans l’ordre
                // (amplis / effets avec plusieurs blocs `read_params` dans le même segment).
                let mut merged: Vec<preset_chain_params::ChainParamValue> =
                    Vec::with_capacity(blocks.iter().map(|b| b.len()).sum());
                for b in &blocks {
                    merged.extend_from_slice(b);
                }
                return Some(merged);
            }
            let ids = augmented_module_ids_for_assignable_chunk(seg, blocks.len());
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

    fn infer_flow_split_chain_hex(chunk: &[u8]) -> Option<String> {
        if chunk.first().copied() != Some(0x02) {
            return None;
        }
        // Cas direct observé sur certains dumps de flow : signature `6c cd 00 XX`.
        for w in chunk.windows(4) {
            if w[0] == 0x6c && w[1] == 0xcd && w[2] == 0x00 {
                return Some(format!("6ccd00{:02x}", w[3]));
            }
        }
        // Variante compacte dans le segment split `02`: `cd 01 vv` ou `cd 02 33`.
        let body = chunk.get(1..).unwrap_or(&[]);
        for w in body.windows(3) {
            if w[0] != 0xcd {
                continue;
            }
            if w[1] == 0x01 {
                let mapped = match w[2] {
                    0x01 => Some("6ccd0023"), // Split Y
                    0x00 => Some("6ccd0024"), // Split A/B
                    0x02 => Some("6ccd0025"), // Split Crossover
                    _ => None,
                };
                if let Some(h) = mapped {
                    return Some(h.to_string());
                }
            } else if w[1] == 0x02 && w[2] == 0x33 {
                return Some(String::from("6ccd0026")); // Split Dynamic
            }
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
        let parsed_len_before = parsed_slots.len();

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

        if parsed_slots.len() == parsed_len_before {
            if let Some(split_hex) = infer_flow_split_chain_hex(chunk) {
                if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(&split_hex) {
                    parsed_slots.push(ParsedSlot {
                        category: entry[0].clone(),
                        name: entry[1].clone(),
                        grid_x: None,
                        grid_y: None,
                        module_hex: split_hex,
                    });
                }
            }
        }

        if let Some(slot) = best_unknown {
            parsed_slots.push(slot);
        } else if parsed_slots.len() == parsed_len_before
            && is_assignable_chunk
            && is_amp_cab_assignable_chunk(chunk)
        {
            // Même logique que la grille Kempline : Amp+Cab sans `19…1a` catalogue dans ce segment.
            let inferred = extract_first_module_from_assignable_chunk(chunk);
            if !inferred.module_hex.is_empty() {
                parsed_slots.push(inferred);
            }
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

/// `true` si le segment porte le couple **85188317 + c319** (blocs `c219` ampli puis cab).
/// Même convention que la fenêtre Kempline / `parse_assignable_segment_param_blocks` : premier octet
/// **`0x06` ou `0x08`** (les dumps USB / slots réels utilisent souvent **`0x08`** — ex. *Paquet Amp+Cab sans EQ*).
fn is_amp_cab_assignable_chunk(chunk: &[u8]) -> bool {
    if !matches!(chunk.first().copied(), Some(0x06 | 0x08)) {
        return false;
    }
    // Signature historique explicite : `85188317c319`.
    if chunk[1..]
        .windows(AMP_CAB_MARKER.len())
        .any(|w| w == AMP_CAB_MARKER)
    {
        return true;
    }
    // Tolérance firmware/dumps : certains segments Amp+Cab n'exposent pas `c319` mais
    // conservent deux blocs `c219` dont les types se classent `AmpLike` puis `CabLike`.
    segment_has_amp_then_cab_c219_signature(chunk)
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
        // Faux positif Amp+Cab uniquement : fin du marqueur `85 18 83 17 c3 19` — l’octet `0x19` est le 2ᵉ
        // octet de l’opcode `0xc319`, pas le préfixe `0x19` d’un ID module. Ne pas élargir à tout `c3`/`c2`
        // précédent un `0x19` : `0xc3`/`0xc2` encodent aussi des booléens dans `read_params`.
        if cursor >= 3
            && chunk[cursor - 3] == 0x83
            && chunk[cursor - 2] == 0x17
            && chunk[cursor - 1] == 0xc3
            && chunk[cursor] == 0x19
        {
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

fn assignable_segment_hex_lower_body(seg: &[u8]) -> String {
    let mut s = String::with_capacity(seg.len().saturating_sub(1) * 2);
    for b in seg.get(1..).unwrap_or(&[]) {
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

/// Détection Amp+Cab sans marqueur `c319` :
/// au moins deux types `c219`, avec un `AmpLike` suivi d'un `CabLike`.
fn segment_has_amp_then_cab_c219_signature(seg: &[u8]) -> bool {
    if !matches!(seg.first().copied(), Some(0x06 | 0x08)) {
        return false;
    }
    let h = assignable_segment_hex_lower_body(seg);
    let types = extract_c219_argument_type_hexes(&h);
    if types.len() < 2 {
        return false;
    }
    let keys: Vec<String> = types
        .iter()
        .map(|t| chain_hex_key_from_c219_argument_type(t))
        .collect();
    for (i, amp_key) in keys.iter().enumerate() {
        if !matches!(
            catalog_slot_kind_for_chain_hex(amp_key),
            CatalogSlotKind::AmpLike
        ) {
            continue;
        }
        for cab_key in keys.iter().skip(i + 1) {
            if matches!(
                catalog_slot_kind_for_chain_hex(cab_key),
                CatalogSlotKind::CabLike
            ) {
                return true;
            }
        }
    }
    // Cas catalogue où les parties individuelles sont peu classées, mais la combinaison est connue.
    for (i, first) in keys.iter().enumerate() {
        for second in keys.iter().skip(i + 1) {
            let forward = format!("{first}1a{second}");
            if HX_CATALOG_MODULE_BY_HEX
                .get(&forward)
                .map(|e| e[0].eq_ignore_ascii_case("amp+cab"))
                .unwrap_or(false)
            {
                return true;
            }
            let reverse = format!("{second}1a{first}");
            if HX_CATALOG_MODULE_BY_HEX
                .get(&reverse)
                .map(|e| e[0].eq_ignore_ascii_case("amp+cab"))
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

/// Chaîne hex (sans le premier octet du segment) : même convention que `preset_chain_params::hex_lower(&seg[1..])`.
fn extract_c219_argument_type_hexes(h: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = h.get(search..).and_then(|s| s.find("c219")) {
        let start = search + rel;
        let Some(slice) = h.get(start..) else {
            break;
        };
        let Some(rel09) = slice.find("09").filter(|&p| p >= 4) else {
            search = start.saturating_add(4);
            continue;
        };
        if let Some(t) = slice.get(4..rel09) {
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
        search = start + rel09;
    }
    out
}

/// Préfixe `chainHex` catalogue le plus courant : 6 caractères hex (3 octets), ex. `cd0217`.
fn chain_hex_key_from_c219_argument_type(t: &str) -> String {
    let s = t.trim().to_ascii_lowercase();
    if s.len() >= 6 && s[..6].chars().all(|c| c.is_ascii_hexdigit()) {
        return s[..6].to_string();
    }
    s
}

/// Clé `chainHex` candidate à partir d'un champ binaire brut (`0x19...0x1a` ou `0x1a...0x09`).
/// Priorité: exact catalogue -> préfixe 6 hex (convention majoritaire).
fn chain_hex_key_from_raw_field_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let mut full = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut full, "{:02x}", b);
    }
    let full = full.trim().to_ascii_lowercase();
    if full.is_empty() {
        return None;
    }
    if HX_CATALOG_MODULE_BY_HEX.contains_key(&full) {
        return Some(full);
    }
    if full.len() >= 6 && full[..6].chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(full[..6].to_string());
    }
    Some(full)
}

/// Fallback Amp+Cab depuis les marqueurs "dualslot" vus côté Kempline :
/// `0x19 <amp_field> 0x1a <cab_field> 0x09`.
fn infer_amp_cab_hex_pair_from_19_1a_09_markers(chunk: &[u8]) -> Option<(String, String)> {
    if !matches!(chunk.first().copied(), Some(0x06 | 0x08)) {
        return None;
    }
    let mut cursor = 0usize;
    while cursor < chunk.len() {
        if chunk[cursor] != 0x19 {
            cursor += 1;
            continue;
        }
        let amp_start = cursor + 1;
        let Some(rel_sep) = chunk.get(amp_start..)?.iter().position(|&b| b == 0x1a) else {
            cursor += 1;
            continue;
        };
        let amp_end = amp_start + rel_sep;
        let cab_start = amp_end + 1;
        let Some(rel_end09) = chunk.get(cab_start..)?.iter().position(|&b| b == 0x09) else {
            cursor = cab_start;
            continue;
        };
        let cab_end = cab_start + rel_end09;
        let amp_key = chain_hex_key_from_raw_field_bytes(&chunk[amp_start..amp_end])?;
        let cab_key = chain_hex_key_from_raw_field_bytes(&chunk[cab_start..cab_end])?;
        if amp_key.is_empty() || cab_key.is_empty() {
            cursor = cab_end.saturating_add(1);
            continue;
        }
        let amp_k = catalog_slot_kind_for_chain_hex(&amp_key);
        let cab_k = catalog_slot_kind_for_chain_hex(&cab_key);
        if matches!(amp_k, CatalogSlotKind::CabLike) && matches!(cab_k, CatalogSlotKind::AmpLike) {
            return Some((cab_key, amp_key));
        }
        if matches!(amp_k, CatalogSlotKind::AmpLike) && matches!(cab_k, CatalogSlotKind::CabLike) {
            return Some((amp_key, cab_key));
        }
        if HX_CATALOG_MODULE_BY_HEX.contains_key(&format!("{amp_key}1a{cab_key}")) {
            return Some((amp_key, cab_key));
        }
        if HX_CATALOG_MODULE_BY_HEX.contains_key(&format!("{cab_key}1a{amp_key}")) {
            return Some((cab_key, amp_key));
        }
        cursor = cab_end.saturating_add(1);
    }
    None
}

/// Module principal extrait d'une structure dual-slot `0x19 <A> 0x1a <B> 0x09`.
/// Retourne:
/// - `A1aB` si la combinaison existe au catalogue,
/// - sinon le premier champ (`A` ou `B`) reconnu au catalogue (préférence `B`, souvent utile sur ces dumps).
fn infer_module_hex_from_19_1a_09_fields(chunk: &[u8]) -> Option<String> {
    if !matches!(chunk.first().copied(), Some(0x06 | 0x08)) {
        return None;
    }
    let mut cursor = 0usize;
    while cursor < chunk.len() {
        if chunk[cursor] != 0x19 {
            cursor += 1;
            continue;
        }
        let a_start = cursor + 1;
        let Some(rel_sep) = chunk.get(a_start..)?.iter().position(|&b| b == 0x1a) else {
            cursor += 1;
            continue;
        };
        let a_end = a_start + rel_sep;
        let b_start = a_end + 1;
        let Some(rel_end09) = chunk.get(b_start..)?.iter().position(|&b| b == 0x09) else {
            cursor = b_start;
            continue;
        };
        let b_end = b_start + rel_end09;
        let a_key = chain_hex_key_from_raw_field_bytes(&chunk[a_start..a_end]);
        let b_key = chain_hex_key_from_raw_field_bytes(&chunk[b_start..b_end]);

        if let (Some(a), Some(b)) = (&a_key, &b_key) {
            let combined = format!("{a}1a{b}");
            if HX_CATALOG_MODULE_BY_HEX.contains_key(&combined) {
                return Some(combined);
            }
            let a_known = HX_CATALOG_MODULE_BY_HEX.contains_key(a);
            let b_known = HX_CATALOG_MODULE_BY_HEX.contains_key(b);
            let a_kind = catalog_slot_kind_for_chain_hex(a);
            let b_kind = catalog_slot_kind_for_chain_hex(b);
            // Priorité affichage slot principal: Amp/Preamp avant Cab/IR.
            if a_known
                && matches!(a_kind, CatalogSlotKind::AmpLike)
                && matches!(b_kind, CatalogSlotKind::CabLike)
            {
                return Some(a.clone());
            }
            if b_known
                && matches!(b_kind, CatalogSlotKind::AmpLike)
                && matches!(a_kind, CatalogSlotKind::CabLike)
            {
                return Some(b.clone());
            }
            if a_known && !matches!(a_kind, CatalogSlotKind::CabLike) && matches!(b_kind, CatalogSlotKind::CabLike) {
                return Some(a.clone());
            }
            if b_known && !matches!(b_kind, CatalogSlotKind::CabLike) && matches!(a_kind, CatalogSlotKind::CabLike) {
                return Some(b.clone());
            }
        }
        if let Some(a) = a_key {
            if HX_CATALOG_MODULE_BY_HEX.contains_key(&a) {
                return Some(a);
            }
            if a.len() >= 4 {
                return Some(a);
            }
        }
        if let Some(b) = b_key {
            if HX_CATALOG_MODULE_BY_HEX.contains_key(&b) {
                return Some(b);
            }
            if b.len() >= 4 {
                return Some(b);
            }
        }
        cursor = b_end.saturating_add(1);
    }
    None
}

/// Paire ampli + cab à partir des types d’argument de **tous** les `c219` visibles dans le corps hex du segment.
/// Plusieurs blocs `c219` (ampli + effet interne + cab, etc.) : premier type **AmpLike** catalogue puis premier **CabLike** après ;
/// si le catalogue ne classe pas, repli = premier et dernier préfixe 6 hex.
fn infer_amp_cab_hex_pair_from_segment_hex_body(h: &str) -> Option<(String, String)> {
    let types = extract_c219_argument_type_hexes(h);
    if types.len() < 2 {
        return None;
    }
    let keys: Vec<String> = types
        .iter()
        .map(|t| chain_hex_key_from_c219_argument_type(t))
        .collect();
    for (i, ak) in keys.iter().enumerate() {
        if matches!(
            catalog_slot_kind_for_chain_hex(ak),
            CatalogSlotKind::AmpLike
        ) {
            for ck in keys.iter().skip(i + 1) {
                if matches!(
                    catalog_slot_kind_for_chain_hex(ck),
                    CatalogSlotKind::CabLike
                ) {
                    return Some((ak.clone(), ck.clone()));
                }
            }
        }
    }
    Some((keys.first()?.clone(), keys.last()?.clone()))
}

/// IDs `19…1a` si présents ; sinon, segment **Amp+Cab** avec **au moins deux** blocs `c219` parsés :
/// infère `[hex_ampli, hex_cab]` depuis les types `c219` (y compris ampli + cab + blocs intermédiaires).
fn augmented_module_ids_for_assignable_chunk(seg: &[u8], blocks_len: usize) -> Vec<String> {
    let ids = extract_module_ids_from_assignable_chunk(seg);
    if !is_amp_cab_assignable_chunk(seg) || blocks_len < 2 {
        return ids;
    }
    let inferred_pair = infer_amp_cab_hex_pair_from_19_1a_09_markers(seg).or_else(|| {
        let h = assignable_segment_hex_lower_body(seg);
        infer_amp_cab_hex_pair_from_segment_hex_body(&h)
    });
    let Some((amp_k, cab_k)) = inferred_pair else {
        if !ids.is_empty() {
            return ids;
        }
        return Vec::new();
    };
    let inferred = vec![amp_k.clone(), cab_k.clone()];
    // Faux positif : l’octet `0x19` du couple d’opcode `c219` (`c2` + `19`) est lu comme début d’un ID `19…1a`,
    // souvent égal au seul type ampli des blocs — on préfère alors la paire inférée depuis les `c219`.
    if ids.len() == 1 && ids[0] == amp_k {
        return inferred;
    }
    if !ids.is_empty() {
        return ids;
    }
    inferred
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

/// Paire `(ampli, cab)` inférée : parse `c219` si possible, sinon types extraits du corps hex seul.
fn inferred_amp_cab_hex_keys(chunk: &[u8]) -> Option<(String, String)> {
    if !matches!(chunk.first().copied(), Some(0x06 | 0x08)) {
        return None;
    }
    let h = assignable_segment_hex_lower_body(chunk);
    let blocks_len = preset_chain_params::parse_assignable_segment_param_blocks(chunk)
        .map(|b| b.len())
        .unwrap_or(0);
    if is_amp_cab_assignable_chunk(chunk) && blocks_len >= 2 {
        let v = augmented_module_ids_for_assignable_chunk(chunk, blocks_len);
        if v.len() == 2 {
            return Some((v[0].clone(), v[1].clone()));
        }
    }
    if let Some(p) = infer_amp_cab_hex_pair_from_19_1a_09_markers(chunk) {
        return Some(p);
    }
    infer_amp_cab_hex_pair_from_segment_hex_body(&h)
}

/// `ampHex1acabHex` catalogue quand l’ID `19…1a` ne donne que l’ampli (faux positif `c219`) ou est absent.
fn amp_cab_combined_chain_hex_for_slot_if_better(chunk: &[u8], extracted_id_hex: &str) -> Option<String> {
    let (amp, cab) = inferred_amp_cab_hex_keys(chunk)?;
    let amp = amp.trim().to_ascii_lowercase();
    let cab = cab.trim().to_ascii_lowercase();
    if amp.is_empty() || cab.is_empty() {
        return None;
    }
    let combined = format!("{amp}1a{cab}");
    if !HX_CATALOG_MODULE_BY_HEX.contains_key(&combined) {
        return None;
    }
    let ext = extracted_id_hex.trim().to_ascii_lowercase();
    if ext.is_empty() || ext == amp {
        return Some(combined);
    }
    None
}

/// Premier module 0x19…0x1a dans un segment de slot assignable.
fn extract_first_module_from_assignable_chunk(chunk: &[u8]) -> ParsedSlot {
    let mut cursor = 0usize;
    let mut first_unknown_id_hex: Option<String> = None;
    while cursor < chunk.len() {
        if chunk[cursor] == 0x19 {
            if cursor >= 3
                && chunk[cursor - 3] == 0x83
                && chunk[cursor - 2] == 0x17
                && chunk[cursor - 1] == 0xc3
            {
                cursor += 1;
                continue;
            }
            let id_start = cursor + 1;
            if let Some(rel_end) = chunk[id_start..].iter().position(|&b| b == 0x1a) {
                let id_bytes = &chunk[id_start..id_start + rel_end];
                if !id_bytes.is_empty() {
                    let mut id_hex = String::with_capacity(id_bytes.len() * 2);
                    for b in id_bytes {
                        let _ = write!(&mut id_hex, "{:02x}", b);
                    }
                    let module_hex = amp_cab_combined_chain_hex_for_slot_if_better(chunk, &id_hex)
                        .unwrap_or_else(|| id_hex.clone());
                    let after = id_start + rel_end + 1;
                    // Cas dual-slot observé: `0x19 <ctrl/amp> 0x1a <id réel> 0x09`.
                    // Si un meilleur module est inféré depuis cette structure, on le privilégie.
                    let module_hex = if id_bytes.len() == 1 {
                        infer_module_hex_from_19_1a_09_fields(chunk).unwrap_or(module_hex)
                    } else {
                        module_hex
                    };
                    let (category, name) = if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(&module_hex) {
                        let e0 = entry[0].trim().to_ascii_lowercase();
                        let cat = if is_amp_cab_assignable_chunk(chunk)
                            && (e0 == "amp" || e0 == "preamp" || e0 == "amp+cab")
                        {
                            String::from("Amp+Cab")
                        } else {
                            entry[0].clone()
                        };
                        (cat, entry[1].clone())
                    } else {
                        // On ne retourne pas immédiatement: certains segments dual-slot portent
                        // d'abord un octet de contrôle (`06`) dans `0x19...0x1a`, puis l'ID réel
                        // dans la partie `0x1a...0x09`.
                        if first_unknown_id_hex.is_none() {
                            first_unknown_id_hex = Some(id_hex.clone());
                        }
                        (String::new(), String::new())
                    };
                    if !category.is_empty() || !name.is_empty() {
                        let (grid_x, grid_y) = extract_grid_xy_after_id(chunk, after);
                        return ParsedSlot {
                            category,
                            name,
                            grid_x,
                            grid_y,
                            module_hex,
                        };
                    }
                    cursor = after;
                    continue;
                }
                cursor = id_start + rel_end + 1;
                continue;
            }
        }
        cursor += 1;
    }
    // Repli grille : Amp+Cab sans ID `19…1a` utilisable.
    // 1) Essai via la détection Amp+Cab historique.
    // 2) Tolérance: essai direct sur les types `c219` (même sans marqueur `c319`), mais
    //    on ne retient le résultat que si le `chainHex` combiné existe dans le catalogue.
    let h = assignable_segment_hex_lower_body(chunk);
    let inferred_pair = inferred_amp_cab_hex_keys(chunk)
        .or_else(|| infer_amp_cab_hex_pair_from_segment_hex_body(&h));
    if let Some((amp, cab)) = inferred_pair {
        let amp = amp.trim().to_ascii_lowercase();
        let cab = cab.trim().to_ascii_lowercase();
        if !amp.is_empty() && !cab.is_empty() {
            let combined = format!("{amp}1a{cab}");
            if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(&combined) {
                let (grid_x, grid_y) = extract_grid_xy_after_id(chunk, 0);
                let e0 = entry[0].trim().to_ascii_lowercase();
                let category = if e0 == "amp" || e0 == "preamp" || e0 == "amp+cab" {
                    String::from("Amp+Cab")
                } else {
                    entry[0].clone()
                };
                return ParsedSlot {
                    category,
                    name: entry[1].clone(),
                    grid_x,
                    grid_y,
                    module_hex: combined,
                };
            }
        }
    }
    if let Some(module_hex) = infer_module_hex_from_19_1a_09_fields(chunk) {
        let (grid_x, grid_y) = extract_grid_xy_after_id(chunk, 0);
        if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(&module_hex) {
            let e0 = entry[0].trim().to_ascii_lowercase();
            let category = if is_amp_cab_assignable_chunk(chunk)
                && (e0 == "amp" || e0 == "preamp" || e0 == "amp+cab")
            {
                String::from("Amp+Cab")
            } else {
                entry[0].clone()
            };
            return ParsedSlot {
                category,
                name: entry[1].clone(),
                grid_x,
                grid_y,
                module_hex,
            };
        }
        return ParsedSlot {
            category: String::from("Unknown"),
            name: module_hex.clone(),
            grid_x,
            grid_y,
            module_hex,
        };
    }
    if let Some(id_hex) = first_unknown_id_hex {
        return ParsedSlot {
            category: String::from("Unknown"),
            name: id_hex.clone(),
            grid_x: None,
            grid_y: None,
            module_hex: id_hex,
        };
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
    let (start, _segs_len) = kempline_grid_window_start_and_seg_count(data)?;
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
    // Lire les flags de debug depuis l'environnement au démarrage.
    if std::env::var("PRESET_DEBUG_VERBOSE").map(|v| v == "1").unwrap_or(false) {
        set_preset_debug_verbose_enabled(true);
    }
    if std::env::var("USB_PACKET_TRACE").map(|v| v == "1").unwrap_or(false) {
        set_usb_packet_trace_enabled(true);
    }
    if std::env::var("USB_IO_DIAG").map(|v| v == "1").unwrap_or(false) {
        set_usb_io_diag_enabled(true);
    }

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
            get_active_hardware_slot_state,
            get_connected_device_name,
            get_connection_hint_text,
            request_active_preset_name,
            rename_preset,
            activate_preset,
            switch_active_hardware_slot,
            probe_hardware_slot_focus_usb,
            sync_hardware_slot_focus_usb,
            request_preset_content,
            force_recover_preset_reader,
            probe_live_param_write,
            probe_slot_model_usb,
            write_live_param,
            write_live_param_midi_cc,
            set_usb_trace_enabled,
            set_usb_trace_delta_only,
            set_preset_debug_verbose,
            set_usb_io_diag,
            log_frontend_message,
            get_preset_slots,
            get_active_preset_slots,
            get_active_preset_slots_debug,
            get_active_preset_routing_markers,
            get_active_preset_stomp_layout,
            get_active_preset_slot_chain_param_values,
            get_active_preset_path1_io_chain_param_values,
            get_active_preset_kempline_flow_chain_param_values,
            get_active_preset_slot_assignable_usb_json,
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
            let mut seen_fingerprints: HashSet<Vec<u8>> = HashSet::new();
            let mut suppressed_repeats: u64 = 0;
            loop {
                match handle_midi.read_bulk(0x82, &mut buf, Duration::from_millis(500)) {
                    Ok(n) if n >= 4 => {
                        {
                            let mut s = state_midi.lock().unwrap();
                            // Certains firmwares routent des paquets paramètre sur le flux MIDI USB (0x82).
                            // On tente l'ingestion ED03 ici aussi pour alimenter le mode live `in_echo_strict`.
                            s.ingest_ed03_param_echo(&buf[..n]);
                        }
                        if usb_packet_trace_enabled() {
                            let delta_only = usb_packet_trace_delta_only();
                            let data = buf[..n].to_vec();
                            let fingerprint = helix::usb_trace_fingerprint(&data);
                            if delta_only {
                                if !seen_fingerprints.insert(fingerprint.clone()) {
                                    suppressed_repeats = suppressed_repeats.saturating_add(1);
                                    continue;
                                } else if suppressed_repeats > 0 {
                                    eprintln!(
                                        "[UsbTrace][IN  0x82] known patterns suppressed total={}",
                                        suppressed_repeats
                                    );
                                    suppressed_repeats = 0;
                                }
                            }
                            if !delta_only || seen_fingerprints.contains(&fingerprint) {
                                let hex = data
                                    .iter()
                                    .map(|b| format!("{:02x}", b))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                eprintln!("[UsbTrace][IN  0x82][len={}] {}", n, hex);
                            }
                        } else {
                            seen_fingerprints.clear();
                            suppressed_repeats = 0;
                        }
                        if buf[0] == 0x0C && (buf[1] & 0xF0) == 0xC0 {
                            let preset_no = buf[2] as usize;
                            // Si activate_preset a posé le flag, cet écho MIDI est la
                            // confirmation que le hardware a appliqué le MIDI PC.
                            // Le hardware ne génère pas de x2 0x04:6a pour un MIDI PC
                            // (uniquement pour les boutons hardware) — l'écho MIDI est
                            // le seul signal fiable pour déclencher la lecture content_only.
                            let want_content = {
                                let mut s = state_midi.lock().unwrap();
                                s.preset_index = preset_no;
                                if s.want_content_only_after_x2 {
                                    s.want_content_only_after_x2 = false;
                                    s.preset_content_only = true;
                                    true
                                } else {
                                    false
                                }
                            };
                            if want_content {
                                let _ = mode_tx_midi.send(ModeRequest::RequestPreset(true));
                            } else {
                                let _ = mode_tx_midi.send(ModeRequest::RequestPresetName);
                            }
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
            Ok(ModeRequest::RequestPreset(content_only)) => {
                // Dédupliquer les RequestPreset consécutifs (course avec flux auto).
                // On garde le content_only du DERNIER message (le plus récent = intention UI).
                let mut effective_content_only = content_only;
                let mut dropped = 0usize;
                loop {
                    match mode_rx.try_recv() {
                        Ok(ModeRequest::RequestPreset(co)) => {
                            dropped += 1;
                            effective_content_only = co;
                        }
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
                // Restaurer preset_content_only depuis l'intention du message, APRÈS shutdown()
                // qui le remet toujours à false — c'est la correction de la race condition.
                s.preset_content_only = effective_content_only;
                // Incrémenter la génération : tout StandardPresetRead(gen) avec l'ancienne
                // génération sera ignoré comme orphelin (watchdog/timer d'une lecture précédente).
                s.preset_read_generation = s.preset_read_generation.wrapping_add(1);
                *m = Box::new(RequestPreset::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::RequestPresetName) => {
                // Dédupliquer les RequestPresetName consécutifs (souvent émis en rafale
                // par plusieurs chemins: sync front + événements async backend).
                let mut dropped = 0usize;
                loop {
                    match mode_rx.try_recv() {
                        Ok(ModeRequest::RequestPresetName) => dropped += 1,
                        Ok(other) => {
                            let s = state.lock().unwrap();
                            s.switch_mode(other);
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => break,
                    }
                }
                if dropped > 0 {
                    eprintln!("[PresetDebug][ModeLoop] dropped duplicate RequestPresetName x{}", dropped);
                }
                let mut s = state.lock().unwrap();
                // Bloquer RequestPresetName si on attend le x2 de confirmation
                // (want_content_only_after_x2) OU si un RequestPreset est en cours
                // (preset_content_only). Sans ce guard, le MIDI listener peut envoyer
                // RequestPresetName avant que request_preset_content() ait posé
                // preset_content_only=true, et RequestPresetName consomme le x2 0x04:6a
                // avant que Standard::data_in puisse déclencher RequestPreset(true).
                if s.preset_content_only || s.want_content_only_after_x2 {
                    eprintln!("[PresetDebug][ModeLoop] ignore RequestPresetName while content_only={} want_x2={}",
                        s.preset_content_only, s.want_content_only_after_x2);
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
            Ok(ModeRequest::StandardPresetRead(gen)) => {
                // Message émis par le timer (20ms) ou watchdog (2000ms) interne de RequestPreset.
                // Si gen != génération courante, c'est un orphelin d'une lecture précédente → ignorer.
                let mut s = state.lock().unwrap();
                if gen != s.preset_read_generation {
                    if helix::preset_debug_verbose_enabled() {
                        eprintln!(
                            "[PresetDebug][ModeLoop] StandardPresetRead(gen={}) orphelin (current={}), ignoré",
                            gen,
                            s.preset_read_generation
                        );
                    }
                    continue;
                }
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                {
                    let mut app = app_state.lock().unwrap();
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
        eprintln!("[Helix] attach_kernel_driver(0) : {}", e);
    }
    // Libérer aussi l'interface MIDI (4) réclamée à la connexion.
    // Ne pas laisser l'interface réclamée sinon le hardware reste en mode éditeur.
    let _ = handle.release_interface(4);
    if let Err(e) = handle.attach_kernel_driver(4) {
        eprintln!("[Helix] attach_kernel_driver(4) : {}", e);
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

#[cfg(test)]
mod assignable_amp_cab_chunk_tests {
    use super::{is_amp_cab_assignable_chunk, AMP_CAB_MARKER};

    #[test]
    fn is_amp_cab_true_when_marker_present_after_prefix_08() {
        let mut chunk = vec![0x08, 0x00];
        chunk.extend_from_slice(&AMP_CAB_MARKER);
        assert!(is_amp_cab_assignable_chunk(&chunk));
    }

    #[test]
    fn is_amp_cab_true_when_marker_present_after_prefix_06() {
        let mut chunk = vec![0x06, 0x00];
        chunk.extend_from_slice(&AMP_CAB_MARKER);
        assert!(is_amp_cab_assignable_chunk(&chunk));
    }

    #[test]
    fn is_amp_cab_false_without_marker() {
        assert!(!is_amp_cab_assignable_chunk(&[0x08, 0x00, 0x00]));
    }
}

#[cfg(test)]
mod assignable_module_id_extraction_tests {
    use super::{
        augmented_module_ids_for_assignable_chunk, extract_module_ids_from_assignable_chunk,
        AMP_CAB_MARKER,
    };
    use crate::preset_chain_params;

    /// L’octet `0x19` final de `…83 17 c3 19` (marqueur Amp+Cab) ne doit pas être pris pour un ID `19…1a`.
    #[test]
    fn skips_c319_trailing_byte_19_false_positive() {
        let mut v = vec![0x08];
        v.extend_from_slice(&AMP_CAB_MARKER);
        v.extend_from_slice(&[0x06, 0x1a]);
        v.push(0x19);
        v.extend_from_slice(&[0xcd, 0x02, 0xcf]);
        v.push(0x1a);
        let ids = extract_module_ids_from_assignable_chunk(&v);
        assert_eq!(ids, vec!["cd02cf".to_string()]);
    }

    /// Un `0xc3` (bool chaîne) immédiatement suivi d’un vrai `0x19…1a` ne doit pas être ignoré.
    #[test]
    fn does_not_skip_module_mark_after_bool_c3() {
        let v = vec![0x08, 0xc3, 0x19, 0xcd, 0x02, 0xcf, 0x1a];
        let ids = extract_module_ids_from_assignable_chunk(&v);
        assert_eq!(ids, vec!["cd02cf".to_string()]);
    }

    #[test]
    fn augments_dual_slot_ids_with_cab_when_only_amp_marker_is_present() {
        let seg = vec![
            0x06, 0x14, 0x85, 0x18, 0x83, 0x17, 0xc3, 0x19, 0x06, 0x1a, 0xcd, 0x02, 0xcf, 0x09,
            0x21, 0x0a, 0xc3, 0x0b, 0x83, 0x02, 0x0c, 0x03, 0x0c, 0x04, 0x9c, 0xca, 0x3f, 0x4c,
            0xcc, 0xcc, 0xca, 0x3f, 0x4f, 0x5c, 0x29, 0xca, 0x3f, 0x51, 0xeb, 0x85, 0xca, 0x3f,
            0x54, 0x7a, 0xe1, 0xca, 0x3f, 0x57, 0x0a, 0x3d, 0xca, 0x3f, 0x59, 0x99, 0x99, 0xca,
            0x3f, 0x5c, 0x28, 0xf5, 0xca, 0x3f, 0x5e, 0xb8, 0x52, 0xca, 0x3f, 0x61, 0x47, 0xae,
            0xca, 0x3f, 0x63, 0xd7, 0x0a, 0xca, 0x3f, 0x66, 0x66, 0x66, 0xca, 0x3f, 0x68, 0xf5,
            0xc2, 0x0c, 0x83, 0x02, 0x07, 0x03, 0x07, 0x04, 0x97, 0x0a, 0xca, 0x3f, 0x40, 0x00,
            0x00, 0xca, 0x41, 0x10, 0x00, 0x00, 0xca, 0x42, 0x34, 0x00, 0x00, 0xca, 0x41, 0x9f,
            0x33, 0x33, 0xca, 0x46, 0x9d, 0x08, 0x00, 0xca, 0x00, 0x00, 0x00, 0x00,
        ];
        let blocks_len = preset_chain_params::parse_assignable_segment_param_blocks(&seg)
            .map(|b| b.len())
            .unwrap_or(0);
        assert_eq!(blocks_len, 2);
        let ids = augmented_module_ids_for_assignable_chunk(&seg, blocks_len);
        assert_eq!(ids, vec!["06".to_string(), "cd02cf".to_string()]);
    }
}

#[cfg(test)]
mod extract_first_module_amp_cab_inference_tests {
    use super::extract_first_module_from_assignable_chunk;

    fn assignable_seg_from_ascii_hex(hex_lower: &str) -> Vec<u8> {
        let mut out = vec![0x08u8];
        let h = hex_lower.as_bytes();
        for i in (0..h.len()).step_by(2) {
            let b = u8::from_str_radix(std::str::from_utf8(&h[i..i + 2]).unwrap(), 16).unwrap();
            out.push(b);
        }
        out
    }

    /// Sans aucun `19…1a` : `module_hex` = `cd02171acd0228` (catalogue) à partir des deux blocs `c219`.
    #[test]
    fn infers_combined_chain_hex_for_amp_cab_without_module_markers() {
        let one_float = "ca3f800000";
        let params_hex: String = std::iter::repeat(one_float).take(21).collect();
        let amp = "c219cd02171aff09110ac30b830215031504dc0015";
        let cab = "c219cd02281aff09110ac30b830215031504dc0015";
        let body = format!("85188317c319{amp}{params_hex}{cab}{params_hex}");
        let seg = assignable_seg_from_ascii_hex(&body);
        assert!(super::is_amp_cab_assignable_chunk(&seg));
        let slot = extract_first_module_from_assignable_chunk(&seg);
        assert_eq!(slot.module_hex, "cd02171acd0228");
        assert_eq!(slot.category, "Amp+Cab");
        assert!(!slot.name.is_empty());
    }

    #[test]
    fn infers_combined_chain_hex_from_dual_slot_19_1a_09_markers_without_c219() {
        // Structure proche du parse Kempline dual-slot:
        // 0x19 <amp> 0x1a <cab> 0x09 ... ; sans `c319` ni blocs `c219`.
        let seg = vec![
            0x08, 0x85, 0x18, 0x83, 0x17, 0xc2, 0x19, 0xcd, 0x02, 0x17, 0x1a, 0xcd, 0x02, 0x28,
            0x09, 0x10, 0x0a, 0xc3,
        ];
        let slot = extract_first_module_from_assignable_chunk(&seg);
        assert_eq!(slot.module_hex, "cd02171acd0228");
        assert!(!slot.category.is_empty());
    }

    #[test]
    fn prefers_amp_like_side_from_dual_slot_when_a_and_b_are_known() {
        let seg = vec![
            0x06, 0x14, 0x85, 0x18, 0x83, 0x17, 0xc3, 0x19, 0x06, 0x1a, 0xcd, 0x02, 0xcf, 0x09,
            0x21, 0x0a, 0xc3,
        ];
        let slot = extract_first_module_from_assignable_chunk(&seg);
        assert_eq!(slot.module_hex, "06");
    }
}