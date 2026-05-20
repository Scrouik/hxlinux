//! Changement de modèle sur le **slot actif** hardware (captures HX Edit
//! `Slot0_Change_Model_2_Time.json`).
//!
//! Flux cible (sans `preset_data`) :
//! 1. IN `1d` puis `1f` 40 o (`f0:03:02:10`) — souvent **même lot de lecture USB** (même timestamp).
//!    **Un seul** pull host sur `1f` ; `1d` ignoré (le debounce temps ne sépare pas un même batch).
//! 2. Host : `1b` 36 o → court `f0:03` 16 o → attendre IN ~92 o
//! 3. Host : `19` 36 o → attendre IN ~68 o
//! 4. Host : `19` 36 o → IN ~272 o avec `chainHex` / id module `19…1a`
//! 5. UI : pastille + panneau params depuis **catalogue + défauts `.models`**
//!
//! **Important** : HX Edit n’envoie pas `1b`+`19`+`19` d’un bloc ; envoyer les trois
//! `19` avant la réponse au `1b` empêche le Stomp de renvoyer les bulks `83:66`.

use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::helix::model_catalog;
use crate::helix::preset_debug_verbose_enabled;
use crate::helix::is_special_slot_bus;
use crate::helix::kempline_index_to_slot_bus;
use crate::helix::slot_bus_to_kempline_index;
use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

/// Délai entre les deux `19` (après réponse IN). `1b`+`f0` = rafale sans attente (HX Edit ~0,02 ms).
const INTER_19_DELAY_MS: u64 = 4;
/// Filet de sécurité si deux `1f` rapprochés (la paire `1d`+`1f` est ~15 ms — trop court pour débouncer entre les deux).
pub const PULL_DEBOUNCE_MS: u64 = 50;
/// Plafond si aucun `module_hex` (sinon `try_early_finalize` sort dès le bulk ~92/272 o).
const PULL_CAPTURE_MS: u64 = 400;
const PULL_CAPTURE_MAX_FRAMES: usize = 48;

// --- `live_write_ctr` (octets 12–13, LE) sur les OUT `80:10:ed:03` 36 o du pull modèle ---
//
// Même compteur que le live write param (`HelixState::live_write_ctr`, voir `live_write.rs` et
// `Line6_HX_Stomp_USB_Protocol.md`), mais les pas d’incrément **ne sont pas tous identiques** :
//
// | Contexte | Pas typique | Où c’est codé |
// |----------|-------------|---------------|
// | Write paramètre 48 o | **+0x1F** (31 déc.) | `live_write.rs`, défaut `edit_slot_model.rs` |
// | Pull : `1b` → 1er `19` | **+0x4B** ou **+0x44** (capture slot 0) | ci-dessous |
// | Pull : 1er `19` → 2e `19` | **+0x31** (49 déc.) — stable sur capture HX Edit | ci-dessous |
//
// Ne pas confondre **0x1F** (31 déc.) et **0x31** (49 déc.) : ce sont deux valeurs hex différentes.
// Réf. capture : `Slot0_Change_Model_2_Time.json` (`3f:41` → `8a:41` → `bb:41`, etc.).

/// Après envoi du `1b` : avance le CTR avant de construire le 1er `19` (délai réel + trafic intermédiaire).
const PULL_CTR_DELTA_AFTER_1B: u16 = 0x004b;

/// Entre les deux `19` consécutifs : pas observé **+0x31** (49 déc.), pas le +0x1F du live write.
const PULL_CTR_DELTA_AFTER_19: u16 = 0x0031;

static PULL_DEBUG: AtomicBool = AtomicBool::new(false);

pub fn set_slot_model_hw_pull_debug(enabled: bool) {
    PULL_DEBUG.store(enabled, Ordering::Relaxed);
}

pub fn slot_model_hw_pull_debug_enabled() -> bool {
    PULL_DEBUG.load(Ordering::Relaxed) || preset_debug_verbose_enabled()
}

