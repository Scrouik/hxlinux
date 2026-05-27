//! Cinématique amorçage USB — alignée captures HX Edit (`01_connect`, `stomp_running…`).
//!
//! **Entrelacement** (pas de batch ARM) :
//! - `Connect` : réponse init x2 → `ARM_ed` puis ack x11 x2
//! - `ReconfigureX1` : x11 ef final → `ARM_f0` → ack x11 → `ARM_ef`
//! - Thread : silence ~235 ms → phase 4 → settle 700 ms → `EditorReady`

use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use crate::helix::editor_phase4_bootstrap;
use crate::helix::keep_alive::POST_PHASE4_SETTLE_MS;
use crate::helix::packet::OutPacket;
use crate::helix::{HelixState, KeepAliveCommand, ModeRequest};

/// HX Edit : ARM_ef ~16 ms après ARM_f0 (#1483 vs #1473).
pub const ARM_EF_DELAY_AFTER_F0_MS: u64 = 16;
/// HX Edit : 1er OUT phase4 ~235 ms après ARM_f0 (#1545 vs #1473 connect).
pub const PHASE4_DELAY_AFTER_ARM_F0_MS: u64 = 235;

fn arm_packet_ed(cnt: u8) -> Vec<u8> {
    vec![
        0x08, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt, 0x00, 0x08,
        0x09, 0x10, 0x00, 0x00,
    ]
}

fn arm_packet_f0(cnt: u8) -> Vec<u8> {
    vec![
        0x08, 0x00, 0x00, 0x18,
        0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08,
        0x09, 0x10, 0x00, 0x00,
    ]
}

fn arm_packet_ef(cnt: u8) -> Vec<u8> {
    vec![
        0x08, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt, 0x00, 0x08,
        0x09, 0x10, 0x00, 0x00,
    ]
}

/// Après réponse init x2 — avant ack x11 x2 (`01_connect` #1455).
pub fn send_arm_ed(state: &mut HelixState) {
    crate::helix::init_trace::trace("amorcage ARM_ed 09:10");
    let cnt = state.next_x80_cnt();
    state.send(OutPacket::new(arm_packet_ed(cnt)));
}

/// Fin ReconfigureX1 — avant ack x11 ef et `ARM_ef` (`#1473`).
pub fn send_arm_f0(state: &mut HelixState) {
    crate::helix::init_trace::trace("amorcage ARM_f0 09:10");
    state.firmware_scroll_armed = false;
    let cnt = state.next_x2_cnt();
    state.send(OutPacket::new(arm_packet_f0(cnt)));
    state.note_firmware_scroll_bootstrap_sent();
}

/// Juste après `ARM_f0` (+16 ms sur le fil HX Edit).
pub fn send_arm_ef(state: &mut HelixState) {
    crate::helix::init_trace::trace("amorcage ARM_ef 09:10");
    let cnt = state.next_x1_cnt();
    state.send(OutPacket::with_delay(
        arm_packet_ef(cnt),
        ARM_EF_DELAY_AFTER_F0_MS,
    ));
}

/// Phase 4 + settle + keep-alive — **après** entrelacement ARM (modes Connect / ReconfigureX1).
pub fn spawn_post_arm_sequence(state: Arc<Mutex<HelixState>>, mode_tx: Sender<ModeRequest>) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(PHASE4_DELAY_AFTER_ARM_F0_MS));

        {
            let mut s = state.lock().unwrap();
            if !s.connected {
                return;
            }
            crate::helix::init_trace::trace("amorcage phase4 BEGIN");
            editor_phase4_bootstrap::send(&mut s);
            s.begin_init_usb_settle();
        }

        thread::sleep(Duration::from_millis(POST_PHASE4_SETTLE_MS));

        {
            let mut s = state.lock().unwrap();
            if !s.connected {
                return;
            }
            s.editor_ready = true;
            s.start_keepalive(KeepAliveCommand::StartOrdered);
            s.end_init_usb_settle();
            crate::helix::init_trace::trace("amorcage EditorReady → keep-alive + RequestPresetNames");
        }

        let _ = mode_tx.send(ModeRequest::RequestPresetNames);
    });
}
