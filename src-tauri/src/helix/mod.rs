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
pub mod live_write;

use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::helix::packet::OutPacket;

/// Mapping index grille Kempline (0..15) -> slot bus observé en USB.
/// Path 1: 0..7 -> 0x01..0x08
/// Path 2: 8..15 -> 0x0b..0x12 (offset +0x03 observé dans les captures UI/HX Edit)
pub fn kempline_index_to_slot_bus(slot_index: usize) -> Option<u8> {
    if slot_index >= 16 {
        return None;
    }
    let i = slot_index as u8;
    if i < 8 {
        Some(i + 1)
    } else {
        Some(i + 3)
    }
}

/// Mapping slot bus USB -> index grille Kempline (0..15).
pub fn slot_bus_to_kempline_index(slot_bus: u8) -> Option<usize> {
    match slot_bus {
        0x01..=0x08 => Some((slot_bus - 1) as usize),
        0x0b..=0x12 => Some((slot_bus - 3) as usize),
        _ => None,
    }
}

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
#[allow(dead_code)]
pub enum ModeRequest {
    Connect,
    ReconfigureX1,
    RequestPresetName,
    RequestPresetNames,
    Standard,
    /// bool = content_only : true = lecture déclenchée par l'UI (revient à Standard),
    /// false = flux de démarrage (revient à RequestPresetNames).
    RequestPreset(bool),
    /// Fin de lecture preset émise par le timer/watchdog interne de RequestPreset.
    /// Le u64 est la génération de la lecture qui a armé le timer ; si elle ne correspond
    /// plus à `HelixState::preset_read_generation`, le message est orphelin et ignoré.
    StandardPresetRead(u64),
}

// ===========================================================
// HelixState — état partagé entre tous les threads
// ===========================================================
pub struct HelixState {

    pub session_quadruple: [u8; 4],

    pub got_preset: bool,
    pub request_preset_session_id: u8,

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
    // Nom du preset actif (mis à jour par `RequestPresetName`)
    pub active_preset_name: Option<String>,
    pub active_preset_name_index: Option<usize>,

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
    // Indique que `preset_names` vient d'être reconstruit par le mode
    // `RequestPresetNames` (utile pour éviter d'écraser une liste déjà corrigée
    // pendant d'autres transitions de mode).
    pub just_fetched_preset_names: bool,

    // Données brutes du dernier preset lu
    pub preset_data:       Vec<u8>,
    pub preset_data_ready: bool,

    // Si true : RequestPreset revient à Standard (clic utilisateur)
    // Si false : RequestPreset revient à RequestPresetNames (démarrage)
    pub preset_content_only: bool,

    /// Génération courante de lecture preset. Incrémentée à chaque démarrage de
    /// RequestPreset ; les StandardPresetRead(gen) avec gen != valeur courante sont
    /// des orphelins issus d'un watchdog/timer d'une lecture précédente et sont ignorés.
    pub preset_read_generation: u64,

    /// Dernier bloc `83 66 cd 05 … 1c` extrait d’un écho IN `27 … ed 03 03 10`
    /// (capture HX Edit). Sert à aligner les écritures live sur la session USB réelle.
    pub last_ed03_echo_model: Option<[u8; 16]>,
    /// Dernier octet de séquence (`… cd 05 XX …`) envoyé en OUT live tant qu’on n’a pas reçu d’écho IN.
    pub ed03_live_write_seq_sent: Option<u8>,
    /// Compteurs dédiés au write live `27` (reverse-engineering HX Edit).
    pub live_write_ctr: u16,
    pub live_write_yy: u8,

    /// Dernière paire **device → host** (IN `0x81`, 16 octets) « changement de slot » vue sur le bus.
    /// Schéma documenté dans `Line6_HX_Stomp_USB_Protocol.md` (capture `Slot1 to slot2 hardware.json`).
    pub hw_slot_notify_ed_in: Option<[u8; 16]>,
    pub hw_slot_notify_ef_in: Option<[u8; 16]>,
    /// +1 à chaque réception d’un EF03 court **alors qu’un** ED03 court était déjà mémorisé (cycle complet).
    pub hw_slot_notify_sequence: u32,
    /// Slot actif observé côté hardware (index Kempline 0..15), déduit du flux IN `0x81`.
    pub hw_active_slot_index: Option<usize>,
    /// +1 à chaque changement détecté de `hw_active_slot_index`.
    pub hw_active_slot_sequence: u32,
}

