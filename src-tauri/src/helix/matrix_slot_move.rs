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

/// `HX_DD_CREATE_SPLIT_FAITHFUL=0` → témoin (comportement historique : commits `19` même en
/// création de split → device en no-op, le bloc ne bouge pas).
fn create_split_faithful_enabled() -> bool {
    match std::env::var("HX_DD_CREATE_SPLIT_FAITHFUL").as_deref() {
        Ok(v) if v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false") => false,
        _ => true,
    }
}

/// Trame `08` ed:03 création de split : préambule (`sub=0x10`) ou ACK (`sub=0x08`), portant le
/// **lane éditeur** (`live_write_ctr`) à l'offset 12-13, offset 14-15 = `00 00`
/// (capture `d&d_path1_to_path2_before_split.json`).
fn build_ed03_lane08(state: &mut HelixState, sub: u8, lane: u16) -> Vec<u8> {
    let cnt = state.next_x80_cnt();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt, 0x00, sub,
        (lane & 0xff) as u8, (lane >> 8) as u8, 0x00, 0x00,
    ]
}

/// SELECT de la source AVANT le MOVE (création de split, capture before_split).
/// `1d …<lane_lo> <lane_hi> 00 00 …83 66 cd 03 <yy> 64 4e 65 82 62 <src_bus> 1a`.
/// **Copie conforme HX Edit** : lane éditeur (`live_write_ctr`) à l'offset 12-13 — PAS session+double
/// (statique = deux écritures identiques → device no-op) ; `yy = live_write_yy` séquentiel.
fn build_select_source_lane(state: &mut HelixState, src_bus: u8, lane: u16) -> Vec<u8> {
    let cnt = state.next_x80_cnt();
    let yy = state.live_write_yy;
    state.live_write_yy = yy.wrapping_add(1);
    vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt, 0x00, 0x04,
        (lane & 0xff) as u8, (lane >> 8) as u8, 0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x0d,
        0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, yy, 0x64, 0x4e, 0x65, 0x82, 0x62, src_bus,
        0x1a, 0x00, 0x00, 0x00, 0x00,
    ]
}

/// MOVE `1d …<lane_lo> <lane_hi> 00 00 …83 66 cd 03 <yy> 64 2b 65 82 4b <src> 4c <dst>` en
/// création de split. Comme [`build_matrix_slot_move_packet`] mais avec le **lane éditeur** à
/// l'offset 12-13 (au lieu de session+double) — byte-fidèle before_split.
fn build_matrix_move_lane(
    state: &mut HelixState,
    src_index: usize,
    dest_index: usize,
    lane: u16,
) -> Result<Vec<u8>, String> {
    let src_bus = kempline_index_to_slot_bus(src_index)
        .ok_or_else(|| format!("matrix move : bus source invalide pour index {src_index}"))?;
    let dst_bus = kempline_index_to_slot_bus(dest_index)
        .ok_or_else(|| format!("matrix move : bus destination invalide pour index {dest_index}"))?;
    let cnt = state.next_x80_cnt();
    let yy = state.live_write_yy;
    state.live_write_yy = yy.wrapping_add(1);
    Ok(vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt, 0x00, 0x04,
        (lane & 0xff) as u8, (lane >> 8) as u8, 0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x0d,
        0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, yy, 0x64, 0x2b, 0x65, 0x82, 0x4b, src_bus,
        0x4c, dst_bus, 0x00, 0x00, 0x00,
    ])
}