fn pull_trace(msg: &str) {
    if slot_model_hw_pull_debug_enabled() {
        eprintln!("[SlotModelHwPull] {msg}");
    }
}

/// Log console (`npm run tauri dev`) : changement de modèle détecté depuis le HW.
fn log_hw_model_changed(module_hex: &str) {
    if let Some((chain_hex, name)) = model_catalog::resolve_chain_hex_and_name(module_hex) {
        eprintln!("\"{chain_hex}\"; \"{name}\"");
    } else {
        eprintln!("\"{module_hex}\"; \"(inconnu catalogue)\"");
    }
}

fn pull_trace_hex(label: &str, data: &[u8]) {
    if !slot_model_hw_pull_debug_enabled() || data.is_empty() {
        return;
    }
    let n = data.len().min(48);
    let hex: String = data[..n]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!(
        "[SlotModelHwPull] {label} len={} hex={}{}",
        data.len(),
        hex,
        if data.len() > n { " …" } else { "" }
    );
}

pub fn is_hw_model_change_notify_loose(data: &[u8]) -> bool {
    data.len() >= 32
        && (data[0] == 0x1d || data[0] == 0x1f || data[0] == 0x21)
        && data.get(1..4) == Some(&[0x00, 0x00, 0x18])
        && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
        && data.get(12..14) == Some(&[0x09, 0x02])
}

#[cfg(test)]
pub fn is_hw_model_change_notify_40(data: &[u8]) -> bool {
    data.len() == 40
        && is_hw_model_change_notify_loose(data)
        && data[8] == 0x00
        && data[10] == 0x00
        && data[11] == 0x04
}

pub fn is_hw_model_change_notify_1f(data: &[u8]) -> bool {
    is_hw_model_change_notify_loose(data) && data[0] == 0x1f
}

/// Seule entrée valide pour démarrer le pull (pas `1d`, pas les IN `21` 44 o preset/focus).
pub fn is_hw_model_pull_trigger_notify(data: &[u8]) -> bool {
    is_hw_model_change_notify_1f(data)
}

/// Bus du slot effet **actif** (`HelixState::hw_active_slot_*`, marqueur IN `82:62:SS:1a`).
pub fn active_effect_slot_bus(state: &HelixState) -> Option<u8> {
    if let Some(bus) = state.hw_active_slot_bus {
        if !is_special_slot_bus(bus) {
            return Some(bus);
        }
    }
    state
        .hw_active_slot_index
        .and_then(kempline_index_to_slot_bus)
}

/// Bus dans une notif changement modèle `1d`/`1f` (champ protocole `81:62:SS`) — **secours** pull.
pub fn parse_slot_bus_from_model_notify(data: &[u8]) -> Option<u8> {
    if !is_hw_model_change_notify_loose(data) {
        return None;
    }
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == 0x81 && data[i + 1] == 0x62 {
            let bus = data[i + 2];
            if !is_special_slot_bus(bus) {
                return Some(bus);
            }
        }
    }
    None
}

