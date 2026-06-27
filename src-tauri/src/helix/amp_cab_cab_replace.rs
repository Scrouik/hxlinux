//! Replace **cab seul** sur slot Amp+Cab occupé.
//!
//! Legacy (`0x23` / `0x25`) et IR (`0x27`) : **`1d` focus cab → `ed:08` → bulk**
//! (capture `ampcab_legacy_change_cab.json` : bulk lane `cd:03`, pas `cd:08`).
//! Legacy replace : le bulk catalogue a `cd:07` (assign AddToEmpty) ; on patch `cd:07`→`cd:03`
//! avant envoi (capture `ampcab_legacy_change_cab.json` #4249).

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

/// Bulk compact legacy Amp+Cab (capture `amp_cab legacy bass.json` frame 1357, head `0x23`).
pub fn amp_cab_replace_bulk_implies_legacy(bulk: &[u8]) -> bool {
    bulk.first() == Some(&0x23)
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

/// Replace cab legacy : lane `cd:03` sur le fil (pas `cd:07` de l’assign initial).
fn reframe_legacy_replace_cd07_to_cd03(bulk: &mut [u8]) {
    for i in 0..bulk.len().saturating_sub(4) {
        if bulk[i] == 0x83
            && bulk[i + 1] == 0x66
            && bulk[i + 2] == 0xcd
            && bulk[i + 3] == 0x07
        {
            bulk[i + 3] = 0x03;
            return;
        }
    }
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

/// Focus cab (`1d`) → ed:08 → bulk replace (`0x27` / `0x23` / `0x25`).
pub fn execute_amp_cab_cab_replace(
    helix_arc: Arc<Mutex<HelixState>>,
    slot_index: u32,
    slot_bus: u8,
    usb_bulk: &[u8],
    legacy: bool,
) -> Result<String, String> {
    let delay_ed08 = env_delay_ms("HX_CAB2_DELAY_ED08_MS", 93);
    let delay_bulk = if legacy {
        env_delay_ms("HX_AMP_CAB_LEGACY_REPLACE_BULK_MS", 1100)
    } else {
        env_delay_ms("HX_CAB2_DELAY_BULK_MS", 400)
    };

    let l = {
        let mut s = helix_arc.lock().unwrap();
        let l = s.live_write_ctr;
        let mut focus = build_amp_cab_ir_cab_focus_packet(&mut s, slot_bus);
        force_ed03_ctr(&mut focus, l);
        s.live_write_yy = s.live_write_yy.wrapping_add(1);
        s.slot_model_lane_seq = Some(s.live_write_yy);
        s.amp_cab_cab_focus_sent_for_slot = Some(slot_index);
        init_trace::trace_fmt(format_args!(
            "amp_cab_cab_replace FIRE slot={} bus={:#04x} legacy={} L={:#06x}",
            slot_index,
            slot_bus,
            legacy,
            l,
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
        if legacy {
            reframe_legacy_replace_cd07_to_cd03(&mut bulk_in);
        } else {
            prepare_amp_cab_cab_replace_bulk(&mut bulk_in);
        }

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
        crate::helix::amp_cab_live_write::record_amp_cab_cab_replace_session(&mut s, slot_index, &bulk);
        format!("bulk head={head:#04x} len={} {hx}", bulk.len())
    };

    init_trace::trace_fmt(format_args!(
        "amp_cab_cab_replace OK legacy={legacy} L={l:#06x} | {line}"
    ));
    Ok(format!(
        "amp+cab cab replace OK (legacy={legacy}) — {line}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{amp_cab_replace_bulk_implies_legacy, reframe_legacy_replace_cd07_to_cd03};

    #[test]
    fn legacy_replace_reframes_cd07_to_cd03() {
        let mut bulk = vec![
            0x23, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x39, 0x00, 0x04, 0xbd, 0x6c,
            0x02, 0x00, 0x01, 0x00, 0x06, 0x00, 0x13, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x07,
            0xfb, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17, 0xc3, 0x19, 0x2c, 0x1a,
            0x47, 0x00,
        ];
        reframe_legacy_replace_cd07_to_cd03(&mut bulk);
        assert_eq!(&bulk[24..28], &[0x83, 0x66, 0xcd, 0x03]);
    }

    #[test]
    fn legacy_bulk_head_23_implies_legacy_path() {
        assert!(amp_cab_replace_bulk_implies_legacy(&[0x23, 0x00]));
        assert!(!amp_cab_replace_bulk_implies_legacy(&[0x25, 0x00]));
        assert!(!amp_cab_replace_bulk_implies_legacy(&[0x27, 0x00]));
        assert!(!amp_cab_replace_bulk_implies_legacy(&[]));
    }
}
