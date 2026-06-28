// ===========================================================
// helix/editor_go_live.rs
// « Go live » de l'éditeur : 2 commandes ED03 sur lane ef03 que HX Edit
// envoie APRÈS la lecture des noms (RequestPresetNames) et AVANT la lecture
// du preset actif (cd:04). Décodé sur 4 captures HX (stomp_running_start_hxedit*).
//
// Hypothèse (à valider sur HW) : ces 2 commandes font passer le Stomp en
// « mode éditeur actif » — après elles, HX reçoit un flux continu de IN 1d de
// fond et le scroll modèle DUMPE (IN 53/54). Sans elles, notre Stomp reste
// quasi silencieux (1 seul IN 1d) et le pull scroll est ignoré.
//
// GARDE : tout l'envoi est conditionné par la variable d'environnement
// HX_EDITOR_GO_LIVE=1. Désactivé par défaut → comportement inchangé, aucun
// octet supplémentaire envoyé au device tant que le flag n'est pas posé.
//
// === Structure décodée (constante sauf compteurs) ===
//   GO_LIVE #1 (head 19) :
//     19 00 00 18 | 01 10 ef 03 | 00 CNT 00 0c | LANE_lo LANE_hi 00 00
//     | 01 00 02 00 09 00 00 00 | 83 66 cd 03 | DBL 64 | 70 65 c0 00 00 00
//   GO_LIVE #2 (head 1b) :
//     1b 00 00 18 | 01 10 ef 03 | 00 CNT 00 0c | LANE_lo LANE_hi 00 00
//     | 01 00 02 00 0b 00 00 00 | 83 66 cd 03 | DBL 64 | 0d 65 81 65 02 00
//
// Compteurs (dérivés de NOTRE état, pas hardcodés — device tolérant sur le lo,
// prouvé en PHASE B où notre lo divergeait de HX et était quand même accepté) :
//   CNT      = next_x1_cnt() (lane ef03, comme les ACK noms)
//   LANE_lo  = continuité +0x11 par commande sur la lane ef03
//   LANE_hi  = hi courant ef03 (figé depuis les ACK noms ; HX = 0x1d)
//   DBL      = next_editor_ed03_double() : ea (noms) → eb → ec
// ===========================================================

use crate::helix::{HelixState, packet::OutPacket};

/// Pas du lo ef03 entre deux commandes go-live (incrément éditeur standard).
const GO_LIVE_LANE_LO_DELTA: u8 = 0x11;

/// `true` si l'étape go-live est activée (variable d'environnement HX_EDITOR_GO_LIVE).
pub fn editor_go_live_enabled() -> bool {
    std::env::var_os("HX_EDITOR_GO_LIVE").is_some_and(|v| {
        let s = v.to_string_lossy();
        !s.is_empty() && s != "0" && !s.eq_ignore_ascii_case("false")
    })
}

/// Construit GO_LIVE #1 (head 0x19) à partir de l'état courant.
fn build_go_live_19(state: &mut HelixState, lane_lo: u8, lane_hi: u8) -> Vec<u8> {
    let cnt = state.next_x1_cnt();
    let dbl = state.next_editor_ed03_double(); // ea → eb (1re commande go-live)
    vec![
        0x19, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt, 0x00, 0x0c,
        lane_lo, lane_hi, 0x00, 0x00,
        0x01, 0x00, 0x02, 0x00,
        0x09, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        dbl[0], dbl[1], 0x70, 0x65,
        0xc0, 0x00, 0x00, 0x00,
    ]
}

/// Construit GO_LIVE #2 (head 0x1b) à partir de l'état courant.
fn build_go_live_1b(state: &mut HelixState, lane_lo: u8, lane_hi: u8) -> Vec<u8> {
    let cnt = state.next_x1_cnt();
    let dbl = state.next_editor_ed03_double(); // eb → ec (2e commande go-live)
    vec![
        0x1b, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt, 0x00, 0x0c,
        lane_lo, lane_hi, 0x00, 0x00,
        0x01, 0x00, 0x02, 0x00,
        0x0b, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        dbl[0], dbl[1], 0x0d, 0x65,
        0x81, 0x65, 0x02, 0x00,
    ]
}

