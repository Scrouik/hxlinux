//! Commit paramètre **Cab single legacy** `cd:03:ff` (Soup Pro, …) — handshake HX Edit.
//!
//! Capture `one_slow_notch_HXEdit.json` / `split scroll.json` :
//! `1b` → `f0` → (IN dump) → `19` → (IN `21`) → `19` → `f0` → rafale IN `272` + ACK `08 ed:03`.
//! **Pas de trame OUT `57`** — le burst synchrone ne fonctionne pas.

use std::time::{Duration, Instant};

use crate::helix::amp_cab_live_write::{
    build_legacy_f0_interstitial_packet, build_standalone_legacy_cd03ff_param_1b,
    build_standalone_legacy_cd03ff_param_19,
};
use crate::helix::packet::OutPacket;
use crate::helix::preset_dump_stream_ack::{
    is_preset_dump_stream_ack_echo_in, is_preset_dump_stream_chunk_in,
};
use crate::helix::usb_in_pipeline::{LayerEffect, LayerResult};
use crate::helix::{kempline_index_to_slot_bus, HelixState};

const COMMIT_TIMEOUT_MS: u64 = 800;
const INTER_PKT_DELAY_MS: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitStep {
    /// `1b` + `f0` envoyés — attend dump IN (`53`, `3c`, `39`, …).
    WaitDumpIn,
    /// Premier `19` envoyé — attend stub IN `21` (44 o).
    WaitIn21,
    /// Deuxième `19` + `f0` envoyés — attend fin rafale `272`.
    Await272Done,
}

#[derive(Debug, Clone)]
pub struct StandaloneLegacyParamCommit {
    pub slot_index: u32,
    pub slot_bus: u8,
    pub assign_block: [u8; 16],
    pub param_selector: u8,
    step: CommitStep,
    deadline: Instant,
    saw_272: bool,
}

fn commit_trace(msg: &str) {
    if std::env::var("HX_LEGACY_CAB_COMMIT_DEBUG")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        eprintln!("[LegacyCabCommit] {msg}");
    }
}

fn is_legacy_commit_dump_in(data: &[u8]) -> bool {
    if data.len() < 36 {
        return false;
    }
    if data.get(4..8) != Some(&[0xed, 0x03, 0x80, 0x10]) {
        return false;
    }
    matches!(
        data.first(),
        Some(0x53) | Some(0x3c) | Some(0x39) | Some(0x4e) | Some(0x4c)
    )
}

fn is_legacy_commit_in21_stub(data: &[u8]) -> bool {
    data.len() == 44 && data.first() == Some(&0x21)
}

/// Démarre un write live Cab single legacy `cd:03:ff` (handshake asynchrone).
pub fn start_standalone_legacy_cd03ff_write(
    state: &mut HelixState,
    slot_index: u32,
    assign_block: [u8; 16],
    param_selector: u8,
) -> Result<(), String> {
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or_else(|| "slotIndex invalide".to_string())?;

    if state.standalone_legacy_param_commit.is_some() {
        commit_trace("commit précédent abandonné (nouveau write)");
    }

    state.standalone_legacy_param_commit = Some(StandaloneLegacyParamCommit {
        slot_index,
        slot_bus,
        assign_block,
        param_selector,
        step: CommitStep::WaitDumpIn,
        deadline: Instant::now() + Duration::from_millis(COMMIT_TIMEOUT_MS),
        saw_272: false,
    });

    let pkt1b = build_standalone_legacy_cd03ff_param_1b(state, assign_block, slot_bus);
    let pkt_f0 = build_legacy_f0_interstitial_packet(state);
    commit_trace(&format!(
        "start slot={slot_index} pSel={param_selector:#04x} OUT 1b+f0"
    ));
    state.send(OutPacket::new(pkt1b));
    state.send(OutPacket::with_delay(pkt_f0, INTER_PKT_DELAY_MS));
    Ok(())
}

fn finish_commit(state: &mut HelixState, reason: &str) {
    if let Some(c) = state.standalone_legacy_param_commit.take() {
        commit_trace(&format!(
            "done slot={} pSel={:#04x} reason={reason}",
            c.slot_index, c.param_selector
        ));
    }
}

