//! Couche Pipeline — pull modèle slot hardware sur `IN 1f` scroll.
//!
//! Placée **avant** `FirmwareScroll` dans `run_usb_in_active_layers` :
//! - `IN 1f` f0:03 non-None → `Consumed` (envoie `1b` + `f0` + attend les IN)
//! - `IN 1d` f0:03 → `Ignored` (laissé à `FirmwareScroll`)
//! - `IN 21` 44o post-assign → `Ignored`
//!
//! ## Compteurs
//!
//! | Champ | Rôle | Pas |
//! |-------|------|-----|
//! | `hw_model_pull_ed03_double` | octets 28–29 des OUT `1b`/`19` (double cd:03) | **+1 par OUT émis** |
//! | `hw_model_pull_ctr` | octets 12–13 des OUT `1b`/`19` | +0x4B (après 1b), +0x31 (après 19) |
//! | `firmware_scroll_ack_ctr` | octets 12–13 du `f0:03 sub=08` | avancé de `+0x17` pour le `1f` trigger |
//!
//! ## Modèle du double — sourcé sur capture canonique HX Edit (one_notch, juin 2026)
//!
//! `stomp_running_start_hxedit_one_notch.json`, pull qui DUMPE :
//! ```
//! [195] OUT 1b  ctr=0x1c7e  double=f1:64   ← le dump (IN 53) part sur CE 1b seul
//! [200] OUT 19  ctr=0x1cc9  double=f2:64   (+0x4b sur ctr, +1 sur double)
//! [203] OUT 19  ctr=0x1cfa  double=f3:64   (+0x31 sur ctr, +1 sur double)
//! ```
//! Donc le double avance de **+1 sur CHAQUE OUT réellement émis** (f1→f2→f3). Le
//! « +3 entre pulls » observé jadis (d6eb2b1 eb→ee→f1) n'est PAS une règle : c'est
//! simplement +1 × 3 OUT par pull. Le hi reste figé `0x64`, le `cd_lane` (octet 27)
//! passe 03→04 au wrap bas du lo. Le device tolère la valeur absolue (un pull qui
//! dumpe peut partir de f1 comme de f8) ; ce qui compte est la **continuité +1/OUT**.
//!
//! ## Modes (flag `HX_PULL_COUPLE_LANE=1`)
//!
//! Le mécanisme du double est **identique** dans les deux modes (+1/OUT, hi figé 0x64,
//! wrap cd_lane). Seule la GRAINE du pull diffère, posée une fois par session (sentinelle
//! `0xFFFF`) :
//! - **couplé** (`HX_PULL_COUPLE_LANE=1`) : double = `editor_ed03_double` VIVANT ; ctr =
//!   `0x6cbd` (page 0x6c) — EMPIRIQUEMENT la seule famille qui fait partir le `IN 53`
//!   (dump) sur notre session. La page 0x1c (lane vivante, ou constante 0x1c7e) ne dumpe
//!   jamais ici. La règle exacte des octets 12-13 reste inconnue (pas de specs Line 6).
//! - **figé** (défaut, témoin) : graine = `editor_ed03_double` + `HX_PULL_DOUBLE_DELTA`,
//!   ctr base figée `0x1c7e`.
//!
//! ## Mode GRAB-53 (depuis juin 2026)
//!
//! On n'a besoin QUE du `chainHex`, et il est **entièrement dans le `IN 53`** (92 o,
//! motif `… 19 <id> 1a …`). Le `272` ne porte que les paramètres du modèle (inutiles).
//! Donc dès qu'on reçoit le `53`, on **extrait le chainHex et on finalise** : on n'envoie
//! **jamais** les `19`, on ne poursuit **jamais** le `272`. Ça supprime la traîne
//! `19/272` qui (a) gelait le device et (b) causait très probablement les rejects
//! intermittents (device en retard sur le tail du pull précédent). Un `IN 21` (reject)
//! s'aborte proprement → l'utilisateur re-scrolle. Cf. `scroll_model_pull_HANDOFF.md`.
//!
//! ## Modèle du double — capture canonique HX Edit (one_notch, juin 2026)
//!
//! `stomp_running_start_hxedit_one_notch.json` : le dump (`IN 53`) part sur le `1b` SEUL,
//! avant tout `19`. Le double avance +1 par OUT (le hi reste `0x64`, le `cd_lane` octet 27
//! passe 03→04 au wrap bas du lo). Le device tolère la valeur absolue du double.
//!
//! ## Historique du correctif (juin 2026)
//! - SUPPRIMÉ le `+3` aveugle en finalize (avançait le double même sur pull RATÉ →
//!   désync → freeze). Remplacé par +1 sur chaque OUT effectivement émis.
//! - AJOUT abort propre sur `IN 21` (reject) à l'étape 1 : plus de transaction pendante.
//! - Wrap `cd 03→04` géré dans les deux modes.
//! - PASSAGE en mode GRAB-53 : finalisation sur le `IN 53`, plus aucun `19` ni `272`
//!   émis (la traîne était la source du freeze ET des rejects intermittents).
//!   `send_pull_both_19s` conservée `#[allow(dead_code)]` comme référence du handshake
//!   complet pour qui voudrait les paramètres du modèle.
//! - AJOUT retry-once-on-reject — RETIRÉ ensuite : la capture a montré que le `IN 21`
//!   n'est PAS un reject (le device l'envoie parfois AVANT le dump, qui arrive ~2-16 ms
//!   après). Le retry réémettait donc un `1b` inutile. Remplacé par : on n'agit plus sur
//!   le `21` (il ne porte aucun model-id), on attend simplement le dump.
//! - CORRECTION reconnaissance du dump : on finalise dès qu'une trame porte un MODEL-ID
//!   (marqueur + `19…1a`), quel que soit le head/la longueur. Le device dumpe sous des
//!   formats très variés (head 0x44..0x6c, 76-116 o) ; l'ancien filtre par head ratait
//!   p.ex. 0x44/76o (notch 22 → modèle émis en retard via timeout → « UI décalée »).
//!
//! ## Référence captures
//! `stomp_running_start_hxedit_one_notch.json` (HX, dumpe),
//! `stomp_running_start_linux_multi_notch_crash.json` (run du freeze, ancien chemin 19/272).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static SCROLL_PULL_DEBUG: AtomicBool = AtomicBool::new(false);

use serde::Serialize;

use crate::helix::model_catalog;
use crate::helix::is_special_slot_bus;
use crate::helix::kempline_index_to_slot_bus;
use crate::helix::slot_bus_to_kempline_index;
use crate::helix::HelixState;
use crate::helix::packet::OutPacket;
use crate::helix::usb_in_pipeline::{LayerEffect, LayerResult};

// ── Constantes timing ────────────────────────────────────────────────────────

