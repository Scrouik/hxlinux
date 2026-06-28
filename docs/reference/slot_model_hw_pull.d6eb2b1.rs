//! Changement de modèle sur le **slot actif** hardware (captures HX Edit
//! `Slot0_Change_Model_2_Time.json`).
//!
//! Flux cible (sans `preset_data`) :
//! 1. IN `1d` puis `1f` 40 o (`f0:03:02:10`) — souvent **même lot de lecture USB** (même timestamp).
//!    **Un seul** pull host sur `1f` ; `1d` ignoré (même lot USB). Après un pull, le settling
//!    post-pull (272 dump) retarde le prochain `1b` — pas de debounce temps sur `1f`.
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
use crate::helix::packet::{byte_cmp, OutPacket};
use crate::pattern;

/// Délai entre les deux `19` (après réponse IN). `1b`+`f0` = rafale sans attente (HX Edit ~0,02 ms).
const INTER_19_DELAY_MS: u64 = 4;
/// Pas de nouveau pull juste après un pull terminé (scroll rapide sur le knob).
/// Les `1f` pendant cette fenêtre sont ignorés (pas de `1b` depuis un scroll périmé).
const PULL_COOLDOWN_AFTER_DONE_MS: u64 = 40;
/// Après finalize : pas de pull pending ni d’ACK `1d` (évite 272 tardif pris pour 1ʳᵉ réponse `1b`).
const PULL_POST_FINALIZE_QUIET_MS: u64 = 85;
/// Silence requis après un pull réussi : pas de nouveau `1b` tant que des 272 dump arrivent encore.
const PULL_POST_PULL_SETTLING_MS: u64 = 50;
/// Pas de `request_preset_content` pendant/après scroll HW (évite dump USB vs ACK 1d).
pub const HW_MODEL_USB_BUSY_AFTER_SCROLL_MS: u64 = 700;
/// Attente du bulk ~272 o après `19` #2 (captures HX Edit).
const PULL_CAPTURE_MS: u64 = 600;
const PULL_CAPTURE_MAX_FRAMES: usize = 48;

// --- `hw_model_pull_ctr` (octets 12–13, LE) sur les OUT `80:10:ed:03` 36 o du pull modèle ---
//
// Lane **distincte** de `live_write_ctr` (live write + sonde UI `edit_slot_model.rs`) :
//
// | Contexte | Pas typique | Où c’est codé |
// |----------|-------------|---------------|
// | Write paramètre / sonde UI | **+0x1F** | `live_write.rs`, `edit_slot_model.rs` |
// | Pull : `1b` → 1er `19` | **+0x4B** | ci-dessous |
// | Pull : 1er `19` → 2e `19` | **+0x31** | ci-dessous |
//
// Réf. capture : `Slot0_Change_Model_2_Time.json` (`3f:41` → `8a:41` → `bb:41`, etc.).

/// Après envoi du `1b` : avance le CTR avant de construire le 1er `19` (délai réel + trafic intermédiaire).
const PULL_CTR_DELTA_AFTER_1B: u16 = 0x004b;

/// Entre les deux `19` consécutifs : pas observé **+0x31** (49 déc.), pas le +0x1F du live write.
const PULL_CTR_DELTA_AFTER_19: u16 = 0x0031;

// --- `hw_model_scroll_ack_ctr` (octets 12–13, LE) sur le court OUT `f0:03 sub=08` 16 o ---
//
// Même lane que les ACK scroll `1d`/`1f` (`ack_hw_model_scroll_in`, `mod::hw_model_scroll_ack_step`).
// Capture `3_scroll_HXEdit.json` :
//
// | Étape HX | Δ u16 LE | Rôle chez HXLinux |
// |----------|----------|-------------------|
// | `f0` après `1b` (pull) | — (valeur courante) | `build_pull_f0_interstitial` |
// | `f0` après 19 #1 | **+0x2e** | `advance_scroll_ack_after_pull_interstitial_f0` avant 19 #1 |
// | ACK `f0` après IN `1f` | **+0x17** | `next_hw_model_scroll_ack_double(0x1f)` |
// | ACK `f0` après IN `1d` | **+0x15** | `next_hw_model_scroll_ack_double(0x1d)` |
// | Entre deux pulls (après `1f` préc.) | **+0x15** typ. | ACK `1d` hors capture pull |
//
/// HX envoie un 2ᵉ `f0` interstitiel après 19 #1 ; on n’envoie qu’un seul `f0` avec le `1b`.
const PULL_SCROLL_ACK_ADVANCE_AFTER_INTERSTITIAL_F0: u16 = 0x002e;

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

/// USB occupé par scroll modèle / pull — ne pas lancer un dump preset UI en parallèle.
pub fn hw_model_usb_busy(state: &HelixState) -> bool {
    if state.init_usb_settle_active() {
        return true;
    }
    // Pendant RequestPreset* / RequestPresetNames : pas de « busy scroll » (rafale `1d` firmware).
    if state.preset_usb_read_in_progress() {
        return false;
    }
    if state.hw_model_pull_capture_deadline.is_some() {
        return true;
    }
    if let Some(t) = state.hw_model_last_scroll_in_at {
        if t.elapsed() < Duration::from_millis(HW_MODEL_USB_BUSY_AFTER_SCROLL_MS) {
            return true;
        }
    }
    if let Some(t) = state.hw_model_pull_last_at {
        if t.elapsed() < Duration::from_millis(PULL_COOLDOWN_AFTER_DONE_MS) {
            return true;
        }
    }
    false
}

/// Mêmes garde-fous que [`ingest_slot_model_hw_notify`] : ce `1f` va lancer un `1b`+`f0` pull.
pub fn would_start_hw_model_pull_on_1f(state: &HelixState, data: &[u8]) -> bool {
    if !is_hw_model_pull_trigger_notify(data) {
        return false;
    }
    if state.init_usb_settle_active()
        || state.preset_usb_read_in_progress()
        || state.preset_content_only
    {
        return false;
    }
    if slot_bus_for_model_pull(state, data).is_none() {
        return false;
    }
    if post_pull_stream_settling_active(state) {
        return false;
    }
    if state.hw_model_pull_capture_deadline.is_some() {
        return false;
    }
    true
}

