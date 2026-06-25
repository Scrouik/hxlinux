//! Live write type Split Path 1 (`opcode 0x25`, 48 octets) — HX Edit `split select.json`.
//!
//! Séquence observée par changement de type Split :
//!   OUT `25` (48 o) → OUT `08` ed03 (16 o) ; IN `21` avec `82:62:0a:1a:…:05`.

use std::sync::OnceLock;

use serde_json::Value;

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

const USB_ASSIGN_JSON: &str = include_str!("../../resources/HX_ModelUsbAssign.json");

/// Octets `c2:19:cd:??` dans la trame `25` (capture HX Edit).
const PATH1_SPLIT_CD_SUB_OFFSET: usize = 41;
const PATH1_SPLIT_WIRE_OFFSET: usize = 42;

#[derive(Clone, Debug)]
struct SplitSourceEntry {
    id: String,
    catalog_model_id: String,
    wire_value: u8,
    template: Vec<u8>,
}

static SPLIT_SOURCES: OnceLock<Vec<SplitSourceEntry>> = OnceLock::new();

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

fn load_split_sources() -> Vec<SplitSourceEntry> {
    let v: Value = serde_json::from_str(USB_ASSIGN_JSON).unwrap_or(Value::Null);
    let Some(arr) = v.get("splitSources").and_then(|x| x.as_array()) else {
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
        let catalog_model_id = obj
            .get("catalogModelId")
            .or_else(|| obj.get("parentModelId"))
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
        if template.len() != 48 || template.first() != Some(&0x25) {
            continue;
        }
        out.push(SplitSourceEntry {
            id,
            catalog_model_id,
            wire_value,
            template,
        });
    }
    out
}

fn split_sources() -> &'static [SplitSourceEntry] {
    SPLIT_SOURCES.get_or_init(load_split_sources)
}

fn resolve_split_source_entry(split_source_id: &str) -> Option<&'static SplitSourceEntry> {
    let id = split_source_id.trim();
    split_sources().iter().find(|e| e.id == id)
}

/// Ancre scroll Split Path 1 dans IN `21` 44 o (`split scroll.json` / HX Edit).
const SPLIT_SCROLL_21_ANCHOR: [u8; 12] = [
    0x82, 0x69, 0x27, 0x6a, 0x84, 0x52, 0x01, 0x44, 0x03, 0x79, 0x13, 0x6a,
];

fn is_path1_split_type_wire_value(w: u8) -> bool {
    matches!(w, 0x00 | 0x01 | 0x02 | 0x33)
}

fn has_split_path1_flow_context(buf: &[u8]) -> bool {
    buf.windows(4).any(|w| w == [0x82, 0x0d, 0x0a, 0x18])
        || buf.windows(SPLIT_SCROLL_21_ANCHOR.len())
            .any(|w| w == SPLIT_SCROLL_21_ANCHOR)
}

/// Octet `i+6` brut → wire catalogue (`splitSources[].wireValue`).
///
/// Deux conventions Line 6 :
/// - **Select / OUT `25` / HX Edit (`TT=05`)** : `0=Y`, `1=A/B` (JSON `splitSources[]`).
/// - **Scroll hardware Stomp** (`21` + ancre, ou `TT=92`) : **Y=1**, **A/B=0** sur le fil.
pub(crate) fn split_type_wire_in_21_to_catalog(raw: u8, term: u8, scroll_notify_21: bool) -> u8 {
    if term == 0x05 {
        return raw;
    }
    if scroll_notify_21 && matches!(raw, 0x00 | 0x01) {
        return raw ^ 1;
    }
    if term == 0x92 && matches!(raw, 0x00 | 0x01) {
        return raw ^ 1;
    }
    raw
}

