//! Écriture live des champs d'assignation Command Center (onglet « Controllers »).
//!
//! Contrairement à la bascule actif/inactif (`slot_active_state_write.rs`), la plupart de ces
//! paquets ne portent **aucun identifiant de bus/switch** dans leur payload (`82:66:00:<champ>:...`,
//! confirmé sur la capture Type) : le device retient implicitement « quel switch est en cours
//! d'édition » à partir du dernier paquet de sélection de **Source** envoyé (`82:62:<bus>:66:<idx>`,
//! qui lui porte bien le bus). Il faut donc TOUJOURS écrire la Source en premier — avec le bon slot
//! déjà actif côté device (vérifié aussi manuellement dans HX Edit par l'user, 2026-07-09) — avant
//! d'écrire Type/Couleur/Nom/Min-Max sur ce même switch.
//!
//! Format vérifié par capture (`controllers_select_all_switch_one_by_one.json`, 8 échantillons
//! Footswitch 1-8 cohérents, ctr delta=0x57, yy delta=+2). Seul le format compact Footswitch est
//! implémenté ici ; le format long EXP Pedal/None (`...5f:05:60:cd:01:2c:4a:<N>:47:...`, voir
//! mémoire session 2026-07-07) reste à faire.

use crate::helix::HelixState;

/// Construit le paquet d'écriture de Source pour un **Footswitch** (format compact uniquement).
/// `footswitch_number` est le numéro HX Edit (1-8), converti en index 0-based sur le fil.
pub fn build_controller_source_footswitch_write_packet(
    state: &mut HelixState,
    slot_bus: u8,
    footswitch_number: u8,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;
    // `pp` fixe à 0x03 : même simplification que le reste du codebase (amp_cab_live_write.rs ne
    // gère pas non plus dynamiquement le passage à 0x04 quand `yy` boucle 0xff→0x00) — cas rare
    // pour une action manuelle d'assignation, pas un poll haute fréquence.
    let pp: u8 = 0x03;
    let term: u8 = 0x38;
    let fs_index_0based = footswitch_number.saturating_sub(1);

    let packet = vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, term, 0x65,
        0x82, 0x62, slot_bus, 0x66, fs_index_0based, 0x00, 0x00, 0x00,
    ];

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x57);
    state.live_write_yy = state.live_write_yy.wrapping_add(2);
    packet
}

/// Construit le paquet d'écriture générique `82:66:00:<field_id>:<value>` (Type/Couleur), commun
/// aux champs à valeur fixe sur 1 octet. `momentary` : `false`=Latching(`c2`), `true`=Momentary(`c3`).
fn build_controller_single_byte_field_write_packet(
    state: &mut HelixState,
    field_id: u8,
    term: u8,
    value: u8,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;
    let pp: u8 = 0x03;

    let packet = vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, term, 0x65,
        0x82, 0x66, 0x00, field_id, value, 0x00, 0x00, 0x00,
    ];

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    packet
}

/// Construit le paquet d'écriture du Type (Latching/Momentary) pour le switch actuellement en
/// contexte (cf. note de module — écrire la Source d'abord). Champ `0x41`, terme `0x3a`.
pub fn build_controller_type_write_packet(state: &mut HelixState, momentary: bool) -> Vec<u8> {
    let value = if momentary { 0xc3 } else { 0xc2 };
    build_controller_single_byte_field_write_packet(state, 0x41, 0x3a, value)
}

/// Construit le paquet d'écriture de la couleur LED (index 0-based, voir liste front
/// `CONTROLLERS_LED_COLORS`) pour le switch actuellement en contexte. Champ `0x42`, terme `0x3d`.
pub fn build_controller_color_write_packet(state: &mut HelixState, color_index: u8) -> Vec<u8> {
    build_controller_single_byte_field_write_packet(state, 0x42, 0x3d, color_index)
}

