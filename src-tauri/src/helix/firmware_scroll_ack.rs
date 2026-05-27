//! ACK scroll firmware — IN `1d` / `1f` 40 o → OUT `f0:03` sub=`08` (lane dédiée).
//!
//! Couche **minimale** : pas de pull modèle, pas d’UI. Calquée sur
//! `captures/usb-wireshark/stomp_running_start_hxedit.json` (idle post-connect).
//! Le pull (`slot_model_hw_pull`) pourra s’appuyer sur cette lane plus tard sans la remplacer.

use crate::helix::{HelixState, SessionPhase};
use crate::helix::init_trace;
use crate::helix::packet::OutPacket;
use crate::helix::usb_in_pipeline::{LayerEffect, LayerResult};

/// Lane initiale après bootstrap connect (`09:10` sur le fil).
pub const SCROLL_LANE_BOOT: u16 = 0x1009;

/// Pas d’incrément lane (octets 12–13 LE du OUT `f0:03` sub=`08`) selon captures HX Edit.
pub(crate) fn scroll_ack_step(prev: Option<u8>, head: u8, skip_inc_once: bool) -> (u16, bool) {
    if skip_inc_once && prev == Some(0x1f) && head == 0x1d {
        return (0, false);
    }
    let step = match (prev, head) {
        // Premier `1d` après bootstrap wire `09:10` : premier ACK host ≈ `48:10` (+0x3f).
        (None, 0x1d) => 0x003f,
        (None, 0x1f) => 0x0017,
        (Some(0x1d), 0x1d) => 0x0015,
        (Some(0x1d), 0x1f) => 0x0017,
        (Some(0x1f), 0x1d) => 0x002e,
        (Some(0x1f), 0x1f) => 0x0017,
        (Some(0x1f), 0x21) => 0x0015,
        (Some(0x21), 0x1d) => 0x002e,
        (Some(0x21), 0x1f) => 0x0017,
        (Some(0x21), 0x21) => 0x0015,
        _ => 0x0015,
    };
    (step, skip_inc_once)
}

impl HelixState {
    pub fn firmware_scroll_lane_double(&self) -> [u8; 2] {
        let lo = (self.firmware_scroll_ack_ctr & 0xFF) as u8;
        let hi = ((self.firmware_scroll_ack_ctr >> 8) & 0xFF) as u8;
        [lo, hi]
    }

    /// Avance la lane **puis** renvoie le double à émettre (aligné capture : premier ACK après bootstrap = `48:10`).
    pub fn next_firmware_scroll_ack_out(&mut self, head: u8) -> [u8; 2] {
        let (step, skip_next) = scroll_ack_step(
            self.firmware_scroll_ack_prev,
            head,
            self.firmware_scroll_skip_inc_once,
        );
        self.firmware_scroll_skip_inc_once = skip_next;
        self.firmware_scroll_ack_ctr = self.firmware_scroll_ack_ctr.wrapping_add(step);
        self.firmware_scroll_ack_prev = Some(head);
        self.firmware_scroll_lane_double()
    }

    /// Après envoi du bootstrap connect `09:10` (valeur fil = lane courante, pas encore de `1d` vu).
    pub fn note_firmware_scroll_bootstrap_sent(&mut self) {
        self.firmware_scroll_ack_ctr = SCROLL_LANE_BOOT;
        self.firmware_scroll_ack_prev = None;
        self.firmware_scroll_skip_inc_once = false;
    }
}

/// Couche active « fond » : IN `1d` / `1f` 40 o → lane scroll + ACK `f0:03` sub=`08`.
pub fn handle_in_layer(state: &mut HelixState, data: &[u8]) -> LayerResult {
    let head = data.first().copied().unwrap_or(0);
    if head != 0x1d && head != 0x1f || data.len() != 40 {
        return LayerResult::Ignored;
    }
    if data.get(4..8) != Some(&[0xf0, 0x03, 0x02, 0x10]) {
        return LayerResult::Ignored;
    }
    if matches!(state.session_phase(), SessionPhase::Bootstrapping) {
        init_trace::trace_1d_ack_decision(false, "bootstrapping");
        return LayerResult::Ignored;
    }
    if state.preset_usb_read_in_progress() {
        init_trace::trace_1d_ack_decision(false, "preset_usb_read");
        return LayerResult::Ignored;
    }
    if head == 0x1d && !state.should_ack_firmware_1d_notify() {
        init_trace::trace_1d_ack_decision(false, "suppress_1d");
        return LayerResult::Ignored;
    }
    let cnt = state.next_x2_cnt();
    let double = state.next_firmware_scroll_ack_out(head);
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08,
        double[0], double[1], 0x00, 0x00,
    ]));
    init_trace::trace_1d_ack_decision(true, if head == 0x1f { "1f" } else { "1d" });
    LayerResult::Consumed {
        effect: LayerEffect::ScrollLaneAndAck,
    }
}

/// ACK immédiat sur notif scroll / firmware (`1d` / `1f`). Retourne `true` si head == `1f`.
#[allow(dead_code)]
pub fn ack_firmware_scroll_in(state: &mut HelixState, data: &[u8]) -> bool {
    matches!(
        handle_in_layer(state, data),
        LayerResult::Consumed { .. }
    ) && data.first() == Some(&0x1f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_1d_after_bootstrap_advances_to_1048() {
        let mut state = HelixState::new();
        state.note_firmware_scroll_bootstrap_sent();
        let d = state.next_firmware_scroll_ack_out(0x1d);
        assert_eq!(d, [0x48, 0x10]);
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1048);
    }

    #[test]
    fn scroll_ack_step_1d_to_1d_is_0x15() {
        assert_eq!(scroll_ack_step(Some(0x1d), 0x1d, false).0, 0x0015);
    }

    #[test]
    fn scroll_ack_step_none_1d_is_0x3f() {
        assert_eq!(scroll_ack_step(None, 0x1d, false).0, 0x003f);
    }
}
