// ===========================================================
// helix/usb_writer.rs
// Thread d'écriture vers le HX sur endpoint 0x01
// Tous les modules envoient via state.send(pkt)
// Ce thread est le seul à toucher l'USB en écriture
// ===========================================================

use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;
use rusb::DeviceHandle;
use rusb::GlobalContext;

use crate::helix::packet::OutPacket;

const ENDPOINT_OUT: u8 = 0x01;
const WRITE_TIMEOUT_MS: u64 = 1000;

pub fn start_writer(
    handle: std::sync::Arc<DeviceHandle<GlobalContext>>,
    rx: Receiver<OutPacket>,
) {
    thread::spawn(move || {
        println!("[UsbWriter] thread démarré");

        loop {
            // On attend le prochain paquet à envoyer
            match rx.recv() {
                Ok(pkt) => {
                    // Délai éventuel avant envoi (kempline : delay=0.140)
                    if pkt.delay_ms > 0 {
                        thread::sleep(Duration::from_millis(pkt.delay_ms));
                    }

                    // Envoi sur endpoint 0x01
                    match handle.write_bulk(
                        ENDPOINT_OUT,
                        &pkt.data,
                        Duration::from_millis(WRITE_TIMEOUT_MS),
                    ) {
                        Ok(n) => {
                            println!("[UsbWriter] envoyé {} bytes : {:02x?}", n, pkt.data);
                        }
                        Err(e) => {
                            println!("[UsbWriter] erreur écriture : {}", e);
                        }
                    }
                }
                Err(_) => {
                    // Le channel est fermé → on arrête le thread
                    println!("[UsbWriter] channel fermé, arrêt");
                    break;
                }
            }
        }
    });
}