/// Délai entre `19` #1 et `19` #2.
const INTER_19_DELAY_MS: u64 = 4;
/// Délai `IN 1f` → OUT `1b` pull (captures HX ≈ 14 ms ; à affiner si besoin).
const PULL_1B_DELAY_AFTER_1F_MS: u64 = 14;
/// Cooldown après un pull terminé (scroll rapide).
const PULL_COOLDOWN_AFTER_DONE_MS: u64 = 40;
/// Fenêtre silence post-finalize (évite 272 tardif pris pour 1ère réponse `1b`).
const PULL_POST_FINALIZE_QUIET_MS: u64 = 85;
/// Settling post-pull court (272 dump tardifs), utilisé quand le coalescing est désactivé.
const PULL_POST_PULL_SETTLING_MS: u64 = 50;
/// Settling post-pull en mode coalescing = THROTTLE. Valeur MESURÉE sur matériel :
/// le gel de scroll rapide réapparaît vers ~300 ms d'espacement (fenêtre de drainage
/// ED03 du device ≈ 300-400 ms), tient à 500 ms → ~1,3-1,6× de marge. Voir addendum §10.
const PULL_THROTTLE_SETTLING_MS: u64 = 500;
/// Timeout capture (attend le bulk ~272).
const PULL_CAPTURE_MS: u64 = 600;
const PULL_CAPTURE_MAX_FRAMES: usize = 48;
/// Délai USB busy après scroll HW (bloque `request_preset_content` UI).
pub const HW_MODEL_USB_BUSY_AFTER_SCROLL_MS: u64 = 700;

// ── Constantes compteurs ─────────────────────────────────────────────────────

/// Avance `hw_model_pull_ctr` après le `1b`.
const PULL_CTR_DELTA_AFTER_1B: u16 = 0x004b;
/// Avance `hw_model_pull_ctr` après chaque `19`.
const PULL_CTR_DELTA_AFTER_19: u16 = 0x0031;

/// Base ctr du pull en mode FIGÉ (témoin) = `0x1c7e`, valeur du `1b` scroll HX one_notch
/// [195]. NB : en mode COUPLÉ le ctr n'est PAS une constante — il continue
/// `editor_ed03_lane` vivant (voir `handle_in_layer_trigger`), car le device rejette tout
/// ctr périmé sous sa lane et `0x1c7e` est en dessous de notre lane (≈ 0x1cf9).
const FROZEN_PULL_CTR_BASE: u16 = 0x1c7e;

// ── [TEST] Mode lane couplée ─────────────────────────────────────────────────
//
// HX_PULL_COUPLE_LANE=1 : double = editor_ed03_double VIVANT, ctr = editor_ed03_lane VIVANT.
// Défaut (absent) = mode figé témoin (graine snap + delta, ctr=0x1c7e).
fn couple_lane_enabled() -> bool {
    std::env::var("HX_PULL_COUPLE_LANE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false)
}

// ── Coalescing multi-cran (DÉFAUT ON) ───────────────────────────────────────
//
// En scroll rapide, un `1f` arrivant pendant le settling n'est pas jeté mais mémorisé
// (le DERNIER gagne) ; un pull différé est tiré en fin de settling par `tick_hw_model_pull`
// (appelé depuis `usb_listener`). On lit toujours le modèle FINAL d'un balayage, et le
// settling allongé (cf. post_pull_settling_ms) agit comme THROTTLE qui plafonne les
// transactions ED03 non fermées en vol → plus de gel (cf. addendum §10).
//
// Host-side PUR : aucun nouveau paquet n'est envoyé au device. Ne FERME PAS la transaction
// (cf. handoff §6 / proposition A, bloquée sur la règle du `ctr` des `19`).
//
// DÉFAUT : activé (comportement validé). `HX_PULL_COALESCE_LAST=0` pour revenir à l'ancien
// comportement (1f jeté pendant le settling, settling court de 50 ms).
fn coalesce_last_enabled() -> bool {
    std::env::var("HX_PULL_COALESCE_LAST")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("no")))
        .unwrap_or(true)
}

// Durée (ms) de la fenêtre de settling post-pull.
//
// - Override explicite `HX_PULL_SETTLING_MS=<n>` toujours prioritaire (tuning / mesure du
//   seuil device).
// - Sinon : 500 ms (throttle) si le coalescing est actif, 50 ms (historique) sinon.
fn post_pull_settling_ms() -> u64 {
    if let Some(v) = std::env::var("HX_PULL_SETTLING_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v > 0)
    {
        return v;
    }
    if coalesce_last_enabled() {
        PULL_THROTTLE_SETTLING_MS
    } else {
        PULL_POST_PULL_SETTLING_MS
    }
}

// ── [TEST] Offset double cd:03 du pull (mode figé uniquement) ─────────────────
//
// `HX_PULL_DOUBLE_DELTA` applique un décalage signé à la GRAINE du double figé.
// Ignoré en mode couplé (graine = lane vivante).
fn pull_double_delta() -> i32 {
    std::env::var("HX_PULL_DOUBLE_DELTA")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0)
}

// ── Payload UI ───────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotModelHwChangedPayload {
    pub sequence: u32,
    pub slot_index: u32,
    pub slot_bus: u8,
    pub module_hex: Option<String>,
}

// ── Couche Pipeline ───────────────────────────────────────────────────────────