fn should_defer_model_notify_1d_ack(state: &HelixState, data: &[u8]) -> bool {
    data.first() == Some(&0x1d)
        && is_hw_model_change_notify_loose(data)
        && state.should_ack_firmware_1d_notify()
        && state.hw_model_pull_capture_deadline.is_none()
        && !state
            .hw_model_pull_quiet_until
            .is_some_and(|t| Instant::now() < t)
}

/// Envoie l’ACK scroll et avance [`HelixState::hw_model_scroll_ack_ctr`].
fn send_hw_model_scroll_ack(state: &mut HelixState, data: &[u8]) -> bool {
    let head = data.first().copied().unwrap_or(0);
    debug_assert!(head == 0x1d || head == 0x1f);
    if state.preset_data_ready {
        state.hw_model_last_scroll_in_at = Some(Instant::now());
    }
    let cnt = state.next_x2_cnt();
    if head == 0x1f && is_hw_model_slot_cleared_notify(data) {
        state.hw_model_scroll_skip_inc_once = true;
    }
    let double = state.next_hw_model_scroll_ack_double(head);
    if slot_model_hw_pull_debug_enabled() {
        pull_trace(&format!(
            "ACK OUT f0:03 sub=08 pour IN {head:#x} len={}",
            data.len()
        ));
    }
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08,
        double[0], double[1], 0x00, 0x00,
    ]));
    if head == 0x1d {
        crate::helix::init_trace::trace_1d_ack_decision(true, "ACK f0:03 sub=08");
    }
    true
}

/// ACK `f0:03` sub=08 sur notifs firmware `1d` / `1f` 40 o (molette **ou** sync preset/slots).
/// L’IN `21` 44 o post-assign (stomp → host) est **unidirectionnel** : pas d’ACK host.
/// Politique `1d` : ACK en Standard / init settle ; pas d’ACK pendant RequestPreset* ni pendant
/// un pull modèle actif (les `1d` en rafale pendant `1b`/`19` saturent la file OUT — cf. crash scroll).
/// **`1f`** : ACK hors lecture preset (fin de paire — le Stomp attend la réponse).
/// **Pull** : HX Edit n’ACK pas la paire `1d`/`1f` déclencheuse avant le `1b` — lane inchangée pour le `f0` interstitiel.
pub fn ack_hw_model_scroll_in(state: &mut HelixState, data: &[u8]) -> bool {
    let head = data.first().copied().unwrap_or(0);
    if head == 0x21 {
        if is_hw_model_post_assign_21(data) {
            pull_trace("notify 21 post-assign — ignorée (pas d’ACK host)");
        }
        return false;
    }
    if head != 0x1d && head != 0x1f || data.len() != 40 {
        return false;
    }
    if !byte_cmp(
        data,
        &pattern![
            XX, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x04,
            0x09, 0x02
        ],
        14,
    ) {
        return false;
    }
    if head == 0x1d && !state.should_ack_firmware_1d_notify() {
        crate::helix::init_trace::trace_1d_ack_decision(
            false,
            "suppress_1d ou init settle (pas d'ACK scroll)",
        );
        pull_trace("1d sans ACK (mode lecture preset — sync firmware, pas molette)");
        return false;
    }
    if head == 0x1d
        && (state.hw_model_pull_capture_deadline.is_some()
            || state
                .hw_model_pull_quiet_until
                .is_some_and(|t| Instant::now() < t))
    {
        let reason = if state.hw_model_pull_capture_deadline.is_some() {
            "pull modèle actif"
        } else {
            "post-finalize quiet"
        };
        crate::helix::init_trace::trace_1d_ack_decision(false, reason);
        pull_trace(&format!("1d sans ACK ({reason} — évite rafale OUT sub=08)"));
        return false;
    }

    if head == 0x1d && should_defer_model_notify_1d_ack(state, data) {
        if let Some(prev) = state.hw_model_scroll_deferred_1d.take() {
            let _ = send_hw_model_scroll_ack(state, &prev);
        }
        state.hw_model_scroll_deferred_1d = Some(data.to_vec());
        pull_trace("1d modèle — ACK différé (paire avant pull possible, HX Edit)");
        return false;
    }

    if head == 0x1f {
        if would_start_hw_model_pull_on_1f(state, data) {
            state.hw_model_scroll_deferred_1d = None;
            pull_trace("1f pull — pas d'ACK scroll avant 1b (lane inchangée, HX Edit)");
            return false;
        }
        if let Some(d1d) = state.hw_model_scroll_deferred_1d.take() {
            let _ = send_hw_model_scroll_ack(state, &d1d);
        }
    }

    send_hw_model_scroll_ack(state, data)
}

