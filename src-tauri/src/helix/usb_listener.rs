// ===========================================================
// helix/usb_listener.rs
// Thread de lecture en continu sur endpoint 0x81
// Dispatch chaque paquet vers le mode actif
// C'est le chef d'orchestre de toute la machine à états
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};
use rusb::DeviceHandle;
use rusb::GlobalContext;

use tauri::Emitter;

use crate::helix::{
    HelixState, Mode, models_debug_sync_trace_enabled, preset_debug_verbose_enabled,
    slot_param_debug_enabled, usb_io_diag_enabled, usb_packet_trace_active,
    usb_packet_trace_delta_only, usb_packet_trace_should_log, usb_trace_fingerprint,
};
use crate::helix::packet::{classify_in_packet, packet_counter, OutPacket};

const ENDPOINT_IN: u8 = 0x81;
/// Timeout modéré (pas infini) : réveille la boucle pour tester `stop` (arrêt propre) et permet
/// une annulation d'URB parfaitement inoffensive après un silence — le bulk USB n'est pas lossy,
/// le device bufferise et répond au `read_bulk` suivant. Découplé du poll depuis le 2026-07-10
/// (voir `LiveParamPollShared`) : ce timeout n'a plus aucun impact sur la cadence du poll, donc
/// peut être aussi long que souhaité sans retarder l'envoi du poll f0:03.
const READ_TIMEOUT_MS: u64 = 500;
const BUFFER_SIZE: usize = 512;
/// Intervalle de poll f0:03 pour recevoir les changements de paramètre knob HW (85:62).
const LIVE_PARAM_POLL_INTERVAL_MS: u64 = 40;
/// Log si acquisition ou section critique HelixState dépasse ce seuil (contention / travail long).
const STATE_LOCK_WARN_MS: u128 = 10;

fn warn_slow_lock(label: &str, wait_ms: u128, hold_ms: u128, in_len: usize) {
    if wait_ms > STATE_LOCK_WARN_MS || hold_ms > STATE_LOCK_WARN_MS {
        eprintln!(
            "[WARN] {label} wait={wait_ms}ms hold={hold_ms}ms (IN len={in_len})"
        );
    }
}

/// État partagé entre le thread POLL (cadence 40ms, indépendante) et le thread LECTURE (draine
/// 0x81 en continu) — voir découverte 2026-07-10 (mémoire session) : un `read_bulk` bloquant
/// jusqu'à 50ms dans la MÊME boucle que le poll retardait/gigotait ce dernier, donc le device
/// était interrogé irrégulièrement et répondait moins (376 octets/18 transferts côté Linux contre
/// 1724 octets/54 transferts côté HX Edit sur ce même canal, mesuré par capture différentielle).
/// Séparer les deux responsabilités sur deux threads élimine ce couplage : la cadence du poll ne
/// dépend plus jamais de la durée d'un `read_bulk`.
struct LiveParamPollShared {
    /// Diagnostic transitoire (gel du poll live-param) : log uniquement au CHANGEMENT d'état pour
    /// identifier laquelle des 3 conditions bloque, sans spammer.
    last_gate_ok: Option<bool>,
    /// Référence pour le compteur roulant des octets 12-15 du poll actif (voir plus bas) —
    /// capture HX Edit longue durée (`long_long_capture.pcapng`, 6 min) : ce champ progresse
    /// à ~1,14 unité/ms de façon continue, jamais figé. Notre valeur figée précédente
    /// (`09 10 01 00` constant) est la cause probable du gel après un moment (requête perçue
    /// comme périmée par le device).
    poll_epoch: Instant,
    // Récupération après gel : sur 4 captures, un silence device de ~650-800ms précède
    // systématiquement une dégradation PERMANENTE des réponses au poll actif (48/52 octets
    // → 16 octets, sans 85:62). HX Edit tolère des silences plus courts (≤243ms observé sur
    // 6 min) sans jamais dégrader. Faute de capture de référence pour une reprise, on tente
    // une récupération empirique : après N réponses dégradées consécutives, on marque une
    // pause (laisse le device souffler) puis on repart avec un tick à zéro (nouvelle
    // "session" pour le compteur roulant du poll actif).
    degraded_in_a_row: u32,
    backoff_until: Option<Instant>,
}

