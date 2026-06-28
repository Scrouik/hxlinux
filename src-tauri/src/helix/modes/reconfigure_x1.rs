use std::time::Instant;

use crate::helix::{Mode, HelixState};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;

/// Secours si le Stomp ne répond pas au `0c` subscribe (rare).
const RECONFIGURE_BOOTSTRAP_TIMEOUT_MS: u128 = 2500;

pub struct ReconfigureX1 {
    phase_since: Instant,
    done: bool,
}

impl ReconfigureX1 {
    pub fn new() -> Self {
        Self {
            phase_since: Instant::now(),
            done: false,
        }
    }

    fn finish_bootstrap(&mut self, state: &mut HelixState) {
        if self.done {
            return;
        }
        self.done = true;
        eprintln!("[ReconfigureX1] bootstrap OK → ARM + AwaitPostBootstrapSettle");
        crate::helix::amorcage::send_arm_f0(state);
        // Pas de `11 ef` ici — HX Edit n'en envoie pas entre ARM_f0 et ARM_ef (évite de
        // consommer un `x1_cnt` et de décaler ARM_ef de 03 à 04).
        crate::helix::amorcage::finish_usb_bootstrap(state, false);
    }
}

impl Mode for ReconfigureX1 {
    fn start(&mut self, state: &mut HelixState) {
        self.phase_since = Instant::now();
        self.done = false;
        state.x1_cnt = 0x02;
        state.send(OutPacket::new(vec![
            0x0c, 0x00, 0x00, 0x28,
            0x01, 0x10, 0xef, 0x03,
            0x00, 0x00, 0x00, 0x02,
            0x00, 0x01, 0x00, 0x21,
            0x00, 0x10, 0x00, 0x00,
        ]));
        eprintln!("[ReconfigureX1] OUT 0c ef subscribe");
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {
        if !self.done && self.phase_since.elapsed().as_millis() > RECONFIGURE_BOOTSTRAP_TIMEOUT_MS {
            eprintln!(
                "[ReconfigureX1] timeout {} ms — bootstrap forcé",
                RECONFIGURE_BOOTSTRAP_TIMEOUT_MS
            );
            crate::helix::amorcage::send_arm_f0(state);
            crate::helix::amorcage::finish_usb_bootstrap(state, false);
            self.done = true;
            return true;
        }

        // ACK `0c` → `11 OUT` (Kempline + HX Edit)
        if byte_cmp(
            data,
            &pattern![
                0x0c, 0x00, 0x00, 0x28,
                0xef, 0x03, 0x01, 0x10,
                0x00, XX, 0x00, 0x02,
                0x00, 0x01, 0x00, 0x01,
                0x00, 0x02, 0x00, 0x00
            ],
            20,
        ) {
            let cnt = state.next_x1_cnt();
            state.send(OutPacket::new(vec![
                0x11, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, cnt, 0x00, 0x04,
                0x00, 0x10, 0x00, 0x00,
                0x01, 0x00, 0x02, 0x00,
                0x01, 0x00, 0x00, 0x00,
                0x02, 0x00, 0x00, 0x00,
            ]));
            eprintln!("[ReconfigureX1] IN 0c ack → OUT 11 ef");
            return true;
        }

        // `IN 11 ef` final (chemin Stomp / Kempline — une seule ronde)
        if byte_cmp(
            data,
            &pattern![
                0x11, 0x00, 0x00, 0x18,
                0xef, 0x03, 0x01, 0x10,
                0x00, XX, 0x00, 0x04
            ],
            12,
        ) {
            self.finish_bootstrap(state);
            return true;
        }

        if Standard::check_keep_alive(data, state) {
            return false;
        }

        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {}
}
