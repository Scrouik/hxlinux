// Grand échange « éditeur » observé sur HX Edit entre subscribe f0 et le polling régulier
// (`Start_Model_change.json` : 3× OUT `19` ed puis 1× OUT `1a` ef).
//
// Double transaction (octets 28–29 après `83:66:cd:03`) sur les OUT :
//   19 #1 → e8:64   19 #2 → ea:64 (+2 sur le 1er octet, pas e9:64)   19 #3 → eb:64
//   1a     → e8:64 réutilisé (pas de 4ᵉ incrément — HX Edit ne met pas ec:64 ici).
// Ensuite RequestPreset continue depuis editor_ed03_double == 0x64ec.
//
// Lane : editor_ed03_double (0x64xx) — distinct de preset_dump_ack_ctr et live_write_ctr.

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

const INTER_PACKET_DELAY_MS: u64 = 20;

/// Envoie les requêtes longues `ed` / `ef` qui amorcent l'état preset + liste (phase 4).
pub fn send(state: &mut HelixState) {
    crate::helix::init_trace::trace("editor_phase4_bootstrap BEGIN (3×19 ed + 1a ef)");
    // Repositionne editor_ed03_double à 0x64e7 → prochain next() = 0x64e8
    state.editor_ed03_double = HelixState::PRESET_ED03_TRANSACTION_FIRST.wrapping_sub(1);

    // 19 #1 — wire e8:64 (0x64e8)
    let d_e8 = state.next_editor_ed03_double();
    let c0 = state.next_x80_cnt();
    state.send(OutPacket::with_delay(
        vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, c0, 0x00, 0x04,
            0x09, 0x10, 0x00, 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d_e8[0], d_e8[1], 0x4c, 0x65,
            0x80, 0x00, 0x00, 0x00,
        ],
        INTER_PACKET_DELAY_MS,
    ));

    // 19 #2 — wire ea:64 (0x64ea) : HX saute e9:64 entre e8 et ea
    state.editor_ed03_double = 0x64ea;
    let d_ea = state.editor_ed03_double_val();
    let c1 = state.next_x80_cnt();
    state.send(OutPacket::with_delay(
        vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, c1, 0x00, 0x0c,
            0x6c, 0x10, 0x00, 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d_ea[0], d_ea[1], 0x17, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ],
        INTER_PACKET_DELAY_MS,
    ));

    // 19 #3 — wire eb:64 (0x64eb), compteur → 0x64ec pour la suite
    let d_eb = state.next_editor_ed03_double();
    let c2 = state.next_x80_cnt();
    state.send(OutPacket::with_delay(
        vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, c2, 0x00, 0x0c,
            0x9d, 0x10, 0x00, 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d_eb[0], d_eb[1], 0x16, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ],
        INTER_PACKET_DELAY_MS,
    ));

    // 1a ef — wire e8:64 encore (même double que #1, pas de next)
    let cx = state.next_x1_cnt();
    state.send(OutPacket::with_delay(
        vec![
            0x1a, 0x00, 0x00, 0x18,
            0x01, 0x10, 0xef, 0x03,
            0x00, cx, 0x00, 0x04,
            0x09, 0x10, 0x00, 0x00,
            0x01, 0x00, 0x02, 0x00,
            0x0a, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d_e8[0], d_e8[1], 0xcc, 0xfe,
            0x65, 0x80, 0x00, 0x00,
        ],
        INTER_PACKET_DELAY_MS,
    ));
}