//! Write live **paramètre** (une paire de trames après pré `08` / `x2`), aligné captures HX Edit.
//!
//! - **Unifié ici** : plus d’envoi bool en `27`+float (`77 ca`) — remplacé par `23`+`77 c2`/`c3`
//!   (voir `HelixLiveWrite.json` + `infer_bool_wire_payload`).
//! - **Discrets multi-positions** (ex. `comp_ratio`) : même trame `23` que le bool, mais octet après `77` = index
//!   `0..n-1` (`discrete23DisplayTypes`, captures `Modif Ratio.json`).
//! - **PP** : défaut depuis `HelixLiveWrite.ppDefault` (plus d’heuristique `param_index` → `0x04` / `Tone`).
//! - **Autres flux USB** (changement de modèle dans un slot, etc.) : `edit_slot_model.rs` — autre commande, pas ce module.
//! - **Sécurité** : avant envoi, `validate_usb_live_write_metadata` refuse les `valueType` non listés pour le float `0x27`
//!   (`HelixLiveWrite.allowedFloatValueTypes`) — les bool connus passent en `0x23` sans cette contrainte.
//! - **Float `27` paire `04`/`0c`** : jambe A = normalisé 0…1 (IEEE) ; jambe B = si `chain_min`/`chain_max` (.models) fournis
//!   et `max > min`, valeur physique `min + norm×(max−min)` (captures HX Level : ~0,32 norm ↔ ~−37 dB).

use crate::helix::live_write_config::{
    discrete_23_step_count, infer_bool_wire_payload, live_write_cfg, pp_override_for_display_type,
};
use crate::helix::{kempline_index_to_slot_bus, HelixState};

#[derive(Clone)]
pub struct LiveWriteRouteOverride {
    pub pp: u8,
    pub pp_source: &'static str,
    pub param_selector: u8,
    pub param_selector_source: &'static str,
    pub model_block: [u8; 16],
    /// Si vrai, ne pas écraser l'octet tag (index 4) avec la séquence live write.
    pub preserve_model_tag: bool,
}

pub struct LiveWriteFrames {
    pub model_block_kind: &'static str,
    /// Premier octet de la paire principale : `0x27` (float) ou `0x23` (bool ou discret indexé HX Edit).
    pub primary_opcode: u8,
    /// Octet sous `85:62:XX:1d` dans la trame : captures → `XX` = bus slot 1..16.
    pub slot_bus: u8,
    pub pp: u8,
    pub pp_source: &'static str,
    pub param_selector: u8,
    pub param_selector_source: &'static str,
    pub pre_packet_x80: Vec<u8>,
    pub pre_packet_x2: Vec<u8>,
    pub pre_packet_x80_sel: Vec<u8>,
    /// Première jambe : octet 11 = `0x04`.
    pub packet_27: Vec<u8>,
    /// Deuxième jambe : octet 11 = `0x0c`.
    pub packet_27_b: Vec<u8>,
    pub post_packet_x80_sel: Vec<u8>,
    pub frame27_diff_vs_static: String,
}

fn pp_for_live_write(display_type: Option<&str>) -> (u8, &'static str) {
    if let Some(p) = pp_override_for_display_type(display_type) {
        return (p, "config:HelixLiveWrite.ppByDisplayType");
    }
    let p = live_write_cfg().pp_default;
    (p, "config:HelixLiveWrite.ppDefault")
}

fn param_selector_byte_from_index(param_index: u32) -> (u8, &'static str) {
    ((param_index.min(0xff)) as u8, "index_to_offset40")
}

fn slot_bus_byte_from_kempline_index(slot_index: u32) -> u8 {
    kempline_index_to_slot_bus(slot_index.min(15) as usize).unwrap_or(1)
}

