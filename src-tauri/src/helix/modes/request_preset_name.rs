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
                    println!("[RequestPresetName] watchdog — pas de réponse, switch mode");
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
        println!("[RequestPresetName] démarré");
        self.preset_name_data.clear();

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

        if data.len() >= 39 && byte_cmp(&data[23..], &pattern![
            0x00, 0x83, 0x66, 0xcd, XX, XX,
            0x67, 0x00, 0x68, 0x86,
            0x6b, 0xcd, 0x00, 0x00,
            0x6c, 0xcd
        ], 16) {
            self.preset_name_data.extend_from_slice(&data[16..]);

            if data[1] == 0x00 {
                let slot_number_idx = 27;
                let mut preset_name = String::new();
                for i in slot_number_idx..slot_number_idx + 24 {
                    if i >= self.preset_name_data.len() { break; }
                    let b = self.preset_name_data[i];
                    if b == 0x00 { break; }
                    preset_name.push(b as char);
                }

                println!("[RequestPresetName] nom du preset : {}", preset_name);
                state.preset_names = vec![preset_name];
                state.new_session_no();

                // Annuler le watchdog avant de switcher
                self.cancel_watchdog();
                // Lire le preset actif depuis la réponse
                if self.preset_name_data.len() > 24 {
                    state.preset_index = self.preset_name_data[24] as usize;
                }
                state.switch_mode(ModeRequest::RequestPreset);
            }
            return false;
        }

        println!("[RequestPresetName] paquet non reconnu : {:02x?}", data);
        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {
        self.cancel_watchdog();
        println!("[RequestPresetName] arrêt");
    }
}