/// Point d'entrée pipeline — déclenchement pull sur `IN 1f` uniquement.
/// La capture (`ingest_pull_capture`) est appelée depuis `usb_listener`.
pub fn handle_in_layer_trigger(data: &[u8], state: &mut HelixState) -> LayerResult {
    if state.hw_model_pull_capture_deadline.is_some() {
        return LayerResult::Ignored;
    }

    if !is_hw_model_pull_trigger_notify(data) {
        return LayerResult::Ignored;
    }

    if state.init_usb_settle_active()
        || state.preset_usb_read_in_progress()
        || state.preset_content_only
    {
        pull_trace("1f ignoré : settle / lecture preset");
        return LayerResult::Ignored;
    }

    let Some(slot_bus) = slot_bus_for_model_pull(state, data) else {
        pull_trace("1f sans slot_bus — ignoré");
        return LayerResult::Ignored;
    };

    if post_pull_stream_settling_active(state) {
        if coalesce_last_enabled() {
            // Coalescing : on ne jette plus le `1f`, on retient le DERNIER slot demandé.
            // Le pull différé partira en fin de settling (tick_hw_model_pull, usb_listener),
            // de sorte qu'on lise toujours le modèle FINAL du balayage.
            queue_pending_hw_model_pull(state, slot_bus);
        } else {
            pull_trace("1f pendant settling — ignoré (pas de file)");
        }
        return LayerResult::Ignored;
    }

    // ── Graine du double + ctr : posée UNE fois par session (sentinelle 0xFFFF) ──
    // Ensuite le double avance de +1 par OUT (build_pull_1b/19), monotone à travers
    // les pulls (motif HX f1→f2→f3…). Plus de +3 aveugle, plus de re-snap par pull.
    if state.hw_model_pull_ed03_double == 0xFFFF {
        let base = state.editor_ed03_double;
        if couple_lane_enabled() {
            // Couplé : double = editor_ed03_double VIVANT (≈ 0x64f2 après PHASE B).
            //   ctr = 0x6cbd (page 0x6c). C'est EMPIRIQUEMENT la seule famille de valeurs
            //   qui fait partir le `IN 53` (dump) ; la page 0x1c ne dumpe jamais sur notre
            //   session (cf. handoff §5 — règle exacte inconnue, pas de specs Line 6).
            //   Le freeze qu'on associait à 0x6cbd venait de la TRAÎNE 19/272, qu'on
            //   n'émet plus (on s'arrête au 53, cf. ingest_pull_capture).
            state.hw_model_pull_ed03_double = base;
            state.hw_model_pull_ctr = 0x6cbd;
            pull_trace(&format!(
                "[couple] graine double={:04x} ctr={:04x} (mode grab-53)",
                state.hw_model_pull_ed03_double, state.hw_model_pull_ctr,
            ));
        } else {
            // Figé (témoin) : graine = lane + delta, ctr base HX 0x1c7e.
            let delta = pull_double_delta();
            let snapped = if delta != 0 {
                ((base as i32 + delta).rem_euclid(0x10000)) as u16
            } else {
                base
            };
            state.hw_model_pull_ed03_double = snapped;
            state.hw_model_pull_ctr = FROZEN_PULL_CTR_BASE;
            pull_trace(&format!(
                "[figé] graine double={:04x} (base={:04x} delta={}) ctr={:04x}",
                state.hw_model_pull_ed03_double, base, delta, state.hw_model_pull_ctr,
            ));
        }
    }

    pull_trace(&format!(
        "pull slot_bus={slot_bus:02x} (kempline {:?}) — double={:04x} ctr={:04x}",
        slot_bus_to_kempline_index(slot_bus),
        state.hw_model_pull_ed03_double,
        state.hw_model_pull_ctr,
    ));

    state.hw_model_last_scroll_in_at = Some(Instant::now());

    // Avance de la lane scroll f0 pour le `1f` TRIGGER lui-même. Ce `1f` est consommé ici
    // (couche ScrollModelPull, avant FirmwareScroll) : si on ne l'avance pas, la lane reste
    // en retard d'un cran et le `f0` interstitiel du `1b` (qui lit firmware_scroll_lane_double)
    // porte une valeur que le device ne reconnaît pas → réponse `21` au lieu du dump.
    // Le `1d` pré-scroll qui précède a déjà été avancé par firmware_scroll_ack (skip + avance).
    let lane = state.advance_firmware_scroll_lane(0x1f);
    pull_trace(&format!(
        "lane scroll avancée pour 1f trigger → {:02x}:{:02x}",
        lane[0], lane[1]
    ));

    // Ce `1f` hors-settling supersède tout pull coalescé encore en attente.
    state.hw_model_pull_pending_slot_bus = None;
    send_pull_sequence(state, slot_bus);
    LayerResult::Consumed {
        effect: LayerEffect::None,
    }
}

// ── API publique (appelée depuis usb_listener) ────────────────────────────────

