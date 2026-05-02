// ===========================================================
// helix/modes/request_preset.rs
// Lecture des données du preset actif
// Protocole two-phase ED03 validé sur captures Wireshark
// ===========================================================

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::helix::{Mode, HelixState, ModeRequest, preset_debug_verbose_enabled};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;

pub struct RequestPreset {
    preset_data:             Vec<u8>,
    /// true = Phase 1 envoyée, en attente de la réponse 68 octets
    waiting_phase1_response: bool,
    /// Session choisie aléatoirement pour Phase 2 et tous les ACKs data
    phase2_session:          u8,
    /// Double (counter bytes) du dernier ACK chunk envoyé (pour le FDT ACK)
    last_ack_double:         [u8; 2],
    watchdog_cancel_tx:      Option<mpsc::Sender<()>>,
    mode_tx:                 Option<mpsc::Sender<ModeRequest>>,
}

impl RequestPreset {
    pub fn new() -> Self {
        Self {
            preset_data:             Vec::new(),
            waiting_phase1_response: false,
            phase2_session:          0,
            last_ack_double:         [0, 0],
            watchdog_cancel_tx:      None,
            mode_tx:                 None,
        }
    }

    fn cancel_watchdog(&mut self) {
        if let Some(tx) = self.watchdog_cancel_tx.take() {
            let _ = tx.send(());
        }
    }

    fn arm_watchdog(&mut self, mode_tx: Option<mpsc::Sender<ModeRequest>>, content_only: bool, generation: u64) {
        self.cancel_watchdog();
        if let Some(tx) = mode_tx {
            let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
            self.watchdog_cancel_tx = Some(cancel_tx);
            thread::spawn(move || {
                match cancel_rx.recv_timeout(Duration::from_millis(2000)) {
                    Ok(_) => {}
                    Err(_) => {
                        let next_mode = if content_only {
                            ModeRequest::StandardPresetRead(generation)
                        } else {
                            ModeRequest::RequestPresetNames
                        };
                        if preset_debug_verbose_enabled() {
                            eprintln!(
                                "[PresetDebug][RequestPreset::watchdog] timeout -> switch {:?}",
                                next_mode
                            );
                        }
                        let _ = tx.send(next_mode);
                    }
                }
            });
        }
    }

    /// Envoie Phase 2 (sub=0x0c, byte30=0x16) après réception de la réponse Phase 1.
    fn send_phase2(&mut self, state: &mut HelixState) {
        let cnt      = state.next_x80_cnt();
        let double   = state.next_preset_data_packet_double();
        let sess_id  = state.request_preset_session_id;
        state.request_preset_session_id = state.request_preset_session_id.wrapping_add(1);
        let cmd_type = state.ed03_cmd_type;

        let pkt = OutPacket::new(vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, cnt,  0x00, 0x0c,
            self.phase2_session, double[0], double[1], 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, cmd_type,
            sess_id, 0x64, 0x16, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ]);
        state.send(pkt);
        self.last_ack_double = double;
        self.waiting_phase1_response = false;

        if preset_debug_verbose_enabled() {
            eprintln!(
                "[PresetDebug][RequestPreset::send_phase2] cnt={:#04x} sess={:#04x} sess_id={:#04x} double={:02x}:{:02x}",
                cnt, self.phase2_session, sess_id, double[0], double[1]
            );
        }

        self.arm_watchdog(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation);
    }
}

impl Mode for RequestPreset {

