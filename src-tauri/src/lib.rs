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
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::{self, Write as IoWrite};
use std::fs;
use std::path::PathBuf;
use std::thread;
use tauri::{
    Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, Position, Size, WindowEvent,
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
    set_usb_packet_trace_defer_until_ready, set_usb_packet_trace_max_len,
    usb_packet_trace_active, usb_packet_trace_delta_only,
    usb_packet_trace_max_len, usb_packet_trace_should_log,
};
use helix::packet::OutPacket;
use helix::live_write::build_cab_dual_minimal_param_packets_from_state;
use helix::live_write::build_live_write_frames_from_state;
use helix::live_write::LiveWriteRouteOverride;
use helix::live_write_config::validate_usb_live_write_metadata;
use helix::path1_io_live_write::send_path1_input_source;
use helix::path1_split_live_write::send_path1_split_type;
use helix::path1_routing_structural::{
    ensure_path2_dual_routing as send_ensure_path2_dual_routing,
    teardown_path2_dual_routing as send_teardown_path2_dual_routing,
};
use helix::matrix_routing_move::{send_matrix_routing_marker_move, RoutingMarkerKind};
use helix::matrix_slot_move::send_matrix_slot_move;
use helix::edit_slot_model::{
    build_amp_cab_replace_cab_bulk, build_cab_dual_replace_cab_bulk, build_slot_model_probe_packets, change_model_hxedit_replace_test_bulk,
    resolve_catalog_model_chain_bytes, resolve_usb_assign_bulk, slot_probe_use_change_model_test_bulk,
    SlotModelProbeOp,
};
use helix::keep_alive::KeepAliveManager;
use helix::modes::await_post_bootstrap_settle::AwaitPostBootstrapSettle;
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
    /// Arrêt de la boucle de modes / threads de la session `start_helix` en cours.
    helix_session_stop: Option<Arc<AtomicBool>>,
    /// Arrêt du thread `usb_listener` (lecture bulk 0x81).
    helix_stop_listener: Option<Arc<AtomicBool>>,
    /// `true` pendant toute la durée de `start_helix` (ouverture USB → libération interfaces).
    helix_session_busy: Option<Arc<AtomicBool>>,
    // Garde-fous backend anti-spam de recovery preset.
    preset_recover_in_flight: bool,
    last_preset_recover_at: Option<std::time::Instant>,
    last_preset_request_at: Option<std::time::Instant>,
}

const HELIX_SESSION_JOIN_TIMEOUT_MS: u64 = 3500;

/// Coupe la session USB active, purge l’état exposé à l’UI et notifie le frontend.
fn disconnect_helix_session(app: &mut AppState, app_handle: &tauri::AppHandle, reason: &str) {
    if let Some(stop) = &app.helix_session_stop {
        stop.store(true, Ordering::SeqCst);
    }
    if let Some(stop) = &app.helix_stop_listener {
        stop.store(true, Ordering::SeqCst);
    }
    if let Some(hs) = app.helix_state.take() {
        let mut s = hs.lock().unwrap();
        s.connected = false;
        s.tx = None;
        s.mode_tx = None;
        if let Some(ka) = &s.keepalive_tx {
            let _ = ka.send(KeepAliveCommand::StopAll);
        }
        s.keepalive_tx = None;
        s.editor_ready = false;
        s.post_arm_sequence_started = false;
        s.post_ef_arm_ack_mask = 0;
        s.post_ef_arm_gate_active = false;
        s.post_ef_gate_rx = None;
        s.post_ef_gate_tx = None;
        s.phase4_bootstrap_active = false;
        s.phase4_complete_rx = None;
        s.phase4_complete_tx = None;
        s.reset_hw_model_pull_state();
    }
    app.usb_handle = None;
    app.connected_device_name = None;
    app.connection_issue_hint = None;
    app.preset_names.clear();
    app.active_preset = 0;
    app.preset_recover_in_flight = false;
    eprintln!("[Helix] session déconnectée ({reason})");
    if let Err(e) = app_handle.emit("helix-device-lost", reason) {
        eprintln!("[Helix] emit helix-device-lost: {e}");
    }
}

/// Fermeture gracieuse USB sur quit d'application : envoie le tour `sub=0x02`
/// (désabonnement éditeur — cf. keep_alive::graceful_close_packets) sur les 3 lanes,
/// laisse le Stomp le traiter, puis libère les interfaces. SYNCHRONE et appelé AVANT
/// `exit(0)` : Tauri tue le process juste après, donc aucun teardown différé
/// (start_helix / disconnect_helix_session) ne tournerait à temps.
///
/// Ne pas toucher `helix_session_stop` ici : ça déclencherait `disconnect_helix_session`
/// (`connected=false`) avant l'envoi du close. On coupe seulement le poll idle via
/// `KeepAliveCommand::StopAll`.
fn graceful_helix_close(app: &tauri::AppHandle) {
    let app_state = app.state::<Arc<Mutex<AppState>>>();
    let (helix_arc, handle, stop_listener) = {
        let a = app_state.lock().unwrap();
        (a.helix_state.clone(), a.usb_handle.clone(), a.helix_stop_listener.clone())
    };
    let (Some(helix_arc), Some(handle)) = (helix_arc, handle) else { return; };

    // 1) Stopper le poll idle (canal KA) ET le listener : on va lire 0x81 nous-mêmes,
    //    en paced, pour que chaque ACK device ait un URB IN où atterrir.
    {
        let s = helix_arc.lock().unwrap();
        if let Some(ka) = &s.keepalive_tx { let _ = ka.send(KeepAliveCommand::StopAll); }
    }
    if let Some(s) = stop_listener.as_ref() { s.store(true, Ordering::SeqCst); }
    thread::sleep(Duration::from_millis(80)); // laisse poll + listener libérer 0x81

    // 2) graceful_close_packets renvoie [f0, ed, ef] ; on réordonne en ed → f0 → ef (ordre HX Edit).
    let pkts = {
        let mut s = helix_arc.lock().unwrap();
        helix::keep_alive::graceful_close_packets(&mut s)
    };
    let ordered = [&pkts[1], &pkts[0], &pkts[2]]; // ed, f0, ef
    let mut buf = [0u8; 64];
    for p in ordered {
        let _ = handle.write_bulk(0x01, p, Duration::from_millis(200));
        // read_bulk poste l'URB IN ET attend l'ACK avant la lane suivante (= pacing HX Edit).
        let _ = handle.read_bulk(0x81, &mut buf, Duration::from_millis(150));
    }

    // 3) Libérer les interfaces.
    let _ = handle.release_interface(0);
    let _ = handle.attach_kernel_driver(0);
    let _ = handle.release_interface(4);
    let _ = handle.attach_kernel_driver(4);
    eprintln!("[Helix] graceful close paced (ed→f0→ef) + interfaces libérées");
}


fn wait_previous_helix_session_end(busy: &AtomicBool, stop: Option<&AtomicBool>) {
    if let Some(s) = stop {
        s.store(true, Ordering::SeqCst);
    }
    let deadline = Instant::now() + Duration::from_millis(HELIX_SESSION_JOIN_TIMEOUT_MS);
    while busy.load(Ordering::SeqCst) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    if busy.load(Ordering::SeqCst) {
        eprintln!(
            "[Helix] WARN: session précédente encore active après {HELIX_SESSION_JOIN_TIMEOUT_MS} ms"
        );
    }
}

/// Copie `preset_names` / `active_preset` depuis `HelixState` vers `AppState`.
/// Les noms sont souvent prêts dans `HelixState` avant le passage en mode `Standard`
/// (qui était le seul chemin de copie) — l’UI poll `get_preset_names()` et voyait une liste vide.
fn sync_app_presets_from_helix(app: &mut AppState, s: &HelixState) -> bool {
    let mut synced = false;
    if s.got_preset_names && s.preset_names.len() >= EXPECTED_PRESET_COUNT {
        if app.preset_names != s.preset_names {
            app.preset_names = s.preset_names.clone();
            synced = true;
            eprintln!(
                "[PresetDebug][sync] preset_names -> AppState ({} noms, actif={})",
                app.preset_names.len(),
                s.preset_index
            );
        }
    } else if s.got_preset_names {
        eprintln!(
            "[PresetDebug][sync] skip: got_preset_names mais len={} (< {})",
            s.preset_names.len(),
            EXPECTED_PRESET_COUNT
        );
    }
    if app.active_preset != s.preset_index {
        app.active_preset = s.preset_index;
        synced = true;
    }
    synced
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
    let helix_arc = {
        let app = state.lock().unwrap();
        if app.preset_names.len() >= EXPECTED_PRESET_COUNT {
            return app.preset_names.clone();
        }
        app.helix_state.clone()
    };
    let mut app = state.lock().unwrap();
    if let Some(helix_arc) = helix_arc {
        if let Ok(s) = helix_arc.lock() {
            sync_app_presets_from_helix(&mut app, &s);
        }
    }
    app.preset_names.clone()
}

