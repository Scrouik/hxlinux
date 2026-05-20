use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::helix::{Mode, HelixState, ModeRequest};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;

pub struct RequestPresetName {
    preset_name_data:   Vec<u8>,
    watchdog_cancel_tx: Option<mpsc::Sender<()>>,
}

impl RequestPresetName {
    pub fn new() -> Self {
        Self {
            preset_name_data:   Vec::new(),
            watchdog_cancel_tx: None,
        }
    }

    fn arm_watchdog(&mut self, mode_tx: mpsc::Sender<ModeRequest>) {
        self.cancel_watchdog();
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
        self.watchdog_cancel_tx = Some(cancel_tx);
        thread::spawn(move || {
            match cancel_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(_)  => { /* annulé — réponse reçue à temps */ }
                Err(_) => {
                    let _ = mode_tx.send(ModeRequest::Standard);
                }
            }
        });
    }

    fn cancel_watchdog(&mut self) {
        if let Some(tx) = self.watchdog_cancel_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Mode for RequestPresetName {

    fn start(&mut self, state: &mut HelixState) {
        self.preset_name_data.clear();
        // `active_preset_name` sera mis à jour lors de la réception complète.

        let double  = state.preset_data_packet_double();
        let session = state.session_no;
        let cnt     = state.next_x80_cnt();

        let pkt = OutPacket::new(vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, cnt,  0x00, 0x04,
            session, double[0], double[1], 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x04,
            0x04, 0x64, 0x17, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ]);
        state.send(pkt);

        if let Some(mode_tx) = state.mode_tx.clone() {
            self.arm_watchdog(mode_tx);
        }
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {
        if Standard::check_keep_alive(data, state) {
            return false;
        }

        // x2 PRESET SWITCH pendant la lecture du nom : ACK minimal, sans déclencher
        // switch_mode. Chaque x2 non ACKé pendant ce mode remplit la file x2 du device
        // et provoque des write timeouts après plusieurs changements consécutifs.
        if data.len() > 6 && data[6] == 0x02 {
            if byte_cmp(data, &pattern![
                XX, 0x00, 0x00, 0x18,
                0xf0, 0x03, 0x02, 0x10,
                0x00, XX, 0x00, 0x04
            ], 12) {
                let cnt = state.next_x2_cnt();
                let double = state.next_preset_dump_ack_double();
                state.send(OutPacket::with_delay(vec![
                    0x08, 0x00, 0x00, 0x18,
                    0x02, 0x10, 0xf0, 0x03,
                    0x00, cnt, 0x00, 0x08,
                    double[0], double[1], 0x00, 0x00,
                ], 10));
            }
            return false;
        }

        if data.len() >= 39 && byte_cmp(&data[23..], &pattern![
            0x00, 0x83, 0x66, 0xcd, XX, XX,
            0x67, 0x00, 0x68, 0x86,
            0x6b, 0xcd, 0x00, 0x00,
            0x6c, 0xcd
        ], 16) {
            self.preset_name_data.extend_from_slice(&data[16..]);

            if data[1] == 0x00 {
                state.new_session_no();

                // Annuler le watchdog avant de switcher
                self.cancel_watchdog();
                // Lire le preset actif depuis la réponse
                if self.preset_name_data.len() > 24 {
                    state.preset_index = self.preset_name_data[24] as usize;
                }

                // Extraire le nom du preset actif (traduction de request_preset_name.py)
                // Kempline : slot_number_idx = 27; lecture jusqu'à 27+24 (ou 0x00).
                let name_start = 27usize;
                let name_end = name_start.saturating_add(24);
                let mut decoded = String::new();
                if self.preset_name_data.len() > name_start {
                    let slice = &self.preset_name_data[name_start..self.preset_name_data.len().min(name_end)];
                    for &b in slice {
                        if b == 0x00 {
                            break;
                        }
                        decoded.push(if (32..=126).contains(&b) { b as char } else { '?' });
                    }
                }
                let decoded = if decoded.is_empty() { "<empty>".to_string() } else { decoded };
                state.active_preset_name = Some(decoded);
                state.active_preset_name_index = Some(state.preset_index);
                eprintln!(
                    "[PresetDebug][RequestPresetName] active preset={} name='{}'",
                    state.preset_index,
                    state.active_preset_name.as_ref().unwrap()
                );
                // Au démarrage, on garde la séquence historique complète.
                // En mode nominal (noms déjà chargés), on évite de relancer
                // RequestPreset/RequestPresetNames à chaque changement preset.
                if state.got_preset_names {
                    state.switch_mode(ModeRequest::Standard);
                } else {
                    state.switch_mode(ModeRequest::RequestPreset(false));
                }
            }
            return false;
        }

        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {
        self.cancel_watchdog();
    }
}