// ===========================================================
// Commandes vers le KeepAliveManager
// ===========================================================
#[derive(Debug)]
#[allow(dead_code)]
pub enum KeepAliveCommand {
    StartX1,
    StartX2,
    StartX80,
    StopAll,
}

static USB_PACKET_TRACE_ENABLED: AtomicBool = AtomicBool::new(false);
static USB_PACKET_TRACE_DELTA_ONLY: AtomicBool = AtomicBool::new(true);
static PRESET_DEBUG_VERBOSE_ENABLED: AtomicBool = AtomicBool::new(false);
static USB_IO_DIAG_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_usb_packet_trace_enabled(enabled: bool) {
    USB_PACKET_TRACE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn usb_packet_trace_enabled() -> bool {
    USB_PACKET_TRACE_ENABLED.load(Ordering::Relaxed)
}

pub fn set_usb_packet_trace_delta_only(enabled: bool) {
    USB_PACKET_TRACE_DELTA_ONLY.store(enabled, Ordering::Relaxed);
}

pub fn usb_packet_trace_delta_only() -> bool {
    USB_PACKET_TRACE_DELTA_ONLY.load(Ordering::Relaxed)
}

pub fn set_preset_debug_verbose_enabled(enabled: bool) {
    PRESET_DEBUG_VERBOSE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn preset_debug_verbose_enabled() -> bool {
    PRESET_DEBUG_VERBOSE_ENABLED.load(Ordering::Relaxed)
}

pub fn set_usb_io_diag_enabled(enabled: bool) {
    USB_IO_DIAG_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn usb_io_diag_enabled() -> bool {
    USB_IO_DIAG_ENABLED.load(Ordering::Relaxed)
}

/// Construit une empreinte "stable" pour réduire le bruit de traces :
/// certains keep-alive de 16 octets ne changent qu'au niveau d'un compteur.
pub fn usb_trace_fingerprint(data: &[u8]) -> Vec<u8> {
    let mut fp = data.to_vec();
    // Trames USB internes observées: header fixe 08 00 00 18, compteur roulant
    // en position 9 (0-based). On le neutralise pour la déduplication "delta-only".
    if fp.len() == 16 && fp.starts_with(&[0x08, 0x00, 0x00, 0x18]) {
        fp[9] = 0;
    }
    fp
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
            active_preset_name: None,
            active_preset_name_index: None,
            tx:                 None,
            mode_tx:            None,
            keepalive_tx:       None,
            connected:          false,
            got_preset_names:   false,
            just_fetched_preset_names: false,
            preset_data:        Vec::new(),
            preset_data_ready:  false,
            preset_content_only: false,
            preset_read_generation: 0,
            connecting:         true,
            got_preset:         false,
            preset_pkt_counter: 0x001e,
            request_preset_session_id: 0xf4,
            session_quadruple: [0xf4, 0x1e, 0x00, 0x00],
            last_ed03_echo_model: None,
            ed03_live_write_seq_sent: None,
            live_write_ctr: 0x6cbd,
            live_write_yy: 0x17,
            hw_slot_notify_ed_in: None,
            hw_slot_notify_ef_in: None,
            hw_slot_notify_sequence: 0,
            hw_active_slot_index: None,
            hw_active_slot_sequence: 0,
        }
    }

    /// Mémorise le suffixe « modèle » des échos paramètre (IN) pour caler les OUT live.
    pub fn ingest_ed03_param_echo(&mut self, data: &[u8]) {
        if data.len() < 16 {
            return;
        }
        // Heuristique large:
        // 1) repérer un marqueur ED03 plausible n'importe où dans la trame,
        // 2) repérer un bloc modèle `83 66 cd ??` n'importe où,
        // 3) conserver seulement si le bloc modèle est après le marqueur ED03.
        let mut ed03_off: Option<usize> = None;
        for i in 0..=data.len().saturating_sub(4) {
            let w = &data[i..i + 4];
            if *w == [0xed, 0x03, 0x03, 0x10]
                || *w == [0x03, 0x10, 0xed, 0x03]
                || *w == [0x80, 0x10, 0xed, 0x03]
            {
                ed03_off = Some(i);
                break;
            }
        }
        let Some(ed03_i) = ed03_off else {
            return;
        };

        let mut found: Option<([u8; 16], usize)> = None;
        for i in 0..=data.len().saturating_sub(16) {
            if data[i] == 0x83 && data[i + 1] == 0x66 && data[i + 2] == 0xcd {
                if i > ed03_i {
                    let mut b = [0u8; 16];
                    b.copy_from_slice(&data[i..i + 16]);
                    found = Some((b, i));
                    break;
                }
            }
        }
        let Some((b, model_i)) = found else {
            return;
        };
        self.last_ed03_echo_model = Some(b);
        // Nouvel écho device : on repart de sa séquence pour les prochains OUT.
        self.ed03_live_write_seq_sent = None;
        eprintln!(
            "[LiveWrite][echo-captured] len={} ed03_off={} model_off={} seq={:02x}",
            data.len(),
            ed03_i,
            model_i,
            b[4]
        );
    }

    /// Détecte les petites trames IN « slot » du Stomp (`ed 03 80 10` puis `ef 03 01 10` sur 16 octets).
    pub fn ingest_hw_slot_notify_in(&mut self, data: &[u8]) {
        // Keep-alive x2 long: recherche d'un marqueur `82 62 SS 1a`
        // où SS semble porter le slot bus (1..16) dans nos captures.
        for i in 0..=data.len().saturating_sub(4) {
            if data[i] == 0x82 && data[i + 1] == 0x62 && data[i + 3] == 0x1a {
                let slot_bus = data[i + 2];
                if let Some(slot_index) = slot_bus_to_kempline_index(slot_bus) {
                    if self.hw_active_slot_index != Some(slot_index) {
                        self.hw_active_slot_index = Some(slot_index);
                        self.hw_active_slot_sequence = self.hw_active_slot_sequence.wrapping_add(1);
                        if preset_debug_verbose_enabled() {
                            eprintln!(
                                "[PresetDebug][slot_active_in] seq={} slot_bus={} slot_index={}",
                                self.hw_active_slot_sequence,
                                slot_bus,
                                slot_index
                            );
                        }
                    }
                }
            }
        }

        if data.len() != 16 {
            return;
        }
        if !data.starts_with(&[0x08, 0x00, 0x00, 0x18]) {
            return;
        }
        let mut b = [0u8; 16];
        b.copy_from_slice(data);

        // Device IN : octets 4–7 = ed 03 80 10 (voir doc protocole)
        if data[4] == 0xed && data[5] == 0x03 && data[6] == 0x80 && data[7] == 0x10 {
            self.hw_slot_notify_ed_in = Some(b);
            self.hw_slot_notify_ef_in = None;
            return;
        }
        // Device IN : octets 4–7 = ef 03 01 10
        if data[4] == 0xef && data[5] == 0x03 && data[6] == 0x01 && data[7] == 0x10 {
            self.hw_slot_notify_ef_in = Some(b);
            if self.hw_slot_notify_ed_in.is_some() {
                self.hw_slot_notify_sequence = self.hw_slot_notify_sequence.wrapping_add(1);
                if preset_debug_verbose_enabled() {
                    let ed = self.hw_slot_notify_ed_in.unwrap();
                    eprintln!(
                        "[PresetDebug][slot_notify_in] pair_seq={} ed_tail={:02x}:{:02x}:{:02x}:{:02x} ef_tail={:02x}:{:02x}:{:02x}:{:02x}",
                        self.hw_slot_notify_sequence,
                        ed[12],
                        ed[13],
                        ed[14],
                        ed[15],
                        b[12],
                        b[13],
                        b[14],
                        b[15],
                    );
                }
            }
        }
    }

    pub fn increase_session_quadruple_x11(&mut self) {
        self.session_quadruple[0] = self.session_quadruple[0].wrapping_add(0x11);
        if self.session_quadruple[0] < 0x11 {
            self.session_quadruple[1] = self.session_quadruple[1].wrapping_add(0x01);
            if self.session_quadruple[1] == 0x00 {
                self.session_quadruple[2] = self.session_quadruple[2].wrapping_add(0x01);
                if self.session_quadruple[2] == 0x00 {
                    self.session_quadruple[3] = self.session_quadruple[3].wrapping_add(0x01);
                }
            }
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

}