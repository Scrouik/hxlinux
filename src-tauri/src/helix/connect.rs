use crate::helix::usb::HelixUsb;
use crate::helix::protocol::*;

pub fn connect_sequence(helix: &HelixUsb) -> Result<(), rusb::Error> {
    // Envoi du message de connexion initial
    helix.write(&CONNECT_INIT)?;
    
    let mut buf = [0u8; 64];
    let mut received_x11_on_x2 = false;
    let mut received_x11_on_x80 = false;
    
    // Boucle de handshake
    loop {
        match helix.read(&mut buf) {
            Ok(n) => {
                let data = &buf[..n];
                
                // Détection x11 sur canal x2
                if n >= 10 && data[4] == 0xf0 && data[6] == 0x02 && data[9] == 0x02 {
                    received_x11_on_x2 = true;
                }
                
                // Détection x11 sur canal x80
                if n >= 10 && data[4] == 0xed && data[6] == 0x80 && data[9] == 0x02 {
                    received_x11_on_x80 = true;
                }
                
                // Connexion établie
                if received_x11_on_x2 && received_x11_on_x80 {
                    return Ok(());
                }
            }
            Err(rusb::Error::Timeout) => continue,
            Err(e) => return Err(e),
        }
    }
}