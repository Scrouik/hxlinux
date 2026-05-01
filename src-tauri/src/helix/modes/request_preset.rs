// ===========================================================
// helix/modes/request_preset.rs
// Lecture des données du preset actif
// Traduction fidèle de request_preset.py de kempline
// ===========================================================

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::helix::{Mode, HelixState, ModeRequest, preset_debug_verbose_enabled};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;

pub struct RequestPreset {
    preset_data:    Vec<u8>,
    in_transfer:    bool,
    // Timer 20ms pour détecter fin des chunks
    timer_cancel_tx: Option<mpsc::Sender<()>>,
    // Watchdog global pour éviter de rester bloqué sans données
    watchdog_cancel_tx: Option<mpsc::Sender<()>>,
    mode_tx:        Option<mpsc::Sender<ModeRequest>>,
}

impl RequestPreset {
    pub fn new() -> Self {
        Self {
            preset_data:    Vec::new(),
            in_transfer:    false,
            timer_cancel_tx: None,
            watchdog_cancel_tx: None,
            mode_tx:        None,
        }
    }

    fn cancel_timer(&mut self) {
        if let Some(tx) = self.timer_cancel_tx.take() {
            let _ = tx.send(());
        }
    }

    fn cancel_watchdog(&mut self) {
        if let Some(tx) = self.watchdog_cancel_tx.take() {
            let _ = tx.send(());
        }
    }

    fn arm_watchdog(&mut self, state_mode_tx: Option<mpsc::Sender<ModeRequest>>, content_only: bool, generation: u64) {
        self.cancel_watchdog();
        if let Some(mode_tx) = state_mode_tx {
            let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
            self.watchdog_cancel_tx = Some(cancel_tx);
            thread::spawn(move || {
                // Le HX peut prendre un peu de temps juste après un changement de preset.
                match cancel_rx.recv_timeout(Duration::from_millis(2000)) {
                    Ok(_) => {}
                    Err(_) => {
                        // Envoyer StandardPresetRead(generation) : le mode loop vérifiera si
                        // cette génération est toujours courante avant d'agir.
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
                        let _ = mode_tx.send(next_mode);
                    }
                }
            });
        }
    }

    /// Kempline : threading.Timer(0.02, self.parse_preset_data)
    fn arm_timer(&mut self, state_mode_tx: Option<mpsc::Sender<ModeRequest>>, content_only: bool, generation: u64) {
        self.cancel_timer();
        if let Some(mode_tx) = state_mode_tx {
            let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
            self.timer_cancel_tx = Some(cancel_tx);
            thread::spawn(move || {
                match cancel_rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(_)  => {}
                    Err(_) => {
                        let next_mode = if content_only {
                            ModeRequest::StandardPresetRead(generation)
                        } else {
                            ModeRequest::RequestPresetNames
                        };
                        if preset_debug_verbose_enabled() {
                            eprintln!(
                                "[PresetDebug][RequestPreset::timer] timeout -> switch {:?}",
                                next_mode
                            );
                        }
                        let _ = mode_tx.send(next_mode);
                    }
                }
            });
        }
    }
}

impl Mode for RequestPreset {

    fn start(&mut self, state: &mut HelixState) {
        if preset_debug_verbose_enabled() {
            eprintln!(
                "[PresetDebug][RequestPreset::start] preset_index={} content_only={} pkt_counter={:#06x} session_id={:#04x}",
                state.preset_index,
                state.preset_content_only,
                state.preset_pkt_counter,
                state.request_preset_session_id
            );
        }
        self.preset_data.clear();
        self.in_transfer  = false;
        self.timer_cancel_tx = None;
        self.watchdog_cancel_tx = None;
        self.mode_tx = state.mode_tx.clone();

        let double  = state.preset_data_packet_double();
        let cnt     = state.next_x80_cnt();
        let session = state.session_no;
        let sess_id = state.request_preset_session_id;

        // Kempline : data = [0x19, 0x0, ..., maybe_session_no, double[0], double[1], ...]
        let pkt = OutPacket::new(vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, cnt,  0x00, 0x0c,
            session, double[0], double[1], 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            sess_id, 0x64, 0x16, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ]);
        state.send(pkt);

        // Incrémenter session_id comme kempline
        state.request_preset_session_id =
            state.request_preset_session_id.wrapping_add(2);

