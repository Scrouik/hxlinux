// ===========================================================
// helix/keep_alive.rs
// Thread keep-alive : poll `ed:03` sub=`10` (~1,04 s).
//
// Captures HX Edit (`01_connect`, `stomp_running` idle) : après settle,
// seul OUT périodique = `80:10:ed:03` sub=`10` + double éditeur figé (`ee:1c` / `7e:1c`).
// Pas de poll `ef` sub=`08` ni `f0` sub=`10` en boucle — le fond scroll = IN `1d` →
// OUT `f0` sub=`08` via `firmware_scroll_ack` (lane qui avance).
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

/// Période entre deux polls `ed` (kempline / HX Edit ~1,04 s).
const KEEP_ALIVE_CYCLE_MS: u64 = 1040;
/// Délai HX Edit après bootstrap phase 4 avant le 1er poll `ed` et `RequestPresetNames`.
pub const POST_PHASE4_SETTLE_MS: u64 = 700;

pub struct KeepAliveManager {
    stop_ordered: Arc<AtomicBool>,
}

impl KeepAliveManager {
    pub fn new() -> Self {
        Self {
            stop_ordered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Poll `ed:03` sub=`10` — double = [`HelixState::editor_ed03_double_val`] (lane éditeur,
    /// pas la lane scroll `09:10`). Relu à chaque cycle pour suivre phase 4 / preset sans
    /// hardcoder `72:1e` ou réutiliser la lane scroll sur `f0`.
    pub fn start_ordered(&self, state: Arc<Mutex<HelixState>>) {
        let stop = Arc::clone(&self.stop_ordered);
        stop.store(false, Ordering::SeqCst);

        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let skip_cycle = {
                    let s = state.lock().unwrap();
                    s.preset_content_only
                };
                if skip_cycle {
                    thread::sleep(Duration::from_millis(KEEP_ALIVE_CYCLE_MS));
                    continue;
                }

                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x80_cnt();
                    let session = s.session_no;
                    let double = s.editor_ed03_double_val();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x80, 0x10, 0xed, 0x03,
                        0x00, cnt, 0x00, 0x10,
                        session, double[0], double[1], 0x00,
                    ]);
                    s.send(pkt);
                }

                thread::sleep(Duration::from_millis(KEEP_ALIVE_CYCLE_MS));
            }
        });
    }

    pub fn stop_all(&self) {
        self.stop_ordered.store(true, Ordering::SeqCst);
    }
}
