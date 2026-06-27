//! Routage write live **partie cab** d'un slot Amp+Cab.
//!
//! - **IR** (hardware Stomp XL) : capture `add_amp_cab_modif_param_cab.json` — PP `0x03`,
//!   `param_selector` = index local 0..n, bloc `83:66:cd:03:YY:64:1e:65:85:62:bus:1d:c3:1a:01:1c`.
//! - **Legacy hybrid** (référence seulement) : `amp_cab legacy guitar.json` — PP `0x08`, sel `0x25+`.

use std::collections::HashMap;

use crate::helix::live_write::LiveWriteRouteOverride;
use crate::helix::live_write_config::{discrete_23_step_count, infer_bool_wire_payload, live_write_cfg};
use crate::helix::{echo_model_cache_key, kempline_index_to_slot_bus, HelixState};

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

pub fn legacy_cab_wire_pair(local_param_index: u32, amp_block_len: usize) -> Option<(u8, u8)> {
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

const AMP_CAB_TAB_FOCUS_SUB: u8 = 0x04;

/// Tag session (octet après `83:66:cd:XX`) dans un bulk assign/replace Amp+Cab.
pub fn lane_tag_after_cd_lane_in_bulk(bulk: &[u8]) -> Option<u8> {
    for i in 0..bulk.len().saturating_sub(5) {
        if bulk[i] == 0x83 && bulk[i + 1] == 0x66 && bulk[i + 2] == 0xcd {
            return Some(bulk[i + 4]);
        }
    }
    None
}

/// Alias : lane `cd:03` (replace cab / focus).
pub fn lane_tag_after_cd03_in_bulk(bulk: &[u8]) -> Option<u8> {
    for i in 0..bulk.len().saturating_sub(5) {
        if bulk[i..i + 4] == [0x83, 0x66, 0xcd, 0x03] {
            return Some(bulk[i + 4]);
        }
    }
    None
}

fn patch_amp_cab_tab_focus_lane_tag(focus: &mut [u8], tag: u8) {
    for i in 0..focus.len().saturating_sub(5) {
        if focus[i..i + 4] == [0x83, 0x66, 0xcd, 0x03] {
            focus[i + 4] = tag;
            return;
        }
    }
}

/// Après assign AddToEmpty : mémorise le tag ampli+cab (pas de bump `live_write_ctr` : pas d'ed:08).
pub fn record_amp_cab_assign_session(state: &mut HelixState, slot_index: u32, bulk: &[u8]) {
    if let Some(tag) = lane_tag_after_cd_lane_in_bulk(bulk) {
        state
            .amp_cab_focus_lane_tags_by_slot
            .insert(slot_index, (tag, tag));
    }
}

/// Après replace cab : met à jour le tag cab (le tag ampli reste celui de l’assign).
pub fn record_amp_cab_cab_replace_session(state: &mut HelixState, slot_index: u32, bulk: &[u8]) {
    if let Some(cab_tag) = lane_tag_after_cd03_in_bulk(bulk) {
        let entry = state
            .amp_cab_focus_lane_tags_by_slot
            .entry(slot_index)
            .or_insert((cab_tag, cab_tag));
        entry.1 = cab_tag;
    }
}

/// Focus onglet **Cab** (`1d`, `cd:03`, `1a:01`) — lane `live_write_ctr` (HX Edit).
pub fn build_amp_cab_ir_cab_focus_packet(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    use crate::helix::cab_dual_live_write::{build_cab_dual_cab2_tab_focus_packet_with_lane, Cab2FocusLane};
    build_cab_dual_cab2_tab_focus_packet_with_lane(
        state,
        slot_bus,
        Cab2FocusLane::LiveWrite,
        AMP_CAB_TAB_FOCUS_SUB,
    )
}

/// Focus onglet **Amp** (`1d`, `cd:03`, `1a:00`) — capture `ampcab_legacy_switch_tab.json`.
pub fn build_amp_cab_amp_focus_packet(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    use crate::helix::cab_dual_live_write::{build_cab_dual_focus_packet_with_lane, Cab2FocusLane};
    build_cab_dual_focus_packet_with_lane(
        state,
        slot_bus,
        Cab2FocusLane::LiveWrite,
        AMP_CAB_TAB_FOCUS_SUB,
        0x03,
        0x00,
    )
}

fn env_delay_ms(var: &str, default_ms: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default_ms)
}

/// Focus onglet Amp ou Cab : `1d` → `ed:08` → poke `f0:08` (legacy + IR, HX Edit).
pub fn spawn_amp_cab_tab_focus_usb(
    helix_arc: std::sync::Arc<std::sync::Mutex<crate::helix::HelixState>>,
    slot_index: u32,
    slot_bus: u8,
    cab: bool,
) {
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::helix::cab_dual_live_write::{send_cab_dual_cab2_f008_poke, send_cab_dual_f0_poke};
    use crate::helix::ed03_lane::{build_ed08_short, force_ed03_ctr};
    use crate::helix::init_trace;
    use crate::helix::packet::OutPacket;

    let delay_ed08 = env_delay_ms("HX_CAB2_DELAY_ED08_MS", 93);
    let l = {
        let mut s = match helix_arc.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if !s.connected || s.preset_content_only {
            return;
        }
        // Bloque l’ACK `ed:08` auto de `standard` (session_quadruple) post-IN `19` :
        // il part avant le nôtre (ctr = L+0x11) et peut afficher le mauvais sous-bloc (ex. Soup Pro sur Amp).
        s.cab_dual_cab2_suppress_standard_ed08_until =
            Some(Instant::now() + Duration::from_millis(450));
        let lane_tag = s
            .amp_cab_focus_lane_tags_by_slot
            .get(&slot_index)
            .map(|(amp_tag, cab_tag)| if cab { *cab_tag } else { *amp_tag })
            .unwrap_or(s.live_write_yy);
        let l = s.live_write_ctr;
        let mut focus = if cab {
            build_amp_cab_ir_cab_focus_packet(&mut s, slot_bus)
        } else {
            build_amp_cab_amp_focus_packet(&mut s, slot_bus)
        };
        patch_amp_cab_tab_focus_lane_tag(&mut focus, lane_tag);
        force_ed03_ctr(&mut focus, l);
        s.live_write_yy = s.live_write_yy.wrapping_add(1);
        s.slot_model_lane_seq = Some(s.live_write_yy);
        s.cab_dual_live_write_tab_focus = None;
        if cab {
            s.amp_cab_cab_focus_sent_for_slot = Some(slot_index);
        } else {
            s.amp_cab_cab_focus_sent_for_slot = None;
        }
        init_trace::trace_fmt(format_args!(
            "amp_cab_tab_focus OUT slot={} bus={:#04x} part={} tag={:#04x} L={:#06x} ed08={:#06x}",
            slot_index,
            slot_bus,
            if cab { "cab" } else { "amp" },
            lane_tag,
            l,
            l.wrapping_add(0x11),
        ));
        s.send(OutPacket::new(focus));
        l
    };

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(delay_ed08));
        let ctr_model = l.wrapping_add(0x11);
        let Ok(mut s) = helix_arc.lock() else {
            return;
        };
        let ed08 = build_ed08_short(&mut s, ctr_model);
        s.send(OutPacket::new(ed08));
        s.live_write_ctr = ctr_model;
        if cab {
            send_cab_dual_cab2_f008_poke(&mut s);
        } else {
            // Capture `ampcab_legacy_switch_tab.json` onglet Amp (#2659) : f0:10 puis f0:08.
            send_cab_dual_f0_poke(&mut s, 0x10);
            thread::sleep(Duration::from_millis(30));
            send_cab_dual_f0_poke(&mut s, 0x08);
        }
    });
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

