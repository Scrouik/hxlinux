//! Écriture live des champs d'assignation Command Center (onglet « Controllers »).
//!
//! **Créer une assignation sur un VRAI paramètre** (pas Bypass) est un trio de paquets
//! (`build_controller_create_real_param_write_packets`, termes `0x25`+`0x24`+`0x21`) qui porte à
//! la fois le `param_selector` (même convention que `live_write.rs` : index du paramètre dans la
//! liste du modèle) ET le Footswitch choisi — **contrairement à ce qu'on pensait avant le
//! 2026-07-12, HX Edit n'envoie JAMAIS de paquet Source (`term=0x38`) séparé pour ce cas** : le FS
//! est directement encodé dans ce trio (champ `4a` du `term=0x25`, dernier octet du Confirm). Sans
//! le `term=0x24` (découvert le 2026-07-12, absent de l'ancienne implémentation), le lien n'est
//! jamais finalisé côté device, qui assigne par défaut au Bypass du bloc — bug rapporté par l'user.
//! Bypass, lui, n'a pas de vrai paramètre à lier : il garde le format Source+Confirm simple (voir
//! `build_controller_source_and_confirm_write_packets`).
//!
//! Le reste des paquets (Type/Couleur/Nom) ne porte **aucun identifiant de bus/switch** dans leur
//! payload (`82:66:00:<champ>:...`, confirmé sur la capture Type) : le device retient implicitement
//! « quel switch est en cours d'édition » à partir du dernier paquet Source/création envoyé. Il
//! faut donc toujours créer/sélectionner la Source d'abord — avec le bon slot déjà actif côté
//! device (vérifié aussi manuellement dans HX Edit par l'user, 2026-07-09) — avant d'écrire
//! Type/Couleur/Nom/Min-Max sur ce même switch.
//!
//! Format Source (Bypass) vérifié par capture (`controllers_select_all_switch_one_by_one.json`, 8
//! échantillons Footswitch 1-8 cohérents, ctr delta=0x57, yy delta=+2). Seul le format compact
//! Footswitch est implémenté ici ; le format long EXP Pedal/None
//! (`...5f:05:60:cd:01:2c:4a:<N>:47:...`, voir mémoire session 2026-07-07) reste à faire.

use crate::helix::HelixState;

