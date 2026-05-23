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

fn device_visible() -> bool {
    rusb::devices()
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
        .unwrap_or(false)
}

pub fn start_monitor(
    state: Arc<Mutex<HelixState>>,
    stop: Arc<AtomicBool>,
    // `true` seulement après `start_helix` a ouvert le USB avec succès.
    helix_attached: Arc<AtomicBool>,
    // `true` pendant qu’un `start_helix` est en cours (évite les doublons).
    helix_connecting: Arc<AtomicBool>,
    on_connected: Arc<dyn Fn() + Send + Sync>,
    on_lost: Arc<dyn Fn() + Send + Sync>,
) {
    thread::spawn(move || {
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }

            let found = device_visible();
            let attached = helix_attached.load(Ordering::SeqCst);
            let connecting = helix_connecting.load(Ordering::SeqCst);

            if !found {
                if attached {
                    helix_attached.store(false, Ordering::SeqCst);
                    helix_connecting.store(false, Ordering::SeqCst);
                    {
                        let mut s = state.lock().unwrap();
                        s.connected = false;
                    }
                    on_lost();
                }
            } else if !attached && !connecting {
                // Visible mais pas encore de session (ou échec open précédent) → (re)lancer.
                if helix_connecting
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    on_connected();
                }
            }

            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }
    });
}
