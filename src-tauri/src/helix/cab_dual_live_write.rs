//! Routage write live **Cab dual IR** (Stomp XL).
//!
//! Capture `add_dual_cab_modif_param_cab2.json` (HX Edit, 2026-06) :
//! - **Cab 1** : PP `0x03`, `param_selector` = index local 0..n (comme Amp+Cab cab IR).
//! - **Cab 2** : PP `0x04`, même `param_selector` wire que Cab 1 (index global dans le slot dual).
//! - Bloc modèle IR : `…1d:c3:1a:01:1c` ; legacy hybrid : `…64:83:17:c3:19`.
//! - **Param write** : HX Edit n’envoie que `23`/`27` (capture `add_dual_cab_modif_param_cab2.json`) ;
//!   focus onglet **une fois** au clic UI (`focus_cab_dual_usb_part`), pas de pré `08` avant chaque param.
//! - `prepare_cab_dual_param_live_write` : focus lane `live_write` + ed:08 (handshake replace cab2).
//!
//! FOCUS onglet Cab 1 / Cab 2 — capture HX Edit `cab2_cab1_change.json` (2026-06) :
//! les **deux** onglets envoient `1d` + `83:66:cd:03` + `82:62:bus:1a:XX` (`XX=00` Cab 1, `01` Cab 2).
//! Replace modèle Cab 2 (`cab dual change right.json`) : `cd:04` + `1a:01` — voir
//! [`build_cab_dual_cab2_replace_focus_packet`].

use std::time::{Duration, Instant};

use crate::helix::amp_cab_live_write::build_amp_cab_legacy_param_model_block;
use crate::helix::amp_cab_live_write::legacy_cab_wire_pair;

/// Premier sélecteur param cab2 d'un slot Cab dual legacy (`cab dual legacy.json`).
const LEGACY_DUAL_CAB2_PARAM_SELECTOR_BASE: u8 = 0x2b;

fn legacy_cab_dual_param_selector(cab_index: u8, local_param_index: u32) -> Option<u8> {
    match cab_index {
        0 => legacy_cab_wire_pair(local_param_index, 0).map(|(sel, _)| sel),
        1 => {
            let sel = LEGACY_DUAL_CAB2_PARAM_SELECTOR_BASE as u32 + local_param_index;
            if sel > 0xff {
                return None;
            }
            Some(sel as u8)
        }
        _ => None,
    }
}
use crate::helix::ed03_lane::build_ed08_short;
use crate::helix::live_write::LiveWriteRouteOverride;
use crate::helix::packet::OutPacket;
use crate::helix::{echo_model_cache_key, kempline_index_to_slot_bus, HelixState};

/// Source de la lane (octets 12-14) portée par le focus `1d`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cab2FocusLane {
    /// **Défaut.** `session_quadruple` (octets 0-2) — la VRAIE lane modèle : c'est elle que
    /// l'ed:08 auto-ACK de `standard.rs` échoue au device (`sq[0..4]`), donc la lane que le
    /// device valide. Après l'add-dual elle vaut `05 1f` (init `f4 1e` + 0x11), figée pendant
    /// le handshake (`cab_dual_cab2_block_standard_auto_ack`). HX : focus = lane modèle = ed:08
    /// add-dual. → focus `05 1f`, ed:08 `16 1f`, IN 21 attendu.
    SessionQuadruple,
    /// `editor_ed03_lane` + `editor_ed03_lane_b14` — MAUVAIS champ (déborde à `55 2f`, dumpe
    /// mais ed:08 refusé). Conservé en témoin `HX_CAB2_FOCUS_LANE=editor`.
    Editor,
    /// Lane keepalive ed:03 (`7e 1c`). RÉFUTÉE HW : le device ne dumpe même plus.
    Keepalive,
    /// `live_write_ctr` (témoin plus ancien, octet 14 = 0).
    LiveWrite,
}

/// `HX_CAB2_FOCUS_LANE` : `editor` (défaut — SEULE lane qui fait dumper le device en pratique) |
/// `sq`/`session` | `keepalive` | `livewrite`. Les lanes `sq` (`05 1f`) et `keepalive` (`7e 1c`)
/// n'ont jamais déclenché le dump sur HW — seule `editor` (`55 2f`) le fait. Tant que le dump
/// n'est pas obtenu, l'`IN 21` est hors de portée.
fn cab2_focus_lane_from_env() -> Cab2FocusLane {
    match std::env::var("HX_CAB2_FOCUS_LANE")
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Ok("sq") | Ok("session") | Ok("session_quadruple") => Cab2FocusLane::SessionQuadruple,
        Ok("keepalive") => Cab2FocusLane::Keepalive,
        Ok("livewrite") | Ok("live_write") => Cab2FocusLane::LiveWrite,
        _ => Cab2FocusLane::Editor,
    }
}

