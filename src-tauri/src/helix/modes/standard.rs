// ===========================================================
// helix/modes/standard.rs
// Mode nominal — gère tous les événements après connexion
// Traduction fidèle de standard.py de kempline
// ===========================================================

use crate::helix::{Mode, HelixState, ModeRequest};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::pattern;

pub struct Standard;

impl Standard {
    /// Vérifie si le paquet est un keep-alive entrant et acquitte.
    /// Kempline : check_keep_alive_response()
    pub fn check_keep_alive(data: &[u8], state: &mut HelixState) -> bool {
        // Pendant la connexion, on bloque x80 et x2
        if state.connecting {
            if byte_cmp(data, &pattern![XX, 0x00, 0x00, 0x18, 0xef, 0x03, 0x01, 0x10], 8) {
                return true; // ← ajouter x1
            }
            if byte_cmp(data, &pattern![XX, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10], 8) {
                return true;
            }
            if byte_cmp(data, &pattern![XX, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10], 8) {
                return true;
            }
        }
        // x1
        if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, XX, 0x00, 0x10
        ], 12) {
            return true;  // pas de send() ici
        }
        // x2
        if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x10
        ], 12) {
            return true;
        }
        false
    }
}

impl Mode for Standard {
    fn start(&mut self, _state: &mut HelixState) {
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {

        // LED COLOR CHANGE
        if byte_cmp(data, &pattern![
            XX, 0x00, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00, XX, 0x00, 0x04,
            XX, XX, XX, XX
        ], 16) {
            state.increase_session_quadruple_x11();
            let sq = state.session_quadruple;
            let cnt = state.next_x80_cnt();
            state.send(OutPacket::with_delay(vec![
                0x08, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, cnt,  0x00, 0x08,
                sq[0], sq[1], sq[2], sq[3],
            ], 0));
            return true;
        }

        // x2 ack générique
        if byte_cmp(data, &pattern![
            0x17, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x04,
            0x09, 0x02, 0x00, 0x00
        ], 16) {
            let cnt = state.next_x2_cnt();
            state.send(OutPacket::with_delay(vec![
                0x08, 0x00, 0x00, 0x18,
                0x02, 0x10, 0xf0, 0x03,
                0x00, cnt,  0x00, 0x08,
                0x74, 0x77, 0x00, 0x00,
            ], 10));
            return true;
        }

        // VIEW CHANGE
        if byte_cmp(data, &pattern![
            0x23, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x04,
            0x09, 0x02, 0x00, 0x00,
            0x00, 0x00, 0x04, 0x00,
            0x13, 0x00, 0x00, 0x00,
            0x82, 0x69, 0x16, 0x6a,
            0x84, 0x52, 0x00, 0x44,
            0x09, 0x79, 0x19, 0x6a,
            0x82, 0x76, 0xcd, 0x00,
            0x13, 0x77
        ], 42) {
            return true;
        }

        // UI MODE CHANGE
        if byte_cmp(data, &pattern![
            0x23, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x04,
            0x09, 0x02, 0x00, 0x00,
            0x00, 0x00, 0x04, 0x00,
            0x13, 0x00, 0x00, 0x00,
            0x82, 0x69, 0x16, 0x6a,
            0x84, 0x52, 0x00, 0x44,
            0x09, 0x79, 0x19, 0x6a,
            0x82, 0x76, 0xcd, 0x00,
            0x15, 0x77
        ], 42) {
            return true;
        }

        // PRESET SWITCH — pattern principal
        // Kempline : data[40] porte le numéro de preset
        if byte_cmp(data, &pattern![
            0x21, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x04,
            0x09, 0x02, 0x00, 0x00,
            0x00, 0x00, 0x04, 0x00,
            0x11, 0x00, 0x00, 0x00,
            0x82, 0x69, 0x04, 0x6a,
            0x84, 0x52
        ], 30) {
            let cnt = state.next_x2_cnt();
            state.send(OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x02, 0x10, 0xf0, 0x03,
                0x00, cnt,  0x00, 0x08,
                0x74, 0x77, 0x00, 0x00,
            ]));
            if data.len() > 40 {
                state.preset_index = data[40] as usize;
            }
            // Si activate_preset a posé le flag, c'est ce x2 de confirmation qu'on
            // attendait : on lance la lecture content_only maintenant que tous les x2
            // sont ACKés. Sinon, comportement normal : lire le nom du preset.
            if state.want_content_only_after_x2 {
                state.want_content_only_after_x2 = false;
                state.preset_content_only = true;
                state.switch_mode(ModeRequest::RequestPreset(true));
            } else {
                state.switch_mode(ModeRequest::RequestPresetName);
            }
            return true;
        }

