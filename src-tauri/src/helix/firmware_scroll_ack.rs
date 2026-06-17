//! ACK scroll firmware — IN `1d` / `1f` 40 o → OUT `f0:03` sub=`08` (lane dédiée).
//!
//! ## Règle HX Edit (capture)
//!
//! - Fond idle : **chaque** `1d` → ACK `f0/08` immédiat.
//! - Parfois : pas d’ACK sur le `1d` juste avant un `1f` pull (même rafale USB côté HX).
//!
//! ## Règle HXLinux (pragmatique)
//!
//! Linux reçoit souvent `1d` et `1f` sur des lectures séparées — on ne peut pas savoir,
//! au moment du `1d`, si un `1f` pull suivra. **Toujours ACKer le `1d` fond** ; si un pull
//! démarre ensuite, `ScrollModelPull` consomme le `1f` (pas d’ACK fond) et le `f0`
//! interstitiel du `1b` remet la lane en ordre.
//!
//! **Exception** : pendant une capture pull active (`hw_model_pull_capture_deadline`),
//! ne pas ACKer le fond (`1d`/`1f`) — le fil est piloté par la séquence `1b`/`19`/272.
//!
//! ## Checkpoint `EditorReady` (spec A — todo-scroll-hw.md §52-55)
//!
//! Le fond est **OFF** pendant tout `Bootstrapping` (amorçage + settle ~700 ms). Le Stomp
//! peut pousser des `1d` dès `ARM_f0` (vu sur captures Linux `one_notch` ET HX `one_notch`),
//! mais HX **n'y répond pas** tant que la session n'est pas `EditorReady`. C'est ce silence
//! host — pas l'absence de `1d` — qui empêche la boucle ACK↔notif ~75 ms observée sous Linux
//! (`stomp_running_start_linux_one_notch` : flux `1d` continu auto-entretenu par l'ACK 0,1 ms).
//!
//! ## Synchronisation de la lane sur les notifs **non ACKées** (mai 2026)
//!
//! Capture HX (`one_scroll`, `3_scroll`, `multi_scroll`) : la lane scroll avance de
//! `(payload_len + 8)` — `0x15` pour un `1d`, `0x17` pour un `1f` — à **chaque** notif émise
//! par le Stomp, et l'`OUT f0:03 sub=08` collé au `1b` porte la valeur **cumulée**. Le device
//! suit son propre compteur de notifs ; si l'host gèle la lane sur les notifs qu'il ne ré-ACK
//! pas (pré-scroll, capture pull), l'ACK du `1b` référence une notif antérieure et le device
//! répond `21` (assign) au lieu de dumper le modèle. On fait donc **toujours avancer la lane**
//! ([`HelixState::advance_firmware_scroll_lane`]) même quand on n'émet pas l'ACK.

use crate::helix::scroll_model_pull;
use crate::helix::HelixState;
use crate::helix::init_trace;
use crate::helix::packet::OutPacket;
use crate::helix::usb_in_pipeline::{LayerEffect, LayerResult};

/// Lane initiale après bootstrap connect (`09:10` sur le fil).
pub const SCROLL_LANE_BOOT: u16 = 0x1009;

/// Pas d’incrément lane (octets 12–13 LE du OUT `f0:03` sub=`08`) selon captures HX Edit.
pub(crate) fn scroll_ack_step(prev: Option<u8>, head: u8, skip_inc_once: bool) -> (u16, bool) {
    if skip_inc_once && prev == Some(0x1f) && head == 0x1d {
        return (0, false);
    }
    let step = match (prev, head) {
        (None, 0x1d) => 0x003f,
        (None, 0x1f) => 0x0017,
        (Some(0x1d), 0x1d) => 0x0015,
        (Some(0x1d), 0x1f) => 0x0017,
        (Some(0x1f), 0x1d) => 0x002e,
        (Some(0x1f), 0x1f) => 0x0017,
        (Some(0x1f), 0x21) => 0x0015,
        (Some(0x21), 0x1d) => 0x002e,
        (Some(0x21), 0x1f) => 0x0017,
        (Some(0x21), 0x21) => 0x0015,
        _ => 0x0015,
    };
    (step, skip_inc_once)
}

