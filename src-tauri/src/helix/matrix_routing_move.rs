//! Déplacement split / merge (drag & drop HX Edit) — opcode `1d` (40 o), captures
//! `d&d_split.json`, `d&d_merge.json`.
//!
//! Même path : `cd:04`, ancre `64:4e:65`, queue `82:62:<bus>:1a` + ACK `08`.
//! Octet 28 = colonne frontière destination + `0x11` (split col 1 → `0x12`, merge col 8 → `0x19`).
//! Octets 20–23 : `0d 00 00 00` (captures `d&d_split.json`, `d&d_merge.json`).
//! ACK `08` : octet 12 = octet 12 du `1d` + `0x11` (split `70`→`81`, merge `e7`→`f8`), octet 13 inchangé.

use crate::helix::packet::OutPacket;
use crate::helix::path1_io_live_write::build_post_1d_ack08;
use crate::helix::HelixState;

const SPLIT_SLOT_BUS: u8 = 0x0a;
const MERGE_SLOT_BUS: u8 = 0x13;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMarkerKind {
    Split,
    Merge,
}

impl RoutingMarkerKind {
    fn slot_bus(self) -> u8 {
        match self {
            Self::Split => SPLIT_SLOT_BUS,
            Self::Merge => MERGE_SLOT_BUS,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Merge => "merge",
        }
    }
}

/// Octet après `83:66:cd:04` — `boundary_col + 0x11` (0..8 → `0x11`..`0x19`).
pub fn routing_move_tag_byte(dest_boundary_col: u8) -> Result<u8, String> {
    if dest_boundary_col > 8 {
        return Err(format!(
            "routing move : colonne frontière hors plage 0..8 ({dest_boundary_col})"
        ));
    }
    Ok(dest_boundary_col.wrapping_add(0x11))
}

pub fn build_matrix_routing_marker_move_packet(
    state: &mut HelixState,
    kind: RoutingMarkerKind,
    dest_boundary_col: u8,
) -> Result<Vec<u8>, String> {
    let tag = routing_move_tag_byte(dest_boundary_col)?;
    let slot_bus = kind.slot_bus();

    let cnt = state.next_x80_cnt();
    let session = state.session_no;
    let double = state.preset_data_packet_double();

    let pkt = vec![
        0x1d,
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
        session,
        double[0],
        double[1],
        0x00,
        0x01,
        0x00,
        0x06,
        0x00,
        0x0d,
        0x00,
        0x00,
        0x00,
        0x83,
        0x66,
        0xcd,
        0x04,
        tag,
        0x64,
        0x4e,
        0x65,
        0x82,
        0x62,
        slot_bus,
        0x1a,
        0x00,
        0x00,
        0x00,
        0x00,
    ];

    Ok(pkt)
}

/// Déplace split ou merge vers une nouvelle colonne frontière (0..8).
pub fn send_matrix_routing_marker_move(
    state: &mut HelixState,
    kind: RoutingMarkerKind,
    dest_boundary_col: u8,
    first_path2_col: u8,
    last_path2_col: u8,
    current_split_col: u8,
    current_merge_col: u8,
) -> Result<String, String> {
    routing_move_tag_byte(dest_boundary_col)?;

    if first_path2_col > 7 || last_path2_col > 7 {
        return Err("routing move : colonnes path 2 invalides".to_string());
    }

    match kind {
        RoutingMarkerKind::Split => {
            if dest_boundary_col > first_path2_col {
                return Err(format!(
                    "routing move split : colonne {dest_boundary_col} après le premier slot path 2 ({first_path2_col})"
                ));
            }
            if dest_boundary_col >= current_merge_col {
                return Err(format!(
                    "routing move split : colonne {dest_boundary_col} doit rester avant merge ({current_merge_col})"
                ));
            }
        }
        RoutingMarkerKind::Merge => {
            let min_merge = last_path2_col.saturating_add(1).min(8);
            if dest_boundary_col < min_merge {
                return Err(format!(
                    "routing move merge : colonne {dest_boundary_col} avant la fin path 2 (min {min_merge})"
                ));
            }
            if dest_boundary_col <= current_split_col {
                return Err(format!(
                    "routing move merge : colonne {dest_boundary_col} doit rester après split ({current_split_col})"
                ));
            }
        }
    }

    let pkt = build_matrix_routing_marker_move_packet(state, kind, dest_boundary_col)?;
    // Move matrix `1d` (session @ 12) : captures HX Edit ACK avec lo + 0x11 (`d&d_split` / `d&d_merge`).
    let ack_lo = pkt[12].wrapping_add(0x11);
    let ack_hi = pkt[13];
    let post = build_post_1d_ack08(state, ack_lo, ack_hi);
    let hex = pkt
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");

    eprintln!(
        "[MatrixRoutingMove][sent] {} col={dest_boundary_col} tag={:#04x} bus={:#04x} packet={hex}",
        kind.label(),
        pkt[28],
        kind.slot_bus()
    );

    state.send(OutPacket::new(pkt));
    state.send(OutPacket::with_delay(post, 8));

    state.hw_active_slot_index = None;
    state.hw_active_slot_bus = Some(kind.slot_bus());
    state.hw_active_slot_sequence = state.hw_active_slot_sequence.wrapping_add(1);

    Ok(format!(
        "routing_move_{} col->{dest_boundary_col} bus {:#04x}",
        kind.label(),
        kind.slot_bus()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> HelixState {
        let mut s = HelixState::new();
        s.session_no = 0xdc;
        s.live_write_yy = 0x13;
        s
    }

    #[test]
    fn routing_move_split_capture_shape() {
        let mut s = test_state();
        let pkt = build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Split, 1).unwrap();
        assert_eq!(pkt.len(), 40);
        assert_eq!(pkt[0], 0x1d);
        assert_eq!(pkt[27], 0x04);
        assert_eq!(pkt[28], 0x12);
        assert_eq!(&pkt[20..24], &[0x0d, 0x00, 0x00, 0x00]);
        assert_eq!(&pkt[29..32], &[0x64, 0x4e, 0x65]);
        assert_eq!(&pkt[32..40], &[0x82, 0x62, 0x0a, 0x1a, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn routing_move_merge_capture_shape() {
        let mut s = test_state();
        let pkt = build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Merge, 8).unwrap();
        assert_eq!(pkt[28], 0x19);
        assert_eq!(&pkt[32..40], &[0x82, 0x62, 0x13, 0x1a, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn routing_move_ack_lo_is_session_plus_11() {
        let mut s = test_state();
        s.session_no = 0x70;
        let pkt =
            build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Split, 1).unwrap();
        assert_eq!(pkt[12], 0x70);
        assert_eq!(pkt[12].wrapping_add(0x11), 0x81);
        s.session_no = 0xe7;
        let pkt =
            build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Merge, 8).unwrap();
        assert_eq!(pkt[12], 0xe7);
        assert_eq!(pkt[12].wrapping_add(0x11), 0xf8);
    }

    #[test]
    fn routing_move_split_guard_first_path2() {
        let mut s = test_state();
        let err = send_matrix_routing_marker_move(
            &mut s,
            RoutingMarkerKind::Split,
            3,
            2,
            5,
            1,
            7,
        )
        .unwrap_err();
        assert!(err.contains("premier slot path 2"));
    }

    #[test]
    fn routing_move_merge_guard_last_path2() {
        let mut s = test_state();
        let err = send_matrix_routing_marker_move(
            &mut s,
            RoutingMarkerKind::Merge,
            4,
            2,
            5,
            1,
            7,
        )
        .unwrap_err();
        assert!(err.contains("fin path 2"));
    }
}
