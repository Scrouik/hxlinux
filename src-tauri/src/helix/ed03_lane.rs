//! Patches compteur lane `80:10:ed:03` (focus / ed:08 avant bulk replace).

use crate::helix::HelixState;

/// Patche les octets 12-13 (ctr LE) et 14 (=0) d'un paquet `80:10:ed:03`.
pub(crate) fn force_ed03_ctr(pkt: &mut [u8], ctr: u16) {
    if pkt.len() > 14 {
        pkt[12] = (ctr & 0xff) as u8;
        pkt[13] = ((ctr >> 8) & 0xff) as u8;
        pkt[14] = 0x00;
    }
}

/// Court `08 … 80:10:ed:03` (ed:08), ctr sur les octets 12-13.
pub(crate) fn build_ed08_short(state: &mut HelixState, ctr: u16) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    vec![
        0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, seq, 0x00, 0x08,
        (ctr & 0xff) as u8,
        ((ctr >> 8) & 0xff) as u8,
        0x00,
        0x00,
    ]
}
