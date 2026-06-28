//! Parse des réponses **IN bulk `0x81`** après un OUT « focus slot » type HX Edit (`83:66:cd:04`).
//! Références : `Slot1_to_slot2_PresetTest_HXEdit.json` / `Slot2_to_slot3_PresetTest_HXEdit.json`.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotFocusInCapsule {
    /// Octet après `83:66:cd:04` dans la réponse ED03 36 o (aligné slot bus path effet).
    pub slot_bus: u8,
    pub ed03_36_hex: Option<String>,
    pub f003_44_hex: Option<String>,
    /// Bloc stable `f0:03` octets 24–35 (captures HX Edit identiques slot 2 vs 3 sans édition).
    pub anchor12_hex: String,
    /// Copie binaire pour corrélation avec `preset_data` (non sérialisée vers le front).
    #[serde(skip_serializing)]
    pub anchor12: [u8; 12],
    /// Suffixe ED03 octets 29–35 (identique entre les deux captures de référence).
    pub ed_suffix7_hex: String,
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

/// Reconnaît la paire **36 o ED03** + **44 o `f0:03:02:10`** dans les trames captées.
pub fn parse_slot_focus_bulk_in_frames(frames: &[Vec<u8>]) -> Option<SlotFocusInCapsule> {
    let mut ed36: Option<&[u8]> = None;
    let mut f044: Option<&[u8]> = None;
    for f in frames {
        if f.len() == 36
            && f.get(0..8) == Some(&[0x19, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10])
            && f.get(24..28) == Some(&[0x83, 0x66, 0xcd, 0x04])
        {
            ed36 = Some(f.as_slice());
        }
        if f.len() == 44
            && f.get(0..8) == Some(&[0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10])
        {
            f044 = Some(f.as_slice());
        }
    }
    let ed = ed36?;
    let f0 = f044?;
    let slot_bus = *ed.get(28)?;
    let anchor = f0.get(24..36)?;
    let suf = ed.get(29..36)?;
    let mut anchor12 = [0u8; 12];
    anchor12.copy_from_slice(anchor);
    Some(SlotFocusInCapsule {
        slot_bus,
        ed03_36_hex: Some(hex_lower(ed)),
        f003_44_hex: Some(hex_lower(f0)),
        anchor12_hex: hex_lower(anchor),
        anchor12,
        ed_suffix7_hex: hex_lower(suf),
    })
}

/// Cherche l’ancre 12 octets dans un dump preset (corrélation expérimentale).
pub fn find_anchor_subsequence(preset: &[u8], anchor: &[u8; 12]) -> Option<usize> {
    preset
        .windows(anchor.len())
        .position(|w| w == anchor)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IN36_S2: &[u8] = &[
        0x19, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0xf9, 0x00, 0x04, 0x14, 0x04, 0x00,
        0x00, 0x00, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04, 0x02, 0x67,
        0x00, 0x68, 0xc0, 0x79, 0x13, 0x6a,
    ];
    const IN44_S2: &[u8] = &[
        0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0xc5, 0x00, 0x04, 0x09, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x82, 0x69, 0x27, 0x6a, 0x84, 0x52,
        0x01, 0x44, 0x03, 0x79, 0x13, 0x6a, 0x82, 0x62, 0x02, 0x1a, 0x00, 0xc2, 0x40, 0xc0,
    ];

    #[test]
    fn parses_reference_slot1_to_slot2_pair() {
        let frames = vec![IN36_S2.to_vec(), IN44_S2.to_vec()];
        let c = parse_slot_focus_bulk_in_frames(&frames).expect("parse");
        assert_eq!(c.slot_bus, 0x02);
        assert_eq!(c.anchor12_hex, "8269276a845201440379136a");
        assert_eq!(c.ed_suffix7_hex, "670068c079136a");
    }
}
