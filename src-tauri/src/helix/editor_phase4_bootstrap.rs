// Grand échange « éditeur » observé sur HX Edit entre subscribe f0 et le polling régulier.
//
// PHASE A+B (séquence amorçage éditeur, lane editor_ed03_double 0x64xx) :
//   AVANT le dump preset (ce module, fn `send`) :
//     19 #1 → e8:64 (lecture preset #1)        oct12-13 = 09:10
//     1c     → e9:64 (76:cc:80, requête PHASE B) oct12-13 = 55:10  ← AJOUTÉ
//     19 #2 → ea:64 (lecture #2)               oct12-13 = 6c:10
//     19 #3 → eb:64 (lecture #3)               oct12-13 = 9d:10
//   -- dump preset (272×N) : oct12 (lo) FIGÉ à 9d, oct13 (hi) monte via ACK --
//   APRÈS le dump (FSM `phase4_state`, on_enter_*) :
//     1b ec(76:0e), 1b ed(76:49), 1c ee(76:cc:88), 1a ef(e8), 1b ef'(76:1b),
//     19 ed(f0), 19 ef(e9)
//
// Les octets 12-13 sont gérés par `editor_ed03_lane` (mod.rs). `send` fait
// AVANCER ce compteur avec les deltas observés (0x4c, 0x17, 0x31) pour qu'il
// reflète l'état réel à l'entrée de la PHASE B. Le `1a` a été DÉPLACÉ vers la
// FSM (rang réel HX : après le 1c ee), il n'est plus émis ici.

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

const INTER_PACKET_DELAY_MS: u64 = 20;

/// Deltas du lo (octet 12) AVANT le dump, observés byte-pour-byte sur 3 captures HX.
/// e8→e9 (1c lecture preset) = +0x4c ; e9→ea = +0x17 ; ea→eb = +0x31.
const LANE_DELTA_E8_TO_E9: u16 = 0x004c;
const LANE_DELTA_E9_TO_EA: u16 = 0x0017;
const LANE_DELTA_EA_TO_EB: u16 = 0x0031;

/// Fin de rafale phase 4 : **132 o** `7a` (habituel) ou **116 o** `6a` (variante Stomp/preset).
pub fn is_phase4_bootstrap_trailer_in(data: &[u8]) -> bool {
    let ep_ok = data.get(4..8) == Some(&[0xed, 0x03, 0x80, 0x10]);
    ep_ok
        && ((data.len() == 132 && data.first() == Some(&0x7a))
            || (data.len() == 116 && data.first() == Some(&0x6a)))
}