/// Envoie les 2 commandes go-live si HX_EDITOR_GO_LIVE est posé.
/// À appeler APRÈS la finalisation des noms, AVANT RequestPresetName (preset actif).
///
/// Le lo de la lane ef03 est pris en continuité (+0x11 par commande) depuis
/// `editor_ed03_lane` ; le hi courant (figé depuis les ACK noms) est conservé.
/// Renvoie `true` si les commandes ont été envoyées.
pub fn send_if_enabled(state: &mut HelixState) -> bool {
    if !editor_go_live_enabled() {
        return false;
    }

    let lane = state.editor_ed03_lane_bytes();
    let hi = lane[1];
    // 1re commande : émet le lo courant, puis on avance le lo de +0x11 pour la suivante.
    let lo1 = lane[0];
    let lo2 = lo1.wrapping_add(GO_LIVE_LANE_LO_DELTA);

    crate::helix::init_trace::trace_fmt(format_args!(
        "EditorGoLive ENABLED — 2 commandes ef03 (lane hi={:02x}, lo1={:02x} lo2={:02x})",
        hi, lo1, lo2
    ));

    let pkt1 = build_go_live_19(state, lo1, hi);
    crate::helix::init_trace::trace_out(&pkt1, "EditorGoLive #1 (19 ef03)");
    state.send(OutPacket::new(pkt1));

    let pkt2 = build_go_live_1b(state, lo2, hi);
    crate::helix::init_trace::trace_out(&pkt2, "EditorGoLive #2 (1b ef03)");
    state.send(OutPacket::new(pkt2));

    // Refléter dans editor_ed03_lane l'avancée du lo (2 commandes → +0x22),
    // pour que d'éventuelles commandes ef03 ultérieures restent en continuité.
    let new_lo = lo2.wrapping_add(GO_LIVE_LANE_LO_DELTA);
    state.editor_ed03_lane = (state.editor_ed03_lane & 0xff00) | (new_lo as u16);

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::HelixState;

    #[test]
    fn packets_have_expected_structure() {
        let mut s = HelixState::new();
        let p1 = build_go_live_19(&mut s, 0xbe, 0x1d);
        assert_eq!(p1.len(), 36);
        assert_eq!(&p1[0..8], &[0x19, 0x00, 0x00, 0x18, 0x01, 0x10, 0xef, 0x03]);
        assert_eq!(&p1[10..12], &[0x00, 0x0c]);
        assert_eq!(&p1[12..14], &[0xbe, 0x1d]);
        assert_eq!(&p1[24..28], &[0x83, 0x66, 0xcd, 0x03]);
        assert_eq!(p1[29], 0x64);
        assert_eq!(&p1[30..36], &[0x70, 0x65, 0xc0, 0x00, 0x00, 0x00]);

        let p2 = build_go_live_1b(&mut s, 0xcf, 0x1d);
        assert_eq!(p2.len(), 36);
        assert_eq!(p2[0], 0x1b);
        assert_eq!(&p2[12..14], &[0xcf, 0x1d]);
        assert_eq!(&p2[30..36], &[0x0d, 0x65, 0x81, 0x65, 0x02, 0x00]);
    }

    /// Sans le flag, aucun envoi (et l'état n'est pas modifié).
    #[test]
    fn disabled_by_default_no_send() {
        // Pas de flag posé dans l'env de test → send_if_enabled renvoie false.
        let mut s = HelixState::new();
        let lane_before = s.editor_ed03_lane;
        let sent = send_if_enabled(&mut s);
        assert!(!sent, "go-live ne doit rien envoyer sans HX_EDITOR_GO_LIVE");
        assert_eq!(s.editor_ed03_lane, lane_before);
    }

    /// Le double avance ea → eb → ec sur les deux commandes (continuité editor_ed03_double).
    #[test]
    fn double_advances_eb_then_ec() {
        let mut s = HelixState::new();
        // Positionner le double comme en sortie de RequestPresetNames (ea:64).
        s.editor_ed03_double = 0x64ea;
        let p1 = build_go_live_19(&mut s, 0xbe, 0x1d);
        assert_eq!(&p1[28..30], &[0xeb, 0x64], "GO_LIVE #1 double = eb:64");
        let p2 = build_go_live_1b(&mut s, 0xcf, 0x1d);
        assert_eq!(&p2[28..30], &[0xec, 0x64], "GO_LIVE #2 double = ec:64");
    }
}