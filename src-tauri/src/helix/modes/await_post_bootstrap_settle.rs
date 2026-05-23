// Attente post bootstrap phase 4 (aligné HX Edit) avant RequestPresetNames.
// Kempline enchaînait trop tôt : dump preset / noms en parallèle du handshake.

use std::thread;
use std::time::Duration;

use crate::helix::keep_alive::POST_PHASE4_SETTLE_MS;
use crate::helix::modes::standard::Standard;
use crate::helix::{HelixState, Mode, ModeRequest};

pub struct AwaitPostBootstrapSettle;

impl AwaitPostBootstrapSettle {
    pub fn new() -> Self {
        Self
    }
}

impl Mode for AwaitPostBootstrapSettle {
    fn start(&mut self, state: &mut HelixState) {
        crate::helix::init_trace::trace("AwaitPostBootstrapSettle::start — attente POST_PHASE4_SETTLE");
        state.begin_init_usb_settle();
        if preset_debug_verbose_enabled() {
            eprintln!(
                "[PresetDebug][init] {POST_PHASE4_SETTLE_MS} ms : ACK seulement (pas de requête host) → RequestPresetNames"
            );
        }
        if let Some(mode_tx) = state.mode_tx.clone() {
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(POST_PHASE4_SETTLE_MS));
                let _ = mode_tx.send(ModeRequest::RequestPresetNames);
            });
        }
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {
        if Standard::check_keep_alive(data, state) {
            return false;
        }
        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {}
}

fn preset_debug_verbose_enabled() -> bool {
    crate::helix::preset_debug_verbose_enabled()
}