/// USB occupé par scroll modèle — bloque `request_preset_content` UI.
pub fn hw_model_usb_busy(state: &HelixState) -> bool {
    if state.init_usb_settle_active() {
        return true;
    }
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

/// Appelé depuis `FirmwareScroll` pour savoir si ce `1f` va lancer un pull
/// (et donc ne pas ACKer la lane scroll avant le `1b`).
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

pub fn post_pull_stream_settling_active(state: &HelixState) -> bool {
    state.hw_model_post_pull_settling
        && state
            .hw_model_post_pull_deadline
            .is_some_and(|t| Instant::now() < t)
}

/// Tick appelé depuis `usb_listener` à CHAQUE IN, indépendamment d'une capture en cours.
///
/// Sous `HX_PULL_COALESCE_LAST`, déclenche UN pull différé vers le DERNIER slot coalescé
/// quand la fenêtre de settling vient d'expirer — pour lire le modèle FINAL d'un balayage
/// rapide (le `1f` final tombait sinon dans le settling et était jeté). Sans le flag :
/// no-op strict. Ne ferme jamais la transaction et n'émet aucun paquet nouveau pour le
/// device : le pull différé est un pull grab-53 ordinaire.
pub fn tick_hw_model_pull(state: &mut HelixState) {
    if !coalesce_last_enabled() {
        return;
    }
    // Capture en cours : on laisse `ingest_pull_capture` finir ; pas de pull différé.
    if state.hw_model_pull_capture_deadline.is_some() {
        return;
    }
    // Fait avancer la fenêtre de settling ; tant qu'elle est active, on patiente.
    let _ = tick_post_pull_stream_settling(state);
    if post_pull_stream_settling_active(state) {
        return;
    }
    // Settling terminé : si un `1f` a été coalescé, on tire le pull vers le dernier slot.
    let Some(slot_bus) = state.hw_model_pull_pending_slot_bus.take() else {
        return;
    };
    if state.init_usb_settle_active()
        || state.preset_usb_read_in_progress()
        || state.preset_content_only
    {
        return;
    }
    pull_trace(&format!("pull différé (coalescé) slot_bus={slot_bus:02x}"));
    state.hw_model_last_scroll_in_at = Some(Instant::now());
    let lane = state.advance_firmware_scroll_lane(0x1f);
    pull_trace(&format!(
        "lane scroll avancée (pull différé) → {:02x}:{:02x}",
        lane[0], lane[1]
    ));
    send_pull_sequence(state, slot_bus);
}

// ── Détection paquets ─────────────────────────────────────────────────────────

pub fn is_hw_model_change_notify_loose(data: &[u8]) -> bool {
    data.len() >= 32
        && matches!(data[0], 0x1d | 0x1f | 0x21)
        && data.get(1..4) == Some(&[0x00, 0x00, 0x18])
        && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
        && data.get(12..14) == Some(&[0x09, 0x02])
}

pub fn is_hw_model_change_notify_1f(data: &[u8]) -> bool {
    is_hw_model_change_notify_loose(data) && data[0] == 0x1f
}

const HW_MODEL_NONE_NOTIFY_MARK: &[u8] =
    &[0x82, 0x69, 0x31, 0x6a, 0x84, 0x52, 0x00, 0x44, 0x05, 0x79, 0x0e, 0x6a];

pub fn is_hw_model_slot_cleared_notify(data: &[u8]) -> bool {
    is_hw_model_change_notify_1f(data)
        && data
            .windows(HW_MODEL_NONE_NOTIFY_MARK.len())
            .any(|w| w == HW_MODEL_NONE_NOTIFY_MARK)
}

/// Seul déclencheur valide de pull (`1f` non-None).
pub fn is_hw_model_pull_trigger_notify(data: &[u8]) -> bool {
    is_hw_model_change_notify_1f(data) && !is_hw_model_slot_cleared_notify(data)
}

/// `IN 1d` de **pré-scroll** : porte le marqueur "modèle en cours de scroll"
/// (`82:69:31:6a` aux octets 24-27, octet 26 = `0x31`), comme le `1f` pull —
/// par opposition au `1d` de fond idle (`82:69:16:6a`, octet 26 = `0x16`).
///
/// HX **n'ACK pas** ce `1d`-là (capture `one_notch` : `1d` puis `1f` du notch laissés
/// sans ACK avant le `1b`). L'ACKer désynchronise la lane `f0` et fait échouer le pull.
/// Discriminant validé : 0 faux positif sur le fond idle (53 `1d` idle en `16:6a`).
pub fn is_hw_model_scroll_1d(data: &[u8]) -> bool {
    data.len() >= 28
        && data[0] == 0x1d
        && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
        && data.get(24..28) == Some(&[0x82, 0x69, 0x31, 0x6a])
}

/// IN `21` 44 o post-assign — pas d’ACK host (réservé filtrage pipeline / phase C).
#[allow(dead_code)]
pub fn is_hw_model_post_assign_21(data: &[u8]) -> bool {
    data.len() == 44
        && data.first() == Some(&0x21)
        && data.get(24..28) == Some(&[0x82, 0x69, 0x27, 0x6a])
        && data.windows(3).any(|w| w == [0x82, 0x62, 0x01, 0x1a])
}

// ── Résolution slot_bus ───────────────────────────────────────────────────────

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

fn slot_bus_for_model_pull(state: &HelixState, notify: &[u8]) -> Option<u8> {
    if let Some(hw) = active_effect_slot_bus(state) {
        return Some(hw);
    }
    if let Some(from_notify) = parse_slot_bus_from_model_notify(notify) {
        pull_trace(&format!(
            "hw_active vide — secours 81:62 notif={from_notify:02x}"
        ));
        return Some(from_notify);
    }
    None
}

// ── Builders paquets ──────────────────────────────────────────────────────────

fn pull_ctr_bytes(state: &HelixState) -> (u8, u8) {
    let ctr = state.hw_model_pull_ctr;
    ((ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8)
}

fn advance_pull_ctr(state: &mut HelixState, delta: u16) {
    state.hw_model_pull_ctr = state.hw_model_pull_ctr.wrapping_add(delta);
}

/// cd lane (octet 27) : `03` par défaut, passe à `04` au wrap bas du double.
fn cd_lane_for_out(state: &HelixState) -> u8 {
    state.hw_model_pull_cd_lane.unwrap_or(0x03)
}

fn cd_lane_for_hw_model_pull_out(state: &mut HelixState, prev_lo: u8, wire: [u8; 2]) -> u8 {
    if wire[0] < prev_lo {
        state.hw_model_pull_cd_lane = Some(0x04);
        pull_trace("double wrap bas → cd lane 04");
    }
    cd_lane_for_out(state)
}

/// Avance le double cd:03 de +1 (lo cyclique 00..ff, hi figé 0x64) et renvoie
/// `(lo, hi, cd_lane)` pour le fil. Identique en mode couplé et figé : la seule
/// chose qui diffère est la GRAINE posée dans `handle_in_layer_trigger`.
///
/// hi figé 0x64 + `cd_lane` 03→04 au wrap = forme protocolaire vérifiée HX. NE PAS
/// laisser le hi rouler à 0x65 (bug du `+3` sur u16 plein : 0x64ff→0x6500).
fn next_pull_double_wire(state: &mut HelixState) -> (u8, u8, u8) {
    let prev_lo = (state.hw_model_pull_ed03_double & 0xFF) as u8;
    let nlo = prev_lo.wrapping_add(1);
    state.hw_model_pull_ed03_double = 0x6400 | u16::from(nlo);
    let cd_lane = cd_lane_for_hw_model_pull_out(state, prev_lo, [nlo, 0x64]);
    (nlo, 0x64, cd_lane)
}

/// `OUT 1b` 36o — déclenche le pull.
fn build_pull_1b(state: &mut HelixState, slot_bus: u8) -> Vec<u8> {
    let cnt0 = state.next_x80_cnt();
    let (lo, hi, cd_lane) = next_pull_double_wire(state);
    let (ctr_lo, ctr_hi) = pull_ctr_bytes(state);
    advance_pull_ctr(state, PULL_CTR_DELTA_AFTER_1B);
    vec![
        0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, cnt0, 0x00, 0x04, ctr_lo, ctr_hi, 0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, cd_lane, lo, hi,
        0x2d, 0x65, 0x81, 0x62, slot_bus, 0x00,
    ]
}

/// `OUT f0:03 sub=08` 16o interstitiel (entre `1b` et premier bulk IN).
fn build_pull_f0_interstitial(state: &mut HelixState) -> Vec<u8> {
    let cnt = state.next_x2_cnt();
    let double = state.firmware_scroll_lane_double();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08, double[0], double[1], 0x00, 0x00,
    ]
}

/// `OUT 19` 36o — réponse aux bulks. +1 sur le double (motif HX f2/f3).
fn build_pull_19(state: &mut HelixState, second: bool) -> Vec<u8> {
    let cnt = state.next_x80_cnt();
    let (lo, hi, cd_lane) = next_pull_double_wire(state);
    let (ctr_lo, ctr_hi) = pull_ctr_bytes(state);
    advance_pull_ctr(state, PULL_CTR_DELTA_AFTER_19);
    let pre_65 = if second { 0x16u8 } else { 0x17u8 };
    vec![
        0x19, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, cnt, 0x00, 0x0c, ctr_lo, ctr_hi, 0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, cd_lane, lo, hi,
        pre_65, 0x65, 0xc0, 0x00, 0x00, 0x00,
    ]
}

// ── Envois groupés ────────────────────────────────────────────────────────────

fn send_pull_1b_f0_burst(state: &mut HelixState, slot_bus: u8) {
    let pkt1b = build_pull_1b(state, slot_bus);
    let pkt_f0 = build_pull_f0_interstitial(state);
    pull_trace(&format!(
        "OUT 1b+f0 slot_bus={slot_bus:02x} double={:02x}:{:02x} ctr={:04x} delay_1b={PULL_1B_DELAY_AFTER_1F_MS}ms f0_lane={:02x}:{:02x}",
        pkt1b[28], pkt1b[29],
        state.hw_model_pull_ctr.wrapping_sub(PULL_CTR_DELTA_AFTER_1B),
        pkt_f0[12], pkt_f0[13],
    ));
    let mut pkt = OutPacket::with_delay(pkt1b, PULL_1B_DELAY_AFTER_1F_MS);
    pkt.tail_burst = vec![pkt_f0];
    state.send(pkt);
}

/// [RÉFÉRENCE — non utilisée en mode grab-53] Envoie `19#1` puis `19#2` d'affilée pour
/// poursuivre le handshake vers le `272` (paramètres). On ne l'appelle plus : on s'arrête
/// au `IN 53` (chainHex), car la traîne `19/272` gelait le device. Conservée pour un
/// repreneur qui voudrait les paramètres du modèle (cf. handoff §6).
#[allow(dead_code)]
fn send_pull_both_19s(state: &mut HelixState) {
    let pkt1 = build_pull_19(state, false);
    pull_trace(&format!("OUT 19 #1 double={:02x}:{:02x}", pkt1[28], pkt1[29]));
    state.send(OutPacket::with_delay(pkt1, INTER_19_DELAY_MS));

    let pkt2 = build_pull_19(state, true);
    pull_trace(&format!("OUT 19 #2 double={:02x}:{:02x}", pkt2[28], pkt2[29]));
    state.send(OutPacket::with_delay(pkt2, INTER_19_DELAY_MS));

    state.hw_model_pull_step = 3;
}

pub fn send_pull_sequence(state: &mut HelixState, slot_bus: u8) {
    if state.init_usb_settle_active() {
        return;
    }
    if post_pull_stream_settling_active(state) {
        queue_pending_hw_model_pull(state, slot_bus);
        return;
    }
    arm_pull_capture(state, slot_bus);
    send_pull_1b_f0_burst(state, slot_bus);
}

// ── Capture et finalize ───────────────────────────────────────────────────────

fn arm_pull_capture(state: &mut HelixState, slot_bus: u8) {
    state.hw_model_pull_capture.clear();
    state.hw_model_pull_slot_bus = Some(slot_bus);
    state.hw_model_pull_step = 1;
    if state.hw_model_pull_cd_lane != Some(0x04) {
        state.hw_model_pull_cd_lane = None;
    }
    state.hw_model_pull_echo_double = None;
    state.hw_model_pull_saw_final_bulk = false;
    state.hw_model_pull_capture_deadline =
        Some(Instant::now() + Duration::from_millis(PULL_CAPTURE_MS));
}

fn queue_pending_hw_model_pull(state: &mut HelixState, slot_bus: u8) {
    state.hw_model_pull_pending_slot_bus = Some(slot_bus);
    pull_trace(&format!("pull en file slot_bus={slot_bus:02x}"));
}

fn arm_pull_post_finalize_quiet(state: &mut HelixState) {
    state.hw_model_pull_quiet_until =
        Some(Instant::now() + Duration::from_millis(PULL_POST_FINALIZE_QUIET_MS));
}

fn arm_post_pull_stream_settling(state: &mut HelixState) {
    state.hw_model_post_pull_settling = true;
    state.hw_model_post_pull_deadline =
        Some(Instant::now() + Duration::from_millis(post_pull_settling_ms()));
}

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
    pull_trace("post-pull settling terminé");
    true
}

