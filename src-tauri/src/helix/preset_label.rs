//! Rename / save preset sur le HX (captures HX Edit).
//!
//! Lane `ed:03` (`80:10`), compteur lane = `live_write_ctr`.
//! - **Rename** : `cd:03`, sous-commande `02`, suffixe `06:65`
//! - **Save** : `cd:04`, sous-commande `06`, suffixe `47:65`

use crate::helix::packet::OutPacket;
use crate::helix::HelixState;

const LABEL_LANE_LO_DELTA: u16 = 0x11;

struct PresetLabelWire {
    cd: u8,
    cmd: u8,
    suffix: [u8; 2],
}

const RENAME_WIRE: PresetLabelWire = PresetLabelWire {
    cd: 0x03,
    cmd: 0x02,
    suffix: [0x06, 0x65],
};

const SAVE_WIRE: PresetLabelWire = PresetLabelWire {
    cd: 0x04,
    cmd: 0x06,
    suffix: [0x47, 0x65],
};

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

fn build_preset_label_packet(
    preset_index: u8,
    text: &[u8],
    lane_lo: u8,
    lane_hi: u8,
    cnt: u8,
    double: [u8; 2],
    wire: PresetLabelWire,
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
        wire.cmd,
        0x00,
        second_length_byte,
        0x00,
        0x00,
        0x00,
        0x83,
        0x66,
        0xcd,
        wire.cd,
        double[0],
        double[1],
        wire.suffix[0],
        wire.suffix[1],
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

fn send_preset_label(
    state: &mut HelixState,
    preset_index: usize,
    name: &str,
    wire: PresetLabelWire,
    disabled_msg: &str,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        return Err(disabled_msg.to_string());
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

    let data = build_preset_label_packet(
        preset_index as u8,
        &text,
        lane_lo,
        lane_hi,
        cnt,
        double,
        wire,
    );
    state.send(OutPacket::new(data));
    state.live_write_ctr = state.live_write_ctr.wrapping_add(LABEL_LANE_LO_DELTA);
    Ok(())
}

/// Témoin `HX_PRESET_RENAME_HW` (défaut ON) : rename preset sur le HX. `=0` désactive l’envoi.
pub fn preset_rename_hw_enabled() -> bool {
    match std::env::var("HX_PRESET_RENAME_HW").as_deref() {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
}

/// Témoin `HX_PRESET_SAVE_HW` (défaut ON) : sauvegarde preset sur le HX. `=0` désactive l’envoi.
pub fn preset_save_hw_enabled() -> bool {
    match std::env::var("HX_PRESET_SAVE_HW").as_deref() {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
}

/// Envoie le rename preset (`cd:03`, cmd `02`) sur le HX.
pub fn send_preset_rename(state: &mut HelixState, preset_index: usize, name: &str) -> Result<(), String> {
    send_preset_label(
        state,
        preset_index,
        name,
        RENAME_WIRE,
        "rename preset HX désactivé (HX_PRESET_RENAME_HW=0)",
        preset_rename_hw_enabled(),
    )
}

/// Envoie la sauvegarde preset sur le HX (`save_preset.json`).
pub fn send_preset_save(state: &mut HelixState, preset_index: usize, name: &str) -> Result<(), String> {
    send_preset_label(
        state,
        preset_index,
        name,
        SAVE_WIRE,
        "sauvegarde preset HX désactivée (HX_PRESET_SAVE_HW=0)",
        preset_save_hw_enabled(),
    )
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
        let data = build_preset_label_packet(0x0d, &text, 0x05, 0x52, 0xb6, [0x1b, 0x64], SAVE_WIRE);
        let cap = bytes_from_hex_colon(SAVE_CAPTURE);
        assert_eq!(data.len(), cap.len());
        for (i, (&a, &b)) in data.iter().zip(cap.iter()).enumerate() {
            if matches!(i, 9 | 12 | 13 | 28 | 29) {
                continue;
            }
            assert_eq!(a, b, "byte {i}");
        }
    }

    #[test]
    fn rename_uses_cd03_cmd02_suffix_0665() {
        let text = b"New Name".to_vec();
        let data = build_preset_label_packet(0x20, &text, 0x05, 0x52, 0xb6, [0xed, 0x64], RENAME_WIRE);
        assert_eq!(&data[4..8], &[0x80, 0x10, 0xed, 0x03]);
        assert_eq!(data[18], 0x02);
        let cd_idx = data
            .windows(3)
            .position(|w| w == [0x83, 0x66, 0xcd])
            .unwrap();
        assert_eq!(data[cd_idx + 3], 0x03);
        assert_eq!(data[cd_idx + 6], 0x06);
        assert_eq!(data[cd_idx + 7], 0x65);
        assert!(data.windows(text.len()).any(|w| w == text.as_slice()));
    }
}
