//! Live write Path 1 I/O (`opcode 0x1d`, 40 octets) — sélection source Input Stomp, etc.
//!
//! Distinct de `live_write` (`27`/`23` sur bus slot FX) et de `edit_slot_model` (bulk assign).
//!
//! Séquence observée HX Edit (`Input.json`) par changement de source :
//!   OUT `1d` (40 o) → OUT `08` ed03 byte11=`08` (16 o).

use std::sync::OnceLock;

use serde_json::Value;

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

const USB_ASSIGN_JSON: &str = include_str!("../../resources/HX_ModelUsbAssign.json");

/// Octet valeur `@input` dans la trame `1d` (bus `0x00`, sélecteur `0x33`).
const PATH1_INPUT_WIRE_VALUE_OFFSET: usize = 36;
/// Octet `yy` dans le bloc modèle `83:66:cd:03:yy:…` (offset 4 du bloc → 28 dans la trame).
const PATH1_IO_MODEL_YY_OFFSET: usize = 28;

#[derive(Clone, Debug)]
struct IoSourceEntry {
    id: String,
    parent_model_id: String,
    wire_value: u8,
    template: Vec<u8>,
}

static IO_SOURCES: OnceLock<Vec<IoSourceEntry>> = OnceLock::new();

fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let t = s.trim();
    if t.is_empty() || !t.len().is_multiple_of(2) {
        return None;
    }
    (0..t.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&t[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

fn load_io_sources() -> Vec<IoSourceEntry> {
    let v: Value = serde_json::from_str(USB_ASSIGN_JSON).unwrap_or(Value::Null);
    let Some(arr) = v.get("ioSources").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in arr {
        let Some(obj) = e.as_object() else {
            continue;
        };
        let id = obj
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() {
            continue;
        }
        let parent_model_id = obj
            .get("parentModelId")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let wire_value = obj
            .get("wireValue")
            .and_then(|x| x.as_u64())
            .map(|n| n.min(255) as u8)
            .unwrap_or(0);
        let hex = obj
            .get("liveWriteHex")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        let Some(template) = parse_hex_bytes(hex) else {
            continue;
        };
        if template.len() != 40 || template.first() != Some(&0x1d) {
            continue;
        }
        out.push(IoSourceEntry {
            id,
            parent_model_id,
            wire_value,
            template,
        });
    }
    out
}

fn io_sources() -> &'static [IoSourceEntry] {
    IO_SOURCES.get_or_init(load_io_sources)
}

fn resolve_io_source_entry(io_source_id: &str) -> Option<&'static IoSourceEntry> {
    let id = io_source_id.trim();
    io_sources().iter().find(|e| e.id == id)
}

/// Cherche `@input` Path 1 dans un buffer IN (`82:62:00:33:XX`, captures HX Edit `Input.json`).
pub fn scan_path1_input_source_wire(buf: &[u8]) -> Option<u8> {
    let mut i = 0usize;
    while i + 5 <= buf.len() {
        if buf[i] == 0x82 && buf[i + 1] == 0x62 && buf[i + 2] == 0x00 && buf[i + 3] == 0x33 {
            return Some(buf[i + 4]);
        }
        i += 1;
    }
    None
}

/// IN `21` (44 o) émis quand l’utilisateur **scroll** la source Input sur le hardware (capture `scroll Input.json`).
/// Distinct du pull FX (`1d`/`1f` + `82:69:31:6a`) : ici `f0:03:02:10` + ancre `82:69:1b:6a:84:52…` + `82:62:00:33:XX`.
pub fn is_path1_input_scroll_notify_21(data: &[u8]) -> bool {
    const ANCHOR: [u8; 12] = [
        0x82, 0x69, 0x1b, 0x6a, 0x84, 0x52, 0x00, 0x44, 0x07, 0x79, 0x17, 0x6a,
    ];
    data.len() == 44
        && data.first() == Some(&0x21)
        && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
        && data.windows(ANCHOR.len()).any(|w| w == ANCHOR)
        && scan_path1_input_source_wire(data).is_some()
}

fn is_path1_input_model_block(block: &[u8; 16]) -> bool {
    block.len() >= 11 && block[8..11] == [0x82, 0x62, 0x00]
}

/// Focus slot structurel Input/Output/Split/Merge (`82:62:bus:1a`), aligné `switch_active_hardware_slot`.
pub fn build_special_slot_focus_packet(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    let cnt = state.next_x80_cnt();
    let session = state.session_no;
    let double = state.preset_data_packet_double();
    let ed_tag = state.next_editor_ed03_double()[0];
    vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt, 0x00, 0x04, session,
        double[0], double[1], 0x00, 0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x83,
        0x66, 0xcd, 0x03, ed_tag, 0x64, 0x4e, 0x65, 0x82, 0x62, slot_bus, 0x1a, 0x00, 0x00,
        0x00, 0x00,
    ]
}