fn is_pull_final_meta_bulk(data: &[u8]) -> bool {
    data.len() >= 180
}

fn frame_has_assign_marker(data: &[u8]) -> bool {
    data.windows(3).any(|w| w == [0x83, 0x66, 0xcd])
}

/// Une trame de DUMP : porte le marqueur d'assignation `83 66 cd` ET un model-id
/// extractible (`19…1a`). Robuste à TOUS les formats observés (head 0x44..0x6c, 76-116 o)
/// — on se fie au contenu, pas au head. Exclut le bulk `272` (paramètres) et — par nature
/// — le `IN 21` (44 o, aucun model-id) et les keep-alives.
fn frame_carries_model_id(data: &[u8]) -> bool {
    !is_pull_final_meta_bulk(data)
        && frame_has_assign_marker(data)
        && extract_first_module_hex_from_bulk(data).is_some()
}

fn is_in_1c_stub(data: &[u8]) -> bool {
    data.len() == 36
        && data.first() == Some(&0x1c)
        && data.windows(3).any(|w| w == [0x83, 0x66, 0xcd])
}

/// Conservée pour référence protocole — n'est plus dans le chemin actif depuis que les
/// deux `19` partent ensemble sur la première réponse (cf. `send_pull_both_19s`).
#[allow(dead_code)]
fn looks_like_second_pull_reply(data: &[u8]) -> bool {
    if is_pull_final_meta_bulk(data) {
        return false;
    }
    data.first() == Some(&0x39)
        || is_in_1c_stub(data)
        || (data.len() >= 48 && data.len() < 120 && frame_has_assign_marker(data))
}

fn cd_lane_byte(data: &[u8]) -> Option<u8> {
    data.windows(4)
        .find(|w| w[0] == 0x83 && w[1] == 0x66 && w[2] == 0xcd)
        .map(|w| w[3])
}

fn remember_cd_lane_from_in(state: &mut HelixState, data: &[u8]) {
    if is_in_1c_stub(data) {
        return;
    }
    if let Some(lane) = cd_lane_byte(data) {
        if state.hw_model_pull_cd_lane != Some(0x04) || lane == 0x04 {
            state.hw_model_pull_cd_lane = Some(lane);
        }
    }
}

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
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08, d0, d1, 0x00, 0x00,
    ]));
}

/// Extrait le premier model-id (`19 <1..N o> 1a`) d'une trame de dump.
///
/// **Correctif anti-collision écho-double (juin 2026).** Juste après le marqueur
/// d'assignation `83 66 cd <cd_lane>`, le device place l'ÉCHO du double host :
/// `<double_lo> 67 00 68 …`. Quand `double_lo == 0x19`, cet octet se faisait passer
/// pour un marqueur de model-id : l'ancien code cherchait alors le `1a` SUIVANT dans
/// **tout** le buffer, tombait sur le `1a` du VRAI model-id ~20 o plus loin, jugeait
/// l'« id » trop long (>12 o) et l'abandonnait — mais le curseur avait déjà sauté
/// par-dessus le vrai marqueur → `None` → « pull échoué » alors que le dump était bon.
/// C'était la cause du décrochage déterministe vers le 38ᵉ/39ᵉ cran (double semé à
/// `0xf3`, +1/cran → atteint `0x19` après 38 incréments), et de la fausse impression
/// de « plafond ctr 0x7794 » de l'addendum.
///
/// Deux garde-fous : (1) on borne la recherche du `1a` à `MODEL_ID_MAX_LEN` octets
/// (un model-id réel fait 1 à ~4 o) ; (2) si pas de `1a` dans la fenêtre, ce `19`
/// est une collision → on n'avance QUE de 1 (jamais par-dessus un marqueur ultérieur).
pub fn extract_first_module_hex_from_bulk(buf: &[u8]) -> Option<String> {
    /// Longueur max plausible d'un model-id entre `19` et `1a` (marge : observés ≤ 3 o).
    const MODEL_ID_MAX_LEN: usize = 8;
    let mut cursor = 0usize;
    while cursor < buf.len() {
        if buf[cursor] != 0x19 {
            cursor += 1;
            continue;
        }
        // Garde format alternatif : `83 17 c3 19 …` n'est pas un model-id.
        if cursor >= 3
            && buf[cursor - 3] == 0x83
            && buf[cursor - 2] == 0x17
            && buf[cursor - 1] == 0xc3
        {
            cursor += 1;
            continue;
        }
        let id_start = cursor + 1;
        let search_end = (id_start + MODEL_ID_MAX_LEN).min(buf.len());
        // Cherche le `1a` UNIQUEMENT dans la fenêtre : au-delà, ce `19` n'est pas un
        // marqueur (typiquement l'écho du double, p.ex. `double_lo == 0x19`).
        let Some(rel_end) = buf[id_start..search_end].iter().position(|&b| b == 0x1a) else {
            cursor += 1;
            continue;
        };
        let id_bytes = &buf[id_start..id_start + rel_end];
        if id_bytes.is_empty() {
            cursor += 1;
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

fn send_post_pull_resume_traffic(state: &mut HelixState) {
    let cnt_x1 = state.next_x1_cnt();
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18, 0x01, 0x10, 0xef, 0x03,
        0x00, cnt_x1, 0x00, 0x08, 0x72, 0x1e, 0x00, 0x00,
    ]));
    let cnt_x2 = state.next_x2_cnt();
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03,
        0x00, cnt_x2, 0x00, 0x10, 0x09, 0x10, 0x00, 0x00,
    ]));
}