/// IN `21` 44 o stomp « modèle enregistré sur ce slot » (host ne répond pas).
pub fn is_hw_model_post_assign_21(data: &[u8]) -> bool {
    data.len() == 44
        && data.first() == Some(&0x21)
        && data.get(24..28) == Some(&[0x82, 0x69, 0x27, 0x6a])
        && data.windows(3).any(|w| w == [0x82, 0x62, 0x01, 0x1a])
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

/// Suffixe IN `1f` « None » (frame 1315, `change_model_to_none_HW_HXEdit.json`, t≈2.593 s).
/// Distinct de l’assign (`05:79:0a:6a`) ; le scroll intermédiaire reste en `1d` + `82:69:16:6a`.
const HW_MODEL_NONE_NOTIFY_MARK: &[u8] =
    &[0x82, 0x69, 0x31, 0x6a, 0x84, 0x52, 0x00, 0x44, 0x05, 0x79, 0x0e, 0x6a];

pub fn is_hw_model_slot_cleared_notify(data: &[u8]) -> bool {
    is_hw_model_change_notify_1f(data)
        && data
            .windows(HW_MODEL_NONE_NOTIFY_MARK.len())
            .any(|w| w == HW_MODEL_NONE_NOTIFY_MARK)
}

/// Seule entrée valide pour démarrer le pull (pas `1d`, pas les IN `21` 44 o preset/focus).
pub fn is_hw_model_pull_trigger_notify(data: &[u8]) -> bool {
    is_hw_model_change_notify_1f(data) && !is_hw_model_slot_cleared_notify(data)
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

/// Bulk final ~272 o (3ᵉ IN du pull) — ne doit pas déclencher `19` #1 / #2.
fn is_pull_final_meta_bulk(data: &[u8]) -> bool {
    data.len() >= 180
}

fn looks_like_first_pull_reply(data: &[u8]) -> bool {
    if is_pull_final_meta_bulk(data) {
        return false;
    }
    matches!(data.first(), Some(0x53) | Some(0x51) | Some(0x47) | Some(0x6c) | Some(0x50))
        || (data.len() >= 90 && data.len() < 120 && data.first() == Some(&0x56))
        || (data.len() >= 80 && data.len() < 120 && bulk_looks_like_assign_response(data))
        || is_in_1c_stub(data)
}

fn is_in_1c_stub(data: &[u8]) -> bool {
    data.len() == 36
        && data.first() == Some(&0x1c)
        && data.windows(3).any(|w| w == [0x83, 0x66, 0xcd])
}

fn looks_like_second_pull_reply(data: &[u8]) -> bool {
    if is_pull_final_meta_bulk(data) {
        return false;
    }
    data.first() == Some(&0x39)
        || is_in_1c_stub(data)
        || (data.len() >= 48 && data.len() < 120 && bulk_looks_like_assign_response(data))
}

fn arm_pull_post_finalize_quiet(state: &mut HelixState) {
    state.hw_model_pull_quiet_until =
        Some(Instant::now() + Duration::from_millis(PULL_POST_FINALIZE_QUIET_MS));
}

fn arm_post_pull_stream_settling(state: &mut HelixState) {
    state.hw_model_post_pull_settling = true;
    state.hw_model_post_pull_deadline =
        Some(Instant::now() + Duration::from_millis(PULL_POST_PULL_SETTLING_MS));
}

fn touch_post_pull_stream_settling(state: &mut HelixState) {
    state.hw_model_post_pull_settling = true;
    state.hw_model_post_pull_deadline =
        Some(Instant::now() + Duration::from_millis(PULL_POST_PULL_SETTLING_MS));
}

pub(crate) fn post_pull_stream_settling_active(state: &HelixState) -> bool {
    state.hw_model_post_pull_settling
        && state
            .hw_model_post_pull_deadline
            .is_some_and(|t| Instant::now() < t)
}

/// Fin de la fenêtre settling ; retourne `true` si elle vient d’expirer.
fn tick_post_pull_stream_settling(state: &mut HelixState) -> bool {
    if !state.hw_model_post_pull_settling {
        return false;
    }
    let Some(deadline) = state.hw_model_post_pull_deadline else {
        state.hw_model_post_pull_settling = false;
        return false;
    };
    if Instant::now() < deadline {
        return false;
    }
    state.hw_model_post_pull_settling = false;
    state.hw_model_post_pull_deadline = None;
    pull_trace("post-pull settling terminé (plus de 272 dump)");
    true
}

/// Fin de settling : ne pas envoyer de `1b` depuis un `1f` vieux (le Stomp a pu scroller pendant les 272).
fn clear_stale_pending_after_post_pull_settling(state: &mut HelixState) {
    let Some(slot_bus) = state.hw_model_pull_pending_slot_bus.take() else {
        return;
    };
    pull_trace(&format!(
        "pending après settling slot_bus={slot_bus:02x} abandonné — attendre prochain 1f (scroll frais)"
    ));
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
    // Ne pas effacer cd lane `04` post-wrap (sinon le pull suivant repart en `03` → échec).
    if state.hw_model_pull_cd_lane != Some(0x04) {
        state.hw_model_pull_cd_lane = None;
    }
    state.hw_model_pull_echo_double = None;
    state.hw_model_pull_retried_1b = false;
    state.hw_model_pull_saw_final_bulk = false;
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
        // Ne pas rétrograder `04` → `03` (certains IN post-pull gardent `cd:03` dans le bulk).
        if state.hw_model_pull_cd_lane != Some(0x04) || lane == 0x04 {
            state.hw_model_pull_cd_lane = Some(lane);
            pull_trace(&format!("IN cd lane={lane:02x}"));
        }
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

fn cd_lane_for_out(state: &HelixState) -> u8 {
    state.hw_model_pull_cd_lane.unwrap_or(0x03)
}

/// HX Edit (`3_scroll_HXEdit.json`) : à partir du wrap bas `fd:64` → `00:64`, la voie `cd` passe
/// de `03` à `04` et y reste pour les pulls suivants.
fn cd_lane_for_hw_model_pull_out(state: &mut HelixState, prev_lo: u8, wire: [u8; 2]) -> u8 {
    if wire[0] < prev_lo {
        state.hw_model_pull_cd_lane = Some(0x04);
        pull_trace("editor double wrap bas → cd lane 04 (aligné HX Edit)");
    }
    cd_lane_for_out(state)
}

/// Lit `hw_model_pull_ctr` pour les octets 12–13 (valeur au moment de la construction du paquet).
fn pull_ctr_bytes(state: &HelixState) -> (u8, u8) {
    let ctr = state.hw_model_pull_ctr;
    ((ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8)
}

/// Avance la lane pull modèle (voir `PULL_CTR_DELTA_*` — pas +0x1F du live write).
fn advance_pull_ctr(state: &mut HelixState, delta: u16) {
    state.hw_model_pull_ctr = state.hw_model_pull_ctr.wrapping_add(delta);
}

/// OUT pull : `slot_bus` = slot actif host (via [`slot_bus_for_model_pull`]), placé aux octets `81:62`.
fn build_pull_1b(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    let cnt0 = state.next_x80_cnt();
    let prev_lo = (state.editor_ed03_double & 0xFF) as u8;
    let d0 = state.next_editor_ed03_double_for_hw_model_pull();
    let cd_lane = cd_lane_for_hw_model_pull_out(state, prev_lo, d0);
    let (ctr_lo, ctr_hi) = pull_ctr_bytes(state);
    advance_pull_ctr(state, PULL_CTR_DELTA_AFTER_1B);
    vec![
        0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, cnt0, 0x00, 0x04, ctr_lo, ctr_hi, 0x00,
        0x00, 0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, cd_lane, d0[0], d0[1],
        0x2d, 0x65, 0x81, 0x62, slot_bus, 0x00,
    ]
}

/// Court `f0:03` entre `1b` et le premier bulk IN (capture HX Edit #515).
/// Octets 12–13 = `hw_model_scroll_ack_ctr` (pas de pas ici : HX n’incrémente qu’au 2ᵉ `f0`, simulé avant 19 #1).
fn build_pull_f0_interstitial(state: &mut HelixState) -> Vec<u8> {
    let cnt = state.next_x2_cnt();
    let double = state.hw_model_scroll_ack_double();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03, 0x00, cnt, 0x00, 0x08, double[0], double[1],
        0x00, 0x00,
    ]
}

/// Compense le 2ᵉ `f0` HX (après 19 #1, +0x2e) qu’on n’émet pas.
fn advance_scroll_ack_after_pull_interstitial_f0(state: &mut HelixState) {
    state.hw_model_scroll_ack_ctr = state
        .hw_model_scroll_ack_ctr
        .wrapping_add(PULL_SCROLL_ACK_ADVANCE_AFTER_INTERSTITIAL_F0);
}

fn build_pull_19(state: &mut HelixState, second: bool) -> Vec<u8> {
    let cnt = state.next_x80_cnt();
    let d = state.editor_ed03_double_val();
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
    advance_scroll_ack_after_pull_interstitial_f0(state);
    let pkt = build_pull_19(state, false);
    state.send(OutPacket::with_delay(pkt, INTER_19_DELAY_MS));
    state.hw_model_pull_step = 2;
    pull_trace("OUT 19 #1 (1ʳᵉ réponse bulk assign)");
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
    let scroll_d0 = pkt_f0.get(12).copied().unwrap_or(0);
    let scroll_d1 = pkt_f0.get(13).copied().unwrap_or(0);
    state.send(OutPacket::with_tail_burst(pkt1b, vec![pkt_f0]));
    pull_trace(&format!(
        "{label} slot_bus={slot_bus:02x} x80_1b={cnt1b:02x} cd_lane={cd_lane:02x} preset_double={preset_d0:02x}:{preset_d1:02x} x2_f0={cnt_f0:02x} f0_sub={f0_sub:02x} scroll_ack_double={scroll_d0:02x}:{scroll_d1:02x}"
    ));
}

/// Démarre le pull : `1b` puis court `f0:03` (capture HX Edit), puis attend les IN.
pub fn send_pull_sequence(state: &mut HelixState, slot_bus: u8) {
    if state.init_usb_settle_active() {
        pull_trace("pull ignoré : init USB settle (ACK seulement)");
        return;
    }
    if post_pull_stream_settling_active(state) {
        pull_trace("pull ignoré : post-pull settling (272 dump en cours)");
        queue_pending_hw_model_pull(state, slot_bus);
        return;
    }
    arm_pull_capture(state, slot_bus);
    send_pull_1b_f0_burst(state, slot_bus, "sent 1b+f0 (burst USB)");
}

/// Reprend le trafic « idle » HX Edit après un pull (le keep-alive est suspendu pendant la capture).
fn send_post_pull_resume_traffic(state: &mut HelixState) {
    let cnt_x1 = state.next_x1_cnt();
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt_x1, 0x00, 0x08,
        0x72, 0x1e, 0x00, 0x00,
    ]));
    let cnt_x2 = state.next_x2_cnt();
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x02, 0x10, 0xf0, 0x03,
        0x00, cnt_x2, 0x00, 0x10,
        0x09, 0x10, 0x00, 0x00,
    ]));
    if slot_model_hw_pull_debug_enabled() {
        pull_trace("post-pull OUT ef:03 + f0:03 sub=10 (reprise keep-alive)");
    }
}

