use crate::helix::usb::HelixUsb;
use crate::helix::protocol::*;

// Requête pour demander les noms de presets
const REQUEST_PRESET_NAMES: [u8; 40] = [
    0x1d, 0x00, 0x00, 0x18, 0x01, 0x10, 0xef, 0x03,
    0x00, 0x00, 0x00, 0x0c, 0x38, 0x10, 0x00, 0x00,
    0x01, 0x00, 0x02, 0x00, 0x0d, 0x00, 0x00, 0x00,
    0x83, 0x66, 0xcd, 0x03, 0xea, 0x64, 0x01, 0x65,
    0x82, 0x6b, 0x00, 0x65, 0x02, 0x00, 0x00, 0x00
];

pub fn request_preset_names(helix: &HelixUsb) -> Result<Vec<String>, rusb::Error> {
    // Envoi de la requête
    helix.write(&REQUEST_PRESET_NAMES)?;

    let mut stream: Vec<u8> = Vec::new();
    let mut buf = [0u8; 512];
    let mut preset_names: Vec<String> = Vec::new();

    loop {
        match helix.read(&mut buf) {
            Ok(n) => {
                let data = &buf[..n];
                if n > 16 {
                    stream.extend_from_slice(&data[16..]);
                }

                // Parse les noms depuis le stream
                parse_preset_names(&stream, &mut preset_names);

                if preset_names.len() >= PRESET_COUNT {
                    break;
                }
            }
            Err(rusb::Error::Timeout) => break,
            Err(e) => return Err(e),
        }
    }

    Ok(preset_names)
}

fn parse_preset_names(stream: &[u8], names: &mut Vec<String>) {
    let pattern = &PRESET_NAME_PATTERN;
    let mut i = 0;

    while i + 25 <= stream.len() {
        if &stream[i..i+3] == pattern {
            let name_bytes = &stream[i+9..i+25];
            let name: String = name_bytes
                .iter()
                .take_while(|&&b| b != 0x00)
                .filter(|&&b| b >= 32 && b <= 126)
                .map(|&b| b as char)
                .collect();

            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
            i += 25;
        } else {
            i += 1;
        }
    }
}