impl LiveParamPollShared {
    fn new() -> Self {
        Self {
            last_gate_ok: None,
            poll_epoch: Instant::now(),
            degraded_in_a_row: 0,
            backoff_until: None,
        }
    }
}

const DEGRADED_THRESHOLD: u32 = 8;
const RECOVER_BACKOFF_MS: u64 = 500;

/// Thread dédié au poll actif f0:03 (cadence 40ms, propre — voir `LiveParamPollShared`). Ne fait
/// QUE décider d'envoyer le poll et le mettre en file via `state.send()` (le thread `usb_writer`
/// se charge de l'écriture réelle, avec son propre espacement `MIN_ED03_OUT_GAP_MS`) — aucune
/// lecture, aucun couplage avec le thread de lecture 0x81.
fn start_live_param_poll_thread(
    state: Arc<Mutex<HelixState>>,
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<LiveParamPollShared>>,
) {
    thread::spawn(move || {
        // Cadence basée sur un `Instant` de référence (pas un `sleep(40ms)` répété tel quel) pour
        // éviter la dérive accumulée au fil des itérations.
        let mut next_tick = Instant::now();
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let now = Instant::now();
            if now < next_tick {
                thread::sleep(next_tick - now);
            }
            next_tick += Duration::from_millis(LIVE_PARAM_POLL_INTERVAL_MS);

            let mut sh = shared.lock().unwrap();
            let mut s = state.lock().unwrap();
            let connected = s.connected;
            let editor_ready = s.editor_ready;
            let preset_read_in_progress = s.preset_usb_read_in_progress();
            let gate_ok = connected && editor_ready && !preset_read_in_progress;
            if sh.last_gate_ok != Some(gate_ok) {
                eprintln!(
                    "[LiveParamPoll][gate] {} connected={} editor_ready={} preset_usb_read_in_progress={}",
                    if gate_ok { "OUVERT" } else { "FERMÉ" },
                    connected, editor_ready, preset_read_in_progress
                );
                sh.last_gate_ok = Some(gate_ok);
            }
            let backing_off = sh.backoff_until.map(|t| Instant::now() < t).unwrap_or(false);
            if gate_ok && !backing_off {
                let seq = s.next_x2_cnt();
                // En-tête `02:10` (pas `05:10`) : deux captures HX Edit indépendantes
                // (changement de slot au device, changement de paramètre au device —
                // juillet 2026) montrent 100% des polls actifs et 100% des réponses
                // `85:62` (knob HW) livrées avec `02:10`, jamais `05:10`. L'ancienne
                // validation "10950/10950 = 05:10" provenait très probablement d'une
                // édition faite depuis l'UI HX Edit (souris), pas depuis le device —
                // un contexte différent de celui qu'on couvre ici (gestes matériels).
                // Tail = compteur roulant (LE u32), pas figé.
                let tick = sh.poll_epoch.elapsed().as_millis() as u32;
                let tick_bytes = tick.to_le_bytes();
                let mut pkt = vec![
                    0x08, 0x00, 0x00, 0x18,
                    0x02, 0x10, 0xf0, 0x03,
                    0x00, seq, 0x00, 0x08,
                ];
                pkt.extend_from_slice(&tick_bytes);
                s.send(OutPacket::new(pkt));
            }
        }
    });
}

