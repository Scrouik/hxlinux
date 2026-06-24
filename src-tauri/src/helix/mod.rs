// ===========================================================
// helix/mod.rs
// Déclaration des modules + état partagé HelixState
// ===========================================================

pub mod ed03_lane;
pub mod packet;
pub mod usb_monitor;
pub mod usb_listener;
pub mod usb_writer;
pub mod keep_alive;
pub mod modes;
pub mod live_write;
pub mod live_write_config;
pub mod amp_cab_live_write;
pub mod amp_cab_cab_replace;
pub mod cab_dual_live_write;
pub mod cab_dual;
pub mod cab_dual_cab2_replace;
pub mod hx_edit_console_cmds;
pub mod path1_io_live_write;
pub mod path1_split_live_write;
pub mod edit_slot_model;
pub mod slot_focus_in;
pub mod slot_param_in;
pub mod slot_watch;
pub mod firmware_scroll_ack;
pub mod legacy_cab_param_commit;
pub mod scroll_model_pull;
pub mod usb_in_pipeline;
pub mod preset_dump_stream_ack;
pub mod model_catalog;
pub mod editor_phase4_bootstrap;
pub mod editor_go_live;
pub mod amorcage;
pub mod phase4_state;
pub mod init_trace;

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Instant;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

/// Scroll / write Input Path 1 : valeur wire `@input` (1 / 4 / 6 Stomp) apprise depuis IN USB.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Path1InputSourceChangedPayload {
    pub wire_value: u8,
    /// `true` si trame `21` scroll hardware (vs echo post-write).
    pub from_scroll_21: bool,
}

/// Type Split Path 1 apprise depuis IN USB (select `82:62:0a:1a:…:05` ou scroll ed03 / `21`).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Path1SplitTypeChangedPayload {
    pub wire_value: u8,
    /// `true` si trame `21` scroll hardware (vs echo post-write ou ed03 seul).
    pub from_scroll_21: bool,
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