/// Construit le trio de paquets qui crée un lien switch↔**vrai paramètre catalogue** ET lui
/// assigne un Footswitch, en une seule séquence — HX Edit n'envoie JAMAIS de paquet Source
/// (`term=0x38`) séparé pour ce cas, le Footswitch est directement encodé dans ce trio.
/// `param_selector` = index du paramètre dans la liste du modèle (même convention que
/// `live_write.rs::param_selector_byte_from_index`). `footswitch_number` = numéro HX Edit (1-8).
///
/// Format et compteurs vérifiés par capture différentielle le 2026-07-12 sur 3 captures HX Edit,
/// **12/12 contrôles concordants, 0 exception** : `add_drive_slot1_add_level_slot2.json` (2
/// contrôles), `add_8_controles_8_slots.json` (8 contrôles, FS=numéro de slot), et surtout
/// `add_FS1_slot8_add_FS8_slot4.json` — capture DÉLIBÉRÉMENT décorrélée (bus, ordre de création et
/// FS tous différents entre eux : FS1 sur slot8, FS8 sur slot4) qui a permis de trancher sans
/// ambiguïté.
///
/// **3 paquets, `pp=0x04` CONSTANT sur les 3** (pas de dérivation cd comme pour Bypass, dont le
/// premier paquet Source utilise `pp=0x03`) :
/// 1. `term=0x25` (tag `87`, 7 clés) : lie `bus`+`param_selector`, ET porte le FS choisi dans le
///    champ `4a` — **PAS un « ordinal d'assignation » comme documenté avant le 2026-07-12**, mais
///    le champ `source` avec la MÊME convention d'enum que partout ailleurs dans ce module
///    (`0`=None, `1`/`2`=EXP Pedal 1/2, `3+`=Footswitch(N-2)) : **`4a = footswitch_number + 2`**.
///    L'ancienne théorie (« 4a=3 fixe » ou « 3+nombre de contrôles existants ») était une
///    coïncidence : dans toutes les anciennes captures, le FS choisi montait par hasard en
///    séquence depuis 1.
/// 2. `term=0x24` (tag `84`, 4 clés — SOUS-ENSEMBLE de (1), même `bus`+`param_selector`, SANS
///    ordinal ni trailer) : paquet qui manquait entièrement dans l'ancienne implémentation.
/// 3. `term=0x21` Confirm : payload `81:66:<N>:00` où **`N` = `footswitch_number` BRUT** (1-8, PAS
///    la convention enum de (1)) — anciennement fixé à `0x01` en dur (voir aussi le fix apporté à
///    `build_controller_source_and_confirm_write_packets` pour Bypass, même champ).
///
/// Deltas `ctr`/`yy` entre chaque paquet consécutif du trio : **`+0x31`/`+1`**, confirmés
/// identiques sur les 12 contrôles des 3 captures (0 exception) — remplace l'ancien `+0x57`/`+3`
/// (qui datait d'une mesure non isolée, cf ancien commentaire). L'avancement de l'état partagé
/// APRÈS ce trio (utilisé par le paquet suivant, ex. un changement de focus de slot `term=0x4e`
/// observé entre deux créations dans les 3 captures, mais géré ailleurs dans le code) reste une
/// ESTIMATION non isolée (`+0x44`/`+1`, par analogie avec `build_controller_source_and_confirm_write_packets`)
/// — à revalider si les écritures suivantes semblent désynchronisées.
///
/// Découverte bonus (pré-2026-07-12, toujours valable) : `Clear Assignment`/`Clear All` dans HX
/// Edit réutilisent le paquet `term=0x25` avec `param_selector=0` ET `4a=0` (voir
/// `clear_selected_assigment.json`/`clear_all_assigment.json`) — `4a=0` ne correspond à aucun FS
/// réel (formule `footswitch_number+2` donnerait `2` pour FS0 inexistant), cohérent avec un usage
/// comme sentinelle "aucun lien" pour l'effacement.
pub fn build_controller_create_real_param_write_packets(
    state: &mut HelixState,
    slot_bus: u8,
    param_selector: u8,
    footswitch_number: u8,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let pp: u8 = 0x04;
    let source_value = footswitch_number + 2;

    let create_seq = state.next_x80_cnt();
    let create_ctr = state.live_write_ctr;
    let create_yy = state.live_write_yy;
    let create_packet = vec![
        0x28, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, create_seq, 0x00, 0x04,
        (create_ctr & 0xff) as u8, ((create_ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x18, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, create_yy, 0x64, 0x25, 0x65,
        0x87, 0x62, slot_bus, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, param_selector,
        0x4a, source_value, 0x47, 0x04, 0xcc, 0x81, 0xc2,
    ];

    let link_seq = state.next_x80_cnt();
    let link_ctr = create_ctr.wrapping_add(0x31);
    let link_yy = create_yy.wrapping_add(1);
    let link_packet = vec![
        0x21, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, link_seq, 0x00, 0x0c,
        (link_ctr & 0xff) as u8, ((link_ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x11, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, link_yy, 0x64, 0x24, 0x65,
        0x84, 0x62, slot_bus, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, param_selector,
        0x00, 0x00, 0x00,
    ];

    let confirm_seq = state.next_x80_cnt();
    let confirm_ctr = link_ctr.wrapping_add(0x31);
    let confirm_yy = link_yy.wrapping_add(1);
    let confirm_packet = vec![
        0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, confirm_seq, 0x00, 0x0c,
        (confirm_ctr & 0xff) as u8, ((confirm_ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, confirm_yy, 0x64, 0x21, 0x65,
        0x81, 0x66, footswitch_number, 0x00,
    ];

    state.live_write_ctr = confirm_ctr.wrapping_add(0x44);
    state.live_write_yy = confirm_yy.wrapping_add(1);

    (create_packet, link_packet, confirm_packet)
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
        // 3e octet du payload = footswitch_number BRUT (1-8), PAS une constante `1` — confirmé le
        // 2026-07-12 sur le trio de création vrai-paramètre (`build_controller_create_real_param_write_packets`,
        // 12/12 contrôles concordants sur 3 captures dont une décorrélée bus/ordre/FS). Non re-vérifié
        // séparément pour Bypass (FS≠1) mais même paquet `term=0x21`, donc même champ — fix appliqué
        // par cohérence, à confirmer si besoin par une capture Bypass+FS≠1 (cf mémoire session).
        0x81, 0x66, footswitch_number, 0x00,
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

/// Construit le paquet d'écriture de la borne **Min** (`is_max=false`, terme `0x41`) ou **Max**
/// (`is_max=true`, terme `0x42`) d'un contrôle sur vrai paramètre. `value` = valeur BRUTE catalogue
/// du paramètre (même unité que `min_raw`/`max_raw` décodés dans `controller_assignments.rs`),
/// encodée en flottant 32 bits **big-endian** (`ca:<f32>`).
///
/// Format vérifié byte-exact sur capture `controllers/save/Controllers_Drive_control.json`
/// (2026-07-12, Drive slot1 : Min 0.0→0.4, Max 1.0→0.54 — cohérent avec la fixture décodage
/// `BASELINE_DRIVE_FS1`). Contrairement aux champs Type/Couleur/Nom (qui ne portent pas
/// d'identifiant et reposent sur le « switch en cours d'édition »), ce paquet est AUTO-SUFFISANT :
/// il porte le bus du bloc contrôlé ET le `param_selector`, comme le paquet de création `0x25`.
/// `pp=0x04` constant (comme la création, pas `0x03` des single-field). `ctr +0x11` / `yy +1` par
/// écriture (mesuré 6/6 sur la capture). L'octet `sub` (offset 11) vaut `0x04` ici (HX Edit
/// alterne 04/0c dans sa capture — framing interne toléré par le device ; on garde `0x04` comme
/// tous les autres builders single-field déjà validés sur le device).
pub fn build_controller_min_max_write_packet(
    state: &mut HelixState,
    slot_bus: u8,
    param_selector: u8,
    is_max: bool,
    value: f32,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;
    let pp: u8 = 0x04;
    let term: u8 = if is_max { 0x42 } else { 0x41 };
    let f = value.to_be_bytes();

    let packet = vec![
        0x27, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x17, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, term, 0x65,
        0x85, 0x62, slot_bus, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, param_selector,
        0x77, 0xca, f[0], f[1], f[2], f[3], 0x00,
    ];

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    packet
}

/// Supprime une assignation sur **vrai paramètre** : réutilise le paquet de création `term=0x25`
/// mais avec `source(4a)=0` (None) et `47=0` — « réassigner à rien » = suppression côté device.
/// Un SEUL paquet (pas de 0x24/0x21 comme à la création). Byte-exact contre
/// `controllers/clear_selected_assigment.json` (bus 01, param 0). `param_selector` cible le
/// paramètre contrôlé (même convention qu'à la création). Compteurs `ctr +0x11`/`yy +1`.
pub fn build_controller_delete_real_param_write_packet(
    state: &mut HelixState,
    slot_bus: u8,
    param_selector: u8,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;

    let packet = vec![
        0x28, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x18, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x04, yy, 0x64, 0x25, 0x65,
        0x87, 0x62, slot_bus, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, param_selector,
        0x4a, 0x00, 0x47, 0x00, 0xcc, 0x81, 0xc2,
    ];

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    packet
}

/// Supprime une assignation **Bypass** : paquet `term=0x39` (= le Source compact du Bypass
/// `82 62 <bus> 66 <fs_index>` mais terme `0x39` au lieu de `0x38` = « clear source »). Le Bypass
/// n'ayant pas de `param_selector` (source positionnelle), on l'efface par sa position footswitch.
/// Byte-exact contre le 2e paquet de `controllers/clear_all_assigment.json` (bus 01, fs_index 04).
/// `footswitch_number` = numéro HX Edit (1-8). Compteurs `ctr +0x11`/`yy +1`.
pub fn build_controller_delete_bypass_write_packet(
    state: &mut HelixState,
    slot_bus: u8,
    footswitch_number: u8,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;
    let fs_index = footswitch_number.saturating_sub(1);

    let packet = vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x0c,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x04, yy, 0x64, 0x39, 0x65,
        0x82, 0x62, slot_bus, 0x66, fs_index, 0x00, 0x00, 0x00,
    ];

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Octets réels capturés (`controllers/clear_selected_assigment.json`, frame 2243) —
    /// suppression d'un contrôle sur vrai paramètre (bus 01, param 0) : paquet `0x25` avec
    /// `4a=0`/`47=0`. ctr=0xa8b0, yy=0x27.
    #[test]
    fn builds_expected_bytes_for_delete_real_param() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0xa8b0;
        state.live_write_yy = 0x27;
        let pkt = build_controller_delete_real_param_write_packet(&mut state, 0x01, 0x00);
        assert_eq!(pkt.len(), 48);
        assert_eq!(pkt[0], 0x28);
        assert_eq!(&pkt[12..14], &[0xb0, 0xa8]);
        assert_eq!(&pkt[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x27, 0x64, 0x25, 0x65]);
        assert_eq!(
            &pkt[32..],
            &[0x87, 0x62, 0x01, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, 0x00, 0x4a, 0x00, 0x47, 0x00, 0xcc, 0x81, 0xc2],
            "4a=0 (source None) et 47=0 = suppression"
        );
        assert_eq!(state.live_write_ctr, 0xa8c1);
        assert_eq!(state.live_write_yy, 0x28);
    }

    /// Octets réels capturés (`controllers/clear_all_assigment.json`, frame 2793) — suppression
    /// d'un Bypass : paquet `0x39` (`82 62 <bus> 66 <fs_index>`). bus 01, fs_index 04 (=FS5).
    /// ctr=0xa9c7, yy=0x2e.
    #[test]
    fn builds_expected_bytes_for_delete_bypass() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0xa9c7;
        state.live_write_yy = 0x2e;
        let pkt = build_controller_delete_bypass_write_packet(&mut state, 0x01, 5);
        assert_eq!(pkt.len(), 40);
        assert_eq!(pkt[0], 0x1d);
        assert_eq!(&pkt[10..14], &[0x00, 0x0c, 0xc7, 0xa9]);
        assert_eq!(&pkt[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x2e, 0x64, 0x39, 0x65]);
        assert_eq!(&pkt[32..40], &[0x82, 0x62, 0x01, 0x66, 0x04, 0x00, 0x00, 0x00]);
        assert_eq!(state.live_write_ctr, 0xa9d8);
        assert_eq!(state.live_write_yy, 0x2f);
    }

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

    /// Octets réels capturés (`controllers/save/Controllers_Drive_control.json`, frames 3281/3329
    /// pour Min, 3969 pour Max) — Drive (bus=01, param_selector=0). Vérifie term (0x41 Min / 0x42
    /// Max), le flottant f32 big-endian (`ca:<f32>`), et le template auto-suffisant (bus+param).
    #[test]
    fn builds_expected_bytes_for_min_max_write_drive() {
        // Frame 3281 : Min = 0.0, ctr=0x6012, yy=0x07.
        let mut state = HelixState::new();
        state.live_write_ctr = 0x6012;
        state.live_write_yy = 0x07;
        let min0 = build_controller_min_max_write_packet(&mut state, 0x01, 0x00, false, 0.0);
        assert_eq!(min0.len(), 48);
        assert_eq!(min0[0], 0x27);
        assert_eq!(&min0[10..14], &[0x00, 0x04, 0x12, 0x60]);
        assert_eq!(min0[20], 0x17);
        assert_eq!(&min0[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x07, 0x64, 0x41, 0x65]);
        assert_eq!(
            &min0[32..],
            &[0x85, 0x62, 0x01, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, 0x00, 0x77, 0xca, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        // avancement partagé : ctr +0x11, yy +1
        assert_eq!(state.live_write_ctr, 0x6023);
        assert_eq!(state.live_write_yy, 0x08);

        // Frame 3329 : Min = 0.4 (0x3ecccccd), ctr=0x6034, yy=0x09.
        let mut state = HelixState::new();
        state.live_write_ctr = 0x6034;
        state.live_write_yy = 0x09;
        let min04 = build_controller_min_max_write_packet(&mut state, 0x01, 0x00, false, 0.4);
        assert_eq!(&min04[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x09, 0x64, 0x41, 0x65]);
        assert_eq!(
            &min04[41..48],
            &[0x77, 0xca, 0x3e, 0xcc, 0xcc, 0xcd, 0x00],
            "flottant 0.4 encodé big-endian ca:3e:cc:cc:cd"
        );

        // Frame 3969 : Max = 1.0 (0x3f800000), ctr=0x6045, yy=0x0a.
        let mut state = HelixState::new();
        state.live_write_ctr = 0x6045;
        state.live_write_yy = 0x0a;
        let max1 = build_controller_min_max_write_packet(&mut state, 0x01, 0x00, true, 1.0);
        assert_eq!(&max1[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x0a, 0x64, 0x42, 0x65]);
        assert_eq!(
            &max1[41..48],
            &[0x77, 0xca, 0x3f, 0x80, 0x00, 0x00, 0x00],
            "term 0x42 (Max) + flottant 1.0 = ca:3f:80:00:00"
        );
        assert_eq!(max1[34], 0x01, "bus du bloc contrôlé");
        assert_eq!(max1[40], 0x00, "param_selector");
    }

    /// Octets réels capturés (`add_drive_slot1_add_level_slot2.json`, frames 5151/5169/5177) —
    /// Drive (param_selector=0) sur slot bus=01, FS1 choisi → `4a`=1+2=3.
    #[test]
    fn builds_expected_bytes_for_create_real_param_drive_slot1_fs1() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x6f97;
        state.live_write_yy = 0x0a;
        let (create, link, confirm) =
            build_controller_create_real_param_write_packets(&mut state, 0x01, 0x00, 1);

        assert_eq!(create.len(), 48);
        assert_eq!(create[0], 0x28);
        assert_eq!(&create[12..14], &[0x97, 0x6f]);
        assert_eq!(&create[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x0a, 0x64, 0x25, 0x65]);
        assert_eq!(
            &create[32..],
            &[0x87, 0x62, 0x01, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, 0x00, 0x4a, 0x03, 0x47, 0x04, 0xcc, 0x81, 0xc2]
        );

        assert_eq!(link.len(), 44);
        assert_eq!(link[0], 0x21);
        assert_eq!(&link[12..14], &[0xc8, 0x6f], "ctr = create_ctr + 0x31");
        assert_eq!(&link[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x0b, 0x64, 0x24, 0x65]);
        assert_eq!(&link[32..], &[0x84, 0x62, 0x01, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x00]);

        assert_eq!(confirm.len(), 36);
        assert_eq!(confirm[0], 0x1b);
        assert_eq!(&confirm[12..14], &[0xf9, 0x6f], "ctr = link_ctr + 0x31");
        assert_eq!(&confirm[24..32], &[0x83, 0x66, 0xcd, 0x04, 0x0c, 0x64, 0x21, 0x65]);
        assert_eq!(&confirm[32..], &[0x81, 0x66, 0x01, 0x00], "N = footswitch_number brut = 1");
    }

    /// Même capture, 2e contrôle (frames 12543/12563/12573) — Level (param_selector=5) sur slot
    /// bus=02, FS2 choisi → `4a`=2+2=4, Confirm N=2. Ordre de création=2 mais N suit le FS, pas
    /// l'ordre (confirmé sans ambiguïté par une capture ultérieure décorrélée, cf test ci-dessous).
    #[test]
    fn builds_expected_bytes_for_create_real_param_level_slot2_fs2() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x7048;
        state.live_write_yy = 0x0e;
        let (create, link, confirm) =
            build_controller_create_real_param_write_packets(&mut state, 0x02, 0x05, 2);

        assert_eq!(
            &create[32..],
            &[0x87, 0x62, 0x02, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, 0x05, 0x4a, 0x04, 0x47, 0x04, 0xcc, 0x81, 0xc2]
        );
        assert_eq!(&link[32..], &[0x84, 0x62, 0x02, 0x1d, 0xc3, 0x1a, 0x00, 0x1c, 0x05, 0x00, 0x00, 0x00]);
        assert_eq!(&confirm[32..], &[0x81, 0x66, 0x02, 0x00]);
    }

    /// `add_FS1_slot8_add_FS8_slot4.json` — capture DÉLIBÉRÉMENT décorrélée (bus, ordre de
    /// création et FS tous différents) qui a permis de trancher : `4a`/`N` suivent le FS choisi,
    /// PAS le bus ni l'ordre de création. 1er contrôle créé = FS1 sur slot bus=08 (pas FS8/FS-ordre1).
    #[test]
    fn create_ordinal_and_confirm_n_follow_chosen_footswitch_not_bus_or_creation_order() {
        let mut state = HelixState::new();
        state.live_write_ctr = 0x7cb4;
        state.live_write_yy = 0x54;
        let (create, _link, confirm) =
            build_controller_create_real_param_write_packets(&mut state, 0x08, 0x00, 1);
        assert_eq!(create[34], 0x08, "bus du bloc contrôlé = slot8");
        assert_eq!(create[42], 0x03, "4a = FS1+2 = 3, PAS lié au bus (8) ni à l'ordre (1er)");
        assert_eq!(confirm[34], 0x01, "N = FS1 brut");

        // 2e contrôle : FS8 sur slot bus=04 (2e créé, mais N/4a suivent le FS=8, pas l'ordre=2).
        let mut state2 = HelixState::new();
        state2.live_write_ctr = 0x7d65;
        state2.live_write_yy = 0x58;
        let (create2, _link2, confirm2) =
            build_controller_create_real_param_write_packets(&mut state2, 0x04, 0x05, 8);
        assert_eq!(create2[34], 0x04, "bus du bloc contrôlé = slot4");
        assert_eq!(create2[42], 0x0a, "4a = FS8+2 = 10, PAS lié au bus (4) ni à l'ordre (2e)");
        assert_eq!(confirm2[34], 0x08, "N = FS8 brut");
    }

    #[test]
    fn param_selector_byte_reflects_chosen_parameter() {
        let mut state = HelixState::new();
        let (create, link, _confirm) =
            build_controller_create_real_param_write_packets(&mut state, 0x05, 0x07, 3);
        assert_eq!(create[39], 0x1c);
        assert_eq!(create[40], 0x07, "param_selector = index du paramètre choisi");
        assert_eq!(create[34], 0x05, "bus du bloc contrôlé");
        assert_eq!(link[40], 0x07, "le paquet 0x24 porte le même param_selector que le 0x25");
    }

    #[test]
    fn create_link_confirm_trio_advances_shared_live_write_counters_by_0x31_each_step() {
        let mut state = HelixState::new();
        let ctr_before = state.live_write_ctr;
        let yy_before = state.live_write_yy;
        let _ = build_controller_create_real_param_write_packets(&mut state, 0x01, 0x00, 1);
        // Après le trio : confirm_ctr(create_ctr+0x62) + 0x44, confirm_yy(create_yy+2) + 1 —
        // estimation non isolée pour l'état APRÈS le trio (cf doc de fonction).
        let confirm_ctr = ctr_before.wrapping_add(0x62);
        assert_eq!(state.live_write_ctr, confirm_ctr.wrapping_add(0x44));
        assert_eq!(state.live_write_yy, yy_before.wrapping_add(2).wrapping_add(1));
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

    /// Fix 2026-07-12 : le 3e octet du payload Confirm doit suivre `footswitch_number`, pas rester
    /// figé à `1` — non re-capturé spécifiquement pour Bypass (cf doc module), mais même paquet
    /// `term=0x21` que le trio de création vrai-paramètre où c'est confirmé 12/12. Avant ce fix,
    /// cette assertion aurait échoué (`0x01` en dur quel que soit `footswitch_number`).
    #[test]
    fn bypass_confirm_n_follows_footswitch_number_not_hardcoded_one() {
        let mut state = HelixState::new();
        let (_source, confirm_pkt) =
            build_controller_source_and_confirm_write_packets(&mut state, 0x01, 4);
        assert_eq!(&confirm_pkt[32..36], &[0x81, 0x66, 0x04, 0x00]);
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
