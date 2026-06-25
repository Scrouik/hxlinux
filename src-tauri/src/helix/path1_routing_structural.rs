//! Insertion / suppression Split + Merge Path 1 (activation branche path 2).
//! Captures : `delay lecacy.json` (insert), `wah mono.json` (remove).

use crate::helix::path1_split_live_write::send_path1_split_type;
use crate::helix::packet::OutPacket;
use crate::helix::HelixState;

const SPLIT_SLOT_BUS: u8 = 0x0a;
const MERGE_SLOT_BUS: u8 = 0x13;

const TEMPLATE_SPLIT_INSERT_PRELUDE_25: [u8; 48] = [
    0x25, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x82, 0x00, 0x04, 0xe8, 0x74, 0x00,
    0x00, 0x01, 0x00, 0x06, 0x00, 0x15, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x05, 0x09, 0x64,
    0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17, 0xc2, 0x19, 0xcd, 0x01, 0x9b, 0x1a, 0xff,
    0x00, 0x00, 0x00,
];

const TEMPLATE_SPLIT_INSERT_1B: [u8; 36] = [
    0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x83, 0x00, 0x0c, 0x41, 0x75, 0x00,
    0x00, 0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x05, 0x0a, 0x64,
    0x21, 0x65, 0x81, 0x66, 0x08, 0x00,
];

const TEMPLATE_MERGE_INSERT_25: [u8; 48] = [
    0x25, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x95, 0x00, 0x04, 0xfc, 0x77, 0x00,
    0x00, 0x01, 0x00, 0x06, 0x00, 0x15, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x05, 0x13, 0x64,
    0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17, 0xc2, 0x19, 0xcd, 0x01, 0x97, 0x1a, 0xff,
    0x00, 0x00, 0x00,
];

const TEMPLATE_STRUCTURAL_REMOVE_19: [u8; 36] = [
    0x19, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x08, 0x00, 0x0c, 0x63, 0x59, 0x01,
    0x00, 0x01, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x06, 0x0a, 0x64,
    0x16, 0x65, 0xc0, 0x00, 0x00, 0x00,
];

const STRUCTURAL_REMOVE_SLOT_BUS_OFFSET: usize = 28;
const STRUCTURAL_INSERT_SLOT_BUS_OFFSET: usize = 32;

fn patch_live_write_counters(pkt: &mut [u8], state: &mut HelixState, ctr_step: u16) {
    pkt[9] = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    pkt[12] = (ctr & 0xff) as u8;
    pkt[13] = ((ctr >> 8) & 0xff) as u8;
    if pkt.len() > 28 {
        pkt[28] = state.live_write_yy;
    }
    state.live_write_ctr = ctr.wrapping_add(ctr_step);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
}

fn build_post_ack08(state: &mut HelixState, ctr_lo: u8, ctr_hi: u8) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x08, ctr_lo, ctr_hi,
        0x00, 0x00,
    ]
}

fn patch_structural_slot_bus(pkt: &mut [u8], slot_bus: u8, offset: usize) {
    if pkt.len() > offset {
        pkt[offset] = slot_bus;
    }
}

fn focus_special_slot(state: &mut HelixState, slot_bus: u8) {
    if state.hw_active_slot_bus == Some(slot_bus) {
        return;
    }
    let focus =
        crate::helix::path1_io_live_write::build_special_slot_focus_packet(state, slot_bus);
    state.send(OutPacket::new(focus));
    state.hw_active_slot_bus = Some(slot_bus);
    state.hw_active_slot_index = None;
}

pub fn ensure_path2_dual_routing(state: &mut HelixState) -> Result<String, String> {
    let mut lines: Vec<String> = Vec::new();

    let mut prelude = TEMPLATE_SPLIT_INSERT_PRELUDE_25.to_vec();
    patch_live_write_counters(&mut prelude, state, 0x11);
    let pl_lo = prelude[12];
    let pl_hi = prelude[13];
    let pl_ack = build_post_ack08(state, pl_lo, pl_hi);
    state.send(OutPacket::new(prelude));
    state.send(OutPacket::with_delay(pl_ack, 8));
    lines.push("split_insert_prelude_25".to_string());

    let mut split_ins = TEMPLATE_SPLIT_INSERT_1B.to_vec();
    patch_live_write_counters(&mut split_ins, state, 0x11);
    patch_structural_slot_bus(&mut split_ins, SPLIT_SLOT_BUS, STRUCTURAL_INSERT_SLOT_BUS_OFFSET);
    let si_lo = split_ins[12];
    let si_hi = split_ins[13];
    let si_ack = build_post_ack08(state, si_lo, si_hi);
    state.send(OutPacket::with_delay(split_ins, 8));
    state.send(OutPacket::with_delay(si_ack, 8));
    lines.push("split_insert_1b".to_string());

    let split_type = send_path1_split_type(state, "HelixStomp_Split_Y")?;
    lines.push(format!("split_type_default: {split_type}"));

    let mut merge_ins = TEMPLATE_MERGE_INSERT_25.to_vec();
    patch_live_write_counters(&mut merge_ins, state, 0x11);
    patch_structural_slot_bus(&mut merge_ins, MERGE_SLOT_BUS, STRUCTURAL_INSERT_SLOT_BUS_OFFSET);
    focus_special_slot(state, MERGE_SLOT_BUS);
    let mg_lo = merge_ins[12];
    let mg_hi = merge_ins[13];
    let mg_ack = build_post_ack08(state, mg_lo, mg_hi);
    state.send(OutPacket::with_delay(merge_ins, 8));
    state.send(OutPacket::with_delay(mg_ack, 8));
    lines.push("merge_insert_25".to_string());

    Ok(lines.join(" | "))
}

pub fn teardown_path2_dual_routing(state: &mut HelixState) -> Result<String, String> {
    let mut lines: Vec<String> = Vec::new();

    for slot_bus in [MERGE_SLOT_BUS, SPLIT_SLOT_BUS] {
        focus_special_slot(state, slot_bus);
        let mut rem = TEMPLATE_STRUCTURAL_REMOVE_19.to_vec();
        patch_live_write_counters(&mut rem, state, 0x11);
        patch_structural_slot_bus(&mut rem, slot_bus, STRUCTURAL_REMOVE_SLOT_BUS_OFFSET);
        let r_lo = rem[12];
        let r_hi = rem[13];
        let r_ack = build_post_ack08(state, r_lo, r_hi);
        state.send(OutPacket::with_delay(rem, 8));
        state.send(OutPacket::with_delay(r_ack, 8));
        lines.push(format!("remove_19 slot_bus={slot_bus:#04x}"));
    }

    state.path1_split_type_wire = None;
    Ok(lines.join(" | "))
}