/// Clé cache écho ED03 : `(slot_bus, PP bloc modèle, param_selector wire)`.
pub fn echo_model_cache_key(slot_bus: u8, route_pp: u8, param_selector: u8) -> (u8, u8, u8) {
    (slot_bus, route_pp, param_selector)
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
    /// Finalise `RequestPresetNames` (watchdog / secours) puis enchaîne `RequestPresetName` si possible.
    FinalizePresetNames,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // réservé debug / front ; le fond scroll ne gate plus sur cette phase
pub enum SessionPhase {
    Bootstrapping,
    EditorReady,
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

    /// Lane ACK scroll firmware (`1d` / `1f` → OUT `f0:03` sub=`08`, octets 12–13 LE).
    /// Distinct de [`Self::preset_dump_ack_ctr`] et [`Self::editor_ed03_double`].
    pub firmware_scroll_ack_ctr: u16,
    pub firmware_scroll_ack_prev: Option<u8>,
    /// Après `1f` « None », le premier `1d` suivant peut réémettre le même double (step 0 une fois).
    pub firmware_scroll_skip_inc_once: bool,
    /// `true` après OUT bootstrap `09:10` (`note_firmware_scroll_bootstrap_sent`) — autorise l’ACK fond.
    pub firmware_scroll_armed: bool,

    /// Compteur dédié à la lane éditeur ed:03 36 bytes (bytes 28-29, cd:03).
    /// Valeur initiale : 0x64e8 (observée HX Edit cold boot, Start_Model_change.json).
    /// S'incrémente de +1 à chaque OUT 36 bytes cd:03 (phase 4, pull 1b/19).
    /// NE PAS mélanger avec preset_dump_ack_ctr ni live_write_ctr.
    ///
    /// IMPORTANT (capture select_presets.json, HX Edit) : le hi (octet 29) reste
    /// FIGÉ à `0x64`. Au passage `0x64ff`, HX bascule en `0x64xx` (jamais `0x65xx`)
    /// et bumpe `cd` (octet 27). HXLinux bumpe déjà `ed03_cmd_type` par lecture, donc
    /// pinner le hi suffit — voir `next_editor_ed03_double`.
    pub editor_ed03_double: u16,

    /// Compteur "lane ED03" (octets 12-13 des OUT 80:10:ed:03), distinct du
    /// double cd:03 (octets 28-29 = `editor_ed03_double`).
    ///
    /// u16 little-endian = lo (octet 12) + hi (octet 13), deux sous-compteurs
    /// logés dans un même mot, observés byte-pour-byte sur 4 captures HX
    /// (dont select_presets.json) :
    ///   - lo (octet 12) : position de transaction. += 0x17 par commande
    ///     19/1b/1c de PHASE B ; FIGÉ pendant les rafales d'ACK chunks (confirmé :
    ///     rafale de 12 chunks, lo constant, hi qui monte).
    ///   - hi (octet 13) : compteur de chunks GLOBAL. += 1 (=+0x0100) par ACK
    ///     chunk `08 sub=08` émis ; jamais resetté par lecture.
    ///
    /// La paire est lue **octet 12 = MAJEUR** : l'incrément +1 porte sur l'octet 13,
    /// et au débordement de l'octet 13 il faut RETENIR sur l'octet 12 (voir
    /// `advance_editor_ed03_lane_hi`), sinon le u16 recule (`ffdc→00dc` = `dc:00`
    /// sur le fil) et le device gèle le dump.
    ///
    /// Valeur initiale 0x1009 (lo=09, hi=10) — point d'ancrage FIXE observé
    /// au 1er `19 e8` du bootstrap, identique sur tous les presets/slots.
    /// Distinct de `preset_dump_ack_ctr` (qui force lo=f4, vision tronquée
    /// réservée aux ACK hors RequestPreset) et de `editor_ed03_double`.
    pub editor_ed03_lane: u16,

    /// Octet 14 du OUT `80:10:ed:03` (Phase-2 `19` sub=0c ET ACK chunk `08` sub=08) =
    /// POIDS FORT du compteur de chunks. La paire (octet 13, octet 14) est un compteur
    /// 16 bits little-endian (octet 13 = lo, octet 14 = hi) ; l'octet 12 (= lo de
    /// `editor_ed03_lane`, position de transaction) est INDÉPENDANT et n'entre PAS dans
    /// cette retenue.
    ///
    /// BUG C (vrai diagnostic, capture HX `out_only.txt` 16-06-2026) : au débordement de
    /// l'octet 13 (`ff → 00`), HX incrémente l'octet 14 (`00→01→02`), octet 12 inchangé.
    /// Trace observée : `95 ff 00` → `95 00 01` (et `fd ff 01` → `fd 00 02`). L'ancien
    /// code figeait l'octet 14 à `0x00` → au franchissement de `0xff` le device voyait un
    /// compteur désynchronisé et avortait le dump. Les pistes « retenue sur octet 12 »
    /// (HX_LANE_HI_CARRY) et « skip 0x00 » sont FAUSSES (réfutées par la même capture).
    /// Init `0x00`. Avancé par `advance_editor_ed03_lane_hi3`. Témoin : `HX_LANE_B14_CARRY=0`.
    pub editor_ed03_lane_b14: u8,

    // Compteurs keep-alive
    pub x1_cnt:  u8,
    pub x2_cnt:  u8,
    pub x80_cnt: u8,

    /// Dernier compteur (octet 9) reçu du device par lane keep-alive (IN 16o `sub=10`).
    /// Pour le désabonnement gracieux : le close doit porter `device_last + 1` (= requête,
    /// pas ACK). Alimentés par `ingest_hw_slot_notify_in`.
    pub dev_keepalive_cnt_ed: Option<u8>,
    pub dev_keepalive_cnt_ef: Option<u8>,
    pub dev_keepalive_cnt_f0: Option<u8>,

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
    /// Bloc modèle `83:66:cd` mémorisé par `(slot_bus, route_pp, param_selector)` depuis les échos IN.
    pub ed03_echo_model_by_slot_param: HashMap<(u8, u8, u8), [u8; 16]>,
    /// Octets cab après `c2:19` dans le bulk assign (hint 1 o ou `cd02xx`) — Cab single legacy live write.
    pub standalone_legacy_cab_module_field_by_slot: HashMap<u32, Vec<u8>>,
    /// Bloc modèle `83:66:cd:…` du bulk assign (préserve `cd:03:ff` vs `cd:04:00`).
    pub standalone_legacy_assign_model_block_by_slot: HashMap<u32, [u8; 16]>,
    /// Handshake param write Cab single legacy `cd:03:ff` en cours (`legacy_cab_param_commit.rs`).
    pub standalone_legacy_param_commit: Option<legacy_cab_param_commit::StandaloneLegacyParamCommit>,
    /// Single legacy en cours d'écriture : force c2 sur le discret (capture Soup Pro).
    /// Le single modern garde c3 (replay statique). Posé par write_live_param, consommé
    /// par build_live_write_frames_from_state.
    pub force_discrete_c2_for_legacy_single: bool,
    /// Hint cab dual legacy (`c3:19` + octets avant/après `1a`) par `(slot, cab_index)`.
    pub dual_legacy_cab_module_field_by_slot: HashMap<(u32, u8), Vec<u8>>,
    /// Dernier octet de séquence (`… cd 05 XX …`) envoyé en OUT live tant qu’on n’a pas reçu d’écho IN.
    pub ed03_live_write_seq_sent: Option<u8>,
    /// Slot Kempline pour lequel le focus cab `1b` a déjà été envoyé cette session USB.
    pub amp_cab_cab_focus_sent_for_slot: Option<u32>,
    /// Slot Kempline pour lequel le focus Cab 2 dual (`1d` + `cd:04` + `1a:01`) a été envoyé.
    pub cab_dual_cab2_focus_sent_for_slot: Option<u32>,
    /// Dernier focus onglet Cab dual pour write live param (`(slot, cab_index)`).
    pub cab_dual_live_write_tab_focus: Option<(u32, u8)>,
    /// Dernier OUT focus Cab 2 (`1d`) — sert au ctr `ed:08` post-IN 36o.
    pub last_cab_dual_cab2_focus_packet: Option<Vec<u8>>,
    /// Ctr `ed:08` calculé après IN `19`/36o (lane `cd:04` Stomp).
    pub cab_dual_cab2_handshake_ed08_ctr: Option<u16>,
    /// Dernier ctr appris depuis un IN `19`/36o Cab 2 (`cd:04`) pendant focus/handshake.
    pub cab_dual_cab2_last_in36_ed08_ctr: Option<u16>,
    /// Dernière trame IN `19`/36o Cab 2 (`cd:04`) — réponse au focus onglet Cab 2.
    pub cab_dual_cab2_last_in36_frame: Option<Vec<u8>>,
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
    /// **Source unique** du slot actif côté host (cadre orange UI).
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
    /// Dernière source Input Path 1 lue sur le fil (`82:62:00:33:XX`, trames IN).
    pub path1_input_source_wire: Option<u8>,
    /// Dernier type Split Path 1 lu sur le fil (`82:62:0a:1a:…:05`).
    pub path1_split_type_wire: Option<u8>,

    /// Fenêtre de capture des IN `0x81` après un OUT « focus slot » (`sync_hardware_slot_focus_usb`).
    /// Remplie par `usb_listener` tant que `Instant::now() < deadline` (courte, ~55 ms ; max ~40 trames).
    pub usb_slot_focus_capture_deadline: Option<Instant>,
    pub usb_slot_focus_capture: Vec<Vec<u8>>,
    /// Capture IN pendant replace Cab dual (#14–#28 `cab dual change right.json`).
    pub cab_dual_cab2_handshake_until: Option<Instant>,
    pub cab_dual_cab2_handshake_capture: Vec<Vec<u8>>,
    /// Après focus Cab 2 (`1d` + `1a:01`) : bloque l’ACK `ed:08` auto du mode standard
    /// (sinon double ACK → pas de `IN 21` avant le bulk replace).
    pub cab_dual_cab2_suppress_standard_ed08_until: Option<Instant>,
    /// Dernier paquet IN « focus slot » parsé par index Kempline (rempli par `sync_hardware_slot_focus_usb`).
    pub last_slot_focus_capsule: [Option<slot_focus_in::SlotFocusInCapsule>; 16],
    /// Empreinte précédente pour surveillance contenu slot (modèle / vide / params).
    pub slot_watch_prev: [slot_watch::SlotWatchSnapshot; 16],
    pub hw_slot_content_sequence: u32,

    // ── Scroll modèle hardware (pull 1b/19) ──────────────────────────────
    /// Double éditeur dédié pull scroll (séparé de `editor_ed03_double`).
    /// Sentinelle `0xFFFF` = pas encore snapé depuis `editor_ed03_double_val()`.
    pub hw_model_pull_ed03_double: u16,
    pub hw_model_pull_ctr: u16,
    /// 0=idle, 1=attend 1ère rép, 2=attend 2ème, 3=attend bulk 272.
    pub hw_model_pull_step: u8,
    pub hw_model_pull_slot_bus: Option<u8>,
    pub hw_model_pull_capture: Vec<Vec<u8>>,
    pub hw_model_pull_capture_deadline: Option<Instant>,
    /// `None` = cd:03, `Some(0x04)` après wrap bas du double.
    pub hw_model_pull_cd_lane: Option<u8>,
    pub hw_model_pull_echo_double: Option<[u8; 2]>,
    pub hw_model_pull_retried_1b: bool,
    pub hw_model_pull_saw_final_bulk: bool,
    pub hw_model_pull_pending_slot_bus: Option<u8>,
    pub hw_model_pull_last_at: Option<Instant>,
    pub hw_model_pull_quiet_until: Option<Instant>,
    pub hw_model_post_pull_settling: bool,
    pub hw_model_post_pull_deadline: Option<Instant>,
    pub hw_model_last_scroll_in_at: Option<Instant>,
    /// Dernier `IN 1d` scroll en attente d’ACK (paire 1d→1f avant pull).
    pub hw_model_scroll_deferred_1d: Option<Vec<u8>>,
    /// Cab 2 lu dans le dernier pull scroll (bloc `c219`), par slot Kempline 0..15.
    pub last_hw_cab_dual_cab2_hex: [Option<String>; 16],

    /// Déduplication des événements paramètre IN (`85:62…1c:PP:77`).
    pub slot_param_emit: slot_param_in::SlotParamEmitState,
    /// Fenêtre post bootstrap phase 4 (~700 ms HX Edit) : le host n’envoie **aucune** requête
    /// proactive (noms, dump, keep-alive poll) — seulement des ACK sur le trafic IN.
    pub init_usb_settle_until: Option<Instant>,
    /// Legacy : utilisé par d’autres chemins ; le fond scroll (`firmware_scroll_ack`) ignore ce flag.
    pub suppress_1d_firmware_notify_ack: bool,
    /// `false` jusqu’à fin amorçage (`amorcage::spawn_post_arm_sequence`) — gate épisodes host proactifs.
    pub editor_ready: bool,
    /// Empêche de lancer deux fois la séquence post-ARM
    /// si `ModeRequest::AwaitPostBootstrapSettle` est demandé plusieurs fois.
    pub post_arm_sequence_started: bool,
    /// Gate post-ARM_ef : bitmap des ACK reçus après ARM_ef (bit0=ef, bit1=ed, bit2=f0).
    pub post_ef_arm_ack_mask: u8,
    /// `true` après ARM_ef tant que la gate n'est pas complète.
    pub post_ef_arm_gate_active: bool,
    /// Receiver gate — créé dans `arm_post_ef_gate`, consommé dans `lib.rs`.
    pub post_ef_gate_rx: Option<std::sync::mpsc::Receiver<()>>,
    /// Sender gate — signal phase 4 quand les 3 ACK sont reçus.
    pub post_ef_gate_tx: Option<std::sync::mpsc::SyncSender<()>>,
    /// `true` entre envoi des OUT phase 4 et réception du trailer IN `7a` 132 o.
    pub phase4_bootstrap_active: bool,
    pub phase4_complete_rx: Option<std::sync::mpsc::Receiver<()>>,
    pub phase4_complete_tx: Option<std::sync::mpsc::SyncSender<()>>,
    /// Machine à états phase 4 (passive en étape 2).
    pub phase4_step: phase4_state::Phase4Step,
    /// `true` si le `IN 19/36o ef` post-`1a` a été vu avant l'entrée en `PostArm`.
    pub phase4_seen_19ef_pre_postarm: bool,
    /// Timeout global dialogue post-1a (armé à l'entrée de `PostArm`).
    /// Si expiré, la FSM passe en `Done` pour ne pas bloquer l'amorçage.
    pub phase4_post1a_timeout: Option<Instant>,
    /// Chunks dump **pleins** (272 o) vus en `WaitingDump` — fin sans trailer partiel
    /// (presets Amp+Cab slot 0 : rafale de 272 o puis écho IN sub=`08` 16 o).
    pub phase4_dump_full_272_count: u16,
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
/// Trace effective après `EditorReady` (évite de ralentir phase 4 / settle).
static USB_PACKET_TRACE_LIVE: AtomicBool = AtomicBool::new(false);
static USB_PACKET_TRACE_DEFER_UNTIL_READY: AtomicBool = AtomicBool::new(true);
/// 0 = pas de limite ; sinon ne loggue pas les paquets plus longs (évite le flood 272o en delta_only=0).
static USB_PACKET_TRACE_MAX_LEN: AtomicU32 = AtomicU32::new(0);
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

pub fn set_usb_packet_trace_defer_until_ready(defer: bool) {
    USB_PACKET_TRACE_DEFER_UNTIL_READY.store(defer, Ordering::Relaxed);
    if !defer {
        USB_PACKET_TRACE_LIVE.store(true, Ordering::Relaxed);
    }
}

pub fn activate_usb_packet_trace_live() {
    if !usb_packet_trace_enabled() {
        return;
    }
    if USB_PACKET_TRACE_LIVE.swap(true, Ordering::Relaxed) {
        return;
    }
    eprintln!("[UsbTrace] live — tracing actif (post EditorReady)");
}

/// Trace USB effective (respecte le report jusqu'à `EditorReady` sauf `USB_PACKET_TRACE_BOOT=1`).
pub fn usb_packet_trace_active() -> bool {
    usb_packet_trace_enabled()
        && (USB_PACKET_TRACE_LIVE.load(Ordering::Relaxed)
            || !USB_PACKET_TRACE_DEFER_UNTIL_READY.load(Ordering::Relaxed))
}

pub fn set_usb_packet_trace_max_len(max_len: u32) {
    USB_PACKET_TRACE_MAX_LEN.store(max_len, Ordering::Relaxed);
}

pub fn usb_packet_trace_max_len() -> Option<usize> {
    match USB_PACKET_TRACE_MAX_LEN.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n as usize),
    }
}

