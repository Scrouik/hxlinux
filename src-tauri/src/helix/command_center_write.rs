//! Écriture live des champs d'assignation Command Center (onglet « Controllers »).
//!
//! **Créer une assignation sur un VRAI paramètre** (pas Bypass) nécessite d'abord un paquet
//! dédié (`build_controller_create_real_param_assignment_write_packet`, terme `0x25`) qui porte
//! le `param_selector` (même convention que `live_write.rs` : index du paramètre dans la liste du
//! modèle) — sans lui, rien n'indique au device QUEL paramètre est visé, et il assigne par défaut
//! au Bypass du bloc. Ce paquet doit être envoyé **avant** la Source. Bypass n'en a pas besoin
//! (voir `build_controller_source_and_confirm_write_packets`, format simple, fonctionne déjà).
//!
//! Le reste des paquets ne porte **aucun identifiant de bus/switch** dans leur payload
//! (`82:66:00:<champ>:...`, confirmé sur la capture Type) : le device retient implicitement
//! « quel switch est en cours d'édition » à partir du dernier paquet de sélection de **Source**
//! envoyé (`82:62:<bus>:66:<idx>`, qui lui porte bien le bus). Il faut donc TOUJOURS écrire la
//! Source ensuite — avec le bon slot déjà actif côté device (vérifié aussi manuellement dans HX
//! Edit par l'user, 2026-07-09) — avant d'écrire Type/Couleur/Nom/Min-Max sur ce même switch.
//!
//! Format Source vérifié par capture (`controllers_select_all_switch_one_by_one.json`, 8
//! échantillons Footswitch 1-8 cohérents, ctr delta=0x57, yy delta=+2). Seul le format compact
//! Footswitch est implémenté ici ; le format long EXP Pedal/None
//! (`...5f:05:60:cd:01:2c:4a:<N>:47:...`, voir mémoire session 2026-07-07) reste à faire.

use crate::helix::HelixState;

/// Construit le paquet qui lie un switch à un **vrai paramètre catalogue** (obligatoire avant
/// Source pour cibler autre chose que Bypass). `param_selector` = index du paramètre dans la
/// liste du modèle (même convention que `live_write.rs::param_selector_byte_from_index`).
///
/// Format et compteurs vérifiés par capture différentielle (2026-07-10) : `Preset_one_Drive_FS1.json`
/// / `Preset_two_Drive_FS1.json` (une seule assignation, preset propre à chaque fois) +
/// `multi_controls_multi_FS.json` (7 créations consécutives, mêmes bus/slot).
/// - Champ `4a` (ordinal d'assignation) : vaut **`3`** pour la toute première assignation d'un
///   contexte propre (confirmé identique sur 2 presets différents, et sur une capture antérieure
///   distincte `Controllers_Drive_control.json` — donc PAS un compteur global persistant du
///   device : il repart de cette base à chaque nouveau preset). Avance de `+1` par création
///   supplémentaire dans la MÊME session d'édition (confirmé 6/6 sur `multi_controls_multi_FS.json`).
///   Cette fonction ne gère que le cas simple (une seule création, `4a=3` fixe) — créer plusieurs
///   assignations d'affilée dans un même appel `Save` n'est pas encore géré.
/// - `yy` avance de **`+3`** par création (confirmé 6/6, écart exact sur tous les intervalles).
/// - `ctr` : delta **NON confirmé avec certitude** — les échantillons multi-créations incluent la
///   cascade d'accusés de réception/relecture automatique de HX Edit (paquets qu'on n'envoie pas
///   nous-mêmes), donc le delta mesuré (~160, variable ±3) n'isole pas la contribution de CE seul
///   paquet. Valeur retenue ici (`0x57`) par analogie avec Source (l'autre paquet qui établit un
///   nouveau contexte) — **à valider prudemment lors du prochain test device**, un changement à la
///   fois, en surveillant si les écritures suivantes (Source/Type/Couleur/Nom) restent cohérentes.
/// - Découverte bonus : `Clear Assignment`/`Clear All` dans HX Edit réutilisent ce MÊME paquet avec
///   `param_selector=0` ET `4a=0` (voir `clear_selected_assigment.json`/`clear_all_assigment.json`)
///   — confirme que ce paquet est le point central de liaison switch↔paramètre.
pub fn build_controller_create_real_param_assignment_write_packet(
    state: &mut HelixState,
    slot_bus: u8,
    param_selector: u8,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;
    let pp: u8 = 0x04;
    // Ordinal d'assignation : `3` fixe (cas géré ici — une seule création par contexte propre).
    let assignment_ordinal: u8 = 0x03;

    let packet = vec![
        0x28, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x18, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, 0x25, 0x65,
        0x87, 0x62, slot_bus, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, param_selector,
        0x4a, assignment_ordinal, 0x47, 0x04, 0xcc, 0x81, 0xc2,
    ];

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x57);
    state.live_write_yy = state.live_write_yy.wrapping_add(3);
    packet
}

