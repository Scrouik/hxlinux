//! Sauvegarde preset sur le HX (capture HX Edit `save_preset.json`).
//!
//! Lane `ed:03` (`80:10`), `cd:04`, sous-commande `06`, compteur lane = `live_write_ctr`.

use crate::helix::packet::OutPacket;
use crate::helix::HelixState;

const SAVE_LANE_LO_DELTA: u16 = 0x11;

/// Nom preset ASCII (16 caractères max), comme l’UI.
pub fn preset_label_ascii_bytes(name: &str) -> Vec<u8> {
    name.chars()
        .filter(|c| c.is_ascii())
        .take(16)
        .map(|c| c as u8)
        .collect()
}

/// Octet après le tag `6d` : `0xa1 + len(nom)` (capture HX Edit).
fn preset_label_length_tag(name_len: usize) -> u8 {
    0xa1u8.wrapping_add(name_len as u8)
}

fn build_save_preset_packet(
    preset_index: u8,
    text: &[u8],
    lane_lo: u8,
    lane_hi: u8,
    cnt: u8,
    double: [u8; 2],
) -> Vec<u8> {
    let msg_size_byte = 0x20u8.wrapping_add(text.len() as u8);
    let second_length_byte = msg_size_byte.wrapping_sub(0x10);
    let length_tag = preset_label_length_tag(text.len());

    let mut data = vec![
        msg_size_byte,
        0x00,
        0x00,
        0x18,
        0x80,
        0x10,
        0xed,
        0x03,
        0x00,
        cnt,
        0x00,
        0x04,
        lane_lo,
        lane_hi,
        0x00,
        0x00,
        0x01,
        0x00,
        0x06,
        0x00,
        second_length_byte,
        0x00,
        0x00,
        0x00,
        0x83,
        0x66,
        0xcd,
        0x04,
        double[0],
        double[1],
        0x47,
        0x65,
        0x83,
        0x6b,
        0x00,
        0x6c,
        preset_index,
        0x6d,
        length_tag,
    ];
    data.extend_from_slice(text);
    while data.len() < (msg_size_byte as usize) + 10 {
        data.push(0x00);
    }
    data
}

/// Témoin `HX_PRESET_SAVE_HW` (défaut ON) : sauvegarde preset sur le HX. `=0` désactive l’envoi.
pub fn preset_save_hw_enabled() -> bool {
    match std::env::var("HX_PRESET_SAVE_HW").as_deref() {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
}

/// Envoie la sauvegarde preset sur le HX (`save_preset.json`).
pub fn send_preset_save(state: &mut HelixState, preset_index: usize, name: &str) -> Result<(), String> {
    if !preset_save_hw_enabled() {
        return Err("sauvegarde preset HX désactivée (HX_PRESET_SAVE_HW=0)".to_string());
    }
    if preset_index > 0xff {
        return Err(format!("index preset invalide: {preset_index}"));
    }
    let text = preset_label_ascii_bytes(name);
    if text.is_empty() {
        return Err("nom preset vide".to_string());
    }

    let ctr = state.live_write_ctr;
    let lane_lo = (ctr & 0xff) as u8;
    let lane_hi = ((ctr >> 8) & 0xff) as u8;
    let cnt = state.next_x80_cnt();
    let double = state.next_editor_ed03_double();

    let data = build_save_preset_packet(
        preset_index as u8,
        &text,
        lane_lo,
        lane_hi,
        cnt,
        double,
    );
    state.send(OutPacket::new(data));
    state.live_write_ctr = state.live_write_ctr.wrapping_add(SAVE_LANE_LO_DELTA);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAVE_CAPTURE: &str =
        "26:00:00:18:80:10:ed:03:00:b6:00:04:05:52:00:00:01:00:06:00:16:00:00:00:83:66:cd:04:1b:64:47:65:83:6b:00:6c:0d:6d:a7:52:65:6e:61:6d:65:00:00:00";

    fn bytes_from_hex_colon(s: &str) -> Vec<u8> {
        s.split(':').map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    #[test]
    fn save_body_matches_capture_layout() {
        let text = b"Rename".to_vec();
        let data = build_save_preset_packet(0x0d, &text, 0x05, 0x52, 0xb6, [0x1b, 0x64]);
        let cap = bytes_from_hex_colon(SAVE_CAPTURE);
        assert_eq!(data.len(), cap.len());
        for (i, (&a, &b)) in data.iter().zip(cap.iter()).enumerate() {
            if matches!(i, 9 | 12 | 13 | 28 | 29) {
                continue;
            }
            assert_eq!(a, b, "byte {i}");
        }
    }
}
