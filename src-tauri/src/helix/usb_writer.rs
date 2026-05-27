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

/// Délai minimal entre deux envois OUT **commande** ED03 (`1b`, `19`, keep-alive `ed`, etc.).
/// Les ACK flux 272 (`ed:03` sub=`08`, 16 o) sont exclus : HX Edit les enchaîne sans 14 ms
/// entre chaque chunk (sinon 10×14 ms bloque le writer pendant la rafale post-pull).
const MIN_ED03_OUT_GAP_MS: u64 = 14;

#[inline]
fn out_payload_has_ed03(data: &[u8]) -> bool {
    data.len() >= 8 && data[4..8] == [0x80, 0x10, 0xed, 0x03]
}

/// OUT ACK chunk preset/slot (`preset_dump_stream_ack`) — pas soumis à [`MIN_ED03_OUT_GAP_MS`].
#[inline]
fn is_preset_dump_stream_ack_out(data: &[u8]) -> bool {
    data.len() == 16
        && data.get(0..4) == Some(&[0x08, 0x00, 0x00, 0x18])
        && data[4..8] == [0x80, 0x10, 0xed, 0x03]
        && data.get(11) == Some(&0x08)
}

/// `ed:03` soumis au pacing inter-paquets (exclut les ACK sub=`08` du flux 272).
#[inline]
fn out_payload_subject_to_ed03_gap(data: &[u8]) -> bool {
    out_payload_has_ed03(data) && !is_preset_dump_stream_ack_out(data)
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

                    if out_payload_subject_to_ed03_gap(&pkt.data) {
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

                    let mut wire_chunks: Vec<Vec<u8>> = Vec::with_capacity(1 + pkt.tail_burst.len());
                    wire_chunks.push(pkt.data.clone());
                    wire_chunks.extend(pkt.tail_burst.iter().cloned());

                    for (bi, chunk) in wire_chunks.iter().enumerate() {
                        if bi > 0 && out_payload_subject_to_ed03_gap(chunk) {
                            if let Some(prev) = last_ed03_out {
                                let min_gap = Duration::from_millis(MIN_ED03_OUT_GAP_MS);
                                let elapsed = prev.elapsed();
                                if elapsed < min_gap {
                                    thread::sleep(min_gap - elapsed);
                                }
                            }
                        }
                        match handle.write_bulk(
                            ENDPOINT_OUT,
                            chunk,
                            Duration::from_millis(WRITE_TIMEOUT_MS),
                        ) {
                            Ok(written) => {
                                consecutive_errors = 0;
                                if out_payload_subject_to_ed03_gap(chunk) {
                                    last_ed03_out = Some(Instant::now());
                                }
                                if usb_io_diag_enabled() {
                                    eprintln!(
                                        "[UsbIODiag][OUT][ok] id={} burst={}/{} kind={} written={}",
                                        write_id,
                                        bi + 1,
                                        wire_chunks.len(),
                                        classify_out_packet(chunk),
                                        written,
                                    );
                                }
                            }
                            Err(e) => {
                                consecutive_errors += 1;
                                eprintln!(
                                    "[UsbWriter] erreur écriture burst {}/{} : {} (consec={})",
                                    bi + 1,
                                    wire_chunks.len(),
                                    e,
                                    consecutive_errors
                                );
                                if out_payload_subject_to_ed03_gap(chunk) {
                                    last_ed03_out = Some(Instant::now());
                                }
                                if usb_io_diag_enabled() {
                                    eprintln!(
                                        "[UsbIODiag][OUT][err] id={} burst={}/{} err={} consec={}",
                                        write_id,
                                        bi + 1,
                                        wire_chunks.len(),
                                        e,
                                        consecutive_errors
                                    );
                                }
                                if e == rusb::Error::Pipe {
                                    eprintln!("[UsbWriter] pipe stall détecté → clear_halt 0x01");
                                    let _ = handle.clear_halt(ENDPOINT_OUT);
                                }
                                if e == rusb::Error::NoDevice || consecutive_errors >= 5 {
                                    return;
                                }
                                break;
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

#[cfg(test)]
mod tests {
    use super::*;

    const ACK_272: [u8; 16] = [
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x01, 0x00, 0x08, 0xf8, 0x6d, 0x00,
        0x00,
    ];

    #[test]
    fn preset_dump_ack_out_skips_ed03_gap() {
        assert!(out_payload_has_ed03(&ACK_272));
        assert!(is_preset_dump_stream_ack_out(&ACK_272));
        assert!(!out_payload_subject_to_ed03_gap(&ACK_272));
    }

    #[test]
    fn pull_1b_still_subject_to_ed03_gap() {
        let pull_1b = [
            0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x90, 0x00, 0x04, 0x00, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03,
            0x02, 0x65, 0x2d, 0x65, 0x81, 0x62, 0x01, 0x00,
        ];
        assert!(out_payload_subject_to_ed03_gap(&pull_1b));
        assert!(!is_preset_dump_stream_ack_out(&pull_1b));
    }
}