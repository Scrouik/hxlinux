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
use std::thread;

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

const HX_VID: u16 = 0x0e41;
const HX_PID: u16 = 0x4253;

// État partagé entre Rust et Tauri
#[derive(Default)]
struct AppState {
    preset_names:   Vec<String>,
    active_preset:  usize,
}

#[tauri::command]
fn get_preset_names(state: tauri::State<Arc<Mutex<AppState>>>) -> Vec<String> {
    state.lock().unwrap().preset_names.clone()
}

#[tauri::command]
fn get_active_preset(state: tauri::State<Arc<Mutex<AppState>>>) -> usize {
    state.lock().unwrap().active_preset
}

pub fn run() {
    let app_state = Arc::new(Mutex::new(AppState::default()));
    let app_state_clone = Arc::clone(&app_state);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_preset_names,
            get_active_preset,
        ])
        .setup(move |_app| {
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
    println!("[Monitor] démarrage surveillance USB");
    let stop_monitor = Arc::new(AtomicBool::new(false));

    helix::usb_monitor::start_monitor(
        Arc::new(Mutex::new(HelixState::new())),
        Arc::clone(&stop_monitor),
        Arc::new(move || {
            println!("[Monitor] HX détecté → démarrage connexion");
            let state = Arc::clone(&app_state);
            thread::spawn(move || {
                start_helix(state);
            });
        }),
        Arc::new(|| {
            println!("[Monitor] HX débranché");
        }),
    );
}

// ===========================================================
// Connexion complète au HX et boucle de traitement
// ===========================================================
fn start_helix(app_state: Arc<Mutex<AppState>>) {
    println!("[Helix] connexion au HX Stomp XL...");

    // Ouvrir le device USB
    let handle = match rusb::open_device_with_vid_pid(HX_VID, HX_PID) {
        Some(h) => Arc::new(h),
        None => {
            println!("[Helix] HX non trouvé");
            return;
        }
    };

    // Réclamer l'interface USB
    if let Err(e) = handle.claim_interface(0) {
        println!("[Helix] erreur claim_interface : {}", e);
        return;
    }
    println!("[Helix] interface USB réclamée");

    // Réclamer l'interface MIDI (interface 4)
    if let Err(e) = handle.kernel_driver_active(4) {
        println!("[Helix] kernel_driver_active(4) : {}", e);
    }
    if handle.kernel_driver_active(4).unwrap_or(false) {
        println!("[Helix] détachement kernel driver interface 4");
        let _ = handle.detach_kernel_driver(4);
    }
    if let Err(e) = handle.claim_interface(4) {
        println!("[Helix] erreur claim_interface(4) : {}", e);
    } else {
        println!("[Helix] interface MIDI réclamée");
    }

    // Kempline : clear_feature ENDPOINT_HALT sur 0x01 et 0x81
    if let Err(e) = handle.clear_halt(0x01) {
        println!("[Helix] clear_halt 0x01 : {}", e);
    }
    if let Err(e) = handle.clear_halt(0x81) {
        println!("[Helix] clear_halt 0x81 : {}", e);
    }
    println!("[Helix] endpoints réinitialisés");

    // -- Channels --
    let (usb_tx,  usb_rx)  = mpsc::channel::<OutPacket>();
    let (mode_tx, mode_rx) = mpsc::channel::<ModeRequest>();
    let (ka_tx,   ka_rx)   = mpsc::channel::<KeepAliveCommand>();

    // -- État partagé --
    let state = Arc::new(Mutex::new(HelixState::new()));
    {
        let mut s = state.lock().unwrap();
        s.tx           = Some(usb_tx);
        s.mode_tx = Some(mode_tx.clone());
        s.keepalive_tx = Some(ka_tx);
        // Générer un session_no aléatoire dès le départ
        s.new_session_no();
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
            println!("[KeepAliveManager] démarré");
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

    // Thread MIDI listener — endpoint 0x82
    {
        let handle_midi = Arc::clone(&handle);
        let mode_tx_midi = mode_tx.clone();
        let state_midi = Arc::clone(&state);
        thread::spawn(move || {
            println!("[MidiListener] démarré");
            let mut buf = vec![0u8; 64];
            loop {
                match handle_midi.read_bulk(0x82, &mut buf, Duration::from_millis(500)) {
                    Ok(n) if n >= 4 => {
                        // MIDI Program Change : [0x0C, 0xC0, program_no, 0x00]
                        if buf[0] == 0x0C && (buf[1] & 0xF0) == 0xC0 {
                            let preset_no = buf[2] as usize;
                            println!("[MidiListener] Program Change → preset {}", preset_no);
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
    println!("[Helix] en attente de changements de mode");
    loop {
        match mode_rx.recv() {
            Ok(ModeRequest::RequestPreset) => {
                println!("[Helix] → RequestPreset");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(RequestPreset::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::RequestPresetName) => {
                println!("[Helix] → RequestPresetName");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(RequestPresetName::new());
                // Mettre à jour le preset actif
                {
                    let mut app = app_state.lock().unwrap();
                    app.active_preset = s.preset_index;
                }
                m.start(&mut s);
            }
            Ok(ModeRequest::RequestPresetNames) => {
                println!("[Helix] → RequestPresetNames");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(RequestPresetNames::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::Standard) => {
                println!("[Helix] → Standard");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                
                // Mettre à jour l'AppState
                {
                    let mut app = app_state.lock().unwrap();
                    app.preset_names  = s.preset_names.clone();
                    app.active_preset = s.preset_index;
                }
                
                *m = Box::new(Standard);
                m.start(&mut s);
            }
            Ok(ModeRequest::Connect) => {
                println!("[Helix] → Connect");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(Connect::new());
                m.start(&mut s);
            }
            Ok(ModeRequest::ReconfigureX1) => {
                println!("[Helix] → ReconfigureX1");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(ReconfigureX1::new());
                m.start(&mut s);
            }
            Err(_) => {
                println!("[Helix] channel fermé → arrêt");
                break;
            }
        }
    }

    // Nettoyage — kempline : shutdown()
    println!("[Helix] arrêt en cours...");

    // 1. Arrêter tous les threads
    stop_listener.store(true, Ordering::SeqCst);
    ka_manager.stop_all();

    // 2. Attendre que les threads se terminent
    thread::sleep(Duration::from_millis(1100)); // > 1.04s keep-alive interval

    // 3. Libérer l'interface USB
    let _ = handle.release_interface(0);

    // 4. Réattacher le kernel driver — kempline : attach_kernel_driver
    // Cela libère proprement le HX et débloque son écran
    if let Err(e) = handle.attach_kernel_driver(0) {
        println!("[Helix] attach_kernel_driver : {}", e);
    }

    println!("[Helix] arrêt propre");
}