/// Post-ACK `08` ed03 après un write `1d` (captures HX Edit `Input.json`, matrix move).
pub fn build_post_1d_ack08(state: &mut HelixState, ctr_lo: u8, ctr_hi: u8) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x08, ctr_lo, ctr_hi,
        0x00, 0x00,
    ]
}

/// Préambule matrix D&D HX Edit : `ed:08` sub=`10` avant le `1d` (`d&d_split.json` #1363).
pub fn build_ed03_dd_preamble_sub10(state: &mut HelixState) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let session = state.session_no;
    let double = state.preset_data_packet_double();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x10, session,
        double[0], double[1], 0x00,
    ]
}

/// Préambule matrix D&D : `f0:03` sub=`10` avant le `1d` (`d&d_split.json` #1373).
pub fn build_f0_dd_preamble_sub10(state: &mut HelixState) -> Vec<u8> {
    let seq = state.next_x2_cnt();
    let double = state.firmware_scroll_lane_double();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03, 0x00, seq, 0x00, 0x10, double[0],
        double[1], 0x00, 0x00,
    ]
}

/// Post-commit matrix D&D : `f0:03` sub=`08` après ACK `ed:08` (`d&d_split.json` #1545).
pub fn build_f0_dd_post_commit_sub08(state: &mut HelixState) -> Vec<u8> {
    let seq = state.next_x2_cnt();
    let double = state.firmware_scroll_lane_double();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03, 0x00, seq, 0x00, 0x08, double[0],
        double[1], 0x00, 0x00,
    ]
}

/// Arme le D&D matrix au **pointerdown** (HX Edit envoie pré `ed`+`f0` sub 10 au début du drag).
pub fn send_matrix_dd_drag_arm(state: &mut HelixState) -> Result<(), String> {
    let pre_ed = build_ed03_dd_preamble_sub10(state);
    let pre_f0 = build_f0_dd_preamble_sub10(state);
    eprintln!("[MatrixDd] arm ed:08 sub=10 + f0 sub=10");
    state.send(OutPacket::new(pre_ed));
    state.send(OutPacket::with_delay(pre_f0, 30));
    Ok(())
}

/// Commit D&D au **pointerup** : `1d` → ACK lo+`0x11` → `f0` sub 08 (préambule déjà armé).
pub fn send_matrix_dd_1d_commit_only(state: &mut HelixState, pkt: Vec<u8>) -> Result<(), String> {
    if pkt.len() != 40 || pkt.first() != Some(&0x1d) {
        return Err(format!(
            "send_matrix_dd_1d_commit_only: attendu 1d 40o, reçu {}o",
            pkt.len()
        ));
    }
    let ack_lo = pkt[12].wrapping_add(0x11);
    let ack_hi = pkt[13];
    let post_ed = build_post_1d_ack08(state, ack_lo, ack_hi);
    let post_f0 = build_f0_dd_post_commit_sub08(state);

    state.send(OutPacket::new(pkt));
    state.send(OutPacket::with_delay(post_ed, 50));
    state.send(OutPacket::with_delay(post_f0, 15));
    Ok(())
}

/// Séquence HX Edit matrix D&D en un seul envoi (tests / secours) : arm + commit.
pub fn send_matrix_dd_1d_preamble_commit(
    state: &mut HelixState,
    pkt: Vec<u8>,
) -> Result<(), String> {
    send_matrix_dd_drag_arm(state)?;
    send_matrix_dd_1d_commit_only(state, pkt)
}

/// Construit la trame `1d` : compteurs session + bloc modèle Path 1 + valeur `@input`.
pub fn build_path1_input_source_packet(
    state: &mut HelixState,
    template: &[u8],
    wire_value: u8,
) -> Result<Vec<u8>, String> {
    if template.len() != 40 {
        return Err(format!(
            "liveWriteHex attendu 40 octets (opcode 1d), reçu {}",
            template.len()
        ));
    }
    if template.first() != Some(&0x1d) {
        return Err("liveWriteHex : premier octet doit être 0x1d".to_string());
    }

    let mut pkt = template.to_vec();

    let seq = state.next_x80_cnt();
    pkt[9] = seq;

    let ctr = state.live_write_ctr;
    pkt[12] = (ctr & 0xff) as u8;
    pkt[13] = ((ctr >> 8) & 0xff) as u8;

    let mut model = [0u8; 16];
    if let Some(echo) = state.last_ed03_echo_model {
        if is_path1_input_model_block(&echo) {
            model = echo;
        } else {
            model.copy_from_slice(&pkt[24..40]);
            model[4] = state.live_write_yy;
        }
    } else {
        model.copy_from_slice(&pkt[24..40]);
        model[4] = state.live_write_yy;
    }
    model[12] = wire_value;
    pkt[24..40].copy_from_slice(&model);
    pkt[PATH1_INPUT_WIRE_VALUE_OFFSET] = wire_value;
    pkt[PATH1_IO_MODEL_YY_OFFSET] = state.live_write_yy;

    state.live_write_ctr = ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    Ok(pkt)
}

