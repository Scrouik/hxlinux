// ===========================================================
// helix/keep_alive.rs
// Keep-alive idle tri-lane — aligné capture HX Edit (`stomp_running_start_hxedit.json`).
//
// HX Edit en idle (t ≥ ~9 s, 21 cycles observés) n'envoie PAS un seul poll mais
// TROIS polls `sub=10` par cycle (~1,047 s), dans l'ordre `f0 → ed → ef`, chaque
// lane avançant son propre compteur octet 9 (+1/cycle), avec un trailer FIXE :
//
//   f0:03 (02:10)  cnt=x2_cnt   tail = 09 10 00 00   (+0 ms dans le cycle)
//   ed:03 (80:10)  cnt=x80_cnt  tail = 7e 1c 00 00   (+~453 ms)
//   ef:03 (01:10)  cnt=x1_cnt   tail = e0 1d 00 00   (+~266 ms)  → +~328 ms → cycle suivant
//
// Le device NE ré-écho PAS le trailer (il le transforme : 09:10→09:02, 7e:1c→ae:02,
// l'octet 13 passe à 02). Donc le trailer n'est pas un tag de corrélation strict :
// le Stomp l'accepte et répond. Les trois trailers sont donc figés en littéraux.
//
// Compteurs : partagés avec le reste (x2 ↔ firmware_scroll_ack, x1 ↔ RequestPresetNames).
// En idle pur (pas de scroll, pas de requête host), seules ces trois lanes consomment
// leurs compteurs → séquence +1/cycle propre, comme HX. Le saut observé sur
// `one_scroll_hxlinux` n'apparaît QUE pendant un scroll, où HX intercale aussi ses
// ACK f0 sub=08 sur la même lane : comportement attendu, pas une régression.
//
// IMPORTANT : c'est le POLL idle (sub=10). Les ARM bootstrap (`amorcage`) et les
// ACK scroll (`firmware_scroll_ack`) restent sur sub=08 / 09:10 — ne pas confondre.
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

static KEEPALIVE_TRACE: AtomicBool = AtomicBool::new(false);

pub fn init_from_env() {
    if !std::env::var("HX_KEEPALIVE_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return;
    }
    KEEPALIVE_TRACE.store(true, Ordering::Relaxed);
    eprintln!("[KeepAliveTrace] activé — HX_KEEPALIVE_TRACE=1 (OUT ed sub=10 uniquement)");
}

fn trace_ed_poll(cnt: u8) {
    if !KEEPALIVE_TRACE.load(Ordering::Relaxed) {
        return;
    }
    eprintln!(
        "[KeepAliveTrace] OUT ed sub=10 cnt={cnt:02x} tail={:02x}:{:02x}",
        TAIL_ED[0], TAIL_ED[1]
    );
}

/// Période totale d'un cycle (somme des trois deltas inter-lanes). HX ≈ 1,047 s.
pub const KEEP_ALIVE_CYCLE_MS: u64 = 1047;

/// Délais intra-cycle observés sur HX Edit idle (`f0 → ed → ef → f0`).
/// f0 est l'origine du cycle (+0 ms) ; les sleeps séparent les envois suivants.
const DELAY_F0_TO_ED_MS: u64 = 453;
const DELAY_ED_TO_EF_MS: u64 = 266;
const DELAY_EF_TO_NEXT_F0_MS: u64 = 328; // 453 + 266 + 328 = 1047

/// Délai HX Edit après bootstrap phase 4 avant le 1er poll et `RequestPresetNames`.
pub const POST_PHASE4_SETTLE_MS: u64 = 700;

/// Trailers idle figés (octets 12–15) relevés sur HX Edit, constants sur 21 cycles.
/// Seule référence 3-lanes idle connue-bonne ; le device les transforme sans rejeter.
const TAIL_F0: [u8; 4] = [0x09, 0x10, 0x00, 0x00];
const TAIL_ED: [u8; 4] = [0x7e, 0x1c, 0x00, 0x00];
const TAIL_EF: [u8; 4] = [0xe0, 0x1d, 0x00, 0x00];

pub struct KeepAliveManager {
    stop_ordered: Arc<AtomicBool>,
}

impl KeepAliveManager {
    pub fn new() -> Self {
        Self {
            stop_ordered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cycle idle tri-lane `f0 → ed → ef` (sub=10), trailers figés HX.
    /// Un seul thread : l'ordre et les deltas inter-lanes sont sous contrôle direct
    /// (≠ trois timers libres qui dériveraient).
    pub fn start_ordered(&self, state: Arc<Mutex<HelixState>>) {
        let stop = Arc::clone(&self.stop_ordered);
        stop.store(false, Ordering::SeqCst);

        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                // Pendant une lecture de corps de preset (`preset_content_only`),
                // le host n'émet pas de poll proactif : on saute le cycle entier.
                let skip_cycle = {
                    let s = state.lock().unwrap();
                    s.preset_content_only
                };
                if skip_cycle {
                    thread::sleep(Duration::from_millis(KEEP_ALIVE_CYCLE_MS));
                    continue;
                }

                // ── Lane f0 (origine du cycle, +0 ms) ─────────────────────────
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x2_cnt();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x02, 0x10, 0xf0, 0x03,
                        0x00, cnt, 0x00, 0x10,
                        TAIL_F0[0], TAIL_F0[1], TAIL_F0[2], TAIL_F0[3],
                    ]);
                    s.send(pkt);
                }
                if Self::sleep_or_stop(&stop, DELAY_F0_TO_ED_MS) {
                    break;
                }

                // ── Lane ed (+~453 ms) ────────────────────────────────────────
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x80_cnt();
                    trace_ed_poll(cnt);
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x80, 0x10, 0xed, 0x03,
                        0x00, cnt, 0x00, 0x10,
                        TAIL_ED[0], TAIL_ED[1], TAIL_ED[2], TAIL_ED[3],
                    ]);
                    s.send(pkt);
                }
                if Self::sleep_or_stop(&stop, DELAY_ED_TO_EF_MS) {
                    break;
                }

                // ── Lane ef (+~266 ms) ────────────────────────────────────────
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x1_cnt();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x01, 0x10, 0xef, 0x03,
                        0x00, cnt, 0x00, 0x10,
                        TAIL_EF[0], TAIL_EF[1], TAIL_EF[2], TAIL_EF[3],
                    ]);
                    s.send(pkt);
                }
                if Self::sleep_or_stop(&stop, DELAY_EF_TO_NEXT_F0_MS) {
                    break;
                }
            }
        });
    }

    /// Sleep interruptible : retourne `true` si un arrêt a été demandé pendant l'attente
    /// (granularité ~20 ms pour rester réactif au shutdown sans busy-loop).
    fn sleep_or_stop(stop: &Arc<AtomicBool>, total_ms: u64) -> bool {
        const STEP_MS: u64 = 20;
        let mut remaining = total_ms;
        while remaining > 0 {
            if stop.load(Ordering::SeqCst) {
                return true;
            }
            let step = remaining.min(STEP_MS);
            thread::sleep(Duration::from_millis(step));
            remaining -= step;
        }
        stop.load(Ordering::SeqCst)
    }

    pub fn stop_all(&self) {
        self.stop_ordered.store(true, Ordering::SeqCst);
    }
}