pub fn start_listener(
    handle: Arc<DeviceHandle<GlobalContext>>,
    state: Arc<Mutex<HelixState>>,
    mode: Arc<Mutex<Box<dyn Mode>>>,
    stop: Arc<AtomicBool>,
    session_stop: Arc<AtomicBool>,
    app_handle: Option<tauri::AppHandle>,
) {
    let poll_shared = Arc::new(Mutex::new(LiveParamPollShared::new()));
    start_live_param_poll_thread(Arc::clone(&state), Arc::clone(&stop), Arc::clone(&poll_shared));

    thread::spawn(move || {
        let mut buf = vec![0u8; BUFFER_SIZE];
        let mut seen_fingerprints: HashSet<Vec<u8>> = HashSet::new();
        let mut suppressed_repeats: u64 = 0;

        loop {
            // Vérifier si on doit s'arrêter
            if stop.load(Ordering::SeqCst) {
                break;
            }

            // Lire depuis l'endpoint 0x81 — boucle serrée, rien d'autre entre deux appels (le
            // poll périodique vit sur son propre thread depuis le 2026-07-10, voir
            // `start_live_param_poll_thread`).
            match handle.read_bulk(
                ENDPOINT_IN,
                &mut buf,
                Duration::from_millis(READ_TIMEOUT_MS),
            ) {
                Ok(n) if n > 0 => {
                    let data = buf[..n].to_vec();
                    {
                        let mut sh = poll_shared.lock().unwrap();
                        if sh.last_gate_ok == Some(true)
                            && classify_in_packet(&data) == "in_x2_stream"
                        {
                            if data.len() <= 16 {
                                sh.degraded_in_a_row += 1;
                                if sh.degraded_in_a_row == DEGRADED_THRESHOLD {
                                    eprintln!(
                                        "[LiveParamPoll][recover] {DEGRADED_THRESHOLD} réponses dégradées (len<=16) d'affilée — pause {RECOVER_BACKOFF_MS}ms puis reset du tick"
                                    );
                                    sh.backoff_until =
                                        Some(Instant::now() + Duration::from_millis(RECOVER_BACKOFF_MS));
                                    sh.poll_epoch = Instant::now();
                                }
                            } else if data.len() >= 40 {
                                if sh.degraded_in_a_row >= DEGRADED_THRESHOLD {
                                    eprintln!(
                                        "[LiveParamPoll][recover] réponse complète retrouvée (len={}) — abonnement rétabli",
                                        data.len()
                                    );
                                }
                                sh.degraded_in_a_row = 0;
                            }
                        }
                    }
                    if usb_io_diag_enabled() {
                        eprintln!(
                            "[UsbIODiag][IN][recv] kind={} len={} cnt={}",
                            classify_in_packet(&data),
                            data.len(),
                            packet_counter(&data)
                                .map(|v| format!("{:02x}", v))
                                .unwrap_or_else(|| "--".to_string())
                        );
                    }
                    if usb_packet_trace_active() {
                        let delta_only = usb_packet_trace_delta_only();
                        let fingerprint = usb_trace_fingerprint(&data);
                        let log_in = if delta_only {
                            if !seen_fingerprints.insert(fingerprint) {
                                suppressed_repeats = suppressed_repeats.saturating_add(1);
                                if suppressed_repeats % 250 == 0 {
                                    eprintln!(
                                        "[UsbTrace][IN  0x81] known patterns suppressed={}",
                                        suppressed_repeats
                                    );
                                }
                                false
                            } else {
                                if suppressed_repeats > 0 {
                                    eprintln!(
                                        "[UsbTrace][IN  0x81] known patterns suppressed total={}",
                                        suppressed_repeats
                                    );
                                    suppressed_repeats = 0;
                                }
                                true
                            }
                        } else {
                            true
                        };
                        if log_in && usb_packet_trace_should_log(&data) {
                            let hex = data
                                .iter()
                                .map(|b| format!("{:02x}", b))
                                .collect::<Vec<_>>()
                                .join(" ");
                            // Paquets courts 16o = souvent keep-alive / acquittements ; les changements
                            // de paramètre matériel peuvent être des trames plus longues (ou sur 0x82).
                            if data.len() != 16 {
                                eprintln!(
                                    "[UsbTrace][IN  0x81][len={}][non-16 — possible param / UI]",
                                    data.len()
                                );
                            }
                            eprintln!("[UsbTrace][IN  0x81][len={}] {}", data.len(), hex);
                        }
                    } else {
                        // Reset de l'état de dédup quand la trace est désactivée.
                        seen_fingerprints.clear();
                        suppressed_repeats = 0;
                    }

                    // Dispatcher vers le mode actif
                    // On lock state et mode séparément pour éviter deadlock
                    let (hw_slot_changed, fond_bootstrap_alert, slot_model_changed, path1_input_changed, path1_split_changed, slot_active_changed) = {
                        let lock_start = Instant::now();
                        let mut s = state.lock().unwrap();
                        let state_wait_ms = lock_start.elapsed().as_millis();
                        let hold_start = Instant::now();
                        if let Some(deadline) = s.usb_slot_focus_capture_deadline {
                            if Instant::now() < deadline && s.usb_slot_focus_capture.len() < 40 {
                                s.usb_slot_focus_capture.push(data.clone());
                            }
                        }
                        if let Some(deadline) = s.cab_dual_cab2_handshake_until {
                            if Instant::now() < deadline && s.cab_dual_cab2_handshake_capture.len() < 48
                            {
                                s.cab_dual_cab2_handshake_capture.push(data.clone());
                            }
                        }
                        crate::helix::cab_dual_live_write::ingest_cab_dual_cab2_in36(&mut s, &data);
                        // Échos paramètre HX Edit / firmware : mémorisés pour aligner `write_live_param`.
                        s.ingest_ed03_param_echo(&data);
                        let path1_input_changed = s
                            .ingest_path1_input_source_wire_in(&data)
                            .map(|wire| s.path1_input_source_changed_payload(wire, &data));
                        let path1_split_changed = s
                            .ingest_path1_split_type_wire_in(&data)
                            .map(|wire| s.path1_split_type_changed_payload(wire, &data));
                        // Bascule actif/inactif d'un bloc faite directement sur le device.
                        let slot_active_changed =
                            crate::helix::slot_active_state_write::ingest_slot_active_state_wire_in(&data);
                        // Slot actif unique (`hw_active_slot_*`) : `ingest_hw_slot_notify_in` — preset/HW/UI.
                        let ev = s.ingest_hw_slot_notify_in(&data);
                        crate::helix::init_trace::trace_in(&data);
                        let active = s.run_usb_in_active_layers(&data);
                        let slot_model_changed =
                            if s.hw_model_pull_capture_deadline.is_some() {
                                crate::helix::scroll_model_pull::ingest_pull_capture(&mut s, &data)
                            } else {
                                None
                            };
                        // Coalescing multi-cran : pull différé en fin de settling (dernier cran
                        // coalescé). No-op si HX_PULL_COALESCE_LAST=0 (coalescing désactivé ;
                        // défaut = activé). Indépendant de la capture ci-dessus — appelé à
                        // chaque IN (seul endroit qui « bat » hors capture).
                        crate::helix::scroll_model_pull::tick_hw_model_pull(&mut s);
                        crate::helix::legacy_cab_param_commit::tick_commit_timeouts(&mut s);
                        // ── FSM phase 4 (passive) + PHASE B (réactive : OUT via on_enter_*). ──
                        if s.phase4_step.is_active() {
                            let prev_phase4_step = s.phase4_step;
                            crate::helix::phase4_state::handle_in_passive(&mut s, &data);

                            // OUT émis À L'ENTRÉE de chaque état (déclenchement proactif PHASE B :
                            // PostArm envoie déjà le 1b 76:0e ; chaque IN 1f/19 enchaîne la requête
                            // suivante). Les IN 1d / ACK 08 entrelacés sont ignorés par la FSM.
                            if s.phase4_step != prev_phase4_step {
                                use crate::helix::phase4_state::Phase4Step as P;
                                // Armer le timeout secours à l'entrée de PostArm (début PHASE B).
                                if matches!(s.phase4_step, P::PostArm | P::WaitIn1b26 | P::PbCommit) {
                                    s.phase4_post1a_timeout =
                                        Some(Instant::now() + Duration::from_millis(2000));
                                    crate::helix::init_trace::trace_fmt(format_args!(
                                        "[PhaseB] timeout secours armé (2s, état={})",
                                        s.phase4_step.label()
                                    ));
                                }
                                match s.phase4_step {
                                    P::PostArm => {
                                        crate::helix::phase4_state::on_enter_post_arm(&mut s)
                                    }
                                    P::PbWait49 => {
                                        crate::helix::phase4_state::on_enter_pb_wait49(&mut s)
                                    }
                                    P::PbWaitCc => {
                                        crate::helix::phase4_state::on_enter_pb_waitcc(&mut s)
                                    }
                                    P::PbWait1a => {
                                        crate::helix::phase4_state::on_enter_pb_wait1a(&mut s)
                                    }
                                    P::PbWait1b => {
                                        crate::helix::phase4_state::on_enter_pb_wait1b(&mut s)
                                    }
                                    P::WaitIn1b26 => {
                                        crate::helix::phase4_state::on_enter_wait_in_1b26(&mut s)
                                    }
                                    P::PbCommit => {
                                        crate::helix::phase4_state::on_enter_pb_commit(&mut s)
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Timeout secours PHASE B : si le dialogue reste bloqué, on force Done
                        // pour ne pas empêcher la suite (RequestPresetNames). Presets OK,
                        // scroll/dialogue éditeur éventuellement incomplet.
                        if s.phase4_step.is_phase_b() {
                            if let Some(t) = s.phase4_post1a_timeout {
                                if Instant::now() >= t {
                                    crate::helix::init_trace::trace_fmt(format_args!(
                                        "[PhaseB] timeout secours -> Done (état={})",
                                        s.phase4_step.label()
                                    ));
                                    s.phase4_step = crate::helix::phase4_state::Phase4Step::Done;
                                    s.phase4_post1a_timeout = None;
                                }
                            }
                        }
                        if s.phase4_bootstrap_active
                            && crate::helix::editor_phase4_bootstrap::is_phase4_bootstrap_trailer_in(
                                &data,
                            )
                        {
                            s.note_phase4_bootstrap_complete();
                        }
                        if s.post_ef_arm_gate_active {
                            s.tick_post_ef_arm_gate(&data);
                        }
                        let fond_bootstrap_alert = if (s.connecting || s.init_usb_settle_active())
                            && data.len() == 40
                            && matches!(data.first(), Some(0x1d | 0x1f))
                            && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
                        {
                            let preview = data
                                .iter()
                                .take(16)
                                .map(|b| format!("{:02x}", b))
                                .collect::<Vec<_>>()
                                .join(":");
                            Some(format!(
                                "ALERT fond pendant amorcage: head={:02x} len={} preview={}...",
                                data.first().copied().unwrap_or(0),
                                data.len(),
                                preview
                            ))
                        } else {
                            None
                        };
                        let param_events = s.ingest_slot_param_in(&data);
                        let mode_lock_start = Instant::now();
                        let mut m = mode.lock().unwrap();
                        let mode_wait_ms = mode_lock_start.elapsed().as_millis();
                        if !matches!(
                            active.consumed_by,
                            Some(crate::helix::usb_in_pipeline::ActiveLayerId::MatrixRoutingDd)
                                | Some(crate::helix::usb_in_pipeline::ActiveLayerId::ClearAllPreset)
                        )
                        {
                            m.data_in(&data, &mut s);
                        }                        if mode_wait_ms > STATE_LOCK_WARN_MS {
                            eprintln!(
                                "[WARN] mode.lock() wait={mode_wait_ms}ms (IN len={}, HelixState déjà tenu)",
                                data.len()
                            );
                        }
                        let state_hold_ms = hold_start.elapsed().as_millis();
                        warn_slow_lock("HelixState.lock()", state_wait_ms, state_hold_ms, data.len());
                        ((ev, param_events), fond_bootstrap_alert, slot_model_changed, path1_input_changed, path1_split_changed, slot_active_changed)
                    };
                    if let (Some(app), Some(payload)) = (app_handle.as_ref(), hw_slot_changed.0) {
                        eprintln!(
                            "[HwSlot] emit hardware-slot-changed seq={} slot={} bus={}",
                            payload.sequence,
                            payload.slot_index.map(|v| v.to_string()).unwrap_or("?".into()),
                            payload.slot_bus.map(|v| format!("{v:#04x}")).unwrap_or("?".into()),
                        );
                        if let Err(e) = app.emit("models:hardware-slot-changed", payload) {
                            eprintln!("[UsbListener] emit models:hardware-slot-changed: {e}");
                        }
                    }
                    if let (Some(app), Some(payload)) = (app_handle.as_ref(), path1_input_changed) {
                        if let Err(e) = app.emit("models:path1-input-source-changed", payload) {
                            eprintln!("[UsbListener] emit models:path1-input-source-changed: {e}");
                        }
                    }
                    if let (Some(app), Some(payload)) = (app_handle.as_ref(), path1_split_changed) {
                        if let Err(e) = app.emit("models:path1-split-type-changed", payload) {
                            eprintln!("[UsbListener] emit models:path1-split-type-changed: {e}");
                        }
                    }
                    if let (Some(app), Some((slot_bus, active))) = (app_handle.as_ref(), slot_active_changed) {
                        let payload = crate::helix::slot_active_state_write::SlotActiveStateChangedPayload {
                            slot_bus,
                            active,
                        };
                        if let Err(e) = app.emit("models:slot-active-state-changed", payload) {
                            eprintln!("[UsbListener] emit models:slot-active-state-changed: {e}");
                        }
                    }
                    if let (Some(app), Some(payload)) = (app_handle.as_ref(), slot_model_changed) {
                        if let Err(e) = app.emit("models:slot-model-changed", &payload) {
                            eprintln!("[UsbListener] emit models:slot-model-changed: {e}");
                        }
                    }
                    if let Some(app) = app_handle.as_ref() {
                        if let Some(msg) = fond_bootstrap_alert {
                            if let Err(e) = app.emit("debug:fond-amorcage", msg) {
                                eprintln!("[UsbListener] emit debug:fond-amorcage: {e}");
                            }
                        }
                        for payload in hw_slot_changed.1.iter() {
                            if preset_debug_verbose_enabled()
                                || models_debug_sync_trace_enabled()
                                || slot_param_debug_enabled()
                            {
                                eprintln!(
                                    "[SlotParamIn] emit slot={} pp={} type={} val={}",
                                    payload.slot_index,
                                    payload.param_index,
                                    payload.value_type,
                                    payload.value
                                );
                            }
                            if let Err(e) = app.emit("models:slot-param-changed", payload) {
                                eprintln!("[UsbListener] emit models:slot-param-changed: {e}");
                            }
                        }
                    }
                }
                Ok(_) => {
                    // 0 bytes reçus — on continue
                }
                Err(rusb::Error::Timeout) => {
                    // Timeout normal — on reboucle pour vérifier stop
                }
                Err(rusb::Error::NoDevice) => {
                    eprintln!("[UsbListener] HX déconnecté");
                    session_stop.store(true, Ordering::SeqCst);
                    let lock_start = Instant::now();
                    let mut s = state.lock().unwrap();
                    let wait_ms = lock_start.elapsed().as_millis();
                    if wait_ms > STATE_LOCK_WARN_MS {
                        eprintln!("[WARN] HelixState.lock() wait={wait_ms}ms (NoDevice)");
                    }
                    s.connected = false;
                    s.tx = None;
                    if let Some(ka) = &s.keepalive_tx {
                        let _ = ka.send(crate::helix::KeepAliveCommand::StopAll);
                    }
                    break;
                }
                Err(e) => {
                    eprintln!("[UsbListener] erreur lecture : {}", e);
                    break;
                }
            }
        }
    });
}