/// `slot_bus` injecté dans le OUT `1b` (octets protocole `81:62` — nom fixe, valeur = slot actif).
///
/// **Priorité** : [`active_effect_slot_bus`] (`hw_active`, alimenté par `82:62` preset/HW/UI).
/// **Secours** : `81:62` embarqué dans la notif `1d`/`1f` si `hw_active` est encore `None`
/// (fenêtre courte après MIDI Program Change, avant la trame IN `21` avec `82:62`).
///
/// ### Fiabilité du secours (seul cas « non contrôlé »)
///
/// Sémantique protocole : `82:62` = slot **sélectionné** ; `81:62` dans `1d`/`1f` = slot **où le modèle
/// a changé**. En édition normale (molette modèle sur le slot actif) ils coïncident — voir
/// `Slot0_Change_Model_2_Time.json` (`81:62:01` et focus `82:62:01`).
///
/// Après MIDI PC on efface `hw_active` volontairement ; si une notif modèle arrive avant le `21`
/// preset, on fait confiance au `81:62` de l’événement (logique : c’est le slot que le firmware
/// signale comme édité). **Non prouvé** sur toutes les transitions preset ; captures actuelles OK.
/// Si divergence observée en prod : log `hw_active=None, pull via 81:62=…` (déjà en eprintln pull)
/// ou reporter le pull jusqu’au prochain `82:62`.
fn slot_bus_for_model_pull(state: &HelixState, notify: &[u8]) -> Option<u8> {
    if let Some(hw) = active_effect_slot_bus(state) {
        if let Some(from_notify) = parse_slot_bus_from_model_notify(notify) {
            if hw != from_notify {
                pull_trace(&format!(
                    "hw_active={hw:02x} ≠ 81:62 notif={from_notify:02x} — pull sur hw_active"
                ));
            }
        }
        return Some(hw);
    }
    if let Some(from_notify) = parse_slot_bus_from_model_notify(notify) {
        pull_trace(&format!(
            "hw_active vide — secours 81:62 notif={from_notify:02x} (risque fenêtre post–MIDI PC)"
        ));
        return Some(from_notify);
    }
    None
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotModelHwChangedPayload {
    pub sequence: u32,
    pub slot_index: u32,
    pub slot_bus: u8,
    pub module_hex: Option<String>,
}

pub fn extract_first_module_hex_from_bulk(buf: &[u8]) -> Option<String> {
    let mut cursor = 0usize;
    while cursor < buf.len() {
        if buf[cursor] != 0x19 {
            cursor += 1;
            continue;
        }
        if cursor >= 3
            && buf[cursor - 3] == 0x83
            && buf[cursor - 2] == 0x17
            && buf[cursor - 1] == 0xc3
        {
            cursor += 1;
            continue;
        }
        let id_start = cursor + 1;
        let Some(rel_end) = buf[id_start..].iter().position(|&b| b == 0x1a) else {
            cursor += 1;
            continue;
        };
        let id_bytes = &buf[id_start..id_start + rel_end];
        cursor = id_start + rel_end + 1;
        if id_bytes.is_empty() || id_bytes.len() > 12 {
            continue;
        }
        let mut id_hex = String::with_capacity(id_bytes.len() * 2);
        for b in id_bytes {
            use std::fmt::Write as _;
            let _ = write!(&mut id_hex, "{:02x}", b);
        }
        return Some(id_hex);
    }
    None
}

/// Bulk assignable : `83:66:cd` + n’importe quel octet de voie (`02`…`04` observés — pas seulement `03`).
fn bulk_looks_like_assign_response(data: &[u8]) -> bool {
    data.len() >= 48 && frame_has_assign_marker(data)
}

/// Octet après `83:66:cd` (voie session / type de bloc — voir `Line6_HX_Stomp_USB_Protocol.md`).
fn cd_lane_byte(data: &[u8]) -> Option<u8> {
    data.windows(4)
        .find(|w| w[0] == 0x83 && w[1] == 0x66 && w[2] == 0xcd)
        .map(|w| w[3])
}

fn looks_like_first_pull_reply(data: &[u8]) -> bool {
    bulk_looks_like_assign_response(data)
        || data.len() >= 68
        || data.first() == Some(&0x53)
        // Si le `f0` ACK arrive trop tard, le Stomp répond parfois `1c` 36 o (8366) au lieu de `53` 92 o.
        || (data.len() >= 36 && data.first() == Some(&0x1c) && data.windows(3).any(|w| w == [0x83, 0x66, 0xcd]))
}

fn is_in_1c_stub(data: &[u8]) -> bool {
    data.len() == 36
        && data.first() == Some(&0x1c)
        && data.windows(3).any(|w| w == [0x83, 0x66, 0xcd])
}

fn looks_like_second_pull_reply(data: &[u8]) -> bool {
    bulk_looks_like_assign_response(data)
        || data.len() >= 48
        || is_in_1c_stub(data)
}

fn frame_has_assign_marker(data: &[u8]) -> bool {
    data.windows(3).any(|w| w == [0x83, 0x66, 0xcd])
}

fn best_module_hex_from_frames(frames: &[Vec<u8>]) -> Option<Option<String>> {
    let mut best: Option<(usize, String)> = None;
    for f in frames {
        if !frame_has_assign_marker(f) {
            continue;
        }
        let hex = extract_first_module_hex_from_bulk(f);
        let score = f.len();
        match (&best, &hex) {
            (None, Some(h)) => best = Some((score, h.clone())),
            (Some((prev_score, _)), Some(h)) if score > *prev_score => {
                best = Some((score, h.clone()));
            }
            _ => {}
        }
    }
    best.map(|(_, h)| Some(h))
}

fn log_capture_summary(frames: &[Vec<u8>]) {
    let parts: Vec<String> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let head: String = f
                .iter()
                .take(4)
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join("");
            let has8366 = frame_has_assign_marker(f);
            format!("#{i} len={} head={head} 8366={has8366}", f.len())
        })
        .collect();
    pull_trace(&format!(
        "finalize: aucun bulk assignable ({} trames) : {}",
        frames.len(),
        parts.join(" | ")
    ));
    if let Some(f) = frames.iter().find(|f| is_in_1c_stub(f)) {
        let hex: String = f.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
        let lane = cd_lane_byte(f).map(|b| format!("{b:02x}")).unwrap_or_else(|| "?".into());
        pull_trace(&format!("échantillon IN 1c 36o (cd lane={lane}) : {hex}"));
    }
}

