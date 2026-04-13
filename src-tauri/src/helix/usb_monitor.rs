// ===========================================================
// helix/usb_monitor.rs
// Surveillance branchement/débranchement USB du HX Stomp XL
// Équivalent de UsbMonitor dans kempline
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::helix::HelixState;

// Identifiants USB du HX Stomp XL
const HX_VID: u16 = 0x0e41;
const HX_PID: u16 = 0x4253;

// Intervalle de polling (kempline : POLLING_INTERVAL_IN_SEC = 1)
const POLL_INTERVAL_MS: u64 = 1000;

pub fn start_monitor(
    state:        Arc<Mutex<HelixState>>,
    stop:         Arc<AtomicBool>,
    on_connected: Arc<dyn Fn() + Send + Sync>,
    on_lost:      Arc<dyn Fn() + Send + Sync>,
) {
    thread::spawn(move || {
        println!("[UsbMonitor] démarré, surveillance VID={:#06x} PID={:#06x}", HX_VID, HX_PID);

        let mut was_connected = false;

        loop {
            if stop.load(Ordering::SeqCst) {
                println!("[UsbMonitor] arrêt demandé");
                break;
            }

            // Chercher le HX parmi les devices USB connectés
            let found = rusb::devices()
                .ok()
                .and_then(|list| {
                    list.iter().find(|d| {
                        d.device_descriptor()
                            .map(|desc| desc.vendor_id() == HX_VID && desc.product_id() == HX_PID)
                            .unwrap_or(false)
                    })
                    .map(|_| true)
                })
                .unwrap_or(false);

            match (was_connected, found) {
                // HX vient d'être branché
                (false, true) => {
                    println!("[UsbMonitor] HX Stomp XL détecté");
                    was_connected = true;
                    on_connected();
                }
                // HX vient d'être débranché
                (true, false) => {
                    println!("[UsbMonitor] HX Stomp XL déconnecté");
                    was_connected = false;
                    {
                        let mut s = state.lock().unwrap();
                        s.connected = false;
                    }
                    on_lost();
                }
                // Pas de changement
                _ => {}
            }

            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }

        println!("[UsbMonitor] thread arrêté");
    });
}