/// Type Split depuis IN `21` / select UI : `82:62:0a:1a:…:WW:TT`.
/// Comme Input (`82:62:00:33:WW`), on ne filtre pas `TT` — seul `WW` catalogue compte ;
/// swap Y/A/B si scroll hardware (`21` 44 o + ancre) ou `TT=92`, jamais sur select `05`.
fn scan_path1_split_type_wire_in_21(buf: &[u8]) -> Option<u8> {
    let scroll_notify_21 = is_path1_split_scroll_notify_21(buf);
    let mut i = 0usize;
    while i + 8 <= buf.len() {
        if buf[i] == 0x82
            && buf[i + 1] == 0x62
            && buf[i + 2] == 0x0a
            && buf[i + 3] == 0x1a
        {
            let raw = buf[i + 6];
            let term = buf[i + 7];
            if is_path1_split_type_wire_value(raw) {
                return Some(split_type_wire_in_21_to_catalog(raw, term, scroll_notify_21));
            }
        }
        i += 1;
    }
    None
}

/// Type Split depuis pré-notif scroll ed03 (`82:0d:0a:18` … `84:08:cd:01:WW` ou `cd:02:33`).
/// Capture `split scroll.json` frames 1055 / 1485 / 1975 / 2741 (avant IN `21`).
pub(crate) fn scan_path1_split_type_wire_ed03_scroll(buf: &[u8]) -> Option<u8> {
    if !has_split_path1_flow_context(buf) {
        return None;
    }
    let mut i = 0usize;
    while i + 5 <= buf.len() {
        if buf[i..i + 4] == [0x84, 0x08, 0xcd, 0x01] {
            let w = buf[i + 4];
            if is_path1_split_type_wire_value(w) {
                return Some(w);
            }
        }
        if i + 6 <= buf.len()
            && buf[i..i + 4] == [0x84, 0x08, 0xcd, 0x02]
            && buf[i + 4] == 0x33
        {
            return Some(0x33);
        }
        i += 1;
    }
    None
}

/// Type Split Path 1 dans trames IN USB (IN `21` scroll/select, ou ed03 pré-scroll HX Edit).
pub fn scan_path1_split_type_wire(buf: &[u8]) -> Option<u8> {
    scan_path1_split_type_wire_in_21(buf).or_else(|| scan_path1_split_type_wire_ed03_scroll(buf))
}

/// IN `21` (44 o) quand l'utilisateur **scroll** le type Split sur le hardware (`split scroll.json`).
pub fn is_path1_split_scroll_notify_21(data: &[u8]) -> bool {
    data.len() == 44
        && data.first() == Some(&0x21)
        && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
        && data.windows(SPLIT_SCROLL_21_ANCHOR.len())
            .any(|w| w == SPLIT_SCROLL_21_ANCHOR)
        && data.windows(4).any(|w| w == [0x82, 0x62, 0x0a, 0x1a])
}

fn patch_split_wire_in_packet(pkt: &mut [u8], wire_value: u8) {
    if pkt.len() <= PATH1_SPLIT_WIRE_OFFSET {
        return;
    }
    if wire_value == 0x33 {
        pkt[PATH1_SPLIT_CD_SUB_OFFSET] = 0x02;
        pkt[PATH1_SPLIT_WIRE_OFFSET] = 0x33;
    } else {
        pkt[PATH1_SPLIT_CD_SUB_OFFSET] = 0x01;
        pkt[PATH1_SPLIT_WIRE_OFFSET] = wire_value;
    }
}

/// Bloc modèle 16 o embarqué dans la trame `25` (`pkt[24..40]`) — offsets locaux `cd` / wire.
fn patch_split_wire_in_model_block(block: &mut [u8; 16], wire_value: u8) {
    if wire_value == 0x33 {
        block[9] = 0x02;
        block[10] = 0x33;
    } else {
        block[9] = 0x01;
        block[10] = wire_value;
    }
}

fn is_path1_split_model_block(block: &[u8; 16]) -> bool {
    block.len() >= 11 && block[8..11] == [0x82, 0x62, 0x0a]
}

fn build_post_25_ack08(state: &mut HelixState, ctr_lo: u8, ctr_hi: u8) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x08, ctr_lo, ctr_hi,
        0x00, 0x00,
    ]
}

