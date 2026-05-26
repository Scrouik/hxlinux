//! Scroll modèle hardware (molette Stomp) — **pull désactivé** (reset mai 2026).
//!
//! Reprise prévue : replay calqué HX Edit (`captures/usb-wireshark/`, voir `docs/SCROLL_HW_RESET.md`).
//! Seuls les ACK scroll `1d`/`1f` restent actifs pour ne pas bloquer le firmware.

use serde::Serialize;

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

/// Fenêtre pendant laquelle `request_preset_content` évite le dump USB (legacy API).
pub const HW_MODEL_USB_BUSY_AFTER_SCROLL_MS: u64 = 700;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotModelHwChangedPayload {
    pub sequence: u32,
    pub slot_index: u32,
    pub slot_bus: u8,
    pub module_hex: Option<String>,
}

pub fn init_slot_model_hw_pull_debug_from_env() {}

pub fn set_slot_model_hw_pull_debug(_enabled: bool) {}

pub fn slot_model_hw_pull_debug_enabled() -> bool {
    false
}

pub fn hw_model_usb_busy(_state: &HelixState) -> bool {
    false
}

pub fn clear_stale_hw_model_lane_flood(state: &mut HelixState) {
    state.hw_model_last_scroll_in_at = None;
}

pub fn post_pull_stream_settling_active(_state: &HelixState) -> bool {
    false
}

/// IN `21` 44 o post-assign (filtre dump preset dans `standard.rs`).
pub fn is_hw_model_post_assign_21(data: &[u8]) -> bool {
    data.len() == 44
        && data.first() == Some(&0x21)
        && data.get(24..28) == Some(&[0x82, 0x69, 0x27, 0x6a])
        && data.windows(4).any(|w| w == [0x82, 0x62, 0x01, 0x1a])
}

/// ACK `f0:03` sub=08 sur `1d` / `1f` 40 o — lane [`HelixState::hw_model_scroll_ack_ctr`].
pub fn ack_hw_model_scroll_in(state: &mut HelixState, data: &[u8]) -> bool {
    let head = data.first().copied().unwrap_or(0);
    if head != 0x1d && head != 0x1f || data.len() != 40 {
        return false;
    }
    if state.init_usb_settle_active() || state.preset_usb_read_in_progress() {
        return false;
    }
    if head == 0x1d && !state.should_ack_firmware_1d_notify() {
        return false;
    }
    let cnt = state.next_x2_cnt();
    let double = state.next_hw_model_scroll_ack_double(head);
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08,
        double[0], double[1], 0x00, 0x00,
    ]));
    head == 0x1f
}

/// Pull modèle HW désactivé — pas d’émission `models:slot-model-changed`.
pub fn ingest_slot_model_hw_in(
    _state: &mut HelixState,
    _data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    None
}