/// Trame `27` write opcode `80:10:ed:03` (48 octets) — valeur float IEEE BE après `77 ca`.
fn assemble_27_write(
    seq: u8,
    byte11: u8,
    ctr: u16,
    yy: u8,
    pp: u8,
    param_selector: u8,
    slot_bus: u8,
    float_be: [u8; 4],
) -> Vec<u8> {
    vec![
        0x27, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, byte11,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x00,
        0x01, 0x00, 0x06, 0x00, 0x17, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, 0x1e, 0x65,
        0x85, 0x62, slot_bus, 0x1d, 0xc3, 0x1a, 0x00, 0x1c,
        param_selector, 0x77, 0xca, float_be[0], float_be[1], float_be[2], float_be[3], 0x00,
    ]
}

/// Trame `23` (44 octets) — bool (`c2`/`c3`) ou discret (`00`..`n-1`) observé HX Edit.
fn assemble_23_bool_write(
    seq: u8,
    byte11: u8,
    ctr: u16,
    yy: u8,
    pp: u8,
    param_selector: u8,
    slot_bus: u8,
    bool_mark: u8,
) -> Vec<u8> {
    vec![
        0x23, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, byte11,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x00,
        0x01, 0x00, 0x06, 0x00, 0x13, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, 0x1e, 0x65,
        0x85, 0x62, slot_bus, 0x1d, 0xc3, 0x1a, 0x00, 0x1c,
        param_selector, 0x77, bool_mark, 0x00,
    ]
}

fn apply_model_block(
    packet: &mut [u8],
    state: &mut HelixState,
    mut model_block: [u8; 16],
    advance_seq_on_tag: bool,
) {
    if advance_seq_on_tag {
        let next_seq = match state.ed03_live_write_seq_sent {
            Some(prev) => prev.wrapping_add(1),
            None => model_block[4].wrapping_add(1),
        };
        model_block[4] = next_seq;
        state.ed03_live_write_seq_sent = Some(next_seq);
    }
    packet[24..40].copy_from_slice(&model_block);
}

fn apply_echo_model_block(packet: &mut [u8], state: &mut HelixState, last_echo: [u8; 16]) {
    apply_model_block(packet, state, last_echo, true);
}

fn finalize_primary_packet(
    packet: &mut [u8],
    primary_opcode: u8,
    slot_bus: u8,
    param_selector: u8,
    float_be: [u8; 4],
    bool_mark: u8,
) {
    if packet.len() < 40 {
        return;
    }
    packet[34] = slot_bus;
    packet[40] = param_selector;
    match primary_opcode {
        0x23 if packet.len() >= 44 => {
            packet[41] = 0x77;
            packet[42] = bool_mark;
            packet[43] = 0x00;
        }
        _ if packet.len() >= 48 => {
            packet[41] = 0x77;
            packet[42] = 0xca;
            packet[43..47].copy_from_slice(&float_be);
            packet[47] = 0x00;
        }
        _ => {}
    }
}

/// Deuxième float des trames `27` : unités chaîne Line 6 quand la plage est connue, sinon même que la norme.
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

fn raw_to_discrete_index(raw: f32, steps: u8) -> u8 {
    let n = steps as f32;
    if n <= 1.0 {
        return 0;
    }
    let max_i = n - 1.0;
    let i = (raw.clamp(0.0, 1.0) * max_i).round() as i32;
    i.clamp(0, max_i as i32) as u8
}

/// Index discret wire : entiers catalogue (`valueType=0`, min/max entiers) → valeur physique
/// (captures `cabMICir` : octet après `77` = index mic 0..11, pas une norme 0..1 brute).
fn discrete_wire_index(
    raw_norm: f32,
    value_type: Option<i32>,
    chain_min: Option<f64>,
    chain_max: Option<f64>,
    steps: u8,
) -> u8 {
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
    raw_to_discrete_index(raw_norm, steps)
}

