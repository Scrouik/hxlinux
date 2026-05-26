//! Scroll modèle hardware (molette Stomp) — **désactivé** (reset mai 2026).
//!
//! Réimplémentation prévue depuis `captures/usb-wireshark/3_scroll_HXEdit.json`.
//! Voir `docs/SCROLL_HW_RESET.md`.

use serde::Serialize;

use crate::helix::HelixState;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotModelHwChangedPayload {
    pub sequence: u32,
    pub slot_index: u32,
    pub slot_bus: u8,
    pub module_hex: Option<String>,
}

/// Aucun pull ni émission `models:slot-model-changed` tant que la couche n’est pas réécrite.
pub fn ingest_slot_model_hw_in(
    _state: &mut HelixState,
    _data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    None
}
