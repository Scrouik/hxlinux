//! Cinématique amorçage USB — alignée captures HX Edit (`01_connect`, `stomp_running…`).
//!
//! **Entrelacement** (pas de batch ARM) :
//! - `Connect` : réponse init x2 → `ARM_ed` puis ack x11 x2
//! - `ReconfigureX1` : x11 ef final → `ARM_f0` → ack x11 → `ARM_ef`
//! - Gate 3× `IN 08/16o` (ef+ed+f0) après `ARM_ef` → ~200 ms → phase 4 → **trailer `7a` 132o** → settle 700 ms → `EditorReady`
//! - Fallback : `spawn_post_arm_sequence` (timer 235 ms) si pas de gate_rx

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::helix::keep_alive::POST_PHASE4_SETTLE_MS;
use crate::helix::packet::OutPacket;
use crate::helix::{HelixState, KeepAliveCommand, ModeRequest};

/// HX Edit : ARM_ef ~16 ms après ARM_f0 (#1483 vs #1473).
pub const ARM_EF_DELAY_AFTER_F0_MS: u64 = 16;
/// HX Edit : 1er OUT phase4 ~235 ms après ARM_f0 (#1545 vs #1473 connect).
pub const PHASE4_DELAY_AFTER_ARM_F0_MS: u64 = 235;
/// HX Edit (`stomp_running`) : gate ~+20 ms, 1er `19` ~+220 ms après `ARM_ef` → ~200 ms de silence host.
pub const PHASE4_DELAY_AFTER_GATE_MS: u64 = 200;
/// Secours si le trailer `7a` 132 o n'arrive pas (rafale ~11×272 + fin ≈1 s sur captures).
const PHASE4_COMPLETE_TIMEOUT_MS: u64 = 3500;

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