/// Bloc modèle **Cab single** legacy (`…c2:19`, capture `cab single legacy.json`).
/// Octet `[4]` = `param_selector` wire (pas le tag catalogue).
pub fn build_standalone_legacy_cab_param_model_block(param_selector: u8, slot_bus: u8) -> [u8; 16] {
    [
        0x83, 0x66, 0xcd, 0x04, param_selector, 0x64, 0x28, 0x65, 0x82, 0x62, slot_bus, 0x64,
        0x83, 0x17, 0xc2, 0x19,
    ]
}

/// Mémorise le hint cab (`c2:19` + octets) et le bloc modèle assign depuis le bulk probe.
pub fn record_standalone_legacy_cab_module_field(
    state: &mut HelixState,
    slot_index: u32,
    bulk: &[u8],
) {
    if let Some(field) = crate::helix::edit_slot_model::module_field_bytes_after_c219(bulk) {
        state
            .standalone_legacy_cab_module_field_by_slot
            .insert(slot_index, field);
    }
    if let Some(block) = legacy_assign_model_block_from_bulk(bulk) {
        state
            .standalone_legacy_assign_model_block_by_slot
            .insert(slot_index, block);
    }
}

fn legacy_assign_model_block_from_bulk(bulk: &[u8]) -> Option<[u8; 16]> {
    let pos = bulk
        .windows(3)
        .position(|w| w == [0x83, 0x66, 0xcd])?;
    let slice = bulk.get(pos..pos + 16)?;
    let mut block = [0u8; 16];
    block.copy_from_slice(slice);
    Some(block)
}

/// Assign Soup Pro : `cd:03:ff` — les writes param n'utilisent **pas** de `23` (capture HX Edit).
/// Assign Soup Pro : `cd:03:ff` — handshake `19`/`272`, pas de burst `57` synchrone.
pub(crate) fn standalone_legacy_assign_uses_cd03ff(assign: [u8; 16]) -> bool {
    assign[3] == 0x03 && assign[4] == 0xff
}

/// Vrai si le bloc d'assign est un cab legacy « compact » : champ cab = hint 1 octet
/// (ex. `c2 19 33 1a`), par opposition au MicIr modern (`c2 19 cd 03 1b 1a`, 3 octets).
/// Seul ce cas veut le marqueur discret c2 ; le modern garde c3.
pub fn standalone_legacy_assign_is_one_byte_hint(data: &[u8]) -> bool {
    if let Some(f) = crate::helix::edit_slot_model::module_field_bytes_after_c219(data) {
        return f.len() == 1;
    }
    // Champ cab déjà extrait au probe (`standalone_legacy_cab_module_field_by_slot`).
    data.len() == 1
}

fn standalone_legacy_23_model_block(assign: [u8; 16], param_selector: u8, slot_bus: u8) -> [u8; 16] {
    let mut block = assign;
    block[10] = slot_bus;
    if block[3] == 0x04 {
        block[4] = param_selector;
    }
    block
}

fn standalone_legacy_57_model_prefix(assign: [u8; 16], param_selector: u8, slot_bus: u8) -> [u8; 22] {
    let tag = if assign[3] == 0x04 {
        param_selector
    } else {
        assign[4]
    };
    [
        0x83, 0x66, 0xcd, assign[3], tag, 0x67, 0x00, 0x68, 0x82, 0x0d, slot_bus, 0x18, 0x82,
        0x13, 0x06, 0x14, 0x85, 0x18, 0x83, 0x17, 0xc2, 0x19,
    ]
}

fn standalone_legacy_assign_model_block(
    state: &HelixState,
    slot_index: u32,
) -> Option<[u8; 16]> {
    state
        .standalone_legacy_assign_model_block_by_slot
        .get(&slot_index)
        .copied()
}

pub const C219_BULK_MARKER: [u8; 2] = [0xc2, 0x19];
pub const C319_BULK_MARKER: [u8; 2] = [0xc3, 0x19];

pub fn bulk_has_wire_marker(bulk: &[u8], marker: [u8; 2]) -> bool {
    bulk.windows(marker.len()).any(|w| w == marker)
}

/// Bulk assign Cab dual **legacy hybrid** (`c3:19` + hint court, pas `cd:03…` IR).
pub fn bulk_is_dual_legacy_wire(bulk: &[u8]) -> bool {
    let Some(pos) = bulk
        .windows(C319_BULK_MARKER.len())
        .position(|w| w == C319_BULK_MARKER)
    else {
        return false;
    };
    let Some(tail) = bulk.get(pos + C319_BULK_MARKER.len()..) else {
        return false;
    };
    !tail.first().is_some_and(|&b| b == 0xcd)
}