/// Octet 11 du focus. Défaut `0x04` (config connue-dumpante). Témoin `HX_CAB2_FOCUS_SUB14=1` → `0x14`
/// (forme HX add-dual ; à bisecter — on ignore encore si `0x14` casse le dump ou non).
fn cab2_focus_sub() -> u8 {
    match std::env::var("HX_CAB2_FOCUS_SUB14").as_deref() {
        Ok(v) if matches!(v.trim(), "1" | "true" | "yes" | "on") => 0x14,
        _ => 0x04,
    }
}

/// Bloc modèle param **Cab 1** d'un slot Cab dual IR. Kind cd:03, suffixe **1a:00** (cab1).
/// Le suffixe `1a:00`/`1a:01` est la SEULE distinction cab1/cab2 sur le fil — ne pas réutiliser
/// le bloc Amp+Cab (`build_amp_cab_ir_param_model_block`, qui porte `1a:01` car le cab y est partie 1).
pub fn build_cab_dual_cab1_ir_param_model_block(slot_bus: u8, tag_yy: u8) -> [u8; 16] {
    [
        0x83, 0x66, 0xcd, 0x03, tag_yy, 0x64, 0x1e, 0x65, 0x85, 0x62, slot_bus, 0x1d, 0xc3,
        0x1a, 0x00, 0x1c,
    ]
}

/// Bloc modèle param **Cab 2** d'un slot Cab dual IR. Kind **cd:03** (param/live),
/// PAS cd:04 (replace/focus) — capture `add_dual_cab_modif_param_cab2.json`.
pub fn build_cab_dual_cab2_ir_param_model_block(slot_bus: u8, tag_yy: u8) -> [u8; 16] {
    [
        0x83, 0x66, 0xcd, 0x03, tag_yy, 0x64, 0x1e, 0x65, 0x85, 0x62, slot_bus, 0x1d, 0xc3,
        0x1a, 0x01, 0x1c,
    ]
}

/// Encode `live_write_ctr` dans les octets 12–14 du focus `1d` (règle `cab_dual_ed08_ctr_from_focus`).
fn cab_dual_focus_ctr_triple(ctr: u16) -> (u8, u8, u8) {
    let lo = (ctr & 0xff) as u8;
    let hi = ((ctr >> 8) & 0xff) as u8;
    (lo.wrapping_sub(0x11), hi, 0x00)
}

/// Octets 12-14 du focus selon la source de lane choisie.
fn cab2_focus_ctr_triple_for(state: &HelixState, lane_src: Cab2FocusLane) -> (u8, u8, u8) {
    match lane_src {
        Cab2FocusLane::SessionQuadruple => {
            let sq = state.session_quadruple;
            (sq[0], sq[1], sq[2])
        }
        Cab2FocusLane::Keepalive => {
            let ka = crate::helix::keep_alive::keepalive_ed_lane();
            (ka[0], ka[1], 0x00)
        }
        Cab2FocusLane::Editor => {
            let lane = state.editor_ed03_lane_bytes();
            (lane[0], lane[1], state.editor_ed03_lane_b14)
        }
        Cab2FocusLane::LiveWrite => cab_dual_focus_ctr_triple(state.live_write_ctr),
    }
}

/// Builder cœur focus `1d` Cab dual : `cd` + suffixe `1a:XX` (tab ou replace).
pub fn build_cab_dual_focus_packet_with_lane(
    state: &mut HelixState,
    slot_bus: u8,
    lane_src: Cab2FocusLane,
    sub: u8,
    cd: u8,
    suffix_after_1a: u8,
) -> Vec<u8> {
    let cnt = state.next_x80_cnt();
    let (b12, b13, b14) = cab2_focus_ctr_triple_for(state, lane_src);
    let tag = state.live_write_yy;
    let ed08_pred = u16::from_le_bytes([b12.wrapping_add(0x11), b13]);
    let ka = crate::helix::keep_alive::keepalive_ed_lane();
    let lane = state.editor_ed03_lane_bytes();
    let sq = state.session_quadruple;
    crate::helix::init_trace::trace_fmt(format_args!(
        "cab_dual_focus cd={:#04x} 1a={:#04x} src={:?} sub={:#04x} b12-14={:02x}:{:02x}:{:02x} -> ed08_pred={:#06x} \
         (sq={:02x}:{:02x}:{:02x} keepalive={:02x}:{:02x} editor={:02x}:{:02x} b14={:02x} live_ctr={:#06x})",
        cd,
        suffix_after_1a,
        lane_src,
        sub,
        b12,
        b13,
        b14,
        ed08_pred,
        sq[0],
        sq[1],
        sq[2],
        ka[0],
        ka[1],
        lane[0],
        lane[1],
        state.editor_ed03_lane_b14,
        state.live_write_ctr
    ));
    vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt, 0x00, sub, b12, b13, b14,
        0x00, 0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, cd, tag,
        0x64, 0x4e, 0x65, 0x82, 0x62, slot_bus, 0x1a, suffix_after_1a, 0x00, 0x00, 0x00,
    ]
}

