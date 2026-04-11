pub mod helix;

use helix::usb::HelixUsb;
use helix::connect::connect_sequence;
use helix::presets::request_preset_names;

#[tauri::command]
async fn get_preset_names() -> Result<Vec<String>, String> {
    let helix = HelixUsb::connect()
        .map_err(|e| format!("Connexion USB échouée: {}", e))?;
    
    println!("USB connecté !");

    let events = helix.start_listener();

    connect_sequence(&helix, &events)
        .map_err(|e| format!("Handshake échoué: {}", e))?;
    
    
    request_preset_names(&helix, &events)
        .map_err(|e| format!("Lecture presets échouée: {}", e))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_preset_names])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}