        // PRESET SWITCH — pattern secondaire (pré-notification, arrive avant 0x04:6a)
        // Ce paquet précède toujours le pattern principal (0x04:6a) qui porte preset_index
        // et déclenche seul le read chain. ACK silencieux uniquement — déclencher
        // RequestPresetName ici provoque une double interruption du RequestPreset en cours.
        if byte_cmp(data, &pattern![
            0x21, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x04,
            0x09, 0x02, 0x00, 0x00,
            0x00, 0x00, 0x04, 0x00,
            0x11, 0x00, 0x00, 0x00,
            0x82, 0x69, 0x27, 0x6a,
            0x84, 0x52, 0x01, 0x44,
            0x03, 0x79, 0x13, 0x6a,
            0x82, 0x62
        ], 38) {
            let cnt = state.next_x2_cnt();
            state.send(OutPacket::with_delay(vec![
                0x08, 0x00, 0x00, 0x18,
                0x02, 0x10, 0xf0, 0x03,
                0x00, cnt,  0x00, 0x08,
                0x74, 0x77, 0x00, 0x00,
            ], 10));
            return true;
        }

        // PRESET SWITCH — paquets attendus silencieux
        if byte_cmp(data, &pattern![
            0x21, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x04,
            0x09, 0x02, 0x00, 0x00,
            0x00, 0x00, 0x04, 0x00,
            0x11, 0x00, 0x00, 0x00,
            0x82, 0x69, 0x08, 0x6a,
            0x84, 0x52, 0x01, 0x44,
            0x01, 0x79, 0x05, 0x6a,
            0x82, 0x6b, 0x00, 0x6c
        ], 40) {
            return false; // pass silencieux
        }

        // 0x27 sur x2 — paquets UI attendus
        if byte_cmp(data, &pattern![
            0x27, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX, 0x00, 0x04,
            0x09, 0x02, 0x00, 0x00
        ], 16) {
            return false; // pass silencieux
        }

        // 0x27 avec 0x10 et 0x77 — pass silencieux
        if byte_cmp(data, &pattern![
            0x27, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10
        ], 8) {
            return false;
        }

        // 0x23 sur x2 — pass silencieux
        if byte_cmp(data, &pattern![
            0x23, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10
        ], 8) {
            return false;
        }

        // ACK court x80 ed:03 (16 o) — ex. réponse au poll `80:10:ed:03` avec `00:08` ou `00:10`.
        // HXEdit ne renvoie pas un second OUT `…00:08…` sur ce IN : le prochain OUT est encore
        // un poll `…80:10:ed:03…00:10…` (voir connect_device_30s_HXEdit.json après IN `…00:10:c1:02:00:00`).
        // L’ancien bloc « variante 0x10 » (Kempline) consommait `next_x80_cnt()` et injectait un OUT
        // parasite qui désynchronisait la boucle keep-alive (ef/f0).
        if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00, XX, 0x00, 0x08,
            XX, XX, 0x00, 0x00
        ], 16) {
            return false;
        }
        if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00, XX, 0x00, 0x10,
            XX, XX, 0x00, 0x00
        ], 16) {
            return false;
        }

        // Paquets tardifs preset-names — acquitter silencieusement
        if byte_cmp(data, &pattern![
            0x08, 0x01, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, XX, 0x00, 0x04,
            XX, 0x02, 0x00, 0x00, XX
        ], 17) {
            let next_cnt = state.next_x1_cnt();
            let ack_cnt  = data[9].wrapping_add(9);
            state.send(OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, next_cnt, 0x00, 0x08,
                0x38, ack_cnt,  0x00, 0x00,
            ]));
            return false;
        }

        // Paquets tardifs preset-names variante
        if byte_cmp(data, &pattern![
            XX, 0x00, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, XX, 0x00, 0x04,
            XX, 0x02, 0x00, 0x00
        ], 16) {
            let next_cnt = state.next_x1_cnt();
            let ack_cnt  = data[9].wrapping_add(9);
            state.send(OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, next_cnt, 0x00, 0x08,
                0x38, ack_cnt,  0x00, 0x00,
            ]));
            return false;
        }

        // Keep-alives
        if Standard::check_keep_alive(data, state) {
            return false;
        }

        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {
    }
}