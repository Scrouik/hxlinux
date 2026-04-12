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

const HX_VID: u16 = 0x0e41;
const HX_PID: u16 = 0x4253;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|_app| {
            thread::spawn(|| {
                start_monitor();
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ===========================================================
// Surveille le branchement USB et lance/arrête la connexion
// ===========================================================
fn start_monitor() {
    println!("[Monitor] démarrage surveillance USB");

    let stop_monitor = Arc::new(AtomicBool::new(false));

    helix::usb_monitor::start_monitor(
        Arc::new(Mutex::new(HelixState::new())), // state temporaire pour le monitor
        Arc::clone(&stop_monitor),
        Arc::new(|| {
            // HX branché → démarrer la connexion
            println!("[Monitor] HX détecté → démarrage connexion");
            thread::spawn(|| {
                start_helix();
            });
        }),
        Arc::new(|| {
            // HX débranché → le usb_listener détectera NoDevice
            println!("[Monitor] HX débranché");
        }),
    );
}

// ===========================================================
// Connexion complète au HX et boucle de traitement
// ===========================================================
fn start_helix() {
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

    // -- Channels --
    let (usb_tx,  usb_rx)  = mpsc::channel::<OutPacket>();
    let (mode_tx, mode_rx) = mpsc::channel::<ModeRequest>();
    let (ka_tx,   ka_rx)   = mpsc::channel::<KeepAliveCommand>();

    // -- État partagé --
    let state = Arc::new(Mutex::new(HelixState::new()));
    {
        let mut s = state.lock().unwrap();
        s.tx           = Some(usb_tx);
        s.mode_tx      = Some(mode_tx);
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

    // -- Boucle principale : changements de mode --
    println!("[Helix] en attente de changements de mode");
    loop {
        match mode_rx.recv() {
            Ok(ModeRequest::RequestPresetName) => {
                println!("[Helix] → RequestPresetName");
                let mut s = state.lock().unwrap();
                let mut m = current_mode.lock().unwrap();
                m.shutdown(&mut s);
                *m = Box::new(RequestPresetName::new());
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

    // -- Nettoyage --
    stop_listener.store(true, Ordering::SeqCst);
    ka_manager.stop_all();
    let _ = handle.release_interface(0);
    println!("[Helix] arrêt propre");
}