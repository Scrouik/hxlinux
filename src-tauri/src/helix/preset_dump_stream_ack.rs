//! ACK host des trames IN **flux preset / slot** (`08:01:ed:03:80:10`, sub=`04`, souvent 272 o).
//!
//! HX Edit acquitte **chaque** chunk (~11 par dump) avec `80:10:ed:03` sub=`08` et les octets
//! 12–13 = [`HelixState::editor_ed03_lane`] (`9d:10` → `9d:11` → … pendant le dump).
//!
//! **Défaut (depuis mai 2026)** : l'ACK part sur `editor_ed03_lane` (aligné HX / phase 4).
//! **Témoin** : `HX_DUMP_ACK_LANE=f4` reprend l'expérience `f4:1d` → `f4:1e` (octet 12 figé).

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

/// Couche active « ACK chunks 272 ».
pub fn handle_in_layer(state: &mut HelixState, data: &[u8]) -> LayerResult {
    if !is_preset_dump_stream_chunk_in(data) {
        return LayerResult::Ignored;
    }
    // Pendant `init_usb_settle` le host n'envoie pas de requêtes proactives mais doit
    // continuer à ACKer les IN 272 (phase 4 + queue device). Seul RequestPreset* coupe.
    if state.preset_usb_read_in_progress() {
        return LayerResult::Ignored;
    }

    let cnt = state.next_x80_cnt();
    let lane = state.next_preset_stream_chunk_ack_lane();
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt, 0x00, 0x08,
        lane[0], lane[1], 0x00, 0x00,
    ]));
    if preset_dump_stream_ack_debug_enabled() {
        eprintln!(
            "[PresetDumpStreamAck] IN len={} → OUT ed:03 sub=08 cnt={cnt:#04x} lane={:02x}:{:02x} editor={}",
            data.len(),
            lane[0],
            lane[1],
            HelixState::preset_dump_ack_use_editor_lane()
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
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    fn with_f4_lane(f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("HX_DUMP_ACK_LANE", "f4");
        f();
        std::env::remove_var("HX_DUMP_ACK_LANE");
    }

    fn with_editor_lane(f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("HX_DUMP_ACK_LANE");
        f();
    }

    #[test]
    fn detects_stream_chunk() {
        assert!(is_preset_dump_stream_chunk_in(&sample_272()));
        assert!(!is_preset_dump_stream_chunk_in(&[0x53; 92]));
    }

    #[test]
    fn editor_lane_on_wire_during_phase4() {
        with_editor_lane(|| {
            let mut state = HelixState::new();
            state.reset_editor_ed03_lane();
            state.phase4_bootstrap_active = true;
            state.editor_ed03_lane = 0x109d;
            for expected_hi in 0x11u8..=0x1b {
                let _ = handle_in_layer(&mut state, &sample_272());
                let [lo, hi] = state.editor_ed03_lane_bytes();
                assert_eq!(lo, 0x9d, "lo figé pendant le dump");
                assert_eq!(hi, expected_hi, "hi monte après ACK sur le fil");
            }
        });
    }

    #[test]
    fn f4_lane_temoin_increments_hi_only() {
        with_f4_lane(|| {
            let mut state = HelixState::new();
            assert_eq!(state.preset_dump_ack_ctr, 0x1df4);
            assert!(matches!(
                handle_in_layer(&mut state, &sample_272()),
                LayerResult::Consumed { .. }
            ));
            assert_eq!(state.preset_dump_ack_double(), [0xf4, 0x1e]);
            assert_eq!(state.preset_dump_ack_ctr, 0x1ef4);
        });
    }

    #[test]
    fn f4_temoin_many_acks_keep_session_f4() {
        with_f4_lane(|| {
            let mut state = HelixState::new();
            for i in 0..25 {
                let [s, c] = state.preset_dump_ack_double();
                assert_eq!(s, 0xf4, "ACK #{i} session");
                assert_eq!(c, 0x1d + i, "ACK #{i} counter");
                let _ = handle_in_layer(&mut state, &sample_272());
            }
        });
    }

    #[test]
    fn skips_during_request_preset() {
        with_editor_lane(|| {
            let mut state = HelixState::new();
            state.set_preset_usb_read_modes_active(true);
            assert!(matches!(
                handle_in_layer(&mut state, &sample_272()),
                LayerResult::Ignored
            ));
            assert_eq!(state.preset_dump_ack_ctr, 0x1df4);
        });
    }

    #[test]
    fn editor_lane_untouched_outside_phase4() {
        with_editor_lane(|| {
            let mut state = HelixState::new();
            state.reset_editor_ed03_lane();
            let before = state.editor_ed03_lane_bytes();
            assert!(!state.phase4_bootstrap_active);
            let _ = handle_in_layer(&mut state, &sample_272());
            let after = state.editor_ed03_lane_bytes();
            assert_eq!(after[0], before[0]);
            assert_eq!(after[1], before[1].wrapping_add(1), "hi +1 même hors phase4");
        });
    }
}
