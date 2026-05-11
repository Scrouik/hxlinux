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

use crate::helix::{
    HelixState, Mode, usb_io_diag_enabled, usb_packet_trace_delta_only, usb_packet_trace_enabled,
    usb_trace_fingerprint,
};
use crate::helix::packet::{classify_in_packet, packet_counter};

const ENDPOINT_IN: u8 = 0x81;
const READ_TIMEOUT_MS: u64 = 500;
const BUFFER_SIZE: usize = 512;

pub fn start_listener(
    handle: Arc<DeviceHandle<GlobalContext>>,
    state:  Arc<Mutex<HelixState>>,
    mode:   Arc<Mutex<Box<dyn Mode>>>,
    stop:   Arc<AtomicBool>,
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
                    let mut s = state.lock().unwrap();
                    if let Some(deadline) = s.usb_slot_focus_capture_deadline {
                        if Instant::now() < deadline && s.usb_slot_focus_capture.len() < 40 {
                            s.usb_slot_focus_capture.push(data.clone());
                        }
                    }
                    // Échos paramètre HX Edit / firmware : mémorisés pour aligner `write_live_param`.
                    s.ingest_ed03_param_echo(&data);
                    // Notification « slot hardware » (16+16 octets IN), voir Line6_HX_Stomp_USB_Protocol.md
                    s.ingest_hw_slot_notify_in(&data);
                    let mut m = mode.lock().unwrap();
                    m.data_in(&data, &mut s);
                }
                Ok(_) => {
                    // 0 bytes reçus — on continue
                }
                Err(rusb::Error::Timeout) => {
                    // Timeout normal — on reboucle pour vérifier stop
                }
                Err(rusb::Error::NoDevice) => {
                    eprintln!("[UsbListener] HX déconnecté");
                    let mut s = state.lock().unwrap();
                    s.connected = false;
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