/// Mémorise les hints cab1/cab2 (`c3:19` … `1a` …) — Cab dual legacy live write.
pub fn record_dual_legacy_cab_module_fields(
    state: &mut HelixState,
    slot_index: u32,
    bulk: &[u8],
) {
    use crate::helix::edit_slot_model::{
        cab_dual_cab1_field_range_in_bulk, cab_dual_cab2_field_range_in_bulk,
    };
    if let Some((start, end)) = cab_dual_cab1_field_range_in_bulk(bulk) {
        state
            .dual_legacy_cab_module_field_by_slot
            .insert((slot_index, 0), bulk[start..end].to_vec());
    }
    if let Some((start, end)) = cab_dual_cab2_field_range_in_bulk(bulk) {
        state
            .dual_legacy_cab_module_field_by_slot
            .insert((slot_index, 1), bulk[start..end].to_vec());
    }
}

/// Met à jour le hint cab2 dual legacy après un replace cab2 seul.
pub fn record_dual_legacy_cab2_module_field(
    state: &mut HelixState,
    slot_index: u32,
    bulk: &[u8],
) {
    if let Some((start, end)) =
        crate::helix::edit_slot_model::cab_dual_cab2_field_range_in_bulk(bulk)
    {
        state
            .dual_legacy_cab_module_field_by_slot
            .insert((slot_index, 1), bulk[start..end].to_vec());
    }
}

pub fn route_is_standalone_legacy_cab(route: &LiveWriteRouteOverride) -> bool {
    route.pp_source.starts_with("legacy_cab:")
}

pub fn route_is_dual_legacy_cab(route: &LiveWriteRouteOverride) -> bool {
    route.pp_source.contains("legacy")
}

fn standalone_legacy_cab_module_field(state: &HelixState, slot_index: u32) -> Option<Vec<u8>> {
    state
        .standalone_legacy_cab_module_field_by_slot
        .get(&slot_index)
        .cloned()
}

fn dual_legacy_cab_module_field(
    state: &HelixState,
    slot_index: u32,
    cab_index: u8,
) -> Option<Vec<u8>> {
    state
        .dual_legacy_cab_module_field_by_slot
        .get(&(slot_index, cab_index))
        .cloned()
}

fn discrete_wire_value_byte(
    raw_norm: f32,
    value_type: Option<i32>,
    chain_min: Option<f64>,
    chain_max: Option<f64>,
    steps: Option<u8>,
    bool_mark: u8,
) -> u8 {
    if let Some(n) = steps {
        if let (Some(lo), Some(hi)) = (chain_min, chain_max) {
            if matches!(value_type, Some(0)) {
                let lo_i = lo.round();
                let hi_i = hi.round();
                if (lo - lo_i).abs() < 1e-6
                    && (hi - hi_i).abs() < 1e-6
                    && hi_i > lo_i
                    && lo_i >= 0.0
                    && hi_i <= 255.0
                {
                    let span = hi_i - lo_i;
                    let v = lo_i + f64::from(raw_norm.clamp(0.0, 1.0)) * span;
                    return v.round().clamp(lo_i, hi_i) as u8;
                }
            }
        }
        let max_i = (n as f32 - 1.0).max(0.0);
        return ((raw_norm.clamp(0.0, 1.0) * max_i).round() as u8).min(n.saturating_sub(1));
    }
    bool_mark
}

fn float_leg_b_from_norm(norm: f32, chain_min: Option<f64>, chain_max: Option<f64>) -> f32 {
    match (chain_min, chain_max) {
        (Some(lo), Some(hi))
            if hi > lo && lo.is_finite() && hi.is_finite() && (hi - lo).is_finite() =>
        {
            let v = lo + f64::from(norm) * (hi - lo);
            if v.is_finite() {
                v as f32
            } else {
                norm
            }
        }
        _ => norm,
    }
}

/// Octet après `1a` dans les trames `23`/`57` legacy — **pas** l'index discret UI.
/// Single : `0xff` (capture `cab single legacy.json`) ; dual : hint 1 o de l'autre cab (`1a 30:00`).
const LEGACY_SINGLE_MARK_AFTER_1A: u8 = 0xff;

fn legacy_mark_byte_after_1a(
    state: &HelixState,
    slot_index: u32,
    dual_cab_index: Option<u8>,
) -> Result<u8, String> {
    match dual_cab_index {
        None => Ok(LEGACY_SINGLE_MARK_AFTER_1A),
        Some(cab_index) => {
            let other = if cab_index == 0 { 1 } else { 0 };
            let field = dual_legacy_cab_module_field(state, slot_index, other).ok_or_else(|| {
                format!(
                    "Cab dual legacy cab{} : hint cab pair introuvable",
                    other + 1
                )
            })?;
            field.first().copied().ok_or_else(|| {
                format!(
                    "Cab dual legacy cab{} : hint cab pair vide",
                    other + 1
                )
            })
        }
    }
}

fn legacy_send_float_followup(
    bool_23: bool,
    disc_steps: Option<u8>,
    value_type: Option<i32>,
) -> bool {
    if !bool_23 {
        return true;
    }
    disc_steps.is_some() || matches!(value_type, Some(0) | Some(1))
}

/// Trame `23` legacy hybrid (`c2:19` ou `c3:19`) — capture `cab single/dual legacy.json` (≠ IR `77`).
fn build_legacy_hybrid_23_packet(
    seq: u8,
    ctr: u16,
    model_block: [u8; 16],
    cab_module: &[u8],
    value_byte: u8,
) -> Vec<u8> {
    let mut p = vec![
        0x23, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x13, 0x00, 0x00, 0x00,
    ];
    p.extend_from_slice(&model_block);
    p.extend_from_slice(cab_module);
    p.push(0x1a);
    p.push(value_byte);
    p.push(0x00);
    p
}

