// ===========================================================
// helix/usb_writer.rs
// Thread d'écriture vers le HX sur endpoint 0x01
// Tous les modules envoient via state.send(pkt)
// Ce thread est le seul à toucher l'USB en écriture
// ===========================================================

use std::sync::mpsc::Receiver;
use std::collections::HashSet;
use std::thread;
use std::time::Duration;
use rusb::DeviceHandle;
use rusb::GlobalContext;

use crate::helix::packet::OutPacket;
use crate::helix::{usb_packet_trace_delta_only, usb_packet_trace_enabled, usb_trace_fingerprint};

const ENDPOINT_OUT: u8 = 0x01;
const WRITE_TIMEOUT_MS: u64 = 1000;

pub fn start_writer(
    handle: std::sync::Arc<DeviceHandle<GlobalContext>>,
    rx: Receiver<OutPacket>,
) {
    thread::spawn(move || {
        let mut seen_fingerprints: HashSet<Vec<u8>> = HashSet::new();
        let mut suppressed_repeats: u64 = 0;
        loop {
            // On attend le prochain paquet à envoyer
            match rx.recv() {
                Ok(pkt) => {
                    if usb_packet_trace_enabled() {
                        let delta_only = usb_packet_trace_delta_only();
                        let fingerprint = usb_trace_fingerprint(&pkt.data);
                        if delta_only {
                            if !seen_fingerprints.insert(fingerprint.clone()) {
                                suppressed_repeats = suppressed_repeats.saturating_add(1);
                                continue;
                            } else if suppressed_repeats > 0 {
                                eprintln!(
                                    "[UsbTrace][OUT 0x01] known patterns suppressed total={}",
                                    suppressed_repeats
                                );
                                suppressed_repeats = 0;
                            }
                        }
                        if !delta_only || seen_fingerprints.contains(&fingerprint) {
                            let hex = pkt
                                .data
                                .iter()
                                .map(|b| format!("{:02x}", b))
                                .collect::<Vec<_>>()
                                .join(" ");
                            eprintln!("[UsbTrace][OUT 0x01][len={}] {}", pkt.data.len(), hex);
                        }
                    } else {
                        seen_fingerprints.clear();
                        suppressed_repeats = 0;
                    }
                    // Délai éventuel avant envoi (kempline : delay=0.140)
                    if pkt.delay_ms > 0 {
                        thread::sleep(Duration::from_millis(pkt.delay_ms));
                    }

                    // Envoi sur endpoint 0x01
                    match handle.write_bulk(
                        ENDPOINT_OUT,
                        &pkt.data,
                        Duration::from_millis(WRITE_TIMEOUT_MS),
                    ) {
                        Ok(_) => {}
                        Err(e) => eprintln!("[UsbWriter] erreur écriture : {}", e),
                    }
                }
                Err(_) => {
                    // Le channel est fermé → on arrête le thread
                    break;
                }
            }
        }
    });
}