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

use lazy_static::lazy_static;
lazy_static! {
    static ref GLOBAL_HANDLE: Mutex<Option<Arc<rusb::DeviceHandle<rusb::GlobalContext>>>> = 
        Mutex::new(None);
    static ref GLOBAL_MODE_TX: Mutex<Option<mpsc::Sender<ModeRequest>>> =
        Mutex::new(None);
    static ref GLOBAL_STOP_LISTENER: Mutex<Option<Arc<AtomicBool>>> =
        Mutex::new(None);
    static ref GLOBAL_KA_MANAGER: Mutex<Option<Arc<KeepAliveManager>>> =
        Mutex::new(None);
    static ref GLOBAL_USB_TX_DROP: Mutex<Option<mpsc::Sender<OutPacket>>> =
    Mutex::new(None);
}

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
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                println!("[Tauri] fenêtre fermée → nettoyage USB");
                // Signal de shutdown via canal global
                // Pour l'instant : libérer proprement le device
                cleanup_usb();
                window.destroy().unwrap();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ===========================================================
// Deconnect le port USB proprement
// ===========================================================
fn cleanup_usb() {
    // 1. Stopper les keep-alives
    if let Some(ka) = GLOBAL_KA_MANAGER.lock().unwrap().as_ref() {
        ka.stop_all();
    }

    // 2. Stopper le listener
    if let Some(stop) = GLOBAL_STOP_LISTENER.lock().unwrap().as_ref() {
        stop.store(true, Ordering::SeqCst);
    }

    // 3. Fermer le canal mode
    {
        let mut tx = GLOBAL_MODE_TX.lock().unwrap();
        *tx = None;
    }

    {
        let mut tx = GLOBAL_USB_TX_DROP.lock().unwrap();
        *tx = None; // drop → ferme le channel → usb_writer s'arrête
    }
    // 4. Attendre que tout s'arrête
    thread::sleep(Duration::from_millis(200));

    // 5. Libérer l'interface
    if let Some(handle) = GLOBAL_HANDLE.lock().unwrap().take() {
        // Reset USB — équivalent dispose_resources de kempline
        if let Err(e) = handle.reset() {
            println!("[Helix] reset USB : {}", e);
        }
        let _ = handle.release_interface(0);
        drop(handle);
        println!("[Helix] USB libéré proprement");
    }
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

    // Kempline : clear_feature ENDPOINT_HALT sur 0x01 et 0x81
    if let Err(e) = handle.clear_halt(0x01) {
        println!("[Helix] clear_halt 0x01 : {}", e);
    }
    if let Err(e) = handle.clear_halt(0x81) {
        println!("[Helix] clear_halt 0x81 : {}", e);
    }
    println!("[Helix] endpoints réinitialisés");

    {
    let mut global = GLOBAL_HANDLE.lock().unwrap();
    *global = Some(Arc::clone(&handle));
    }

    // -- Channels --
    let (usb_tx,  usb_rx)  = mpsc::channel::<OutPacket>();
    let (mode_tx, mode_rx) = mpsc::channel::<ModeRequest>();
    {
        let mut global_tx = GLOBAL_MODE_TX.lock().unwrap();
        *global_tx = Some(mode_tx.clone());
    }
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
    {
        let mut global_stop = GLOBAL_STOP_LISTENER.lock().unwrap();
        *global_stop = Some(Arc::clone(&stop_listener));
    }
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

    {
        let mut global_ka = GLOBAL_KA_MANAGER.lock().unwrap();
        *global_ka = Some(Arc::clone(&ka_manager));
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