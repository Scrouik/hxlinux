use crate::helix::usb::{HelixUsb, HelixEvent};
use std::sync::mpsc;

pub fn request_preset_names(
    helix: &HelixUsb,
    events: &mpsc::Receiver<HelixEvent>
) -> Result<Vec<String>, rusb::Error> {

    // Envoyer la requête
    helix.write(&[
        0x1d, 0x00, 0x00, 0x18, 0x01, 0x10, 0xef, 0x03,
        0x00, 0x04, 0x00, 0x0c, 0x38, 0x10, 0x00, 0x00,
        0x01, 0x00, 0x02, 0x00, 0x0d, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03, 0xea, 0x64, 0x01, 0x65,
        0x82, 0x6b, 0x00, 0x65, 0x02, 0x00, 0x00, 0x00
    ])?;
    println!("ENVOI requête presets");

    let mut stream: Vec<u8> = Vec::new();
    let mut preset_names: Vec<String> = Vec::new();
    let mut timeout_count = 0;

    loop {
        match events.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(HelixEvent::PresetNamesData(data)) => {
                println!("RECU données presets ({} octets) counter={:02x}", data.len(), data[9]);
                if data.len() > 16 {
                    stream.extend_from_slice(&data[16..]);  // D'abord on ajoute
                }
                println!("Stream total: {} octets", stream.len());

                // Ack
                let ack = [
                    0x08, 0x00, 0x00, 0x18, 0x01, 0x10, 0xef, 0x03,
                    0x00, data[9].wrapping_add(1), 0x00, 0x08,
                    0x38, data[9].wrapping_add(9), 0x00, 0x00
                ];
                helix.write(&ack)?;
                timeout_count = 0;

                if preset_names.len() >= 124 {
                    break;
                }
            }
            
            Ok(HelixEvent::KeepAliveX1 { .. }) => {
                // Ne pas acquitter pendant la lecture des presets
                // Le HX envoie les données sans attendre cet ack
                continue;
            }

            Ok(HelixEvent::KeepAliveX80 { .. }) => {
                // Ignorer pendant la lecture des presets
                continue;
            }

            Ok(HelixEvent::RawMessage(data)) => {
                println!("RAW reçu: data[0]={:02x} data[4]={:02x} data[11]={:02x}", 
                    data[0], data[4], if data.len()>11 {data[11]} else {0});
            }

            Err(_) => {
                timeout_count += 1;
                println!("Timeout #{}", timeout_count);
                if timeout_count >= 2 {
                    break;
                }
            }
            _ => continue,
        }
    }

    parse_preset_names(&stream, &mut preset_names);  // Puis on parse
    println!("Noms décodés: {}", preset_names.len());
    println!("Total presets: {}", preset_names.len());
    Ok(preset_names)
}


fn parse_preset_names(stream: &[u8], names: &mut Vec<String>) {
    let mut i = 0;
    while i + 25 <= stream.len() {
        if stream[i] == 0x81 && stream[i+1] == 0xcd && stream[i+2] == 0x00 {
            if i + 8 <= stream.len() && stream[i+4] == 0x84 && stream[i+5] == 0xcd 
                && stream[i+6] == 0x00 && stream[i+7] == 0x6d {
                
                let preset_idx = stream[i+3] as usize;                
                let name_len = stream[i+8] as usize;
                let name_start = i + 9;
                let name_end = name_start + name_len.saturating_sub(0x80);
                
                if name_end <= stream.len() {
                    let name_bytes = &stream[name_start..name_end];
                    let name: String = name_bytes
                        .iter()
                        .take_while(|&&b| b != 0x00)
                        .filter(|&&b| b >= 32 && b <= 126)
                        .map(|&b| b as char)
                        .collect();

                    if !name.is_empty() {
                        while names.len() <= preset_idx {
                            names.push(String::from("New Preset"));
                        }
                        names[preset_idx] = name;
                    }
                }
                i += 9;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}