impl HelixState {
    pub fn firmware_scroll_lane_double(&self) -> [u8; 2] {
        let lo = (self.firmware_scroll_ack_ctr & 0xFF) as u8;
        let hi = ((self.firmware_scroll_ack_ctr >> 8) & 0xFF) as u8;
        [lo, hi]
    }

    /// Avance la lane scroll d'un cran (`scroll_ack_step`) **sans** émettre de paquet.
    ///
    /// À appeler pour **chaque** notif `1d`/`1f` reçue — y compris celles qu'on ne ré-ACK
    /// pas (pré-scroll, capture pull) — afin que la lane reste synchronisée avec le compteur
    /// de notifs du Stomp. Voir l'en-tête de module : geler la lane sur les notifs skippées
    /// désynchronise l'`OUT f0` du `1b` et fait répondre le device `21` au lieu de dumper.
    pub fn advance_firmware_scroll_lane(&mut self, head: u8) -> [u8; 2] {
        let (step, skip_next) = scroll_ack_step(
            self.firmware_scroll_ack_prev,
            head,
            self.firmware_scroll_skip_inc_once,
        );
        self.firmware_scroll_skip_inc_once = skip_next;
        self.firmware_scroll_ack_ctr = self.firmware_scroll_ack_ctr.wrapping_add(step);
        self.firmware_scroll_ack_prev = Some(head);
        self.firmware_scroll_lane_double()
    }

    /// Armement **lane** au bootstrap (`ARM_f0`) : réinitialise l'état du compteur scroll
    /// et marque la lane prête. ⚠ Ceci n'autorise PAS encore l'ACK fond : le gate réel est
    /// `EditorReady` (cf. `handle_in_layer`). Spec A §44 : l'amorçage **arme** le fond,
    /// il n'est pas le fond lui-même.
    pub fn note_firmware_scroll_bootstrap_sent(&mut self) {
        self.firmware_scroll_ack_ctr = SCROLL_LANE_BOOT;
        self.firmware_scroll_ack_prev = None;
        self.firmware_scroll_skip_inc_once = false;
        self.firmware_scroll_armed = true;
    }
}

fn is_scroll_fond_notify(data: &[u8]) -> bool {
    data.len() == 40
        && matches!(data.first(), Some(0x1d | 0x1f))
        && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
}

fn send_hw_model_scroll_ack(state: &mut HelixState, data: &[u8]) -> LayerResult {
    let head = data.first().copied().unwrap_or(0);
    let cnt = state.next_x2_cnt();
    let double = state.advance_firmware_scroll_lane(head);
    state.send(OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x02, 0x10, 0xf0, 0x03,
        0x00, cnt, 0x00, 0x08,
        double[0], double[1], 0x00, 0x00,
    ]));
    init_trace::trace_1d_ack_decision(true, if head == 0x1f { "1f" } else { "1d" });
    LayerResult::Consumed {
        effect: LayerEffect::ScrollLaneAndAck,
    }
}

