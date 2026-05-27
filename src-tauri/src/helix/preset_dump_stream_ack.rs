//! ACK host des trames IN **flux preset / slot** (`08:01:ed:03:80:10`, sub=`04`, souvent 272 o).
//!
//! HX Edit en scroll modèle HW acquitte **chaque** chunk (~11 par pull) avec
//! `80:10:ed:03` sub=`08` et la lane [`HelixState::preset_dump_ack_ctr`].
//! Sans ces ACK, le Stomp arrête d’émettre (captures `3_scroll_HXLinux.json` : 71 IN, 22 ACK).
//!
//! **Test HW (session)** : on garde l’octet 12 fixe à [`HelixState::request_preset_session_id`]
//! (`f4:1d` → `f4:1e` → …) ; seul l’octet 13 s’incrémente (+`0x0100`). Si le Stomp accepte
//! plusieurs scrolls sans freeze, la session n’est pas contrainte côté firmware.

use crate::helix::{HelixState, packet::OutPacket};
use crate::helix::usb_in_pipeline::{LayerEffect, LayerResult};

/// Trame IN bulk du flux dump (preset ou état slot pendant scroll HW).
pub fn is_preset_dump_stream_chunk_in(data: &[u8]) -> bool {
    if data.len() <= 16 {
        return false;
    }
    if data.get(0..4) != Some(&[0x08, 0x01, 0x00, 0x18]) {
        return false;
    }
    if data.get(4..8) != Some(&[0xed, 0x03, 0x80, 0x10]) {
        return false;
    }
    if data.get(11) != Some(&0x04) {
        return false;
    }
    // FDT fin de transfert RequestPreset (32 o, `a1` @16) — autre gabarit d’ACK (3 octets session).
    if data.len() == 32 && data.get(16) == Some(&0xa1) {
        return false;
    }
    true
}

/// Couche active « ACK chunks 272 » (`preset_dump_ack_ctr`).
pub fn handle_in_layer(state: &mut HelixState, data: &[u8]) -> LayerResult {
    if !is_preset_dump_stream_chunk_in(data) {
        return LayerResult::Ignored;
    }
    if state.init_usb_settle_active() || state.preset_usb_read_in_progress() {
        return LayerResult::Ignored;
    }
    let cnt = state.next_x80_cnt();
    let double = state.next_preset_dump_ack_double();
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt, 0x00, 0x08,
        double[0], double[1], 0x00, 0x00,
    ]));
    if preset_dump_stream_ack_debug_enabled() {
        eprintln!(
            "[PresetDumpStreamAck] IN len={} → OUT ed:03 sub=08 cnt={cnt:#04x} double={:02x}:{:02x}",
            data.len(),
            double[0],
            double[1]
        );
    }
    LayerResult::Consumed {
        effect: LayerEffect::PresetDumpLaneAndAck,
    }
}

fn preset_dump_stream_ack_debug_enabled() -> bool {
    std::env::var_os("HX_PRESET_DUMP_STREAM_ACK_DEBUG").is_some_and(|v| {
        let s = v.to_string_lossy();
        !s.is_empty() && s != "0" && !s.eq_ignore_ascii_case("false")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::HelixState;

    fn sample_272() -> Vec<u8> {
        let mut b = vec![0u8; 272];
        b[0] = 0x08;
        b[1] = 0x01;
        b[3] = 0x18;
        b[4] = 0xed;
        b[5] = 0x03;
        b[6] = 0x80;
        b[7] = 0x10;
        b[11] = 0x04;
        b
    }

    #[test]
    fn detects_stream_chunk() {
        assert!(is_preset_dump_stream_chunk_in(&sample_272()));
        assert!(!is_preset_dump_stream_chunk_in(&[0x53; 92]));
    }

    #[test]
    fn ack_increments_lane_hi_byte() {
        let mut state = HelixState::new();
        assert_eq!(state.preset_dump_ack_ctr, 0x1df4);
        assert!(matches!(
            handle_in_layer(&mut state, &sample_272()),
            LayerResult::Consumed { .. }
        ));
        assert_eq!(state.preset_dump_ack_double(), [0xf4, 0x1e]);
        assert_eq!(state.preset_dump_ack_ctr, 0x1ef4);
    }

    #[test]
    fn many_acks_keep_session_f4() {
        let mut state = HelixState::new();
        for i in 0..25 {
            let [s, c] = state.preset_dump_ack_double();
            assert_eq!(s, 0xf4, "ACK #{i} session");
            assert_eq!(c, 0x1d + i, "ACK #{i} counter");
            let _ = handle_in_layer(&mut state, &sample_272());
        }
    }

    #[test]
    fn skips_during_request_preset() {
        let mut state = HelixState::new();
        state.set_preset_usb_read_modes_active(true);
        assert!(matches!(
            handle_in_layer(&mut state, &sample_272()),
            LayerResult::Ignored
        ));
        assert_eq!(state.preset_dump_ack_ctr, 0x1df4);
    }

}