pub fn build_path1_split_type_packet(
    state: &mut HelixState,
    template: &[u8],
    wire_value: u8,
) -> Result<Vec<u8>, String> {
    if template.len() != 48 {
        return Err(format!(
            "liveWriteHex attendu 48 octets (opcode 25), reçu {}",
            template.len()
        ));
    }
    if template.first() != Some(&0x25) {
        return Err("liveWriteHex : premier octet doit être 0x25".to_string());
    }

    let mut pkt = template.to_vec();
    let seq = state.next_x80_cnt();
    pkt[9] = seq;

    let ctr = state.live_write_ctr;
    pkt[12] = (ctr & 0xff) as u8;
    pkt[13] = ((ctr >> 8) & 0xff) as u8;

    let mut model = [0u8; 16];
    if let Some(echo) = state.last_ed03_echo_model {
        if is_path1_split_model_block(&echo) {
            model = echo;
        } else {
            model.copy_from_slice(&pkt[24..40]);
            model[4] = state.live_write_yy;
        }
    } else {
        model.copy_from_slice(&pkt[24..40]);
        model[4] = state.live_write_yy;
    }
    patch_split_wire_in_model_block(&mut model, wire_value);
    pkt[24..40].copy_from_slice(&model);
    patch_split_wire_in_packet(&mut pkt, wire_value);
    pkt[28] = state.live_write_yy;

    state.live_write_ctr = ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    Ok(pkt)
}

