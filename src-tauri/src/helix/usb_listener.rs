// ===========================================================
// helix/usb_listener.rs
// Thread de lecture en continu sur endpoint 0x81
// Dispatch chaque paquet vers le mode actif
// C'est le chef d'orchestre de toute la machine à états
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use rusb::DeviceHandle;
use rusb::GlobalContext;

use crate::helix::{HelixState, Mode};

const ENDPOINT_IN: u8 = 0x81;
const READ_TIMEOUT_MS: u64 = 500;
const BUFFER_SIZE: usize = 512;

pub fn start_listener(
    handle: Arc<DeviceHandle<GlobalContext>>,
    state:  Arc<Mutex<HelixState>>,
    mode:   Arc<Mutex<Box<dyn Mode>>>,
    stop:   Arc<AtomicBool>,
) {
    thread::spawn(move || {
        println!("[UsbListener] thread démarré");
        let mut buf = vec![0u8; BUFFER_SIZE];

        loop {
            // Vérifier si on doit s'arrêter
            if stop.load(Ordering::SeqCst) {
                println!("[UsbListener] arrêt demandé");
                break;
            }

            // Lire depuis l'endpoint 0x81
            match handle.read_bulk(
                ENDPOINT_IN,
                &mut buf,
                Duration::from_millis(READ_TIMEOUT_MS),
            ) {
                Ok(n) if n > 0 => {
                    let data = buf[..n].to_vec();
                    // Log temporaire - TOUS les paquets
                    if data.len() > 6 && data[4] == 0xf0 {
                        println!("[UsbListener RAW x2] {} bytes : {:02x?}", n, data);
                    }
                    println!("[UsbListener] reçu {} bytes : {:02x?}", n, data);

                    // Dispatcher vers le mode actif
                    // On lock state et mode séparément pour éviter deadlock
                    let mut s = state.lock().unwrap();
                    let mut m = mode.lock().unwrap();
                    m.data_in(&data, &mut s);
                }
                Ok(_) => {
                    // 0 bytes reçus — on continue
                }
                Err(rusb::Error::Timeout) => {
                    // Timeout normal — on reboucle pour vérifier stop
                }
                Err(rusb::Error::NoDevice) => {
                    println!("[UsbListener] HX déconnecté");
                    let mut s = state.lock().unwrap();
                    s.connected = false;
                    break;
                }
                Err(e) => {
                    println!("[UsbListener] erreur lecture : {}", e);
                    break;
                }
            }
        }

        println!("[UsbListener] thread arrêté");
    });
}