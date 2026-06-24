//! Decode preset index + label from HX USB wire buffers (Kempline offsets).
//! Offsets are relative to the ED03 payload slice (`data[16..]`).

/// Index at byte 24, name ASCII at byte 27 (max 24 chars, NUL-terminated).
pub fn decode_from_transfer_buf(buf: &[u8]) -> Option<(usize, String)> {
    if buf.len() <= 27 {
        return None;
    }
    let idx = buf[24] as usize;
    let name_start = 27usize;
    let name_end = name_start.saturating_add(24);
    let mut decoded = String::new();
    if buf.len() > name_start {
        let slice = &buf[name_start..buf.len().min(name_end)];
        for &b in slice {
            if b == 0x00 {
                break;
            }
            decoded.push(if (32..=126).contains(&b) { b as char } else { '?' });
        }
    }
    let decoded = if decoded.is_empty() {
        "<empty>".to_string()
    } else {
        decoded
    };
    Some((idx, decoded))
}

pub fn decode_from_ed03_packet(data: &[u8]) -> Option<(usize, String)> {
    if data.len() < 16 {
        return None;
    }
    decode_from_transfer_buf(&data[16..])
}

pub fn log_wire_preset(source: &str, idx: usize, name: Option<&str>) {
    match name {
        Some(n) => eprintln!(
            "[Preset] wire ({source}) index={idx} ({idx:03}) name='{n}'"
        ),
        None => eprintln!("[Preset] wire ({source}) index={idx} ({idx:03})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Capture `02_change_preset_UI_HXEdit.json` pkt 532 — phase-1 name response.
    #[test]
    fn decode_phase1_from_change_preset_capture() {
        let pkt: [u8; 68] = [
            0x39, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x3d, 0x00, 0x04, 0x31, 0x04,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x29, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04,
            0x05, 0x67, 0x00, 0x68, 0x86, 0x6b, 0xcd, 0x00, 0x00, 0x6c, 0xcd, 0x00, 0x20, 0x6d,
            0xac, 0x50, 0x72, 0x65, 0x73, 0x65, 0x74, 0x20, 0x54, 0x65, 0x73, 0x74, 0x00, 0x75,
            0xc2, 0x53, 0x92, 0xcd, 0x11, 0x57, 0x00, 0x5c, 0x00, 0x40, 0xc0, 0x93,
        ];
        let (idx, name) = decode_from_ed03_packet(&pkt).expect("decode");
        assert_eq!(idx, 32);
        assert_eq!(name, "Preset Test");
    }
}
