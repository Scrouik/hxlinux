//! Fire agnostique replace **Cab 2** — lane `live_write_ctr` (`focus=L` → `ed:08`/`bulk`=L+0x11).
//!
//! Têtes bulk acceptées : `0x27` (IR), `0x23` (legacy 1 o), `0x25` (legacy cd02xx cab2).

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::helix::cab_dual::ir::build_cab_dual_cab2_focus_packet_with_lane;
use crate::helix::cab_dual::ir::Cab2FocusLane;
use crate::helix::cab_dual::legacy::wire::{
    accepted_cab2_replace_heads, prepare_cab2_replace_bulk, CAB2_REPLACE_HEAD_LEGACY,
    CAB2_REPLACE_HEAD_IR, DUAL_PARENT_REPLACE_HEAD,
};
use crate::helix::edit_slot_model::{build_slot_model_probe_packets, SlotModelProbeOp};
use crate::helix::init_trace;
use crate::helix::packet::OutPacket;
use crate::helix::HelixState;

fn env_delay_ms(var: &str, default_ms: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default_ms)
}

/// Patche les octets 12-13 (ctr LE) et 14 (=0) d'un paquet `80:10:ed:03`.
fn force_ed03_ctr(pkt: &mut [u8], ctr: u16) {
    if pkt.len() > 14 {
        pkt[12] = (ctr & 0xff) as u8;
        pkt[13] = ((ctr >> 8) & 0xff) as u8;
        pkt[14] = 0x00;
    }
}

/// Construit un court `08 … 80:10:ed:03` (ed:08), ctr posé sur les octets 12-13.
fn build_ed08_short(state: &mut HelixState, ctr: u16) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x08,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x00,
    ]
}

fn find_cab2_replace_bulk(packs: Vec<Vec<u8>>) -> Result<Vec<u8>, String> {
    let heads = accepted_cab2_replace_heads();
    packs
        .into_iter()
        .find(|p| p.first().is_some_and(|h| heads.contains(h)))
        .ok_or_else(|| {
            format!(
                "cab dual cab2 replace : bulk introuvable (attendu head {CAB2_REPLACE_HEAD_IR:#04x}, \
                 {CAB2_REPLACE_HEAD_LEGACY:#04x} ou {DUAL_PARENT_REPLACE_HEAD:#04x})"
            )
        })
}

/// Focus Cab 2 → ed:08 → bulk (`0x27` IR ou `0x23` legacy), lane modèle `live_write_ctr`.
pub fn execute_cab_dual_cab2_replace_fire(
    helix_arc: Arc<Mutex<HelixState>>,
    slot_index: u32,
    slot_bus: u8,
    usb_bulk: &[u8],
) -> Result<String, String> {
    let delay_ed08 = env_delay_ms("HX_CAB2_DELAY_ED08_MS", 93);
    let delay_bulk = env_delay_ms("HX_CAB2_DELAY_BULK_MS", 400);

    let l = {
        let mut s = helix_arc.lock().unwrap();
        let l = s.live_write_ctr;

        let mut focus =
            build_cab_dual_cab2_focus_packet_with_lane(&mut s, slot_bus, Cab2FocusLane::LiveWrite, 0x04);
        force_ed03_ctr(&mut focus, l);

        s.live_write_yy = s.live_write_yy.wrapping_add(1);
        s.slot_model_lane_seq = Some(s.live_write_yy);
        s.cab_dual_cab2_focus_sent_for_slot = Some(slot_index);

        init_trace::trace_fmt(format_args!(
            "cab_dual_cab2_replace FIRE slot={} bus={:#04x} L={:#06x} (focus=L, ed08/bulk={:#06x})",
            slot_index,
            slot_bus,
            l,
            l.wrapping_add(0x11)
        ));
        s.send(OutPacket::new(focus));
        l
    };
    thread::sleep(Duration::from_millis(delay_ed08));

    let ctr_model = l.wrapping_add(0x11);
    {
        let mut s = helix_arc.lock().unwrap();
        let ed08 = build_ed08_short(&mut s, ctr_model);
        s.send(OutPacket::new(ed08));
        s.live_write_ctr = ctr_model;
    }
    thread::sleep(Duration::from_millis(delay_bulk));

    let line = {
        let mut s = helix_arc.lock().unwrap();
        s.live_write_ctr = ctr_model;

        let mut bulk_in = usb_bulk.to_vec();
        prepare_cab2_replace_bulk(&mut bulk_in);

        let packs = build_slot_model_probe_packets(
            &mut s,
            SlotModelProbeOp::ReplaceOccupied,
            slot_index as usize,
            slot_bus,
            None,
            Some(&bulk_in),
            true,
        );
        let mut bulk = find_cab2_replace_bulk(packs)?;

        if bulk.len() > 15 {
            bulk[14] = 0x00;
            bulk[15] = 0x00;
        }

        let head = bulk.first().copied().unwrap_or(0);
        let hx: String = bulk
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");
        s.send(OutPacket::new(bulk.clone()));
        format!("bulk head={head:#04x} len={} {hx}", bulk.len())
    };

    init_trace::trace_fmt(format_args!(
        "cab_dual_cab2_replace OK L={l:#06x} model={ctr_model:#06x} | {line}"
    ));
    Ok(format!(
        "cab dual cab2 replace OK (lane live_write L={l:#06x}, ed08/bulk={ctr_model:#06x}) — {line}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ed08_short_carries_ctr_on_bytes_12_13() {
        let mut s = HelixState::new();
        let ed08 = build_ed08_short(&mut s, 0x6e8e);
        assert_eq!(ed08[0], 0x08);
        assert_eq!(&ed08[4..8], &[0x80, 0x10, 0xed, 0x03]);
        assert_eq!(ed08[11], 0x08);
        assert_eq!(ed08[12], 0x8e);
        assert_eq!(ed08[13], 0x6e);
    }

    #[test]
    fn force_ctr_sets_bytes_12_14() {
        let mut pkt = vec![0u8; 40];
        force_ed03_ctr(&mut pkt, 0x6e7d);
        assert_eq!(pkt[12], 0x7d);
        assert_eq!(pkt[13], 0x6e);
        assert_eq!(pkt[14], 0x00);
    }

    #[test]
    fn lane_model_focus_l_ed08_bulk_l_plus_0x11() {
        let l = 0x6e7du16;
        assert_eq!(l.wrapping_add(0x11), 0x6e8e);
    }
}