fn arm_pull_capture(state: &mut HelixState, slot_bus: u8) {
    state.hw_model_pull_capture.clear();
    state.hw_model_pull_slot_bus = Some(slot_bus);
    state.hw_model_pull_step = 1;
    state.hw_model_pull_cd_lane = None;
    state.hw_model_pull_echo_double = None;
    state.hw_model_pull_retried_1b = false;
    state.hw_model_pull_capture_deadline =
        Some(Instant::now() + Duration::from_millis(PULL_CAPTURE_MS));
}

fn remember_cd_lane_from_in(state: &mut HelixState, data: &[u8]) {
    // Ne pas recopier le lane/double d’un stub `1c` (rejet) sur les OUT suivants.
    if is_in_1c_stub(data) {
        pull_trace("IN 1c stub — cd_lane/double non mémorisés");
        return;
    }
    if let Some(lane) = cd_lane_byte(data) {
        state.hw_model_pull_cd_lane = Some(lane);
        pull_trace(&format!("IN cd lane={lane:02x}"));
    }
    if let Some(d) = preset_double_after_cd(data) {
        state.hw_model_pull_echo_double = Some(d);
        pull_trace(&format!("IN echo double {:02x}:{:02x}", d[0], d[1]));
    }
}

/// Deux octets après `83:66:cd:PP` (ex. IN `1c` Linux : `… cd 02 f7 67 …`).
fn preset_double_after_cd(data: &[u8]) -> Option<[u8; 2]> {
    for i in 0..data.len().saturating_sub(5) {
        if data[i] == 0x83 && data[i + 1] == 0x66 && data[i + 2] == 0xcd {
            return Some([data[i + 4], data[i + 5]]);
        }
    }
    None
}

/// Double pour les OUT pull `1b`/`19` (octets 28–29, lane éditeur 0x64xx).
/// Utilise `editor_ed03_double` — distinct de preset_dump_ack_ctr et live_write_ctr.
fn pull_preset_double(state: &HelixState) -> [u8; 2] {
    // Lane éditeur (0x64xx) — toujours, indépendamment du dernier ACK dump
    state.editor_ed03_double_val()
}

fn cd_lane_for_out(state: &HelixState) -> u8 {
    state.hw_model_pull_cd_lane.unwrap_or(0x03)
}

