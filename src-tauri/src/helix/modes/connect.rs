// ===========================================================
// helix/modes/connect.rs
// Séquence de connexion au HX Stomp XL
// Traduction fidèle de connect.py de kempline
// ===========================================================

use crate::helix::{Mode, HelixState};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;
use crate::helix::ModeRequest;

/// [TEST] Si `HX_F0_ARM_EARLY=1`, on envoie l'ARM f0 `09:10` dès la phase Connect
/// (juste après le `11 f0`, en miroir exact de la capture HX Edit paquet [19]),
/// au lieu de le laisser uniquement à `amorcage` post-ReconfigureX1.
///
/// Objectif : vérifier si cet ARM précoce déclenche le flux `IN 1d` de fond
/// (lane `f0:03:02:10` = état « éditeur vivant ») que le report n'obtient pas, et
/// qui conditionne le dump modèle au scroll. Voir `docs/scroll-dump-analysis.md`.
///
/// Désactivé par défaut → comportement strictement inchangé sans le flag.
fn f0_arm_early_enabled() -> bool {
    std::env::var("HX_F0_ARM_EARLY")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
}

pub struct Connect {
    received_x11_on_x2:   bool,
    received_x11_on_x80:  bool,
}

impl Connect {
    pub fn new() -> Self {
        Self {
            received_x11_on_x2:  false,
            received_x11_on_x80: false,
        }
    }
}

impl Mode for Connect {