/// Focus onglet **Cab 2** (`cd:03`, `1a:01`) — `cab2_cab1_change.json`.
pub fn build_cab_dual_cab2_tab_focus_packet(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    build_cab_dual_cab2_tab_focus_packet_with_lane(
        state,
        slot_bus,
        cab2_focus_lane_from_env(),
        cab2_focus_sub(),
    )
}

pub fn build_cab_dual_cab2_tab_focus_packet_with_lane(
    state: &mut HelixState,
    slot_bus: u8,
    lane_src: Cab2FocusLane,
    sub: u8,
) -> Vec<u8> {
    build_cab_dual_focus_packet_with_lane(state, slot_bus, lane_src, sub, 0x03, 0x01)
}

/// Focus **replace** Cab 2 (`cd:04`, `1a:01`) — `cab dual change right.json`.
pub fn build_cab_dual_cab2_replace_focus_packet(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    build_cab_dual_cab2_replace_focus_packet_with_lane(
        state,
        slot_bus,
        cab2_focus_lane_from_env(),
        cab2_focus_sub(),
    )
}

/// Focus sous-bloc **Cab 2** (replace). Enveloppe `1d …`, bloc `83:66:cd:04`, suffixe `1a:01`.
pub fn build_cab_dual_cab2_focus_packet(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    build_cab_dual_cab2_replace_focus_packet(state, slot_bus)
}

/// Builder cœur replace Cab 2 : `lane_src` choisit les octets 12-14, `sub` l'octet 11.
pub fn build_cab_dual_cab2_focus_packet_with_lane(
    state: &mut HelixState,
    slot_bus: u8,
    lane_src: Cab2FocusLane,
    sub: u8,
) -> Vec<u8> {
    build_cab_dual_cab2_replace_focus_packet_with_lane(state, slot_bus, lane_src, sub)
}

pub fn build_cab_dual_cab2_replace_focus_packet_with_lane(
    state: &mut HelixState,
    slot_bus: u8,
    lane_src: Cab2FocusLane,
    sub: u8,
) -> Vec<u8> {
    build_cab_dual_focus_packet_with_lane(state, slot_bus, lane_src, sub, 0x04, 0x01)
}

/// Compat tests historiques : `use_editor_lane` true→Editor / false→LiveWrite, octet 11 = `0x04`.
pub fn build_cab_dual_cab2_focus_packet_with_source(
    state: &mut HelixState,
    slot_bus: u8,
    use_editor_lane: bool,
) -> Vec<u8> {
    let lane_src = if use_editor_lane {
        Cab2FocusLane::Editor
    } else {
        Cab2FocusLane::LiveWrite
    };
    build_cab_dual_cab2_focus_packet_with_lane(state, slot_bus, lane_src, 0x04)
}

/// Mémorise le ctr Cab 2 depuis un IN `19`/36o (`cd:04`).
pub fn ingest_cab_dual_cab2_in36(state: &mut HelixState, data: &[u8]) {
    if data.len() != 36 || data.first() != Some(&0x19) || !cab_dual_in36_uses_cd04_lane(data) {
        return;
    }
    state.cab_dual_cab2_last_in36_frame = Some(data.to_vec());
    if let Some(ctr) = cab_dual_ed08_ctr_from_in36(data) {
        state.cab_dual_cab2_last_in36_ed08_ctr = Some(ctr);
    }
}

/// OUT `f0:08` après focus Cab 2 — requis sur Stomp XL pour obtenir IN `19`/36o.
pub fn send_cab_dual_cab2_f008_poke(state: &mut HelixState) {
    let d = state.firmware_scroll_lane_double();
    let ctr = u16::from_le_bytes([d[0], d[1]]);
    let seq = state.next_x2_cnt();
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03, 0x00, seq, 0x00, 0x08,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x00,
    ]));
}