/// Lit `live_write_ctr` pour les octets 12–13 (valeur au moment de la construction du paquet).
fn pull_ctr_bytes(state: &HelixState) -> (u8, u8) {
    let ctr = state.live_write_ctr;
    ((ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8)
}

/// Simule l’avancement du CTR entre deux OUT du pull (voir `PULL_CTR_DELTA_*` — pas +0x1F systématique).
fn advance_pull_ctr(state: &mut HelixState, delta: u16) {
    state.live_write_ctr = state.live_write_ctr.wrapping_add(delta);
}

/// OUT pull : `slot_bus` = slot actif host (via [`slot_bus_for_model_pull`]), placé aux octets `81:62`.
fn build_pull_1b(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    let cnt0 = state.next_x80_cnt();
    let d0 = pull_preset_double(state);
    let cd_lane = cd_lane_for_out(state);
    let (ctr_lo, ctr_hi) = pull_ctr_bytes(state);
    advance_pull_ctr(state, PULL_CTR_DELTA_AFTER_1B);
    vec![
        0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt0, 0x00, 0x04, ctr_lo, ctr_hi, 0x00,
        0x00, 0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, cd_lane, d0[0], d0[1],
        0x2d, 0x65, 0x81, 0x62, slot_bus, 0x00,
    ]
}

/// Court `f0:03` entre `1b` et le premier bulk IN (capture HX Edit #515).
fn build_pull_f0_interstitial(state: &mut HelixState) -> Vec<u8> {
    let cnt = state.next_x2_cnt();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03, 0x00, cnt, 0x00, 0x08, 0x52, 0x11, 0x00,
        0x00,
    ]
}

fn build_pull_19(state: &mut HelixState, second: bool) -> Vec<u8> {
    let cnt = state.next_x80_cnt();
    let d = pull_preset_double(state);
    let cd_lane = cd_lane_for_out(state);
    let (ctr_lo, ctr_hi) = pull_ctr_bytes(state);
    advance_pull_ctr(state, PULL_CTR_DELTA_AFTER_19);
    // Capture HX : octet avant `65` = `17` (1er 19) / `16` (2e 19) — pas le CTR.
    let pre_65 = if second { 0x16 } else { 0x17 };
    vec![
        0x19, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt, 0x00, 0x0c, ctr_lo, ctr_hi, 0x00,
        0x00, 0x01, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, cd_lane, d[0], d[1],
        pre_65, 0x65, 0xc0, 0x00, 0x00, 0x00,
    ]
}

fn send_pull_19_first(state: &mut HelixState, _in_data: &[u8]) {
    let pkt = build_pull_19(state, false);
    state.send(OutPacket::with_delay(pkt, INTER_19_DELAY_MS));
    state.hw_model_pull_step = 2;
    pull_trace("OUT 19 #1 (réponse IN 53 reçue)");
}

fn send_pull_19_second(state: &mut HelixState) {
    let pkt = build_pull_19(state, true);
    state.send(OutPacket::with_delay(pkt, INTER_19_DELAY_MS));
    state.hw_model_pull_step = 3;
    pull_trace("OUT 19 #2 (réponse IN méta 1 reçue)");
}

fn send_pull_1b_f0_burst(state: &mut HelixState, slot_bus: u8, label: &str) {
    let pkt1b = build_pull_1b(state, slot_bus);
    let cnt1b = pkt1b[9];
    let cd_lane = pkt1b[27];
    let preset_d0 = pkt1b.get(28).copied().unwrap_or(0);
    let preset_d1 = pkt1b.get(29).copied().unwrap_or(0);
    let pkt_f0 = build_pull_f0_interstitial(state);
    let cnt_f0 = pkt_f0[9];
    let f0_sub = pkt_f0.get(11).copied().unwrap_or(0);
    state.send(OutPacket::with_tail_burst(pkt1b, vec![pkt_f0]));
    pull_trace(&format!(
        "{label} slot_bus={slot_bus:02x} x80_1b={cnt1b:02x} cd_lane={cd_lane:02x} preset_double={preset_d0:02x}:{preset_d1:02x} x2_f0={cnt_f0:02x} f0_sub={f0_sub:02x}"
    ));
}