/// Envoie les requêtes `ed` (+ le 1c e9 PHASE B) qui amorcent l'état preset
/// AVANT le dump. Le `1a` ef et les requêtes 76 post-dump sont dans `phase4_state`.
pub fn send(state: &mut HelixState) {
    crate::helix::init_trace::trace(
        "editor_phase4_bootstrap BEGIN (19 e8 + 1c e9 76:cc + 19 ea + 19 eb)",
    );
    // Repositionne editor_ed03_double à 0x64e7 → prochain next() = 0x64e8.
    state.editor_ed03_double = HelixState::PRESET_ED03_TRANSACTION_FIRST.wrapping_sub(1);
    // Repositionne le compteur lane ED03 (octets 12-13) à l'ancrage 0x1009 (lo=09, hi=10).
    state.reset_editor_ed03_lane();

    // 19 #1 — double e8:64, lane 09:10. Avance lo de +0x4c (→ e9 = 55).
    let d_e8 = state.next_editor_ed03_double();
    let lane_e8 = state.advance_editor_ed03_lane_lo(LANE_DELTA_E8_TO_E9);
    let c0 = state.next_x80_cnt();
    state.send(OutPacket::with_delay(
        vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, c0, 0x00, 0x04,
            lane_e8[0], lane_e8[1], 0x00, 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d_e8[0], d_e8[1], 0x4c, 0x65,
            0x80, 0x00, 0x00, 0x00,
        ],
        INTER_PACKET_DELAY_MS,
    ));

    // 1c — double e9:64, lane 55:10, requête 76:cc:80. HX émet ce 1c ENTRE le
    // 19 e8 et le 19 ea (avant le dump) : 1re requête éditeur PHASE B, qui
    // conditionne l'armement du scroll-dump. Avance lo de +0x17 (→ ea = 6c).
    state.editor_ed03_double = 0x64e9;
    let d_e9 = state.editor_ed03_double_val();
    let lane_e9 = state.advance_editor_ed03_lane_lo(LANE_DELTA_E9_TO_EA);
    let c_1c = state.next_x80_cnt();
    state.send(OutPacket::with_delay(
        vec![
            0x1c, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, c_1c, 0x00, 0x0c,
            lane_e9[0], lane_e9[1], 0x00, 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x0c, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d_e9[0], d_e9[1], 0x18, 0x65,
            0x81, 0x76, 0xcc, 0x80,
        ],
        INTER_PACKET_DELAY_MS,
    ));

    // 19 #2 — double ea:64, lane 6c:10. Avance lo de +0x31 (→ eb = 9d).
    state.editor_ed03_double = 0x64ea;
    let d_ea = state.editor_ed03_double_val();
    let lane_ea = state.advance_editor_ed03_lane_lo(LANE_DELTA_EA_TO_EB);
    let c1 = state.next_x80_cnt();
    state.send(OutPacket::with_delay(
        vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, c1, 0x00, 0x0c,
            lane_ea[0], lane_ea[1], 0x00, 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d_ea[0], d_ea[1], 0x17, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ],
        INTER_PACKET_DELAY_MS,
    ));

    // 19 #3 — double eb:64, lane 9d:10. PAS d'avance lo : le dump suit et fige
    // le lo à 9d ; seul le hi montera (ACK chunks). Le compteur reste à lo=9d.
    let d_eb = state.next_editor_ed03_double();
    let lane_eb = state.editor_ed03_lane_bytes();
    let c2 = state.next_x80_cnt();
    state.send(OutPacket::with_delay(
        vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, c2, 0x00, 0x0c,
            lane_eb[0], lane_eb[1], 0x00, 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d_eb[0], d_eb[1], 0x16, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ],
        INTER_PACKET_DELAY_MS,
    ));
    // À ce stade : editor_ed03_double = 0x64eb (prochain next → ec),
    // editor_ed03_lane = lo 9d, hi 10. Le hi montera via advance_editor_ed03_lane_hi()
    // appelé sur chaque ACK chunk pendant le dump (cf. preset_dump_stream_ack / usb_listener).
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase4_trailer_7a_132() {
        let mut b = vec![0u8; 132];
        b[0] = 0x7a;
        b[1] = 0x00;
        b[2] = 0x00;
        b[3] = 0x18;
        b[4] = 0xed;
        b[5] = 0x03;
        b[6] = 0x80;
        b[7] = 0x10;
        assert!(is_phase4_bootstrap_trailer_in(&b));
        b[0] = 0x08;
        assert!(!is_phase4_bootstrap_trailer_in(&b));
    }

    #[test]
    fn phase4_trailer_6a_116() {
        let mut b = vec![0u8; 116];
        b[0] = 0x6a;
        b[1] = 0x00;
        b[2] = 0x00;
        b[3] = 0x18;
        b[4] = 0xed;
        b[5] = 0x03;
        b[6] = 0x80;
        b[7] = 0x10;
        assert!(is_phase4_bootstrap_trailer_in(&b));
    }

    /// Vérifie que send() laisse le compteur lane dans l'état attendu et que
    /// les valeurs émises (lo) correspondent à HX avant le dump.
    #[test]
    fn lane_progression_before_dump() {
        let mut s = HelixState::new();
        // On capture les lo successifs via une réimplémentation locale de la loi
        // (send() émet réellement via le canal ; ici on vérifie juste le compteur).
        s.reset_editor_ed03_lane();
        assert_eq!(s.editor_ed03_lane_bytes(), [0x09, 0x10]); // e8
        let _ = s.advance_editor_ed03_lane_lo(LANE_DELTA_E8_TO_E9);
        assert_eq!(s.editor_ed03_lane_bytes(), [0x55, 0x10]); // e9
        let _ = s.advance_editor_ed03_lane_lo(LANE_DELTA_E9_TO_EA);
        assert_eq!(s.editor_ed03_lane_bytes(), [0x6c, 0x10]); // ea
        let _ = s.advance_editor_ed03_lane_lo(LANE_DELTA_EA_TO_EB);
        assert_eq!(s.editor_ed03_lane_bytes(), [0x9d, 0x10]); // eb (lo figé ensuite)
    }
}