/// Fin séquence ARM post-Reconfigure : `ARM_f0` → `ARM_ef` → gate → `AwaitPostBootstrapSettle`.
/// `send_f0` : `false` si `ARM_f0` vient d'être envoyé (ex. 2e ACK `0c`).
pub fn finish_usb_bootstrap(state: &mut HelixState, send_f0: bool) {
    if send_f0 {
        send_arm_f0(state);
    }
    send_arm_ef(state);
    state.arm_post_ef_gate();
    state.arm_phase4_complete_gate();
    state.switch_mode(ModeRequest::AwaitPostBootstrapSettle);
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

fn run_phase4_then_settle(state: &Arc<Mutex<HelixState>>, phase4_rx: Receiver<()>) {
    {
        let mut s = state.lock().unwrap();
        if !s.connected {
            return;
        }
        s.phase4_seen_19ef_pre_postarm = false;
        s.phase4_post1a_timeout = None;
        crate::helix::init_trace::trace("amorcage phase4 OUT BEGIN (3×19 + 1a)");
        crate::helix::phase4_state::arm(&mut s.phase4_step);
        s.start_phase4_bootstrap();
    }

    match phase4_rx.recv_timeout(Duration::from_millis(PHASE4_COMPLETE_TIMEOUT_MS)) {
        Ok(()) => {
            crate::helix::init_trace::trace("amorcage phase4 IN trailer reçu");
        }
        Err(_) => {
            eprintln!(
                "[amorcage] WARN timeout phase4 ({} ms) — settle forcé",
                PHASE4_COMPLETE_TIMEOUT_MS
            );
            let mut s = state.lock().unwrap();
            s.phase4_bootstrap_active = false;
        }
    }

    {
        let mut s = state.lock().unwrap();
        if !s.connected {
            return;
        }
        s.begin_init_usb_settle();
        crate::helix::init_trace::trace_fmt(format_args!(
            "amorcage settle BEGIN ({} ms, après fin phase 4)",
            POST_PHASE4_SETTLE_MS
        ));
    }

    thread::sleep(Duration::from_millis(POST_PHASE4_SETTLE_MS));

    // Attendre la fin de la FSM post-1a (secours max 3s) pour éviter
    // d'entrelacer RequestPresetNames avec un dialogue encore en cours.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(3000);
    loop {
        {
            let s = state.lock().unwrap();
            if !matches!(
                s.phase4_step,
                crate::helix::phase4_state::Phase4Step::PostArm
                    | crate::helix::phase4_state::Phase4Step::WaitAck2
                    | crate::helix::phase4_state::Phase4Step::WaitIn1f
                    | crate::helix::phase4_state::Phase4Step::WaitIn1b26
                    | crate::helix::phase4_state::Phase4Step::WaitPresetAck
            ) {
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            crate::helix::init_trace::trace(
                "[amorcage] timeout attente FSM post-1a -> RequestPresetNames forcé",
            );
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }

    {
        let mut s = state.lock().unwrap();
        if !s.connected {
            eprintln!("[amorcage] settle abort: disconnected");
            return;
        }
        s.editor_ready = true;
        s.end_init_usb_settle();
        s.start_keepalive(KeepAliveCommand::StartOrdered);
        crate::helix::init_trace::trace("amorcage EditorReady");
    }
}

fn finish_editor_bootstrap(state: &Arc<Mutex<HelixState>>, mode_tx: &Sender<ModeRequest>) {
    eprintln!("[amorcage] → RequestPresetNames (post settle)");
    if mode_tx.send(ModeRequest::RequestPresetNames).is_err() {
        eprintln!("[amorcage] ERREUR: mode_tx.send(RequestPresetNames) failed");
    }
}

/// Phase 4 après gate événementielle (3× `IN 08/16o` post `ARM_ef`, timeout secours 500 ms).
pub fn spawn_post_gate_sequence(
    state: Arc<Mutex<HelixState>>,
    mode_tx: Sender<ModeRequest>,
    gate_rx: Receiver<()>,
) {
    let phase4_rx = {
        let mut s = state.lock().unwrap();
        s.phase4_complete_rx.take()
    };
    let Some(phase4_rx) = phase4_rx else {
        eprintln!("[amorcage] ERREUR: phase4_complete_rx absent");
        return;
    };

    thread::spawn(move || {
        match gate_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(()) => {
                crate::helix::init_trace::trace("gate post-ARM_ef reçue");
            }
            Err(_) => {
                crate::helix::init_trace::trace(
                    "[WARN] timeout gate post-ARM_ef (500ms) — phase 4 quand même",
                );
            }
        }

        // Silence proactif post-gate : laisser le Stomp pousser des `1d` fond + ACK réactifs
        // avant les OUT `19`/`1a` (HX : Δ gate → 1er `19` ≈ 200 ms, `stomp_running_start_hxedit.json`).
        crate::helix::init_trace::trace_fmt(format_args!(
            "amorcage pause {} ms (gate → phase 4)",
            PHASE4_DELAY_AFTER_GATE_MS
        ));
        thread::sleep(Duration::from_millis(PHASE4_DELAY_AFTER_GATE_MS));

        run_phase4_then_settle(&state, phase4_rx);
        finish_editor_bootstrap(&state, &mode_tx);
    });
}

/// Phase 4 + settle — fallback timer 235 ms (si `post_ef_gate_rx` absent).
pub fn spawn_post_arm_sequence(state: Arc<Mutex<HelixState>>, mode_tx: Sender<ModeRequest>) {
    let phase4_rx = {
        let mut s = state.lock().unwrap();
        s.arm_phase4_complete_gate();
        s.phase4_complete_rx.take()
    };
    let Some(phase4_rx) = phase4_rx else {
        eprintln!("[amorcage] ERREUR: phase4_complete_rx absent (fallback)");
        return;
    };

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(PHASE4_DELAY_AFTER_ARM_F0_MS));

        run_phase4_then_settle(&state, phase4_rx);
        finish_editor_bootstrap(&state, &mode_tx);
    });
}