/// Démarre le pull : `1b` puis court `f0:03` (capture HX Edit), puis attend les IN.
pub fn send_pull_sequence(state: &mut HelixState, slot_bus: u8) {
    arm_pull_capture(state, slot_bus);
    send_pull_1b_f0_burst(state, slot_bus, "sent 1b+f0 (burst USB)");
}

fn try_early_finalize(state: &mut HelixState) -> Option<SlotModelHwChangedPayload> {
    let frames = &state.hw_model_pull_capture;
    if let Some(Some(hex)) = best_module_hex_from_frames(frames) {
        return finalize_pull_capture(state, None).map(|mut p| {
            p.module_hex = Some(hex);
            p
        });
    }
    None
}

fn finalize_pull_capture(
    state: &mut HelixState,
    extra: Option<&[u8]>,
) -> Option<SlotModelHwChangedPayload> {
    let slot_bus = state.hw_model_pull_slot_bus.take()?;
    state.hw_model_pull_capture_deadline = None;
    state.hw_model_pull_step = 0;
    state.hw_model_pull_cd_lane = None;
    state.hw_model_pull_echo_double = None;
    state.hw_model_pull_retried_1b = false;

    let mut frames = Vec::new();
    std::mem::swap(&mut frames, &mut state.hw_model_pull_capture);
    if let Some(e) = extra {
        if frames.len() < PULL_CAPTURE_MAX_FRAMES {
            frames.push(e.to_vec());
        }
    }

    let Some(module_hex) = best_module_hex_from_frames(&frames) else {
        if slot_model_hw_pull_debug_enabled() {
            log_capture_summary(&frames);
        }
        return None;
    };
    let slot_index = slot_bus_to_kempline_index(slot_bus)? as u32;

    if let Some(ref hex) = module_hex {
        log_hw_model_changed(hex);
    }

    state.hw_slot_content_sequence = state.hw_slot_content_sequence.wrapping_add(1);
    let sequence = state.hw_slot_content_sequence;

    Some(SlotModelHwChangedPayload {
        sequence,
        slot_index,
        slot_bus,
        module_hex,
    })
}

fn ingest_pull_capture(
    state: &mut HelixState,
    data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    let deadline = state.hw_model_pull_capture_deadline?;
    let now = Instant::now();

    if state.hw_model_pull_capture.len() < PULL_CAPTURE_MAX_FRAMES {
        state.hw_model_pull_capture.push(data.to_vec());
    }
    pull_trace_hex("capture IN", data);
    remember_cd_lane_from_in(state, data);

    match state.hw_model_pull_step {
        1 if looks_like_first_pull_reply(data) => {
            if is_in_1c_stub(data) {
                let lane = cd_lane_byte(data).map(|b| format!("{b:02x}")).unwrap_or_else(|| "?".into());
                pull_trace(&format!(
                    "IN 1c 36o (cd lane={lane}) — attendu IN 53 ; pas de retry 1b ni OUT 19 (HX Edit)"
                ));
            } else {
                send_pull_19_first(state, data);
            }
        }
        2 if looks_like_second_pull_reply(data) => {
            send_pull_19_second(state);
        }
        _ => {}
    }

    if let Some(payload) = try_early_finalize(state) {
        return Some(payload);
    }

    if state.hw_model_pull_step >= 3 && data.len() >= 200 {
        return finalize_pull_capture(state, None);
    }

    if now >= deadline {
        return finalize_pull_capture(state, Some(data));
    }

    None
}

pub fn ingest_slot_model_hw_in(
    state: &mut HelixState,
    data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    if is_hw_model_change_notify_loose(data) {
        pull_trace_hex("notify IN", data);
        if data[0] == 0x1d {
            pull_trace(
                "notify 1d — ignorée (paire modèle ; pull uniquement sur 1f, même batch USB possible)",
            );
        } else if is_hw_model_pull_trigger_notify(data) {
            ingest_slot_model_hw_notify(state, data);
        }
        return None;
    }

    if state.hw_model_pull_capture_deadline.is_some() {
        return ingest_pull_capture(state, data);
    }

    None
}

