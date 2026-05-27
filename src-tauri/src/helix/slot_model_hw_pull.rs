//! Scroll modèle hardware (molette Stomp) — **pull désactivé** (reset mai 2026).
//!
//! Les ACK `1d`/`1f` sont dans [`crate::helix::firmware_scroll_ack`].
//! Réimplémentation pull prévue depuis `captures/usb-wireshark/3_scroll_HXEdit.json`.

use serde::Serialize;

use crate::helix::HelixState;
use crate::helix::usb_in_pipeline::LayerResult;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotModelHwChangedPayload {
    pub sequence: u32,
    pub slot_index: u32,
    pub slot_bus: u8,
    pub module_hex: Option<String>,
}

/// Couche active scroll (pull) — désactivée jusqu’à phase 4.
pub fn handle_in_layer(_state: &mut HelixState, _data: &[u8]) -> LayerResult {
    LayerResult::Ignored
}

/// Aucun pull ni émission `models:slot-model-changed` tant que la couche n’est pas réécrite.
pub fn ingest_slot_model_hw_in(
    _state: &mut HelixState,
    _data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    None
}