/// Écho IN 16 o après notre `f0` interstitiel du pull (`ed:03` / sub `08` / double) — à acquitter.
fn try_ack_pull_interstitial_echo(state: &mut HelixState, data: &[u8]) {
    if data.len() != 16 {
        return;
    }
    if data.get(4..8) != Some(&[0xed, 0x03, 0x80, 0x10]) || data.get(11) != Some(&0x08) {
        return;
    }
    let cnt = state.next_x2_cnt();
    let d0 = data.get(12).copied().unwrap_or(0);
    let d1 = data.get(13).copied().unwrap_or(0);
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08,
        d0, d1, 0x00, 0x00,
    ]));
    if slot_model_hw_pull_debug_enabled() {
        pull_trace(&format!(
            "ACK echo IN 16o post f0 interstitial pull (double={d0:02x}:{d1:02x})"
        ));
    }
}

fn queue_pending_hw_model_pull(state: &mut HelixState, slot_bus: u8) {
    state.hw_model_pull_pending_slot_bus = Some(slot_bus);
    pull_trace(&format!(
        "pull en file slot_bus={slot_bus:02x} (pull en cours ou cooldown)"
    ));
}

/// Relance un pull fileté (réservé ; le rattrapage se fait via le prochain `1f` utilisateur).
#[allow(dead_code)]
pub fn flush_pending_hw_model_pull(state: &mut HelixState) {
    let Some(slot_bus) = state.hw_model_pull_pending_slot_bus.take() else {
        return;
    };
    if state.init_usb_settle_active()
        || state.preset_usb_read_in_progress()
        || state.preset_content_only
        || state.hw_model_pull_capture_deadline.is_some()
        || state
            .hw_model_pull_quiet_until
            .is_some_and(|t| Instant::now() < t)
    {
        state.hw_model_pull_pending_slot_bus = Some(slot_bus);
        return;
    }
    if let Some(last) = state.hw_model_pull_last_at {
        if last.elapsed() < Duration::from_millis(PULL_COOLDOWN_AFTER_DONE_MS) {
            state.hw_model_pull_pending_slot_bus = Some(slot_bus);
            return;
        }
    }
    pull_trace(&format!("flush pull pending slot_bus={slot_bus:02x} (après quiet)"));
    state.hw_model_pull_pending_slot_bus = None;
    send_pull_sequence(state, slot_bus);
}

