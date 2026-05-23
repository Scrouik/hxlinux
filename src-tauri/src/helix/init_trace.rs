// Timeline d'initialisation USB — corrélation avec capture Wireshark (HX_INIT_TRACE=1).
//
// Rituel :
//   HX_INIT_TRACE=1 npm run tauri dev   (+ Wireshark sur le Stomp)
//   HW éteint → lancer l'app → allumer le HW → T0 = première détection USB
//
// Filtres Wireshark utiles :
//   IN scroll  : usb.endpoint_address == 0x81 && usb.capdata contains "1d:00:00:18:f0:03:02:10"
//   OUT noms   : usb.endpoint_address == 0x01 && usb.capdata contains "1d:00:00:18:01:10:ef:03"

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init_from_env() {
    if !std::env::var("HX_INIT_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return;
    }
    ENABLED.store(true, Ordering::SeqCst);
    eprintln!(
        "[InitTrace] activé — HX_INIT_TRACE=1. Éteignez le HW, lancez l'app + capture, puis allumez le Stomp."
    );
    if let Ok(path) = std::env::var("HX_INIT_TRACE_FILE") {
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(f) => {
                emit_raw(&format!("--- session file={path} ---"));
                *LOG_FILE.lock().unwrap() = Some(f);
            }
            Err(e) => eprintln!("[InitTrace] impossible d'ouvrir HX_INIT_TRACE_FILE: {e}"),
        }
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// T0 pour les deltas `+XXXms` (première détection HW).
pub fn mark_origin(event: &str) {
    if !enabled() {
        return;
    }
    let wall = wall_clock_iso();
    if ORIGIN.set(Instant::now()).is_ok() {
        emit(&format!("ORIGIN T0 wall={wall} — {event}"));
    } else {
        emit(&format!("ORIGIN déjà posé — {event}"));
    }
}

pub fn trace(event: &str) {
    if !enabled() {
        return;
    }
    emit(event);
}

pub fn trace_fmt(args: std::fmt::Arguments<'_>) {
    if !enabled() {
        return;
    }
    emit(&args.to_string());
}

/// OUT 0x01 — jalons (1d ef:03 noms, ed:03 dump, …). Les ACK `08` courants sont filtrés.
pub fn trace_out(data: &[u8], label: &str) {
    if !enabled() || data.is_empty() {
        return;
    }
    let head = data[0];
    let milestone = matches!(head, 0x1d | 0x19 | 0x1a | 0x1b | 0x21 | 0x27);
    if label == "send" && !milestone {
        return;
    }
    let kind = classify_packet_1d_lane(data);
    let cnt = packet_counter_byte(data);
    let hex8 = hex_prefix(data, 8);
    emit(&format!(
        "OUT {label} head={head:#04x} lane={kind} cnt={cnt} hex8={hex8}"
    ));
}

/// IN 0x81 — notamment distinguer `1d f0:03` (notif) vs autre.
pub fn trace_in(data: &[u8]) {
    if !enabled() || data.is_empty() {
        return;
    }
    let head = data[0];
    if head != 0x1d && head != 0x1f && head != 0x21 {
        return;
    }
    let kind = classify_packet_1d_lane(data);
    let cnt = packet_counter_byte(data);
    let hex8 = hex_prefix(data, 8);
    emit(&format!(
        "IN  head={head:#04x} lane={kind} len={} cnt={cnt} hex8={hex8}",
        data.len()
    ));
}

pub fn trace_1d_ack_decision(acked: bool, reason: &str) {
    if !enabled() {
        return;
    }
    emit(&format!(
        "ACK_1d decision={} reason={reason}",
        if acked { "send" } else { "skip" }
    ));
}

pub fn trace_mode_switch(mode: &str, detail: &str) {
    if !enabled() {
        return;
    }
    emit(&format!("MODE → {mode}{detail}"));
}

fn classify_packet_1d_lane(data: &[u8]) -> &'static str {
    if data.first() != Some(&0x1d) {
        return if data.first() == Some(&0x1f) {
            "1f_notify"
        } else if data.first() == Some(&0x21) {
            "21_frame"
        } else {
            "other"
        };
    }
    if data.len() >= 8 && data[4..8] == [0x01, 0x10, 0xef, 0x03] {
        return "1d_OUT_names_ef03";
    }
    if data.len() >= 8 && data[4..8] == [0xf0, 0x03, 0x02, 0x10] {
        return "1d_IN_notify_f003";
    }
    "1d_other"
}

fn packet_counter_byte(data: &[u8]) -> String {
    if data.len() > 9 {
        format!("{:02x}", data[9])
    } else {
        "--".into()
    }
}

fn hex_prefix(data: &[u8], n: usize) -> String {
    data.iter()
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn wall_clock_iso() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let ms = dur.subsec_millis();
    format!("{secs}.{ms:03}Z")
}

fn emit(msg: &str) {
    emit_raw(&format_line(msg));
}

fn emit_raw(line: &str) {
    eprintln!("{line}");
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

fn format_line(msg: &str) -> String {
    if let Some(t0) = ORIGIN.get() {
        format!(
            "[InitTrace][+{:09.3}ms] {}",
            t0.elapsed().as_secs_f64() * 1000.0,
            msg
        )
    } else {
        format!("[InitTrace][pre-T0] {msg}")
    }
}
