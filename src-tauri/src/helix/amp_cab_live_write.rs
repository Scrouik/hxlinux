//! Routage write live **partie cab** d'un slot Amp+Cab.
//!
//! - **IR** (hardware Stomp XL) : capture `add_amp_cab_modif_param_cab.json` — PP `0x03`,
//!   `param_selector` = index local 0..n, bloc `83:66:cd:03:YY:64:1e:65:85:62:bus:1d:c3:1a:01:1c`.
//! - **Legacy hybrid** (référence seulement) : `amp_cab legacy guitar.json` — PP `0x08`, sel `0x25+`.

use std::collections::HashMap;

use crate::helix::live_write::LiveWriteRouteOverride;
use crate::helix::{echo_model_cache_key, kempline_index_to_slot_bus, HelixState};
use crate::preset_chain_params;

/// `(param_selector, model_tag)` — guitar legacy hybrid (non assignable sur Stomp XL actuel).
const LEGACY_GUITAR_CAB_ROUTES: &[(u8, u8)] = &[
    (0x25, 0x05),
    (0x26, 0x03),
    (0x27, 0x1b),
    (0x28, 0x1d),
    (0x29, 0x1f),
    (0x2a, 0x21),
    (0x2b, 0x13),
];

const LEGACY_COMPACT_CAB_ROUTES: &[(u8, u8)] = &[
    (0x00, 0xcb),
    (0x01, 0xcd),
    (0x02, 0xc9),
    (0x03, 0xcf),
    (0x04, 0xbb),
    (0x05, 0xb9),
    (0x06, 0xb7),
];

fn is_legacy_variant(assign_variant: &str) -> bool {
    assign_variant.eq_ignore_ascii_case("amp+cab-legacy")
}

fn legacy_cab_wire_pair(local_param_index: u32, amp_block_len: usize) -> Option<(u8, u8)> {
    let idx = local_param_index as usize;
    if amp_block_len >= 10 {
        LEGACY_GUITAR_CAB_ROUTES.get(idx).copied()
    } else {
        LEGACY_COMPACT_CAB_ROUTES.get(idx).copied()
    }
}

/// Bloc modèle cab **IR** Amp+Cab (capture HX Edit 2026-06).
pub fn build_amp_cab_ir_param_model_block(slot_bus: u8, tag_yy: u8) -> [u8; 16] {
    [
        0x83, 0x66, 0xcd, 0x03, tag_yy, 0x64, 0x1e, 0x65, 0x85, 0x62, slot_bus, 0x1d, 0xc3,
        0x1a, 0x01, 0x1c,
    ]
}

/// Bloc modèle cab legacy hybrid (`…c3:19`).
pub fn build_amp_cab_legacy_param_model_block(pp: u8, tag: u8, slot_bus: u8) -> [u8; 16] {
    [
        0x83, 0x66, 0xcd, pp, tag, 0x64, 0x28, 0x65, 0x82, 0x62, slot_bus, 0x64, 0x83, 0x17,
        0xc3, 0x19,
    ]
}

/// Focus sous-bloc cab legacy (`1b`, capture legacy guitar) — absent sur IR.
pub fn build_amp_cab_cab_focus_packet(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    let cnt = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let model_block = [
        0x83, 0x66, 0xcd, 0x08, 0x04, 0x64, 0x21, 0x65, 0x81, 0x66, slot_bus, 0x08, 0x00, 0x00,
        0x00, 0x00,
    ];
    vec![
        0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt, 0x00, 0x04,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00,
        model_block[0],
        model_block[1],
        model_block[2],
        model_block[3],
        model_block[4],
        model_block[5],
        model_block[6],
        model_block[7],
        model_block[8],
        model_block[9],
        model_block[10],
        model_block[11],
        model_block[12],
        model_block[13],
        model_block[14],
        model_block[15],
    ]
}

pub fn cache_ed03_model_blocks_from_echo(
    cache: &mut HashMap<(u8, u8, u8), [u8; 16]>,
    data: &[u8],
    model_block: [u8; 16],
) {
    use crate::helix::slot_param_in::scan_slot_param_samples;
    let route_pp = model_block[3];
    for sample in scan_slot_param_samples(data) {
        cache.insert(
            echo_model_cache_key(sample.slot_bus, route_pp, sample.param_selector),
            model_block,
        );
    }
}

pub fn resolve_cab_live_write_route(
    state: &HelixState,
    seg: &[u8],
    local_param_index: u32,
    assign_variant: &str,
    slot_index: u32,
) -> Option<LiveWriteRouteOverride> {
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)?;
    let legacy = is_legacy_variant(assign_variant);
    let amp_block_len = preset_chain_params::parse_assignable_segment_param_blocks(seg)
        .and_then(|blocks| blocks.first().map(|b| b.len()))
        .unwrap_or(0);

    if legacy {
        let (param_selector, tag) = legacy_cab_wire_pair(local_param_index, amp_block_len)?;
        let cache_key = echo_model_cache_key(slot_bus, 0x08, param_selector);
        if let Some(block) = state.ed03_echo_model_by_slot_param.get(&cache_key) {
            return Some(LiveWriteRouteOverride {
                pp: block[3],
                pp_source: "amp_cab:legacy_echo_cache",
                param_selector,
                param_selector_source: "amp_cab:legacy_echo_sel",
                model_block: *block,
                preserve_model_tag: true,
            });
        }
        let model_block = build_amp_cab_legacy_param_model_block(0x08, tag, slot_bus);
        return Some(LiveWriteRouteOverride {
            pp: 0x08,
            pp_source: "amp_cab:legacy_table",
            param_selector,
            param_selector_source: if amp_block_len >= 10 {
                "amp_cab:legacy_guitar_sel"
            } else {
                "amp_cab:legacy_compact_sel"
            },
            model_block,
            preserve_model_tag: true,
        });
    }

    // IR Amp+Cab : index catalogue = sélecteur wire (capture add_amp_cab_modif_param_cab.json).
    let param_selector = local_param_index.min(0xff) as u8;
    let cache_key = echo_model_cache_key(slot_bus, 0x03, param_selector);
    if let Some(block) = state.ed03_echo_model_by_slot_param.get(&cache_key) {
        return Some(LiveWriteRouteOverride {
            pp: block[3],
            pp_source: "amp_cab:ir_echo_cache",
            param_selector,
            param_selector_source: "amp_cab:ir_echo_sel",
            model_block: *block,
            preserve_model_tag: true,
        });
    }

    let model_block = build_amp_cab_ir_param_model_block(slot_bus, state.live_write_yy);
    Some(LiveWriteRouteOverride {
        pp: 0x03,
        pp_source: "amp_cab:ir_capture",
        param_selector,
        param_selector_source: "amp_cab:ir_local_index",
        model_block,
        preserve_model_tag: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ir_cab_level_is_sel_00_pp_03() {
        use crate::helix::HelixState;
        let state = HelixState::new();
        let route = resolve_cab_live_write_route(&state, &[], 0, "amp+cab", 3).expect("route");
        assert_eq!(route.pp, 0x03);
        assert_eq!(route.param_selector, 0x00);
        assert_eq!(route.model_block[3], 0x03);
        assert_eq!(&route.model_block[11..16], &[0x1d, 0xc3, 0x1a, 0x01, 0x1c]);
    }

    #[test]
    fn legacy_guitar_cab_level_is_sel_25_tag_05() {
        let pair = legacy_cab_wire_pair(0, 21).expect("route");
        assert_eq!(pair, (0x25, 0x05));
    }
}