fn ingest_slot_model_hw_notify(state: &mut HelixState, data: &[u8]) -> bool {
    debug_assert_eq!(data.first(), Some(&0x1f));
    if state.preset_content_only {
        pull_trace("notify ignorée : preset_content_only");
        return true;
    }
    let now = Instant::now();
    if let Some(last) = state.hw_model_pull_last_at {
        if now.duration_since(last) < Duration::from_millis(PULL_DEBOUNCE_MS) {
            pull_trace(&format!("debounced 1f (within {} ms)", PULL_DEBOUNCE_MS));
            return true;
        }
    }
    let Some(slot_bus) = slot_bus_for_model_pull(state, data) else {
        pull_trace("notify 0x1f : pas de 81:62 dans la trame ni slot hw_active — ignorée");
        return true;
    };
    // Si le pull a utilisé le secours `81:62` (hw_active vide), aligner l’état unique slot actif.
    if active_effect_slot_bus(state).is_none() {
        if let Some(idx) = slot_bus_to_kempline_index(slot_bus) {
            state.hw_active_slot_index = Some(idx);
            state.hw_active_slot_bus = Some(slot_bus);
        }
    }
    if state.hw_model_pull_capture_deadline.is_some() {
        pull_trace("pull déjà en cours — notify 1f ignorée");
        return true;
    }
    pull_trace(&format!(
        "pull slot_bus={:02x} (kempline {:?}) depuis notif 0x1f",
        slot_bus,
        slot_bus_to_kempline_index(slot_bus),
    ));
    send_pull_sequence(state, slot_bus);
    state.hw_model_pull_last_at = Some(now);
    true
}