/// Construit le paquet d'écriture du nom personnalisé (Customize) pour le switch actuellement en
/// contexte. Champ `0x6d`, terme `0x3b`, valeur fixstr `<nom ASCII>\0`.
///
/// Format longueur variable déduit d'un SEUL échantillon capturé ("toto", 4 caractères) — voir
/// mémoire session 2026-07-09. La relation `byte[0] = longueur_payload + 16` est en revanche
/// confirmée sur 3 formats différents (dont 2 déjà utilisés ailleurs dans le codebase,
/// `live_write.rs::assemble_27_write`/`assemble_23_bool_write`), donc fiable. Ce qui reste
/// hypothétique : le nombre d'octets de bourrage `0x00` en fin de paquet (2, déduit du seul
/// échantillon) pour un nom d'une AUTRE longueur — à reconfirmer avant de considérer ce cas robuste.
///
/// `name` est tronqué à 30 caractères (limite fixstr msgpack 1 octet moins le terminateur nul).
pub fn build_controller_name_write_packet(state: &mut HelixState, name: &str) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;
    let pp: u8 = 0x03;
    let term: u8 = 0x3b;

    let truncated: &str = {
        let mut end = name.len().min(30);
        while end > 0 && !name.is_char_boundary(end) {
            end -= 1;
        }
        &name[..end]
    };
    let name_bytes = truncated.as_bytes();
    let fixstr_len = (name_bytes.len() + 1) as u8; // +1 terminateur nul
    let fixstr_tag = 0xa0 | fixstr_len;
    let payload_len: u8 = 8 + 4 + 1 + fixstr_len; // cd..65 + 82:66:00:6d + tag + contenu
    let byte0 = payload_len + 16;

    let mut packet: Vec<u8> = vec![
        byte0, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, payload_len, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, term, 0x65,
        0x82, 0x66, 0x00, 0x6d, fixstr_tag,
    ];
    packet.extend_from_slice(name_bytes);
    packet.push(0x00); // terminateur nul du fixstr
    packet.extend_from_slice(&[0x00, 0x00]); // bourrage final (déduit d'un seul échantillon)

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Octets réels capturés (`controllers_select_all_switch_one_by_one.json`), bus=0x0b,
    /// Footswitch 1 (pkt#440) et Footswitch 4 (pkt#1904, après un premier wrap de `yy`).
    #[test]
    fn builds_expected_bytes_for_footswitch1() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x1f65;
        state.live_write_yy = 0xf9;
        let pkt = build_controller_source_footswitch_write_packet(&mut state, 0x0b, 1);
        assert_eq!(pkt.len(), 40);
        assert_eq!(&pkt[0..8], &[0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03]);
        assert_eq!(&pkt[12..14], &[0x65, 0x1f]);
        assert_eq!(&pkt[24..30], &[0x83, 0x66, 0xcd, 0x03, 0xf9, 0x64]);
        assert_eq!(&pkt[30..32], &[0x38, 0x65]);
        assert_eq!(&pkt[32..40], &[0x82, 0x62, 0x0b, 0x66, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn builds_expected_bytes_for_footswitch4() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x206a;
        state.live_write_yy = 0xff;
        let pkt = build_controller_source_footswitch_write_packet(&mut state, 0x0b, 4);
        assert_eq!(&pkt[12..14], &[0x6a, 0x20]);
        assert_eq!(&pkt[24..30], &[0x83, 0x66, 0xcd, 0x03, 0xff, 0x64]);
        assert_eq!(&pkt[32..40], &[0x82, 0x62, 0x0b, 0x66, 0x03, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn advances_shared_live_write_counters() {
        let mut state = HelixState::new();
        let ctr_before = state.live_write_ctr;
        let yy_before = state.live_write_yy;
        let _ = build_controller_source_footswitch_write_packet(&mut state, 0x01, 1);
        assert_eq!(state.live_write_ctr, ctr_before.wrapping_add(0x57));
        assert_eq!(state.live_write_yy, yy_before.wrapping_add(2));
    }

    /// Octets réels capturés (`controllers_change_type.json`, pkt#376/#894) — hors octet `seq`
    /// (position 9, dépend du compteur global `x80_cnt`, pas figé au moment de la capture).
    #[test]
    fn builds_expected_bytes_for_type_momentary_and_latching() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x1e5f;
        state.live_write_yy = 0xf4;
        let momentary_pkt = build_controller_type_write_packet(&mut state, true);
        assert_eq!(momentary_pkt.len(), 40);
        assert_eq!(&momentary_pkt[0..9], &[0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00]);
        assert_eq!(&momentary_pkt[10..14], &[0x00, 0x04, 0x5f, 0x1e]);
        assert_eq!(&momentary_pkt[24..32], &[0x83, 0x66, 0xcd, 0x03, 0xf4, 0x64, 0x3a, 0x65]);
        assert_eq!(&momentary_pkt[32..40], &[0x82, 0x66, 0x00, 0x41, 0xc3, 0x00, 0x00, 0x00]);

        state.live_write_ctr = 0x1e70;
        state.live_write_yy = 0xf5;
        let latching_pkt = build_controller_type_write_packet(&mut state, false);
        assert_eq!(&latching_pkt[12..14], &[0x70, 0x1e]);
        assert_eq!(&latching_pkt[32..40], &[0x82, 0x66, 0x00, 0x41, 0xc2, 0x00, 0x00, 0x00]);
    }

    /// Octets réels capturés (`controllers_colors_W_R_G_B.json`, pkt#418, couleur White=index 1).
    #[test]
    fn builds_expected_bytes_for_color_white() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x1f07;
        state.live_write_yy = 0xfa;
        let pkt = build_controller_color_write_packet(&mut state, 1);
        assert_eq!(&pkt[12..14], &[0x07, 0x1f]);
        assert_eq!(&pkt[24..32], &[0x83, 0x66, 0xcd, 0x03, 0xfa, 0x64, 0x3d, 0x65]);
        assert_eq!(&pkt[32..40], &[0x82, 0x66, 0x00, 0x42, 0x01, 0x00, 0x00, 0x00]);
    }

    /// Octets réels capturés (`controller_change_params.json`, pkt#4014, nom "toto").
    #[test]
    fn builds_expected_bytes_for_name_toto() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x2f79;
        state.live_write_yy = 0x16;
        let pkt = build_controller_name_write_packet(&mut state, "toto");
        assert_eq!(pkt.len(), 44);
        assert_eq!(pkt[0], 0x22);
        assert_eq!(&pkt[12..14], &[0x79, 0x2f]);
        assert_eq!(pkt[20], 0x12);
        assert_eq!(&pkt[24..32], &[0x83, 0x66, 0xcd, 0x03, 0x16, 0x64, 0x3b, 0x65]);
        assert_eq!(
            &pkt[32..],
            &[0x82, 0x66, 0x00, 0x6d, 0xa5, b't', b'o', b't', b'o', 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn name_write_scales_byte0_and_length_field_with_name_length() {
        let mut state = HelixState::new();
        let pkt = build_controller_name_write_packet(&mut state, "AB");
        // fixstr_len = 3 (2 car. + nul) -> payload_len = 8+4+1+3 = 16 = 0x10 -> byte0 = 0x20
        assert_eq!(pkt[0], 0x20);
        assert_eq!(pkt[20], 0x10);
        assert_eq!(pkt.len(), 24 + 0x10 + 2);
        assert_eq!(&pkt[32..], &[0x82, 0x66, 0x00, 0x6d, 0xa3, b'A', b'B', 0x00, 0x00, 0x00]);
    }
}