/// Trame `25` (48 o) — hint cab `cd02xx` (capture dual legacy mic long).
fn build_legacy_hybrid_25_packet(
    seq: u8,
    ctr: u16,
    model_block: [u8; 16],
    cab_module: &[u8],
    value_byte: u8,
) -> Vec<u8> {
    let mut p = vec![
        0x25, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x15, 0x00, 0x00, 0x00,
    ];
    p.extend_from_slice(&model_block);
    p.extend_from_slice(cab_module);
    p.push(0x1a);
    p.push(value_byte);
    p.extend_from_slice(&[0x00, 0x00, 0x00]);
    p
}

/// Trame `23` Cab single legacy (44 o) — bloc modèle issu du bulk assign.
fn build_standalone_legacy_23_packet(
    seq: u8,
    ctr: u16,
    model_block: [u8; 16],
    cab_module: &[u8],
    value_byte: u8,
) -> Vec<u8> {
    let mut p = vec![
        0x23, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x13, 0x00, 0x00, 0x00,
    ];
    p.extend_from_slice(&model_block);
    p.extend_from_slice(cab_module);
    p.push(0x1a);
    p.push(value_byte);
    p.push(0x00);
    p
}

/// Trame `25` (48 o) — hint cab `cd02xx` (capture mic long / `cd024d`).
fn build_standalone_legacy_25_packet(
    seq: u8,
    ctr: u16,
    model_block: [u8; 16],
    cab_module: &[u8],
    value_byte: u8,
) -> Vec<u8> {
    let mut p = vec![
        0x25, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x15, 0x00, 0x00, 0x00,
    ];
    p.extend_from_slice(&model_block);
    p.extend_from_slice(cab_module);
    p.push(0x1a);
    p.push(value_byte);
    p.extend_from_slice(&[0x00, 0x00, 0x00]);
    p
}

/// Octet avant `0x0c` en fin de trame `57` — dérivé des scroll IN `cab single legacy.json`.
fn legacy_single_57_tail_prefix_byte(cab_hint: u8) -> u8 {
    match cab_hint {
        0x2d | 0x3f | 0x42 | 0x44 | 0x47 => 0x00,
        0x2e | 0x43 => 0x0b,
        0x2f | 0x30 | 0x3e | 0x45 | 0x46 | 0x48 => 0x01,
        0x32 => 0x0a,
        0x33 | 0x31 | 0x38 | 0x3d | 0x40 => 0x06,
        0x34 => 0x08,
        0x35 => 0x03,
        0x36 | 0x39 | 0x3a | 0x41 => 0x05,
        0x37 | 0x3c => 0x02,
        0x3b => 0x04,
        0x49 | 0x4a => 0x0c,
        _ => 0x06,
    }
}

pub(crate) fn build_legacy_f0_interstitial_packet(state: &mut HelixState) -> Vec<u8> {
    let cnt = state.next_x2_cnt();
    let double = state.firmware_scroll_lane_double();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03, 0x00, cnt, 0x00, 0x08, double[0],
        double[1], 0x00, 0x00,
    ]
}

/// `OUT 1b` avant commit param Soup Pro — capture `split scroll.json` (cd:03:fe).
pub(crate) fn build_standalone_legacy_cd03ff_param_1b(
    state: &mut HelixState,
    assign_block: [u8; 16],
    slot_bus: u8,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x004b);
    let pre_tag = assign_block[4].wrapping_sub(1);
    vec![
        0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd,
        assign_block[3], pre_tag, 0x64, 0x2d, 0x65, 0x81, 0x62, slot_bus, 0x00,
    ]
}

/// `OUT 19` commit param — `cd:03:ff` puis `cd:04:<pSel>` (`split scroll.json`).
pub(crate) fn build_standalone_legacy_cd03ff_param_19(
    state: &mut HelixState,
    assign_block: [u8; 16],
    param_selector: u8,
    second: bool,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x0031);
    let (cd_lane, tag, pre_65) = if second {
        (0x04u8, param_selector, 0x16u8)
    } else {
        (assign_block[3], assign_block[4], 0x17u8)
    };
    vec![
        0x19, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x0c,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd,
        cd_lane, tag, 0x64, pre_65, 0x65, 0xc0, 0x00, 0x00, 0x00,
    ]
}

/// Trame `57` (96 o) — float legacy single (`ed:03:80:10`, capture `cab single legacy.json`).
fn build_standalone_legacy_57_packet(
    seq: u8,
    ctr: u16,
    model_prefix: &[u8],
    cab_module: &[u8],
    mark_after_1a: u8,
    float_be: [u8; 4],
    cab_hint: u8,
) -> Vec<u8> {
    let tail_pre = legacy_single_57_tail_prefix_byte(cab_hint);
    let mut p = vec![
        0x57, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x47, 0x00, 0x00, 0x00,
    ];
    p.extend_from_slice(model_prefix);
    p.extend_from_slice(cab_module);
    p.extend_from_slice(&[
        0x1a, mark_after_1a, 0x09, 0x0f, 0x0a, 0xc3, 0x0b, 0x83, 0x02, 0x06, 0x03, 0x05, 0x04, 0x96, 0xca,
    ]);
    p.extend_from_slice(&float_be);
    p.extend_from_slice(&[
        0x00, 0x00, 0xca, 0x42, 0xa0, 0x00, 0x00, 0xca, 0x45, 0xfa, 0x00, 0x00, 0xca, 0x00, 0x00,
        0x00, 0x00, 0xca, 0x00, 0x00, 0x00, 0x00, tail_pre, 0x0c, 0x83, 0x02, 0x00, 0x03, 0x00, 0x04,
        0x90, 0x00,
    ]);
    p
}