/// Focus onglet **Cab 1** (`cd:03`, `1a:00`) — `cab2_cab1_change.json`.
fn build_cab_dual_cab1_focus_packet(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    build_cab_dual_focus_packet_with_lane(
        state,
        slot_bus,
        cab2_focus_lane_from_env(),
        cab2_focus_sub(),
        0x03,
        0x00,
    )
}

/// Focus Cab 1 + poke `f0:08` (onglet UI).
pub fn send_cab_dual_cab1_focus_and_poke(state: &mut HelixState, slot_index: u32, slot_bus: u8) {
    state.cab_dual_live_write_tab_focus = Some((slot_index, 0));
    state.cab_dual_cab2_handshake_capture.clear();
    state.cab_dual_cab2_handshake_until = None;
    state.cab_dual_cab2_focus_sent_for_slot = None;
    state.last_cab_dual_cab2_focus_packet = None;
    state.cab_dual_cab2_suppress_standard_ed08_until = None;
    let focus = build_cab_dual_cab1_focus_packet(state, slot_bus);
    crate::helix::init_trace::trace_fmt(format_args!(
        "cab_dual_cab1_focus OUT len={} (cd:03 1a:00, capture cab2_cab1_change.json)",
        focus.len()
    ));
    state.send(OutPacket::new(focus));
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    send_cab_dual_cab2_f008_poke(state);
}

/// Focus onglet Cab 2 + poke `f0:08`.
pub fn send_cab_dual_cab2_focus_and_poke(state: &mut HelixState, slot_index: u32, slot_bus: u8) {
    state.cab_dual_live_write_tab_focus = Some((slot_index, 1));
    state.cab_dual_cab2_handshake_capture.clear();
    state.cab_dual_cab2_handshake_until =
        Some(Instant::now() + Duration::from_millis(700));
    let focus = build_cab_dual_cab2_tab_focus_packet(state, slot_bus);
    state.send(OutPacket::new(focus.clone()));
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    state.slot_model_lane_seq = Some(state.live_write_yy);
    state.cab_dual_cab2_focus_sent_for_slot = Some(slot_index);
    send_cab_dual_cab2_f008_poke(state);
}

/// IN `19`/36o sur lane occupée `cd:04` (Stomp XL après add dual).
pub fn cab_dual_in36_uses_cd04_lane(in36: &[u8]) -> bool {
    in36.len() >= 28 && in36.get(24..28) == Some(&[0x83, 0x66, 0xcd, 0x04])
}

/// Ctr `ed:08` dérivé de l’IN 36o : `LE(in36[12]+0x11, in36[13])`.
/// ⚠ NON utilisé pour le handshake `IN 21` (la lane d'écho device ne valide pas l'ed:08) —
/// conservé pour diagnostic uniquement.
pub fn cab_dual_ed08_ctr_from_in36(in36: &[u8]) -> Option<u16> {
    if in36.len() < 14 {
        return None;
    }
    Some(u16::from_le_bytes([
        in36[12].wrapping_add(0x11),
        in36[13],
    ]))
}

/// Ctr `ed:08` dérivé du focus `1d`. Octet14 ≠ 0 (ancienne lane éditeur, ex. `14 8a 1c`) →
/// `LE(focus[13]+0x11, focus[14])`. Octet14 = 0 (lane keepalive `7e 1c` / live_write_ctr) →
/// `LE(focus[12]+0x11, focus[13])` (keepalive `7e 1c` → `8f 1c`).
pub fn cab_dual_ed08_ctr_from_focus(focus: &[u8]) -> Option<u16> {
    if focus.len() < 15 || focus.first() != Some(&0x1d) {
        return None;
    }
    if focus[14] != 0 {
        Some(u16::from_le_bytes([
            focus[13].wrapping_add(0x11),
            focus[14],
        ]))
    } else {
        Some(u16::from_le_bytes([
            focus[12].wrapping_add(0x11),
            focus[13],
        ]))
    }
}

/// Ctr `ed:08` du handshake `IN 21` = **toujours** dérivé du focus (= lane keepalive + 0x11).
/// La capture HX prouve que la lane d'écho device (`ec 02` / `5f 03`) ne valide PAS l'ed:08 ;
/// on ne préfère donc plus l'IN 36o. `in36` reste en paramètre pour compat/diagnostic.
pub fn cab_dual_ed08_ctr_for_handshake(focus: &[u8], _in36: &[u8]) -> u16 {
    cab_dual_ed08_ctr_from_focus(focus).unwrap_or(0)
}

