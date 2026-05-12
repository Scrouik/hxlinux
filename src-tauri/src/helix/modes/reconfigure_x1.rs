use crate::helix::{Mode, HelixState, ModeRequest, KeepAliveCommand};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;

pub struct ReconfigureX1;

impl ReconfigureX1 {
    pub fn new() -> Self { Self }
}

impl Mode for ReconfigureX1 {

    fn start(&mut self, state: &mut HelixState) {
        // Kempline : self.helix_usb.x1x10_cnt = 0x2
        state.x1_cnt = 0x02;

        // Même paquet que le début de Connect
        let pkt = OutPacket::new(vec![
            0x0c, 0x00, 0x00, 0x28,
            0x01, 0x10, 0xef, 0x03,
            0x00, 0x00, 0x00, 0x02,
            0x00, 0x01, 0x00, 0x21,
            0x00, 0x10, 0x00, 0x00,
        ]);
        state.send(pkt);
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {

        // Réponse init x1 → ack avec compteur 0x02
        if byte_cmp(data, &pattern![
            0x0c, 0x00, 0x00, 0x28,
            0xef, 0x03, 0x01, 0x10,
            0x00, 0x00, 0x00, 0x02,
            0x00, 0x01, 0x00, 0x01,
            0x00, 0x02, 0x00, 0x00
        ], 20) {
            let cnt = state.next_x1_cnt(); // retourne 0x02
            let pkt = OutPacket::new(vec![
                0x11, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, cnt,  0x00, 0x04,
                0x00, 0x10, 0x00, 0x00,
                0x01, 0x00, 0x02, 0x00,
                0x01, 0x00, 0x00, 0x00,
                0x02, 0x00, 0x00, 0x00,
            ]);
            state.send(pkt);

        // Réponse finale → démarrer keep-alive x1 et switcher
        } else if byte_cmp(data, &pattern![
            0x11, 0x00, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, 0x02, 0x00, 0x04
        ], 12) {
            // Phase 4 HX Edit : requêtes `19` ed + `1a` ef avant le polling `08…f0:03` régulier.
            crate::helix::editor_phase4_bootstrap::send(state);
            state.start_keepalive(KeepAliveCommand::StartOrdered);
            state.switch_mode(ModeRequest::RequestPresetName);

        } else if Standard::check_keep_alive(data, state) {
            return false;
        }

        true
    }

    fn shutdown(&mut self, _state: &mut HelixState) {
    }
}