/// Trame `71` (124 o) — float legacy dual (`cab dual legacy.json`, paire après `23`).
fn build_dual_legacy_71_packet(
    seq: u8,
    ctr: u16,
    pp: u8,
    param_selector: u8,
    slot_bus: u8,
    cab_module: &[u8],
    mark_after_1a: u8,
    float_be: [u8; 4],
) -> Vec<u8> {
    let mut p = vec![
        0x71, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x61, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, pp,
        param_selector, 0x67, 0x00, 0x68, 0x82, 0x0d, slot_bus, 0x18, 0x82, 0x13, 0x06, 0x14,
        0x85, 0x18, 0x83, 0x17, 0xc3, 0x19,
    ];
    p.extend_from_slice(cab_module);
    p.extend_from_slice(&[
        0x1a, mark_after_1a, 0x09, 0x10, 0x0a, 0xc3, 0x0b, 0x83, 0x02, 0x06, 0x03, 0x05, 0x04, 0x96, 0xca,
    ]);
    p.extend_from_slice(&float_be);
    p.extend_from_slice(&[
        0x00, 0x00, 0xca, 0x42, 0xa0, 0x00, 0x00, 0xca, 0x45, 0xfa, 0x00, 0x00, 0xca, 0x00, 0x00,
        0x00, 0x00, 0xca, 0x00, 0x00, 0x00, 0x00, 0x06, 0x0c, 0x83, 0x02, 0x06, 0x03, 0x05, 0x04, 0x96, 0xca,
        0x40, 0x80, 0x00, 0x00, 0xca, 0x42, 0xa0, 0x00, 0x00, 0xca, 0x45, 0xfa, 0x00, 0x00, 0xca, 0x00, 0x00,
        0x00, 0x00, 0xca, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
    ]);
    p
}

pub struct StandaloneLegacyMinimalLiveWrite {
    pub packets: Vec<Vec<u8>>,
    pub primary_opcode: u8,
    pub slot_bus: u8,
    pub param_selector: u8,
}

/// Write live minimal Cab **single** legacy — pas de paire `27` IR ni préambule session (`cab single legacy.json`).
pub fn build_standalone_legacy_minimal_param_packets_from_state(
    state: &mut HelixState,
    raw_value: f32,
    slot_index: u32,
    display_type: Option<&str>,
    value_type: Option<i32>,
    chain_min: Option<f64>,
    chain_max: Option<f64>,
    route: LiveWriteRouteOverride,
) -> Result<StandaloneLegacyMinimalLiveWrite, String> {
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or_else(|| "slotIndex invalide".to_string())?;
    let cab_module = standalone_legacy_cab_module_field(state, slot_index)
        .ok_or_else(|| {
            "Cab single legacy : hint cab inconnu (assign legacy requis avant write param)".to_string()
        })?;
    if cab_module.is_empty() {
        return Err("Cab single legacy : champ cab vide".to_string());
    }
    let assign_block = standalone_legacy_assign_model_block(state, slot_index).ok_or_else(|| {
        "Cab single legacy : bloc modèle assign inconnu (assign legacy requis avant write param)"
            .to_string()
    })?;

    let bool_23 = infer_bool_wire_payload(display_type, value_type);
    let disc_steps = discrete_23_step_count(display_type)
        .or_else(|| {
            if matches!(value_type, Some(0)) {
                chain_min
                    .zip(chain_max)
                    .filter(|(lo, hi)| *hi > *lo)
                    .map(|(lo, hi)| (hi - lo).round() as u8 + 1)
            } else {
                None
            }
        });
    let wire_23 = bool_23 || disc_steps.is_some();
    let send_discrete_23 =
        wire_23 && !standalone_legacy_assign_uses_cd03ff(assign_block);
    let mark_after_1a = legacy_mark_byte_after_1a(state, slot_index, None)?;
    let leg_b = float_leg_b_from_norm(raw_value, chain_min, chain_max);
    let float_be = leg_b.to_bits().to_be_bytes();
    let param_selector = route.param_selector;
    let send_float = legacy_send_float_followup(bool_23, disc_steps, value_type);
    let cab_hint = cab_module.first().copied().unwrap_or(0x33);

    if standalone_legacy_assign_uses_cd03ff(assign_block) {
        return Err(
            "Cab single legacy cd:03:ff : utiliser legacy_cab_param_commit (handshake asynchrone)"
                .to_string(),
        );
    }

    let model_23 = standalone_legacy_23_model_block(assign_block, param_selector, slot_bus);
    let model_57 = standalone_legacy_57_model_prefix(assign_block, param_selector, slot_bus);

    let mut packets: Vec<Vec<u8>> = Vec::new();
    let ctr = state.live_write_ctr;
    let ed08_seq = state.next_x80_cnt();
    packets.push(vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, ed08_seq, 0x00, 0x08,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x00,
    ]);

    let mut primary_opcode = 0x57;
    if send_discrete_23 {
        let seq = state.next_x80_cnt();
        let discrete = if cab_module.len() == 3 {
            build_standalone_legacy_25_packet(
                seq,
                ctr,
                model_23,
                &cab_module,
                mark_after_1a,
            )
        } else {
            build_standalone_legacy_23_packet(
                seq,
                ctr,
                model_23,
                &cab_module,
                mark_after_1a,
            )
        };
        primary_opcode = discrete.first().copied().unwrap_or(0x23);
        packets.push(discrete);
    }

    if send_float {
        let seq = state.next_x80_cnt();
        let float_ctr = if send_discrete_23 {
            state.live_write_ctr.wrapping_add(0x1f)
        } else {
            ctr
        };
        let float_pkt = build_standalone_legacy_57_packet(
            seq,
            float_ctr,
            &model_57,
            &cab_module,
            mark_after_1a,
            float_be,
            cab_hint,
        );
        primary_opcode = 0x57;
        packets.push(float_pkt);
        if send_discrete_23 {
            state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
            state.live_write_yy = state.live_write_yy.wrapping_add(1);
        }
    }

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    Ok(StandaloneLegacyMinimalLiveWrite {
        packets,
        primary_opcode,
        slot_bus,
        param_selector,
    })
}

