// ===========================================================
// helix/keep_alive.rs
// Un seul thread keep-alive : cycle ordonné ed → ef → f0 (captures HX Edit),
// au lieu de 4 threads aux rythmes indépendants (compteurs mélangés, ed dynamique).
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

/// Période entre deux cycles complets (kempline ~1.04 s).
const KEEP_ALIVE_CYCLE_MS: u64 = 1040;
/// Pause entre deux OUT du même cycle (laisser le device / la pile répondre sur 0x81).
const BETWEEN_OPCODE_MS: u64 = 28;
/// HX Edit attend ~688 ms après le bootstrap phase 4 avant le premier poll `f0:03` court.
/// Sans ce délai, le Stomp peut encore dumper le preset sur `0x81` et ignorer le `f0`.
/// Ref. `src/Paquets Json/connect_device_30s_HXEdit.json`, frames #3447 → #3761.
const POST_PHASE4_SETTLE_MS: u64 = 700;

// ===========================================================
// Structure — un seul thread
// ===========================================================
pub struct KeepAliveManager {
    stop_ordered: Arc<AtomicBool>,
}

impl KeepAliveManager {
    pub fn new() -> Self {
        Self {
            stop_ordered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Boucle `ed:03` → `ef:03` → `f0:03`.
    ///
    /// Sur `ed` sub=10 : `double` = snapshot de `editor_ed03_double_val()` pris **une seule fois**
    /// avant la boucle — HX Edit utilise une valeur fixe `ee:1c` tout au long du polling
    /// (ref. `01_connect_HXEdit.json` : Δ=0 sur tous les cycles).
    /// Ne pas appeler `preset_data_packet_double()` dans la boucle — ça ferait varier le double
    /// à chaque cycle, ce qui n'est pas le comportement HX Edit.
    pub fn start_ordered(&self, state: Arc<Mutex<HelixState>>) {
        let stop = Arc::clone(&self.stop_ordered);
        stop.store(false, Ordering::SeqCst);

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(POST_PHASE4_SETTLE_MS));

            // Snapshot unique — valeur fixe pour toute la durée du polling
            // (aligné HX Edit : double figé à ee:1c après init, Δ=0 entre cycles)
            let (ed03_session, ed03_double) = {
                let s = state.lock().unwrap();
                (s.session_no, s.editor_ed03_double_val())
            };

            while !stop.load(Ordering::SeqCst) {
                let skip_cycle = {
                    let s = state.lock().unwrap();
                    s.preset_content_only || s.hw_model_pull_capture_deadline.is_some()
                };
                if skip_cycle {
                    thread::sleep(Duration::from_millis(KEEP_ALIVE_CYCLE_MS));
                    continue;
                }

                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x80_cnt();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x80, 0x10, 0xed, 0x03,
                        0x00, cnt, 0x00, 0x10,
                        ed03_session, ed03_double[0], ed03_double[1], 0x00,
                    ]);
                    s.send(pkt);
                }
                thread::sleep(Duration::from_millis(BETWEEN_OPCODE_MS));

                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x1_cnt();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x01, 0x10, 0xef, 0x03,
                        0x00, cnt, 0x00, 0x08,
                        0x72, 0x1e, 0x00, 0x00,
                    ]);
                    s.send(pkt);
                }
                thread::sleep(Duration::from_millis(BETWEEN_OPCODE_MS));

                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x2_cnt();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x02, 0x10, 0xf0, 0x03,
                        0x00, cnt, 0x00, 0x10,
                        0x09, 0x10, 0x00, 0x00,
                    ]);
                    s.send(pkt);
                }

                let step_budget = BETWEEN_OPCODE_MS.saturating_mul(2);
                let tail = KEEP_ALIVE_CYCLE_MS.saturating_sub(step_budget);
                thread::sleep(Duration::from_millis(tail.max(1)));
            }
        });
    }

    pub fn stop_all(&self) {
        self.stop_ordered.store(true, Ordering::SeqCst);
    }
}