/// Règle générique discrète: pour `valueType=0` avec bornes entières (`min..max`),
/// on déduit `N = max - min + 1` positions (ex. 0..5 => 6).
/// Utilisée en fallback quand `displayType` n'est pas explicitement mappé.
fn inferred_discrete_steps_from_bounds(
    value_type: Option<i32>,
    chain_min: Option<f64>,
    chain_max: Option<f64>,
) -> Option<u8> {
    if !matches!(value_type, Some(0)) {
        return None;
    }
    let (lo, hi) = match (chain_min, chain_max) {
        (Some(lo), Some(hi)) if lo.is_finite() && hi.is_finite() && hi >= lo => (lo, hi),
        _ => return None,
    };
    let lo_i = lo.round();
    let hi_i = hi.round();
    if (lo - lo_i).abs() > 1e-6 || (hi - hi_i).abs() > 1e-6 {
        return None;
    }
    let span = (hi_i as i64) - (lo_i as i64);
    if span < 1 || span > 254 {
        return None;
    }
    Some((span + 1) as u8)
}

/// Construit les trames de write live (paire `04` / `0c`, pré/post `08` comme les captures HX Edit).
pub fn build_live_write_frames_from_state(
    state: &mut HelixState,
    raw_value: f32,
    slot_index: u32,
    param_index: u32,
    _symbolic_id: &str,
    display_type: Option<&str>,
    value_type: Option<i32>,
    chain_min: Option<f64>,
    chain_max: Option<f64>,
    route_override: Option<LiveWriteRouteOverride>,
) -> LiveWriteFrames {
    let cfg = live_write_cfg();
    let bool_23 = infer_bool_wire_payload(display_type, value_type);
    let disc_steps = discrete_23_step_count(display_type)
        .or_else(|| inferred_discrete_steps_from_bounds(value_type, chain_min, chain_max));
    let wire_23 = bool_23 || disc_steps.is_some();
    let mark_23: u8 = if bool_23 {
        if raw_value >= 0.5 {
            cfg.bool_mark_on
        } else {
            cfg.bool_mark_off
        }
    } else if let Some(n) = disc_steps {
        discrete_wire_index(raw_value, value_type, chain_min, chain_max, n)
    } else {
        0
    };

    let float_be_a = raw_value.to_bits().to_be_bytes();
    let leg_b = float_leg_b_from_norm(raw_value, chain_min, chain_max);
    let float_be_b = leg_b.to_bits().to_be_bytes();
    let slot_bus = slot_bus_byte_from_kempline_index(slot_index);
    let pre_cnt_x80 = state.next_x80_cnt();
    let pre_cnt_x2 = state.next_x2_cnt();
    let pre_session = state.session_no;
    let pre_double = state.preset_data_packet_double();

    let pre_packet_x80 = vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, pre_cnt_x80, 0x00, 0x10, pre_session, pre_double[0], pre_double[1], 0x00,
    ];
    let scroll_double = state.firmware_scroll_lane_double();
    let pre_packet_x2 = vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03,
        0x00, pre_cnt_x2, 0x00, 0x10, scroll_double[0], scroll_double[1], 0x00, 0x00,
    ];

    let (pp, pp_source) = if let Some(ref r) = route_override {
        (r.pp, r.pp_source)
    } else {
        pp_for_live_write(display_type)
    };
    let (param_selector, param_selector_source) = if let Some(ref r) = route_override {
        (r.param_selector, r.param_selector_source)
    } else {
        param_selector_byte_from_index(param_index)
    };

    let seq_sel = state.next_x80_cnt();
    let ctr_a = state.live_write_ctr;
    let pre_packet_x80_sel = vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq_sel, 0x00, 0x08, (ctr_a & 0xff) as u8, ((ctr_a >> 8) & 0xff) as u8, 0x00, 0x00,
    ];

    let primary_opcode: u8 = if wire_23 { 0x23 } else { 0x27 };
    let seq_a = state.next_x80_cnt();
    let yy_a = state.live_write_yy;

    let mut packet_a = if wire_23 {
        assemble_23_bool_write(seq_a, 0x04, ctr_a, yy_a, pp, param_selector, slot_bus, mark_23)
    } else {
        assemble_27_write(
            seq_a,
            0x04,
            ctr_a,
            yy_a,
            pp,
            param_selector,
            slot_bus,
            float_be_a,
        )
    };

    let route_ref = route_override.as_ref();

    let (model_block_kind, _) = if let Some(r) = route_ref {
        apply_model_block(
            &mut packet_a,
            state,
            r.model_block,
            !r.preserve_model_tag,
        );
        finalize_primary_packet(
            &mut packet_a,
            primary_opcode,
            slot_bus,
            param_selector,
            float_be_a,
            mark_23,
        );
        ("amp_cab_cab_route", ())
    } else if let Some(last_echo) = state.last_ed03_echo_model {
        apply_echo_model_block(&mut packet_a, state, last_echo);
        finalize_primary_packet(
            &mut packet_a,
            primary_opcode,
            slot_bus,
            param_selector,
            float_be_a,
            mark_23,
        );
        ("in_echo_strict", ())
    } else {
        ("replay_static", ())
    };

    if model_block_kind == "replay_static" {
        if wire_23 {
            finalize_primary_packet(
                &mut packet_a,
                primary_opcode,
                slot_bus,
                param_selector,
                float_be_a,
                mark_23,
            );
        } else {
            packet_a[43..47].copy_from_slice(&float_be_a);
        }
    }

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    let seq_b = state.next_x80_cnt();
    let ctr_b = state.live_write_ctr;
    let yy_b = state.live_write_yy;

    let mut packet_b = if wire_23 {
        assemble_23_bool_write(seq_b, 0x0c, ctr_b, yy_b, pp, param_selector, slot_bus, mark_23)
    } else {
        assemble_27_write(
            seq_b,
            0x0c,
            ctr_b,
            yy_b,
            pp,
            param_selector,
            slot_bus,
            float_be_b,
        )
    };

    if let Some(r) = route_ref {
        apply_model_block(
            &mut packet_b,
            state,
            r.model_block,
            !r.preserve_model_tag,
        );
        finalize_primary_packet(
            &mut packet_b,
            primary_opcode,
            slot_bus,
            param_selector,
            float_be_b,
            mark_23,
        );
    } else if let Some(last_echo) = state.last_ed03_echo_model {
        apply_echo_model_block(&mut packet_b, state, last_echo);
        finalize_primary_packet(
            &mut packet_b,
            primary_opcode,
            slot_bus,
            param_selector,
            float_be_b,
            mark_23,
        );
    } else if wire_23 {
        finalize_primary_packet(
            &mut packet_b,
            primary_opcode,
            slot_bus,
            param_selector,
            float_be_b,
            mark_23,
        );
    } else {
        packet_b[43..47].copy_from_slice(&float_be_b);
    }

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    let seq_post_sel = state.next_x80_cnt();
    let ctr_post = state.live_write_ctr;
    let post_packet_x80_sel = vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq_post_sel, 0x00, 0x08, (ctr_post & 0xff) as u8, ((ctr_post >> 8) & 0xff) as u8, 0x00, 0x00,
    ];

    let frame27_diff_vs_static = if wire_23 {
        let static_ref = assemble_23_bool_write(
            0x8f,
            0x04,
            0x6cbd,
            0x17,
            pp,
            param_selector,
            slot_bus,
            mark_23,
        );
        diff_packet_hex(&static_ref, &packet_a)
    } else {
        let static_ref = assemble_27_write(
            0x8f,
            0x04,
            0x6cbd,
            0x17,
            pp,
            param_selector,
            slot_bus,
            float_be_a,
        );
        diff_packet_hex(&static_ref, &packet_a)
    };

    LiveWriteFrames {
        model_block_kind,
        primary_opcode,
        slot_bus,
        pp,
        pp_source,
        param_selector,
        param_selector_source,
        pre_packet_x80,
        pre_packet_x2,
        pre_packet_x80_sel,
        packet_27: packet_a,
        packet_27_b: packet_b,
        post_packet_x80_sel,
        frame27_diff_vs_static,
    }
}

