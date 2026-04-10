// HXLinux USB Architecture
//
// The HX Stomp XL USB protocol requires continuous keep-alive responses on three
// separate channels (x1/0xef, x2/0xf0, x80/0xed) to maintain an active connection.
// If keep-alive messages go unanswered, the device stops responding to any requests.
//
// To handle this, we use a dedicated listener thread that continuously reads from
// the USB Bulk IN endpoint (0x81) and dispatches events via an mpsc channel.
// The main thread writes to the USB Bulk OUT endpoint (0x01) independently.
//
// This is safe because USB Bulk IN (0x81) and Bulk OUT (0x01) are physically
// separate endpoints with independent buffers — simultaneous read/write operations
// on different endpoints do not interfere at the hardware level, and libusb is
// documented as thread-safe for operations on different endpoints.
//
// The event-driven architecture also allows the UI to react in real-time to
// any changes made directly on the device (preset switches, parameter changes),
// keeping the interface always in sync with the hardware state.


// Si tu ne comprends pas utilises Deepl :)

use rusb::{Context, DeviceHandle, UsbContext};
use crate::helix::protocol::*;
use std::sync::Arc;
use std::time::Duration;
use std::sync::mpsc;

pub enum HelixEvent {
    KeepAliveX1 { counter: u8 },
    KeepAliveX80 { counter: u8, ack: u8 },
    PresetChunk(Vec<u8>),
    PresetHeader,
    PresetNamesData(Vec<u8>),
    RawMessage(Vec<u8>),
    Disconnected,
}

pub struct HelixUsb {
    handle: Arc<DeviceHandle<Context>>,
}

impl HelixUsb {

    pub fn connect() -> Result<Self, rusb::Error> {
        let context = Context::new()?;
        
        let mut handle = context
            .open_device_with_vid_pid(VENDOR_ID, PRODUCT_ID)
            .ok_or(rusb::Error::NoDevice)?;

        if handle.kernel_driver_active(0)? {
            handle.detach_kernel_driver(0)?;
        }
        handle.claim_interface(0)?;

        Ok(HelixUsb {
            handle: Arc::new(handle),
        })
    }

    pub fn write(&self, data: &[u8]) -> Result<(), rusb::Error> {
        let timeout = Duration::from_millis(5000);
        self.handle.write_bulk(ENDPOINT_BULK_OUT, data, timeout)?;
        Ok(())
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize, rusb::Error> {
        let timeout = Duration::from_millis(2000);
        let n = self.handle.read_bulk(ENDPOINT_BULK_IN, buf, timeout)?;
        Ok(n)
    }

    pub fn flush(&self) {
        let mut buf = [0u8; 512];
        let timeout = Duration::from_millis(100);
        loop {
            match self.handle.read_bulk(ENDPOINT_BULK_IN, &mut buf, timeout) {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
    }

    pub fn start_listener(&self) -> mpsc::Receiver<HelixEvent> {
        let (tx, rx) = mpsc::channel();
        let handle = Arc::clone(&self.handle);

        std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            loop {
                match handle.read_bulk(ENDPOINT_BULK_IN, &mut buf, Duration::from_millis(5000)) {
                    Ok(n) if n >= 10 => {
                        let data = buf[..n].to_vec();

                        // Keep-alive x1
                        if data[4] == 0xef && data[6] == 0x01 
                            && (data[11] == 0x10 || data[11] == 0x08) {
                            let _ = tx.send(HelixEvent::KeepAliveX1 { 
                                counter: data[9] 
                            });
                        }

                        // Keep-alive x80
                        else if data[4] == 0xed && data[6] == 0x80 && (data[11] == 0x10) {
                            let _ = tx.send(HelixEvent::KeepAliveX80 { 
                                counter: data[9],
                                ack: data[12],
                            });
                        }

                        // Chunk preset x80
                        else if data[4] == 0xed && data[6] == 0x80 && data[1] == 0x01 {
                            let _ = tx.send(HelixEvent::PresetChunk(data));
                        }

                        // Header preset 0x39
                        else if (data[0] == 0x39 || data[0] == 0x3c) && data[4] == 0xed && data[6] == 0x80 {
                            let _ = tx.send(HelixEvent::PresetHeader);
                        }

                        // Données noms de presets x1 — paquet avec data[1]==0x01
                        else if data[4] == 0xef && data[6] == 0x01 && data[1] == 0x01 && n > 16 {
                            println!("LISTENER: PresetNamesData détecté ! {} octets", n);
                            let _ = tx.send(HelixEvent::PresetNamesData(data));
                        }
                        else {
                            println!("LISTENER RAW: data[0]={:02x} data[1]={:02x} data[4]={:02x} data[6]={:02x} data[11]={:02x} n={}", 
                                data[0], data[1], data[4], data[6], 
                                if n>11 {data[11]} else {0}, n);
                            let _ = tx.send(HelixEvent::RawMessage(data));
                        }
                    }
                    Ok(_) => continue,
                    Err(rusb::Error::Timeout) => continue,
                    Err(_) => {
                        let _ = tx.send(HelixEvent::Disconnected);
                        break;
                    }
                }
            }
        });

        rx
    }
}