    fn start(&mut self, state: &mut HelixState) {
        self.preset_data.clear();
        self.waiting_phase1_response = true;
        self.watchdog_cancel_tx = None;
        self.mode_tx = state.mode_tx.clone();

        let cnt      = state.next_x80_cnt();
        let sess1    = state.session_no;
        let double1  = state.preset_data_packet_double();   // pas d'incrément pour Phase 1
        let sess_id1 = state.request_preset_session_id;
        let cmd_type = state.ed03_cmd_type;

        // Session aléatoire indépendante pour Phase 2
        self.phase2_session = rand::random::<u8>().max(0x04);
        // Avancer sess_id de 1 pour que Phase 2 utilise sess_id1 + 1
        state.request_preset_session_id = state.request_preset_session_id.wrapping_add(1);

        if preset_debug_verbose_enabled() {
            eprintln!(
                "[PresetDebug][RequestPreset::start] preset_index={} content_only={} pkt_counter={:#06x} sess_id1={:#04x} sess1={:#04x} cmd_type={:#04x} phase2_session={:#04x}",
                state.preset_index,
                state.preset_content_only,
                state.preset_pkt_counter,
                sess_id1,
                sess1,
                cmd_type,
                self.phase2_session,
            );
        }

        // Phase 1 : sub=0x04, byte30=0x17 — demande du nom du preset
        let pkt = OutPacket::new(vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, cnt,  0x00, 0x04,
            sess1, double1[0], double1[1], 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, cmd_type,
            sess_id1, 0x64, 0x17, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ]);
        state.send(pkt);

