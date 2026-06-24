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
        crate::helix::init_trace::trace("RequestPresetName::start (nom preset actif)");
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
                let wire_name = crate::helix::preset_name_wire::decode_from_transfer_buf(
                    &self.preset_name_data,
                );
                if state.pending_rename_name_verify {
                    if let Some((idx, decoded)) = wire_name {
                        state.preset_index = idx;
                        state.active_preset_name = Some(decoded.clone());
                        state.resolve_preset_index_from_active_name();
                        crate::helix::preset_name_wire::log_wire_preset(
                            "rename-verify",
                            idx,
                            Some(&decoded),
                        );
                        if idx < state.preset_names.len() {
                            state.preset_names[idx] = decoded;
                        }
                    } else {
                        eprintln!(
                            "[Preset] rename verify: impossible de décoder le nom depuis le fil"
                        );
                    }
                    state.pending_rename_name_verify = false;
                    crate::helix::init_trace::trace(
                        "RequestPresetName::done rename verify → Standard",
                    );
                    state.switch_mode(ModeRequest::Standard);
                    return false;
                }
                if let Some((idx, decoded)) = wire_name {
                    state.preset_index = idx;
                    state.active_preset_name = Some(decoded.clone());
                    state.resolve_preset_index_from_active_name();
                    crate::helix::preset_name_wire::log_wire_preset(
                        "RequestPresetName",
                        state.preset_index,
                        Some(&decoded),
                    );
                }
                // Noms déjà chargés (init HX Edit) : lire le corps du preset actif si absent.
                // Sinon retour Standard (changement preset nominal).
                if state.got_preset_names {
                    if !state.preset_data_ready || state.preset_data.is_empty() {
                        crate::helix::init_trace::trace(
                            "RequestPresetName::done → switch RequestPreset(false)",
                        );
                        state.switch_mode(ModeRequest::RequestPreset(false));
                    } else {
                        crate::helix::init_trace::trace(
                            "RequestPresetName::done → switch Standard (corps déjà en RAM)",
                        );
                        state.switch_mode(ModeRequest::Standard);
                    }
                } else {
                    crate::helix::init_trace::trace(
                        "RequestPresetName::done → switch RequestPreset(false)",
                    );
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