fn diff_packet_hex(reference: &[u8], actual: &[u8]) -> String {
    let max = reference.len().min(actual.len());
    let mut diffs = Vec::new();
    for i in 0..max {
        if reference[i] != actual[i] {
            diffs.push(format!("{i}:{:02x}->{:02x}", reference[i], actual[i]));
        }
    }
    if reference.len() != actual.len() {
        diffs.push(format!("len:{}->{}", reference.len(), actual.len()));
    }
    if diffs.is_empty() {
        return "none".to_string();
    }
    const MAX_ITEMS: usize = 12;
    if diffs.len() > MAX_ITEMS {
        let remaining = diffs.len() - MAX_ITEMS;
        diffs.truncate(MAX_ITEMS);
        diffs.push(format!("+{} more", remaining));
    }
    diffs.join(",")
}

/// Résultat write live **minimal** Cab dual IR — captures `add_dual_cab_modif_param_cab2.json` :
/// `08` sub=`08` (armement, défaut) + `23` ou paire `27` `04`/`0c` (pas de pré `08:10` / `f0:03`).
pub struct CabDualMinimalLiveWrite {
    pub packets: Vec<Vec<u8>>,
    pub primary_opcode: u8,
    pub slot_bus: u8,
    pub pp: u8,
    pub pp_source: &'static str,
    pub param_selector: u8,
    pub param_selector_source: &'static str,
    pub model_block_kind: &'static str,
}

