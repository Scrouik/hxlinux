// ===========================================================
// helix/usb_writer.rs
// Thread d'écriture vers le HX sur endpoint 0x01
// Tous les modules envoient via state.send(pkt)
// Ce thread est le seul à toucher l'USB en écriture
// ===========================================================

use std::sync::mpsc::Receiver;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use rusb::DeviceHandle;
use rusb::GlobalContext;

use crate::helix::packet::{OutPacket, classify_out_packet, packet_counter};
use crate::helix::{usb_io_diag_enabled, usb_packet_trace_delta_only, usb_packet_trace_enabled, usb_trace_fingerprint};

const ENDPOINT_OUT: u8 = 0x01;
// kempline utilise 100ms ; 150ms laisse une marge sans bloquer le canal trop longtemps
// en cas d'absence de réponse device (ex : lecture preset en cours côté firmware).
const WRITE_TIMEOUT_MS: u64 = 150;
static USB_WRITE_SEQ: AtomicU64 = AtomicU64::new(1);

/// Délai minimal entre deux envois OUT contenant l’opcode ED03 (`80 10 ed 03` aux octets 4–7).
/// Les captures HX Edit montrent moins de rafales que HXLinux ; sans cette garde, les ACK
/// Phase preset / LED / keep-alive x80 peuvent partir en salves et saturer le device (timeouts).
const MIN_ED03_OUT_GAP_MS: u64 = 14;

#[inline]
fn out_payload_has_ed03(data: &[u8]) -> bool {
    data.len() >= 8 && data[4..8] == [0x80, 0x10, 0xed, 0x03]
}

pub fn start_writer(
    handle: std::sync::Arc<DeviceHandle<GlobalContext>>,
    rx: Receiver<OutPacket>,
) {
    thread::spawn(move || {
        let mut seen_fingerprints: HashSet<Vec<u8>> = HashSet::new();
        let mut suppressed_repeats: u64 = 0;
        let mut consecutive_errors: u32 = 0;
        let mut last_ed03_out: Option<Instant> = None;
        loop {
            // On attend le prochain paquet à envoyer
            match rx.recv() {
                Ok(pkt) => {
                    let write_id = USB_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
                    let kind = classify_out_packet(&pkt.data);
                    let cnt = packet_counter(&pkt.data);
                    if usb_io_diag_enabled() {
                        eprintln!(
                            "[UsbIODiag][OUT][queue] id={} kind={} len={} delay_ms={} cnt={}",
                            write_id,
                            kind,
                            pkt.data.len(),
                            pkt.delay_ms,
                            cnt.map(|v| format!("{:02x}", v)).unwrap_or_else(|| "--".to_string())
                        );
                    }
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
                    // Délai demandé par l’émetteur (kempline : delay=0.140)
                    if pkt.delay_ms > 0 {
                        thread::sleep(Duration::from_millis(pkt.delay_ms));
                    }

                    if out_payload_has_ed03(&pkt.data) {
                        if let Some(prev) = last_ed03_out {
                            let min_gap = Duration::from_millis(MIN_ED03_OUT_GAP_MS);
                            let elapsed = prev.elapsed();
                            if elapsed < min_gap {
                                let pad = min_gap - elapsed;
                                if usb_io_diag_enabled() {
                                    eprintln!(
                                        "[UsbWriter][pace] ed03 extra_sleep_ms={}",
                                        pad.as_millis()
                                    );
                                }
                                thread::sleep(pad);
                            }
                        }
                    }

                    // Envoi sur endpoint 0x01
                    match handle.write_bulk(
                        ENDPOINT_OUT,
                        &pkt.data,
                        Duration::from_millis(WRITE_TIMEOUT_MS),
                    ) {
                        Ok(written) => {
                            consecutive_errors = 0;
                            if out_payload_has_ed03(&pkt.data) {
                                last_ed03_out = Some(Instant::now());
                            }
                            if usb_io_diag_enabled() {
                                eprintln!(
                                    "[UsbIODiag][OUT][ok] id={} kind={} written={} cnt={}",
                                    write_id,
                                    kind,
                                    written,
                                    cnt.map(|v| format!("{:02x}", v)).unwrap_or_else(|| "--".to_string())
                                );
                            }
                        }
                        Err(e) => {
                            consecutive_errors += 1;
                            eprintln!("[UsbWriter] erreur écriture : {} (consec={})", e, consecutive_errors);
                            if out_payload_has_ed03(&pkt.data) {
                                // Éviter d’enchaîner des ED03 en rafale après timeout / device occupé.
                                last_ed03_out = Some(Instant::now());
                            }
                            if usb_io_diag_enabled() {
                                eprintln!(
                                    "[UsbIODiag][OUT][err] id={} kind={} err={} cnt={} consec={}",
                                    write_id,
                                    kind,
                                    e,
                                    cnt.map(|v| format!("{:02x}", v)).unwrap_or_else(|| "--".to_string()),
                                    consecutive_errors
                                );
                            }
                            // Stall USB (Pipe) : clear_halt pour débloquer le pipe OUT.
                            if e == rusb::Error::Pipe {
                                eprintln!("[UsbWriter] pipe stall détecté → clear_halt 0x01");
                                let _ = handle.clear_halt(ENDPOINT_OUT);
                            }
                        }
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