/// Filtre longueur pour `USB_PACKET_TRACE` — n'affecte que le log, pas l'envoi/réception.
pub fn usb_packet_trace_should_log(data: &[u8]) -> bool {
    usb_packet_trace_max_len()
        .is_none_or(|max| data.len() <= max)
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

/// Helper flag « ON sauf si explicitement désactivé » (témoin = `0`/`false`/`no`).
fn env_flag_on_by_default(var: &str) -> bool {
    match std::env::var(var).as_deref() {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Bug A — `HX_EDITOR_DOUBLE_PIN_HI` (défaut ON) : pinne le hi du double éditeur à `0x64`.
/// Confirmé capture HX Edit `select_presets.json` : `0x64ff → 0x64xx` (jamais `0x65xx`).
/// `=0` → témoin : ancien comportement (u16 plein, le hi roule à `0x65`, stub `1c`).
fn editor_double_pin_hi() -> bool {
    env_flag_on_by_default("HX_EDITOR_DOUBLE_PIN_HI")
}

/// Bug A (raffinement) — `HX_EDITOR_DOUBLE_SKIP_00` (défaut ON) : au franchissement du
/// lo du double éditeur, HX SAUTE `0x00` (`0x64ff → 0x6401`, jamais `0x6400`). Confirmé
/// sur `out_only.txt` (HX Edit). `lo=0x00` est une valeur que HX n'émet jamais.
/// `=0` → témoin : ancien `0x6400`. N.B. : fidélité HX ; n'adresse PAS le décrochage §5.
fn editor_double_skip_00() -> bool {
    env_flag_on_by_default("HX_EDITOR_DOUBLE_SKIP_00")
}

/// Bug C — `HX_LANE_HI_CARRY` (DÉPRÉCIÉ, plus appelé). Hypothèse réfutée par la capture
/// `out_only.txt` (16-06) : au wrap de l'octet 13, la retenue NE va PAS sur l'octet 12
/// (qui reste figé). Conservé pour mémoire ; remplacé par `lane_b14_carry`.
#[allow(dead_code)]
fn editor_lane_hi_carry() -> bool {
    env_flag_on_by_default("HX_LANE_HI_CARRY")
}

/// Bug C (vrai fix) — `HX_LANE_B14_CARRY` (défaut ON) : le compteur de chunks est 16 bits
/// little-endian sur (octet 13 = lo, octet 14 = hi). Au débordement de l'octet 13
/// (`ff → 00`), la retenue va dans l'**octet 14** (l'octet 12 = position de transaction
/// reste figé). Confirmé byte-pour-byte sur `out_only.txt` (HX Edit) : `95 ff 00 → 95 00 01`.
/// Témoin `=0` : octet 14 figé à `0x00` + octet 13 qui wrappe `ff→00` (comportement
/// d'origine, échoue au franchissement de `0xff` — revert instantané).
fn lane_b14_carry() -> bool {
    env_flag_on_by_default("HX_LANE_B14_CARRY")
}

impl HelixState {
    pub fn new() -> Self {
        Self {
            x1_cnt:             0x02,
            x2_cnt:             0x02,
            x80_cnt:            0x02,
            dev_keepalive_cnt_ed: None,
            dev_keepalive_cnt_ef: None,
            dev_keepalive_cnt_f0: None,
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
            firmware_scroll_ack_ctr: firmware_scroll_ack::SCROLL_LANE_BOOT,
            firmware_scroll_ack_prev: None,
            firmware_scroll_skip_inc_once: false,
            firmware_scroll_armed: false,
            editor_ed03_double: Self::preset_ed03_transaction_counter_before_first(),
            editor_ed03_lane:   Self::EDITOR_ED03_LANE_FIRST,
            editor_ed03_lane_b14: 0x00,
            preset_last_ack_double: [0, 0],
            request_preset_session_id: 0xf4,
            ed03_cmd_type:      0x01,
            session_quadruple: [0xf4, 0x1e, 0x00, 0x00],
            last_ed03_echo_model: None,
            ed03_echo_model_by_slot_param: HashMap::new(),
            standalone_legacy_cab_module_field_by_slot: HashMap::new(),
            standalone_legacy_assign_model_block_by_slot: HashMap::new(),
            standalone_legacy_param_commit: None,
            force_discrete_c2_for_legacy_single: false,
            dual_legacy_cab_module_field_by_slot: HashMap::new(),
            ed03_live_write_seq_sent: None,
            amp_cab_cab_focus_sent_for_slot: None,
            cab_dual_cab2_focus_sent_for_slot: None,
            cab_dual_live_write_tab_focus: None,
            last_cab_dual_cab2_focus_packet: None,
            cab_dual_cab2_handshake_ed08_ctr: None,
            cab_dual_cab2_last_in36_ed08_ctr: None,
            cab_dual_cab2_last_in36_frame: None,
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
            path1_input_source_wire: None,
            path1_split_type_wire: None,
            usb_slot_focus_capture_deadline: None,
            usb_slot_focus_capture: Vec::new(),
            cab_dual_cab2_handshake_until: None,
            cab_dual_cab2_handshake_capture: Vec::new(),
            cab_dual_cab2_suppress_standard_ed08_until: None,
            last_slot_focus_capsule: std::array::from_fn(|_| None),
            slot_watch_prev: std::array::from_fn(|_| slot_watch::SlotWatchSnapshot::default()),
            hw_slot_content_sequence: 0,
            hw_model_pull_ed03_double: 0xFFFF,
            hw_model_pull_ctr: 0x6cbd, // base ctr pull scroll couplé (d6eb2b1)
            hw_model_pull_step: 0,
            hw_model_pull_slot_bus: None,
            hw_model_pull_capture: Vec::new(),
            hw_model_pull_capture_deadline: None,
            hw_model_pull_cd_lane: None,
            hw_model_pull_echo_double: None,
            hw_model_pull_retried_1b: false,
            hw_model_pull_saw_final_bulk: false,
            hw_model_pull_pending_slot_bus: None,
            hw_model_pull_last_at: None,
            hw_model_pull_quiet_until: None,
            hw_model_post_pull_settling: false,
            hw_model_post_pull_deadline: None,
            hw_model_last_scroll_in_at: None,
            hw_model_scroll_deferred_1d: None,
            last_hw_cab_dual_cab2_hex: std::array::from_fn(|_| None),
            slot_param_emit: slot_param_in::SlotParamEmitState::default(),
            init_usb_settle_until: None,
            suppress_1d_firmware_notify_ack: false,
            editor_ready: false,
            post_arm_sequence_started: false,
            post_ef_arm_ack_mask: 0,
            post_ef_arm_gate_active: false,
            post_ef_gate_rx: None,
            post_ef_gate_tx: None,
            phase4_bootstrap_active: false,
            phase4_complete_rx: None,
            phase4_complete_tx: None,
            phase4_step: phase4_state::Phase4Step::Idle,
            phase4_seen_19ef_pre_postarm: false,
            phase4_post1a_timeout: None,
            phase4_dump_full_272_count: 0,
        }
    }

    /// ACK immédiat des IN `1d` (ne pas figer le Stomp) sauf pendant la lecture preset USB.
    pub fn should_ack_firmware_1d_notify(&self) -> bool {
        if self.init_usb_settle_active() {
            return true;
        }
        !self.suppress_1d_firmware_notify_ack
    }

    /// Phase de session protocolaire : amorçage unique, puis runtime éditeur prêt.
    #[allow(dead_code)]
    pub fn session_phase(&self) -> SessionPhase {
        if self.connecting || !self.editor_ready {
            SessionPhase::Bootstrapping
        } else {
            SessionPhase::EditorReady
        }
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

    fn editor_ready_blocks_mode_request(req: &ModeRequest) -> bool {
        matches!(
            req,
            ModeRequest::RequestPresetName | ModeRequest::RequestPresetNames
        )
    }

    /// Pipeline couches actives (pull scroll, fond scroll, ACK dump 272) — voir `usb_in_pipeline`.
    pub fn run_usb_in_active_layers(&mut self, data: &[u8]) -> usb_in_pipeline::ActivePipelineOutcome {
        usb_in_pipeline::run_active_layers(self, data)
    }

    /// Remet à zéro l’état pull modèle HW (déconnexion USB).
    pub fn reset_hw_model_pull_state(&mut self) {
        self.hw_model_pull_ed03_double = 0xFFFF;
        self.hw_model_pull_ctr = 0x0000;
        self.hw_model_pull_step = 0;
        self.hw_model_pull_slot_bus = None;
        self.hw_model_pull_capture.clear();
        self.hw_model_pull_capture_deadline = None;
        self.hw_model_pull_cd_lane = None;
        self.hw_model_pull_echo_double = None;
        self.hw_model_pull_retried_1b = false;
        self.hw_model_pull_saw_final_bulk = false;
        self.hw_model_pull_pending_slot_bus = None;
        self.hw_model_pull_last_at = None;
        self.hw_model_pull_quiet_until = None;
        self.hw_model_post_pull_settling = false;
        self.hw_model_post_pull_deadline = None;
        self.hw_model_last_scroll_in_at = None;
        self.hw_model_scroll_deferred_1d = None;
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
        amp_cab_live_write::cache_ed03_model_blocks_from_echo(
            &mut self.ed03_echo_model_by_slot_param,
            data,
            b,
        );
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

    /// Source Input Path 1 depuis trames IN (`82:62:00:33:XX`).
    /// Retourne la nouvelle valeur si elle a changé (scroll hardware ou echo post-write).
    pub fn ingest_path1_input_source_wire_in(&mut self, data: &[u8]) -> Option<u8> {
        let wire = crate::helix::path1_io_live_write::scan_path1_input_source_wire(data)?;
        if self.path1_input_source_wire == Some(wire) {
            return None;
        }
        self.path1_input_source_wire = Some(wire);
        eprintln!(
            "[Path1Input][wire-in] @input={wire} scroll_21={}",
            crate::helix::path1_io_live_write::is_path1_input_scroll_notify_21(data)
        );
        Some(wire)
    }

    pub fn path1_input_source_changed_payload(
        &self,
        wire: u8,
        data: &[u8],
    ) -> Path1InputSourceChangedPayload {
        Path1InputSourceChangedPayload {
            wire_value: wire,
            from_scroll_21: crate::helix::path1_io_live_write::is_path1_input_scroll_notify_21(data),
        }
    }

    pub fn ingest_path1_split_type_wire_in(&mut self, data: &[u8]) -> Option<u8> {
        if self.preset_content_only {
            return None;
        }
        let wire = crate::helix::path1_split_live_write::scan_path1_split_type_wire(data)?;
        let from_scroll_21 =
            crate::helix::path1_split_live_write::is_path1_split_scroll_notify_21(data);
        let from_ed03 = crate::helix::path1_split_live_write::scan_path1_split_type_wire_ed03_scroll(
            data,
        )
        .is_some();
        if self.path1_split_type_wire == Some(wire) && !from_scroll_21 && !from_ed03 {
            return None;
        }
        self.path1_split_type_wire = Some(wire);
        eprintln!(
            "[Path1Split][wire-in] type={wire} scroll_21={from_scroll_21} ed03={from_ed03}"
        );
        Some(wire)
    }

    pub fn path1_split_type_changed_payload(
        &self,
        wire: u8,
        data: &[u8],
    ) -> Path1SplitTypeChangedPayload {
        Path1SplitTypeChangedPayload {
            wire_value: wire,
            from_scroll_21: crate::helix::path1_split_live_write::is_path1_split_scroll_notify_21(
                data,
            ),
        }
    }

    /// Met à jour [`Self::hw_active_slot_*`] et notifie l’UI — **seul** chemin pour le slot actif.
    ///
    /// Scan linéaire de `82:62:SS:1a` sur **toute** trame IN (16 o keep-alive, 40 o switch UI,
    /// 44 o après preset, etc.). Ne pas ajouter de filtre par head `21` / `f0:03:02:10` : ce serait
    /// le même traitement dupliqué. Les paires courtes `ed:03:80:10` + `ef:03:01:10` (16 o) sont
    /// mémorisées en plus pour le debug protocole.
    pub fn ingest_hw_slot_notify_in(&mut self, data: &[u8]) -> Option<HardwareSlotChangedPayload> {
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

        // Mémorise le dernier compteur device (octet 9) par lane keep-alive — pour le
        // désabonnement gracieux (close = device_last + 1 = requête, pas ACK).
        match (data[4], data[5], data[6], data[7]) {
            (0xed, 0x03, 0x80, 0x10) => self.dev_keepalive_cnt_ed = Some(data[9]),
            (0xef, 0x03, 0x01, 0x10) => self.dev_keepalive_cnt_ef = Some(data[9]),
            (0xf0, 0x03, 0x02, 0x10) => self.dev_keepalive_cnt_f0 = Some(data[9]),
            _ => {}
        }

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

    /// `true` (défaut) : ACK chunks 272 sur [`Self::editor_ed03_lane`] (`9d:11`, …).
    /// `HX_DUMP_ACK_LANE=f4` : témoin legacy (octet 12 figé `0xf4`).
    pub fn preset_dump_ack_use_editor_lane() -> bool {
        match std::env::var("HX_DUMP_ACK_LANE").as_deref() {
            Ok(v) if matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "f4" | "legacy" | "0" | "false" | "no"
            ) =>
            {
                false
            }
            Ok(_) => true,
            Err(_) => true,
        }
    }

    /// Octets 12–13–14 du OUT `ed:03` sub=`08` (ACK d'un chunk 272 du flux preset).
    /// Renvoie 3 octets : `[o12 = position transaction, o13 = compteur chunks lo,
    /// o14 = compteur chunks hi]`. Voir `advance_editor_ed03_lane_hi3` / champ
    /// `editor_ed03_lane_b14`.
    pub fn next_preset_stream_chunk_ack_lane(&mut self) -> [u8; 3] {
        if Self::preset_dump_ack_use_editor_lane() {
            return self.advance_editor_ed03_lane_hi3();
        }
        // Témoin f4 : continuer à faire monter editor_ed03_lane en phase 4 (PHASE B).
        if self.phase4_bootstrap_active {
            let _ = self.advance_editor_ed03_lane_hi3();
        }
        let f4 = self.next_preset_dump_ack_f4_lane();
        [f4[0], f4[1], 0x00]
    }

    /// Lane témoin `f4:1d` → `f4:1e` (expérience HW — voir `preset_dump_stream_ack`).
    pub fn preset_dump_ack_double(&self) -> [u8; 2] {
        let lo = (self.preset_dump_ack_ctr & 0xFF) as u8;
        let hi = ((self.preset_dump_ack_ctr >> 8) & 0xFF) as u8;
        [lo, hi]
    }

    fn next_preset_dump_ack_f4_lane(&mut self) -> [u8; 2] {
        let out = self.preset_dump_ack_double();
        self.preset_dump_ack_ctr = (self.preset_dump_ack_ctr.wrapping_add(0x0100) & 0xff00)
            | (self.request_preset_session_id as u16);
        out
    }

    /// ACK lane `f4:xx` — réservé aux réponses `f0:03` (preset switch), pas aux chunks 272.
    pub fn next_preset_dump_ack_double(&mut self) -> [u8; 2] {
        self.next_preset_dump_ack_f4_lane()
    }

    /// Fenêtre active de replace Cab dual #2 (évite les ACK `ed:08` automatiques du mode standard).
    pub fn cab_dual_cab2_handshake_active(&self) -> bool {
        self.cab_dual_cab2_handshake_until
            .as_ref()
            .is_some_and(|d| Instant::now() < *d)
    }

    /// Ne pas laisser `standard` acquitter un IN `19`/36o post-focus Cab 2.
    pub fn cab_dual_cab2_block_standard_auto_ack(&self) -> bool {
        self.cab_dual_cab2_handshake_active()
            || self
                .cab_dual_cab2_suppress_standard_ed08_until
                .as_ref()
                .is_some_and(|d| Instant::now() < *d)
    }

    /// Double pour la lane éditeur ed:03 36 bytes (phase 4, pull 1b/19).
    /// Valeur initiale : 0x64e8. Ne pas utiliser pour les ACK chunks.
    pub fn editor_ed03_double_val(&self) -> [u8; 2] {
        let lo = (self.editor_ed03_double & 0xFF) as u8;
        let hi = ((self.editor_ed03_double >> 8) & 0xFF) as u8;
        [lo, hi]
    }

    /// Avance le double éditeur d'un cran.
    ///
    /// BUG A (corrigé) — confirmé sur `select_presets.json` (HX Edit) : le hi (octet 29
    /// sur le fil) reste **figé à `0x64`**. À `0x64ff`, HX repasse en page `0x64`
    /// (`…→0x6401`, jamais `0x6500`) et bumpe `cd` (octet 27). Or HXLinux incrémente
    /// déjà `ed03_cmd_type` à chaque lecture (shutdown) — la « page neuve » qui rend
    /// le reset du lo acceptable est donc déjà fournie. Il suffit de pinner le hi.
    ///
    /// Ancien comportement (témoin `HX_EDITOR_DOUBLE_PIN_HI=0`) : `wrapping_add(1)` sur
    /// le u16 plein → `0x64ff + 1 = 0x6500` (hi roule à `0x65`) → le device renvoie un
    /// stub `1c` (pas de dump) → `bytes=0` → ralentissement (récupéré au retry car le
    /// shutdown reset le double à `0x64e7`).
    pub fn next_editor_ed03_double(&mut self) -> [u8; 2] {
        if editor_double_pin_hi() {
            let mut lo = (self.editor_ed03_double & 0xff).wrapping_add(1) & 0xff;
            // HX saute 0x00 au franchissement : 0x64ff -> 0x6401 (jamais 0x6400).
            // Observé sur out_only.txt. Aligne HXLinux exactement sur HX.
            if editor_double_skip_00() && lo == 0x00 {
                lo = 0x01;
            }
            self.editor_ed03_double = 0x6400 | lo; // hi figé 0x64 (page éditeur)
        } else {
            self.editor_ed03_double = self.editor_ed03_double.wrapping_add(1);
        }
        self.editor_ed03_double_val()
    }

    // ── Compteur lane ED03 (octets 12-13 des OUT 80:10:ed:03) ────────────────
    //
    // Décodé byte-pour-byte sur 4 captures HX (dont select_presets.json) :
    // lo (octet 12) = position transaction (+0x17/commande, figé pendant ACK),
    // hi (octet 13) = compteur chunks GLOBAL (+1/ACK chunk). Voir `editor_ed03_lane`.

    /// Valeur d'ancrage du compteur lane ED03 au 1er OUT bootstrap (`19 e8`).
    /// Observée fixe (`09:10`) sur toutes les captures HX.
    pub const EDITOR_ED03_LANE_FIRST: u16 = 0x1009;

    /// Pas du lo (octet 12) pour une commande PHASE B (`19`/`1b`/`1c` courte).
    pub const EDITOR_ED03_LANE_CMD_DELTA: u16 = 0x0017;

    /// Octets 12-13 courants (lo, hi) pour insertion dans un paquet.
    pub fn editor_ed03_lane_bytes(&self) -> [u8; 2] {
        [
            (self.editor_ed03_lane & 0xff) as u8,
            ((self.editor_ed03_lane >> 8) & 0xff) as u8,
        ]
    }

    /// Avance le lo (octet 12) de `delta` (hi inchangé), et renvoie les octets
    /// **avant** avance : le paquet courant porte la valeur courante, puis on
    /// incrémente pour le suivant (sémantique observée sur le fil HX).
    pub fn advance_editor_ed03_lane_lo(&mut self, delta: u16) -> [u8; 2] {
        let out = self.editor_ed03_lane_bytes();
        let lo = (self.editor_ed03_lane & 0xff).wrapping_add(delta) & 0xff;
        self.editor_ed03_lane = (self.editor_ed03_lane & 0xff00) | lo;
        out
    }

    /// Avance le compteur de chunks de +1 et renvoie `[o12, o13, o14]` **avant** avance
    /// (sémantique émettre-puis-avancer, observée sur le fil HX).
    ///
    /// BUG C (vrai fix, capture `out_only.txt` 16-06) — le compteur de chunks est 16 bits
    /// little-endian sur **(octet 13 = lo, octet 14 = hi)**. L'octet 12 (= lo de
    /// `editor_ed03_lane`, position de transaction) est INDÉPENDANT et reste figé pendant
    /// le dump. L'incrément +1 porte sur l'octet 13 ; au débordement `0xff → 0x00`, la
    /// retenue va sur l'**octet 14** (`editor_ed03_lane_b14`), JAMAIS sur l'octet 12.
    ///
    /// Trace HX reproduite : `95 fe 00 → 95 ff 00 → 95 00 01 → 95 01 01`. Les deux
    /// anciennes pistes (retenue octet 12 = `HX_LANE_HI_CARRY` ; « skip 0x00 ») sont
    /// réfutées par cette même capture. Témoin : `HX_LANE_B14_CARRY=0` (octet 14 figé
    /// `0x00`, octet 13 wrappe `ff→00` → comportement d'origine).
    pub fn advance_editor_ed03_lane_hi3(&mut self) -> [u8; 3] {
        let b12 = (self.editor_ed03_lane & 0x00ff) as u8;
        let b13 = ((self.editor_ed03_lane >> 8) & 0xff) as u8;
        let b14 = self.editor_ed03_lane_b14;
        let out = [b12, b13, b14];
        // Avance l'octet 13 de +1 ; octet 12 inchangé.
        let b13_next = b13.wrapping_add(1);
        self.editor_ed03_lane = (self.editor_ed03_lane & 0x00ff) | ((b13_next as u16) << 8);
        // Retenue sur l'octet 14 quand l'octet 13 vient de wrapper ff→00.
        if lane_b14_carry() && b13 == 0xff {
            self.editor_ed03_lane_b14 = self.editor_ed03_lane_b14.wrapping_add(1);
        }
        out
    }

    /// Shim 2 octets `[o12, o13]` pour les appelants hors RequestPreset
    /// (`phase4_state.rs`, pull scroll…) qui n'émettent pas l'octet 14. Avance le
    /// compteur à l'identique — l'octet 14 reste suivi en interne pour cohérence globale.
    pub fn advance_editor_ed03_lane_hi(&mut self) -> [u8; 2] {
        let [b12, b13, _b14] = self.advance_editor_ed03_lane_hi3();
        [b12, b13]
    }

    /// Repositionne le compteur lane au démarrage de la phase 4 (ancrage fixe).
    pub fn reset_editor_ed03_lane(&mut self) {
        self.editor_ed03_lane = Self::EDITOR_ED03_LANE_FIRST;
        self.editor_ed03_lane_b14 = 0x00;
    }

    /// Aligne `preset_index` sur le nom lu par `RequestPresetName` (octet [24] seul est
    /// souvent 0 — le nom est à [27..] et la liste est déjà dans `preset_names`).
    pub fn resolve_preset_index_from_active_name(&mut self) {
        const SLOT_COUNT: usize = 125;
        if !self.got_preset_names || self.preset_names.len() < SLOT_COUNT {
            return;
        }
        let Some(ref name) = self.active_preset_name else {
            return;
        };
        let Some(idx) = self.preset_names.iter().position(|n| n == name) else {
            return;
        };
        if self.preset_index != idx {
            eprintln!(
                "[PresetDebug] preset_index {} → {} (nom actif '{}')",
                self.preset_index, idx, name
            );
        }
        self.preset_index = idx;
        self.active_preset_name_index = Some(idx);
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
        if !self.editor_ready && Self::editor_ready_blocks_mode_request(&req) {
            init_trace::trace_fmt(format_args!(
                "switch_mode BLOCKED {:?} (editor not ready)",
                req
            ));
            return;
        }
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
        if !self.editor_ready && matches!(cmd, KeepAliveCommand::StartOrdered) {
            init_trace::trace("start_keepalive BLOCKED (not EditorReady)");
            return;
        }
        if let Some(tx) = &self.keepalive_tx {
            let _ = tx.send(cmd);
        }
    }

    /// Gate fin phase 4 — trailer IN `7a` 132 o (consommé par le thread amorcage).
    pub fn arm_phase4_complete_gate(&mut self) {
        use std::sync::mpsc::sync_channel;
        let (tx, rx) = sync_channel::<()>(1);
        self.phase4_bootstrap_active = false;
        self.phase4_complete_tx = Some(tx);
        self.phase4_complete_rx = Some(rx);
        init_trace::trace("gate phase4 armée — attend trailer IN 7a 132o");
    }

    /// OUT phase 4 (3×`19` + `1a`) — la fin est signalée par [`Self::note_phase4_bootstrap_complete`].
    pub fn start_phase4_bootstrap(&mut self) {
        self.phase4_bootstrap_active = true;
        self.phase4_dump_full_272_count = 0;
        crate::helix::editor_phase4_bootstrap::send(self);
    }

    /// Appelé sur le trailer `7a` 132 o (usb_listener / preset_dump).
    pub fn note_phase4_bootstrap_complete(&mut self) {
        if !self.phase4_bootstrap_active {
            return;
        }
        self.phase4_bootstrap_active = false;
        init_trace::trace("gate phase4 complète (trailer 7a 132o) — settle 700ms peut démarrer");
        if let Some(tx) = self.phase4_complete_tx.take() {
            let _ = tx.try_send(());
        }
    }

    /// Arme la gate post-ARM_ef (après `send_arm_ef` dans `reconfigure_x1`).
    pub fn arm_post_ef_gate(&mut self) {
        use std::sync::mpsc::sync_channel;
        let (tx, rx) = sync_channel::<()>(1);
        self.post_ef_arm_ack_mask = 0;
        self.post_ef_arm_gate_active = true;
        self.post_ef_gate_tx = Some(tx);
        self.post_ef_gate_rx = Some(rx);
        init_trace::trace("gate post-ARM_ef armée — attend 3× IN 08/16o (ef+ed+f0)");
    }

    /// Compte les ACK `IN 08/16o` sub=`08` (ef, ed, f0). Signale la gate si complète.
    pub fn tick_post_ef_arm_gate(&mut self, data: &[u8]) -> bool {
        if !self.post_ef_arm_gate_active {
            return false;
        }
        if data.len() != 16
            || !data.starts_with(&[0x08, 0x00, 0x00, 0x18])
            || data[11] != 0x08
        {
            return false;
        }
        let ep = &data[4..8];
        // IN device→host : `ef:03:01:10` / `ed:03:80:10` / `f0:03:02:10` (≠ ordre OUT host→device).
        let bit: u8 = if ep == &[0xef, 0x03, 0x01, 0x10] {
            0b001
        } else if ep == &[0xed, 0x03, 0x80, 0x10] {
            0b010
        } else if ep == &[0xf0, 0x03, 0x02, 0x10] {
            0b100
        } else {
            return false;
        };
        self.post_ef_arm_ack_mask |= bit;
        init_trace::trace_fmt(format_args!(
            "gate post-ARM_ef tick mask={:03b}",
            self.post_ef_arm_ack_mask
        ));
        if self.post_ef_arm_ack_mask == 0b111 {
            self.post_ef_arm_gate_active = false;
            init_trace::trace("gate post-ARM_ef complète (ef+ed+f0) — signal phase 4");
            if let Some(tx) = self.post_ef_gate_tx.take() {
                let _ = tx.try_send(());
            }
            return true;
        }
        false
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