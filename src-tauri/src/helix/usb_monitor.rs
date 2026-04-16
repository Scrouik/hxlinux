// ===========================================================
// helix/usb_monitor.rs
// Surveillance branchement/débranchement des devices HX supportés
// Équivalent de UsbMonitor dans kempline
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::helix::HelixState;

const SUPPORTED_DEVICES: &[(u16, u16)] = &[
    (0x0e41, 0x4253), // HX Stomp XL
    (0x0e41, 0x4246), // HX Stomp
    (0x0e41, 0x4248), // Helix Floor
    (0x0e41, 0x424a), // Helix LT
];

// Intervalle de polling (kempline : POLLING_INTERVAL_IN_SEC = 1)
const POLL_INTERVAL_MS: u64 = 1000;

pub fn start_monitor(
    state:        Arc<Mutex<HelixState>>,
    stop:         Arc<AtomicBool>,
    on_connected: Arc<dyn Fn() + Send + Sync>,
    on_lost:      Arc<dyn Fn() + Send + Sync>,
) {
    thread::spawn(move || {
        let mut was_connected = false;

        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }

            // Chercher le HX parmi les devices USB connectés
            let found = rusb::devices()
                .ok()
                .and_then(|list| {
                    list.iter().find(|d| {
                        d.device_descriptor()
                            .map(|desc| {
                                SUPPORTED_DEVICES
                                    .iter()
                                    .any(|(vid, pid)| desc.vendor_id() == *vid && desc.product_id() == *pid)
                            })
                            .unwrap_or(false)
                    })
                    .map(|_| true)
                })
                .unwrap_or(false);

            match (was_connected, found) {
                // HX vient d'être branché
                (false, true) => {
                    was_connected = true;
                    on_connected();
                }
                // HX vient d'être débranché
                (true, false) => {
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
    });
}