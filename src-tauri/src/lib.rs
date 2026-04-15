// ===========================================================
// lib.rs
// Point d'entrée — assemble tous les composants
// Le UsbMonitor surveille le branchement du HX et déclenche
// automatiquement la séquence de connexion
// ===========================================================

mod helix;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::thread;
use tauri::{WebviewUrl, WebviewWindowBuilder};

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

const HX_VID: u16 = 0x0e41;
const HX_PID: u16 = 0x4253;
const EXPECTED_PRESET_COUNT: usize = 125;

// ===========================================================
// État partagé entre Rust et Tauri
// ===========================================================
#[derive(Default)]
struct AppState {
    preset_names:  Vec<String>,
    active_preset: usize,
    // Accès au HelixState pour envoyer des commandes USB depuis Tauri
    helix_state:   Option<Arc<Mutex<HelixState>>>,
    // Handle USB pour les commandes directes (ex: MIDI Program Change sur 0x02)
    usb_handle:    Option<Arc<rusb::DeviceHandle<rusb::GlobalContext>>>,
}

// ===========================================================
// Commandes Tauri
// ===========================================================

#[tauri::command]
fn get_preset_names(state: tauri::State<Arc<Mutex<AppState>>>) -> Vec<String> {
    state.lock().unwrap().preset_names.clone()
}

#[tauri::command]
fn get_active_preset(state: tauri::State<Arc<Mutex<AppState>>>) -> usize {
    state.lock().unwrap().active_preset
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

    // Limiter à 24 caractères ASCII — limite du HX
    let text: Vec<u8> = name
        .chars()
        .filter(|c| c.is_ascii())
        .take(24)
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
    let helix_arc = {
        let app = state.lock().unwrap();
        app.helix_state.clone().ok_or("HX non connecté")?
    };
    let mut s = helix_arc.lock().unwrap();
    if s.preset_content_only && !s.preset_data_ready {
        eprintln!("[PresetDebug][request_preset_content] already in progress, skip");
        return Ok(());
    }
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
}

/// Parse les slots d'un preset brut.
/// Kempline : split par [0x82, 0x13], extrait l'ID entre markers 0x19 et 0x1a.
fn parse_preset_slots(data: &[u8]) -> Vec<[String; 2]> {
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

    let mut result = Vec::new();
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
                        if let Some(entry) = MODULES_BY_ID.get(&id_hex) {
                            result.push(entry.clone());
                        } else {
                            result.push([String::from("Unknown"), id_hex]);
                        }
                    }
                    break;
                }
            }
            cursor += 1;
        }
    }
    result
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
        .invoke_handler(tauri::generate_handler![
            get_preset_names,
            get_active_preset,
            rename_preset,
            activate_preset,
            request_preset_content,
            get_preset_slots,
            get_active_preset_slots,
            get_preset_data_hex,
        ])
        .setup(move |app| {
            let _ = WebviewWindowBuilder::new(
                app,
                "models",
                WebviewUrl::App("models.html".into()),
            )
            .title("HX Models")
            .inner_size(560.0, 760.0)
            .min_inner_size(420.0, 420.0)
            .resizable(true)
            .build();

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

    helix::usb_monitor::start_monitor(
        Arc::new(Mutex::new(HelixState::new())),
        Arc::clone(&stop_monitor),
        Arc::new(move || {
            let state = Arc::clone(&app_state);
            thread::spawn(move || {
                start_helix(state);
            });
        }),
        Arc::new(|| {}),
    );
}

// ===========================================================
// Connexion complète au HX et boucle de traitement
// ===========================================================
fn start_helix(app_state: Arc<Mutex<AppState>>) {
    // Ouvrir le device USB
    let handle = match rusb::open_device_with_vid_pid(HX_VID, HX_PID) {
        Some(h) => Arc::new(h),
        None => {
            eprintln!("[Helix] HX non trouvé");
            return;
        }
    };

    // Réclamer l'interface USB
    if let Err(e) = handle.claim_interface(0) {
        eprintln!("[Helix] erreur claim_interface : {}", e);
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
                    if !s.got_preset && s.preset_names.len() >= EXPECTED_PRESET_COUNT {
                        app.preset_names = s.preset_names.clone();
                    } else if !s.got_preset {
                        eprintln!(
                            "[PresetDebug][Standard] skip preset_names overwrite len={}",
                            s.preset_names.len()
                        );
                    }
                    s.got_preset = false;
                    app.active_preset = s.preset_index;
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
    }
}