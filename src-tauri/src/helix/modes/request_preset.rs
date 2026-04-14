// ===========================================================
// helix/modes/request_preset.rs
// Lecture des données du preset actif
// Traduction fidèle de request_preset.py de kempline
// ===========================================================

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::helix::{Mode, HelixState, ModeRequest};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;

pub struct RequestPreset {
    preset_data:    Vec<u8>,
    in_transfer:    bool,
    // Timer 20ms pour détecter fin des chunks
    timer_cancel_tx: Option<mpsc::Sender<()>>,
    mode_tx:        Option<mpsc::Sender<ModeRequest>>,
}

impl RequestPreset {
    pub fn new() -> Self {
        Self {
            preset_data:    Vec::new(),
            in_transfer:    false,
            timer_cancel_tx: None,
            mode_tx:        None,
        }
    }

    fn cancel_timer(&mut self) {
        if let Some(tx) = self.timer_cancel_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Kempline : threading.Timer(0.02, self.parse_preset_data)
    fn arm_timer(&mut self, state_mode_tx: Option<mpsc::Sender<ModeRequest>>) {
        self.cancel_timer();
        if let Some(mode_tx) = state_mode_tx {
            let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
            self.timer_cancel_tx = Some(cancel_tx);
            thread::spawn(move || {
                match cancel_rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(_)  => {}
                    Err(_) => {
                        println!("[RequestPreset] timer 20ms → fin du preset");
                        let _ = mode_tx.send(ModeRequest::RequestPresetNames);
                    }
                }
            });
        }
    }
}

impl Mode for RequestPreset {

    fn start(&mut self, state: &mut HelixState) {
        println!("[RequestPreset] démarré — preset_pkt_counter={:#06x} session_id={:#04x}", 
        state.preset_pkt_counter, 
        state.request_preset_session_id);
        self.preset_data.clear();
        self.in_transfer  = false;
        self.timer_cancel_tx = None;
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
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {

        // Keep-alive → acquitter silencieusement
        if Standard::check_keep_alive(data, state) {
            return false;
        }

        // Paquets x2 (changement de preset) — déléguer à Standard
        // Kempline : RequestPreset hérite de Standard, donc data_in de Standard est appelé
        if data.len() > 6 && data[6] == 0x02 {
            let mut std = crate::helix::modes::standard::Standard;
            println!("[RequestPreset] paquet x2) : {:02x?}", data);
            return std.data_in(data, state);
        }

        // Kempline : data_in[6] != 0x80 → paquet inattendu
        if data.len() > 6 && data[6] != 0x80 {
            println!("[RequestPreset] paquet inattendu (pas x80) : {:02x?}", data);
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
            // Annuler le timer en cours
            self.cancel_timer();
            self.in_transfer = false;

            let reply_here = !self.preset_data.is_empty();

            // Accumuler les données à partir de byte[16]
            if data.len() > 16 {
                self.preset_data.extend_from_slice(&data[16..]);
            }

            if !reply_here {
                // Premier paquet — on ne répond pas encore
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

            // Timer 20ms — si pas de nouveau paquet → fin
            self.arm_timer(state.mode_tx.clone());
            return true;
        }

        println!("[RequestPreset] paquet non reconnu : {:02x?}", data);
        true
    }

    fn shutdown(&mut self, state: &mut HelixState) {
        self.cancel_timer();
        state.got_preset = true;
        println!("[RequestPreset] arrêt — {} bytes de données preset", self.preset_data.len());
    }
}