fn finalize_pull_capture(
    state: &mut HelixState,
    extra: Option<&[u8]>,
) -> Option<SlotModelHwChangedPayload> {
    let slot_bus = state.hw_model_pull_slot_bus.take()?;
    state.hw_model_pull_capture_deadline = None;
    state.hw_model_pull_step = 0;
    // Garder cd lane `04` après wrap bas — ne pas repasser à `03` au pull suivant (HX Edit).
    if state.hw_model_pull_cd_lane != Some(0x04) {
        state.hw_model_pull_cd_lane = None;
    }
    state.hw_model_pull_echo_double = None;
    state.hw_model_pull_retried_1b = false;
    state.hw_model_pull_saw_final_bulk = false;

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
        pull_trace("pull échoué (pas de bulk assignable) — reprise keep-alive, pending effacé");
        state.hw_model_pull_pending_slot_bus = None;
        send_post_pull_resume_traffic(state);
        arm_pull_post_finalize_quiet(state);
        arm_post_pull_stream_settling(state);
        state.hw_model_pull_last_at = Some(Instant::now());
        return None;
    };
    let Some(slot_index) = slot_bus_to_kempline_index(slot_bus).map(|i| i as u32) else {
        state.hw_model_pull_pending_slot_bus = None;
        arm_pull_post_finalize_quiet(state);
        state.hw_model_pull_last_at = Some(Instant::now());
        return None;
    };

    if let Some(ref hex) = module_hex {
        log_hw_model_changed(hex);
    }

    send_post_pull_resume_traffic(state);

    state.hw_slot_content_sequence = state.hw_slot_content_sequence.wrapping_add(1);
    let sequence = state.hw_slot_content_sequence;

    let payload = SlotModelHwChangedPayload {
        sequence,
        slot_index,
        slot_bus,
        module_hex,
    };
    arm_pull_post_finalize_quiet(state);
    arm_post_pull_stream_settling(state);
    if state.hw_model_pull_pending_slot_bus.is_some() {
        pull_trace("pending conservé — rattrapage au prochain 1f (pas de flush sur keep-alive/21)");
    }
    state.hw_model_pull_last_at = Some(Instant::now());
    Some(payload)
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
    if state.hw_model_pull_step == 1 {
        try_ack_pull_interstitial_echo(state, data);
    }

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

    // HX Edit : après `19` #2, le Stomp envoie encore un bulk ~272 o. Finaliser avant
    // (post-pull ef/f0) laisse le firmware bloqué — modèle affiché mais HW mort.
    if is_pull_final_meta_bulk(data) {
        state.hw_model_pull_saw_final_bulk = true;
        pull_trace(&format!(
            "IN bulk final len={} (après 19 #2)",
            data.len()
        ));
    } else if slot_model_hw_pull_debug_enabled() {
        if let Some(Some(hex)) = best_module_hex_from_frames(&state.hw_model_pull_capture) {
            if state.hw_model_pull_step >= 2 && !state.hw_model_pull_saw_final_bulk {
                pull_trace(&format!(
                    "hex={hex} visible step={} — attend bulk ~272 avant finalize",
                    state.hw_model_pull_step
                ));
            }
        }
    }

    if state.hw_model_pull_step >= 3 && state.hw_model_pull_saw_final_bulk {
        return finalize_pull_capture(state, None);
    }

    if now >= deadline {
        if state.hw_model_pull_step >= 3 && !state.hw_model_pull_saw_final_bulk {
            pull_trace(
                "timeout sans bulk 272 — finalize forcé (module_hex depuis 92/116 o si présent)",
            );
        }
        return finalize_pull_capture(state, Some(data));
    }

    None
}