#[tauri::command]
fn get_active_preset(state: tauri::State<Arc<Mutex<AppState>>>) -> usize {
    let helix_arc = state.lock().unwrap().helix_state.clone();
    let mut app = state.lock().unwrap();
    if let Some(helix_arc) = helix_arc {
        if let Ok(s) = helix_arc.lock() {
            sync_app_presets_from_helix(&mut app, &s);
        }
    }
    app.active_preset
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
    if s.init_usb_settle_active() {
        return Err(format!(
            "Initialisation USB en cours (~{} ms) — réessayer",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
    if !s.editor_ready {
        return Err("Amorçage USB en cours — preset actif pas encore disponible".to_string());
    }
    s.switch_mode(ModeRequest::RequestPresetName);
    Ok(())
}

/// `true` pendant la fenêtre post bootstrap (~700 ms) où le host n’envoie que des ACK (HX Edit).
#[tauri::command]
fn is_helix_usb_init_settling(state: tauri::State<Arc<Mutex<AppState>>>) -> bool {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let Some(helix_arc) = helix_arc else {
        return false;
    };
    helix_arc
        .lock()
        .map(|s| s.init_usb_settle_active())
        .unwrap_or(false)
}

/// Renomme un preset sur le HX.
/// Uniquement le preset **actif** (même contrainte que la lecture wire du nom).
#[tauri::command]
fn rename_preset(
    index: usize,
    name: String,
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let (helix_arc, active_preset) = {
        let app = state.lock().unwrap();
        (
            app.helix_state.clone(),
            app.active_preset,
        )
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;

    if index != active_preset {
        return Err(format!(
            "Renommage refusé : seul le preset actif ({:03}) peut être renommé",
            active_preset
        ));
    }

    // Limiter à 16 caractères ASCII — limite produit utilisée par l'application.
    let text: Vec<u8> = name
        .chars()
        .filter(|c| c.is_ascii())
        .take(16)
        .map(|c| c as u8)
        .collect();

    let effective_name = String::from_utf8(text.clone()).unwrap_or_default();
    if effective_name.is_empty() {
        return Err("nom preset vide".to_string());
    }

    let mut s = helix_arc.lock().unwrap();
    if !s.editor_ready {
        return Err("Amorçage USB en cours — renommage indisponible".to_string());
    }
    if s.preset_index != index {
        return Err(format!(
            "Renommage refusé : le matériel n'a pas confirmé le preset {:03}",
            index
        ));
    }

    helix::preset_label::send_preset_rename(&mut s, index, &effective_name)?;
    s.pending_rename_name_verify = true;
    s.switch_mode(ModeRequest::RequestPresetName);

    Ok(())
}

/// Enregistre un preset sur le HX (capture HX Edit `save_preset.json`).
/// Uniquement le preset **actif** (modifications en RAM du preset courant).
#[tauri::command]
fn save_preset_to_hardware(
    index: usize,
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let (helix_arc, active_preset) = {
        let app = state.lock().unwrap();
        (app.helix_state.clone(), app.active_preset)
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;

    if index != active_preset {
        return Err(format!(
            "Sauvegarde refusée : seul le preset actif ({:03}) peut être enregistré",
            active_preset
        ));
    }

    let name = {
        let app = state.lock().unwrap();
        app.preset_names
            .get(index)
            .cloned()
            .filter(|n| !n.is_empty() && n != "<empty>")
            .ok_or_else(|| format!("nom preset {index} indisponible"))?
    };

    let mut s = helix_arc.lock().unwrap();
    if !s.editor_ready {
        return Err("Amorçage USB en cours — sauvegarde indisponible".to_string());
    }
    if s.preset_index != index {
        return Err(format!(
            "Sauvegarde refusée : le matériel n'a pas confirmé le preset {:03}",
            index
        ));
    }
    helix::preset_label::send_preset_save(&mut s, index, &name)
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
            // Masque le slot du preset précédent pendant le chargement. Le slot du nouveau preset
            // est rétabli par `ingest_hw_slot_notify_in` (trame IN `82:62`, souvent 44 o head `21`
            // après PC — cf. `Change_preset.json`), même variable que cadre orange / pull modèle.
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
    // Même état que `ingest_hw_slot_notify_in` (`hw_active_slot_*`) — pas un second registre.
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
    app: tauri::AppHandle,
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

    /// Fenêtre courte : les IN utiles arrivent en général tout de suite ; 130 ms + sleep
    /// ajoutait ~150 ms de latence ressentie sur chaque changement de slot hardware.
    const CAPTURE_MS: u64 = 55;
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

    // Marge minuscule au-delà de la deadline pour la dernière trame IN.
    thread::sleep(Duration::from_millis(CAPTURE_MS.saturating_add(8)));

    let mut frames: Vec<Vec<u8>> = Vec::new();
    {
        let mut s = helix_arc.lock().unwrap();
        s.usb_slot_focus_capture_deadline = None;
        std::mem::swap(&mut frames, &mut s.usb_slot_focus_capture);
    }

    let parsed = helix::slot_focus_in::parse_slot_focus_bulk_in_frames(&frames);
    let slot_focus_parsed =
        serde_json::to_value(&parsed).unwrap_or(serde_json::Value::Null);
    let slot_idx_usize = slot_index as usize;
    let content_change = {
        let mut s = helix_arc.lock().unwrap();
        if slot_idx_usize < 16 {
            s.last_slot_focus_capsule[slot_idx_usize] = parsed.clone();
        }
        let next = helix::slot_watch::SlotWatchSnapshot::from_capsule(parsed.as_ref());
        let kind = if slot_idx_usize < 16 {
            helix::slot_watch::detect_slot_content_change(
                &mut s.slot_watch_prev[slot_idx_usize],
                &next,
            )
        } else {
            None
        };
        kind.map(|k| {
            s.hw_slot_content_sequence = s.hw_slot_content_sequence.wrapping_add(1);
            helix::slot_watch::SlotContentChangedPayload {
                sequence: s.hw_slot_content_sequence,
                slot_index,
                kind: k.as_str().to_string(),
                capsule_sig: next.capsule_sig.clone(),
            }
        })
    };
    if let Some(ref payload) = content_change {
        if preset_debug_verbose_enabled() {
            eprintln!(
                "[SlotWatch] slot_index={} kind={} seq={}",
                payload.slot_index, payload.kind, payload.sequence
            );
        }
        let _ = app.emit("models:slot-content-changed", payload);
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
            "[SlotFocusSync] slot_index={} slot_bus={:02x} in_frames={} parsed={} out={}",
            slot_index,
            slot_bus,
            frames_hex.len(),
            parsed.is_some(),
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
        "slotFocusParsed": slot_focus_parsed,
        "contentChange": content_change,
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
/// `cab_catalog_model_id` + `cab_assign_variant` : remplacement **cab seul** sur Amp+Cab
/// (`catalog_model_id` = id ampli, `assign_variant` = `amp+cab` ou `amp+cab-legacy`).
/// Avec `cab_dual_cab_index` (`0` = Cab 1, `1` = Cab 2) : patch d’un cab dans un slot **Cab dual**
/// (`catalog_model_id` = id dual, `assign_variant` = `dual`).
#[tauri::command]
fn probe_slot_model_usb(
    state: tauri::State<Arc<Mutex<AppState>>>,
    op: String,
    slot_index: u32,
    catalog_model_id: Option<String>,
    assign_variant: Option<String>,
    cab_catalog_model_id: Option<String>,
    cab_assign_variant: Option<String>,
    cab_dual_cab_index: Option<u32>,
    skip_cab_dual_focus: Option<bool>,
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
    let cab_id_for_log = cab_catalog_model_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let cab_variant_lc = cab_assign_variant
        .as_ref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "single".to_string());
    let use_change_model_test_bulk = matches!(probe_op, SlotModelProbeOp::ReplaceOccupied)
        && slot_probe_use_change_model_test_bulk()
        && cab_id_for_log.is_none();
    let usb_bulk_from_json: Option<Vec<u8>> = if use_change_model_test_bulk {
        Some(change_model_hxedit_replace_test_bulk())
    } else if let Some(cab_id) = cab_id_for_log.as_deref() {
        let parent_id = id_for_log.as_deref().ok_or_else(|| {
            "cab_catalog_model_id requiert catalog_model_id (parent Amp+Cab ou Cab dual)".to_string()
        })?;
        if let Some(idx) = cab_dual_cab_index {
            if idx > 1 {
                return Err(format!("cabDualCabIndex hors plage: {idx} (attendu 0 ou 1)"));
            }
            Some(build_cab_dual_replace_cab_bulk(
                parent_id,
                cab_id,
                &cab_variant_lc,
                idx as u8,
            )?)
        } else {
            let amp_cab_variant = if cab_variant_lc == "legacy" && variant_lc == "amp+cab" {
                "amp+cab-legacy".to_string()
            } else {
                variant_lc.clone()
            };
            Some(build_amp_cab_replace_cab_bulk(
                parent_id,
                &amp_cab_variant,
                cab_id,
                &cab_variant_lc,
            )?)
        }
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

    let cab_dual_cab2_replace = cab_dual_cab_index == Some(1);
    let amp_cab_cab_replace =
        cab_id_for_log.is_some() && cab_dual_cab_index.is_none() && matches!(probe_op, SlotModelProbeOp::ReplaceOccupied);
    if cab_dual_cab2_replace {
        {
            let s = helix_arc.lock().unwrap();
            if s.init_usb_settle_active() {
                return Err(format!(
                    "probe_slot_model_usb ignoré (init USB ~{} ms, ACK seulement)",
                    helix::keep_alive::POST_PHASE4_SETTLE_MS
                ));
            }
            if !s.connected || s.preset_content_only {
                return Err(
                    "probe_slot_model_usb ignoré (HX non prêt ou lecture preset en cours)"
                        .to_string(),
                );
            }
        }
        let bulk = usb_bulk_from_json
            .as_deref()
            .ok_or_else(|| "cab dual cab2 replace sans bulk JSON".to_string())?;
        let summary = helix::cab_dual::execute_cab_dual_cab2_replace(
            helix_arc.clone(),
            slot_index,
            slot_bus,
            bulk,
        )?;
        eprintln!(
            "[SlotModelProbe] op={:?} slot_index={} slot_bus={:#04x} catalog_id={:?} variant={} cab_id={:?} cab_variant={} cab_dual_idx=Some(1) usb_json=true cab_dual_handshake=true {}",
            probe_op,
            slot_index,
            slot_bus,
            id_for_log,
            variant_lc,
            cab_id_for_log,
            cab_variant_lc,
            summary
        );
        return Ok(summary);
    }

    if amp_cab_cab_replace {
        {
            let s = helix_arc.lock().unwrap();
            if s.init_usb_settle_active() {
                return Err(format!(
                    "probe_slot_model_usb ignoré (init USB ~{} ms, ACK seulement)",
                    helix::keep_alive::POST_PHASE4_SETTLE_MS
                ));
            }
            if !s.connected || s.preset_content_only {
                return Err(
                    "probe_slot_model_usb ignoré (HX non prêt ou lecture preset en cours)"
                        .to_string(),
                );
            }
        }
        let bulk = usb_bulk_from_json
            .as_deref()
            .ok_or_else(|| "amp+cab cab replace sans bulk JSON".to_string())?;
        // UI peut encore envoyer `amp+cab` si linkedCabHex / catégorie grille sont ambigus ;
        // le bulk `amp+cab-legacy` catalogue est toujours head `0x23` (44 o compact).
        let legacy = variant_lc == "amp+cab-legacy"
            || cab_variant_lc == "legacy"
            || helix::amp_cab_cab_replace::amp_cab_replace_bulk_implies_legacy(bulk);
        let summary = helix::amp_cab_cab_replace::execute_amp_cab_cab_replace(
            helix_arc.clone(),
            slot_index,
            slot_bus,
            bulk,
            legacy,
        )?;
        eprintln!(
            "[SlotModelProbe] op={:?} slot_index={} slot_bus={:#04x} catalog_id={:?} variant={} cab_id={:?} cab_variant={} amp_cab_cab_handshake=true {}",
            probe_op,
            slot_index,
            slot_bus,
            id_for_log,
            variant_lc,
            cab_id_for_log,
            cab_variant_lc,
            summary
        );
        return Ok(summary);
    }

    let mut s = helix_arc.lock().unwrap();
    if s.init_usb_settle_active() {
        return Err(format!(
            "probe_slot_model_usb ignoré (init USB ~{} ms, ACK seulement)",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
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
        false,
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
    if matches!(probe_op, SlotModelProbeOp::AddToEmpty)
        && matches!(variant_lc.as_str(), "amp+cab" | "amp+cab-legacy")
    {
        if let Some(bulk) = packs.iter().filter(|p| p.len() >= 32).max_by_key(|p| p.len()) {
            helix::amp_cab_live_write::record_amp_cab_assign_session(&mut s, slot_index, bulk);
        }
    }
    if let Some(bulk) = usb_bulk_from_json.as_deref() {
        if cab_dual_cab_index.is_none() && cab_id_for_log.is_none() {
            if variant_lc == "legacy"
                || helix::amp_cab_live_write::bulk_has_wire_marker(
                    bulk,
                    helix::amp_cab_live_write::C219_BULK_MARKER,
                )
            {
                helix::amp_cab_live_write::record_standalone_legacy_cab_module_field(
                    &mut s,
                    slot_index,
                    bulk,
                );
            } else if variant_lc == "dual"
                && helix::amp_cab_live_write::bulk_is_dual_legacy_wire(bulk)
            {
                helix::amp_cab_live_write::record_dual_legacy_cab_module_fields(
                    &mut s,
                    slot_index,
                    bulk,
                );
            }
        } else if cab_dual_cab_index == Some(1)
            && helix::amp_cab_live_write::bulk_is_dual_legacy_wire(bulk)
        {
            helix::amp_cab_live_write::record_dual_legacy_cab2_module_field(
                &mut s,
                slot_index,
                bulk,
            );
        }
    }
    drop(s);

    let summary = lines.join(" | ");
    eprintln!(
        "[SlotModelProbe] op={:?} slot_index={} slot_bus={:#04x} catalog_id={:?} variant={} cab_id={:?} cab_variant={} cab_dual_idx={:?} usb_json={} change_model_test_bulk={} {}",
        probe_op,
        slot_index,
        slot_bus,
        id_for_log,
        variant_lc,
        cab_id_for_log,
        cab_variant_lc,
        cab_dual_cab_index,
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

fn resolve_live_write_route_override(
    state: &helix::HelixState,
    slot_index: u32,
    param_index: u32,
    dual_part: Option<&str>,
    amp_cab_assign_variant: Option<&str>,
    cab_dual_assign_variant: Option<&str>,
    amp_cab_amp_param_count: Option<u32>,
) -> Option<LiveWriteRouteOverride> {
    match dual_part.map(str::trim) {
        Some("cab") => {
            let variant = amp_cab_assign_variant.unwrap_or("amp+cab");
            helix::amp_cab_live_write::resolve_cab_live_write_route(
                state,
                param_index,
                variant,
                slot_index,
                amp_cab_amp_param_count,
            )
        }
        Some("cab1") => {
            let legacy =
                helix::cab_dual_live_write::cab_dual_assign_variant_is_legacy_hybrid(
                    cab_dual_assign_variant,
                );
            helix::cab_dual_live_write::resolve_cab_dual_live_write_route(
                state, 0, param_index, slot_index, legacy,
            )
        }
        Some("cab2") => {
            let legacy =
                helix::cab_dual_live_write::cab_dual_assign_variant_is_legacy_hybrid(
                    cab_dual_assign_variant,
                );
            helix::cab_dual_live_write::resolve_cab_dual_live_write_route(
                state, 1, param_index, slot_index, legacy,
            )
        }
        _ => helix::amp_cab_live_write::resolve_standalone_legacy_cab_live_write_route_from_probe(
            state,
            param_index,
            slot_index,
        ),
    }
}

/// Focus USB sous-bloc Amp ou Cab d'un slot Amp+Cab (`1d`, capture `ampcab_legacy_switch_tab.json`).
#[tauri::command]
fn focus_amp_cab_usb_part(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
    part: String,
    amp_cab_assign_variant: Option<String>,
) -> Result<(), String> {
    let _ = amp_cab_assign_variant;
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    let part_lc = part.trim().to_ascii_lowercase();
    let cab = part_lc == "cab";
    let amp = part_lc == "amp";
    if !cab && !amp {
        return Ok(());
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or_else(|| "slotIndex invalide".to_string())?;
    {
        let s = helix_arc.lock().unwrap();
        if !s.connected || s.preset_content_only {
            return Err("focus Amp+Cab ignoré (HX non prêt ou lecture preset)".to_string());
        }
    }
    helix::amp_cab_live_write::spawn_amp_cab_tab_focus_usb(helix_arc, slot_index, slot_bus, cab);
    Ok(())
}

/// Focus USB sous-bloc **Cab 1** ou **Cab 2** d’un slot Cab dual (`1d`, capture HX Edit).
#[tauri::command]
fn focus_cab_dual_usb_part(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
    part: String,
) -> Result<(), String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or_else(|| "slotIndex invalide".to_string())?;
    let mut s = helix_arc.lock().unwrap();
    if !s.connected || s.preset_content_only {
        return Err("focus Cab dual ignoré (HX non prêt ou lecture preset)".to_string());
    }
    match part.trim().to_ascii_lowercase().as_str() {
        "cab2" => {
            helix::cab_dual_live_write::send_cab_dual_cab2_focus_and_poke(
                &mut s, slot_index, slot_bus,
            );
        }
        "cab1" | "cab" => {
            helix::cab_dual_live_write::send_cab_dual_cab1_focus_and_poke(
                &mut s, slot_index, slot_bus,
            );
        }
        _ => {}
    }
    Ok(())
}

/// Témoin `HX_LEGACY_SINGLE_IR_PARAM` (défaut ON) : écrire les params d'un cab single legacy
/// avec la trame IR standard `23`/`27` (capture add_single_legacy_change_param.json), comme le
/// single modern, au lieu du burst minimal `57`/`25`. `=0|false|no|off` restaure le burst (témoin).
fn legacy_single_ir_param_enabled() -> bool {
    match std::env::var("HX_LEGACY_SINGLE_IR_PARAM").as_deref() {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
}

/// `HX_DUAL_LEGACY_STD_PARAM` (défaut ON) : params dual legacy via le builder dual modern
/// (trame cd 04 standard + c2 sur discret), au lieu du burst hybride 23/25/71.
/// `=0` -> ancien burst.
fn dual_legacy_standard_param_enabled() -> bool {
    match std::env::var("HX_DUAL_LEGACY_STD_PARAM").as_deref() {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
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
    dual_part: Option<String>,
    amp_cab_assign_variant: Option<String>,
    cab_dual_assign_variant: Option<String>,
    amp_cab_amp_param_count: Option<u32>,
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
    let dual_part_ref = dual_part.as_deref().map(str::trim);
    let variant_ref = amp_cab_assign_variant.as_deref().map(str::trim);
    let cab_dual_variant_ref = cab_dual_assign_variant.as_deref().map(str::trim);
    let amp_cab_amp_count = amp_cab_amp_param_count;
    let mut s = helix_arc.lock().unwrap();
    if dual_part_ref == Some("cab") && s.amp_cab_cab_focus_sent_for_slot != Some(slot_index) {
        if let Some(slot_bus) = kempline_index_to_slot_bus(slot_index as usize) {
            let focus =
                helix::amp_cab_live_write::build_amp_cab_ir_cab_focus_packet(&mut s, slot_bus);
            s.send(crate::helix::packet::OutPacket::new(focus));
            s.amp_cab_cab_focus_sent_for_slot = Some(slot_index);
        }
    }
    let mut route_override = resolve_live_write_route_override(
        &s,
        slot_index,
        param_index,
        dual_part_ref,
        variant_ref,
        cab_dual_variant_ref,
        amp_cab_amp_count,
    );

    let leg_b = match (chain_min, chain_max) {
        (Some(lo), Some(hi)) if hi > lo && lo.is_finite() && hi.is_finite() => {
            lo + f64::from(raw) * (hi - lo)
        }
        _ => f64::from(raw),
    };

    if matches!(dual_part_ref, Some("cab1") | Some("cab2")) {
        let cab_index: u8 = if dual_part_ref == Some("cab2") { 1 } else { 0 };
        if let Some(slot_bus) = kempline_index_to_slot_bus(slot_index as usize) {
            helix::cab_dual_live_write::prepare_cab_dual_param_live_write(
                &mut s,
                slot_index,
                slot_bus,
                cab_index,
            );
        }
        let route = route_override.ok_or_else(|| {
            "route Cab dual introuvable (slot ou param_index invalide)".to_string()
        })?;
        if helix::amp_cab_live_write::route_is_dual_legacy_cab(&route)
            && !dual_legacy_standard_param_enabled()
        {
            let cab_index: u8 = if route.pp == 0x04 { 1 } else { 0 };
            let route_pp = route.pp;
            let minimal =
                helix::amp_cab_live_write::build_dual_legacy_minimal_param_packets_from_state(
                    &mut s,
                    raw,
                    slot_index,
                    cab_index,
                    dt,
                    vt,
                    chain_min,
                    chain_max,
                    route,
                )?;
            for (i, pkt) in minimal.packets.iter().enumerate() {
                let delay = if i == 0 { 0 } else { 8 };
                s.send(OutPacket::with_delay(pkt.clone(), delay));
            }
            eprintln!(
                "[LiveWrite][sent] slot={} param={} symbolicId={} displayType={} valueType={:?} opcode={:02x} rawValue={} sentRaw={} legBChain={} chainMin={:?} chainMax={:?} slotBus={:02x} pp={:02x} cab_dual=legacy_minimal cab{} pSel={:02x} packets={} frame_b={}",
                slot_index,
                param_index,
                sid,
                display_type.unwrap_or_default(),
                value_type,
                minimal.primary_opcode,
                raw_value,
                raw,
                leg_b,
                chain_min,
                chain_max,
                minimal.slot_bus,
                route_pp,
                cab_index + 1,
                minimal.param_selector,
                minimal.packets.len(),
                minimal
                    .packets
                    .last()
                    .map(|p| p.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default(),
            );
            return Ok(());
        }
        let minimal = build_cab_dual_minimal_param_packets_from_state(
            &mut s,
            raw,
            slot_index,
            dt,
            vt,
            chain_min,
            chain_max,
            route,
        );
        for (i, pkt) in minimal.packets.iter().enumerate() {
            let delay = if i == 0 { 0 } else { 8 };
            s.send(OutPacket::with_delay(pkt.clone(), delay));
        }
        eprintln!(
            "[LiveWrite][sent] slot={} param={} symbolicId={} displayType={} valueType={:?} opcode={:02x} rawValue={} sentRaw={} legBChain={} chainMin={:?} chainMax={:?} slotBus={:02x} pp={:02x} ppSource={} pSel={:02x} pSelSource={} model_block={} cab_dual=minimal packets={} frame_a={} frame_b={}",
            slot_index,
            param_index,
            sid,
            display_type.unwrap_or_default(),
            value_type,
            minimal.primary_opcode,
            raw_value,
            raw,
            leg_b,
            chain_min,
            chain_max,
            minimal.slot_bus,
            minimal.pp,
            minimal.pp_source,
            minimal.param_selector,
            minimal.param_selector_source,
            minimal.model_block_kind,
            minimal.packets.len(),
            minimal
                .packets
                .first()
                .map(|p| p.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "))
                .unwrap_or_default(),
            minimal
                .packets
                .get(1)
                .map(|p| p.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "))
                .unwrap_or_default(),
        );
        return Ok(());
    }

    // Cab single legacy : capture add_single_legacy_change_param.json (HD2_Cab1x6x9SoupProEllipse,
    // hint 0x33, bulk cd:03:ff). HX Edit écrit TOUS les params en trame IR standard 23/27 à
    // cd:03 <tag> (tags f7,f8,fe,ff,00…) — aucun 1b/19 (pas de handshake async cd03ff), aucun
    // 57/25 (pas de burst minimal). Le test assign[4]==0xff est un FAUX POSITIF : 0xff = tag figé
    // du template JSON, pas un marqueur de cab. Le chemin IR standard (route_override = None) est
    // identique au single modern, déjà prouvé sur le Stomp.
    // Témoin HX_LEGACY_SINGLE_IR_PARAM=0 : restaure l'ancien (handshake cd03ff / burst minimal).
    let standalone_legacy = route_override
        .as_ref()
        .map(helix::amp_cab_live_write::route_is_standalone_legacy_cab)
        .unwrap_or(false);
    if standalone_legacy {
        if let Some(assign_block) = s
            .standalone_legacy_assign_model_block_by_slot
            .get(&slot_index)
            .copied()
        {
            if legacy_single_ir_param_enabled() {
                // Chemin IR standard pour TOUS les single legacy/modern enregistrés.
                route_override = None;
                // c2 sur le discret SEULEMENT pour un legacy compact (hint 1 octet).
                // Un MicIr modern (c2 19 cd03xx) garde c3 (replay statique).
                if s.standalone_legacy_cab_module_field_by_slot
                    .get(&slot_index)
                    .is_some_and(|field| {
                        helix::amp_cab_live_write::standalone_legacy_assign_is_one_byte_hint(field)
                    })
                {
                    s.force_discrete_c2_for_legacy_single = true;
                }
            } else if helix::amp_cab_live_write::standalone_legacy_assign_uses_cd03ff(assign_block) {
                // TÉMOIN (=0) : ancien handshake async cd:03:ff.
                let param_selector =
                    route_override.as_ref().map(|r| r.param_selector).unwrap_or(0);
                helix::legacy_cab_param_commit::start_standalone_legacy_cd03ff_write(
                    &mut s,
                    slot_index,
                    assign_block,
                    param_selector,
                )?;
                eprintln!(
                    "[LiveWrite][sent] slot={} param={} symbolicId={} legacy_cab=cd03ff_handshake(temoin) pSel={:02x}",
                    slot_index, param_index, sid, param_selector,
                );
                return Ok(());
            } else {
                // TÉMOIN (=0) : ancien burst minimal 23/25/57.
                let route = route_override.take().expect("route standalone legacy");
                let minimal =
                    helix::amp_cab_live_write::build_standalone_legacy_minimal_param_packets_from_state(
                        &mut s, raw, slot_index, dt, vt, chain_min, chain_max, route,
                    )?;
                for (i, pkt) in minimal.packets.iter().enumerate() {
                    let delay = if i == 0 { 0 } else { 8 };
                    s.send(OutPacket::with_delay(pkt.clone(), delay));
                }
                eprintln!(
                    "[LiveWrite][sent] slot={} param={} symbolicId={} legacy_cab=minimal(temoin) opcode={:02x} packets={}",
                    slot_index, param_index, sid, minimal.primary_opcode, minimal.packets.len(),
                );
                return Ok(());
            }
        }
    }

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
        route_override,
    );

    s.send(OutPacket::new(frames.pre_packet_x80.clone()));
    s.send(OutPacket::with_delay(frames.pre_packet_x2.clone(), 4));
    s.send(OutPacket::with_delay(frames.pre_packet_x80_sel.clone(), 8));
    s.send(OutPacket::with_delay(frames.packet_27.clone(), 12));
    // Deuxième jambe HX Edit (octet 11 = 0x0c), CTR/SEQ déjà avancés dans le builder.
    s.send(OutPacket::with_delay(frames.packet_27_b.clone(), 8));
    s.send(OutPacket::with_delay(frames.post_packet_x80_sel.clone(), 8));

    drop(s);

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

/// Focus slot structurel Path 1 (Input `0x00`, Output `0x09`, Split `0x0a`, Merge `0x13`).
#[tauri::command]
fn switch_active_hardware_special_slot(
    slot_bus: u8,
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    if !helix::is_special_slot_bus(slot_bus) {
        return Err(format!(
            "slotBus {slot_bus:#04x} invalide (attendu 0x00|0x09|0x0a|0x13)"
        ));
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;

    let mut s = helix_arc.lock().unwrap();
    if s.preset_content_only || !s.connected {
        return Err("switch slot I/O ignoré pendant chargement preset".to_string());
    }
    let packet = helix::path1_io_live_write::build_special_slot_focus_packet(&mut s, slot_bus);
    s.send(OutPacket::new(packet.clone()));
    s.hw_active_slot_bus = Some(slot_bus);
    s.hw_active_slot_index = None;
    drop(s);

    eprintln!(
        "[HwIoSlotSwitch][sent] slot_bus={slot_bus:#04x} packet={}",
        packet
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(())
}

/// Change la source Input Path 1 (Stomp : Main / Return / USB 5/6) via trame `1d` capturée
/// (`HX_ModelUsbAssign.json` → `ioSources[]`, pas `bulkHex` assign slot).
#[tauri::command]
fn write_path1_input_source(
    state: tauri::State<Arc<Mutex<AppState>>>,
    io_source_id: String,
) -> Result<String, String> {
    let id = io_source_id.trim();
    if id.is_empty() {
        return Err("ioSourceId vide".to_string());
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;
    let mut s = helix_arc.lock().unwrap();
    if s.init_usb_settle_active() {
        return Err(format!(
            "write_path1_input_source ignoré (init USB ~{} ms)",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
    if !s.connected || s.preset_content_only {
        return Err(
            "write_path1_input_source ignoré (HX non prêt ou lecture preset en cours)".to_string(),
        );
    }
    send_path1_input_source(&mut s, id)
}

/// Valeur wire `@input` Path 1 mémorisée depuis le trafic IN USB (prioritaire sur le dump preset pour le picker).
#[tauri::command]
fn get_path1_input_source_wire_value(
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Option<u8> {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()?
    };
    let s = helix_arc.lock().unwrap();
    s.path1_input_source_wire
}

/// Change le type Split Path 1 (Y / A/B / Crossover / Dynamic) via trame `25` capturée
/// (`HX_ModelUsbAssign.json` → `splitSources[]`).
#[tauri::command]
fn write_path1_split_type(
    state: tauri::State<Arc<Mutex<AppState>>>,
    split_source_id: String,
) -> Result<String, String> {
    let id = split_source_id.trim();
    if id.is_empty() {
        return Err("splitSourceId vide".to_string());
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;
    let mut s = helix_arc.lock().unwrap();
    if s.init_usb_settle_active() {
        return Err(format!(
            "write_path1_split_type ignoré (init USB ~{} ms)",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
    if !s.connected || s.preset_content_only {
        return Err(
            "write_path1_split_type ignoré (HX non prêt ou lecture preset en cours)".to_string(),
        );
    }
    send_path1_split_type(&mut s, id)
}

/// Valeur wire type Split Path 1 mémorisée depuis le trafic IN USB.
#[tauri::command]
fn get_path1_split_type_wire_value(
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Option<u8> {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()?
    };
    let s = helix_arc.lock().unwrap();
    s.path1_split_type_wire
}

#[tauri::command]
fn move_matrix_slot_usb(
    source_slot_index: u32,
    dest_slot_index: u32,
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<String, String> {
    if source_slot_index >= 16 || dest_slot_index >= 16 {
        return Err("sourceSlotIndex / destSlotIndex hors plage (0..15)".to_string());
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;
    let mut s = helix_arc.lock().unwrap();
    if s.init_usb_settle_active() {
        return Err(format!(
            "move_matrix_slot_usb ignoré (init USB ~{} ms)",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
    if !s.connected || s.preset_content_only {
        return Err(
            "move_matrix_slot_usb ignoré (HX non prêt ou lecture preset en cours)".to_string(),
        );
    }
    send_matrix_slot_move(&mut s, source_slot_index as usize, dest_slot_index as usize)
}

#[tauri::command]
fn move_matrix_routing_marker_usb(
    marker: String,
    dest_boundary_col: u32,
    first_path2_col: u32,
    last_path2_col: u32,
    current_split_col: u32,
    current_merge_col: u32,
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<String, String> {
    if dest_boundary_col > 8 {
        return Err("destBoundaryCol hors plage (0..8)".to_string());
    }
    let kind = match marker.trim().to_ascii_lowercase().as_str() {
        "split" => RoutingMarkerKind::Split,
        "merge" => RoutingMarkerKind::Merge,
        other => return Err(format!("marker inconnu: {other} (attendu split | merge)")),
    };
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;
    let mut s = helix_arc.lock().unwrap();
    if s.init_usb_settle_active() {
        return Err(format!(
            "move_matrix_routing_marker_usb ignoré (init USB ~{} ms)",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
    if !s.connected || s.preset_content_only {
        return Err(
            "move_matrix_routing_marker_usb ignoré (HX non prêt ou lecture preset en cours)"
                .to_string(),
        );
    }
    send_matrix_routing_marker_move(
        &mut s,
        kind,
        dest_boundary_col as u8,
        first_path2_col as u8,
        last_path2_col as u8,
        current_split_col as u8,
        current_merge_col as u8,
    )
}

#[tauri::command]
fn ensure_path2_dual_routing(
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<String, String> {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;
    let mut s = helix_arc.lock().unwrap();
    if s.init_usb_settle_active() {
        return Err(format!(
            "ensure_path2_dual_routing ignoré (init USB ~{} ms)",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
    if !s.connected || s.preset_content_only {
        return Err(
            "ensure_path2_dual_routing ignoré (HX non prêt ou lecture preset en cours)".to_string(),
        );
    }
    send_ensure_path2_dual_routing(&mut s)
}

#[tauri::command]
fn teardown_path2_dual_routing(
    state: tauri::State<Arc<Mutex<AppState>>>,
) -> Result<String, String> {
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()
    };
    let helix_arc = helix_arc.ok_or("HX non connecté")?;
    let mut s = helix_arc.lock().unwrap();
    if s.init_usb_settle_active() {
        return Err(format!(
            "teardown_path2_dual_routing ignoré (init USB ~{} ms)",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
    if !s.connected || s.preset_content_only {
        return Err(
            "teardown_path2_dual_routing ignoré (HX non prêt ou lecture preset en cours)".to_string(),
        );
    }
    send_teardown_path2_dual_routing(&mut s)
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
                return Err(format!(
                    "request_preset_content throttled ({} ms < {} ms)",
                    elapsed_ms,
                    PRESET_REQUEST_MIN_GAP_MS
                ));
            }
        }
        (
            app.active_preset,
            app.helix_state.clone().ok_or("HX non connecté")?,
        )
    };
    let mut s = helix_arc.lock().unwrap();
    helix::init_trace::trace_fmt(format_args!(
        "request_preset_content UI active_preset={active_preset}"
    ));
    if s.init_usb_settle_active() {
        return Err(format!(
            "Initialisation USB en cours (~{} ms) — aucune requête preset",
            helix::keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }
    if helix::scroll_model_pull::hw_model_usb_busy(&s) {
        return Err(
            "Scroll modèle hardware en cours — reportez request_preset_content".to_string(),
        );
    }
    if s.preset_content_only {
        if s.want_content_only_after_x2 {
            // activate_preset + attente écho MIDI PC — ne pas réinitialiser ni dupliquer.
            eprintln!(
                "[PresetDebug][request_preset_content] content_only déjà actif (attente MIDI PC)"
            );
            return Ok(());
        }
        // Session fantôme fréquente après rafales `probe_slot_model_usb` : content_only
        // reste posé sans RequestPreset actif → les relances UI étaient ignorées silencieusement.
        eprintln!(
            "[PresetDebug][request_preset_content] content_only fantôme — reset session avant relance"
        );
        s.preset_content_only = false;
        s.preset_data_ready = false;
        s.preset_data.clear();
    }
    // L'UI met à jour `active_preset` (ex. après `activate_preset` + MIDI PC) avant cette
    // commande, alors que `preset_index` côté Helix ne bouge qu'avec les paquets USB x2 ou
    // l'écoute MIDI — parfois après le dump, ou jamais si `RequestPresetName` est ignoré
    // pendant `preset_content_only`. Sans cette ligne, `get_active_preset_slots` reste à
    // None et la fenêtre models timeoute.
    s.preset_index = active_preset;
    s.preset_data_ready = false;
    s.preset_data.clear();
    s.last_slot_focus_capsule = std::array::from_fn(|_| None);
    s.slot_watch_prev = std::array::from_fn(|_| helix::slot_watch::SlotWatchSnapshot::default());
    s.slot_param_emit.clear();
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
        s.last_slot_focus_capsule = std::array::from_fn(|_| None);
        s.slot_watch_prev = std::array::from_fn(|_| helix::slot_watch::SlotWatchSnapshot::default());
    s.slot_param_emit.clear();
        // Re-synchronise l'état de requête (mêmes valeurs de base que RequestPreset::shutdown no-data).
        s.reset_preset_ed03_transaction_counter();
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
///
/// **Sans buffer preset** : si un [`SlotFocusInCapsule`] a été rempli pour ce slot (via
/// `sync_hardware_slot_focus_usb`) et que le `slot_bus` correspond, renvoie `Some(vec![])` pour
/// éviter un timeout côté front ; l’empreinte USB est dans `slotFocusParsed` sur la réponse sync.
#[tauri::command]
fn get_active_preset_slot_chain_param_values(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
) -> Option<Vec<preset_chain_params::ChainParamValue>> {
    if slot_index >= 16 {
        return None;
    }
    let slot_idx = slot_index as usize;
    let (active_preset, helix_arc) = {
        let app = state.lock().unwrap();
        (app.active_preset, app.helix_state.clone()?)
    };
    let expected_bus = kempline_index_to_slot_bus(slot_idx)?;
    let s = helix_arc.lock().unwrap();
    let dump_ok =
        s.preset_data_ready && !s.preset_data.is_empty() && s.preset_index == active_preset;
    if dump_ok {
        if let Some(seg) = kempline_assignable_segment_bytes(&s.preset_data, slot_idx) {
            if let Some(vals) = chain_param_values_for_assignable_segment(&seg) {
                if preset_debug_verbose_enabled() {
                    if let Some(ref cap) = s.last_slot_focus_capsule[slot_idx] {
                        if cap.slot_bus == expected_bus {
                            if let Some(off) = helix::slot_focus_in::find_anchor_subsequence(
                                &s.preset_data,
                                &cap.anchor12,
                            ) {
                                eprintln!(
                                    "[SlotFocus][corr] slot_index={} anchor12 @ preset offset {:#x} (len={})",
                                    slot_index,
                                    off,
                                    s.preset_data.len()
                                );
                            }
                        }
                    }
                }
                return Some(vals);
            }
        }
    }
    if let Some(ref cap) = s.last_slot_focus_capsule[slot_idx] {
        if cap.slot_bus == expected_bus {
            return Some(Vec::new());
        }
    }
    None
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
    linked_cab_info_from_assignable_chunk(seg)
}

/// Cab rattaché dans un segment assignable Amp+Cab : `[hex, catégorie, nom, model_id]`.
pub(crate) fn linked_cab_info_from_assignable_chunk(chunk: &[u8]) -> Option<[String; 4]> {
    if !is_amp_cab_assignable_chunk(chunk) {
        return None;
    }
    let blocks = preset_chain_params::parse_assignable_segment_param_blocks(chunk)?;
    let ids = augmented_module_ids_for_assignable_chunk(chunk, blocks.len());
    let cab_bi = catalog_cab_c219_block_index(&ids, blocks.len());
    cab_bi
        .and_then(|bi| block_chain_hex_for_c219(bi, &ids))
        .and_then(|h| cab_info_from_module_id(&h))
        .or_else(|| ids.iter().find_map(|id| cab_info_from_module_id(id)))
}

pub(crate) fn cab_hex_from_combined_module_wire(module_hex: &str) -> Option<String> {
    let h = module_hex.trim().to_lowercase();
    let (_, cab) = h.split_once("1a")?;
    let cab = cab.trim();
    if cab.is_empty() {
        None
    } else {
        Some(cab.to_string())
    }
}

/// Segment assignable scroll (`82 13 06|08` + `85188317`…) dans un dump IN pull.
fn assignable_chunk_for_hw_scroll_dump(buf: &[u8]) -> Option<&[u8]> {
    const SLOT_INFO_HEAD: [u8; 4] = SLOT_ASSIGNABLE_INFO_HEAD;
    for i in 0..buf.len().saturating_sub(4) {
        if buf[i] == 0x82
            && buf[i + 1] == 0x13
            && matches!(buf[i + 2], 0x06 | 0x08)
        {
            let rest = &buf[i + 2..];
            if assignable_chunk_has_slot_info_head(rest) {
                return Some(rest);
            }
        }
    }
    for start in 0..buf.len() {
        let b = *buf.get(start)?;
        if b != 0x06 && b != 0x08 {
            continue;
        }
        let rest = &buf[start..];
        if rest.len() < 8 {
            continue;
        }
        let head_near = rest.len().min(32);
        if !rest[..head_near].windows(4).any(|w| w == SLOT_INFO_HEAD) {
            continue;
        }
        return Some(rest);
    }
    None
}

/// Hex cab inféré depuis un dump scroll (blocs `c219`), comme `get_active_preset_slot_linked_cab`.
pub(crate) fn extract_linked_cab_hex_for_hw_scroll_dump(buf: &[u8]) -> Option<String> {
    let chunk = assignable_chunk_for_hw_scroll_dump(buf)?;
    linked_cab_info_from_assignable_chunk(chunk).map(|c| c[0].clone())
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DualSlotPartOut {
    chain_hex: String,
    category: String,
    name: String,
    model_id: String,
    values: Vec<preset_chain_params::ChainParamValue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DualSlotPartsOut {
    /// `amp_cab` ou `cab_dual`.
    kind: String,
    parts: Vec<DualSlotPartOut>,
}

fn module_quadruple_from_chain_hex(hex: &str) -> Option<[String; 4]> {
    let h = hex.trim().to_lowercase();
    if h.is_empty() {
        return None;
    }
    if let Some(cab) = cab_info_from_module_id(&h) {
        return Some(cab);
    }
    let entry = HX_CATALOG_MODULE_BY_HEX.get(&h)?;
    let model_id = MODEL_ID_BY_HEX.get(&h).cloned().unwrap_or_default();
    Some([
        h,
        entry[0].clone(),
        entry[1].clone(),
        model_id,
    ])
}

fn dual_slot_kind(seg: &[u8], blocks_len: usize) -> Option<&'static str> {
    if blocks_len < 2 {
        return None;
    }
    if cab_dual_c219_cab_hexes(seg)
        .map(|v| v.len() >= 2)
        .unwrap_or(false)
    {
        return Some("cab_dual");
    }
    let ids = augmented_module_ids_for_assignable_chunk(seg, blocks_len);
    let h0 = block_chain_hex_for_c219(0, &ids)?;
    let h1 = block_chain_hex_for_c219(1, &ids)?;
    let k0 = catalog_slot_kind_for_chain_hex(&h0);
    let k1 = catalog_slot_kind_for_chain_hex(&h1);
    if matches!(k0, CatalogSlotKind::CabLike) && matches!(k1, CatalogSlotKind::CabLike) {
        return Some("cab_dual");
    }
    if matches!(k0, CatalogSlotKind::AmpLike) && matches!(k1, CatalogSlotKind::CabLike) {
        return Some("amp_cab");
    }
    if is_amp_cab_assignable_chunk(seg) {
        return Some("amp_cab");
    }
    None
}

/// Indices des blocs `c219` pour les deux sous-modèles (ampli+cab ou cab dual).
fn dual_slot_block_indices(
    seg: &[u8],
    blocks_len: usize,
    kind: &str,
) -> Option<(usize, usize)> {
    if blocks_len < 2 {
        return None;
    }
    let ids = augmented_module_ids_for_assignable_chunk(seg, blocks_len);
    match kind {
        "cab_dual" => {
            let h = assignable_segment_hex_lower_body(seg);
            let types = extract_c219_argument_type_hexes(&h);
            let mut cab_indices: Vec<usize> = Vec::new();
            for (bi, t) in types.iter().enumerate() {
                let k = chain_hex_key_from_c219_argument_type(t);
                if matches!(
                    catalog_slot_kind_for_chain_hex(&k),
                    CatalogSlotKind::CabLike
                ) {
                    cab_indices.push(bi);
                }
            }
            if cab_indices.len() >= 2 {
                return Some((cab_indices[0], cab_indices[1]));
            }
            let mut cab_indices_legacy: Vec<usize> = Vec::new();
            for bi in 0..blocks_len {
                let Some(h) = block_chain_hex_for_c219(bi, &ids) else {
                    continue;
                };
                if matches!(
                    catalog_slot_kind_for_chain_hex(&h),
                    CatalogSlotKind::CabLike
                ) {
                    cab_indices_legacy.push(bi);
                }
            }
            if cab_indices_legacy.len() >= 2 {
                return Some((cab_indices_legacy[0], cab_indices_legacy[1]));
            }
            Some((0, 1))
        }
        "amp_cab" => {
            let mut amp_bi: Option<usize> = None;
            let mut cab_bi: Option<usize> = None;
            for bi in 0..blocks_len {
                let Some(h) = block_chain_hex_for_c219(bi, &ids) else {
                    continue;
                };
                let k = catalog_slot_kind_for_chain_hex(&h);
                if matches!(k, CatalogSlotKind::AmpLike) && amp_bi.is_none() {
                    amp_bi = Some(bi);
                }
                if matches!(k, CatalogSlotKind::CabLike) && cab_bi.is_none() {
                    cab_bi = Some(bi);
                }
            }
            match (amp_bi, cab_bi) {
                (Some(a), Some(c)) if a != c => Some((a, c)),
                _ if blocks_len >= 2 => Some((0, blocks_len - 1)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Tous les `chainHex` cab repérés dans l’ordre des blocs `c219` (vérité trame scroll / preset).
fn cab_dual_c219_cab_hexes(seg: &[u8]) -> Option<Vec<String>> {
    if !matches!(seg.first().copied(), Some(0x06 | 0x08)) {
        return None;
    }
    let h = assignable_segment_hex_lower_body(seg);
    let types = extract_c219_argument_type_hexes(&h);
    let cabs: Vec<String> = types
        .iter()
        .map(|t| chain_hex_key_from_c219_argument_type(t))
        .filter(|k| matches!(catalog_slot_kind_for_chain_hex(k), CatalogSlotKind::CabLike))
        .collect();
    if cabs.len() >= 2 {
        Some(cabs)
    } else {
        None
    }
}

/// `chainHex` d’un bloc `c219` à l’index `block_index` (vérité matérielle dans la trame).
pub(crate) fn chain_hex_for_c219_block_index(seg: &[u8], block_index: usize) -> Option<String> {
    let h = assignable_segment_hex_lower_body(seg);
    let types = extract_c219_argument_type_hexes(&h);
    types
        .get(block_index)
        .map(|t| chain_hex_key_from_c219_argument_type(t))
        .filter(|k| !k.is_empty())
}

/// Suffixe Cab 2 usine sur le fil `c319` (ex. Jazz Rivet `cd02d6` sur Soup Pro Ellipse dual).
pub(crate) const CAB_DUAL_FACTORY_CAB2_SUFFIX: &str = "cd02d6";

/// Cab 2 affiché : fil `c319` scroll en priorité ; `c219` seulement si le fil a encore le suffixe usine.
pub(crate) fn cab_dual_effective_cab2_hex(wire_cab2: &str, c219_cab2: Option<&str>) -> String {
    let wire = wire_cab2.trim().to_ascii_lowercase();
    if !wire.is_empty() {
        if let Some(c2) = c219_cab2.filter(|c| !c.trim().is_empty()) {
            let c2 = c2.trim().to_ascii_lowercase();
            if wire == CAB_DUAL_FACTORY_CAB2_SUFFIX && c2 != CAB_DUAL_FACTORY_CAB2_SUFFIX {
                return c2;
            }
        }
        return wire;
    }
    c219_cab2
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

/// 2ᵉ cab `c219` brut (sans fusion fil `c319`).
fn raw_cab_dual_cab2_c219_hex(seg: &[u8]) -> Option<String> {
    cab_dual_c219_cab_hexes(seg).and_then(|v| v.get(1).cloned())
}

/// Cab 2 d’un slot Cab dual : fil `c319` + override `c219` si suffixe usine encore sur le fil.
pub(crate) fn linked_cab_dual_cab2_hex_from_assignable_chunk(chunk: &[u8]) -> Option<String> {
    let (_, wire_cab2) = resolve_cab_dual_wire_pair(chunk)?;
    let c219_cab2 = raw_cab_dual_cab2_c219_hex(chunk);
    let effective = cab_dual_effective_cab2_hex(&wire_cab2, c219_cab2.as_deref());
    if effective.is_empty() {
        None
    } else {
        Some(effective)
    }
}

pub(crate) fn extract_cab_dual_cab2_hex_for_hw_scroll_dump(buf: &[u8]) -> Option<String> {
    let chunk = assignable_chunk_for_hw_scroll_dump(buf)?;
    raw_cab_dual_cab2_c219_hex(chunk)
}

/// Si le fil combiné dual a un Cab 2 périmé (`c319`) mais le `c219` scroll dit autre chose, corrige le fil.
pub(crate) fn reconcile_cab_dual_module_wire_with_cab2(module_hex: &str, cab2: &str) -> String {
    let combined = module_hex.trim().to_ascii_lowercase();
    let cab2 = cab2.trim().to_ascii_lowercase();
    if combined.is_empty() || cab2.is_empty() {
        return combined;
    }
    let Some((cab1, wire_cab2)) = combined.split_once("1a") else {
        return combined;
    };
    if cab1.is_empty() || wire_cab2.trim() == cab2 {
        return combined;
    }
    if !matches!(
        catalog_slot_kind_for_chain_hex(cab1),
        CatalogSlotKind::CabLike
    ) || !matches!(
        catalog_slot_kind_for_chain_hex(&cab2),
        CatalogSlotKind::CabLike
    ) {
        return combined;
    }
    format!("{cab1}1a{cab2}")
}

pub(crate) fn dual_slot_parts_from_segment(seg: &[u8]) -> Option<DualSlotPartsOut> {
    let blocks = preset_chain_params::parse_assignable_segment_param_blocks(seg)?;
    let kind = dual_slot_kind(seg, blocks.len())?;
    let (bi_a, bi_b) = dual_slot_block_indices(seg, blocks.len(), kind)?;
    let ids = augmented_module_ids_for_assignable_chunk(seg, blocks.len());
    let cab_dual = kind == "cab_dual";
    let cab_dual_wire = cab_dual.then(|| resolve_cab_dual_wire_pair(seg)).flatten();
    let cab_dual_hexes = cab_dual.then(|| cab_dual_c219_cab_hexes(seg)).flatten();
    let mut parts: Vec<DualSlotPartOut> = Vec::with_capacity(2);
    for (part_i, bi) in [bi_a, bi_b].into_iter().enumerate() {
        let hex = if cab_dual {
            if part_i == 0 {
                cab_dual_wire
                    .as_ref()
                    .map(|(c1, _)| c1.clone())
                    .or_else(|| cab_dual_hexes.as_ref().and_then(|v| v.get(0).cloned()))
                    .or_else(|| chain_hex_for_c219_block_index(seg, bi))
                    .or_else(|| block_chain_hex_for_c219(bi, &ids))
            } else {
                cab_dual_wire
                    .as_ref()
                    .map(|(_, c2)| {
                        cab_dual_effective_cab2_hex(
                            c2,
                            cab_dual_hexes.as_ref().and_then(|v| v.get(1).map(String::as_str)),
                        )
                    })
                    .filter(|s| !s.is_empty())
                    .or_else(|| cab_dual_hexes.as_ref().and_then(|v| v.get(1).cloned()))
                    .or_else(|| chain_hex_for_c219_block_index(seg, bi))
                    .or_else(|| block_chain_hex_for_c219(bi, &ids))
            }
        } else {
            block_chain_hex_for_c219(bi, &ids)
        }?;
        let [chain_hex, category, name, model_id] = module_quadruple_from_chain_hex(&hex)?;
        let values = blocks.get(bi)?.clone();
        parts.push(DualSlotPartOut {
            chain_hex,
            category,
            name,
            model_id,
            values,
        });
    }
    if parts.len() != 2 {
        return None;
    }
    Some(DualSlotPartsOut {
        kind: kind.to_string(),
        parts,
    })
}

/// Deux sous-modèles d’un slot (Amp+Cab ou Cab dual) : noms catalogue + valeurs par bloc `c219`.
#[tauri::command]
fn get_active_preset_slot_dual_parts(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
) -> Option<DualSlotPartsOut> {
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
    dual_slot_parts_from_segment(seg)
}

/// Cab 2 dual lu dans le **dernier pull scroll** (bloc `c219`), pas le défaut `bulkHex` JSON.
#[tauri::command]
fn get_hw_slot_cab_dual_cab2_hex(
    state: tauri::State<Arc<Mutex<AppState>>>,
    slot_index: u32,
) -> Option<String> {
    if slot_index >= 16 {
        return None;
    }
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone()?
    };
    let s = helix_arc.lock().unwrap();
    s.last_hw_cab_dual_cab2_hex[slot_index as usize].clone()
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

lazy_static! {
    /// ID module (`chainHexHint`) → `[catégorie, nom]`.
    /// Source unique : `HX_ModelUsbAssign.json` (`chainHexHint` + `category` + `name`).
    static ref HX_CATALOG_MODULE_BY_HEX: HashMap<String, [String; 2]> =
        crate::helix::model_catalog::module_by_hex_map().clone();
    /// ID module hex -> ID modèle (`HX_ModelUsbAssign.json` → `entries[].id`).
    static ref MODEL_ID_BY_HEX: HashMap<String, String> =
        crate::helix::model_catalog::model_id_by_hex_map().clone();
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

/// Octets d’en-tête valides pour le segment Kempline qui suit un vrai `82 13`.
const KEMPLINE_SEG_HEADER_AFTER_8213: [u8; 6] = [0x00, 0x01, 0x02, 0x03, 0x06, 0x08];

fn is_kempline_8213_delimiter(data: &[u8], at: usize) -> bool {
    if data.get(at) != Some(&0x82) || data.get(at + 1) != Some(&0x13) {
        return false;
    }
    data.get(at + 2)
        .map(|b| KEMPLINE_SEG_HEADER_AFTER_8213.contains(b))
        .unwrap_or(false)
}

/// Découpe le flux preset aux marqueurs [0x82, 0x13] (équivalent Kempline `split('8213')` sur l'hex).
///
/// Un `82 13` n’est retenu que si l’octet suivant est un en-tête de segment (`00`…`08`).
/// Les faux positifs dans les gros blobs Amp+Cab (paramètres) ne doivent pas fragmenter la
/// fenêtre fixe de 20 segments — sinon `w[9]`/`w[19]` ne sont plus `01`/`03` (échec slot 1).
fn split_preset_by_8213(data: &[u8]) -> Vec<&[u8]> {
    let mut chunks: Vec<&[u8]> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + 1 < data.len() {
        if is_kempline_8213_delimiter(data, i) {
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

/// Id modèle encodé `84 08 <octets id> 09` (loopers / blocs `fixed.models`, pas de `19…1a`).
fn extract_module_hex_from_8408_09(buf: &[u8]) -> Option<String> {
    for j in 0..buf.len().saturating_sub(4) {
        if buf[j] != 0x84 || buf[j + 1] != 0x08 {
            continue;
        }
        let id_start = j + 2;
        let rel = buf[id_start..].iter().position(|&b| b == 0x09)?;
        let id_bytes = &buf[id_start..id_start + rel];
        if id_bytes.is_empty() || id_bytes.len() > 4 {
            continue;
        }
        if let Some(hex) = chain_hex_key_from_raw_field_bytes(id_bytes) {
            return Some(hex);
        }
    }
    None
}

/// Loopers : segment `82 13 07` (scroll) ou `07` en tête (après split preset `82 13`).
fn is_looper_style_assignable_chunk(chunk: &[u8]) -> bool {
    chunk.windows(3).any(|w| w == [0x82, 0x13, 0x07]) || chunk.first() == Some(&0x07)
}

fn extract_module_hex_from_looper_style_assignable(buf: &[u8]) -> Option<String> {
    if !is_looper_style_assignable_chunk(buf) {
        return None;
    }
    extract_module_hex_from_8408_09(buf)
}

fn parsed_slot_from_module_hex(chunk: &[u8], module_hex: String) -> ParsedSlot {
    let (grid_x, grid_y) = extract_grid_xy_after_id(chunk, 0);
    if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(&module_hex) {
        return ParsedSlot {
            category: entry[0].clone(),
            name: entry[1].clone(),
            grid_x,
            grid_y,
            module_hex,
        };
    }
    ParsedSlot {
        category: String::from("Unknown"),
        name: module_hex.clone(),
        grid_x,
        grid_y,
        module_hex,
    }
}

const SLOT_ASSIGNABLE_INFO_HEAD: [u8; 4] = [0x85, 0x18, 0x83, 0x17];

/// `true` si le segment assignable porte l'en-tête slot `85 18 83 17` (effets, amp, cab, preamp…).
fn assignable_chunk_has_slot_info_head(chunk: &[u8]) -> bool {
    chunk.len() >= 8 && chunk.windows(4).any(|w| w == SLOT_ASSIGNABLE_INFO_HEAD)
}

/// Paire ampli+cab lue sur le fil — ordre de priorité aligné sur les captures scroll.
fn resolve_amp_cab_wire_pair(chunk: &[u8]) -> Option<(String, String)> {
    infer_amp_cab_hex_pair_from_c319_1a_09_tail(chunk)
        .or_else(|| infer_amp_cab_hex_pair_from_19_1a_09_markers(chunk))
        .or_else(|| {
            if is_amp_cab_assignable_chunk(chunk) {
                inferred_amp_cab_hex_keys(chunk)
            } else {
                None
            }
        })
}

/// Combine fil `amp1acab` : oui pour paires catalogue / `cd02xx`, non pour le token contrôle `06`.
fn should_use_amp_cab_combined_wire_hex(amp: &str, cab: &str) -> bool {
    let amp = amp.trim().to_ascii_lowercase();
    let cab = cab.trim().to_ascii_lowercase();
    if amp.is_empty() || cab.is_empty() || amp == cab {
        return false;
    }
    let amp_k = catalog_slot_kind_for_chain_hex(&amp);
    let cab_k = catalog_slot_kind_for_chain_hex(&cab);
    // Deux cabs : Cab dual, pas Amp+Cab (évite `cd03xx` + `cd03yy` classé à tort en ampli+cab).
    if matches!(amp_k, CatalogSlotKind::CabLike) && matches!(cab_k, CatalogSlotKind::CabLike) {
        return false;
    }
    if amp == "06" {
        return false;
    }
    let combined = format!("{amp}1a{cab}");
    if HX_CATALOG_MODULE_BY_HEX
        .get(&combined)
        .map(|e| e[0].eq_ignore_ascii_case("amp+cab"))
        .unwrap_or(false)
    {
        return true;
    }
    let amp_full = amp.starts_with("cd") && amp.len() >= 6;
    let cab_full = cab.starts_with("cd") && cab.len() >= 6;
    if amp_full && matches!(catalog_slot_kind_for_chain_hex(&cab), CatalogSlotKind::CabLike) {
        return true;
    }
    // Scroll asymétrique : token ampli court (`2b`) + cab catalogue (`cd0321`).
    amp.len() >= 2
        && amp.len() <= 4
        && cab_full
        && matches!(catalog_slot_kind_for_chain_hex(&cab), CatalogSlotKind::CabLike)
}

/// Chemin dédié **Amp+Cab** (cf. `docs/todo-scroll-hw.md` § extraction par type).
fn try_extract_amp_cab_combined_hex_from_chunk(chunk: &[u8]) -> Option<String> {
    let (amp, cab) = resolve_amp_cab_wire_pair(chunk)?;
    if !should_use_amp_cab_combined_wire_hex(&amp, &cab) {
        return None;
    }
    Some(format!("{amp}1a{cab}"))
}

/// Paire cab1+cab2 sur le fil preset — marqueurs `c319` / `19…1a…09` et types `c219`.
fn resolve_cab_dual_wire_pair(chunk: &[u8]) -> Option<(String, String)> {
    infer_cab_dual_hex_pair_from_c319_1a_09_tail(chunk)
        .or_else(|| infer_cab_dual_hex_pair_from_19_1a_09_markers(chunk))
        .or_else(|| {
            let h = assignable_segment_hex_lower_body(chunk);
            infer_cab_dual_hex_pair_from_segment_hex_body(&h)
        })
}

fn should_use_cab_dual_combined_wire_hex(cab1: &str, cab2: &str) -> bool {
    let cab1 = cab1.trim().to_ascii_lowercase();
    let cab2 = cab2.trim().to_ascii_lowercase();
    if cab1.is_empty() || cab2.is_empty() {
        return false;
    }
    if cab1 == cab2 {
        return matches!(
            catalog_slot_kind_for_chain_hex(&cab1),
            CatalogSlotKind::CabLike
        );
    }
    matches!(
        catalog_slot_kind_for_chain_hex(&cab1),
        CatalogSlotKind::CabLike
    ) && matches!(
        catalog_slot_kind_for_chain_hex(&cab2),
        CatalogSlotKind::CabLike
    )
}

/// Chemin dédié **Cab dual** : fil combiné `cab1Hex1acab2Hex` pour la grille preset.
fn try_extract_cab_dual_combined_hex_from_chunk(chunk: &[u8]) -> Option<String> {
    let (cab1, cab2) = resolve_cab_dual_wire_pair(chunk)?;
    if !should_use_cab_dual_combined_wire_hex(&cab1, &cab2) {
        return None;
    }
    Some(format!("{cab1}1a{cab2}"))
}

fn parsed_slot_from_cab_dual_combined(chunk: &[u8], combined: &str) -> ParsedSlot {
    let (grid_x, grid_y) = extract_grid_xy_after_id(chunk, 0);
    let cab1 = combined.split("1a").next().unwrap_or(combined);
    let (category, name) = HX_CATALOG_MODULE_BY_HEX
        .get(cab1)
        .map(|e| (e[0].clone(), e[1].clone()))
        .unwrap_or_else(|| (String::from("Cab"), combined.to_string()));
    ParsedSlot {
        category,
        name,
        grid_x,
        grid_y,
        module_hex: combined.to_string(),
    }
}

fn parsed_slot_from_amp_cab_combined(chunk: &[u8], combined: &str) -> ParsedSlot {
    let (grid_x, grid_y) = extract_grid_xy_after_id(chunk, 0);
    if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(combined) {
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
            module_hex: combined.to_string(),
        };
    }
    let amp_part = combined.split("1a").next().unwrap_or(combined);
    let (category, name) = HX_CATALOG_MODULE_BY_HEX
        .get(amp_part)
        .map(|e| (String::from("Amp+Cab"), e[1].clone()))
        .unwrap_or_else(|| (String::from("Amp+Cab"), combined.to_string()));
    ParsedSlot {
        category,
        name,
        grid_x,
        grid_y,
        module_hex: combined.to_string(),
    }
}

/// Résout `module_hex` depuis un segment assignable — route par famille (Cab dual, Amp+Cab, standard, looper).
fn extract_module_hex_from_assignable_chunk(chunk: &[u8]) -> Option<String> {
    if let Some(hex) = try_extract_cab_dual_combined_hex_from_chunk(chunk) {
        return Some(hex);
    }
    if let Some(hex) = try_extract_amp_cab_combined_hex_from_chunk(chunk) {
        return Some(hex);
    }
    if is_looper_style_assignable_chunk(chunk) {
        return extract_module_hex_from_looper_style_assignable(chunk);
    }
    let slot = extract_first_module_standard_from_assignable_chunk(chunk);
    if slot.module_hex.is_empty() {
        None
    } else {
        Some(slot.module_hex)
    }
}

/// Pull scroll USB : routeur par type de modèle (standard → Amp+Cab → looper).
pub(crate) fn extract_module_hex_for_hw_scroll_dump(buf: &[u8]) -> Option<String> {
    const SLOT_INFO_HEAD: [u8; 4] = SLOT_ASSIGNABLE_INFO_HEAD;
    // Les IN scroll (9c/53, 76–164 o) préfixent l'en-tête USB ; le slot assignable commence
    // par `82 13 06|08` (cf. capture Grammatico Brt juin 2026). Ne pas confondre avec un
    // `0x06` isolé dans l'en-tête (`… 06 00 8c …`).
    for i in 0..buf.len().saturating_sub(4) {
        if buf[i] == 0x82
            && buf[i + 1] == 0x13
            && matches!(buf[i + 2], 0x06 | 0x08)
        {
            let rest = &buf[i + 2..];
            if assignable_chunk_has_slot_info_head(rest) {
                if let Some(hex) = extract_module_hex_from_assignable_chunk(rest) {
                    return Some(hex);
                }
            }
        }
    }
    // Filet : segment `06|08` immédiat suivi de `85 18 83 17` (dumps courts sans `82 13`).
    for start in 0..buf.len() {
        let b = *buf.get(start)?;
        if b != 0x06 && b != 0x08 {
            continue;
        }
        let rest = &buf[start..];
        if rest.len() < 8 {
            continue;
        }
        let head_near = rest.len().min(32);
        if !rest[..head_near].windows(4).any(|w| w == SLOT_INFO_HEAD) {
            continue;
        }
        if let Some(hex) = extract_module_hex_from_assignable_chunk(rest) {
            return Some(hex);
        }
    }
    // Loopers / fixed : chemin séparé, uniquement si le format looper est présent.
    extract_module_hex_from_looper_style_assignable(buf)
}

/// Catégorie slot pour l'UI scroll quand le fil est Amp+Cab (hint explicite).
pub(crate) fn extract_category_hint_for_hw_scroll_dump(buf: &[u8]) -> Option<String> {
    const SLOT_INFO_HEAD: [u8; 4] = SLOT_ASSIGNABLE_INFO_HEAD;
    let try_chunk = |chunk: &[u8]| -> Option<String> {
        if try_extract_amp_cab_combined_hex_from_chunk(chunk).is_some() {
            return Some(String::from("Amp+Cab"));
        }
        let slot = extract_first_module_from_assignable_chunk(chunk);
        if slot.category.eq_ignore_ascii_case("amp+cab") {
            Some(slot.category)
        } else {
            None
        }
    };
    for i in 0..buf.len().saturating_sub(4) {
        if buf[i] == 0x82
            && buf[i + 1] == 0x13
            && matches!(buf[i + 2], 0x06 | 0x08)
        {
            let rest = &buf[i + 2..];
            if assignable_chunk_has_slot_info_head(rest) {
                if let Some(c) = try_chunk(rest) {
                    return Some(c);
                }
            }
        }
    }
    for start in 0..buf.len() {
        let b = *buf.get(start)?;
        if b != 0x06 && b != 0x08 {
            continue;
        }
        let rest = &buf[start..];
        if rest.len() < 8 {
            continue;
        }
        let head_near = rest.len().min(32);
        if !rest[..head_near].windows(4).any(|w| w == SLOT_INFO_HEAD) {
            continue;
        }
        if let Some(c) = try_chunk(rest) {
            return Some(c);
        }
    }
    None
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
                    0x00 => Some("6ccd0023"), // Split Y — OUT 25 / split select.json
                    0x01 => Some("6ccd0024"), // Split A/B
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
        let Some(rel09) = preset_chain_params::c219_param_delim_rel_in_slice(slice) else {
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

/// Clé ampli/cab plausible sur le fil (`cd0215`, …) — exclut les octets de contrôle (`06`, `08`).
fn looks_like_wire_module_chain_hex(key: &str) -> bool {
    let k = key.trim().to_ascii_lowercase();
    if k.len() < 4 {
        return false;
    }
    k.starts_with("cd") || HX_CATALOG_MODULE_BY_HEX.contains_key(&k)
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

/// Après `85188317c319` : paire `<amp> 1a <cab> 09` sans préfixe `0x19` (ex. WhoWatt en slot 0).
fn infer_amp_cab_hex_pair_from_c319_1a_09_tail(chunk: &[u8]) -> Option<(String, String)> {
    if !is_amp_cab_assignable_chunk(chunk) {
        return None;
    }
    let pos = chunk
        .windows(AMP_CAB_MARKER.len())
        .position(|w| w == AMP_CAB_MARKER)?;
    let mut cursor = pos + AMP_CAB_MARKER.len();
    while cursor + 2 < chunk.len() {
        let Some(rel_sep) = chunk.get(cursor..)?.iter().position(|&b| b == 0x1a) else {
            break;
        };
        let amp_end = cursor + rel_sep;
        let cab_start = amp_end + 1;
        let Some(rel09) = chunk.get(cab_start..)?.iter().position(|&b| b == 0x09) else {
            cursor = amp_end.saturating_add(1);
            continue;
        };
        let cab_end = cab_start + rel09;
        let amp_key = chain_hex_key_from_raw_field_bytes(&chunk[cursor..amp_end])?;
        let cab_key = chain_hex_key_from_raw_field_bytes(&chunk[cab_start..cab_end])?;
        let combined = format!("{amp_key}1a{cab_key}");
        if HX_CATALOG_MODULE_BY_HEX.contains_key(&combined) {
            return Some((amp_key, cab_key));
        }
        if matches!(
            catalog_slot_kind_for_chain_hex(&amp_key),
            CatalogSlotKind::AmpLike
        ) && matches!(
            catalog_slot_kind_for_chain_hex(&cab_key),
            CatalogSlotKind::CabLike
        ) {
            return Some((amp_key, cab_key));
        }
        // Marqueur `c319` + `<amp> 1a <cab> 09` : vérité fil (scroll Amp+Cab). Le 2ᵉ champ
        // est la cab DSP même si le catalogue indexe ce hex ailleurs (ex. `cd02bb`).
        if looks_like_wire_module_chain_hex(&amp_key)
            && looks_like_wire_module_chain_hex(&cab_key)
            && cab_key != amp_key
        {
            return Some((amp_key, cab_key));
        }
        // Scroll asymétrique : token ampli court (`2b`) + cab `cd02xx` après `c319`.
        if is_amp_cab_assignable_chunk(chunk)
            && cab_key != amp_key
            && amp_key != "06"
            && (2..=4).contains(&amp_key.len())
            && cab_key.starts_with("cd")
            && cab_key.len() >= 6
            && matches!(
                catalog_slot_kind_for_chain_hex(&cab_key),
                CatalogSlotKind::CabLike
            )
        {
            return Some((amp_key, cab_key));
        }
        cursor = cab_end.saturating_add(1);
    }
    None
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

/// Après `85188317c319` : paire `<cab1> 1a <cab2> 09` (Cab dual sur le même marqueur que Amp+Cab).
fn infer_cab_dual_hex_pair_from_c319_1a_09_tail(chunk: &[u8]) -> Option<(String, String)> {
    if !is_amp_cab_assignable_chunk(chunk) {
        return None;
    }
    let pos = chunk
        .windows(AMP_CAB_MARKER.len())
        .position(|w| w == AMP_CAB_MARKER)?;
    let mut cursor = pos + AMP_CAB_MARKER.len();
    while cursor + 2 < chunk.len() {
        let Some(rel_sep) = chunk.get(cursor..)?.iter().position(|&b| b == 0x1a) else {
            break;
        };
        let cab1_end = cursor + rel_sep;
        let cab2_start = cab1_end + 1;
        let Some(rel09) = chunk.get(cab2_start..)?.iter().position(|&b| b == 0x09) else {
            cursor = cab1_end.saturating_add(1);
            continue;
        };
        let cab2_end = cab2_start + rel09;
        let cab1_key = chain_hex_key_from_raw_field_bytes(&chunk[cursor..cab1_end])?;
        let cab2_key = chain_hex_key_from_raw_field_bytes(&chunk[cab2_start..cab2_end])?;
        if should_use_cab_dual_combined_wire_hex(&cab1_key, &cab2_key) {
            return Some((cab1_key, cab2_key));
        }
        cursor = cab2_end.saturating_add(1);
    }
    None
}

/// Cab dual depuis marqueurs `0x19 <cab1> 0x1a <cab2> 0x09`.
fn infer_cab_dual_hex_pair_from_19_1a_09_markers(chunk: &[u8]) -> Option<(String, String)> {
    if !matches!(chunk.first().copied(), Some(0x06 | 0x08)) {
        return None;
    }
    let mut cursor = 0usize;
    while cursor < chunk.len() {
        if chunk[cursor] != 0x19 {
            cursor += 1;
            continue;
        }
        let cab1_start = cursor + 1;
        let Some(rel_sep) = chunk.get(cab1_start..)?.iter().position(|&b| b == 0x1a) else {
            cursor += 1;
            continue;
        };
        let cab1_end = cab1_start + rel_sep;
        let cab2_start = cab1_end + 1;
        let Some(rel_end09) = chunk.get(cab2_start..)?.iter().position(|&b| b == 0x09) else {
            cursor = cab2_start;
            continue;
        };
        let cab2_end = cab2_start + rel_end09;
        let cab1_key = chain_hex_key_from_raw_field_bytes(&chunk[cab1_start..cab1_end])?;
        let cab2_key = chain_hex_key_from_raw_field_bytes(&chunk[cab2_start..cab2_end])?;
        if should_use_cab_dual_combined_wire_hex(&cab1_key, &cab2_key) {
            return Some((cab1_key, cab2_key));
        }
        cursor = cab2_end.saturating_add(1);
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
            let a_kind = catalog_slot_kind_for_chain_hex(a);
            let b_kind = catalog_slot_kind_for_chain_hex(b);
            if should_use_cab_dual_combined_wire_hex(a, b) {
                return Some(combined);
            }
            let a_known = HX_CATALOG_MODULE_BY_HEX.contains_key(a);
            let b_known = HX_CATALOG_MODULE_BY_HEX.contains_key(b);
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

/// Paire cab1 + cab2 à partir des types d’argument `c219` (deux champs CabLike).
fn infer_cab_dual_hex_pair_from_segment_hex_body(h: &str) -> Option<(String, String)> {
    let types = extract_c219_argument_type_hexes(h);
    if types.len() < 2 {
        return None;
    }
    let keys: Vec<String> = types
        .iter()
        .map(|t| chain_hex_key_from_c219_argument_type(t))
        .collect();
    let mut cab_keys: Vec<String> = keys
        .into_iter()
        .filter(|k| matches!(catalog_slot_kind_for_chain_hex(k), CatalogSlotKind::CabLike))
        .collect();
    if cab_keys.len() >= 2 {
        let cab1 = cab_keys.remove(0);
        let cab2 = cab_keys.remove(0);
        if should_use_cab_dual_combined_wire_hex(&cab1, &cab2) {
            return Some((cab1, cab2));
        }
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
    if blocks_len >= 2 {
        if let Some((cab1, cab2)) = resolve_cab_dual_wire_pair(seg) {
            let inferred = vec![cab1.clone(), cab2.clone()];
            if ids.len() == 1 && (ids[0] == cab1 || ids[0] == cab2) {
                return inferred;
            }
            if ids.is_empty() {
                return inferred;
            }
        }
    }
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
    if let Some(p) = infer_amp_cab_hex_pair_from_c319_1a_09_tail(chunk) {
        return Some(p);
    }
    infer_amp_cab_hex_pair_from_segment_hex_body(&h)
}

/// `ampHex1acabHex` **lu sur le fil** quand l’ID `19…1a` ne donne que l’ampli ou est absent
/// (format `c319` + `<amp> 1a <cab> 09`, etc.). La vérité matérielle prime sur le catalogue :
/// ex. scroll Grammatico Brt → `cd02151acd02bb` même si le JSON n’a que `cd02151acd0228`.
fn amp_cab_combined_chain_hex_for_slot_if_better(chunk: &[u8], extracted_id_hex: &str) -> Option<String> {
    let (amp, cab) = inferred_amp_cab_hex_keys(chunk)?;
    let amp = amp.trim().to_ascii_lowercase();
    let cab = cab.trim().to_ascii_lowercase();
    if amp.is_empty() || cab.is_empty() {
        return None;
    }
    if !looks_like_wire_module_chain_hex(&amp) || !looks_like_wire_module_chain_hex(&cab) {
        return None;
    }
    let combined = format!("{amp}1a{cab}");
    let ext = extracted_id_hex.trim().to_ascii_lowercase();
    if ext.is_empty() || ext == amp || ext == combined {
        return Some(combined);
    }
    None
}

/// Premier module dans un segment assignable — routeur (Cab dual, Amp+Cab, puis standard).
fn extract_first_module_from_assignable_chunk(chunk: &[u8]) -> ParsedSlot {
    if let Some(combined) = try_extract_cab_dual_combined_hex_from_chunk(chunk) {
        return parsed_slot_from_cab_dual_combined(chunk, &combined);
    }
    if let Some(combined) = try_extract_amp_cab_combined_hex_from_chunk(chunk) {
        return parsed_slot_from_amp_cab_combined(chunk, &combined);
    }
    extract_first_module_standard_from_assignable_chunk(chunk)
}

/// Chemin standard : effets, amp/cab/preamp seuls, loopers — `19…1a` et replis.
fn extract_first_module_standard_from_assignable_chunk(chunk: &[u8]) -> ParsedSlot {
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
    // Repli grille : Amp+Cab sans ID `19…1a` utilisable (`c319` + `<amp> 1a <cab> 09`, blocs `c219`, …).
    let h = assignable_segment_hex_lower_body(chunk);
    let inferred_pair = inferred_amp_cab_hex_keys(chunk)
        .or_else(|| infer_amp_cab_hex_pair_from_segment_hex_body(&h));
    if let Some((amp, cab)) = inferred_pair {
        let amp = amp.trim().to_ascii_lowercase();
        let cab = cab.trim().to_ascii_lowercase();
        if !amp.is_empty() && !cab.is_empty() {
            let combined = format!("{amp}1a{cab}");
            let (grid_x, grid_y) = extract_grid_xy_after_id(chunk, 0);
            if let Some(entry) = HX_CATALOG_MODULE_BY_HEX.get(&combined) {
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
            if is_amp_cab_assignable_chunk(chunk)
                && looks_like_wire_module_chain_hex(&amp)
                && looks_like_wire_module_chain_hex(&cab)
            {
                let (category, name) = HX_CATALOG_MODULE_BY_HEX
                    .get(&amp)
                    .map(|e| (String::from("Amp+Cab"), e[1].clone()))
                    .unwrap_or_else(|| (String::from("Amp+Cab"), combined.clone()));
                return ParsedSlot {
                    category,
                    name,
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
    if let Some(module_hex) = extract_module_hex_from_looper_style_assignable(chunk) {
        return parsed_slot_from_module_hex(chunk, module_hex);
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
            .map(|b| *b == 0x06 || *b == 0x08 || *b == 0x07)
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
        if fb != 0x06 && fb != 0x08 && fb != 0x07 {
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
        if std::env::var("USB_PACKET_TRACE_DELTA_ONLY")
            .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
            .unwrap_or(false)
        {
            set_usb_packet_trace_delta_only(false);
        }
        if let Ok(v) = std::env::var("USB_PACKET_TRACE_MAX_LEN") {
            if let Ok(n) = v.parse::<u32>() {
                set_usb_packet_trace_max_len(n);
            }
        }
        if std::env::var("USB_PACKET_TRACE_BOOT")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            set_usb_packet_trace_defer_until_ready(false);
        }
        let max_len = usb_packet_trace_max_len()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "none".to_string());
        eprintln!(
            "[UsbTrace] armé — USB_PACKET_TRACE=1 (delta_only={}, max_len={}, defer_until_ready={})",
            usb_packet_trace_delta_only(),
            max_len,
            !std::env::var("USB_PACKET_TRACE_BOOT")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        );
    }
    if std::env::var("USB_IO_DIAG").map(|v| v == "1").unwrap_or(false) {
        set_usb_io_diag_enabled(true);
    }
    helix::init_trace::init_from_env();
    helix::scroll_model_pull::init_from_env();
    helix::keep_alive::init_from_env();

    let app_state = Arc::new(Mutex::new(AppState::default()));
    let app_state_clone = Arc::clone(&app_state);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    save_window_layout(&window.app_handle());
                    graceful_helix_close(&window.app_handle());
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
            save_preset_to_hardware,
            activate_preset,
            switch_active_hardware_slot,
            switch_active_hardware_special_slot,
            probe_hardware_slot_focus_usb,
            sync_hardware_slot_focus_usb,
            request_preset_content,
            is_helix_usb_init_settling,
            force_recover_preset_reader,
            probe_live_param_write,
            probe_slot_model_usb,
            write_live_param,
            focus_amp_cab_usb_part,
            focus_cab_dual_usb_part,
            write_live_param_midi_cc,
            write_path1_input_source,
            get_path1_input_source_wire_value,
            write_path1_split_type,
            get_path1_split_type_wire_value,
            move_matrix_slot_usb,
            move_matrix_routing_marker_usb,
            ensure_path2_dual_routing,
            teardown_path2_dual_routing,
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
            get_active_preset_slot_dual_parts,
            get_hw_slot_cab_dual_cab2_hex,
            helix::hx_edit_console_cmds::hx_console_change_cab2,
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
                    let _ = main_window.set_size(Size::Logical(LogicalSize::new(
                        1293.0, 1143.0,
                    )));
                    let _ = main_window
                        .set_position(Position::Logical(LogicalPosition::new(80.0, 80.0)));
                }
                let _ = main_window.set_focus();
            }

            let ah = app.app_handle().clone();
            let state = app_state_clone;
            thread::spawn(move || {
                start_monitor(state, ah);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ===========================================================
// Surveille le branchement USB et lance/arrête la connexion
// ===========================================================
fn start_monitor(app_state: Arc<Mutex<AppState>>, app_handle: tauri::AppHandle) {
    let stop_monitor = Arc::new(AtomicBool::new(false));
    let state_for_connect = Arc::clone(&app_state);
    let state_for_lost = Arc::clone(&app_state);
    let ah_for_helix = app_handle.clone();
    let ah_for_lost = app_handle.clone();
    let helix_attached = Arc::new(AtomicBool::new(false));
    let helix_connecting = Arc::new(AtomicBool::new(false));
    let helix_session_busy = Arc::new(AtomicBool::new(false));

    helix::usb_monitor::start_monitor(
        Arc::new(Mutex::new(HelixState::new())),
        Arc::clone(&stop_monitor),
        Arc::clone(&helix_attached),
        Arc::clone(&helix_connecting),
        Arc::clone(&helix_session_busy),
        Arc::new(move || {
            helix::init_trace::mark_origin("hw_detected_usb_poll");
            let state = Arc::clone(&state_for_connect);
            let ah = ah_for_helix.clone();
            let attached = Arc::clone(&helix_attached);
            let connecting = Arc::clone(&helix_connecting);
            let session_busy = Arc::clone(&helix_session_busy);
            {
                let mut app = state.lock().unwrap();
                app.connection_issue_hint = None;
            }
            thread::spawn(move || {
                start_helix(state, ah, attached, connecting, session_busy);
            });
        }),
        Arc::new(move || {
            let mut app = state_for_lost.lock().unwrap();
            disconnect_helix_session(&mut app, &ah_for_lost, "usb_unplugged");
        }),
    );
}

// ===========================================================
// Connexion complète au HX et boucle de traitement
// ===========================================================
fn start_helix(
    app_state: Arc<Mutex<AppState>>,
    app_handle: tauri::AppHandle,
    helix_attached: Arc<AtomicBool>,
    helix_connecting: Arc<AtomicBool>,
    helix_session_busy: Arc<AtomicBool>,
) {
    {
        let (prev_busy, prev_stop) = {
            let app = app_state.lock().unwrap();
            (
                app.helix_session_busy.clone(),
                app.helix_session_stop.clone(),
            )
        };
        if let Some(busy) = prev_busy.as_ref() {
            wait_previous_helix_session_end(busy, prev_stop.as_deref());
        }
    }
    helix_session_busy.store(true, Ordering::SeqCst);
    helix::init_trace::trace("start_helix BEGIN (open USB, claim, flush)");
    // Ouvrir le premier device supporté trouvé (HX Stomp XL / Stomp / Floor / LT).
    let (device_name, handle) = match find_supported_device_handle_with_retry() {
        Some(tuple) => tuple,
        None => {
            eprintln!("[Helix] Aucun device HX supporté trouvé (open USB échoué — nouvel essai au prochain poll)");
            helix::init_trace::trace(
                "start_helix ABORT: device visible mais device.open() a échoué (busy/permissions?)",
            );
            helix_connecting.store(false, Ordering::SeqCst);
            helix_attached.store(false, Ordering::SeqCst);
            helix_session_busy.store(false, Ordering::SeqCst);
            let mut app = app_state.lock().unwrap();
            app.connection_issue_hint = Some(
                "HX visible mais accès USB impossible (autre app, permissions, ou énumération)".to_string(),
            );
            return;
        }
    };
    helix_connecting.store(false, Ordering::SeqCst);
    helix_attached.store(true, Ordering::SeqCst);
    helix::init_trace::trace("start_helix USB open OK");
    eprintln!("[Helix] Device connecté: {}", device_name);
    helix::init_trace::trace_fmt(format_args!("start_helix device={device_name}"));

    // Réclamer l'interface USB
    if let Err(e) = handle.claim_interface(0) {
        eprintln!("[Helix] erreur claim_interface : {}", e);
        helix::init_trace::trace_fmt(format_args!("start_helix ABORT claim_interface(0): {e}"));
        helix_attached.store(false, Ordering::SeqCst);
        helix_session_busy.store(false, Ordering::SeqCst);
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
    helix::init_trace::trace("start_helix flush 0x81 done");
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
    let session_stop = Arc::new(AtomicBool::new(false));
    let stop_listener = Arc::new(AtomicBool::new(false));
    {
        let mut app = app_state.lock().unwrap();
        app.helix_session_stop = Some(Arc::clone(&session_stop));
        app.helix_stop_listener = Some(Arc::clone(&stop_listener));
        app.helix_session_busy = Some(Arc::clone(&helix_session_busy));
    }
    helix::usb_writer::start_writer(Arc::clone(&handle), usb_rx);

    // -- Mode initial : Connect --
    let current_mode: Arc<Mutex<Box<dyn helix::Mode>>> =
        Arc::new(Mutex::new(Box::new(Connect::new())));

    {
        let mut s = state.lock().unwrap();
        s.connected = true;
        let mut m = current_mode.lock().unwrap();
        helix::init_trace::trace_mode_switch("Connect", "::start");
        m.start(&mut s);
    }

    // -- Démarrer usb_listener --
    helix::usb_listener::start_listener(
        Arc::clone(&handle),
        Arc::clone(&state),
        Arc::clone(&current_mode),
        Arc::clone(&stop_listener),
        Arc::clone(&session_stop),
        Some(app_handle.clone()),
    );

    // -- KeepAlive manager --
    let ka_manager = Arc::new(KeepAliveManager::new());
    {
        let ka = Arc::clone(&ka_manager);
        let s  = Arc::clone(&state);
        thread::spawn(move || {
            loop {
                match ka_rx.recv() {
                    Ok(KeepAliveCommand::StartOrdered) => {
                        ka.start_ordered(Arc::clone(&s));
                    }
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
        let session_stop_midi = Arc::clone(&session_stop);
        thread::spawn(move || {
            let mut buf = vec![0u8; 64];
            let mut seen_fingerprints: HashSet<Vec<u8>> = HashSet::new();
            let mut suppressed_repeats: u64 = 0;
            loop {
                if session_stop_midi.load(Ordering::SeqCst) {
                    break;
                }
                match handle_midi.read_bulk(0x82, &mut buf, Duration::from_millis(500)) {
                    Ok(n) if n >= 4 => {
                        {
                            let mut s = state_midi.lock().unwrap();
                            // Certains firmwares routent des paquets paramètre sur le flux MIDI USB (0x82).
                            // On tente l'ingestion ED03 ici aussi pour alimenter le mode live `in_echo_strict`.
                            s.ingest_ed03_param_echo(&buf[..n]);
                        }
                        if usb_packet_trace_active() {
                            let delta_only = usb_packet_trace_delta_only();
                            let data = buf[..n].to_vec();
                            let fingerprint = helix::usb_trace_fingerprint(&data);
                            let log_in = if delta_only {
                                if !seen_fingerprints.insert(fingerprint) {
                                    suppressed_repeats = suppressed_repeats.saturating_add(1);
                                    false
                                } else {
                                    if suppressed_repeats > 0 {
                                        eprintln!(
                                            "[UsbTrace][IN  0x82] known patterns suppressed total={}",
                                            suppressed_repeats
                                        );
                                        suppressed_repeats = 0;
                                    }
                                    true
                                }
                            } else {
                                true
                            };
                            if log_in && usb_packet_trace_should_log(&data) {
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
                    Err(rusb::Error::NoDevice) => break,
                    Err(_) => {}
                }
            }
        });
    }

    // -- Boucle principale : changements de mode --
    loop {
        if session_stop.load(Ordering::SeqCst) {
            let mut app = app_state.lock().unwrap();
            disconnect_helix_session(&mut app, &app_handle, "usb_lost");
            break;
        }
        match mode_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(ModeRequest::RequestPreset(content_only)) => {
                helix::init_trace::trace_mode_switch(
                    "RequestPreset",
                    &format!(" content_only={content_only}"),
                );
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
                {
                    let mut app = app_state.lock().unwrap();
                    sync_app_presets_from_helix(&mut app, &s);
                }
                // Restaurer preset_content_only depuis l'intention du message, APRÈS shutdown()
                // qui le remet toujours à false — c'est la correction de la race condition.
                s.preset_content_only = effective_content_only;
                // Incrémenter la génération : tout StandardPresetRead(gen) avec l'ancienne
                // génération sera ignoré comme orphelin (watchdog/timer d'une lecture précédente).
                s.preset_read_generation = s.preset_read_generation.wrapping_add(1);
                s.set_preset_usb_read_modes_active(true);
                *m = Box::new(RequestPreset::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::RequestPresetName) => {
                helix::init_trace::trace_mode_switch("RequestPresetName", "");
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
                if !s.editor_ready {
                    eprintln!(
                        "[PresetDebug][ModeLoop] ignore RequestPresetName (editor_ready=false)"
                    );
                    continue;
                }
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                s.set_preset_usb_read_modes_active(true);
                *m = Box::new(RequestPresetName::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::FinalizePresetNames) => {
                helix::init_trace::trace_mode_switch("FinalizePresetNames", "");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                {
                    let mut app = app_state.lock().unwrap();
                    sync_app_presets_from_helix(&mut app, &s);
                }
                if s.preset_content_only || s.want_content_only_after_x2 {
                    eprintln!(
                        "[PresetDebug][ModeLoop] FinalizePresetNames: liste prête, RequestPresetName différé (content_only={} want_x2={})",
                        s.preset_content_only, s.want_content_only_after_x2
                    );
                    continue;
                }
                if !s.got_preset_names {
                    eprintln!(
                        "[PresetDebug][ModeLoop] FinalizePresetNames: got_preset_names=false — retry RequestPresetNames"
                    );
                    s.set_preset_usb_read_modes_active(true);
                    *m = Box::new(RequestPresetNames::new());
                    m.start(&mut s);
                    continue;
                }
                s.set_preset_usb_read_modes_active(true);
                *m = Box::new(RequestPresetName::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::AwaitPostBootstrapSettle) => {
                helix::init_trace::trace_mode_switch("AwaitPostBootstrapSettle", "");
                let should_spawn = {
                    let mut s = state.lock().unwrap();
                    if s.post_arm_sequence_started {
                        false
                    } else {
                        s.post_arm_sequence_started = true;
                        true
                    }
                };
                if should_spawn {
                    let mode_tx = {
                        let s = state.lock().unwrap();
                        s.mode_tx.clone()
                    };
                    let gate_rx = {
                        let mut s = state.lock().unwrap();
                        s.post_ef_gate_rx.take()
                    };
                    if let Some(tx) = mode_tx {
                        if let Some(rx) = gate_rx {
                            helix::init_trace::trace(
                                "AwaitPostBootstrapSettle — spawn_post_gate_sequence (gate événementielle)",
                            );
                            eprintln!("[Helix] spawn_post_gate_sequence");
                            helix::amorcage::spawn_post_gate_sequence(
                                Arc::clone(&state),
                                tx,
                                rx,
                            );
                        } else {
                            helix::init_trace::trace(
                                "AwaitPostBootstrapSettle — spawn_post_arm_sequence (fallback timer)",
                            );
                            eprintln!("[Helix] spawn_post_arm_sequence (pas de gate_rx)");
                            helix::amorcage::spawn_post_arm_sequence(Arc::clone(&state), tx);
                        }
                    } else {
                        eprintln!(
                            "[Helix] ERREUR: mode_tx absent — post-ARM / RequestPresetNames non démarré"
                        );
                        let mut s = state.lock().unwrap();
                        s.post_arm_sequence_started = false;
                    }
                }
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                s.set_preset_usb_read_modes_active(false);
                *m = Box::new(AwaitPostBootstrapSettle::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::RequestPresetNames) => {
                helix::init_trace::trace_mode_switch("RequestPresetNames", "");
                eprintln!("[Helix] mode RequestPresetNames");
                let mut s = state.lock().unwrap();
                if !s.editor_ready {
                    eprintln!(
                        "[PresetDebug][ModeLoop] ignore RequestPresetNames (editor_ready=false)"
                    );
                    continue;
                }
                s.end_init_usb_settle();
                s.set_preset_usb_read_modes_active(true);
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][init] fin fenêtre settle → RequestPresetNames (requêtes host autorisées)"
                    );
                }
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(RequestPresetNames::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::Standard) => {
                helix::init_trace::trace_mode_switch("Standard", "");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                let need_preset_content =
                    s.got_preset_names && !s.preset_data_ready && s.preset_data.is_empty();
                {
                    let mut app = app_state.lock().unwrap();
                    sync_app_presets_from_helix(&mut app, &s);
                    s.just_fetched_preset_names = false;
                    s.got_preset = false;
                    app.connection_issue_hint = None;
                }
                if need_preset_content {
                    s.preset_content_only = true;
                    s.switch_mode(ModeRequest::RequestPreset(true));
                } else {
                    s.set_preset_usb_read_modes_active(false);
                    *m = Box::new(Standard);
                    m.start(&mut s);
                }
            }
            Ok(ModeRequest::StandardPresetRead(gen)) => {
                helix::init_trace::trace_mode_switch("StandardPresetRead", &format!(" gen={gen}"));
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
                    sync_app_presets_from_helix(&mut app, &s);
                    s.just_fetched_preset_names = false;
                    s.got_preset = false;
                    app.connection_issue_hint = None;
                }
                s.set_preset_usb_read_modes_active(false);
                *m = Box::new(Standard);
                m.start(&mut s);
            }
            Ok(ModeRequest::Connect) => {
                helix::init_trace::trace_mode_switch("Connect", " (re-entry)");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                s.set_preset_usb_read_modes_active(false);
                *m = Box::new(Connect::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::ReconfigureX1) => {
                helix::init_trace::trace_mode_switch("ReconfigureX1", "");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                s.set_preset_usb_read_modes_active(false);
                *m = Box::new(ReconfigureX1::new());
                m.start(&mut s);
            }
            Err(RecvTimeoutError::Timeout) => {
                let connected = state.lock().unwrap().connected;
                if session_stop.load(Ordering::SeqCst) || !connected {
                    let mut app = app_state.lock().unwrap();
                    disconnect_helix_session(&mut app, &app_handle, "usb_lost");
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
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
    session_stop.store(true, Ordering::SeqCst);
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

    helix_attached.store(false, Ordering::SeqCst);
    {
        let mut app = app_state.lock().unwrap();
        if app.helix_state.is_some() || app.connected_device_name.is_some() {
            disconnect_helix_session(&mut app, &app_handle, "session_ended");
        }
        app.helix_session_stop = None;
        app.helix_stop_listener = None;
        app.helix_session_busy = None;
    }
    helix_session_busy.store(false, Ordering::SeqCst);
}

const USB_OPEN_RETRY_ATTEMPTS: u32 = 20;
const USB_OPEN_RETRY_DELAY_MS: u64 = 150;

fn find_supported_device_handle_with_retry(
) -> Option<(&'static str, Arc<rusb::DeviceHandle<rusb::GlobalContext>>)> {
    for attempt in 1..=USB_OPEN_RETRY_ATTEMPTS {
        if let Some(tuple) = find_supported_device_handle_once() {
            if attempt > 1 {
                helix::init_trace::trace_fmt(format_args!(
                    "device.open OK at attempt {attempt}/{USB_OPEN_RETRY_ATTEMPTS}"
                ));
            }
            return Some(tuple);
        }
        if attempt < USB_OPEN_RETRY_ATTEMPTS {
            helix::init_trace::trace_fmt(format_args!(
                "device.open retry {attempt}/{USB_OPEN_RETRY_ATTEMPTS} in {USB_OPEN_RETRY_DELAY_MS}ms"
            ));
            thread::sleep(Duration::from_millis(USB_OPEN_RETRY_DELAY_MS));
        }
    }
    None
}

fn find_supported_device_handle_once(
) -> Option<(&'static str, Arc<rusb::DeviceHandle<rusb::GlobalContext>>)> {
    let devices = rusb::devices().ok()?;
    for device in devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        let vid = desc.vendor_id();
        let pid = desc.product_id();
        if let Some((name, _, _)) = SUPPORTED_DEVICES
            .iter()
            .find(|(_, ev, ep)| vid == *ev && pid == *ep)
        {
            match device.open() {
                Ok(handle) => return Some((*name, Arc::new(handle))),
                Err(e) => {
                    helix::init_trace::trace_fmt(format_args!(
                        "device.open failed name={name} vid={vid:04x} pid={pid:04x} err={e}"
                    ));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod split_preset_by_8213_tests {
    use super::split_preset_by_8213;

    #[test]
    fn ignores_8213_not_followed_by_segment_header() {
        let data = vec![
            0x08, 0x00, 0x82, 0x13, 0xff, 0x00, // faux positif (suivi de 0xff)
            0x82, 0x13, 0x08, 0x14, 0xc0,       // vrai séparateur → slot vide
        ];
        let segs = split_preset_by_8213(&data);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], &[0x08, 0x00, 0x82, 0x13, 0xff, 0x00]);
        assert_eq!(segs[1], &[0x08, 0x14, 0xc0]);
    }

    #[test]
    fn still_splits_on_real_kempline_boundaries() {
        let data = vec![0x00, 0xaa, 0x82, 0x13, 0x08, 0x14, 0xc0];
        let segs = split_preset_by_8213(&data);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], &[0x00, 0xaa]);
        assert_eq!(segs[1], &[0x08, 0x14, 0xc0]);
    }
}

#[cfg(test)]
mod hw_scroll_dump_module_hex_tests {
    use super::extract_module_hex_for_hw_scroll_dump;

    #[test]
    fn infers_amp_cab_combined_from_dual_slot_without_c219() {
        let seg = vec![
            0x08, 0x85, 0x18, 0x83, 0x17, 0xc2, 0x19, 0xcd, 0x02, 0x17, 0x1a, 0xcd, 0x02, 0x28,
            0x09, 0x10, 0x0a, 0xc3,
        ];
        assert_eq!(
            extract_module_hex_for_hw_scroll_dump(&seg).as_deref(),
            Some("cd02171acd0228")
        );
    }

    #[test]
    fn looper_scroll_dump_8213_07_8408_cc99() {
        let dump: Vec<u8> = vec![
            0x44, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x43, 0x00, 0x04, 0x4f, 0x03,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x34, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03,
            0xf6, 0x67, 0x00, 0x68, 0x82, 0x0d, 0x01, 0x18, 0x82, 0x13, 0x07, 0x14, 0x84, 0x08,
            0xcc, 0x99, 0x09, 0x16, 0x0a, 0xc2, 0x07, 0x83, 0x02, 0x04, 0x03, 0x04, 0x04, 0x94,
            0xca, 0x00, 0x00, 0x00, 0x00, 0xca, 0x00, 0x00, 0x00, 0x00, 0xca, 0x41, 0xa0, 0x00,
            0x00, 0xca, 0x46, 0x9c, 0x40, 0x00,
        ];
        assert_eq!(
            extract_module_hex_for_hw_scroll_dump(&dump).as_deref(),
            Some("cc99")
        );
    }

    #[test]
    fn looper_scroll_dump_8213_07_8408_cd0268() {
        let dump: Vec<u8> = vec![
            0x67, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x40, 0x00, 0x04, 0x3c, 0x03,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x57, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03,
            0xf5, 0x67, 0x00, 0x68, 0x82, 0x0d, 0x01, 0x18, 0x82, 0x13, 0x07, 0x14, 0x84, 0x08,
            0xcd, 0x02, 0x68, 0x09, 0x16, 0x0a, 0xc2, 0x07,
        ];
        assert_eq!(
            extract_module_hex_for_hw_scroll_dump(&dump).as_deref(),
            Some("cd0268")
        );
    }
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

    #[test]
    fn extract_c219_types_cd0209_not_truncated_to_cd02() {
        let h = "85188317c219cd02091aff09110ac30b830206031504dc0006";
        let types = super::extract_c219_argument_type_hexes(h);
        assert_eq!(types.len(), 1);
        assert_eq!(
            super::chain_hex_key_from_c219_argument_type(&types[0]),
            "cd0209"
        );
    }

    #[test]
    fn reconcile_cab_dual_wire_replaces_stale_c319_cab2_with_c219() {
        let out = super::reconcile_cab_dual_module_wire_with_cab2("cd031c1acd02d6", "cd031c");
        assert_eq!(out, "cd031c1acd031c");
    }

    #[test]
    fn cab_dual_effective_cab2_prefers_wire_when_c219_absent() {
        assert_eq!(super::cab_dual_effective_cab2_hex("cd02d6", None), "cd02d6");
    }

    #[test]
    fn cab_dual_effective_cab2_overrides_stale_factory_suffix_from_c219() {
        assert_eq!(
            super::cab_dual_effective_cab2_hex("cd02d6", Some("cd031c")),
            "cd031c"
        );
        assert_eq!(
            super::cab_dual_effective_cab2_hex("cd031c", Some("cd02d6")),
            "cd031c"
        );
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

    /// Scroll Amp+Cab GrammaticoLG Brt (capture USB 164 o, juin 2026) : `c319` puis
    /// `cd0215 1a cd02bb 09` sans préfixe `19` — le combiné fil n’est pas dans le catalogue
    /// (`cd02151acd0228` seulement en Preamp) mais doit quand même sortir du parseur.
    #[test]
    fn scroll_grammatico_brt_amp_cab_wire_combined_without_catalog_entry() {
        const IN164: &[u8] = &[
            0x9c, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x57, 0x00, 0x04, 0x29, 0x03,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x8c, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03,
            0xf4, 0x67, 0x00, 0x68, 0x82, 0x0d, 0x01, 0x18, 0x82, 0x13, 0x06, 0x14, 0x85, 0x18,
            0x83, 0x17, 0xc3, 0x19, 0xcd, 0x02, 0x15, 0x1a, 0xcd, 0x02, 0xbb, 0x09, 0x21, 0x0a,
            0xc3, 0x0b, 0x83, 0x02, 0x0c, 0x03, 0x0c, 0x04, 0x9c, 0xca, 0x00, 0x00, 0x00, 0x00,
            0xca, 0x3f, 0x00, 0x00, 0x00, 0xca, 0x3f, 0x00, 0x00, 0x00, 0xca, 0x3f, 0x33, 0x33,
            0x33, 0xca, 0x3f, 0x00, 0x00, 0x00, 0xca, 0x3f, 0x40, 0x00, 0x00, 0xca, 0x3f, 0x54,
            0x7a, 0xe1, 0xca, 0x3f, 0x11, 0xeb, 0x85, 0xca, 0x3f, 0x26, 0x66, 0x66, 0xca, 0x3f,
            0x42, 0x8f, 0x5c, 0xca, 0x3e, 0xe6, 0x66, 0x66, 0xca, 0x3e, 0xd1, 0xeb, 0x85, 0x0c,
            0x83, 0x02, 0x07, 0x03, 0x07, 0x04, 0x97, 0x0b, 0xca, 0x3e, 0x42, 0x8f, 0x5c, 0xca,
            0x40, 0xe0, 0x00, 0x00, 0xca, 0x00, 0x00, 0x00, 0x00, 0xca, 0x41, 0x9f, 0x33, 0x33,
            0xca, 0x46, 0x9d, 0x08, 0x00, 0xca, 0x00, 0x00, 0x00, 0x00,
        ];
        let hex = super::extract_module_hex_for_hw_scroll_dump(IN164).expect("scroll dump");
        assert_eq!(hex, "cd02151acd02bb");
        let seg_off = IN164
            .windows(3)
            .position(|w| w[0] == 0x82 && w[1] == 0x13 && w[2] == 0x06)
            .expect("82 13 06")
            + 2;
        let slot = extract_first_module_from_assignable_chunk(&IN164[seg_off..]);
        assert_eq!(slot.module_hex, "cd02151acd02bb");
        assert_eq!(slot.category, "Amp+Cab");
        assert_eq!(slot.name, "GrammaticoLG Brt");
    }

    /// US Small Tweed Amp+Cab scroll (capture 164 o) : `c319` puis `2b 1a cd0321 09`
    /// (1ᵉ champ = token ampli court, 2ᵉ = cab catalogue) → combiné fil `2b1acd0321`.
    #[test]
    fn scroll_us_small_tweed_amp_cab_asymmetric_c319_pair() {
        const IN164: &[u8] = &[
            0x9a, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x3e, 0x00, 0x04, 0x16, 0x03,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x8a, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03,
            0xf3, 0x67, 0x00, 0x68, 0x82, 0x0d, 0x01, 0x18, 0x82, 0x13, 0x06, 0x14, 0x85, 0x18,
            0x83, 0x17, 0xc3, 0x19, 0x2b, 0x1a, 0xcd, 0x03, 0x21, 0x09, 0x21, 0x0a, 0xc3, 0x0b,
        ];
        let hex = super::extract_module_hex_for_hw_scroll_dump(IN164).expect("scroll");
        assert_eq!(hex, "2b1acd0321");
        let seg_off = IN164
            .windows(3)
            .position(|w| w[0] == 0x82 && w[1] == 0x13 && w[2] == 0x06)
            .expect("82 13 06")
            + 2;
        let slot = extract_first_module_from_assignable_chunk(&IN164[seg_off..]);
        assert_eq!(slot.module_hex, "2b1acd0321");
        assert_eq!(slot.category, "Amp+Cab");
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

/// Références Wireshark **HX Edit**, même preset « Preset Test », changement de slot **sans**
/// modification du contenu des slots (`Slot1_to_slot2_PresetTest_HXEdit.json` vs
/// `Slot2_to_slot3_PresetTest_HXEdit.json`). Sert de garde-fou si on parse les IN `0x81` côté Rust.
#[cfg(test)]
mod hxedit_slot_focus_preset_test_reference {
    fn capdata_hex(s: &str) -> Vec<u8> {
        s.split(':')
            .filter(|t| !t.is_empty())
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    /// OUT bulk `0x01`, 40 octets — aligné [`probe_hardware_slot_focus_usb`] `hx_edit_cd04`.
    const OUT_SLOT1_TO_2: &str = "1d:00:00:18:80:10:ed:03:00:eb:00:04:6b:4e:00:00:01:00:06:00:0d:00:00:00:83:66:cd:04:02:64:4e:65:82:62:02:1a:00:00:00:00";
    const OUT_SLOT2_TO_3: &str = "1d:00:00:18:80:10:ed:03:00:1c:00:04:7c:4e:00:00:01:00:06:00:0d:00:00:00:83:66:cd:04:03:64:4e:65:82:62:03:1a:00:00:00:00";

    /// IN `0x81` immédiatement après l’OUT (36 puis 44 octets dans les captures).
    const IN36_SLOT2: &str = "19:00:00:18:ed:03:80:10:00:f9:00:04:14:04:00:00:00:00:06:00:09:00:00:00:83:66:cd:04:02:67:00:68:c0:79:13:6a";
    const IN36_SLOT3: &str = "19:00:00:18:ed:03:80:10:00:2c:00:04:29:04:00:00:00:00:06:00:09:00:00:00:83:66:cd:04:03:67:00:68:c0:79:13:6a";

    const IN44_SLOT2: &str = "21:00:00:18:f0:03:02:10:00:c5:00:04:09:02:00:00:00:00:04:00:11:00:00:00:82:69:27:6a:84:52:01:44:03:79:13:6a:82:62:02:1a:00:c2:40:c0";
    const IN44_SLOT3: &str = "21:00:00:18:f0:03:02:10:00:f7:00:04:09:02:00:00:00:00:04:00:11:00:00:00:82:69:27:6a:84:52:01:44:03:79:13:6a:82:62:03:1a:00:c2:40:c0";

    #[test]
    fn out_packets_differ_only_in_session_like_fields_and_slot_bus() {
        let a = capdata_hex(OUT_SLOT1_TO_2);
        let b = capdata_hex(OUT_SLOT2_TO_3);
        assert_eq!(a.len(), 40);
        assert_eq!(b.len(), 40);
        // En-tête ED03 + compteur : varie entre captures.
        assert_ne!(a[8..12], b[8..12]);
        assert_ne!(a[12..16], b[12..16]);
        // Corps fixe jusqu’à `83:66:cd:04`.
        assert_eq!(&a[16..28], &b[16..28]);
        // Octet après `cd:04` = tag (= slot_bus dans ces captures).
        assert_eq!(a[28], 0x02);
        assert_eq!(b[28], 0x03);
        assert_eq!(&a[29..32], &[0x64, 0x4e, 0x65]);
        assert_eq!(&a[29..32], &b[29..32]);
        assert_eq!(a[32], 0x82);
        assert_eq!(a[33], 0x62);
        assert_eq!(a[34], 0x02);
        assert_eq!(b[34], 0x03);
        assert_eq!(a[35], 0x1a);
        assert_eq!(&a[36..40], &b[36..40]);
    }

    #[test]
    fn in36_ed03_replies_echo_slot_bus_and_share_suffix() {
        let s2 = capdata_hex(IN36_SLOT2);
        let s3 = capdata_hex(IN36_SLOT3);
        assert_eq!(s2.len(), 36);
        assert_eq!(s3.len(), 36);
        assert_eq!(&s2[0..8], &s3[0..8]);
        assert_eq!(s2[8], s3[8]);
        assert_ne!(s2[9], s3[9]);
        assert_eq!(&s2[10..12], &s3[10..12]);
        assert_ne!(&s2[12..14], &s3[12..14]);
        assert_eq!(&s2[14..28], &s3[14..28]);
        assert_eq!(s2[28], 0x02);
        assert_eq!(s3[28], 0x03);
        // Suffixe identique sur ces deux gestes (preset inchangé, slots non édités).
        assert_eq!(&s2[29..36], &[0x67, 0x00, 0x68, 0xc0, 0x79, 0x13, 0x6a]);
        assert_eq!(s2[29..36], s3[29..36]);
    }

    #[test]
    fn in44_f003_payload_shares_model_like_block_only_slot_bus_changes() {
        let s2 = capdata_hex(IN44_SLOT2);
        let s3 = capdata_hex(IN44_SLOT3);
        assert_eq!(s2.len(), 44);
        assert_eq!(s3.len(), 44);
        assert_eq!(&s2[0..9], &s3[0..9]);
        assert_ne!(s2[9], s3[9]);
        assert_eq!(&s2[10..24], &s3[10..24]);
        // Bloc stable `82:69…6a` … `03:79:13:6a` puis `82:62:SS:1a` (candidat corrélation preset / module).
        assert_eq!(&s2[24..36], &s3[24..36]);
        assert_eq!(
            &s2[24..36],
            &[
                0x82, 0x69, 0x27, 0x6a, 0x84, 0x52, 0x01, 0x44, 0x03, 0x79, 0x13, 0x6a
            ]
        );
        assert_eq!(&s2[36..39], &[0x82, 0x62, 0x02]);
        assert_eq!(&s3[36..39], &[0x82, 0x62, 0x03]);
        assert_eq!(&s2[39..44], &s3[39..44]);
    }
}