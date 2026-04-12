// ===========================================================
// helix/modes/standard.rs
// Mode de base — gère les événements communs à tous les modes
// Équivalent de Standard dans kempline
// ===========================================================

use crate::helix::{Mode, HelixState};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::pattern;

pub struct Standard;

impl Standard {
    /// Vérifie si le paquet est un keep-alive entrant et acquitte.
    /// Kempline : check_keep_alive_response()
    /// Retourne true si c'était un keep-alive (le mode n'a plus rien à faire)
    pub fn check_keep_alive(data: &[u8], state: &mut HelixState) -> bool {
        if state.connecting {
            if byte_cmp(data, &pattern![XX, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10], 8) {
                return true;
            }
            if byte_cmp(data, &pattern![XX, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10], 8) {
                return true;
            }
        }

        // x1 — byte[11] = 0x10
        if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, XX,   0x00, 0x10
        ], 12) {
            let cnt = state.next_x1_cnt();
            let pkt = OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, cnt,  0x00, 0x08,
                0x72, 0x1e, 0x00, 0x00,
            ]);
            state.send(pkt);
            return true;
        }

        // x2 — byte[11] = 0x10
        if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xf0, 0x03, 0x02, 0x10,
            0x00, XX,   0x00, XX
        ], 12) {
            let cnt = state.next_x2_cnt();
            let pkt = OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x02, 0x10, 0xf0, 0x03,
                0x00, cnt,  0x00, 0x08,
                0x74, 0x77, 0x00, 0x00,
            ]);
            state.send(pkt);
            return true;
        }

        // x80 — byte[11] = 0x10
        if byte_cmp(data, &pattern![
            0x08, 0x00, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00, XX,   0x00, XX
        ], 12) {
            let cnt = state.next_x80_cnt();
            let pkt = OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, cnt,  0x00, 0x08,
                0x20, 0x10, 0x00, 0x00,
            ]);
            state.send(pkt);
            return true;
        }

        false
    }
}

impl Mode for Standard {
    fn start(&mut self, _state: &mut HelixState) {
        println!("[Standard] mode démarré");
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {
        if Standard::check_keep_alive(data, state) {
            return false;
        }
        // Paquets non reconnus
        println!("[Standard] paquet non reconnu : {:02x?}", data);
        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {
        println!("[Standard] mode arrêté");
    }
}