pub mod helix;

use helix::usb::HelixUsb;
use helix::connect::connect_sequence;
use helix::presets::request_preset_names;

#[tauri::command]
async fn check_device() -> bool {
    HelixUsb::is_connected()
}

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[tauri::command]
async fn get_preset_names() -> Result<Vec<String>, String> {
    let helix = HelixUsb::connect()
        .map_err(|e| format!("Connexion USB échouée: {}", e))?;

    let stop = Arc::new(AtomicBool::new(false));
    let events = helix.start_listener(Arc::clone(&stop));

    let active_preset = connect_sequence(&helix, &events)
        .map_err(|e| format!("Handshake échoué: {}", e))?;

    println!("Preset actif: {}", active_preset);

    let result = request_preset_names(&helix, &events)
        .map_err(|e| format!("Lecture presets échouée: {}", e));

    // Arrêter le listener
    stop.store(true, Ordering::Relaxed);
    
    // Attendre que le thread s'arrête
    std::thread::sleep(std::time::Duration::from_millis(600));

    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_preset_names, check_device])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}