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
pub mod live_write_config;
pub mod edit_slot_model;
pub mod slot_focus_in;
pub mod slot_param_in;
pub mod slot_watch;
pub mod slot_model_hw_pull;
pub mod preset_dump_stream_ack;
pub mod model_catalog;
pub mod editor_phase4_bootstrap;
pub mod init_trace;

use std::sync::mpsc::Sender;
use std::time::Instant;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::Serialize;
use crate::helix::packet::OutPacket;

/// Émis vers le front (`models:hardware-slot-changed`) quand le bus de slot actif change (trafic `82 62 … 1a`).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareSlotChangedPayload {
    pub sequence: u32,
    pub slot_index: Option<usize>,
    pub slot_bus: Option<u8>,
}

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

/// Renvoie true si slot_bus correspond à un bloc structurel (Input/Output/Split/Merge).
pub fn is_special_slot_bus(slot_bus: u8) -> bool {
    matches!(slot_bus, 0x00 | 0x09 | 0x0a | 0x13)
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
    /// Attente `POST_PHASE4_SETTLE_MS` après bootstrap phase 4, puis `RequestPresetNames`.
    AwaitPostBootstrapSettle,
    Standard,
    /// bool = content_only : true = lecture UI (revient à Standard) ;
    /// false = dump corps preset actif (revient à Standard).
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
    /// Compteur cd:XX dans les ouvertures de session ED03 (bytes[27]).
    /// Initialisé à 0x01, incrémenté à chaque fin de RequestPreset.
    pub ed03_cmd_type: u8,

    pub connecting: bool,

    /// Dernier double ACK chunk preset (lane dump) — conservé pour debug / FDT ; le pull modèle utilise [`Self::editor_ed03_double`].
    pub preset_last_ack_double: [u8; 2],

    /// Lane ACK chunks preset (`80:10:ed:03` sub=`08`, octets 12–13 sur le fil).
    /// u16 LE : octet 12 = session (HX Edit varie ; expérience HW : fixe [`Self::request_preset_session_id`]),
    /// octet 13 = compteur global (+1 / ACK via +`0x0100`). Distinct de [`Self::editor_ed03_double`].
    pub preset_dump_ack_ctr: u16,

    /// Lane ACK scroll modèle HW (`1d` / `1f` 40 o → OUT `f0:03` sub=08, octets 12–13 LE).
    /// **Ne pas** utiliser [`Self::preset_dump_ack_ctr`] ni [`Self::live_write_ctr`].
    pub hw_model_scroll_ack_ctr: u16,
    pub hw_model_scroll_ack_prev: Option<u8>,
    /// Après `1f` « None », le premier `1d` suivant réémet le même double (step 0 une fois).
    pub hw_model_scroll_skip_inc_once: bool,
    /// `1d` modèle en attente : HX Edit n’ACK pas la paire `1d`/`1f` avant le `1b` pull.
    pub hw_model_scroll_deferred_1d: Option<Vec<u8>>,

    /// Octets 12–13 (LE) des OUT pull modèle `1b`/`19` — distinct de [`Self::live_write_ctr`]
    /// (sonde UI / live write / picker) pour éviter qu’un `probe_slot_model_usb` désynchronise le pull HW.
    pub hw_model_pull_ctr: u16,

    /// Compteur dédié à la lane éditeur ed:03 36 bytes (bytes 28-29, cd:03).
    /// Valeur initiale : 0x64e8 (observée HX Edit cold boot, Start_Model_change.json).
    /// S'incrémente de +1 à chaque OUT 36 bytes cd:03 (phase 4, pull 1b/19).
    /// NE PAS mélanger avec preset_dump_ack_ctr ni live_write_ctr.
    pub editor_ed03_double: u16,

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
    // Si false : RequestPreset revient à Standard (corps preset après noms + nom actif)
    pub preset_content_only: bool,

    /// Génération courante de lecture preset. Incrémentée à chaque démarrage de
    /// RequestPreset ; les StandardPresetRead(gen) avec gen != valeur courante sont
    /// des orphelins issus d'un watchdog/timer d'une lecture précédente et sont ignorés.
    pub preset_read_generation: u64,

    /// Si true : le prochain x2 (0x04:6a) déclenche RequestPreset(content_only=true)
    /// au lieu de RequestPresetName. Mis par `activate_preset` après le MIDI PC ;
    /// effacé par Standard::data_in quand le x2 arrive.
    pub want_content_only_after_x2: bool,

    /// Dernier bloc `83 66 cd 05 … 1c` extrait d’un écho IN `27 … ed 03 03 10`
    /// (capture HX Edit). Sert à aligner les écritures live sur la session USB réelle.
    pub last_ed03_echo_model: Option<[u8; 16]>,
    /// Dernier octet de séquence (`… cd 05 XX …`) envoyé en OUT live tant qu’on n’a pas reçu d’écho IN.
    pub ed03_live_write_seq_sent: Option<u8>,
    /// Compteurs dédiés au write live `27` (reverse-engineering HX Edit).
    pub live_write_ctr: u16,
    pub live_write_yy: u8,
    /// Compteur de session pour l'octet juste après `83 66 cd 03|04` sur
    /// l'assignation modèle de slot (profil 03:10 observé).
    pub slot_model_lane_seq: Option<u8>,

    /// Dernière paire **device → host** (IN `0x81`, 16 octets) « changement de slot » vue sur le bus.
    /// Schéma documenté dans `Line6_HX_Stomp_USB_Protocol.md` (capture `Slot1 to slot2 hardware.json`).
    pub hw_slot_notify_ed_in: Option<[u8; 16]>,
    pub hw_slot_notify_ef_in: Option<[u8; 16]>,
    /// +1 à chaque réception d’un EF03 court **alors qu’un** ED03 court était déjà mémorisé (cycle complet).
    pub hw_slot_notify_sequence: u32,
    /// **Source unique** du slot actif côté host (cadre orange UI + `slot_bus` des OUT pull modèle).
    ///
    /// Mis à jour uniquement par [`Self::ingest_hw_slot_notify_in`] quand une trame IN contient
    /// `82:62:SS:1a` (sélection HW/UI, ou preset chargé — souvent IN 44 o head `21` + `f0:03:02:10`,
    /// voir capture `Change_preset.json`). Pas de second registre ni de parse « preset » séparé.
    ///
    /// Index Kempline 0..15 ; `None` pour les blocs structurels (Input/Output/Split/Merge).
    pub hw_active_slot_index: Option<usize>,
    /// Bus brut du slot actif (`SS` dans `82:62:SS:1a`).
    pub hw_active_slot_bus: Option<u8>,
    /// +1 à chaque changement de `hw_active_slot_bus` (événement `models:hardware-slot-changed`).
    pub hw_active_slot_sequence: u32,

    /// Fenêtre de capture des IN `0x81` après un OUT « focus slot » (`sync_hardware_slot_focus_usb`).
    /// Remplie par `usb_listener` tant que `Instant::now() < deadline` (courte, ~55 ms ; max ~40 trames).
    pub usb_slot_focus_capture_deadline: Option<Instant>,
    pub usb_slot_focus_capture: Vec<Vec<u8>>,
    /// Dernier paquet IN « focus slot » parsé par index Kempline (rempli par `sync_hardware_slot_focus_usb`).
    pub last_slot_focus_capsule: [Option<slot_focus_in::SlotFocusInCapsule>; 16],
    /// Empreinte précédente pour surveillance contenu slot (modèle / vide / params).
    pub slot_watch_prev: [slot_watch::SlotWatchSnapshot; 16],
    pub hw_slot_content_sequence: u32,
    /// Déduplication des événements paramètre IN (`85:62…1c:PP:77`).
    pub slot_param_emit: slot_param_in::SlotParamEmitState,
    /// Fin du dernier pull modèle HW (cooldown scroll rapide).
    pub hw_model_pull_last_at: Option<Instant>,
    /// `1f` reçu pendant pull/cooldown : un pull sera relancé dès que possible (dernier slot gagne).
    pub hw_model_pull_pending_slot_bus: Option<u8>,
    /// Avant cet instant : pas de flush pending ni d’ACK `1d` (laisse finir bulks / `21` post-pull).
    pub hw_model_pull_quiet_until: Option<Instant>,
    /// Après finalize : 272 dump peuvent encore arriver — pas de nouveau `1b` avant silence.
    pub hw_model_post_pull_settling: bool,
    pub hw_model_post_pull_deadline: Option<Instant>,
    /// Collecte des IN bulk après pull modèle HW (parse `module_hex`).
    pub hw_model_pull_capture_deadline: Option<Instant>,
    pub hw_model_pull_capture: Vec<Vec<u8>>,
    pub hw_model_pull_slot_bus: Option<u8>,
    /// Machine d’états pull HX Edit : 0=off, 1=après `1b`+`f0`, 2=après 1er `19`, 3=après 2e `19`.
    pub hw_model_pull_step: u8,
    /// IN ~272 o reçu après `19` #2 (obligatoire avant finalize — sinon Stomp figé).
    pub hw_model_pull_saw_final_bulk: bool,
    /// Octet « voie » après `83:66:cd` vu dans la dernière IN stub (`02`/`03`/`04`…).
    pub hw_model_pull_cd_lane: Option<u8>,
    /// Octets après `83:66:cd:PP` renvoyés par l’IN `1c` (ex. `f7:67`) — recollés sur le retry `1b`.
    pub hw_model_pull_echo_double: Option<[u8; 2]>,
    /// Un retry `1b`+`f0` a déjà été tenté avec lane/double issus du `1c`.
    pub hw_model_pull_retried_1b: bool,
    /// Dernier IN scroll modèle (`1d`/`1f`) — évite `request_preset_content` en parallèle.
    pub hw_model_last_scroll_in_at: Option<Instant>,
    /// Fenêtre post bootstrap phase 4 (~700 ms HX Edit) : le host n’envoie **aucune** requête
    /// proactive (noms, dump, pull modèle, keep-alive poll) — seulement des ACK sur le trafic IN.
    pub init_usb_settle_until: Option<Instant>,
    /// IN `1d` = notif d’état firmware (scroll molette **ou** sync preset/slots), pas seulement l’utilisateur.
    /// `true` pendant `RequestPresetNames` / `RequestPresetName` / `RequestPreset` : ne pas ACK les `1d`
    /// (priorité dump sur la file OUT — HX Edit). `false` en Standard / init settle → ACK immédiat.
    pub suppress_1d_firmware_notify_ack: bool,
}