/// Write live minimal Cab **dual** legacy (`c3:19`, capture `cab dual legacy.json`).
pub fn build_dual_legacy_minimal_param_packets_from_state(
    state: &mut HelixState,
    raw_value: f32,
    slot_index: u32,
    cab_index: u8,
    display_type: Option<&str>,
    value_type: Option<i32>,
    chain_min: Option<f64>,
    chain_max: Option<f64>,
    route: LiveWriteRouteOverride,
) -> Result<StandaloneLegacyMinimalLiveWrite, String> {
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)
        .ok_or_else(|| "slotIndex invalide".to_string())?;
    let cab_module = dual_legacy_cab_module_field(state, slot_index, cab_index).ok_or_else(|| {
        format!(
            "Cab dual legacy cab{} : hint cab inconnu (assign dual legacy requis avant write param)",
            cab_index + 1
        )
    })?;
    if cab_module.is_empty() {
        return Err(format!(
            "Cab dual legacy cab{} : champ cab vide",
            cab_index + 1
        ));
    }

    let bool_23 = infer_bool_wire_payload(display_type, value_type);
    let disc_steps = discrete_23_step_count(display_type)
        .or_else(|| {
            if matches!(value_type, Some(0)) {
                chain_min
                    .zip(chain_max)
                    .filter(|(lo, hi)| *hi > *lo)
                    .map(|(lo, hi)| (hi - lo).round() as u8 + 1)
            } else {
                None
            }
        });
    let wire_23 = bool_23 || disc_steps.is_some();
    let mark_after_1a = legacy_mark_byte_after_1a(state, slot_index, Some(cab_index))?;
    let leg_b = float_leg_b_from_norm(raw_value, chain_min, chain_max);
    let float_be = leg_b.to_bits().to_be_bytes();
    let param_selector = route.param_selector;
    let model_block = route.model_block;
    let pp = route.pp;
    let send_float = legacy_send_float_followup(bool_23, disc_steps, value_type);
    if !wire_23 && !send_float {
        return Err(
            "Cab dual legacy : write float non implémenté (capture manquante)".to_string(),
        );
    }

    let mut packets: Vec<Vec<u8>> = Vec::new();
    let ctr = state.live_write_ctr;
    let ed08_seq = state.next_x80_cnt();
    packets.push(vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, ed08_seq, 0x00, 0x08,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x00,
    ]);

    let mut primary_opcode = 0x71;
    if wire_23 {
        let seq = state.next_x80_cnt();
        let discrete = if cab_module.len() == 3 {
            build_legacy_hybrid_25_packet(seq, ctr, model_block, &cab_module, mark_after_1a)
        } else {
            build_legacy_hybrid_23_packet(seq, ctr, model_block, &cab_module, mark_after_1a)
        };
        primary_opcode = discrete.first().copied().unwrap_or(0x23);
        packets.push(discrete);
    }

    if send_float {
        let seq = state.next_x80_cnt();
        let float_ctr = if wire_23 {
            state.live_write_ctr.wrapping_add(0x1f)
        } else {
            ctr
        };
        let float_pkt = build_dual_legacy_71_packet(
            seq,
            float_ctr,
            pp,
            param_selector,
            slot_bus,
            &cab_module,
            mark_after_1a,
            float_be,
        );
        primary_opcode = 0x71;
        packets.push(float_pkt);
        if wire_23 {
            state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
            state.live_write_yy = state.live_write_yy.wrapping_add(1);
        }
    }

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    Ok(StandaloneLegacyMinimalLiveWrite {
        packets,
        primary_opcode,
        slot_bus,
        param_selector,
    })
}

/// Route live write mono quand un bulk **legacy** (`c2:19`) a été assigné sur ce slot (probe).
/// Réutilise le bloc modèle du bulk — pas de relecture `preset_data`, pas de focus Amp+Cab.
pub fn resolve_standalone_legacy_cab_live_write_route_from_probe(
    state: &HelixState,
    local_param_index: u32,
    slot_index: u32,
) -> Option<LiveWriteRouteOverride> {
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)?;
    let assign = state
        .standalone_legacy_assign_model_block_by_slot
        .get(&slot_index)
        .copied()?;
    let (param_selector, _tag) = legacy_cab_wire_pair(local_param_index, 0)?;
    let cache_key = echo_model_cache_key(slot_bus, assign[3], param_selector);
    if let Some(block) = state.ed03_echo_model_by_slot_param.get(&cache_key) {
        return Some(LiveWriteRouteOverride {
            pp: block[3],
            pp_source: "legacy_cab:echo_cache",
            param_selector,
            param_selector_source: "legacy_cab:echo_sel",
            model_block: *block,
            preserve_model_tag: true,
            discrete_wants_c2: false,
        });
    }
    let model_block = standalone_legacy_23_model_block(assign, param_selector, slot_bus);
    Some(LiveWriteRouteOverride {
        pp: model_block[3],
        pp_source: "legacy_cab:assign_probe",
        param_selector,
        param_selector_source: "legacy_cab:compact_sel",
        model_block,
        preserve_model_tag: true,
        discrete_wants_c2: false,
    })
}

pub fn resolve_standalone_legacy_cab_live_write_route(
    state: &HelixState,
    local_param_index: u32,
    slot_index: u32,
) -> Option<LiveWriteRouteOverride> {
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)?;
    let (param_selector, _tag) = legacy_cab_wire_pair(local_param_index, 0)?;
    let cache_key = echo_model_cache_key(slot_bus, 0x04, param_selector);
    if let Some(block) = state.ed03_echo_model_by_slot_param.get(&cache_key) {
        return Some(LiveWriteRouteOverride {
            pp: block[3],
            pp_source: "legacy_cab:echo_cache",
            param_selector,
            param_selector_source: "legacy_cab:echo_sel",
            model_block: *block,
            preserve_model_tag: true,
            discrete_wants_c2: false,
        });
    }
    let model_block = build_standalone_legacy_cab_param_model_block(param_selector, slot_bus);
    Some(LiveWriteRouteOverride {
        pp: 0x04,
        pp_source: "legacy_cab:standalone_compact",
        param_selector,
        param_selector_source: "legacy_cab:compact_sel",
        model_block,
        preserve_model_tag: true,
        discrete_wants_c2: false,
    })
}

