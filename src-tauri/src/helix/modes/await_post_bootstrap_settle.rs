// Mode passif pendant l’amorçage USB (`amorcage::spawn_post_arm_sequence` : phase4 + settle).

use crate::helix::modes::standard::Standard;
use crate::helix::{HelixState, Mode};

pub struct AwaitPostBootstrapSettle;

impl AwaitPostBootstrapSettle {
    pub fn new() -> Self {
        Self
    }
}

impl Mode for AwaitPostBootstrapSettle {
    fn start(&mut self, _state: &mut HelixState) {
        crate::helix::init_trace::trace(
            "AwaitPostBootstrapSettle::start — passif (timeline amorcage thread)",
        );
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {
        if Standard::check_keep_alive(data, state) {
            return false;
        }
        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {}
}