pub fn ingest_slot_model_hw_in(
    state: &mut HelixState,
    data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    // Pas de flush ici : un keep-alive / `21` après quiet ne doit pas lancer un `1b` fantôme.
    // Le pending « pendant pull » est rattrapé au prochain `1f` utilisateur.
    if tick_post_pull_stream_settling(state) {
        clear_stale_pending_after_post_pull_settling(state);
    }

    if state.hw_model_pull_capture_deadline.is_none()
        && crate::helix::preset_dump_stream_ack::is_preset_dump_stream_chunk_in(data)
    {
        touch_post_pull_stream_settling(state);
        if slot_model_hw_pull_debug_enabled() {
            pull_trace(&format!(
                "chunk 272 post-finalize — prolonge settling ({} ms)",
                PULL_POST_PULL_SETTLING_MS
            ));
        }
        return None;
    }

    if is_hw_model_change_notify_loose(data) {
        pull_trace_hex("notify IN", data);
        if data[0] == 0x21 && is_hw_model_post_assign_21(data) {
            pull_trace("notify 21 post-assign — ignorée (stomp unidirectionnel)");
            return None;
        }
        if data[0] == 0x1d {
            arm_pull_post_finalize_quiet(state);                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                
            pull_trace(
                "notify 1d — ignorée (paire modèle ; pull uniquement sur 1f, même batch USB possible)",
            );
        } else if is_hw_model_slot_cleared_notify(data) {
            pull_trace("notify 1f slot None (05:79:0e:6a) — pas de pull");
            return emit_slot_cleared_from_hw_notify(state, data);
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

fn emit_slot_cleared_from_hw_notify(
    state: &mut HelixState,
    data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    let slot_bus = slot_bus_for_model_pull(state, data)?;
    let slot_index = slot_bus_to_kempline_index(slot_bus)? as u32;
    if active_effect_slot_bus(state).is_none() {
        state.hw_active_slot_index = Some(slot_bus as usize);
        state.hw_active_slot_bus = Some(slot_bus);
    }
    pull_trace(&format!(
        "slot None slot_bus={slot_bus:02x} (kempline {slot_index})"
    ));
    state.hw_slot_content_sequence = state.hw_slot_content_sequence.wrapping_add(1);
    Some(SlotModelHwChangedPayload {
        sequence: state.hw_slot_content_sequence,
        slot_index,
        slot_bus,
        module_hex: None,
    })
}

fn ingest_slot_model_hw_notify(state: &mut HelixState, data: &[u8]) -> bool {
    debug_assert_eq!(data.first(), Some(&0x1f));
    if state.init_usb_settle_active() {
        pull_trace("notify 1f ignorée : init USB settle (ACK seulement)");
        return true;
    }
    if state.preset_usb_read_in_progress() {
        pull_trace("notify 1f ignorée : lecture preset USB (pas de pull modèle)");
        return true;
    }
    if state.preset_content_only {
        pull_trace("notify ignorée : preset_content_only");
        return true;
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
    // Pas de debounce temps sur `1f` : le settling post-pull (272 dump) remplace ce rôle ;
    // ignorer un `1f` à 50 ms faisait rater le scroll réel du Stomp.
    if post_pull_stream_settling_active(state) {
        pull_trace(
            "post-pull settling — notify 1f ignorée (pas de file ; prochain 1f après silence 272)",
        );
        return true;
    }
    if state.hw_model_pull_capture_deadline.is_some() {
        pull_trace("pull déjà en cours — notify 1f → file");
        queue_pending_hw_model_pull(state, slot_bus);
        return true;
    }
    if state.hw_model_pull_pending_slot_bus.take().is_some() {
        pull_trace(&format!(
            "pending rattrapé par 1f slot_bus={slot_bus:02x} (kempline {:?})",
            slot_bus_to_kempline_index(slot_bus),
        ));
    }
    pull_trace(&format!(
        "pull slot_bus={:02x} (kempline {:?}) depuis notif 0x1f",
        slot_bus,
        slot_bus_to_kempline_index(slot_bus),
    ));
    send_pull_sequence(state, slot_bus);
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
    /// Frame 1315 — `change_model_to_none_HW_HXEdit.json` (assign → `0a:6a`, None → `0e:6a`).
    const IN1F_NONE: &[u8] = &[
        0x1f, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x34, 0x00, 0x04, 0x09, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x82, 0x69, 0x31, 0x6a, 0x84, 0x52,
        0x00, 0x44, 0x05, 0x79, 0x0e, 0x6a, 0x81, 0x62, 0x01, 0xc0,
    ];

    const IN92_ASSIGN: &[u8] = &[
        0x53, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0xf9, 0x00, 0x04, 0x88, 0x03, 0x00,
        0x00, 0x00, 0x00, 0x06, 0x00, 0x43, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0xfc, 0x67,
        0x00, 0x68, 0x82, 0x0d, 0x01, 0x18, 0x82, 0x13, 0x06, 0x14, 0x85, 0x18, 0x83, 0x17, 0xc2,
        0x19, 0xcd, 0x01, 0xfe, 0x1a, 0xff, 0x09, 0x01, 0x0a, 0xc3, 0x0b, 0x83, 0x02, 0x04, 0x03,
        0x04, 0x04, 0x94,
    ];

    const IN68_META: &[u8] = &[
        0x39, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x35, 0x00, 0x04, 0xb5, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x06, 0x00, 0x29, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0xfc, 0x67,
        0x00, 0x68, 0x86, 0x6b, 0xcd, 0x00, 0x00, 0x6c, 0xcd, 0x00, 0x20, 0x6d, 0xac, 0x50, 0x72,
        0x65, 0x73, 0x65, 0x00,
    ];

    fn test_final_bulk_272(module: &[u8; 3]) -> Vec<u8> {
        let mut buf = vec![0u8; 272];
        buf[0] = 0x08;
        buf[1] = 0x01;
        buf[2..5].copy_from_slice(&[0x00, 0x00, 0x18]);
        buf[6..10].copy_from_slice(&[0xed, 0x03, 0x80, 0x10]);
        let off = 200;
        buf[off..off + 3].copy_from_slice(&[0x83, 0x66, 0xcd]);
        buf[off + 3] = 0x03;
        buf[off + 10] = 0x19;
        buf[off + 11..off + 14].copy_from_slice(module);
        buf[off + 14] = 0x1a;
        buf
    }

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
    fn pull_f0_interstitial_uses_hw_model_scroll_ack_double() {
        let mut state = HelixState::new();
        state.hw_model_scroll_ack_ctr = 0x108f;
        let ctr_before = state.hw_model_scroll_ack_ctr;
        let pkt = build_pull_f0_interstitial(&mut state);
        assert_eq!(pkt[12], 0x8f);
        assert_eq!(pkt[13], 0x10);
        assert_eq!(state.hw_model_scroll_ack_ctr, ctr_before);
    }

    #[test]
    fn advance_scroll_ack_after_interstitial_f0_matches_hx_second_f0() {
        let mut state = HelixState::new();
        state.hw_model_scroll_ack_ctr = 0x1035;
        advance_scroll_ack_after_pull_interstitial_f0(&mut state);
        assert_eq!(state.hw_model_scroll_ack_ctr, 0x1063);
    }

    #[test]
    fn ack_pull_interstitial_echo_16o() {
        const IN_ECHO: &[u8] = &[
            0x08, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x40, 0x00, 0x08, 0xea, 0x02, 0x00,
            0x00,
        ];
        let mut state = HelixState::new();
        state.hw_model_pull_step = 1;
        try_ack_pull_interstitial_echo(&mut state, IN_ECHO);
        // Pas de panic ; envoi ignoré si tx absent.
    }

    #[test]
    fn post_assign_21_matches_user_capture() {
        const IN21_POST: &[u8] = &[
            0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x23, 0x00, 0x04, 0x09, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x82, 0x69, 0x27, 0x6a, 0x84, 0x52,
            0x01, 0x44, 0x03, 0x79, 0x13, 0x6a, 0x82, 0x62, 0x01, 0x1a, 0x00, 0x6d, 0xac, 0x50,
        ];
        assert!(is_hw_model_post_assign_21(IN21_POST));
    }

    #[test]
    fn pull_trigger_1f_skips_scroll_ack_lane_unchanged() {
        let mut state = HelixState::new();
        state.tx = None;
        state.preset_content_only = false;
        state.hw_active_slot_bus = Some(0x01);
        state.hw_model_scroll_ack_ctr = 0x1035;
        assert!(!ack_hw_model_scroll_in(&mut state, IN1D));
        assert!(state.hw_model_scroll_deferred_1d.is_some());
        assert_eq!(state.hw_model_scroll_ack_ctr, 0x1035);
        assert_eq!(state.hw_model_scroll_ack_prev, None);
        assert!(!ack_hw_model_scroll_in(&mut state, IN1F));
        assert!(state.hw_model_scroll_deferred_1d.is_none());
        assert_eq!(state.hw_model_scroll_ack_ctr, 0x1035);
        assert_eq!(state.hw_model_scroll_ack_prev, None);
        let pkt = build_pull_f0_interstitial(&mut state);
        assert_eq!(pkt[12], 0x35);
        assert_eq!(pkt[13], 0x10);
    }

    #[test]
    fn deferred_1d_flushed_when_1f_not_pull_trigger() {
        let mut state = HelixState::new();
        state.tx = None;
        state.preset_content_only = false;
        state.hw_active_slot_bus = Some(0x01);
        state.hw_model_scroll_ack_ctr = 0x1035;
        assert!(!ack_hw_model_scroll_in(&mut state, IN1D));
        assert!(ack_hw_model_scroll_in(&mut state, IN1F_NONE));
        assert!(state.hw_model_scroll_deferred_1d.is_none());
        assert_eq!(state.hw_model_scroll_ack_ctr, 0x104c); // +0x17 après 1d puis 1f None
        assert_eq!(state.hw_model_scroll_ack_prev, Some(0x1f));
    }

    #[test]
    fn ack_1d_suppressed_during_active_pull() {
        let mut state = HelixState::new();
        state.tx = None;
        state.hw_model_pull_capture_deadline =
            Some(Instant::now() + Duration::from_millis(PULL_CAPTURE_MS));
        let ctr_before = state.hw_model_scroll_ack_ctr;
        assert!(!ack_hw_model_scroll_in(&mut state, IN1D));
        assert_eq!(state.hw_model_scroll_ack_ctr, ctr_before);
        assert_eq!(state.hw_model_scroll_ack_prev, None);
    }

    #[test]
    fn ack_scroll_ignores_21_44o_post_assign() {
        const IN21_POST: &[u8] = &[
            0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x30, 0x00, 0x04, 0x09, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x82, 0x69, 0x27, 0x6a, 0x84, 0x52,
            0x01, 0x44, 0x03, 0x79, 0x13, 0x6a, 0x82, 0x62, 0x01, 0x1a, 0x00, 0x6d, 0xac, 0x50,
        ];
        let mut state = HelixState::new();
        state.tx = None;
        assert!(!ack_hw_model_scroll_in(&mut state, IN21_POST));
        assert_eq!(state.hw_model_scroll_ack_prev, None);
    }

    #[test]
    fn ingest_hw_slot_notify_ignores_post_assign_21() {
        const IN21_POST: &[u8] = &[
            0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x30, 0x00, 0x04, 0x09, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x82, 0x69, 0x27, 0x6a, 0x84, 0x52,
            0x01, 0x44, 0x03, 0x79, 0x13, 0x6a, 0x82, 0x62, 0x01, 0x1a, 0x00, 0x6d, 0xac, 0x50,
        ];
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x02);
        state.hw_active_slot_sequence = 5;
        assert!(state.ingest_hw_slot_notify_in(IN21_POST).is_none());
        assert_eq!(state.hw_active_slot_sequence, 5);
        assert_eq!(state.hw_active_slot_bus, Some(0x02));
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
    fn detects_none_notify_from_capture_frame_1315() {
        assert!(is_hw_model_slot_cleared_notify(IN1F_NONE));
        assert!(!is_hw_model_slot_cleared_notify(IN1F));
        assert!(!is_hw_model_pull_trigger_notify(IN1F_NONE));
        assert!(is_hw_model_pull_trigger_notify(IN1F));
    }

    #[test]
    fn ingest_1f_none_emits_empty_without_pull() {
        let mut state = HelixState::new();
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        let payload = ingest_slot_model_hw_in(&mut state, IN1F_NONE).expect("payload");
        assert!(payload.module_hex.is_none());
        assert!(state.hw_model_pull_capture_deadline.is_none());
        assert_eq!(state.hw_model_pull_step, 0);
    }

    #[test]
    fn ingest_1f_arms_capture() {
        let mut state = HelixState::new();
        state.preset_data_ready = true;
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        assert!(ingest_slot_model_hw_in(&mut state, IN1F).is_none());
        assert!(state.hw_model_pull_last_at.is_none());
        assert_eq!(state.hw_model_pull_step, 1);
    }

    #[test]
    fn pull_final_272_is_not_first_reply() {
        let mut buf = vec![0u8; 272];
        buf[0] = 0x08;
        buf[4..8].copy_from_slice(&[0xed, 0x03, 0x80, 0x10]);
        buf[24..28].copy_from_slice(&[0x83, 0x66, 0xcd, 0x03]);
        buf[28..32].copy_from_slice(&[0x19, 0xcd, 0x01, 0xfe]);
        assert!(!looks_like_first_pull_reply(&buf));
        assert!(!looks_like_second_pull_reply(&buf));
    }

    #[test]
    fn rapid_1f_after_finalize_ignored_during_settling_not_immediate_pull() {
        let mut state = HelixState::new();
        state.preset_data_ready = true;
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        ingest_slot_model_hw_in(&mut state, IN1F);
        assert!(ingest_slot_model_hw_in(&mut state, IN92_ASSIGN).is_none());
        assert!(ingest_slot_model_hw_in(&mut state, IN68_META).is_none());
        let _ = ingest_slot_model_hw_in(&mut state, &test_final_bulk_272(&[0xcd, 0x01, 0xfe]))
            .expect("payload");
        assert!(state.hw_model_post_pull_settling);
        ingest_slot_model_hw_in(&mut state, IN1F);
        assert!(state.hw_model_pull_pending_slot_bus.is_none());
        assert!(state.hw_model_pull_capture_deadline.is_none());
    }

    #[test]
    fn ingest_1f_during_pull_queues_pending_then_flushes() {
        let mut state = HelixState::new();
        state.preset_data_ready = true;
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        ingest_slot_model_hw_in(&mut state, IN1F);
        assert_eq!(state.hw_model_pull_step, 1);
        ingest_slot_model_hw_in(&mut state, IN1F);
        assert_eq!(state.hw_model_pull_pending_slot_bus, Some(0x01));
        assert!(ingest_slot_model_hw_in(&mut state, IN92_ASSIGN).is_none());
        assert!(ingest_slot_model_hw_in(&mut state, IN68_META).is_none());
        let payload =
            ingest_slot_model_hw_in(&mut state, &test_final_bulk_272(&[0xcd, 0x01, 0xfe]))
                .expect("payload");
        assert_eq!(payload.module_hex.as_deref(), Some("cd01fe"));
        assert_eq!(state.hw_model_pull_pending_slot_bus, Some(0x01));
        state.hw_model_post_pull_settling = false;
        state.hw_model_post_pull_deadline = None;
        state.hw_model_pull_quiet_until = Some(Instant::now());
        state.hw_model_pull_last_at =
            Some(Instant::now() - Duration::from_millis(PULL_COOLDOWN_AFTER_DONE_MS + 10));
        ingest_slot_model_hw_in(&mut state, IN1F);
        assert_eq!(state.hw_model_pull_step, 1);
        assert!(state.hw_model_pull_pending_slot_bus.is_none());
    }

    #[test]
    fn post_pull_settling_ignores_1f_then_drops_pending_without_pull() {
        let mut state = HelixState::new();
        state.preset_data_ready = true;
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        arm_post_pull_stream_settling(&mut state);
        state.hw_model_pull_pending_slot_bus = Some(0x01);
        ingest_slot_model_hw_in(&mut state, IN1F);
        assert_eq!(state.hw_model_pull_pending_slot_bus, Some(0x01));
        assert!(state.hw_model_pull_capture_deadline.is_none());
        state.hw_model_post_pull_deadline =
            Some(Instant::now() - Duration::from_millis(1));
        ingest_slot_model_hw_in(&mut state, &[0x08, 0x00, 0x00, 0x18, 0xef, 0x03, 0x01, 0x10]);
        assert_eq!(state.hw_model_pull_step, 0);
        assert!(state.hw_model_pull_pending_slot_bus.is_none());
    }

    #[test]
    fn fresh_1f_after_settling_starts_pull_not_stale_pending_flush() {
        let mut state = HelixState::new();
        state.preset_data_ready = true;
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        arm_post_pull_stream_settling(&mut state);
        state.hw_model_pull_pending_slot_bus = Some(0x01);
        state.hw_model_post_pull_deadline =
            Some(Instant::now() - Duration::from_millis(1));
        ingest_slot_model_hw_in(&mut state, &[0x08, 0x00, 0x00, 0x18, 0xef, 0x03, 0x01, 0x10]);
        assert!(state.hw_model_pull_pending_slot_bus.is_none());
        state.hw_model_pull_last_at =
            Some(Instant::now() - Duration::from_millis(PULL_COOLDOWN_AFTER_DONE_MS + 10));
        ingest_slot_model_hw_in(&mut state, IN1F);
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
    fn cd_lane_04_survives_arm_pull_capture_after_finalize() {
        let mut state = HelixState::new();
        state.hw_model_pull_cd_lane = Some(0x04);
        arm_pull_capture(&mut state, 0x01);
        assert_eq!(state.hw_model_pull_cd_lane, Some(0x04));
        let pkt = build_pull_1b(&mut state, 0x01);
        assert_eq!(pkt[27], 0x04);
    }

    #[test]
    fn cd_lane_switches_to_04_on_editor_double_wrap() {
        let mut state = HelixState::new();
        state.editor_ed03_double = 0x64fd;
        let p = build_pull_1b(&mut state, 0x01);
        assert_eq!(p[27], 0x04);
        assert_eq!(p[28], 0x00);
        assert_eq!(state.hw_model_pull_cd_lane, Some(0x04));
        state.hw_model_pull_cd_lane = None;
        state.editor_ed03_double = 0x6403;
        let p2 = build_pull_19(&mut state, false);
        assert_eq!(p2[27], 0x03);
        state.hw_model_pull_cd_lane = Some(0x04);
        let p3 = build_pull_19(&mut state, false);
        assert_eq!(p3[27], 0x04);
    }

    #[test]
    fn pull_1b_advances_editor_double_by_three_19_reuses_same() {
        let mut state = HelixState::new();
        state.editor_ed03_double = 0x64ee;
        let p1b = build_pull_1b(&mut state, 0x01);
        assert_eq!(p1b[28], 0xf1);
        assert_eq!(p1b[29], 0x64);
        let p19 = build_pull_19(&mut state, false);
        assert_eq!(p19[28], 0xf1);
        assert_eq!(p19[29], 0x64);
        let p1b2 = build_pull_1b(&mut state, 0x01);
        assert_eq!(p1b2[28], 0xf4);
        assert_eq!(p1b2[29], 0x64);
        state.editor_ed03_double = 0x64fd;
        let p_wrap = build_pull_1b(&mut state, 0x01);
        assert_eq!(p_wrap[28], 0x00);
        assert_eq!(p_wrap[29], 0x64);
    }

    #[test]
    fn pull_19_uses_hw_model_pull_ctr_and_advances() {
        let mut state = HelixState::new();
        state.hw_model_pull_ctr = 0x413f;
        let p1 = build_pull_19(&mut state, false);
        assert_eq!(p1[12], 0x3f);
        assert_eq!(p1[13], 0x41);
        assert_eq!(state.hw_model_pull_ctr, 0x4170); // +0x31
        let p2 = build_pull_19(&mut state, true);
        assert_eq!(p2[12], 0x70);
        assert_eq!(p2[13], 0x41);
        assert_eq!(state.hw_model_pull_ctr, 0x41a1); // +0x31
        assert_eq!(state.live_write_ctr, 0x6cbd); // sonde UI n’a pas pollué la lane pull
    }

    #[test]
    fn ingest_1d_then_1f_single_pull() {
        let mut state = HelixState::new();
        state.preset_data_ready = true;
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
        state.preset_data_ready = true;
        state.preset_content_only = false;
        state.hw_active_slot_index = Some(0);
        state.hw_active_slot_bus = Some(0x01);
        ingest_slot_model_hw_in(&mut state, IN1F);
        assert!(ingest_slot_model_hw_in(&mut state, IN92_ASSIGN).is_none());
        assert_eq!(state.hw_model_pull_step, 2);
        assert!(ingest_slot_model_hw_in(&mut state, IN68_META).is_none());
        assert_eq!(state.hw_model_pull_step, 3);
        assert!(!state.hw_model_pull_saw_final_bulk);
        let final272 = test_final_bulk_272(&[0xcd, 0x01, 0xfe]);
        let payload = ingest_slot_model_hw_in(&mut state, &final272).expect("payload");
        assert_eq!(payload.slot_index, 0);
        assert_eq!(payload.module_hex.as_deref(), Some("cd01fe"));
        assert_eq!(state.hw_model_pull_step, 0);
    }
}