        self.arm_watchdog(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation);
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {
        // x1/x2 keep-alive → acquitter silencieusement
        if Standard::check_keep_alive(data, state) {
            return false;
        }

        // Paquets x2
        if data.len() > 6 && data[6] == 0x02 {
            if state.preset_content_only {
                if byte_cmp(data, &pattern![
                    XX, 0x00, 0x00, 0x18,
                    0xf0, 0x03, 0x02, 0x10,
                    0x00, XX, 0x00, 0x04
                ], 12) {
                    let cnt = state.next_x2_cnt();
                    state.send(OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x02, 0x10, 0xf0, 0x03,
                        0x00, cnt, 0x00, 0x08,
                        0x74, 0x77, 0x00, 0x00,
                    ]));
                }
                return false;
            }
            let mut std = Standard;
            return std.data_in(data, state);
        }

        // Paquets non-x80 : déléguer à Standard
        if data.len() > 6 && data[6] != 0x80 {
            if preset_debug_verbose_enabled() {
                eprintln!(
                    "[PresetDebug][RequestPreset::data_in] non-x80 canal={:#04x} → Standard",
                    data[6]
                );
            }
            let mut std = Standard;
            return std.data_in(data, state);
        }

        // Paquet ED03 (canal x80) : valider le header
        if !byte_cmp(data, &pattern![
            XX, XX, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00, XX, 0x00, XX,
            XX, XX, 0x00, 0x00
        ], 16) {
            return true;
        }

        if data.len() < 12 {
            return true;
        }
        let sub = data[11];

        // LED color change (sub=0x04, 16 octets) : ACK identique à Standard.
        // Sans ACK, le device accumule des notifs sans réponse et finit par bloquer Phase 1.
        if sub == 0x04 && data.len() == 16 {
            state.increase_session_quadruple_x11();
            let sq = state.session_quadruple;
            let cnt = state.next_x80_cnt();
            state.send(OutPacket::with_delay(vec![
                0x08, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, cnt, 0x00, 0x08,
                sq[0], sq[1], sq[2], sq[3],
            ], 0));
            return true;
        }

        if self.waiting_phase1_response {
            // Réponse Phase 1 : sub=0x04, au moins 36 octets (64 ou 68 selon l'état device).
            // HXEdit envoie Phase 2 quelle que soit la taille exacte — ne pas contraindre à 68.
            if sub == 0x04 && data.len() >= 36 {
                if preset_debug_verbose_enabled() {
                    eprintln!("[PresetDebug][RequestPreset::data_in] Phase 1 réponse ({} octets) → envoi Phase 2", data.len());
                }
                self.send_phase2(state);
            }
            // Tout autre paquet pendant l'attente Phase 1 : ignorer
            return true;
        }

        // Phase transfert données
        match (data.len(), sub) {
            // FDT (fin-de-transfert) : 32 octets, sub=0x04, data[16]==0xa1
            // Doit être testé EN PREMIER pour ne pas être confondu avec un chunk partiel.
            (32, 0x04) if data[16] == 0xa1 => {
                let cnt         = state.next_x80_cnt();
                let fdt_session = self.phase2_session.wrapping_add(0x10);
                state.send(OutPacket::new(vec![
                    0x08, 0x00, 0x00, 0x18,
                    0x80, 0x10, 0xed, 0x03,
                    0x00, cnt, 0x00, 0x08,
                    fdt_session, self.last_ack_double[0], self.last_ack_double[1], 0x00,
                ]));
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] FDT total={} fdt_session={:#04x}",
                        self.preset_data.len(), fdt_session
                    );
                }
                self.cancel_watchdog();
                let next_mode = if state.preset_content_only {
                    ModeRequest::StandardPresetRead(state.preset_read_generation)
                } else {
                    ModeRequest::RequestPresetNames
                };
                if let Some(ref tx) = self.mode_tx {
                    let _ = tx.send(next_mode);
                }
                true
            }

            // Chunk de données preset : n'importe quelle taille, sub=0x04, au moins 1 octet de data.
            // Couvre les chunks complets (272 octets = 256 data) ET les chunks partiels finaux.
            (_, 0x04) if data.len() > 16 => {
                let chunk_data_len = data.len() - 16;
                self.preset_data.extend_from_slice(&data[16..]);
                let cnt        = state.next_x80_cnt();
                let new_double = state.next_preset_data_packet_double();
                state.send(OutPacket::new(vec![
                    0x08, 0x00, 0x00, 0x18,
                    0x80, 0x10, 0xed, 0x03,
                    0x00, cnt, 0x00, 0x08,
                    self.phase2_session, new_double[0], new_double[1], 0x00,
                ]));
                self.last_ack_double = new_double;
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] chunk len={} total={} ack cnt={:#04x} double={:02x}:{:02x}",
                        chunk_data_len, self.preset_data.len(), cnt, new_double[0], new_double[1]
                    );
                }
                if chunk_data_len < 256 {
                    // Chunk partiel = dernier chunk, pas de FDT pour ce type de preset.
                    if preset_debug_verbose_enabled() {
                        eprintln!(
                            "[PresetDebug][RequestPreset::data_in] chunk partiel → transfert complet total={}",
                            self.preset_data.len()
                        );
                    }
                    self.cancel_watchdog();
                    let next_mode = if state.preset_content_only {
                        ModeRequest::StandardPresetRead(state.preset_read_generation)
                    } else {
                        ModeRequest::RequestPresetNames
                    };
                    if let Some(ref tx) = self.mode_tx {
                        let _ = tx.send(next_mode);
                    }
                } else {
                    self.arm_watchdog(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation);
                }
                true
            }

            // Notification post-session (0x08) et heartbeat (0x10) : ignorer
            (_, 0x08) | (_, 0x10) => true,

            _ => {
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] ED03 inattendu len={} sub={:#04x}",
                        data.len(), sub
                    );
                }
                true
            }
        }
    }

    fn shutdown(&mut self, state: &mut HelixState) {
        self.cancel_watchdog();
        state.preset_data = std::mem::take(&mut self.preset_data);
        let has_data = !state.preset_data.is_empty();
        state.got_preset       = has_data;
        state.preset_data_ready = has_data;
        state.preset_content_only = false;
        // session_no déterministe post-Phase2 (comme HXEdit, observé en capture)
        state.session_no = if has_data {
            self.phase2_session.wrapping_add(0x10)
        } else {
            // Phase 2 jamais établie ou timeout : session aléatoire pour éviter un désalignement
            rand::random::<u8>().max(0x04)
        };
        state.ed03_cmd_type = state.ed03_cmd_type.wrapping_add(1);
        if !has_data {
            state.preset_pkt_counter = 0x001e;
            state.request_preset_session_id = 0xf4;
        }
        if preset_debug_verbose_enabled() {
            eprintln!(
                "[PresetDebug][RequestPreset::shutdown] preset_data_ready={} bytes={} session_no={:#04x} ed03_cmd_type={:#04x}",
                state.preset_data_ready,
                state.preset_data.len(),
                state.session_no,
                state.ed03_cmd_type
            );
        }
    }
}
