// ! Matrix_routing move.rs
//! Déplacement split / merge (drag & drop HX Edit) — opcodes `1d` + `1b`.
//! Capture `d&d_split.json` : lane **`cd:03`**, un `next_editor_ed03_double()` par bulk `1d` **et** `1b`.

use std::sync::{Arc, Mutex};

use crate::helix::matrix_routing_dd::execute_routing_marker_dd;
use crate::helix::HelixState;

const SPLIT_SLOT_BUS: u8 = 0x0a;
const MERGE_SLOT_BUS: u8 = 0x13;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMarkerKind {
    Split,
    Merge,
}

impl RoutingMarkerKind {
    pub fn slot_bus(self) -> u8 {
        match self {
            Self::Split => SPLIT_SLOT_BUS,
            Self::Merge => MERGE_SLOT_BUS,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Merge => "merge",
        }
    }
}

/// Valide la colonne frontière 0..8 (le tag sur le fil est `editor_ed03_double`, pas `col+0x11`).
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
) -> Result<(Vec<u8>, u8), String> {
    routing_move_tag_byte(dest_boundary_col)?;
    let tag = state.next_editor_ed03_double()[0];
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
        0x03,
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

    Ok((pkt, tag))
}

fn validate_routing_marker_move(
    kind: RoutingMarkerKind,
    dest_boundary_col: u8,
    first_path2_col: u8,
    last_path2_col: u8,
    current_split_col: u8,
    current_merge_col: u8,
) -> Result<(), String> {
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
    Ok(())
}

/// Déplace split ou merge (séquence complète dans `matrix_routing_dd`).
pub fn execute_matrix_routing_marker_move(
    helix_arc: Arc<Mutex<HelixState>>,
    kind: RoutingMarkerKind,
    dest_boundary_col: u8,
    first_path2_col: u8,
    last_path2_col: u8,
    current_split_col: u8,
    current_merge_col: u8,
) -> Result<String, String> {
    validate_routing_marker_move(
        kind,
        dest_boundary_col,
        first_path2_col,
        last_path2_col,
        current_split_col,
        current_merge_col,
    )?;
    execute_routing_marker_dd(helix_arc, kind, dest_boundary_col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> HelixState {
        let mut s = HelixState::new();
        s.session_no = 0xdc;
        s.live_write_yy = 0x13;
        s.editor_ed03_double = 0x64f1;
        s
    }

    #[test]
    fn routing_move_split_capture_shape() {
        let mut s = test_state();
        let (pkt, tag) =
            build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Split, 1).unwrap();
        assert_eq!(tag, 0xf2);
        assert_eq!(pkt.len(), 40);
        assert_eq!(pkt[0], 0x1d);
        assert_eq!(pkt[27], 0x03);
        assert_eq!(pkt[28], 0xf2);
        assert_eq!(&pkt[20..24], &[0x0d, 0x00, 0x00, 0x00]);
        assert_eq!(&pkt[29..32], &[0x64, 0x4e, 0x65]);
        assert_eq!(&pkt[32..40], &[0x82, 0x62, 0x0a, 0x1a, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn routing_move_merge_capture_shape() {
        let mut s = test_state();
        let (pkt, tag) =
            build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Merge, 8).unwrap();
        assert_eq!(tag, 0xf2);
        assert_eq!(pkt[27], 0x03);
        assert_eq!(pkt[28], 0xf2);
        assert_eq!(&pkt[32..40], &[0x82, 0x62, 0x13, 0x1a, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn routing_move_ack_lo_is_session_plus_11() {
        let mut s = test_state();
        s.session_no = 0x70;
        let (pkt, _) =
            build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Split, 1).unwrap();
        assert_eq!(pkt[12], 0x70);
        assert_eq!(pkt[12].wrapping_add(0x11), 0x81);
        s.session_no = 0xe7;
        s.editor_ed03_double = 0x64f1;
        let (pkt, _) =
            build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Merge, 8).unwrap();
        assert_eq!(pkt[12], 0xe7);
        assert_eq!(pkt[12].wrapping_add(0x11), 0xf8);
    }

    #[test]
    fn routing_move_split_guard_first_path2() {
        let err = validate_routing_marker_move(RoutingMarkerKind::Split, 3, 2, 5, 1, 7).unwrap_err();
        assert!(err.contains("premier slot path 2"));
    }

    #[test]
    fn routing_move_merge_guard_last_path2() {
        let err = validate_routing_marker_move(RoutingMarkerKind::Merge, 4, 2, 5, 1, 7).unwrap_err();
        assert!(err.contains("fin path 2"));
    }

    #[test]
    fn routing_move_two_cycles_advance_double_twice_per_gesture() {
        let mut s = test_state();
        s.editor_ed03_double = 0x64f7;
        let (_, tag1d) =
            build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Split, 0).unwrap();
        assert_eq!(tag1d, 0xf8);
        let pkt1b = crate::helix::matrix_routing_dd::build_matrix_routing_path2_commit_packet(
            &mut s,
            RoutingMarkerKind::Split,
            0,
            [0xdc, 0xf8, 0x64],
        );
        assert_eq!(pkt1b[28], 0xf9);
        let (_, tag2d) =
            build_matrix_routing_marker_move_packet(&mut s, RoutingMarkerKind::Split, 1).unwrap();
        assert_eq!(tag2d, 0xfa, "2ᵉ 1d ne doit pas réutiliser le tag du 1b précédent");
    }
}
