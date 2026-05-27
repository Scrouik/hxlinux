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
    usb_packet_trace_delta_only, usb_packet_trace_enabled, usb_trace_fingerprint,
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
                    if usb_packet_trace_enabled() {
                        let delta_only = usb_packet_trace_delta_only();
                        let fingerprint = usb_trace_fingerprint(&data);
                        if delta_only {
                            if !seen_fingerprints.insert(fingerprint.clone()) {
                                suppressed_repeats = suppressed_repeats.saturating_add(1);
                                if suppressed_repeats % 250 == 0 {
                                    eprintln!(
                                        "[UsbTrace][IN  0x81] known patterns suppressed={}",
                                        suppressed_repeats
                                    );
                                }
                                continue;
                            } else if suppressed_repeats > 0 {
                                eprintln!(
                                    "[UsbTrace][IN  0x81] known patterns suppressed total={}",
                                    suppressed_repeats
                                );
                                suppressed_repeats = 0;
                            }
                        }
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
                    } else {
                        // Reset de l'état de dédup quand la trace est désactivée.
                        seen_fingerprints.clear();
                        suppressed_repeats = 0;
                    }

                    // Dispatcher vers le mode actif
                    // On lock state et mode séparément pour éviter deadlock
                    let (hw_slot_changed, fond_bootstrap_alert) = {
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
                        ((ev, param_events), fond_bootstrap_alert)
                    };
                    if let (Some(app), Some(payload)) = (app_handle.as_ref(), hw_slot_changed.0) {
                        if let Err(e) = app.emit("models:hardware-slot-changed", payload) {
                            eprintln!("[UsbListener] emit models:hardware-slot-changed: {e}");
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