/// Octets bruts du paquet Source Footswitch (format compact), sans toucher à l'état partagé —
/// utilisé à la fois par l'envoi standalone (réassignation FS sur une assignation existante) et
/// par la paire Source+Confirmation (création), qui ont chacun leurs propres règles d'avancement
/// de `ctr`/`yy`. `footswitch_number` est le numéro HX Edit (1-8), converti en index 0-based.
fn source_footswitch_packet_bytes(
    seq: u8,
    ctr: u16,
    yy: u8,
    slot_bus: u8,
    footswitch_number: u8,
) -> Vec<u8> {
    // `pp` fixe à 0x03 : même simplification que le reste du codebase (amp_cab_live_write.rs ne
    // gère pas non plus dynamiquement le passage à 0x04 quand `yy` boucle 0xff→0x00) — cas rare
    // pour une action manuelle d'assignation, pas un poll haute fréquence.
    let pp: u8 = 0x03;
    let term: u8 = 0x38;
    let fs_index_0based = footswitch_number.saturating_sub(1);

    vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, term, 0x65,
        0x82, 0x62, slot_bus, 0x66, fs_index_0based, 0x00, 0x00, 0x00,
    ]
}

/// Construit la paire **Source + Confirmation** utilisée pour créer/réassigner une Source
/// Footswitch — la confirmation (terme `0x21`, payload `81:66:01`) doit suivre IMMÉDIATEMENT la
/// Source, avec un `ctr` dérivé de celui de Source **+0x11** (pas relu depuis l'état partagé après
/// coup — voir bug ci-dessous). Commune aux deux types d'assignation (Bypass ET vrai paramètre).
///
/// Paquet de confirmation découvert en analysant `add_bypass_switch_FS.json` (capture HX Edit,
/// 2026-07-10) : sans lui, l'assignation créée reste dans un état incomplet côté device — confirmé
/// par un test réel où, après avoir assigné Drive sans ce paquet, appuyer sur le switch physique ne
/// générait **aucun trafic USB** (ni changement de valeur, ni bascule Bypass), alors que la même
/// action sur une assignation créée par HX Edit (avec ce paquet) génère bien la notification déjà
/// connue `82:62:<bus>:3b:<c2|c3>`. Retrouvé à l'identique dans `Controllers_Drive_control.json`.
///
/// **Bug corrigé (2026-07-10)** : une première implémentation appelait Source puis Confirm comme
/// deux fonctions indépendantes, chacune lisant `state.live_write_ctr` — mais Source l'avance de
/// `+0x57` (delta correct pour "encore un Source qui suit", mesuré sur un scénario de
/// réassignation FS sans confirmation), donc Confirm lisait un `ctr` déjà avancé de trop. Mesure
/// DIRECTE sur `add_bypass_switch_FS.json` (Source `ctr=0x419c` → Confirm `ctr=0x41ad`) : l'écart
/// réel est **`+0x11`**, pas `+0x57`. Cette fonction combinée calcule Confirm à partir de la valeur
/// de Source AVANT tout avancement, pas de l'état partagé relu après coup.
///
/// **Correctif 2026-07-11** : le champ `cd:<hi>:<lo>` (octets 27-28) est UN compteur 16 bits unique.
/// Mesuré sur 5 captures HX Edit, Source→Confirm = **+1 exact** sur ce 16 bits (multi_controls
/// 0x0416→0x0417, Preset_one_Bypass 0x0410→0x0411, 2_controls_one_FS 0x03f1→0x03f2 avec octet haut
/// 0x03, add_bypass_switch_FS 0x0401→0x0402). L'octet haut varie selon la session ⇒ compteur libre,
/// pas champ sémantique. L'ancienne version hardcodait pp=0x04 côté Confirm (Source pp=0x03), d'où un
/// saut `+0x101` au lieu de `+1`. Confirm dérive donc `cd = cd_Source + 1` sur le 16 bits complet
/// (carry géré) — on est maintenant strictement calé sur HX Edit. NB (2026-07-11) : ce correctif ne
/// suffit PAS à réparer la synchro live d'un contrôle créé mid-session — capture ultérieure a montré
/// que le device ne sert simplement pas notre canal de notifications `f0:03` (problème d'armement du
/// mode éditeur, indépendant de ce paquet). Ce correctif reste juste (conformité HX Edit) et n'entrave
/// pas ce chantier. L'avancement de l'état partagé APRÈS Confirm (`ctr +0x44`, `yy +1`) reste une
/// estimation non isolée — à revalider si les écritures suivantes semblent désynchronisées.
pub fn build_controller_source_and_confirm_write_packets(
    state: &mut HelixState,
    slot_bus: u8,
    footswitch_number: u8,
) -> (Vec<u8>, Vec<u8>) {
    let source_seq = state.next_x80_cnt();
    let source_ctr = state.live_write_ctr;
    let source_yy = state.live_write_yy;
    let source_packet =
        source_footswitch_packet_bytes(source_seq, source_ctr, source_yy, slot_bus, footswitch_number);

    let confirm_seq = state.next_x80_cnt();
    let confirm_ctr = source_ctr.wrapping_add(0x11);
    // `cd:<hi>:<lo>` (octets 27-28) est UN SEUL compteur 16 bits, pas deux champs séparés.
    // Mesuré 2026-07-11 sur 5 captures HX Edit : Source→Confirm = +1 EXACT (l'octet haut ne
    // change que si l'octet bas boucle 0xff→0x00). L'ancienne version hardcodait pp=0x04 côté
    // Confirm alors que la Source utilise pp=0x03 (voir source_footswitch_packet_bytes), produisant
    // un saut de +0x101. On dérive donc Confirm = Source + 1 sur le 16 bits complet, pour se caler
    // exactement sur HX Edit (voir doc de fonction pour le contexte synchro live / canal f0:03).
    const SOURCE_PP: u8 = 0x03; // doit rester synchro avec source_footswitch_packet_bytes
    let source_cd: u16 = ((SOURCE_PP as u16) << 8) | source_yy as u16;
    let confirm_cd = source_cd.wrapping_add(1);
    let confirm_pp = (confirm_cd >> 8) as u8;
    let confirm_yy = (confirm_cd & 0xff) as u8;

    let confirm_packet = vec![
        0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, confirm_seq, 0x00, 0x0c,
        (confirm_ctr & 0xff) as u8, ((confirm_ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, confirm_pp, confirm_yy, 0x64, 0x21, 0x65,
        0x81, 0x66, 0x01, 0x00,
    ];

    state.live_write_ctr = confirm_ctr.wrapping_add(0x44);
    state.live_write_yy = confirm_yy.wrapping_add(1);

    (source_packet, confirm_packet)
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
    /// Footswitch 1 (pkt#440) et Footswitch 4 (pkt#1904, après un premier wrap de `yy`) — vérifie
    /// uniquement le paquet Source de la paire (le paquet de confirmation n'existait pas encore
    /// dans cette capture historique, antérieure à la découverte du 2026-07-10).
    #[test]
    fn builds_expected_bytes_for_footswitch1() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x1f65;
        state.live_write_yy = 0xf9;
        let (pkt, _confirm) = build_controller_source_and_confirm_write_packets(&mut state, 0x0b, 1);
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
        let (pkt, _confirm) = build_controller_source_and_confirm_write_packets(&mut state, 0x0b, 4);
        assert_eq!(&pkt[12..14], &[0x6a, 0x20]);
        assert_eq!(&pkt[24..30], &[0x83, 0x66, 0xcd, 0x03, 0xff, 0x64]);
        assert_eq!(&pkt[32..40], &[0x82, 0x62, 0x0b, 0x66, 0x03, 0x00, 0x00, 0x00]);
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

    /// Octets réels capturés (`Preset_one_Drive_FS1.json`, pkt#1174) — Drive (param_selector=0),
    /// preset propre, première création (ordinal `4a`=3).
    #[test]
    fn builds_expected_bytes_for_create_real_param_assignment_drive_preset_one() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x8501;
        state.live_write_yy = 0x16;
        let pkt = build_controller_create_real_param_assignment_write_packet(&mut state, 0x01, 0x00);
        assert_eq!(pkt.len(), 48);
        assert_eq!(pkt[0], 0x28);
        assert_eq!(&pkt[12..14], &[0x01, 0x85]);
        assert_eq!(pkt[20], 0x18);
        assert_eq!(&pkt[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x16, 0x64, 0x25, 0x65]);
        assert_eq!(
            &pkt[32..],
            &[0x87, 0x62, 0x01, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, 0x00, 0x4a, 0x03, 0x47, 0x04, 0xcc, 0x81, 0xc2]
        );
    }

    /// Même capture, preset two (`Preset_two_Drive_FS1.json`, pkt#994) — même bus, param, ordinal
    /// (`4a`=3 identique malgré le changement de preset) : confirme que `4a` n'est pas un compteur
    /// global persistant du device (voir mémoire session 2026-07-10).
    #[test]
    fn builds_same_ordinal_across_different_presets() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0xa810;
        state.live_write_yy = 0x24;
        let pkt = build_controller_create_real_param_assignment_write_packet(&mut state, 0x01, 0x00);
        assert_eq!(&pkt[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x24, 0x64, 0x25, 0x65]);
        assert_eq!(&pkt[39..42], &[0x1c, 0x00, 0x4a], "1c, param_selector, puis tag 4a");
        assert_eq!(pkt[42], 0x03, "ordinal 4a=3, identique au preset one");
    }

    #[test]
    fn param_selector_byte_reflects_chosen_parameter() {
        let mut state = HelixState::new();
        let pkt = build_controller_create_real_param_assignment_write_packet(&mut state, 0x05, 0x07);
        assert_eq!(pkt[39], 0x1c);
        assert_eq!(pkt[40], 0x07, "param_selector = index du paramètre choisi");
        assert_eq!(pkt[34], 0x05, "bus du bloc contrôlé");
    }

    #[test]
    fn create_assignment_advances_shared_live_write_counters() {
        let mut state = HelixState::new();
        let ctr_before = state.live_write_ctr;
        let yy_before = state.live_write_yy;
        let _ = build_controller_create_real_param_assignment_write_packet(&mut state, 0x01, 0x00);
        assert_eq!(state.live_write_ctr, ctr_before.wrapping_add(0x57));
        assert_eq!(state.live_write_yy, yy_before.wrapping_add(3));
    }

    /// Octets réels capturés (`add_bypass_switch_FS.json`, pkt#600 Source + pkt#610 Confirm) —
    /// vérifie que Confirm dérive bien son `ctr`/`yy` de Source (`+0x11`/`+1`), PAS de l'état
    /// partagé après l'avancement `+0x57`/`+2` de Source (bug corrigé le 2026-07-10).
    #[test]
    fn source_and_confirm_pair_uses_direct_offset_not_shared_state_after_source_advance() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x419c;
        state.live_write_yy = 0x01;
        let (source_pkt, confirm_pkt) =
            build_controller_source_and_confirm_write_packets(&mut state, 0x01, 1);

        // Source : ctr=0x419c, yy=0x01 (valeurs de départ, inchangées). cd Source = 0x03:01.
        assert_eq!(&source_pkt[12..14], &[0x9c, 0x41]);
        assert_eq!(source_pkt[28], 0x01);
        assert_eq!(&source_pkt[24..29], &[0x83, 0x66, 0xcd, 0x03, 0x01], "cd Source = 0x0301");
        assert_eq!(&source_pkt[32..40], &[0x82, 0x62, 0x01, 0x66, 0x00, 0x00, 0x00, 0x00]);

        // Confirm : ctr=0x41ad (=0x419c+0x11). cd Confirm = cd Source + 1 = 0x0302 (correctif
        // 2026-07-11 : compteur 16 bits, +1 exact comme HX Edit — plus de saut +0x101 pp 03→04).
        assert_eq!(confirm_pkt.len(), 36);
        assert_eq!(confirm_pkt[0], 0x1b);
        assert_eq!(&confirm_pkt[10..14], &[0x00, 0x0c, 0xad, 0x41]);
        assert_eq!(pkt_confirm_yy(&confirm_pkt), 0x02);
        assert_eq!(&confirm_pkt[24..32], &[0x83, 0x66, 0xcd, 0x03, 0x02, 0x64, 0x21, 0x65]);
        assert_eq!(&confirm_pkt[32..36], &[0x81, 0x66, 0x01, 0x00]);
    }

    fn pkt_confirm_yy(pkt: &[u8]) -> u8 {
        pkt[28]
    }

    #[test]
    fn source_and_confirm_pair_advances_shared_state_past_confirm() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x419c;
        state.live_write_yy = 0x01;
        let _ = build_controller_source_and_confirm_write_packets(&mut state, 0x01, 1);
        // confirm_ctr(0x41ad) + 0x44, confirm_yy(0x02) + 1 — estimation non isolée, voir doc.
        assert_eq!(state.live_write_ctr, 0x41ad_u16.wrapping_add(0x44));
        assert_eq!(state.live_write_yy, 0x03);
    }
}
