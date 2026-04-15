// ===========================================================
// helix/keep_alive.rs
// 3 threads keep-alive permanents : x1, x2, x80
// Équivalent de start_x1x10_keep_alive_thread(),
//              start_x2x10_keep_alive_thread(),
//              start_x80x10_keep_alive_thread() dans kempline
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

// Intervalle entre chaque keep-alive (kempline utilise 1.04s)
const KEEP_ALIVE_INTERVAL_MS: u64 = 1040;

// ===========================================================
// Structure qui gère les 3 threads keep-alive
// ===========================================================
pub struct KeepAliveManager {
    // Flags pour arrêter chaque thread proprement
    stop_x1:  Arc<AtomicBool>,
    stop_x2:  Arc<AtomicBool>,
    stop_x80: Arc<AtomicBool>,
}

impl KeepAliveManager {
    pub fn new() -> Self {
        Self {
            stop_x1:  Arc::new(AtomicBool::new(false)),
            stop_x2:  Arc::new(AtomicBool::new(false)),
            stop_x80: Arc::new(AtomicBool::new(false)),
        }
    }

    // -- Démarre le thread keep-alive x1
    // Kempline : start_x1x10_keep_alive_thread()
    pub fn start_x1(&self, state: Arc<Mutex<HelixState>>) {
        let stop = Arc::clone(&self.stop_x1);
        stop.store(false, Ordering::SeqCst);

        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x1_cnt();
                    // Kempline : paquet keep-alive x1
                    // thread x1
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x01, 0x10, 0xef, 0x03,
                        0x00, cnt,  0x00, 0x08,
                        0x72, 0x1e, 0x00, 0x00,
                    ]);
                    s.send(pkt);
                }
                thread::sleep(Duration::from_millis(KEEP_ALIVE_INTERVAL_MS));
            }
        });
    }

    // -- Démarre le thread keep-alive x2
    // Kempline : start_x2x10_keep_alive_thread()
    pub fn start_x2(&self, state: Arc<Mutex<HelixState>>) {
        let stop = Arc::clone(&self.stop_x2);
        stop.store(false, Ordering::SeqCst);

        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x2_cnt();
                    // Kempline : paquet keep-alive x2
                    // thread x2
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x02, 0x10, 0xf0, 0x03,
                        0x00, cnt,  0x00, 0x10,
                        0x09, 0x10, 0x00, 0x00,
                    ]);
                    s.send(pkt);
                }
                thread::sleep(Duration::from_millis(KEEP_ALIVE_INTERVAL_MS));
            }
        });
    }

    // -- Démarre le thread keep-alive x80
    // Kempline : start_x80x10_keep_alive_thread()
    pub fn start_x80(&self, state: Arc<Mutex<HelixState>>) {
        let stop = Arc::clone(&self.stop_x80);
        stop.store(false, Ordering::SeqCst);

        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x80_cnt();
                    // Kempline : paquet keep-alive x80
                    // thread x80
                    let session = s.session_no;
                    let double  = s.preset_data_packet_double();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x80, 0x10, 0xed, 0x03,
                        0x00, cnt,  0x00, 0x10,
                        session, double[0], double[1], 0x00,
                    ]);
                    s.send(pkt);
                }
                thread::sleep(Duration::from_millis(KEEP_ALIVE_INTERVAL_MS));
            }
        });
    }

    // -- Arrête tous les threads proprement
    pub fn stop_all(&self) {
        self.stop_x1.store(true, Ordering::SeqCst);
        self.stop_x2.store(true, Ordering::SeqCst);
        self.stop_x80.store(true, Ordering::SeqCst);
    }
}