/// Couche fond : IN `1d` / `1f` scroll → lane + ACK `f0:03` sub=`08`.
///
/// Gate spec A : ACK **uniquement** une fois `EditorReady` (phase 4 + settle terminés).
/// Pendant `Bootstrapping`, le fond reste silencieux même si le Stomp pousse des `1d` —
/// sinon l'ACK immédiat ouvre la boucle ~75 ms (cf. en-tête de module).
pub fn handle_in_layer(state: &mut HelixState, data: &[u8]) -> LayerResult {
    if !is_scroll_fond_notify(data) {
        return LayerResult::Ignored;
    }
    if !state.firmware_scroll_armed {
        init_trace::trace_1d_ack_decision(false, "scroll_not_armed");
        return LayerResult::Ignored;
    }
    // Checkpoint EditorReady (spec A) : pas d'ACK fond tant que l'amorçage + le settle
    // ~700 ms ne sont pas finis. `editor_ready` passe à true dans `amorcage` après le
    // settle, donc ce gate suffit — pas besoin de retarder l'armement de la lane.
    if !state.editor_ready || state.init_usb_settle_active() {
        init_trace::trace_1d_ack_decision(false, "fond_off_bootstrapping");
        return LayerResult::Ignored;
    }
    if !state.should_ack_firmware_1d_notify() {
        init_trace::trace_1d_ack_decision(false, "preset_usb_read");
        return LayerResult::Ignored;
    }
    if state.hw_model_pull_capture_deadline.is_some() {
        init_trace::trace_1d_ack_decision(false, "pull_capture_active");
        return LayerResult::Ignored;
    }

    let head = data.first().copied().unwrap_or(0);

    if head == 0x1d {
        // HX n'ACK pas le `1d` de pré-scroll (marqueur 82:69:31:6a, comme le 1f pull) :
        // l'ACKer (Consumed + OUT f0) décale la séquence côté host. On NE renvoie PAS d'ACK,
        // MAIS on fait avancer la lane (le Stomp a émis cette notif et incrémenté son
        // compteur). Geler la lane ici la désynchronise → le f0 interstitiel du `1b` porte
        // une valeur en retard → le device répond `21` au lieu de dumper.
        if scroll_model_pull::is_hw_model_scroll_1d(data) {
            let d = state.advance_firmware_scroll_lane(0x1d);
            init_trace::trace_1d_ack_decision(false, "pre_scroll_1d_skip");
            init_trace::trace_fmt(format_args!(
                "scroll lane avancée (1d pré-scroll, sans ACK) → {:02x}:{:02x}",
                d[0], d[1]
            ));
            return LayerResult::Ignored;
        }
        return send_hw_model_scroll_ack(state, data);
    }

    if head == 0x1f {
        if scroll_model_pull::would_start_hw_model_pull_on_1f(state, data) {
            // Le `1f` trigger est consommé par `ScrollModelPull` (couche précédente) : ce
            // chemin n'est en principe pas atteint pour le 1f pull. Conservé par sécurité :
            // on n'avance PAS la lane ici (l'avance du 1f est faite côté pull, juste avant
            // de construire le f0 interstitiel — voir scroll_model_pull).
            init_trace::trace_1d_ack_decision(false, "pull_will_start_skip_1f_ack");
            return LayerResult::Ignored;
        }
        return send_hw_model_scroll_ack(state, data);
    }

    LayerResult::Ignored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_1d_after_bootstrap_advances_to_1048() {
        let mut state = HelixState::new();
        state.note_firmware_scroll_bootstrap_sent();
        let d = state.advance_firmware_scroll_lane(0x1d);
        assert_eq!(d, [0x48, 0x10]);
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1048);
    }

    #[test]
    fn scroll_ack_step_1d_to_1d_is_0x15() {
        assert_eq!(scroll_ack_step(Some(0x1d), 0x1d, false).0, 0x0015);
    }

    #[test]
    fn scroll_ack_step_none_1d_is_0x3f() {
        assert_eq!(scroll_ack_step(None, 0x1d, false).0, 0x003f);
    }

    fn sample_scroll_1d() -> Vec<u8> {
        let mut b = vec![0u8; 40];
        b[0] = 0x1d;
        b[4..8].copy_from_slice(&[0xf0, 0x03, 0x02, 0x10]);
        b
    }

    /// Le `1d` pré-scroll n'est pas ACKé (Ignored) MAIS la lane AVANCE désormais
    /// (synchronisation avec le compteur device). Premier `1d` après bootstrap : +0x3f.
    #[test]
    fn fond_skips_pre_scroll_1d_but_advances_lane() {
        // 1d porteur du marqueur scroll (82:69:31:6a) — ne doit PAS être ACKé.
        let mut b = vec![0u8; 40];
        b[0] = 0x1d;
        b[4..8].copy_from_slice(&[0xf0, 0x03, 0x02, 0x10]);
        b[24..28].copy_from_slice(&[0x82, 0x69, 0x31, 0x6a]);
        let mut state = HelixState::new();
        state.editor_ready = true;
        state.note_firmware_scroll_bootstrap_sent();
        let r = handle_in_layer(&mut state, &b);
        assert!(matches!(r, LayerResult::Ignored), "pas d'ACK sur pré-scroll 1d");
        // (None, 0x1d) => +0x3f : 0x1009 + 0x3f = 0x1048
        assert_eq!(
            state.firmware_scroll_ack_ctr, 0x1048,
            "lane AVANCÉE même sans ACK (sync compteur device)"
        );
    }

    #[test]
    fn fond_skips_during_preset_read() {
        let mut state = HelixState::new();
        state.editor_ready = true;
        state.note_firmware_scroll_bootstrap_sent();
        state.set_preset_usb_read_modes_active(true);
        let r = handle_in_layer(&mut state, &sample_scroll_1d());
        assert!(matches!(r, LayerResult::Ignored));
        // Skip AVANT le head-match (gate preset_usb_read) : lane non touchée ici.
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1009);
    }

    /// Spec A : armé au bootstrap (`ARM_f0`) MAIS fond OFF tant que pas `EditorReady`.
    #[test]
    fn fond_silent_after_arm_before_editor_ready() {
        let mut state = HelixState::new();
        state.editor_ready = false;
        state.note_firmware_scroll_bootstrap_sent();
        let r = handle_in_layer(&mut state, &sample_scroll_1d());
        assert!(matches!(r, LayerResult::Ignored));
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1009);
    }

    /// Et pendant le settle (editor_ready encore false, fenêtre settle active) : silence.
    #[test]
    fn fond_silent_during_settle() {
        let mut state = HelixState::new();
        state.note_firmware_scroll_bootstrap_sent();
        state.begin_init_usb_settle();
        let r = handle_in_layer(&mut state, &sample_scroll_1d());
        assert!(matches!(r, LayerResult::Ignored));
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1009);
    }

    /// EditorReady atteint (settle terminé) : le fond ACK normalement (1d "fond" sans marqueur scroll).
    #[test]
    fn fond_acks_once_editor_ready() {
        let mut state = HelixState::new();
        state.note_firmware_scroll_bootstrap_sent();
        state.editor_ready = true;
        state.end_init_usb_settle();
        let r = handle_in_layer(&mut state, &sample_scroll_1d());
        assert!(matches!(r, LayerResult::Consumed { .. }));
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1048);
    }

    #[test]
    fn fond_silent_before_bootstrap_arm() {
        let mut state = HelixState::new();
        let r = handle_in_layer(&mut state, &sample_scroll_1d());
        assert!(matches!(r, LayerResult::Ignored));
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1009);
    }

    #[test]
    fn fond_silent_during_pull_capture() {
        let mut state = HelixState::new();
        state.editor_ready = true;
        state.note_firmware_scroll_bootstrap_sent();
        state.hw_model_pull_capture_deadline =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(1));

        let r = handle_in_layer(&mut state, &sample_scroll_1d());
        assert!(matches!(r, LayerResult::Ignored));
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1009);
    }

    #[test]
    fn fond_skips_1f_when_pull_will_start() {
        const IN1F_PULL: &[u8] = &[
            0x1f, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0xc1, 0x00, 0x04, 0x09, 0x02, 0x00,
            0x00, 0x00, 0x00, 0x04, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x82, 0x69, 0x31, 0x6a, 0x84, 0x52,
            0x00, 0x44, 0x05, 0x79, 0x0a, 0x6a, 0x81, 0x62, 0x01, 0x93,
        ];
        let mut state = HelixState::new();
        state.editor_ready = true;
        state.note_firmware_scroll_bootstrap_sent();
        state.hw_active_slot_bus = Some(0x01);

        assert!(scroll_model_pull::would_start_hw_model_pull_on_1f(&state, IN1F_PULL));
        let r = handle_in_layer(&mut state, IN1F_PULL);
        assert!(matches!(r, LayerResult::Ignored));
        // Ce chemin n'avance pas la lane (le pull le fait) → inchangée ici.
        assert_eq!(state.firmware_scroll_ack_ctr, 0x1009);
    }
}