/// Déplace un bloc FX matrix (drag & drop HX Edit).
///
/// `create_split` = destination sur un path 2 VIDE (on crée le routage parallèle). Dans ce cas,
/// HX Edit envoie SELECT(source) puis MOVE, et **aucun** commit `19` (capture
/// `d&d_path1_to_path2_before_split.json`). Nos commits parasites y provoquaient un no-op device.
pub fn send_matrix_slot_move(
    state: &mut HelixState,
    src_index: usize,
    dest_index: usize,
    create_split: bool,
) -> Result<String, String> {
    let inter_path = (src_index >= 8) != (dest_index >= 8);
    let faithful_create_split = create_split && inter_path && create_split_faithful_enabled();

    // Création de split (path1 → path2 vide) : séquence COPIE CONFORME de la capture
    // `d&d_path1_to_path2_before_split.json`. Pour CHAQUE `1d` (SELECT puis MOVE) :
    // préambule `08 sub=10` (lane L) → `1d` (lane L) → ACK `08 sub=08` (lane L+0x11).
    // Le lane est `live_write_ctr` incrémenté de +0x11 par étape ; offset 14-15 = `00 00`.
    // AUCUN commit `19`, AUCUN prime_dump_ack.
    if faithful_create_split {
        let src_bus = kempline_index_to_slot_bus(src_index)
            .ok_or_else(|| format!("matrix move : bus source invalide pour index {src_index}"))?;
        let l0 = state.live_write_ctr;
        let l1 = l0.wrapping_add(0x11);
        let l2 = l1.wrapping_add(0x11);

        // Étape SELECT.
        let pre_sel = build_ed03_lane08(state, 0x10, l0);
        state.send(OutPacket::new(pre_sel));
        let sel = build_select_source_lane(state, src_bus, l0);
        state.send(OutPacket::new(sel));
        let ack_sel = build_ed03_lane08(state, 0x08, l1);
        state.send(OutPacket::with_delay(ack_sel, 8));

        // Étape MOVE.
        let pre_mv = build_ed03_lane08(state, 0x10, l1);
        state.send(OutPacket::new(pre_mv));
        let mv = build_matrix_move_lane(state, src_index, dest_index, l1)?;
        let mv_src = mv[34];
        let mv_dst = mv[36];
        state.send(OutPacket::new(mv));
        let ack_mv = build_ed03_lane08(state, 0x08, l2);
        state.send(OutPacket::with_delay(ack_mv, 8));

        state.live_write_ctr = l2;

        if let Some(dst_bus) = kempline_index_to_slot_bus(dest_index) {
            state.hw_active_slot_index = Some(dest_index);
            state.hw_active_slot_bus = Some(dst_bus);
            state.hw_active_slot_sequence = state.hw_active_slot_sequence.wrapping_add(1);
        }

        return Ok(format!(
            "create_split_faithful select_src bus {mv_src:#04x} | move_1d {src_index}->{dest_index} \
             bus {mv_src:#04x}->{mv_dst:#04x} | lane {l0:#06x}->{l1:#06x} | no_commit"
        ));
    }

    let mut lines: Vec<String> = Vec::new();

    let pkt = build_matrix_slot_move_packet(state, src_index, dest_index)?;
    let ack_lo = pkt[12];
    let ack_hi = pkt[13];
    let src_bus = pkt[34];
    let dst_bus = pkt[36];
    let post = build_post_1d_ack08(state, ack_lo, ack_hi);

    state.send(OutPacket::new(pkt));
    state.send(OutPacket::with_delay(post, 8));

    lines.push(format!(
        "move_1d {src_index}->{dest_index} bus {src_bus:#04x}->{dst_bus:#04x}"
    ));

    // Commits `19` SPLIT/MERGE : inter-path avec un split DÉJÀ existant (le cas création de split
    // est traité plus haut, byte-fidèle, et retourne avant ici).
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
    fn create_split_select_and_move_lane_shape() {
        // Capture HX Edit d&d_path1_to_path2_before_split : SELECT+MOVE portent le LANE éditeur
        // (live_write_ctr) à l'offset 12-13, offset 14-15 = 00, +0x11 entre les deux ;
        // yy séquentiel (offset 28).
        let mut s = test_state(); // live_write_ctr=0x4c00, live_write_yy=0x13
        let l0 = s.live_write_ctr;
        let sel = build_select_source_lane(&mut s, 0x03, l0);
        assert_eq!(sel.len(), 40);
        assert_eq!(sel[0], 0x1d);
        assert_eq!(&sel[12..16], &[(l0 & 0xff) as u8, (l0 >> 8) as u8, 0x00, 0x00], "lane offset12-13 + 00 00");
        assert_eq!(sel[27], 0x03, "cd03");
        assert_eq!(sel[28], 0x13, "yy = live_write_yy initial");
        assert_eq!(&sel[29..36], &[0x64, 0x4e, 0x65, 0x82, 0x62, 0x03, 0x1a]);

        let l1 = l0.wrapping_add(0x11);
        let mv = build_matrix_move_lane(&mut s, 1, 9, l1).unwrap();
        assert_eq!(&mv[12..16], &[(l1 & 0xff) as u8, (l1 >> 8) as u8, 0x00, 0x00], "lane +0x11 + 00 00");
        assert_eq!(mv[27], 0x03, "cd03");
        assert_eq!(mv[28], 0x14, "yy séquentiel = SELECT+1");
        assert_eq!(&mv[32..37], &[0x82, 0x4b, 0x02, 0x4c, 0x0c], "82 4b <src_bus> 4c <dst_bus>");

        // Préambule/ACK 08 : lane à l'offset 12-13, offset 14-15 = 00.
        let pre = build_ed03_lane08(&mut s, 0x10, l0);
        assert_eq!(pre.len(), 16);
        assert_eq!(pre[11], 0x10);
        assert_eq!(&pre[12..16], &[(l0 & 0xff) as u8, (l0 >> 8) as u8, 0x00, 0x00]);
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