    fn start(&mut self, state: &mut HelixState) {
        // Kempline : x1x10_cnt = x2x10_cnt = x80x10_cnt = 0x02
        state.x1_cnt  = 0x02;
        state.x2_cnt  = 0x02;
        state.x80_cnt = 0x02;
        state.connecting = true;
        state.editor_ready = false;
        state.firmware_scroll_armed = false;
        state.post_arm_sequence_started = false;
        state.post_ef_arm_ack_mask = 0;
        state.post_ef_arm_gate_active = false;
        state.post_ef_gate_rx = None;
        state.post_ef_gate_tx = None;
        state.phase4_bootstrap_active = false;
        state.phase4_complete_rx = None;
        state.phase4_complete_tx = None;

        self.received_x11_on_x2  = false;
        self.received_x11_on_x80 = false;

        // Premier paquet envoyé au HX — init x1
        // Kempline : data = [0xc, 0x0, 0x0, 0x28, 0x1, 0x10, 0xef, 0x3, ...]
        let pkt = OutPacket::new(vec![
            0x0c, 0x00, 0x00, 0x28,
            0x01, 0x10, 0xef, 0x03,
            0x00, 0x00, 0x00, 0x02,
            0x00, 0x01, 0x00, 0x21,
            0x00, 0x10, 0x00, 0x00,
        ]);
        state.send(pkt);
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {

        // -- Réponse init x1 → on envoie ack x1
        // Kempline : my_byte_cmp(right=[0xc,0x0,0x0,0x28,0xef,0x3,0x1,0x10,...], length=20)
        if byte_cmp(data, &pattern![
            0x0c, 0x00, 0x00, 0x28,
            0xef, 0x03, 0x01, 0x10,
            0x00, 0x00, 0x00, 0x02,
            0x00, 0x01, 0x00, 0x01,
            0x00, 0x02, 0x00, 0x00
        ], 20) {
            let cnt = state.next_x1_cnt();
            let pkt = OutPacket::new(vec![
                0x11, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, cnt,  0x00, 0x04,
                0x00, 0x10, 0x00, 0x00,
                0x01, 0x00, 0x05, 0x00,
                0x01, 0x00, 0x00, 0x00,
                0x05, 0x00, 0x00, 0x00,
            ]);
            state.send(pkt);

        // -- Reconfig x1 (0x28) → ack reconfig
        } else if byte_cmp(data, &pattern![
            0x28, 0x00, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, 0x02, 0x00, 0x04,
            0x09, 0x02
        ], 14) {
            let cnt = state.next_x1_cnt();
            let pkt = OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, cnt,  0x00, 0x08,
                0x20, 0x10, 0x00, 0x00,
            ]);
            state.send(pkt);

        // -- 2ème ack reconfig x1
        } else if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, 0x03, 0x00, XX,
            0x09, 0x02, 0x00, 0x00
        ], 16) {
            let cnt = state.next_x1_cnt();
            let pkt = OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, cnt,  0x00, 0x02,
                0x20, 0x10, 0x00, 0x00,
            ]);
            state.send(pkt);

        // -- Init x80
        } else if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, 0x04, 0x00, XX,
            0x09, 0x02, 0x00, 0x00
        ], 16) {
            let pkt = OutPacket::new(vec![
                0x0c, 0x00, 0x00, 0x28,
                0x80, 0x10, 0xed, 0x03,
                0x00, 0x00, 0x00, 0x02,
                0x00, 0x01, 0x00, 0x21,
                0x00, 0x10, 0x00, 0x00,
            ]);
            state.send(pkt);

        // -- Réponse init x80 → ack x80
        } else if byte_cmp(data, &pattern![
            0x0c, 0x00, 0x00, 0x28,
            0xed, 0x03, 0x80, 0x10,
            0x00, 0x00, 0x00, 0x02,
            0x00, 0x01, 0x00, 0x01,
            0x00, 0x02, 0x00, 0x00
        ], 20) {
            let cnt = state.next_x80_cnt();
            let pkt = OutPacket::new(vec![
                0x11, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, cnt,  0x00, 0x04,
                0x00, 0x10, 0x00, 0x00,
                0x01, 0x00, 0x06, 0x00,
                0x01, 0x00, 0x00, 0x00,
                0x06, 0x00, 0x00, 0x00,
            ]);
            state.send(pkt);

        // -- x11 reçu sur x80 → init x2 (subscribe f0:03)
        } else if byte_cmp(data, &pattern![
            0x11, 0x00, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00, 0x02
        ], 10) {
            self.received_x11_on_x80 = true;
            // Poll `ed` démarré après `EditorReady` dans `amorcage` (`StartOrdered`).
            let pkt = OutPacket::new(vec![
                0x0c, 0x00, 0x00, 0x28,
                0x02, 0x10, 0xf0, 0x03,
                0x00, 0x00, 0x00, 0x02,
                0x00, 0x01, 0x00, 0x21,
                0x00, 0x10, 0x00, 0x00,
            ]);
            state.send(pkt);

        // -- Réponse init x2 → ARM_ed puis ack x2 (HX Edit #1455 → #1457)
        } else if byte_cmp(data, &pattern![
            0x0c, 0x00, 0x00, 0x28,
            0xf0, 0x03, 0x02, 0x10,
            0x00, 0x00, 0x00, 0x02,
            0x00, 0x01, 0x00, 0x01,
            0x00, 0x02, 0x00, 0x00
        ], 20) {
            crate::helix::amorcage::send_arm_ed(state);
            let cnt = state.next_x2_cnt();
            let pkt = OutPacket::new(vec![
                0x11, 0x00, 0x00, 0x18,
                0x02, 0x10, 0xf0, 0x03,
                0x00, cnt,  0x00, 0x04,
                0x00, 0x10, 0x00, 0x00,
                0x01, 0x00, 0x04, 0x00,
                0x01, 0x00, 0x00, 0x00,
                0x04, 0x00, 0x00, 0x00,
            ]);
            state.send(pkt);

        // -- x11 reçu sur x2 (handshake f0 OK) — ARM `09:10` reporté à `amorcage` post-ReconfigureX1
        } else if byte_cmp(data, &pattern![
            0x11, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, 0x02, 0x00, 0x04,
            0x09, 0x02
        ], 14) {
            self.received_x11_on_x2 = true;

            // ── [TEST HX_F0_ARM_EARLY] ───────────────────────────────────────
            // HX Edit envoie l'ARM f0 `09:10` ICI (capture [19]), dans la foulée
            // du `11 f0` et AVANT la phase 4 ; son device démarre alors le flux
            // `IN 1d` de fond (lane f0:03:02:10) dès le paquet [23]. Chez nous cet
            // ARM est reporté à `amorcage` et le flux ne démarre jamais → le scroll
            // ne dumpe pas. On teste l'ARM précoce, additif et flag-gardé.
            //
            // cnt = next_x2_cnt() → 0x03 à ce point (le `11 f0` ci-dessus a pris
            // 0x02), soit exactement HX [19]. L'ARM différé dans `amorcage` reste
            // en place : si le flux démarre puis est perturbé, on l'aura quand même
            // VU apparaître au bootstrap, ce qui suffit à valider l'hypothèse.
            if f0_arm_early_enabled() {
                let cnt = state.next_x2_cnt();
                let pkt = OutPacket::new(vec![
                    0x08, 0x00, 0x00, 0x18,
                    0x02, 0x10, 0xf0, 0x03,
                    0x00, cnt,  0x00, 0x08,
                    0x09, 0x10, 0x00, 0x00,
                ]);
                crate::helix::init_trace::trace(
                    "[HX_F0_ARM_EARLY] ARM f0 09:10 précoce (miroir HX [19])",
                );
                state.send(pkt);
            }
            // ─────────────────────────────────────────────────────────────────

        // -- Paquet 0x54 sur x80
        } else if byte_cmp(data, &pattern![
            0x54, 0x00, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00
        ], 9) {
            let pkt = OutPacket::new(vec![
                0x1c, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, state.x80_cnt, 0x00, 0x0c,
                0x55, 0x10, 0x00, 0x00,
                0x01, 0x00, 0x06, 0x00,
                0x0c, 0x00, 0x00, 0x00,
                0x83, 0x66, 0xcd, 0x03,
                0xe9, 0x64, 0x18, 0x65,
                0x81, 0x76, 0xcc, 0x80,
            ]);
            state.send(pkt);

        // -- Paquet 0x1f sur x80
        } else if byte_cmp(data, &pattern![
            0x1f, 0x00, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00
        ], 9) {
            let pkt = OutPacket::new(vec![
                0x19, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, state.x80_cnt, 0x00, 0x0c,
                0x6c, 0x10, 0x00, 0x00,
                0x01, 0x00, 0x06, 0x00,
                0x09, 0x00, 0x00, 0x00,
                0x83, 0x66, 0xcd, 0x03,
                0xea, 0x64, 0x17, 0x65,
                0xc0, 0x00, 0x00, 0x00,
            ]);
            state.send(pkt);

        // -- Keep-alive → acquitter silencieusement
        } else if Standard::check_keep_alive(data, state) {
            return false;

        }

        // Si on a reçu x11 sur x2 ET sur x80 → connexion établie
        if self.received_x11_on_x2 && self.received_x11_on_x80 {
            state.connecting = false;  // ← ajouter cette ligne
            state.connected = true;
            state.switch_mode(ModeRequest::ReconfigureX1);
        }

        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {
    }
}