fn force_ed03_lane_ctr(packet: &mut [u8], ctr: u16) {
    if packet.len() > 14 {
        packet[12] = (ctr & 0xff) as u8;
        packet[13] = ((ctr >> 8) & 0xff) as u8;
        packet[14] = 0x00;
    }
}

/// Prépare le device avant write live param Cab dual : focus onglet (`L`) puis ed:08 (`L+0x11`).
///
/// Même règle lane que replace cab2 ([`crate::helix::cab_dual::replace_fire`]) — sans ed:08 le HW
/// ignore les trames `23`/`27` (captures `add_dual_cab_modif_param_cab2.json`).
pub fn prepare_cab_dual_param_live_write(
    state: &mut HelixState,
    slot_index: u32,
    slot_bus: u8,
    cab_index: u8,
) {
    if state.cab_dual_live_write_tab_focus == Some((slot_index, cab_index)) {
        return;
    }
    let l = state.live_write_ctr;
    let mut focus = if cab_index == 0 {
        build_cab_dual_focus_packet_with_lane(
            state,
            slot_bus,
            Cab2FocusLane::LiveWrite,
            cab2_focus_sub(),
            0x03,
            0x00,
        )
    } else {
        build_cab_dual_cab2_tab_focus_packet_with_lane(
            state,
            slot_bus,
            Cab2FocusLane::LiveWrite,
            cab2_focus_sub(),
        )
    };
    force_ed03_lane_ctr(&mut focus, l);
    let ctr_model = l.wrapping_add(0x11);
    crate::helix::init_trace::trace_fmt(format_args!(
        "cab_dual_param_prepare cab={} slot={} bus={:#04x} focus_L={:#06x} ed08={:#06x} cd={:#04x} 1a={:#04x}",
        cab_index,
        slot_index,
        slot_bus,
        l,
        ctr_model,
        focus[27],
        focus[36]
    ));
    state.send(OutPacket::new(focus));
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    state.slot_model_lane_seq = Some(state.live_write_yy);
    let ed08 = build_ed08_short(state, ctr_model);
    state.send(OutPacket::with_delay(ed08, 93));
    state.live_write_ctr = ctr_model;
    state.cab_dual_live_write_tab_focus = Some((slot_index, cab_index));
    if cab_index == 1 {
        state.cab_dual_cab2_focus_sent_for_slot = Some(slot_index);
    }
}

/// Alias historique.
pub fn send_cab_dual_tab_focus_for_live_write(
    state: &mut HelixState,
    slot_index: u32,
    slot_bus: u8,
    cab_index: u8,
) {
    prepare_cab_dual_param_live_write(state, slot_index, slot_bus, cab_index);
}

/// Focus Cab 2 immédiatement avant replace (HX Edit : `cd:04` + `1a:01`, pas l’onglet).
pub fn send_cab_dual_cab2_focus(state: &mut HelixState, slot_index: u32, slot_bus: u8) {
    state.cab_dual_live_write_tab_focus = None;
    state.cab_dual_cab2_suppress_standard_ed08_until =
        Some(Instant::now() + Duration::from_secs(45));
    state.cab_dual_cab2_handshake_ed08_ctr = None;
    if let Some(ctr) = state.cab_dual_cab2_last_in36_ed08_ctr {
        state.live_write_ctr = ctr;
    }
    let focus = build_cab_dual_cab2_focus_packet(state, slot_bus);
    state.last_cab_dual_cab2_focus_packet = Some(focus.clone());
    state.send(crate::helix::packet::OutPacket::new(focus));
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    state.slot_model_lane_seq = Some(state.live_write_yy);
    state.cab_dual_cab2_focus_sent_for_slot = Some(slot_index);
}

/// Segment preset dual legacy hybrid (suffixe modèle `64:83:17:c3:19`) — tests / debug uniquement.
pub fn cab_dual_preset_segment_is_legacy_hybrid(seg: &[u8]) -> bool {
    seg.windows(5)
        .any(|w| w == [0x64, 0x83, 0x17, 0xc3, 0x19])
}

/// Variante catalogue live write : `dual-legacy` / `dual legacy` (hybrid `c3:19`), sinon IR.
pub fn cab_dual_assign_variant_is_legacy_hybrid(variant: Option<&str>) -> bool {
    variant.map_or(false, |v| {
        let t = v.trim();
        t.eq_ignore_ascii_case("dual-legacy") || t.eq_ignore_ascii_case("dual legacy")
    })
}

