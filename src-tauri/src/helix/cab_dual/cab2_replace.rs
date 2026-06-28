//! Point d'entrée replace Cab 2 — délègue à [`super::replace_fire`].

use std::sync::{Arc, Mutex};

use crate::helix::HelixState;

pub fn execute_cab_dual_cab2_replace(
    helix_arc: Arc<Mutex<HelixState>>,
    slot_index: u32,
    slot_bus: u8,
    usb_bulk: &[u8],
) -> Result<String, String> {
    super::replace_fire::execute_cab_dual_cab2_replace_fire(
        helix_arc,
        slot_index,
        slot_bus,
        usb_bulk,
    )
}