// ===========================================================
// Commandes vers le KeepAliveManager
// ===========================================================
#[derive(Debug)]
#[allow(dead_code)]
pub enum KeepAliveCommand {
    /// Un seul thread : cycle `ed:03` → `ef:03` → `f0:03` (queues fixes HX Edit).
    StartOrdered,
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

/// Pas d’incrément lane scroll ACK (octets 12–13 OUT) selon captures HX Edit.
pub(crate) fn hw_model_scroll_ack_step(
    prev: Option<u8>,
    head: u8,
    skip_inc_once: bool,
) -> (u16, bool) {
    if skip_inc_once && prev == Some(0x1f) && head == 0x1d {
        return (0, false);
    }
    let step = match (prev, head) {
        (None, _) => 0,
        (Some(0x1d), 0x1d) => 0x0015,
        (Some(0x1d), 0x1f) => 0x0017,
        (Some(0x1f), 0x1d) => 0x002e,
        (Some(0x1f), 0x1f) => 0x0017,
        // Après pull modèle : IN `21` 44 o puis reprise scroll `1d` (captures terrain).
        (Some(0x1f), 0x21) => 0x0015,
        (Some(0x21), 0x1d) => 0x002e,
        (Some(0x21), 0x1f) => 0x0017,
        (Some(0x21), 0x21) => 0x0015,
        _ => 0x0015,
    };
    (step, skip_inc_once)
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
            preset_dump_ack_ctr: ((0x1d_u16) << 8) | (0xf4_u16), // fil f4:1d — session = request_preset_session_id
            hw_model_scroll_ack_ctr: 0x1009, // scroll modèle HW (milieu session, capture none) // cold boot HXEdit (09:10 sur le fil)
            hw_model_scroll_ack_prev: None,
            hw_model_scroll_skip_inc_once: false,
            hw_model_scroll_deferred_1d: None,
            hw_model_pull_ctr: 0x6cbd, // aligné live_write au boot ; diverge après sonde UI
            editor_ed03_double: Self::preset_ed03_transaction_counter_before_first(),
            preset_last_ack_double: [0, 0],
            request_preset_session_id: 0xf4,
            ed03_cmd_type:      0x01,
            session_quadruple: [0xf4, 0x1e, 0x00, 0x00],
            last_ed03_echo_model: None,
            ed03_live_write_seq_sent: None,
            live_write_ctr: 0x6cbd,
            live_write_yy: 0x17,
            slot_model_lane_seq: None,
            want_content_only_after_x2: false,
            hw_slot_notify_ed_in: None,
            hw_slot_notify_ef_in: None,
            hw_slot_notify_sequence: 0,
            hw_active_slot_index: None,
            hw_active_slot_bus: None,
            hw_active_slot_sequence: 0,
            usb_slot_focus_capture_deadline: None,
            usb_slot_focus_capture: Vec::new(),
            last_slot_focus_capsule: std::array::from_fn(|_| None),
            slot_watch_prev: std::array::from_fn(|_| slot_watch::SlotWatchSnapshot::default()),
            hw_slot_content_sequence: 0,
            slot_param_emit: slot_param_in::SlotParamEmitState::default(),
            hw_model_pull_last_at: None,
            hw_model_pull_pending_slot_bus: None,
            hw_model_pull_quiet_until: None,
            hw_model_post_pull_settling: false,
            hw_model_post_pull_deadline: None,
            hw_model_pull_capture_deadline: None,
            hw_model_pull_capture: Vec::new(),
            hw_model_pull_slot_bus: None,
            hw_model_pull_step: 0,
            hw_model_pull_saw_final_bulk: false,
            hw_model_pull_cd_lane: None,
            hw_model_pull_echo_double: None,
            hw_model_pull_retried_1b: false,
            hw_model_last_scroll_in_at: None,
            init_usb_settle_until: None,
            suppress_1d_firmware_notify_ack: false,
        }
    }

    /// ACK immédiat des IN `1d` (ne pas figer le Stomp) sauf pendant la lecture preset USB.
    pub fn should_ack_firmware_1d_notify(&self) -> bool {
        if self.init_usb_settle_active() {
            return true;
        }
        !self.suppress_1d_firmware_notify_ack
    }

    /// Lecture noms/corps preset en cours (modes dédiés, pas le runtime Standard).
    pub fn preset_usb_read_in_progress(&self) -> bool {
        self.suppress_1d_firmware_notify_ack
    }

    pub fn set_preset_usb_read_modes_active(&mut self, active: bool) {
        self.suppress_1d_firmware_notify_ack = active;
        init_trace::trace_fmt(format_args!(
            "suppress_1d_firmware_notify_ack={active} (RequestPreset* : pas d'ACK 1d scroll)"
        ));
    }

    /// Début de la fenêtre « init USB » (aligné HX Edit après bootstrap phase 4).
    pub fn begin_init_usb_settle(&mut self) {
        self.init_usb_settle_until = Some(
            Instant::now() + std::time::Duration::from_millis(keep_alive::POST_PHASE4_SETTLE_MS),
        );
        init_trace::trace_fmt(format_args!(
            "init_usb_settle BEGIN ({} ms, ACK 1d autorisé)",
            keep_alive::POST_PHASE4_SETTLE_MS
        ));
    }

    pub fn end_init_usb_settle(&mut self) {
        self.init_usb_settle_until = None;
        init_trace::trace("init_usb_settle END → requêtes host autorisées");
    }

    pub fn init_usb_settle_active(&self) -> bool {
        self.init_usb_settle_until
            .is_some_and(|deadline| Instant::now() < deadline)
    }

    fn init_usb_settle_blocks_mode_request(req: &ModeRequest) -> bool {
        matches!(
            req,
            ModeRequest::RequestPreset(_)
                | ModeRequest::RequestPresetName
                | ModeRequest::RequestPresetNames
                | ModeRequest::ReconfigureX1
                | ModeRequest::Connect
        )
    }

    /// ACK flux IN `08:01:ed:03:80:10` sub=`04` (scroll HW / état slot) — lane `preset_dump_ack_ctr`.
    pub fn try_ack_preset_dump_stream_chunk_in(&mut self, data: &[u8]) -> bool {
        preset_dump_stream_ack::ack_preset_dump_stream_chunk(self, data)
    }

    /// Slot actif : notif `1d`/`1f` → pull échelonné (`1b` → `f0` → `19` ×2) → parse IN (hors `preset_data`).
    pub fn ingest_slot_model_hw_in(
        &mut self,
        data: &[u8],
    ) -> Option<slot_model_hw_pull::SlotModelHwChangedPayload> {
        slot_model_hw_pull::ingest_slot_model_hw_in(self, data)
    }

    /// Parse les trames IN bulk pour changements de paramètre live (`85:62…77:ca|c2|c3|…`).
    pub fn ingest_slot_param_in(
        &mut self,
        data: &[u8],
    ) -> Vec<slot_param_in::SlotParamChangedPayload> {
        if data.len() < 11 {
            return Vec::new();
        }
        self.slot_param_emit.ingest_buffer(data)
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

    /// Met à jour [`Self::hw_active_slot_*`] et notifie l’UI — **seul** chemin pour le slot actif.
    ///
    /// Scan linéaire de `82:62:SS:1a` sur **toute** trame IN (16 o keep-alive, 40 o switch UI,
    /// 44 o après preset, etc.). Ne pas ajouter de filtre par head `21` / `f0:03:02:10` : ce serait
    /// le même traitement dupliqué. Les paires courtes `ed:03:80:10` + `ef:03:01:10` (16 o) sont
    /// mémorisées en plus pour le debug protocole.
    pub fn ingest_hw_slot_notify_in(&mut self, data: &[u8]) -> Option<HardwareSlotChangedPayload> {
        if crate::helix::slot_model_hw_pull::is_hw_model_post_assign_21(data) {
            return None;
        }
        let mut slot_bus_changed: Option<HardwareSlotChangedPayload> = None;
        // Marqueur sélection / preset : `82:62:SS:1a` (≠ `81:62` des notifs changement modèle).
        for i in 0..=data.len().saturating_sub(4) {
            if data[i] == 0x82 && data[i + 1] == 0x62 && data[i + 3] == 0x1a {
                let slot_bus = data[i + 2];
                let slot_index = slot_bus_to_kempline_index(slot_bus);
                let is_special = is_special_slot_bus(slot_bus);
                if slot_index.is_some() || is_special {
                    if self.hw_active_slot_bus != Some(slot_bus) {
                        self.hw_active_slot_index = slot_index;
                        self.hw_active_slot_bus = Some(slot_bus);
                        self.hw_active_slot_sequence = self.hw_active_slot_sequence.wrapping_add(1);
                        if preset_debug_verbose_enabled() {
                            eprintln!(
                                "[PresetDebug][slot_active_in] seq={} slot_bus={:#04x} slot_index={:?}",
                                self.hw_active_slot_sequence,
                                slot_bus,
                                slot_index,
                            );
                        }
                        slot_bus_changed = Some(HardwareSlotChangedPayload {
                            sequence: self.hw_active_slot_sequence,
                            slot_index: self.hw_active_slot_index,
                            slot_bus: self.hw_active_slot_bus,
                        });
                    }
                }
            }
        }

        if data.len() != 16 {
            return slot_bus_changed;
        }
        if !data.starts_with(&[0x08, 0x00, 0x00, 0x18]) {
            return slot_bus_changed;
        }
        let mut b = [0u8; 16];
        b.copy_from_slice(data);

        // Device IN : octets 4–7 = ed 03 80 10 (voir doc protocole)
        if data[4] == 0xed && data[5] == 0x03 && data[6] == 0x80 && data[7] == 0x10 {
            self.hw_slot_notify_ed_in = Some(b);
            self.hw_slot_notify_ef_in = None;
            return slot_bus_changed;
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
        slot_bus_changed
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
    

    /// Première valeur wire du double transaction (`e8:64` dans `Start_Model_change.json`).
    pub const PRESET_ED03_TRANSACTION_FIRST: u16 = 0x64e8;

    /// Valeur interne pour que le premier [`Self::next_editor_ed03_double`] renvoie
    /// [`Self::PRESET_ED03_TRANSACTION_FIRST`].
    pub fn preset_ed03_transaction_counter_before_first() -> u16 {
        Self::PRESET_ED03_TRANSACTION_FIRST.wrapping_sub(1)
    }

    /// Repositionne la lane éditeur avant phase 4 / après échec preset (prochain OUT = `e8:64`).
    pub fn reset_preset_ed03_transaction_counter(&mut self) {
        self.editor_ed03_double = Self::preset_ed03_transaction_counter_before_first();
    }

    /// Alias historique (Kempline) → lane éditeur [`Self::editor_ed03_double`].
    pub fn preset_data_packet_double(&self) -> [u8; 2] {
        self.editor_ed03_double_val()
    }

    /// Double pour les ACK chunks preset (octets 12–13 du paquet `80:10:ed:03` sub=`08`).
    pub fn preset_dump_ack_double(&self) -> [u8; 2] {
        let lo = (self.preset_dump_ack_ctr & 0xFF) as u8;
        let hi = ((self.preset_dump_ack_ctr >> 8) & 0xFF) as u8;
        [lo, hi]
    }

    /// Valeur courante puis +`0x0100` sur l’octet 13 uniquement (session 12 figée).
    pub fn next_preset_dump_ack_double(&mut self) -> [u8; 2] {
        let out = self.preset_dump_ack_double();
        self.preset_dump_ack_ctr = (self.preset_dump_ack_ctr.wrapping_add(0x0100) & 0xff00)
            | (self.request_preset_session_id as u16);
        out
    }

    pub fn hw_model_scroll_ack_double(&self) -> [u8; 2] {
        let lo = (self.hw_model_scroll_ack_ctr & 0xFF) as u8;
        let hi = ((self.hw_model_scroll_ack_ctr >> 8) & 0xFF) as u8;
        [lo, hi]
    }

    /// Double renvoyé dans l’ACK scroll modèle ; avance la lane selon la transition `prev`→`head`.
    pub fn next_hw_model_scroll_ack_double(&mut self, head: u8) -> [u8; 2] {
        let (step, skip_next) = hw_model_scroll_ack_step(
            self.hw_model_scroll_ack_prev,
            head,
            self.hw_model_scroll_skip_inc_once,
        );
        self.hw_model_scroll_skip_inc_once = skip_next;
        let out = self.hw_model_scroll_ack_double();
        self.hw_model_scroll_ack_ctr = self.hw_model_scroll_ack_ctr.wrapping_add(step);
        self.hw_model_scroll_ack_prev = Some(head);
        out
    }

    /// Double pour la lane éditeur ed:03 36 bytes (phase 4, pull 1b/19).
    /// Valeur initiale : 0x64e8. Ne pas utiliser pour les ACK chunks.
    pub fn editor_ed03_double_val(&self) -> [u8; 2] {
        let lo = (self.editor_ed03_double & 0xFF) as u8;
        let hi = ((self.editor_ed03_double >> 8) & 0xFF) as u8;
        [lo, hi]
    }

    pub fn next_editor_ed03_double(&mut self) -> [u8; 2] {
        self.editor_ed03_double = self.editor_ed03_double.wrapping_add(1);
        self.editor_ed03_double_val()
    }

    /// Lane éditeur pour un pull modèle HW (`1b` puis `19`×2) : +3 sur l’**octet bas** uniquement
    /// (capture `3_scroll_HXEdit.json` : `fd:64` → `00:64` → `03:64`, haut `0x64` fixe).
    /// Les deux `19` du même pull réutilisent la même valeur ([`editor_ed03_double_val`]).
    pub fn next_editor_ed03_double_for_hw_model_pull(&mut self) -> [u8; 2] {
        const ED03_DOUBLE_HI: u8 = 0x64;
        let lo = ((self.editor_ed03_double & 0xFF) as u8).wrapping_add(3);
        self.editor_ed03_double = (ED03_DOUBLE_HI as u16) << 8 | u16::from(lo);
        [lo, ED03_DOUBLE_HI]
    }


    /// Envoie un paquet USB vers le HX
    pub fn send(&self, packet: OutPacket) {
        init_trace::trace_out(&packet.data, "send");
        if let Some(tx) = &self.tx {
            let _ = tx.send(packet);
        }
    }

    /// Demande un changement de mode
    /// Kempline : self.helix_usb.switch_mode()
    pub fn switch_mode(&self, req: ModeRequest) {
        if self.init_usb_settle_active() && Self::init_usb_settle_blocks_mode_request(&req) {
            init_trace::trace_fmt(format_args!(
                "switch_mode BLOCKED {:?} (init settle)",
                req
            ));
            if preset_debug_verbose_enabled() {
                eprintln!(
                    "[PresetDebug][init] switch_mode {req:?} ignoré (fenêtre {POST} ms ACK seulement)",
                    POST = keep_alive::POST_PHASE4_SETTLE_MS
                );
            }
            return;
        }
        init_trace::trace_fmt(format_args!("switch_mode enqueue {:?}", req));
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

#[cfg(test)]
mod hw_model_scroll_ack_tests {
    use super::*;

    #[test]
    fn scroll_ack_step_matches_hx_edit_05_assign() {
        let mut ctr = 0x1c24u16;
        let mut prev = None;
        for (head, expect_out, expect_ctr_after) in [
            (0x1d, 0x1c24, 0x1c24),
            (0x1f, 0x1c24, 0x1c3b),
            (0x1d, 0x1c3b, 0x1c69),
            (0x1d, 0x1c69, 0x1c7e),
        ] {
            let (step, _) = hw_model_scroll_ack_step(prev, head, false);
            let out = ctr;
            ctr = ctr.wrapping_add(step);
            assert_eq!(out, expect_out, "head={head:#x}");
            prev = Some(head);
            assert_eq!(ctr, expect_ctr_after, "head={head:#x}");
        }
    }

    #[test]
    fn scroll_ack_step_1d_to_1d_is_0x15() {
        assert_eq!(hw_model_scroll_ack_step(Some(0x1d), 0x1d, false).0, 0x0015);
    }
}

#[cfg(test)]
mod editor_ed03_pull_double_tests {
    use super::HelixState;

    #[test]
    fn hw_model_pull_double_plus_three_on_low_byte_only_hi_stays_64() {
        let mut state = HelixState::new();
        state.editor_ed03_double = 0x64fd;
        let d = state.next_editor_ed03_double_for_hw_model_pull();
        assert_eq!(d, [0x00, 0x64]);
        assert_eq!(state.editor_ed03_double, 0x6400);
        let d2 = state.next_editor_ed03_double_for_hw_model_pull();
        assert_eq!(d2, [0x03, 0x64]);
    }
}