pub fn resolve_cab_dual_live_write_route(
    state: &HelixState,
    cab_index: u8,
    param_index: u32,
    slot_index: u32,
    legacy_hybrid: bool,
) -> Option<LiveWriteRouteOverride> {
    if cab_index > 1 {
        return None;
    }
    let slot_bus = kempline_index_to_slot_bus(slot_index as usize)?;
    let param_selector = if legacy_hybrid {
        legacy_cab_dual_param_selector(cab_index, param_index)?
    } else {
        param_index.min(0xff) as u8
    };
    let pp = if cab_index == 0 { 0x03 } else { 0x04 };
    let cache_key = echo_model_cache_key(slot_bus, pp, param_selector);
    if let Some(block) = state.ed03_echo_model_by_slot_param.get(&cache_key) {
        return Some(LiveWriteRouteOverride {
            pp: block[3],
            pp_source: "cab_dual:echo_cache",
            param_selector,
            param_selector_source: if legacy_hybrid {
                if cab_index == 0 {
                    "cab_dual:cab1_legacy_echo_sel"
                } else {
                    "cab_dual:cab2_legacy_echo_sel"
                }
            } else if cab_index == 0 {
                "cab_dual:cab1_echo_sel"
            } else {
                "cab_dual:cab2_echo_sel"
            },
            model_block: *block,
            preserve_model_tag: true,
        });
    }

    let model_block = if legacy_hybrid {
        // Octet `[4]` = sélecteur wire (capture dual legacy cab2 : `0x2b` + index local).
        build_amp_cab_legacy_param_model_block(pp, param_selector, slot_bus)
    } else if cab_index == 0 {
        build_cab_dual_cab1_ir_param_model_block(slot_bus, state.live_write_yy)
    } else {
        build_cab_dual_cab2_ir_param_model_block(slot_bus, state.live_write_yy)
    };
    Some(LiveWriteRouteOverride {
        pp,
        pp_source: if legacy_hybrid {
            if cab_index == 0 {
                "cab_dual:cab1_legacy_capture"
            } else {
                "cab_dual:cab2_legacy_capture"
            }
        } else if cab_index == 0 {
            "cab_dual:cab1_capture"
        } else {
            "cab_dual:cab2_capture"
        },
        param_selector,
        param_selector_source: if cab_index == 0 {
            "cab_dual:cab1_local_index"
        } else {
            "cab_dual:cab2_local_index"
        },
        model_block,
        preserve_model_tag: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::HelixState;

    #[test]
    fn cab1_param_block_uses_1a_00() {
        let blk = build_cab_dual_cab1_ir_param_model_block(3, 0x17);
        assert_eq!(blk[3], 0x03, "kind param cd:03");
        assert_eq!(&blk[13..16], &[0x1a, 0x00, 0x1c], "suffixe cab1");
    }

    #[test]
    fn cab2_param_block_uses_1a_01() {
        let blk = build_cab_dual_cab2_ir_param_model_block(3, 0x17);
        assert_eq!(&blk[13..16], &[0x1a, 0x01, 0x1c], "suffixe cab2");
    }

    #[test]
    fn cab2_level_uses_pp_04_and_local_selector() {
        let state = HelixState::new();
        let route = resolve_cab_dual_live_write_route(&state, 1, 5, 3, false).expect("route");
        assert_eq!(route.pp, 0x04);
        assert_eq!(route.param_selector, 0x05);
        assert_eq!(route.model_block[3], 0x03, "kind param cd:03 sur le fil");
        assert_eq!(&route.model_block[13..16], &[0x1a, 0x01, 0x1c], "suffixe cab2");
    }

    /// DÉFAUT : lane `editor` (la seule qui fait dumper le device sur HW) + octet 11 `0x04`.
    #[test]
    fn cab2_focus_default_editor_lane() {
        let mut state = HelixState::new();
        state.editor_ed03_lane = 0x2f55; // valeur observée côté Linux
        state.editor_ed03_lane_b14 = 0x00;
        let pkt = build_cab_dual_cab2_focus_packet(&mut state, 1);
        assert_eq!(pkt[0], 0x1d);
        assert_eq!(pkt.len(), 40);
        assert_eq!(pkt[11], 0x04);
        assert_eq!(pkt[12..15], [0x55, 0x2f, 0x00], "lane editor");
    }

    /// Témoin session_quadruple (ne dumpe pas sur HW) — gardé pour bisection.
    #[test]
    fn cab2_focus_session_quadruple_temoin() {
        let mut state = HelixState::new();
        state.session_quadruple = [0x05, 0x1f, 0x00, 0x00];
        let pkt = build_cab_dual_cab2_focus_packet_with_lane(
            &mut state,
            1,
            Cab2FocusLane::SessionQuadruple,
            0x04,
        );
        assert_eq!(pkt[12..15], [0x05, 0x1f, 0x00]);
        assert_eq!(cab_dual_ed08_ctr_from_focus(&pkt), Some(0x1f16));
    }

    /// Témoin editor (mauvais champ, dumpe mais ed:08 refusé).
    #[test]
    fn cab2_focus_editor_temoin_form() {
        let mut state = HelixState::new();
        state.editor_ed03_lane = 0x8a14;
        state.editor_ed03_lane_b14 = 0x1c;
        let pkt =
            build_cab_dual_cab2_focus_packet_with_lane(&mut state, 1, Cab2FocusLane::Editor, 0x04);
        assert_eq!(pkt[12..15], [0x14, 0x8a, 0x1c]);
    }

    /// Témoin keepalive (réfuté HW : pas de dump) — conservé pour bisection seulement.
    #[test]
    fn cab2_focus_keepalive_temoin_form() {
        let mut state = HelixState::new();
        let pkt =
            build_cab_dual_cab2_focus_packet_with_lane(&mut state, 1, Cab2FocusLane::Keepalive, 0x14);
        assert_eq!(pkt[11], 0x14);
        assert_eq!(pkt[12..15], [0x7e, 0x1c, 0x00]);
        assert_eq!(cab_dual_ed08_ctr_from_focus(&pkt), Some(0x1c8f));
    }

    #[test]
    fn cab2_focus_packet_embeds_live_write_ctr_for_ed08() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x0370;
        // Témoin LiveWrite (octet 11 = 0x04) : focus depuis live_write_ctr.
        let pkt = build_cab_dual_cab2_focus_packet_with_source(&mut state, 1, false);
        assert_eq!(pkt[11], 0x04);
        assert_eq!(pkt[12..15], [0x5f, 0x03, 0x00]);
        assert_eq!(cab_dual_ed08_ctr_from_focus(&pkt), Some(0x0370));
    }

    #[test]
    fn cab2_replace_focus_packet_uses_cd04_and_cab2_suffix() {
        let mut state = HelixState::new();
        let pkt = build_cab_dual_cab2_focus_packet_with_source(&mut state, 3, false);
        assert_eq!(pkt[0], 0x1d);
        assert_eq!(pkt.len(), 40);
        assert_eq!(pkt[24], 0x83);
        assert_eq!(pkt[27], 0x04);
        assert_eq!(pkt[34], 0x03);
        assert_eq!(pkt[35], 0x1a);
        assert_eq!(pkt[36], 0x01);
    }

    /// HX Edit `cab2_cab1_change.json` frame 3269 — onglet Cab 1.
    #[test]
    fn cab1_tab_focus_matches_hx_cab2_cab1_change_capture() {
        let focus = [
            0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x26, 0x00, 0x04, 0x82, 0x1c,
            0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03,
            0xf2, 0x64, 0x4e, 0x65, 0x82, 0x62, 0x01, 0x1a, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(focus[27], 0x03);
        assert_eq!(focus[36], 0x00);
        assert_eq!(cab_dual_ed08_ctr_from_focus(&focus), Some(0x1c93));
    }

    /// HX Edit `cab2_cab1_change.json` frame 1771 — onglet Cab 2 (tab, pas replace).
    #[test]
    fn cab2_tab_focus_matches_hx_cab2_cab1_change_capture() {
        let focus = [
            0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x22, 0x00, 0x04, 0x71, 0x1c,
            0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03,
            0xf1, 0x64, 0x4e, 0x65, 0x82, 0x62, 0x01, 0x1a, 0x01, 0x00, 0x00, 0x00,
        ];
        assert_eq!(focus[27], 0x03);
        assert_eq!(focus[36], 0x01);
        assert_eq!(cab_dual_ed08_ctr_from_focus(&focus), Some(0x1c82));
        let mut state = HelixState::new();
        state.live_write_ctr = 0x1c82;
        state.live_write_yy = 0xf1;
        let pkt = build_cab_dual_cab2_tab_focus_packet_with_lane(
            &mut state,
            1,
            Cab2FocusLane::LiveWrite,
            0x04,
        );
        assert_eq!(pkt[27], 0x03);
        assert_eq!(pkt[36], 0x01);
        assert_eq!(pkt[12..15], [0x71, 0x1c, 0x00]);
    }

    /// Ancienne lane éditeur (octets 12-14 = lane_lo, lane_hi, b14, forme `14 8a 1c`) :
    /// ed:08 dérive via la branche octet14≠0 (`9b 1c`). Conservé comme témoin `HX_CAB2_FOCUS_LANE=editor`.
    #[test]
    fn cab2_focus_editor_lane_form_matches_hx_soup_pro() {
        let mut state = HelixState::new();
        state.editor_ed03_lane = 0x8a14; // LE(0x14, 0x8a)
        state.editor_ed03_lane_b14 = 0x1c;
        let pkt = build_cab_dual_cab2_focus_packet_with_source(&mut state, 1, true);
        assert_eq!(pkt[12..15], [0x14, 0x8a, 0x1c]);
        assert_eq!(cab_dual_ed08_ctr_from_focus(&pkt), Some(0x1c9b));
    }

    #[test]
    fn ed08_ctr_from_in36_cd04_lane_linux_capture() {
        let in36 = [
            0x19, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x46, 0x00, 0x04, 0x5f, 0x03,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04,
            0x18, 0x67, 0x00, 0x68, 0xc0, 0x0d, 0x01, 0x18,
        ];
        assert!(cab_dual_in36_uses_cd04_lane(&in36));
        assert_eq!(cab_dual_ed08_ctr_from_in36(&in36), Some(0x0370));
    }

    #[test]
    fn ed08_ctr_matches_hx_cab_dual_change_right_focus() {
        let focus = [
            0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x99, 0x00, 0x04, 0x7d, 0x6e,
            0x00, 0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04, 0x59,
            0x64, 0x4e, 0x65, 0x82, 0x62, 0x01, 0x1a, 0x01, 0x00, 0x00, 0x00,
        ];
        assert_eq!(cab_dual_ed08_ctr_from_focus(&focus), Some(0x6e8e));
    }

    /// Handshake : on prend la valeur du focus (lane keepalive), JAMAIS l'IN 36o cd:04.
    #[test]
    fn handshake_uses_focus_keepalive_not_in36() {
        // focus keepalive (b12-14 = 7e 1c 00) → ed:08 = 8f 1c.
        let focus = [
            0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x4f, 0x00, 0x14, 0x7e, 0x1c,
            0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04,
            0x18, 0x64, 0x4e, 0x65, 0x82, 0x62, 0x01, 0x1a, 0x01, 0x00, 0x00, 0x00,
        ];
        let in36 = [
            0x19, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x46, 0x00, 0x04, 0x5f, 0x03,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04,
            0x18, 0x67, 0x00, 0x68, 0xc0, 0x0d, 0x01, 0x18,
        ];
        // 8f 1c, PAS 70 03 (in36-derived).
        assert_eq!(cab_dual_ed08_ctr_for_handshake(&focus, &in36), 0x1c8f);
    }

    #[test]
    fn cab1_mic_is_pp_03_sel_00() {
        let state = HelixState::new();
        let route = resolve_cab_dual_live_write_route(&state, 0, 0, 3, false).expect("route");
        assert_eq!(route.pp, 0x03);
        assert_eq!(route.param_selector, 0x00);
        assert_eq!(&route.model_block[13..16], &[0x1a, 0x00, 0x1c]);
    }

    #[test]
    fn assign_variant_dual_legacy_enables_c319_block() {
        assert!(cab_dual_assign_variant_is_legacy_hybrid(Some("dual-legacy")));
        assert!(cab_dual_assign_variant_is_legacy_hybrid(Some("dual legacy")));
        assert!(!cab_dual_assign_variant_is_legacy_hybrid(Some("dual")));
        assert!(!cab_dual_assign_variant_is_legacy_hybrid(None));
    }

    #[test]
    fn legacy_hybrid_uses_c319_model_suffix() {
        let state = HelixState::new();
        let route = resolve_cab_dual_live_write_route(&state, 1, 2, 3, true).expect("route");
        assert_eq!(route.pp, 0x04);
        assert_eq!(route.param_selector, 0x2d);
        assert_eq!(route.model_block[4], 0x2d);
        assert_eq!(&route.model_block[11..16], &[0x64, 0x83, 0x17, 0xc3, 0x19]);
    }

    #[test]
    fn legacy_hybrid_cab1_param0_uses_compact_sel() {
        let state = HelixState::new();
        let route = resolve_cab_dual_live_write_route(&state, 0, 0, 3, true).expect("route");
        assert_eq!(route.pp, 0x03);
        assert_eq!(route.param_selector, 0x00);
        assert_eq!(route.model_block[4], 0x00);
    }

    #[test]
    fn param_live_write_prepare_bumps_ctr_by_0x11() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x6cbd;
        prepare_cab_dual_param_live_write(&mut state, 2, 3, 1);
        assert_eq!(state.cab_dual_live_write_tab_focus, Some((2, 1)));
        assert_eq!(state.live_write_ctr, 0x6cce);
    }
}