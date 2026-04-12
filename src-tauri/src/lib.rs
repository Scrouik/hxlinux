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
    
    for attempt in 1..=3 {
        let helix = HelixUsb::connect()
            .map_err(|e| format!("Connexion USB échouée: {}", e))?;

        let stop = Arc::new(AtomicBool::new(false));
        let events = helix.start_listener(Arc::clone(&stop));

        let active_preset = match connect_sequence(&helix, &events) {
            Ok(p) => p,
            Err(e) => {
                stop.store(true, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(600));
                helix.disconnect();
                continue;
            }
        };

        let (active_preset_idx, x1_counter) = active_preset;

        let result = request_preset_names(&helix, &events, x1_counter)
            .map_err(|e| format!("Lecture presets échouée: {}", e));

        stop.store(true, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(600));
        helix.disconnect();

        match result {
            Ok(presets) if presets.len() >= 124 => return Ok(presets),
            Ok(presets) => {
                println!("Tentative {} — seulement {} presets, on réessaie", attempt, presets.len());
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
            Err(e) => continue,
        }
    }
    Err("An error has occurred. Please unplug the USB cable, change the active preset, and then reconnect it.".to_string())

}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_preset_names, check_device])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}