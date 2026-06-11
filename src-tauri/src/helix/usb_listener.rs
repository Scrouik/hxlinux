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
    HelixState, Mode, preset_debug_verbose_enabled, usb_io_diag_enabled,
    usb_packet_trace_active, usb_packet_trace_delta_only, usb_packet_trace_should_log,
    usb_trace_fingerprint,
};
use crate::helix::packet::{classify_in_packet, packet_counter};

const ENDPOINT_IN: u8 = 0x81;
const READ_TIMEOUT_MS: u64 = 500;
const BUFFER_SIZE: usize = 512;
/// Log si acquisition ou section critique HelixState dépasse ce seuil (contention / travail long).
const STATE_LOCK_WARN_MS: u128 = 10;

fn warn_slow_lock(label: &str, wait_ms: u128, hold_ms: u128, in_len: usize) {
    if wait_ms > STATE_LOCK_WARN_MS || hold_ms > STATE_LOCK_WARN_MS {
        eprintln!(
            "[WARN] {label} wait={wait_ms}ms hold={hold_ms}ms (IN len={in_len})"
        );
    }
}

pub fn start_listener(
    handle: Arc<DeviceHandle<GlobalContext>>,
    state: Arc<Mutex<HelixState>>,
    mode: Arc<Mutex<Box<dyn Mode>>>,
    stop: Arc<AtomicBool>,
    session_stop: Arc<AtomicBool>,
    app_handle: Option<tauri::AppHandle>,
) {
    thread::spawn(move || {
        let mut buf = vec![0u8; BUFFER_SIZE];
        let mut seen_fingerprints: HashSet<Vec<u8>> = HashSet::new();
        let mut suppressed_repeats: u64 = 0;

        loop {
            // Vérifier si on doit s'arrêter
            if stop.load(Ordering::SeqCst) {
                break;
            }

            // Lire depuis l'endpoint 0x81
            match handle.read_bulk(
                ENDPOINT_IN,
                &mut buf,
                Duration::from_millis(READ_TIMEOUT_MS),
            ) {
                Ok(n) if n > 0 => {
                    let data = buf[..n].to_vec();
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
                    let (hw_slot_changed, fond_bootstrap_alert, slot_model_changed) = {
                        let lock_start = Instant::now();
                        let mut s = state.lock().unwrap();
                        let state_wait_ms = lock_start.elapsed().as_millis();
                        let hold_start = Instant::now();
                        if let Some(deadline) = s.usb_slot_focus_capture_deadline {
                            if Instant::now() < deadline && s.usb_slot_focus_capture.len() < 40 {
                                s.usb_slot_focus_capture.push(data.clone());
                            }
                        }
                        // Échos paramètre HX Edit / firmware : mémorisés pour aligner `write_live_param`.
                        s.ingest_ed03_param_echo(&data);
                        // Slot actif unique (`hw_active_slot_*`) : `ingest_hw_slot_notify_in` — preset/HW/UI.
                        let ev = s.ingest_hw_slot_notify_in(&data);
                        crate::helix::init_trace::trace_in(&data);
                        let _active = s.run_usb_in_active_layers(&data);
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
                                if s.phase4_step == P::PostArm && s.phase4_post1a_timeout.is_none() {
                                    s.phase4_post1a_timeout =
                                        Some(Instant::now() + Duration::from_millis(2000));
                                    crate::helix::init_trace::trace(
                                        "[PhaseB] timeout secours armé (2s)",
                                    );
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
                        m.data_in(&data, &mut s);
                        if mode_wait_ms > STATE_LOCK_WARN_MS {
                            eprintln!(
                                "[WARN] mode.lock() wait={mode_wait_ms}ms (IN len={}, HelixState déjà tenu)",
                                data.len()
                            );
                        }
                        let state_hold_ms = hold_start.elapsed().as_millis();
                        warn_slow_lock("HelixState.lock()", state_wait_ms, state_hold_ms, data.len());
                        ((ev, param_events), fond_bootstrap_alert, slot_model_changed)
                    };
                    if let (Some(app), Some(payload)) = (app_handle.as_ref(), hw_slot_changed.0) {
                        if let Err(e) = app.emit("models:hardware-slot-changed", payload) {
                            eprintln!("[UsbListener] emit models:hardware-slot-changed: {e}");
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
                            if preset_debug_verbose_enabled() {
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