/// `HX_CAB_DUAL_PARAM_ED08` (défaut ON) : arme chaque écriture param cab dual d'un `ed:08`
/// (octet 11 = `08`) AU MÊME ctr que la jambe A — HX Edit envoie un `08` avant chaque `23`/`27`
/// (capture `add_dual_cab_modif_param_cab2.json`). `=0` restaure l'ancien comportement (A/B).
pub fn cab_dual_param_ed08_enabled() -> bool {
    !matches!(
        std::env::var("HX_CAB_DUAL_PARAM_ED08")
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

/// Construit les trames param Cab dual (HX Edit : `08` armement + 1×`23` discret ou paire `27`).
pub fn build_cab_dual_minimal_param_packets_from_state(
    state: &mut HelixState,
    raw_value: f32,
    slot_index: u32,
    display_type: Option<&str>,
    value_type: Option<i32>,
    chain_min: Option<f64>,
    chain_max: Option<f64>,
    route: LiveWriteRouteOverride,
) -> CabDualMinimalLiveWrite {
    let cfg = live_write_cfg();
    let bool_23 = infer_bool_wire_payload(display_type, value_type);
    let disc_steps = discrete_23_step_count(display_type)
        .or_else(|| inferred_discrete_steps_from_bounds(value_type, chain_min, chain_max));
    let wire_23 = bool_23 || disc_steps.is_some();
    let mark_23: u8 = if bool_23 {
        if raw_value >= 0.5 {
            cfg.bool_mark_on
        } else {
            cfg.bool_mark_off
        }
    } else if let Some(n) = disc_steps {
        discrete_wire_index(raw_value, value_type, chain_min, chain_max, n)
    } else {
        0
    };

    let float_be_a = raw_value.to_bits().to_be_bytes();
    let leg_b = float_leg_b_from_norm(raw_value, chain_min, chain_max);
    let float_be_b = leg_b.to_bits().to_be_bytes();
    let slot_bus = slot_bus_byte_from_kempline_index(slot_index);
    let pp = route.pp;
    let pp_source = route.pp_source;
    let param_selector = route.param_selector;
    let param_selector_source = route.param_selector_source;
    let primary_opcode: u8 = if wire_23 { 0x23 } else { 0x27 };

    let arm_ed08 = cab_dual_param_ed08_enabled();
    let mut packets: Vec<Vec<u8>> = Vec::new();

    let ctr_a = state.live_write_ctr;
    if arm_ed08 {
        let ed08_seq = state.next_x80_cnt();
        packets.push(vec![
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, ed08_seq, 0x00, 0x08,
            (ctr_a & 0xff) as u8, ((ctr_a >> 8) & 0xff) as u8, 0x00, 0x00,
        ]);
    }

    let seq_a = state.next_x80_cnt();
    let yy_a = state.live_write_yy;
    let mut packet_a = if wire_23 {
        assemble_23_bool_write(seq_a, 0x04, ctr_a, yy_a, pp, param_selector, slot_bus, mark_23)
    } else {
        assemble_27_write(
            seq_a,
            0x04,
            ctr_a,
            yy_a,
            pp,
            param_selector,
            slot_bus,
            float_be_a,
        )
    };
    apply_model_block(
        &mut packet_a,
        state,
        route.model_block,
        !route.preserve_model_tag,
    );
    finalize_primary_packet(
        &mut packet_a,
        primary_opcode,
        slot_bus,
        param_selector,
        float_be_a,
        mark_23,
    );
    packets.push(packet_a);

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);

    if !wire_23 {
        let seq_b = state.next_x80_cnt();
        let ctr_b = state.live_write_ctr;
        let yy_b = state.live_write_yy;
        let mut packet_b = assemble_27_write(
            seq_b,
            0x0c,
            ctr_b,
            yy_b,
            pp,
            param_selector,
            slot_bus,
            float_be_b,
        );
        apply_model_block(
            &mut packet_b,
            state,
            route.model_block,
            !route.preserve_model_tag,
        );
        finalize_primary_packet(
            &mut packet_b,
            primary_opcode,
            slot_bus,
            param_selector,
            float_be_b,
            mark_23,
        );
        packets.push(packet_b);
        state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
        state.live_write_yy = state.live_write_yy.wrapping_add(1);
    }

    CabDualMinimalLiveWrite {
        packets,
        primary_opcode,
        slot_bus,
        pp,
        pp_source,
        param_selector,
        param_selector_source,
        model_block_kind: "amp_cab_cab_route",
    }
}

#[cfg(test)]
mod cab_dual_minimal_tests {
    use super::*;
    use crate::helix::cab_dual_live_write::resolve_cab_dual_live_write_route;
    use crate::helix::HelixState;

    #[test]
    fn cab_dual_mic_minimal_is_single_23() {
        let state = HelixState::new();
        let route = resolve_cab_dual_live_write_route(&state, 0, 0, 0, false).expect("route");
        let mut state = HelixState::new();
        let out = build_cab_dual_minimal_param_packets_from_state(
            &mut state,
            0.0,
            0,
            Some("cabMICir"),
            Some(0),
            Some(0.0),
            Some(11.0),
            route,
        );
        assert_eq!(out.primary_opcode, 0x23);
        let params: Vec<&Vec<u8>> = out.packets.iter().filter(|p| p[0] != 0x08).collect();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0][0], 0x23);
        assert_eq!(params[0][11], 0x04);
        assert_eq!(out.packets[0][0], 0x08, "ed:08 d'armement en tête");
    }

    #[test]
    fn cab_dual_float_minimal_is_27_pair() {
        let state = HelixState::new();
        let route = resolve_cab_dual_live_write_route(&state, 0, 1, 0, false).expect("route");
        let mut state = HelixState::new();
        let out = build_cab_dual_minimal_param_packets_from_state(
            &mut state,
            0.5,
            0,
            Some("CabMicIr_Position"),
            Some(1),
            Some(0.0),
            Some(1.0),
            route,
        );
        assert_eq!(out.primary_opcode, 0x27);
        let params: Vec<&Vec<u8>> = out.packets.iter().filter(|p| p[0] != 0x08).collect();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0][11], 0x04);
        assert_eq!(params[1][11], 0x0c);
        assert_eq!(out.packets[0][0], 0x08);
    }
}
