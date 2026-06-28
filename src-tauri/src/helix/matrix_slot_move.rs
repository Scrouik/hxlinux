//! Déplacement matrix drag & drop — opcode `1d` (40 o), captures HX Edit
//! `d&d_same_path_slot*.json`, `d&d_path1_to_path2.json`, etc.
//!
//! Même path : `1d` (`82:4b:<src_bus>:4c:<dst_bus>`) + ACK `08`.
//! Inter-path : `1d` puis 2× `19` (commit branche dual-path, ancre `64:17:65:c0`).

use crate::helix::packet::OutPacket;
use crate::helix::path1_io_live_write::build_post_1d_ack08;
use crate::helix::{kempline_index_to_slot_bus, HelixState};

const BRANCH_COMMIT_19_SPLIT: [u8; 36] = [
    0x19, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0x00, 0x64,
    0x17, 0x65, 0xc0, 0x00, 0x00, 0x00,
];

/// Second paquet inter-path (`d&d_path1_to_path2.json` : `…641665c0` vs split `…641765c0`).
const BRANCH_COMMIT_19_MERGE: [u8; 36] = [
    0x19, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0x00, 0x64,
    0x16, 0x65, 0xc0, 0x00, 0x00, 0x00,
];

fn matrix_move_cd_variant(src_index: usize, dst_index: usize) -> u8 {
    let src_path2 = src_index >= 8;
    let dst_path2 = dst_index >= 8;
    if src_path2 == dst_path2 {
        0x04
    } else {
        0x03
    }
}

fn patch_branch_commit_19(pkt: &mut [u8], state: &mut HelixState) {
    pkt[9] = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    pkt[12] = (ctr & 0xff) as u8;
    pkt[13] = ((ctr >> 8) & 0xff) as u8;
    pkt[28] = state.live_write_yy;
    state.live_write_ctr = ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
}

pub fn build_matrix_slot_move_packet(
    state: &mut HelixState,
    src_index: usize,
    dst_index: usize,
) -> Result<Vec<u8>, String> {
    if src_index >= 16 || dst_index >= 16 {
        return Err("matrix move : index slot hors plage 0..15".to_string());
    }
    if src_index == dst_index {
        return Err("matrix move : source et destination identiques".to_string());
    }
    let src_bus = kempline_index_to_slot_bus(src_index)
        .ok_or_else(|| format!("matrix move : bus source invalide pour index {src_index}"))?;
    let dst_bus = kempline_index_to_slot_bus(dst_index)
        .ok_or_else(|| format!("matrix move : bus destination invalide pour index {dst_index}"))?;
    let cd = matrix_move_cd_variant(src_index, dst_index);

    let cnt = state.next_x80_cnt();
    let session = state.session_no;
    let double = state.preset_data_packet_double();
    let yy = state.live_write_yy;

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
        cd,
        yy,
        0x64,
        0x2b,
        0x65,
        0x82,
        0x4b,
        src_bus,
        0x4c,
        dst_bus,
        0x00,
        0x00,
        0x00,
    ];

    state.live_write_yy = yy.wrapping_add(1);
    Ok(pkt)
}

fn send_branch_commit_pair(state: &mut HelixState) -> Result<(), String> {
    for template in [BRANCH_COMMIT_19_SPLIT, BRANCH_COMMIT_19_MERGE] {
        let mut pkt = template.to_vec();
        patch_branch_commit_19(&mut pkt, state);
        state.send(OutPacket::with_delay(pkt, 8));
    }
    Ok(())
}

/// `HXLINUX_DD_DUMP_ACK_PRIME=0` → témoin (n'amorce pas la lane dump ACK).
fn dd_dump_ack_prime_enabled() -> bool {
    match std::env::var("HXLINUX_DD_DUMP_ACK_PRIME").as_deref() {
        Ok(v) if v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false") => false,
        Ok(_) => true,
        Err(_) => true,
    }
}

/// Amorce [`HelixState::editor_ed03_lane`] pour acquitter le dump AUTO post-commit
/// inter-path. Les commits `19` patchent `live_write_ctr` ; les ACK chunks 272
/// partent sur `editor_ed03_lane` (cf. `preset_dump_stream_ack`).
///
/// Mesuré sur captures : `lo = session_no + 0x42` (aligné merge commit),
/// `hi = double[0] + 1`, `editor_ed03_lane_b14 = 0`.
pub fn prime_dump_ack_lane_after_interpath(state: &mut HelixState) {
    let double = state.preset_data_packet_double();
    let lane_lo = state.session_no.wrapping_add(0x42);
    let lane_hi = double[0].wrapping_add(1);
    state.editor_ed03_lane = (lane_hi as u16) << 8 | (lane_lo as u16);
    state.editor_ed03_lane_b14 = 0;
    crate::helix::init_trace::trace_fmt(format_args!(
        "prime_dump_ack_lane_after_interpath lo={lane_lo:#04x} hi={lane_hi:#04x} b14=0"
    ));
}