/// Envoie la source Input Path 1 (`ioSources[].id` dans `HX_ModelUsbAssign.json`).
pub fn send_path1_input_source(
    state: &mut HelixState,
    io_source_id: &str,
) -> Result<String, String> {
    let entry = resolve_io_source_entry(io_source_id)
        .ok_or_else(|| format!("ioSource inconnu : {io_source_id}"))?;

    const INPUT_SLOT_BUS: u8 = 0x00;
    if state.hw_active_slot_bus != Some(INPUT_SLOT_BUS) {
        let focus = build_special_slot_focus_packet(state, INPUT_SLOT_BUS);
        state.send(OutPacket::new(focus));
        state.hw_active_slot_bus = Some(INPUT_SLOT_BUS);
        state.hw_active_slot_index = None;
    }

    let pkt = build_path1_input_source_packet(state, &entry.template, entry.wire_value)?;
    let ctr_lo = pkt[12];
    let ctr_hi = pkt[13];
    let hex: String = pkt.iter().map(|b| format!("{b:02x}")).collect();
    let post = build_post_1d_ack08(state, ctr_lo, ctr_hi);

    state.send(OutPacket::new(pkt));
    state.send(OutPacket::with_delay(post, 8));
    state.path1_input_source_wire = Some(entry.wire_value);

    Ok(format!(
        "parentModelId={} wireValue={} len=40 hex={hex}",
        entry.parent_model_id, entry.wire_value
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dd_preamble_sub10_shape() {
        let mut s = HelixState::new();
        s.session_no = 0x70;
        let ed = build_ed03_dd_preamble_sub10(&mut s);
        assert_eq!(ed[11], 0x10);
        assert_eq!(ed[12], 0x70);
        assert_eq!(&ed[4..8], &[0x80, 0x10, 0xed, 0x03]);
        let f0 = build_f0_dd_preamble_sub10(&mut s);
        assert_eq!(f0[11], 0x10);
        assert_eq!(&f0[4..8], &[0x02, 0x10, 0xf0, 0x03]);
    }

    #[test]
    fn dd_post_f0_sub08_shape() {
        let mut s = HelixState::new();
        let f0 = build_f0_dd_post_commit_sub08(&mut s);
        assert_eq!(f0[11], 0x08);
        assert_eq!(&f0[4..8], &[0x02, 0x10, 0xf0, 0x03]);
    }

    #[test]
    fn stomp_io_sources_load_from_json() {
        let sources = io_sources();
        assert!(
            sources.iter().any(|s| s.id == "HelixStomp_Input_MainLR"),
            "HelixStomp_Input_MainLR"
        );
        assert_eq!(
            sources
                .iter()
                .find(|s| s.id == "HelixStomp_Input_ReturnLR")
                .map(|s| s.wire_value),
            Some(4)
        );
    }

    #[test]
    fn build_patches_wire_value_offset() {
        let entry = resolve_io_source_entry("HelixStomp_Input_Usb56").expect("usb56");
        let mut state = HelixState::new();
        let pkt = build_path1_input_source_packet(&mut state, &entry.template, entry.wire_value)
            .expect("build");
        assert_eq!(pkt[PATH1_INPUT_WIRE_VALUE_OFFSET], 6);
        assert_eq!(pkt[0], 0x1d);
    }

    #[test]
    fn special_focus_packet_targets_input_bus() {
        let mut state = HelixState::new();
        let p = build_special_slot_focus_packet(&mut state, 0x00);
        assert_eq!(p[34], 0x00);
        assert_eq!(p[35], 0x1a);
    }

    #[test]
    fn scan_input_wire_from_in_capture() {
        let hex = "21000018f00302100053000409020000000004001100000082691b6a845200440779176a82620033010081cd";
        let buf: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(scan_path1_input_source_wire(&buf), Some(1));
    }

    #[test]
    fn scroll_input_notify_21_from_capture() {
        for (hex, wire) in [
            (
                "21000018f00302100011000409020000000004001100000082691b6a845200440779176a826200330193c240",
                1u8,
            ),
            (
                "21000018f00302100013000409020000000004001100000082691b6a845200440779176a826200330493c240",
                4u8,
            ),
            (
                "21000018f00302100015000409020000000004001100000082691b6a845200440779176a826200330693c240",
                6u8,
            ),
        ] {
            let buf: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            assert!(is_path1_input_scroll_notify_21(&buf), "scroll 21 {wire}");
            assert_eq!(scan_path1_input_source_wire(&buf), Some(wire));
        }
    }
}