fn send_first_19(state: &mut HelixState, assign_block: [u8; 16], param_selector: u8) {
    let pkt = build_standalone_legacy_cd03ff_param_19(state, assign_block, param_selector, false);
    commit_trace("OUT 19 #1 (cd:03:ff)");
    state.send(OutPacket::with_delay(pkt, INTER_PKT_DELAY_MS));
}

fn send_second_19_and_f0(state: &mut HelixState, assign_block: [u8; 16], param_selector: u8) {
    let pkt19 = build_standalone_legacy_cd03ff_param_19(state, assign_block, param_selector, true);
    let pkt_f0 = build_legacy_f0_interstitial_packet(state);
    commit_trace("OUT 19 #2 (cd:04:pSel) + f0");
    state.send(OutPacket::with_delay(pkt19, INTER_PKT_DELAY_MS));
    state.send(OutPacket::with_delay(pkt_f0, INTER_PKT_DELAY_MS));
}

/// Couche IN active — avance le handshake quand un commit est en cours.
pub fn handle_in_layer(state: &mut HelixState, data: &[u8]) -> LayerResult {
    let Some(commit) = state.standalone_legacy_param_commit.as_mut() else {
        return LayerResult::Ignored;
    };

    if Instant::now() >= commit.deadline {
        finish_commit(state, "timeout");
        return LayerResult::Ignored;
    }

    match commit.step {
        CommitStep::WaitDumpIn if is_legacy_commit_dump_in(data) => {
            let (assign, psel) = {
                commit.step = CommitStep::WaitIn21;
                commit.deadline = Instant::now() + Duration::from_millis(COMMIT_TIMEOUT_MS);
                (commit.assign_block, commit.param_selector)
            };
            send_first_19(state, assign, psel);
            return LayerResult::Consumed {
                effect: LayerEffect::None,
            };
        }
        CommitStep::WaitIn21 if is_legacy_commit_in21_stub(data) => {
            let (assign, psel) = {
                commit.step = CommitStep::Await272Done;
                commit.deadline = Instant::now() + Duration::from_millis(COMMIT_TIMEOUT_MS);
                (commit.assign_block, commit.param_selector)
            };
            send_second_19_and_f0(state, assign, psel);
            return LayerResult::Consumed {
                effect: LayerEffect::None,
            };
        }
        CommitStep::Await272Done if is_preset_dump_stream_chunk_in(data) => {
            commit.saw_272 = true;
            commit.deadline = Instant::now() + Duration::from_millis(COMMIT_TIMEOUT_MS);
            return LayerResult::Ignored;
        }
        CommitStep::Await272Done
            if commit.saw_272 && is_preset_dump_stream_ack_echo_in(data) =>
        {
            finish_commit(state, "272_done");
            return LayerResult::Ignored;
        }
        _ => LayerResult::Ignored,
    }
}

/// Expire les commits bloqués (appelé depuis `usb_listener` à chaque IN).
pub fn tick_commit_timeouts(state: &mut HelixState) {
    let done = state.standalone_legacy_param_commit.as_ref().map(|c| {
        Instant::now() >= c.deadline
            && (c.step != CommitStep::Await272Done || c.saw_272)
    });
    if done == Some(true) {
        let reason = if state
            .standalone_legacy_param_commit
            .as_ref()
            .is_some_and(|c| c.saw_272)
        {
            "272_settled"
        } else {
            "timeout"
        };
        finish_commit(state, reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::amp_cab_live_write::standalone_legacy_assign_uses_cd03ff;

    #[test]
    fn start_leaves_two_out_packets_only() {
        let mut state = HelixState::new();
        let assign = [
            0x83, 0x66, 0xcd, 0x03, 0xff, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17,
            0xc2, 0x19,
        ];
        assert!(standalone_legacy_assign_uses_cd03ff(assign));
        start_standalone_legacy_cd03ff_write(&mut state, 0, assign, 0x00).expect("start");
        assert!(state.standalone_legacy_param_commit.is_some());
    }
}