pub fn resolve_cab_live_write_route(
    state: &HelixState,
    local_param_index: u32,
    assign_variant: &str,
    slot_index: u32,
    amp_param_count: Option<u32>,
) -> Option<LiveWriteRouteOverride> {
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)?;
    let legacy = is_legacy_variant(assign_variant);
    let amp_block_len = amp_param_count.unwrap_or(0) as usize;

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
                discrete_wants_c2: false,
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
            discrete_wants_c2: false,
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
            discrete_wants_c2: false,
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
        discrete_wants_c2: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amp_cab_tab_focus_amp_uses_stored_amp_lane_tag() {
        use crate::helix::HelixState;
        let helix = std::sync::Arc::new(std::sync::Mutex::new(HelixState::new()));
        {
            let mut s = helix.lock().unwrap();
            s.live_write_yy = 0xfe;
            s.amp_cab_focus_lane_tags_by_slot
                .insert(3, (0xfb, 0xf9));
        }
        crate::helix::amp_cab_live_write::spawn_amp_cab_tab_focus_usb(helix.clone(), 3, 0x01, false);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let s = helix.lock().unwrap();
        // Dernier paquet 1d envoyé : tag ampli = fb (pas fe live_write_yy ni f9 cab).
        // On vérifie via le builder direct pour éviter la course avec le thread ed:08.
        drop(s);
        let mut state = HelixState::new();
        state.live_write_yy = 0xfe;
        state
            .amp_cab_focus_lane_tags_by_slot
            .insert(3, (0xfb, 0xf9));
        let mut pkt = build_amp_cab_amp_focus_packet(&mut state, 0x01);
        patch_amp_cab_tab_focus_lane_tag(&mut pkt, 0xfb);
        assert_eq!(pkt[28], 0xfb);
        assert_eq!(pkt[36], 0x00, "onglet Amp");
    }

    #[test]
    fn record_assign_session_reads_cd07_lane_tag() {
        let mut state = HelixState::new();
        let bulk = [
            0x23, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x33, 0x00, 0x04, 0xd1, 0xa7,
            0x02, 0x00, 0x01, 0x00, 0x06, 0x00, 0x13, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x07,
            0xfb, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17, 0xc3, 0x19, 0x2c, 0x1a,
            0x47, 0x00,
        ];
        record_amp_cab_assign_session(&mut state, 0, &bulk);
        assert_eq!(
            state.amp_cab_focus_lane_tags_by_slot.get(&0),
            Some(&(0xfb, 0xfb))
        );
    }

    #[test]
    fn ir_cab_level_is_sel_00_pp_03() {
        use crate::helix::HelixState;
        let state = HelixState::new();
        let route = resolve_cab_live_write_route(&state, 0, "amp+cab", 3, Some(12)).expect("route");
        assert_eq!(route.pp, 0x03);
        assert_eq!(route.param_selector, 0x00);
        assert_eq!(route.model_block[3], 0x03);
        assert_eq!(&route.model_block[11..16], &[0x1d, 0xc3, 0x1a, 0x01, 0x1c]);
    }

    #[test]
    fn amp_cab_tab_focus_cab_uses_1d_cd03_1a01() {
        use crate::helix::HelixState;
        let mut state = HelixState::new();
        let pkt = build_amp_cab_ir_cab_focus_packet(&mut state, 0x01);
        assert_eq!(pkt.first(), Some(&0x1d));
        assert_eq!(&pkt[24..28], &[0x83, 0x66, 0xcd, 0x03]);
        assert_eq!(pkt[35], 0x1a);
        assert_eq!(pkt[36], 0x01);
    }

    #[test]
    fn amp_cab_tab_focus_amp_uses_1d_cd03_1a00() {
        use crate::helix::HelixState;
        let mut state = HelixState::new();
        let pkt = build_amp_cab_amp_focus_packet(&mut state, 0x01);
        assert_eq!(pkt.first(), Some(&0x1d));
        assert_eq!(&pkt[24..28], &[0x83, 0x66, 0xcd, 0x03]);
        assert_eq!(pkt[35], 0x1a);
        assert_eq!(pkt[36], 0x00);
    }

    #[test]
    fn legacy_guitar_cab_level_is_sel_25_tag_05() {
        let pair = legacy_cab_wire_pair(0, 21).expect("route");
        assert_eq!(pair, (0x25, 0x05));
    }

    #[test]
    fn standalone_legacy_param0_is_sel_00_byte4() {
        use crate::helix::HelixState;
        let state = HelixState::new();
        let route =
            resolve_standalone_legacy_cab_live_write_route(&state, 0, 3).expect("route");
        assert_eq!(route.pp, 0x04);
        assert_eq!(route.param_selector, 0x00);
        assert_eq!(route.model_block[4], 0x00);
        assert_eq!(&route.model_block[14..16], &[0xc2, 0x19]);
    }

    #[test]
    fn standalone_legacy_23_packet_matches_capture_tail() {
        let assign = [
            0x83, 0x66, 0xcd, 0x04, 0x00, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17,
            0xc2, 0x19,
        ];
        let p = build_standalone_legacy_23_packet(0x48, 0x6f0a, assign, &[0x34], 0xff);
        assert_eq!(p.len(), 44);
        assert_eq!(
            &p[24..44],
            &[
                0x83, 0x66, 0xcd, 0x04, 0x00, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17,
                0xc2, 0x19, 0x34, 0x1a, 0xff, 0x00,
            ]
        );
    }

    #[test]
    fn standalone_legacy_soup_pro_57_tail_matches_capture() {
        let assign = [
            0x83, 0x66, 0xcd, 0x03, 0xff, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17,
            0xc2, 0x19,
        ];
        let prefix = standalone_legacy_57_model_prefix(assign, 0x00, 0x01);
        let p = build_standalone_legacy_57_packet(
            0x98,
            0x0401,
            &prefix,
            &[0x33],
            0xff,
            0x40200000u32.to_be_bytes(),
            0x33,
        );
        assert_eq!(p.len(), 98);
        assert_eq!(
            &p[p.len() - 10..],
            &[0x06, 0x0c, 0x83, 0x02, 0x00, 0x03, 0x00, 0x04, 0x90, 0x00]
        );
    }

    #[test]
    fn standalone_legacy_probe_route_uses_assign_block_not_ir_static() {
        let mut state = HelixState::new();
        state.standalone_legacy_assign_model_block_by_slot.insert(
            0,
            [
                0x83, 0x66, 0xcd, 0x03, 0xff, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83,
                0x17, 0xc2, 0x19,
            ],
        );
        let route =
            resolve_standalone_legacy_cab_live_write_route_from_probe(&state, 0, 0).expect("route");
        assert_eq!(route.pp_source, "legacy_cab:assign_probe");
        assert_eq!(route.pp, 0x03);
        assert_eq!(route.param_selector, 0x00);
        assert_eq!(route.model_block[14], 0xc2);
        assert_eq!(route.model_block[15], 0x19);
        assert_eq!(route.model_block[10], 0x01);
    }

    #[test]
    fn standalone_legacy_cd03ff_uses_async_commit_not_minimal_burst() {
        let mut state = HelixState::new();
        state
            .standalone_legacy_cab_module_field_by_slot
            .insert(0, vec![0x33]);
        state.standalone_legacy_assign_model_block_by_slot.insert(
            0,
            [
                0x83, 0x66, 0xcd, 0x03, 0xff, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83,
                0x17, 0xc2, 0x19,
            ],
        );
        let route = resolve_standalone_legacy_cab_live_write_route(&state, 0, 0).expect("route");
        let err = build_standalone_legacy_minimal_param_packets_from_state(
            &mut state,
            13.0 / 15.0,
            0,
            Some("mic"),
            Some(0),
            Some(0.0),
            Some(15.0),
            route,
        )
        .err()
        .expect("cd03ff ne doit pas passer par minimal burst");
        assert!(err.contains("legacy_cab_param_commit"));
    }

    #[test]
    fn standalone_legacy_soup_pro_57_keeps_cd03ff() {
        let assign = [
            0x83, 0x66, 0xcd, 0x03, 0xff, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17,
            0xc2, 0x19,
        ];
        assert!(standalone_legacy_assign_uses_cd03ff(assign));
        let prefix = standalone_legacy_57_model_prefix(assign, 0x00, 0x01);
        assert_eq!(&prefix[0..5], &[0x83, 0x66, 0xcd, 0x03, 0xff]);
    }

    #[test]
    fn standalone_legacy_one_byte_hint_detects_compact_cab() {
        let bulk = [0xc2, 0x19, 0x33, 0x1a];
        assert!(standalone_legacy_assign_is_one_byte_hint(&bulk));
        assert!(standalone_legacy_assign_is_one_byte_hint(&[0x33]));
    }

    #[test]
    fn standalone_legacy_one_byte_hint_rejects_mic_ir_modern() {
        let bulk = [0xc2, 0x19, 0xcd, 0x03, 0x1b, 0x1a];
        assert!(!standalone_legacy_assign_is_one_byte_hint(&bulk));
        assert!(!standalone_legacy_assign_is_one_byte_hint(&[0xcd, 0x03, 0x1b]));
    }

    #[test]
    fn standalone_legacy_minimal_needs_cab_hint() {
        let mut state = HelixState::new();
        let route = resolve_standalone_legacy_cab_live_write_route(&state, 0, 0).expect("route");
        let err = build_standalone_legacy_minimal_param_packets_from_state(
            &mut state,
            0.5,
            0,
            Some("mic"),
            Some(0),
            Some(0.0),
            Some(15.0),
            route,
        )
        .err()
        .expect("hint cab");
        assert!(err.contains("hint cab"));
    }

    #[test]
    fn dual_legacy_wire_detects_short_c319_hints() {
        let ir_dual = [
            0x83u8, 0x18, 0x83, 0x17, 0xc3, 0x19, 0xcd, 0x03, 0x1c, 0x1a, 0xcd, 0x02, 0xd6,
        ];
        let legacy_dual = [
            0x64, 0x83, 0x17, 0xc3, 0x19, 0x33, 0x1a, 0x30, 0x00,
        ];
        assert!(!bulk_is_dual_legacy_wire(&ir_dual));
        assert!(bulk_is_dual_legacy_wire(&legacy_dual));
    }

    #[test]
    fn dual_legacy_23_packet_matches_capture_tail() {
        let model = build_amp_cab_legacy_param_model_block(0x04, 0x2b, 0x01);
        let p = build_legacy_hybrid_23_packet(0x48, 0x6e31, model, &[0x33], 0x30);
        assert_eq!(p.len(), 44);
        assert_eq!(
            &p[24..44],
            &[
                0x83, 0x66, 0xcd, 0x04, 0x2b, 0x64, 0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83,
                0x17, 0xc3, 0x19, 0x33, 0x1a, 0x30, 0x00,
            ]
        );
    }

    #[test]
    fn legacy_mark_single_is_ff_dual_is_peer_hint() {
        let mut state = HelixState::new();
        state
            .dual_legacy_cab_module_field_by_slot
            .insert((0, 0), vec![0x34]);
        state
            .dual_legacy_cab_module_field_by_slot
            .insert((0, 1), vec![0x30]);
        assert_eq!(legacy_mark_byte_after_1a(&state, 0, None).expect("single"), 0xff);
        assert_eq!(
            legacy_mark_byte_after_1a(&state, 0, Some(0)).expect("cab1"),
            0x30
        );
        assert_eq!(
            legacy_mark_byte_after_1a(&state, 0, Some(1)).expect("cab2"),
            0x34
        );
    }

    #[test]
    fn dual_legacy_minimal_needs_cab_hint() {
        use crate::helix::cab_dual_live_write::resolve_cab_dual_live_write_route;
        let state = HelixState::new();
        let route =
            resolve_cab_dual_live_write_route(&state, 1, 0, 0, true).expect("route");
        let mut state = HelixState::new();
        let err = build_dual_legacy_minimal_param_packets_from_state(
            &mut state,
            0.5,
            0,
            1,
            Some("mic"),
            Some(0),
            Some(0.0),
            Some(15.0),
            route,
        )
        .err()
        .expect("hint");
        assert!(err.contains("hint cab"));
    }
}