/// Envoie le type Split Path 1 (`splitSources[].id` dans `HX_ModelUsbAssign.json`).
pub fn send_path1_split_type(
    state: &mut HelixState,
    split_source_id: &str,
) -> Result<String, String> {
    let entry = resolve_split_source_entry(split_source_id)
        .ok_or_else(|| format!("splitSource inconnu : {split_source_id}"))?;

    const SPLIT_SLOT_BUS: u8 = 0x0a;
    if state.hw_active_slot_bus != Some(SPLIT_SLOT_BUS) {
        let focus = crate::helix::path1_io_live_write::build_special_slot_focus_packet(
            state,
            SPLIT_SLOT_BUS,
        );
        state.send(OutPacket::new(focus));
        state.hw_active_slot_bus = Some(SPLIT_SLOT_BUS);
        state.hw_active_slot_index = None;
    }

    let pkt = build_path1_split_type_packet(state, &entry.template, entry.wire_value)?;
    let ctr_lo = pkt[12];
    let ctr_hi = pkt[13];
    let hex: String = pkt.iter().map(|b| format!("{b:02x}")).collect();
    let post = build_post_25_ack08(state, ctr_lo, ctr_hi);

    state.send(OutPacket::new(pkt));
    state.send(OutPacket::with_delay(post, 8));
    state.path1_split_type_wire = Some(entry.wire_value);

    Ok(format!(
        "catalogModelId={} wireValue={} len=48 hex={hex}",
        entry.catalog_model_id, entry.wire_value
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_sources_load_from_json() {
        let sources = split_sources();
        assert!(
            sources.iter().any(|s| s.id == "HelixStomp_Split_AB"),
            "HelixStomp_Split_AB"
        );
        assert_eq!(
            sources
                .iter()
                .find(|s| s.id == "HelixStomp_Split_Dynamic")
                .map(|s| s.wire_value),
            Some(0x33)
        );
    }

    #[test]
    fn scan_split_wire_from_in_capture() {
        let hex = "21000018f0030210005900040902000000000400110000008269276a845201440379136a82620a1a00010105";
        let buf: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(scan_path1_split_type_wire(&buf), Some(0x01));
    }

    #[test]
    fn scan_split_dynamic_wire() {
        let hex = "21000018f0030210006200040902000000000400110000008269276a845201440379136a82620a1a00023305";
        let buf: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(scan_path1_split_type_wire(&buf), Some(0x33));
    }

    #[test]
    fn scroll_split_ed03_wire_from_capture() {
        for (hex, wire) in [
            (
                "4c000018ed03801000c90004e603000000000006003c0000008366cd03fe670068820d0a1882130214830e82050007830200030004900f8408cd01010d010ac307830203030493ca3f000000ca3f000000c212c3",
                1u8,
            ),
            (
                "47000018ed03801000da00041b0400000000000600370000008366cd0401670068820d0a1882130214830e82050007830200030004900f8408cd01000d010ac307830202030292ca3f000000c212c348",
                0u8,
            ),
            (
                "48000018ed03801000eb0004500400000000000600380000008366cd0404670068820d0a1882130214830e82050007830200030004900f8408cd01020d010ac307830203030493ca43fa0000c2c212c3",
                2u8,
            ),
            (
                "52000018ed03801000fd0004850400000000000600420000008366cd0407670068820d0a1882130214830e82050007830200030004900f8408cd02330d010ac307830205030595cac1700000ca3f5c28f6ca3f5c28f6c2c212c3000d",
                0x33u8,
            ),
        ] {
            let buf: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            assert_eq!(scan_path1_split_type_wire(&buf), Some(wire), "wire {wire}");
        }
    }

    #[test]
    fn scroll_split_notify_21_from_capture() {
        let hex = "21000018f0030210009600040902000000000400110000008269276a845201440379136a82620a1a006daf53";
        let buf: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert!(is_path1_split_scroll_notify_21(&buf));
        assert_eq!(scan_path1_split_type_wire(&buf), None);
    }

    #[test]
    fn scroll_split_linux_in_21_wire_from_capture() {
        // `split scroll Linux.json` — octet i+6 inversé Y/A/B vs select HX Edit (`05`).
        for (hex, catalog_wire) in [
            (
                "21000018f0030210004700040902000000000400110000008269276a845201440379136a82620a1a00010192",
                0u8, // raw 1 → Y
            ),
            (
                "21000018f0030210004a00040902000000000400110000008269276a845201440379136a82620a1a00010092",
                1u8, // raw 0 → A/B
            ),
            (
                "21000018f0030210004c00040902000000000400110000008269276a845201440379136a82620a1a00010292",
                2u8,
            ),
            (
                "21000018f0030210004e00040902000000000400110000008269276a845201440379136a82620a1a00023392",
                0x33u8,
            ),
        ] {
            let buf: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            assert!(is_path1_split_scroll_notify_21(&buf), "scroll21 {catalog_wire}");
            assert_eq!(
                scan_path1_split_type_wire(&buf),
                Some(catalog_wire),
                "catalog_wire {catalog_wire}"
            );
        }
    }

    #[test]
    fn split_type_wire_in_21_y_ab_swap_only_on_stomp_scroll() {
        assert_eq!(split_type_wire_in_21_to_catalog(0, 0x92, false), 1);
        assert_eq!(split_type_wire_in_21_to_catalog(1, 0x92, false), 0);
        assert_eq!(split_type_wire_in_21_to_catalog(0, 0x05, true), 0);
        assert_eq!(split_type_wire_in_21_to_catalog(1, 0x05, true), 1);
        assert_eq!(split_type_wire_in_21_to_catalog(1, 0x81, true), 0);
        assert_eq!(split_type_wire_in_21_to_catalog(0, 0x81, true), 1);
        assert_eq!(split_type_wire_in_21_to_catalog(2, 0x92, true), 2);
    }

    #[test]
    fn split_type_in_21_accepts_nonstandard_terminator_like_input() {
        // XL scroll : brut 1 = Y sur le Stomp → wire catalogue 0 (JSON / select HX Edit).
        let hex = "21000018f0030210004700040902000000000400110000008269276a845201440379136a82620a1a00010181";
        let buf: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(scan_path1_split_type_wire(&buf), Some(0x00));
    }

    #[test]
    fn hxedit_scroll_garbage_tail_not_parsed_as_wire() {
        let hex = "21000018f0030210009900040902000000000400110000008269276a845201440379136a82620a1a000e8205";
        let buf: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(scan_path1_split_type_wire(&buf), None);
    }

    #[test]
    fn patch_wire_offsets() {
        let mut pkt = vec![0u8; 48];
        patch_split_wire_in_packet(&mut pkt, 0x02);
        assert_eq!(pkt[41], 0x01);
        assert_eq!(pkt[42], 0x02);
        patch_split_wire_in_packet(&mut pkt, 0x33);
        assert_eq!(pkt[41], 0x02);
        assert_eq!(pkt[42], 0x33);
    }
}
