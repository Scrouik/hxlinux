pub mod helix;

use helix::usb::HelixUsb;
use helix::connect::connect_sequence;
use helix::presets::request_preset_names;

#[tauri::command]
async fn get_preset_names() -> Result<Vec<String>, String> {
    let helix = HelixUsb::connect()
        .map_err(|e| format!("Connexion USB échouée: {}", e))?;
    
    // Test simple — juste vérifier qu'on peut ouvrir le device
    Ok(vec!["HX Stomp XL connecté !".to_string()])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_preset_names])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}