        // Si aucun paquet n'arrive, on sort proprement.
        self.arm_watchdog(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation);
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {

        // Keep-alive → acquitter silencieusement
        if Standard::check_keep_alive(data, state) {
            return false;
        }

        // Paquets x2 (changement de preset, keep-alive, etc.)
        // Pendant content_only : on envoie seulement l'ACK x2 générique sans déléguer à
        // Standard::data_in, car Standard peut modifier preset_index sur un PRESET SWITCH
        // hardware mid-read, ce qui corromprait la cohérence index / données lues.
        if data.len() > 6 && data[6] == 0x02 {
            if state.preset_content_only {
                // ACK x2 minimal pour ne pas bloquer le device, sans toucher preset_index.
                if byte_cmp(data, &pattern![
                    XX, 0x00, 0x00, 0x18,
                    0xf0, 0x03, 0x02, 0x10,
                    0x00, XX, 0x00, 0x04
                ], 12) {
                    let cnt = state.next_x2_cnt();
                    state.send(OutPacket::with_delay(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x02, 0x10, 0xf0, 0x03,
                        0x00, cnt, 0x00, 0x08,
                        0x74, 0x77, 0x00, 0x00,
                    ], 10));
                }
                return false;
            }
            let mut std = crate::helix::modes::standard::Standard;
            return std.data_in(data, state);
        }

        // Kempline : data_in[6] != 0x80 → paquet inattendu
        if data.len() > 6 && data[6] != 0x80 {
            if preset_debug_verbose_enabled() {
                eprintln!(
                    "[PresetDebug][RequestPreset::data_in] ignored non-x80 packet len={} type={:#04x}",
                    data.len(),
                    data[6]
                );
            }
            return true;
        }

        // Pattern principal : paquet de données preset sur x80
        // Kempline : my_byte_cmp(["XX","XX",0x0,0x18,0xed,0x3,0x80,0x10,...], length=16)
        if byte_cmp(data, &pattern![
            XX, XX, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00, XX, 0x00, XX,
            XX, XX, 0x00, 0x00
        ], 16) {
            // Un paquet valide arrive : on réarme le watchdog global.
            self.arm_watchdog(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation);
            // Annuler le timer en cours
            self.cancel_timer();
            self.in_transfer = false;

            let reply_here = !self.preset_data.is_empty();

            // Accumuler les données à partir de byte[16]
            if data.len() > 16 {
                self.preset_data.extend_from_slice(&data[16..]);
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] chunk len={} total={}",
                        data.len() - 16,
                        self.preset_data.len()
                    );
                }
            }

            if !reply_here {
                // Premier paquet — on ne répond pas encore
                if preset_debug_verbose_enabled() {
                    eprintln!("[PresetDebug][RequestPreset::data_in] first chunk received");
                }
                return true;
            }

            // Kempline : if data_in[1] != 2 → next_double, else → current_double
            let double = if data[1] != 2 {
                state.next_preset_data_packet_double()
            } else {
                state.preset_data_packet_double()
            };

            let cnt     = state.next_x80_cnt();
            let session = state.session_no;

            let pkt = OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, cnt,  0x00, 0x08,
                session, double[0], double[1], 0x00,
            ]);
            state.send(pkt);
            if preset_debug_verbose_enabled() {
                eprintln!(
                    "[PresetDebug][RequestPreset::data_in] ack sent cnt={:#04x} session={:#04x}",
                    cnt,
                    session
                );
            }

            // Timer 20ms — si pas de nouveau paquet → fin
            self.arm_timer(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation);
            return true;
        }

        true
    }

    fn shutdown(&mut self, state: &mut HelixState) {
        self.cancel_timer();
        self.cancel_watchdog();
        state.preset_data = std::mem::take(&mut self.preset_data);
        let has_data = !state.preset_data.is_empty();
        let mut request_state_reset = false;
        state.got_preset = has_data;
        state.preset_data_ready = has_data;
        state.preset_content_only = false;
        if has_data {
            state.new_session_no();
        } else {
            // Re-synchronize request state after a no-data timeout.
            // The capture shows ED03 requests can get "stuck" until we reset these fields.
            state.new_session_no();
            state.preset_pkt_counter = 0x001e;
            state.request_preset_session_id = 0xf4;
            request_state_reset = true;
        }
        if preset_debug_verbose_enabled() {
            eprintln!(
                "[PresetDebug][RequestPreset::shutdown] preset_data_ready={} bytes={} request_state_reset={} pkt_counter={:#06x} req_session_id={:#04x}",
                state.preset_data_ready,
                state.preset_data.len(),
                request_state_reset,
                state.preset_pkt_counter,
                state.request_preset_session_id
            );
        }
    }
}