fn prime_dump_ack_lane_after_interpath_if_enabled(state: &mut HelixState) {
    if dd_dump_ack_prime_enabled() {
        prime_dump_ack_lane_after_interpath(state);
    }
}

/// Déplace un bloc FX matrix (drag & drop HX Edit).
pub fn send_matrix_slot_move(
    state: &mut HelixState,
    src_index: usize,
    dest_index: usize,
) -> Result<String, String> {
    let inter_path = (src_index >= 8) != (dest_index >= 8);
    let pkt = build_matrix_slot_move_packet(state, src_index, dest_index)?;
    let ack_lo = pkt[12];
    let ack_hi = pkt[13];
    let src_bus = pkt[34];
    let dst_bus = pkt[36];
    let post = build_post_1d_ack08(state, ack_lo, ack_hi);

    state.send(OutPacket::new(pkt));
    state.send(OutPacket::with_delay(post, 8));

    let mut lines = vec![format!(
        "move_1d {src_index}->{dest_index} bus {src_bus:#04x}->{dst_bus:#04x}"
    )];

    if inter_path {
        send_branch_commit_pair(state)?;
        prime_dump_ack_lane_after_interpath_if_enabled(state);
        lines.push("branch_commit_19x2".to_string());
        if dd_dump_ack_prime_enabled() {
            lines.push("prime_dump_ack_lane".to_string());
        }
    }

    if let Some(dst_bus) = kempline_index_to_slot_bus(dest_index) {
        state.hw_active_slot_index = Some(dest_index);
        state.hw_active_slot_bus = Some(dst_bus);
        state.hw_active_slot_sequence = state.hw_active_slot_sequence.wrapping_add(1);
    }

    Ok(lines.join(" | "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> HelixState {
        let mut s = HelixState::new();
        s.session_no = 0xdc;
        s.live_write_yy = 0x13;
        s.live_write_ctr = 0x4c00;
        s
    }

    #[test]
    fn matrix_move_same_path_slot1_to_2_shape() {
        let mut s = test_state();
        s.preset_dump_ack_ctr = 0x4c00;
        let pkt = build_matrix_slot_move_packet(&mut s, 0, 1).unwrap();
        assert_eq!(pkt.len(), 40);
        assert_eq!(pkt[0], 0x1d);
        assert_eq!(pkt[27], 0x04);
        assert_eq!(&pkt[32..40], &[0x82, 0x4b, 0x01, 0x4c, 0x02, 0x00, 0x00, 0x00]);
        assert_eq!(pkt[28], 0x13);
        assert_eq!(s.live_write_yy, 0x14);
    }

    #[test]
    fn matrix_move_same_path_slot3_to_8_shape() {
        let mut s = test_state();
        let pkt = build_matrix_slot_move_packet(&mut s, 2, 7).unwrap();
        assert_eq!(&pkt[32..40], &[0x82, 0x4b, 0x03, 0x4c, 0x08, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn matrix_move_inter_path_uses_cd03() {
        let mut s = test_state();
        let pkt = build_matrix_slot_move_packet(&mut s, 1, 9).unwrap();
        assert_eq!(pkt[27], 0x03);
        assert_eq!(pkt[34], 0x02);
        assert_eq!(pkt[36], 0x0c);
    }

    #[test]
    fn branch_commit_merge_differs_from_split() {
        assert_ne!(BRANCH_COMMIT_19_SPLIT[30], BRANCH_COMMIT_19_MERGE[30]);
        assert_eq!(BRANCH_COMMIT_19_SPLIT[30], 0x17);
        assert_eq!(BRANCH_COMMIT_19_MERGE[30], 0x16);
    }

    #[test]
    fn prime_dump_ack_lane_after_interpath_sets_editor_lane() {
        let mut s = test_state();
        s.session_no = 0xdc;
        let double = s.preset_data_packet_double();
        prime_dump_ack_lane_after_interpath(&mut s);
        let [lo, hi] = s.editor_ed03_lane_bytes();
        assert_eq!(lo, 0xdcu8.wrapping_add(0x42));
        assert_eq!(hi, double[0].wrapping_add(1));
        assert_eq!(s.editor_ed03_lane_b14, 0);
    }

    #[test]
    fn dd_dump_ack_prime_disabled_by_env() {
        std::env::set_var("HXLINUX_DD_DUMP_ACK_PRIME", "0");
        assert!(!dd_dump_ack_prime_enabled());
        std::env::remove_var("HXLINUX_DD_DUMP_ACK_PRIME");
        assert!(dd_dump_ack_prime_enabled());
    }
}
