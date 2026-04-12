// ===========================================================
// helix/mod.rs
// Déclaration des modules + état partagé HelixState
// ===========================================================

pub mod packet;
pub mod usb_monitor;
pub mod usb_listener;
pub mod usb_writer;
pub mod keep_alive;
pub mod modes;

use std::sync::mpsc::Sender;
use crate::helix::packet::OutPacket;

// ===========================================================
// Trait Mode
// ===========================================================
pub trait Mode: Send {
    fn start(&mut self, state: &mut HelixState);
    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool;
    fn shutdown(&mut self, state: &mut HelixState);
}

// ===========================================================
// Enum pour demander un changement de mode
// Kempline : switch_mode("RequestPresetName") etc.
// ===========================================================
#[derive(Debug, Clone, PartialEq)]
pub enum ModeRequest {
    Connect,
    ReconfigureX1,
    RequestPresetName,
    RequestPresetNames,
    Standard,
}

// ===========================================================
// HelixState — état partagé entre tous les threads
// ===========================================================
pub struct HelixState {

    pub connecting: bool,

    // Compteur de paquets preset (kempline : preset_data_packet_double)
    pub preset_pkt_counter: u16,

    // Compteurs keep-alive
    pub x1_cnt:  u8,
    pub x2_cnt:  u8,
    pub x80_cnt: u8,

    // Session number aléatoire (kempline : maybe_session_no)
    pub session_no: u8,

    // Preset actif
    pub preset_index: usize,
    pub preset_names: Vec<String>,

    // Canal vers usb_writer
    pub tx: Option<Sender<OutPacket>>,

    // Canal vers le gestionnaire de mode
    // Quand un mode veut switcher, il pousse dans ce canal
    pub mode_tx: Option<Sender<ModeRequest>>,

    // Canal vers le keep_alive manager
    pub keepalive_tx: Option<Sender<KeepAliveCommand>>,

    // Flags
    pub connected:       bool,
    pub got_preset_names: bool,
}

// ===========================================================
// Commandes vers le KeepAliveManager
// ===========================================================
#[derive(Debug)]
pub enum KeepAliveCommand {
    StartX1,
    StartX2,
    StartX80,
    StopAll,
}

impl HelixState {
    pub fn new() -> Self {
        Self {
            x1_cnt:             0x02,
            x2_cnt:             0x02,
            x80_cnt:            0x02,
            session_no:         0x1a,
            preset_index:       0,
            preset_names:       Vec::new(),
            tx:                 None,
            mode_tx:            None,
            keepalive_tx:       None,
            connected:          false,
            got_preset_names:   false,
            preset_pkt_counter: 0x0716,
            connecting: true,
        }
    }

    

    /// Kempline : preset_data_packet_double()
    /// Retourne [lo, hi] du compteur courant
    pub fn preset_data_packet_double(&self) -> [u8; 2] {
        let lo = (self.preset_pkt_counter & 0xFF) as u8;
        let hi = ((self.preset_pkt_counter >> 8) & 0xFF) as u8;
        [lo, hi]
    }

    /// Kempline : next_preset_data_packet_double()
    /// Incrémente et retourne [lo, hi]
    pub fn next_preset_data_packet_double(&mut self) -> [u8; 2] {
        self.preset_pkt_counter = self.preset_pkt_counter.wrapping_add(1);
        self.preset_data_packet_double()
    }

    /// Envoie un paquet USB vers le HX
    pub fn send(&self, packet: OutPacket) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(packet);
        }
    }

    /// Demande un changement de mode
    /// Kempline : self.helix_usb.switch_mode()
    pub fn switch_mode(&self, req: ModeRequest) {
        if let Some(tx) = &self.mode_tx {
            let _ = tx.send(req);
        }
    }

    /// Démarre un keep-alive
    pub fn start_keepalive(&self, cmd: KeepAliveCommand) {
        if let Some(tx) = &self.keepalive_tx {
            let _ = tx.send(cmd);
        }
    }

    /// Incrémente compteur x1
        pub fn next_x1_cnt(&mut self) -> u8 {
            let current = self.x1_cnt;
            self.x1_cnt = self.x1_cnt.wrapping_add(1);
            current  // retourne l'ancienne valeur
        }

        pub fn next_x2_cnt(&mut self) -> u8 {
            let current = self.x2_cnt;
            self.x2_cnt = self.x2_cnt.wrapping_add(1);
            current
        }

        pub fn next_x80_cnt(&mut self) -> u8 {
            let current = self.x80_cnt;
            self.x80_cnt = self.x80_cnt.wrapping_add(1);
            current
        }

    /// Nouveau session_no aléatoire (kempline : rand 0x04..0xff)
    pub fn new_session_no(&mut self) {
        self.session_no = rand::random::<u8>().max(0x04);
    }

    /// Remplace les wildcards None par le compteur dans un template
    pub fn fill_packet(&self, template: &[Option<u8>], cnt: u8) -> Vec<u8> {
        template.iter().map(|b| match b {
            Some(v) => *v,
            None    => cnt,
        }).collect()
    }
}