pub fn init_slot_model_hw_pull_debug_from_env() {
    if env::var_os("HX_SLOT_MODEL_HW_PULL_DEBUG").is_some_and(|v| {
        v.to_str().is_some_and(|s| {
            s == "1" || s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes")
        })
    }) {
        set_slot_model_hw_pull_debug(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IN1D: &[u8] = &[
        0x1d, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0xc0, 0x00, 0x04, 0x09, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x82, 0x69, 0x31, 0x6a, 0x84, 0x52,
        0x00, 0x44, 0x05, 0x79, 0x0a, 0x6a, 0x81, 0x62, 0x01, 0x93,
    ];
    const IN1F: &[u8] = &[
        0x1f, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0xc1, 0x00, 0x04, 0x09, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x82, 0x69, 0x31, 0x6a, 0x84, 0x52,
        0x00, 0x44, 0x05, 0x79, 0x0a, 0x6a, 0x81, 0x62, 0x01, 0x93,
    ];

    const IN92_ASSIGN: &[u8] = &[
        0x53, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0xf9, 0x00, 0x04, 0x88, 0x03, 0x00,
        0x00, 0x00, 0x00, 0x06, 0x00, 0x43, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0xfc, 0x67,
        0x00, 0x68, 0x82, 0x0d, 0x01, 0x18, 0x82, 0x13, 0x06, 0x14, 0x85, 0x18, 0x83, 0x17, 0xc2,
        0x19, 0xcd, 0x01, 0xfe, 0x1a, 0xff, 0x09, 0x01, 0x0a, 0xc3, 0x0b, 0x83, 0x02, 0x04, 0x03,
        0x04, 0x04, 0x94,
    ];

    #[test]
    fn detects_1d_and_1f_notify() {
        assert!(is_hw_model_change_notify_40(IN1D));
        assert!(is_hw_model_change_notify_40(IN1F));
        assert!(!is_hw_model_change_notify_1f(IN1D));
        assert!(is_hw_model_change_notify_1f(IN1F));
    }

    #[test]
    fn parses_slot_bus_from_1f() {
        assert_eq!(parse_slot_bus_from_model_notify(IN1F), Some(0x01));
    }

    #[test]
    fn slot_bus_for_pull_prefers_hw_active_over_notify_8162() {
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x02);
        state.hw_active_slot_index = Some(1);
        assert_eq!(slot_bus_for_model_pull(&state, IN1F), Some(0x02));
    }

    #[test]
    fn slot_bus_for_pull_falls_back_to_8162_when_hw_active_none() {
        let state = HelixState::new();
        assert_eq!(slot_bus_for_model_pull(&state, IN1F), Some(0x01));
    }

    #[test]
    fn extracts_module_hex_from_capture_bulk() {
        assert_eq!(
            extract_first_module_hex_from_bulk(IN92_ASSIGN).as_deref(),
            Some("cd01fe")
        );
    }

    #[test]
    fn ingest_1d_does_not_trigger_pull() {
        let mut state = HelixState::new();
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        ingest_slot_model_hw_in(&mut state, IN1D);
        assert!(state.hw_model_pull_last_at.is_none());
        assert!(state.hw_model_pull_capture_deadline.is_none());
        assert_eq!(state.hw_model_pull_step, 0);
    }

    #[test]
    fn ingest_21_preset_frame_does_not_trigger_pull() {
        const IN21: &[u8] = &[
            0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x29, 0x00, 0x04, 0x09, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x82, 0x69, 0x27, 0x6a, 0x84, 0x52,
            0x01, 0x44, 0x03, 0x79, 0x13, 0x6a, 0x82, 0x62, 0x01, 0x1a, 0x00, 0x00, 0x21, 0x37,
        ];
        let mut state = HelixState::new();
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        ingest_slot_model_hw_in(&mut state, IN21);
        assert!(state.hw_model_pull_capture_deadline.is_none());
    }

    #[test]
    fn ingest_1f_arms_capture() {
        let mut state = HelixState::new();
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        assert!(ingest_slot_model_hw_in(&mut state, IN1F).is_none());
        assert!(state.hw_model_pull_last_at.is_some());
        assert_eq!(state.hw_model_pull_step, 1);
    }

    #[test]
    fn ingest_1f_without_8162_in_notify_and_no_hw_active_does_not_send() {
        let mut state = HelixState::new();
        state.preset_content_only = false;
        let mut no_slot = IN1F.to_vec();
        // Retire le marqueur `81:62:01` — sans lui ni hw_active, pas de pull.
        if let Some(i) = no_slot
            .windows(3)
            .position(|w| w == [0x81, 0x62, 0x01])
        {
            no_slot[i..i + 3].copy_from_slice(&[0x00, 0x00, 0x00]);
        }
        ingest_slot_model_hw_in(&mut state, &no_slot);
        assert!(state.hw_model_pull_last_at.is_none());
    }

    #[test]
    fn pull_19_uses_live_write_ctr_and_advances() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x413f;
        let p1 = build_pull_19(&mut state, false);
        assert_eq!(p1[12], 0x3f);
        assert_eq!(p1[13], 0x41);
        assert_eq!(state.live_write_ctr, 0x4170); // +0x31
        let p2 = build_pull_19(&mut state, true);
        assert_eq!(p2[12], 0x70);
        assert_eq!(p2[13], 0x41);
        assert_eq!(state.live_write_ctr, 0x41a1); // +0x31
    }

    #[test]
    fn ingest_1d_then_1f_single_pull() {
        let mut state = HelixState::new();
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        ingest_slot_model_hw_in(&mut state, IN1D);
        assert!(state.hw_model_pull_capture_deadline.is_none());
        ingest_slot_model_hw_in(&mut state, IN1F);
        assert_eq!(state.hw_model_pull_step, 1);
        assert!(state.hw_model_pull_capture_deadline.is_some());
    }

    #[test]
    fn capture_finalize_on_assign_bulk() {
        let mut state = HelixState::new();
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        ingest_slot_model_hw_in(&mut state, IN1F);
        let payload = ingest_slot_model_hw_in(&mut state, IN92_ASSIGN).expect("payload");
        assert_eq!(payload.slot_index, 0);
        assert_eq!(payload.module_hex.as_deref(), Some("cd01fe"));
        assert_eq!(state.hw_model_pull_step, 0);
    }
}
