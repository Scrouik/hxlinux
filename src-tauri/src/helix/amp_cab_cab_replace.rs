//! Replace **cab seul** sur slot Amp+Cab occupé.
//!
//! - **Legacy** (`0x23` / `0x25`) : même cinématique que l’assign initial — `ef → f0 → bulk`
//!   (capture `amp_cab legacy bass.json` frame 1357 ; octets 14–15 = `02 00` conservés).
//! - **IR** (`0x27`) : `focus → ed:08 → bulk` (lane `live_write`, comme Cab dual).

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::helix::amp_cab_live_write::build_amp_cab_ir_cab_focus_packet;
use crate::helix::cab_dual::legacy::wire::reframe_cd0a_to_cd04;
use crate::helix::ed03_lane::{build_ed08_short, force_ed03_ctr};
use crate::helix::edit_slot_model::{
    accepted_amp_cab_cab_replace_heads, build_slot_model_probe_packets, SlotModelProbeOp,
};
use crate::helix::init_trace;
use crate::helix::packet::OutPacket;
use crate::helix::HelixState;

fn env_delay_ms(var: &str, default_ms: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default_ms)
}

fn find_amp_cab_cab_replace_bulk(packs: Vec<Vec<u8>>) -> Result<Vec<u8>, String> {
    let heads = accepted_amp_cab_cab_replace_heads();
    packs
        .into_iter()
        .find(|p| p.first().is_some_and(|h| heads.contains(h)))
        .ok_or_else(|| {
            format!(
                "amp+cab cab replace : bulk introuvable (attendu head {:?})",
                heads
            )
        })
}

fn prepare_amp_cab_cab_replace_bulk(bulk: &mut [u8]) {
    reframe_cd0a_to_cd04(bulk);
}

/// Ne pas écraser les octets 14–15 sur les bulks assign Amp+Cab (`02 00` dans toutes les captures 23/25/27).
fn finalize_amp_cab_replace_bulk(bulk: &mut [u8]) {
    let head = bulk.first().copied().unwrap_or(0);
    if matches!(head, 0x23 | 0x25 | 0x27) {
        return;
    }
    if bulk.len() > 15 {
        bulk[14] = 0x00;
        bulk[15] = 0x00;
    }
}

fn send_probe_packet_batch(s: &mut HelixState, packs: &[Vec<u8>]) {
    for (i, p) in packs.iter().enumerate() {
        if i > 0 {
            thread::sleep(Duration::from_millis(8));
        }
        s.send(OutPacket::new(p.clone()));
    }
}

fn execute_amp_cab_cab_replace_legacy_ef_f0_bulk(
    helix_arc: Arc<Mutex<HelixState>>,
    slot_index: u32,
    slot_bus: u8,
    usb_bulk: &[u8],
) -> Result<String, String> {
    let line = {
        let mut s = helix_arc.lock().unwrap();
        s.amp_cab_cab_focus_sent_for_slot = Some(slot_index);

        let mut bulk_in = usb_bulk.to_vec();
        prepare_amp_cab_cab_replace_bulk(&mut bulk_in);

        let packs = build_slot_model_probe_packets(
            &mut s,
            SlotModelProbeOp::ReplaceOccupied,
            slot_index as usize,
            slot_bus,
            None,
            Some(&bulk_in),
            false,
        );
        let mut bulk = find_amp_cab_cab_replace_bulk(packs.clone())?;
        finalize_amp_cab_replace_bulk(&mut bulk);

        init_trace::trace_fmt(format_args!(
            "amp_cab_cab_replace FIRE legacy ef/f0/bulk slot={} bus={:#04x}",
            slot_index, slot_bus
        ));
        send_probe_packet_batch(&mut s, &packs);

        let head = bulk.first().copied().unwrap_or(0);
        let hx: String = bulk
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");
        format!("bulk head={head:#04x} len={} {hx}", bulk.len())
    };

    init_trace::trace_fmt(format_args!("amp_cab_cab_replace OK legacy=true ef/f0 | {line}"));
    Ok(format!("amp+cab cab replace OK (legacy=true, ef/f0/bulk) — {line}"))
}

/// Focus cab (IR `1d` / legacy `1b`) → ed:08 → bulk replace (`0x27` / `0x23` / `0x25`).
pub fn execute_amp_cab_cab_replace(
    helix_arc: Arc<Mutex<HelixState>>,
    slot_index: u32,
    slot_bus: u8,
    usb_bulk: &[u8],
    legacy: bool,
) -> Result<String, String> {
    if legacy {
        return execute_amp_cab_cab_replace_legacy_ef_f0_bulk(
            helix_arc,
            slot_index,
            slot_bus,
            usb_bulk,
        );
    }

    let delay_ed08 = env_delay_ms("HX_CAB2_DELAY_ED08_MS", 93);
    let delay_bulk = env_delay_ms("HX_CAB2_DELAY_BULK_MS", 400);

    let l = {
        let mut s = helix_arc.lock().unwrap();
        let l = s.live_write_ctr;
        let mut focus = build_amp_cab_ir_cab_focus_packet(&mut s, slot_bus);
        force_ed03_ctr(&mut focus, l);
        s.live_write_yy = s.live_write_yy.wrapping_add(1);
        s.slot_model_lane_seq = Some(s.live_write_yy);
        s.amp_cab_cab_focus_sent_for_slot = Some(slot_index);
        init_trace::trace_fmt(format_args!(
            "amp_cab_cab_replace FIRE slot={} bus={:#04x} legacy=false L={:#06x} (focus=L, ed08/bulk={:#06x})",
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
        prepare_amp_cab_cab_replace_bulk(&mut bulk_in);

        let packs = build_slot_model_probe_packets(
            &mut s,
            SlotModelProbeOp::ReplaceOccupied,
            slot_index as usize,
            slot_bus,
            None,
            Some(&bulk_in),
            true,
        );
        let mut bulk = find_amp_cab_cab_replace_bulk(packs)?;
        finalize_amp_cab_replace_bulk(&mut bulk);

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
        "amp_cab_cab_replace OK legacy=false L={l:#06x} model={ctr_model:#06x} | {line}"
    ));
    Ok(format!(
        "amp+cab cab replace OK (legacy=false, L={l:#06x}, ed08/bulk={ctr_model:#06x}) — {line}"
    ))
}