fn finalize_pull_capture(
    state: &mut HelixState,
    extra: Option<&[u8]>,
) -> Option<SlotModelHwChangedPayload> {
    let slot_bus = state.hw_model_pull_slot_bus.take()?;
    state.hw_model_pull_capture_deadline = None;
    state.hw_model_pull_step = 0;
    if state.hw_model_pull_cd_lane != Some(0x04) {
        state.hw_model_pull_cd_lane = None;
    }
    state.hw_model_pull_echo_double = None;
    state.hw_model_pull_saw_final_bulk = false;

    // NB : aucune avance « entre pulls » ici. Le double a déjà été avancé de +1 par
    // CHAQUE OUT réellement émis (1b/19). Un pull raté n'aura avancé que du seul `1b`
    // — pas de sur-avance (cf. correctif freeze, supprime l'ancien +3 aveugle).

    let mut frames = Vec::new();
    std::mem::swap(&mut frames, &mut state.hw_model_pull_capture);
    if let Some(e) = extra {
        if frames.len() < PULL_CAPTURE_MAX_FRAMES {
            frames.push(e.to_vec());
        }
    }

    let Some(module_hex) = best_module_hex_from_frames(&frames) else {
        pull_trace("pull échoué (pas de bulk assignable)");
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
    state.hw_model_pull_last_at = Some(Instant::now());
    Some(payload)
}

pub fn ingest_pull_capture(
    state: &mut HelixState,
    data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    let deadline = state.hw_model_pull_capture_deadline?;
    let now = Instant::now();

    if state.hw_model_pull_capture.len() < PULL_CAPTURE_MAX_FRAMES {
        state.hw_model_pull_capture.push(data.to_vec());
    }

    remember_cd_lane_from_in(state, data);

    if state.hw_model_pull_step == 1 {
        try_ack_pull_interstitial_echo(state, data);
    }

    // Le `IN 21` (44 o) n'est PAS un reject : le device l'envoie parfois AVANT le dump
    // (livraison réordonnée), et le dump suit ~2-16 ms après (vérifié sur capture : notch
    // 11 → 21 puis IN 47/80o ; notch 22 → 21 puis IN 44/76o). Le `21` ne porte AUCUN
    // model-id, donc il ne déclenche pas la finalisation ci-dessous : on continue
    // simplement d'attendre le vrai dump. (On a retiré l'ancien retry-on-reject, fondé sur
    // la fausse prémisse « 21 = reject » — il réémettait un `1b` inutile.)

    // Grab-53 : on finalise dès qu'une trame porte un MODEL-ID (marqueur `83 66 cd` +
    // `19…1a`), QUEL QUE SOIT son head/sa longueur. Le device dumpe sous des formats très
    // variés (head 0x44/0x47/0x4c/0x4e/0x4f/0x50/0x51/0x53/0x54/0x55/0x56/0x68/0x6c, 76-116 o)
    // — se fier au model-id, pas au head, capte tous les cas (l'ancien `looks_like_first_pull_reply`
    // ratait p.ex. 0x44/76o → modèle émis en retard via le timeout). Le stub `1c` est exclu.
    if state.hw_model_pull_step == 1
        && !is_in_1c_stub(data)
        && frame_carries_model_id(data)
    {
        let payload = finalize_pull_capture(state, None);
        if let Some(ref p) = payload {
            if let Some(ref hex) = p.module_hex {
                log_hw_model_changed(hex);
            }
            return payload;
        }
        return None;
    }

    if is_pull_final_meta_bulk(data) {
        state.hw_model_pull_saw_final_bulk = true;
    }

    if state.hw_model_pull_step >= 3 && state.hw_model_pull_saw_final_bulk {
        let payload = finalize_pull_capture(state, None);
        if let Some(ref p) = payload {
            if let Some(ref hex) = p.module_hex {
                log_hw_model_changed(hex);
            }
        }
        return payload;
    }

    if now >= deadline {
        let payload = finalize_pull_capture(state, Some(data));
        if let Some(ref p) = payload {
            if let Some(ref hex) = p.module_hex {
                log_hw_model_changed(hex);
            }
        }
        return payload;
    }

    // Tick settling (pour détecter les 272 post-finalize tardifs).
    if tick_post_pull_stream_settling(state) {
        if !coalesce_last_enabled() && state.hw_model_pull_pending_slot_bus.is_some() {
            pull_trace("settling expiré — pending abandonné (prochain 1f utilisateur)");
            state.hw_model_pull_pending_slot_bus = None;
        }
    }

    None
}

// ── Émission slot vidé ────────────────────────────────────────────────────────

/// Slot vidé (notif `1f` None) — emit via `usb_listener` quand branché phase C.
#[allow(dead_code)]
pub fn emit_slot_cleared(
    state: &mut HelixState,
    data: &[u8],
) -> Option<SlotModelHwChangedPayload> {
    let slot_bus = slot_bus_for_model_pull(state, data)?;
    let slot_index = slot_bus_to_kempline_index(slot_bus)? as u32;
    if active_effect_slot_bus(state).is_none() {
        state.hw_active_slot_index = Some(slot_bus as usize);
        state.hw_active_slot_bus = Some(slot_bus);
    }
    state.hw_slot_content_sequence = state.hw_slot_content_sequence.wrapping_add(1);
    Some(SlotModelHwChangedPayload {
        sequence: state.hw_slot_content_sequence,
        slot_index,
        slot_bus,
        module_hex: None,
    })
}

// ── Logs ──────────────────────────────────────────────────────────────────────

/// `HX_SCROLL_PULL_DEBUG=1` au lancement (`lib.rs` → `init_from_env`).
pub fn init_from_env() {
    if std::env::var("HX_SCROLL_PULL_DEBUG")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        SCROLL_PULL_DEBUG.store(true, Ordering::SeqCst);
        eprintln!("[ScrollModelPull] debug activé — HX_SCROLL_PULL_DEBUG=1");
    }
    if couple_lane_enabled() {
        eprintln!("[ScrollModelPull] HX_PULL_COUPLE_LANE=1 — double+ctr = lanes ED03 vivantes, +1/OUT");
    }
}

pub fn scroll_pull_debug_enabled() -> bool {
    SCROLL_PULL_DEBUG.load(Ordering::SeqCst)
        || crate::helix::preset_debug_verbose_enabled()
}

fn pull_trace(msg: &str) {
    if scroll_pull_debug_enabled() {
        eprintln!("[ScrollModelPull] {msg}");
    }
}

fn log_hw_model_changed(module_hex: &str) {
    if !scroll_pull_debug_enabled() {
        return;
    }
    if let Some((chain_hex, name)) = model_catalog::resolve_chain_hex_and_name(module_hex) {
        eprintln!("[ScrollModelPull] model → \"{chain_hex}\"; \"{name}\"");
    } else {
        eprintln!("[ScrollModelPull] model → \"{module_hex}\"; \"(inconnu catalogue)\"");
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const IN1F: &[u8] = &[
        0x1f, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0xc1, 0x00, 0x04, 0x09, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x82, 0x69, 0x31, 0x6a, 0x84, 0x52,
        0x00, 0x44, 0x05, 0x79, 0x0a, 0x6a, 0x81, 0x62, 0x01, 0x93,
    ];
    const IN1F_NONE: &[u8] = &[
        0x1f, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x34, 0x00, 0x04, 0x09, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x82, 0x69, 0x31, 0x6a, 0x84, 0x52,
        0x00, 0x44, 0x05, 0x79, 0x0e, 0x6a, 0x81, 0x62, 0x01, 0xc0,
    ];

    /// +1 par OUT sur le double (motif HX f1→f2→f3), valable dans les deux modes.
    #[test]
    fn pull_double_plus1_each_out() {
        std::env::remove_var("HX_PULL_COUPLE_LANE");
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x01);
        state.hw_model_pull_ed03_double = 0x64ee;

        let p1b = build_pull_1b(&mut state, 0x01);
        assert_eq!(p1b[28], 0xef, "1b wire lo");
        assert_eq!(p1b[29], 0x64);
        assert_eq!(state.hw_model_pull_ed03_double, 0x64ef);

        let p19a = build_pull_19(&mut state, false);
        assert_eq!(p19a[28], 0xf0);

        let p19b = build_pull_19(&mut state, true);
        assert_eq!(p19b[28], 0xf1);
    }

    #[test]
    fn pull_trigger_1f_not_none() {
        assert!(is_hw_model_pull_trigger_notify(IN1F));
        assert!(!is_hw_model_pull_trigger_notify(IN1F_NONE));
    }

    #[test]
    fn cd_lane_switches_to_04_on_wrap() {
        std::env::remove_var("HX_PULL_COUPLE_LANE");
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x01);
        state.hw_model_pull_ed03_double = 0x64ff;
        let p = build_pull_1b(&mut state, 0x01);
        assert_eq!(p[27], 0x04, "cd_lane doit être 04 après wrap");
        assert_eq!(p[28], 0x00, "lo après wrap ff+1 = 00");
    }

    /// MULTI-CRANS : on traverse le wrap bas du double. Le `cd_lane` doit passer 03→04
    /// et le hi rester **0x64** (jamais 0x65) — c'est l'ancien bug `+3` sur u16 plein
    /// (0x64ff→0x6500) qui faisait taire le device « après quelques crans ».
    #[test]
    fn coupled_multi_notch_crosses_wrap_keeps_hi_64() {
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x01);
        // Graine proche du wrap (comme handle_in_layer_trigger la poserait au 1er pull).
        state.hw_model_pull_ed03_double = 0x64fb;

        // 6 OUT d'affilée (≈ 2 pulls : 1b+19+19, 1b+19+19) → fb,fc,fd,fe,ff,(00).
        let mut wires: Vec<(u8, u8, u8)> = Vec::new(); // (cd_lane, lo, hi)
        for i in 0..6u8 {
            let p = if i % 3 == 0 {
                build_pull_1b(&mut state, 0x01)
            } else {
                build_pull_19(&mut state, i % 3 == 2)
            };
            wires.push((p[27], p[28], p[29]));
        }

        // hi TOUJOURS 0x64, jamais 0x65.
        assert!(
            wires.iter().all(|&(_, _, hi)| hi == 0x64),
            "hi doit rester 0x64 à travers le wrap, jamais 0x65 : {wires:02x?}"
        );
        // On a bien traversé le wrap (un lo == 0x00 apparaît).
        let wrapped: Vec<_> = wires.iter().filter(|&&(_, lo, _)| lo == 0x00).collect();
        assert!(!wrapped.is_empty(), "le test doit traverser le wrap (lo→00)");
        // Après le wrap, cd_lane = 0x04 sur les OUT concernés.
        assert!(
            wrapped.iter().all(|&&(cd, _, _)| cd == 0x04),
            "cd_lane doit être 0x04 après le wrap : {wires:02x?}"
        );
    }

    /// Le `IN 21` (44 o) n'est PAS un reject : il ne porte aucun model-id → on ne finalise
    /// pas, on RESTE armé. Le dump qui suit (même AVEC un head/longueur inhabituels) finalise.
    #[test]
    fn in_21_is_not_reject_then_dump_finalizes() {
        std::env::remove_var("HX_PULL_COUPLE_LANE");
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x01);
        state.hw_active_slot_index = Some(0);
        state.hw_model_pull_ed03_double = 0x64f0;
        arm_pull_capture(&mut state, 0x01);

        // 21 (44 o, lane scroll, AUCUN model-id) → on attend, toujours armé.
        let mut notif = vec![0u8; 44];
        notif[0] = 0x21;
        notif[4..8].copy_from_slice(&[0xf0, 0x03, 0x02, 0x10]);
        let p = ingest_pull_capture(&mut state, &notif);
        assert!(p.is_none(), "le 21 ne finalise pas (pas de model-id)");
        assert_eq!(state.hw_model_pull_step, 1, "toujours armé, on attend le dump");
        assert!(state.hw_model_pull_capture_deadline.is_some());

        // Le dump arrive juste après → finalise avec le chainHex.
        let in53 = {
            let mut v = vec![0x53u8; 92];
            v[24..28].copy_from_slice(&[0x83, 0x66, 0xcd, 0x03]);
            v[44..49].copy_from_slice(&[0x19, 0xcd, 0x01, 0xfe, 0x1a]);
            v
        };
        let payload = ingest_pull_capture(&mut state, &in53).expect("dump après le 21");
        assert_eq!(payload.module_hex.as_deref(), Some("cd01fe"));
        assert_eq!(state.hw_model_pull_step, 0);
    }

    /// Régression notch 22 : un dump au head/longueur inhabituels (`0x44`, 76 o) doit être
    /// reconnu (on se fie au model-id, pas au head) et finaliser immédiatement.
    #[test]
    fn short_unusual_head_dump_is_recognized() {
        std::env::remove_var("HX_PULL_COUPLE_LANE");
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x01);
        state.hw_active_slot_index = Some(0);
        state.hw_model_pull_ed03_double = 0x64f0;
        arm_pull_capture(&mut state, 0x01);

        // Dump head 0x44, 76 o (cf. capture notch 22) : marqueur + `19 cd 02 a5 1a`.
        let in44 = {
            let mut v = vec![0x44u8; 76];
            v[24..28].copy_from_slice(&[0x83, 0x66, 0xcd, 0x03]);
            v[44..49].copy_from_slice(&[0x19, 0xcd, 0x02, 0xa5, 0x1a]);
            v
        };
        assert!(frame_carries_model_id(&in44), "0x44/76o doit être vu comme un dump");
        let payload = ingest_pull_capture(&mut state, &in44).expect("dump 0x44/76 reconnu");
        assert_eq!(payload.module_hex.as_deref(), Some("cd02a5"));
        assert_eq!(state.hw_model_pull_step, 0);
    }

    /// Mode grab-53 : la première réponse (`IN 53`) porte le chainHex → on FINALISE
    /// immédiatement (payload émis), sans envoyer aucun `19`, et l'étape retombe à 0.
    #[test]
    fn first_reply_53_grabs_chainhex_no_19s() {
        std::env::remove_var("HX_PULL_COUPLE_LANE");
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x01);
        state.hw_active_slot_index = Some(0);
        state.hw_model_pull_ed03_double = 0x64f2;

        arm_pull_capture(&mut state, 0x01);
        let _ = build_pull_1b(&mut state, 0x01);

        // 53 (92o) avec marqueur d'assignation + bloc model-id `19 cd 01 fe 1a`.
        let in53 = {
            let mut v = vec![0x53u8; 92];
            v[24..28].copy_from_slice(&[0x83, 0x66, 0xcd, 0x03]);
            v[44..49].copy_from_slice(&[0x19, 0xcd, 0x01, 0xfe, 0x1a]);
            v
        };
        let payload = ingest_pull_capture(&mut state, &in53).expect("chainHex extrait du 53");
        assert_eq!(payload.module_hex.as_deref(), Some("cd01fe"));
        assert_eq!(payload.slot_index, 0);
        assert_eq!(state.hw_model_pull_step, 0, "finalisé sur le 53, aucun 19 envoyé");
        assert!(state.hw_model_pull_capture_deadline.is_none());
    }

    /// Un stub `1c` en première réponse n'est PAS une vraie réponse : on patiente (étape 1).
    #[test]
    fn first_reply_1c_stub_waits() {
        std::env::remove_var("HX_PULL_COUPLE_LANE");
        let mut state = HelixState::new();
        state.hw_active_slot_bus = Some(0x01);
        state.hw_model_pull_ed03_double = 0x64f2;
        arm_pull_capture(&mut state, 0x01);

        let mut stub = vec![0x1cu8; 36];
        stub[24..27].copy_from_slice(&[0x83, 0x66, 0xcd]);
        ingest_pull_capture(&mut state, &stub);
        assert_eq!(state.hw_model_pull_step, 1, "stub 1c → on reste en attente");
    }

    #[test]
    fn extract_module_hex_from_bulk() {
        const IN92: &[u8] = &[
            0x53, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0xf9, 0x00, 0x04, 0x88, 0x03, 0x00,
            0x00, 0x00, 0x00, 0x06, 0x00, 0x43, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0xfc, 0x67,
            0x00, 0x68, 0x82, 0x0d, 0x01, 0x18, 0x82, 0x13, 0x06, 0x14, 0x85, 0x18, 0x83, 0x17, 0xc2,
            0x19, 0xcd, 0x01, 0xfe, 0x1a, 0xff, 0x09, 0x01, 0x0a, 0xc3, 0x0b, 0x83, 0x02, 0x04, 0x03,
            0x04, 0x04, 0x94,
        ];
        assert_eq!(
            extract_first_module_hex_from_bulk(IN92).as_deref(),
            Some("cd01fe")
        );
    }

    /// Régression « décrochage ~38ᵉ cran » : dump réel #3875 (capture
    /// `stomp_running_start_linux_multi_one_notch.json`) où l'écho du double vaut
    /// `0x19` (octet 28, juste après `83 66 cd 04`). L'ancien parseur prenait ce `19`
    /// pour un marqueur, engloutissait jusqu'au `1a` du vrai model-id (20 o plus loin),
    /// le rejetait (>12 o) en ayant sauté le vrai marqueur → `None` → « pull échoué ».
    /// Le device avait pourtant bien dumpé (`cd0209`). Doit désormais réussir.
    #[test]
    fn echo_double_0x19_does_not_mask_model_id() {
        const DUMP_3875: &[u8] = &[
            0x53, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x8b, 0x00, 0x04, 0xe8, 0x05,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x43, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04,
            0x19, 0x67, 0x00, 0x68, 0x82, 0x0d, 0x01, 0x18, 0x82, 0x13, 0x06, 0x14, 0x85, 0x18,
            0x83, 0x17, 0xc2, 0x19, 0xcd, 0x02, 0x09, 0x1a, 0xff, 0x09, 0x01, 0x0a, 0xc3, 0x0b,
            0x83, 0x02, 0x04, 0x03, 0x04, 0x04, 0x94, 0xca, 0x3f, 0x42, 0x8f, 0x5c, 0xca, 0x3f,
            0x21, 0x47, 0xae, 0xca, 0x3e, 0xd7, 0x0a, 0x3d, 0xca, 0x3f, 0x28, 0xf5, 0xc3, 0x0c,
            0x83, 0x02, 0x00, 0x03, 0x00, 0x04, 0x90, 0x00,
        ];
        assert_eq!(
            extract_first_module_hex_from_bulk(DUMP_3875).as_deref(),
            Some("cd0209"),
            "le model-id doit être lu malgré l'écho double 0x19 en tête"
        );
        assert!(
            frame_carries_model_id(DUMP_3875),
            "la trame doit être reconnue comme un dump"
        );
    }
}