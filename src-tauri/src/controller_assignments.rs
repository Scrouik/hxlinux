//! Décodeur des assignations Command Center (onglet « Controllers », switchs/pédales
//! d'expression) embarquées dans le dump preset `ed:03`. Format binaire MessagePack (voir
//! captures Wireshark du 2026-07-08) : mini-décodeur maison, pas de crate externe, dans le même
//! esprit hand-rolled que `preset_chain_params::read_params_hex`.
//!
//! Chaque assignation créée dans HX Edit produit deux groupes `9N:<tag>` qui se suivent dans le
//! dump, `N` = nombre de paramètres contrôlés par CE switch (`91` = 1, `92` = 2, etc.) :
//! - Groupe 1 (`9N:87`) : un élément `87` par paramètre contrôlé — nom réel du paramètre, bus du
//!   bloc contrôlé, Type, nom personnalisé (Customize), couleur.
//! - Groupe 2 (`9N:82`) : un élément `82` par paramètre contrôlé — source (switch), Min/Max bruts.
//!
//! Les groupes 1 et 2 se correspondent dans l'ordre d'apparition et ont la même cardinalité N.

use crate::msgpack_lite::{map_get, parse_value, Value};
use serde::Serialize;

/// Une assignation Command Center décodée (un couple bloc/paramètre <-> switch).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerAssignment {
    /// Bus Kempline (0x01-0x08 Path1, 0x0b-0x12 Path2, ou bus spécial I/O) du bloc contrôlé.
    pub slot_bus: u8,
    /// Index grille Kempline (0..15) du bloc contrôlé, `None` si bus spécial (Input/Output/Split/Merge).
    pub kempline_slot_index: Option<usize>,
    /// Nom réel du paramètre catalogue (immuable, ex. "Drive") — fiable même si `custom_name` diffère.
    /// Vaut littéralement `"Bypass"` quand `is_bypass` est vrai (le fil ne porte pas de nom de
    /// paramètre pour ce cas, voir `is_bypass`).
    pub param_name: String,
    /// Nom personnalisé (Customize) si défini, `None` sinon (utiliser `param_name` par défaut).
    pub custom_name: Option<String>,
    /// `false` = Latching, `true` = Momentary.
    pub momentary: bool,
    /// Index couleur switch LED (0 = Autocolor, voir liste front `CONTROLLERS_LED_COLORS`).
    pub color_index: u8,
    /// Source : 0 = None, 1 = EXP Pedal 1, 2 = EXP Pedal 2, 3+ = Footswitch (source - 2).
    /// Pour Bypass, toujours un footswitch (jamais de pédale EXP, voir `is_bypass`).
    pub source: u8,
    /// Bornes Min/Max en unités **brutes catalogue** du paramètre (avant conversion d'affichage).
    /// Sans signification pour Bypass (`is_bypass=true`) — valent `0.0`, à ignorer côté front.
    pub min_raw: f32,
    pub max_raw: f32,
    /// `true` si cette assignation contrôle l'activation/désactivation du bloc entier (« Bypass »)
    /// plutôt qu'un vrai paramètre catalogue. Format wire différent (voir `decode_bypass_slots`) :
    /// pas de Bloc2 (Source encodée par la POSITION dans un tableau de 8 emplacements, un par
    /// footswitch — jamais assignable à une pédale EXP), pas de Min/Max/Snapshot Control.
    pub is_bypass: bool,
}


/// Champs utiles du groupe 1 (`87`, nom réel + bus + Type + couleur + nom personnalisé).
struct Block1Fields {
    param_name: String,
    slot_bus: u8,
    momentary: bool,
    custom_name: Option<String>,
    color_index: u8,
}

fn decode_block1_element(el: &Value) -> Option<Block1Fields> {
    let outer = el.as_map()?;
    let nested = map_get(outer, 0x0b)?.as_map()?;
    // Clé imbriquée `0x00` : `2` = vrai paramètre catalogue, `1` = assignation Bypass (structure
    // différente, décodée séparément par `decode_bypass_slot_element` — pas de nom de paramètre
    // ni de Bloc2 pour ce cas, donc on ne le traite pas ici pour éviter un `param_name` erroné
    // (le nom porté à la clé `0x05` serait alors celui du bloc, pas un paramètre).
    if map_get(nested, 0x00)?.as_int()? != 2 {
        return None;
    }
    let param_name = map_get(nested, 0x05)?.as_str()?.to_string();
    let slot_bus = map_get(nested, 0x08)?.as_int()? as u8;
    let momentary = map_get(outer, 0x0c)?.as_bool()?;
    let color_index = map_get(outer, 0x10)?.as_int()? as u8;
    let custom_name = map_get(outer, 0x0e)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(Block1Fields {
        param_name,
        slot_bus,
        momentary,
        custom_name,
        color_index,
    })
}

/// Champs utiles du groupe 2 (`82`, source + Min/Max bruts + bus, pour appariement avec le groupe 1).
struct Block2Fields {
    bus: u8,
    source: u8,
    min_raw: f32,
    max_raw: f32,
}

fn decode_block2_element(el: &Value) -> Option<Block2Fields> {
    let outer = el.as_map()?;
    let nested = map_get(outer, 0x01)?.as_map()?;
    let source = map_get(nested, 0x00)?.as_int()? as u8;
    let bus = map_get(nested, 0x05)?.as_int()? as u8;
    let min_raw = map_get(nested, 0x02)?.as_float()?;
    let max_raw = map_get(nested, 0x03)?.as_float()?;
    Some(Block2Fields {
        bus,
        source,
        min_raw,
        max_raw,
    })
}

/// Nombre d'emplacements du tableau Bypass (un par footswitch physique, jamais une pédale EXP).
const BYPASS_SLOTS_COUNT: usize = 8;

/// Décode un élément peuplé du tableau Bypass (`91:87:{...}`, structure identique au Bloc1 d'un
/// vrai paramètre mais nested clé `0x00=1` au lieu de `2`, et pas de clé `0x05` utile — le nom
/// porté là est celui du BLOC, pas un nom de paramètre, donc ignoré au profit de `"Bypass"` en dur).
fn decode_bypass_slot_element(el: &Value) -> Option<ControllerAssignment> {
    let outer = el.as_map()?;
    let nested = map_get(outer, 0x0b)?.as_map()?;
    let kind = map_get(nested, 0x00)?.as_int()?;
    if kind != 1 {
        return None; // pas une structure Bypass (nested key 0x00=2 => vrai paramètre, pas notre cas)
    }
    let slot_bus = map_get(nested, 0x08)?.as_int()? as u8;
    let momentary = map_get(outer, 0x0c)?.as_bool()?;
    let color_index = map_get(outer, 0x10)?.as_int()? as u8;
    let custom_name = map_get(outer, 0x0e)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(ControllerAssignment {
        slot_bus,
        kempline_slot_index: crate::helix::slot_bus_to_kempline_index(slot_bus),
        param_name: "Bypass".to_string(),
        custom_name,
        momentary,
        color_index,
        source: 0, // renseigné par l'appelant (position dans le tableau de 8 = footswitch)
        min_raw: 0.0,
        max_raw: 0.0,
        is_bypass: true,
    })
}

/// Tente de décoder un tableau de 8 emplacements footswitch (`98:<8 éléments, chacun `nil` ou
/// `9187:{...}`>`) commençant en `data[pos]`. Retourne les assignations **Bypass** trouvées (un
/// footswitch peut être vide, ou porter un vrai paramètre catalogue — cf. `BASELINE_DRIVE_FS1` où
/// ce même tableau contient "Drive" — auquel cas on l'ignore ici : il est déjà couvert par le scan
/// générique du Bloc1) + le nombre d'octets consommés. `None` dès que la shape ne correspond pas
/// exactement (pas de faux positif silencieux sur un fixarray(8) non lié).
fn try_decode_bypass_slots(data: &[u8], pos: usize) -> Option<(Vec<ControllerAssignment>, usize)> {
    if data.get(pos).copied()? != 0x90 | (BYPASS_SLOTS_COUNT as u8) {
        return None;
    }
    let mut cursor = pos + 1;
    let mut found = Vec::new();
    for fs_index in 0..BYPASS_SLOTS_COUNT {
        match data.get(cursor).copied()? {
            0xc0 => cursor += 1, // emplacement vide (pas d'assignation sur ce switch pour ce bloc)
            0x91 => {
                cursor += 1;
                if data.get(cursor).copied() != Some(0x87) {
                    return None;
                }
                let val = parse_value(data, &mut cursor)?;
                // `None` = emplacement occupé par un vrai paramètre (kind nested 0x00=2), pas du
                // Bypass — on continue le balayage du tableau plutôt que d'abandonner tout le
                // groupe (un bloc peut cumuler un vrai paramètre sur un switch et Bypass sur un
                // autre, dans ce même tableau de 8).
                if let Some(mut a) = decode_bypass_slot_element(&val) {
                    // Source = position dans le tableau : Footswitch (fs_index + 1), même
                    // convention que les vrais paramètres (source = numéro_footswitch + 2,
                    // encodage 0=None, 1=EXP1, 2=EXP2, 3+=FS(source-2)).
                    a.source = (fs_index as u8) + 3;
                    found.push(a);
                }
            }
            _ => return None,
        }
    }
    Some((found, cursor - pos))
}

/// Tente de décoder un groupe `9N:<element_tag>{...}` commençant en `data[pos]`. Retourne les
/// éléments décodés + le nombre d'octets consommés. `None` dès que la shape ne correspond pas
/// exactement à ce qu'on attend (pas de faux positif silencieux sur un fixarray non lié).
fn try_decode_group<T>(
    data: &[u8],
    pos: usize,
    element_tag: u8,
    decode_element: impl Fn(&Value) -> Option<T>,
) -> Option<(Vec<T>, usize)> {
    let tag = *data.get(pos)?;
    if !(0x91..=0x9f).contains(&tag) {
        return None;
    }
    let n = (tag & 0x0f) as usize;
    let mut cursor = pos + 1;
    let mut elements = Vec::with_capacity(n);
    for _ in 0..n {
        if data.get(cursor).copied() != Some(element_tag) {
            return None;
        }
        let val = parse_value(data, &mut cursor)?;
        elements.push(decode_element(&val)?);
    }
    Some((elements, cursor - pos))
}

/// Scanne tout le dump preset et retourne toutes les assignations Command Center trouvées.
/// Balayage tolérant (position par position) : les faux départs (shape non conforme) sont ignorés
/// et le scan reprend un octet plus loin, sans jamais paniquer sur un dump non lié.
///
/// Important : les éléments du groupe 1 (`87`, un par switch+paramètre) et du groupe 2 (`82`,
/// Source/Min/Max) ne sont **pas** dans le même ordre — vérifié sur une capture à 2 paramètres
/// sur le même switch (groupe 1 = [Gain, Drive], groupe 2 = [Drive, Gain]). L'appariement se fait
/// donc par le bus du bloc contrôlé (groupe 1 clé `0x08` == groupe 2 clé `0x05`), pas par position.
pub fn scan_controller_assignments(data: &[u8]) -> Vec<ControllerAssignment> {
    let mut block1: Vec<Block1Fields> = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        match try_decode_group(data, pos, 0x87, decode_block1_element) {
            Some((group, consumed)) if !group.is_empty() => {
                block1.extend(group);
                pos += consumed;
            }
            _ => pos += 1,
        }
    }

    let mut block2: Vec<Block2Fields> = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        match try_decode_group(data, pos, 0x82, decode_block2_element) {
            Some((group, consumed)) if !group.is_empty() => {
                block2.extend(group);
                pos += consumed;
            }
            _ => pos += 1,
        }
    }

    // File par bus : consomme dans l'ordre d'apparition les éléments du groupe 2 partageant le
    // même bus qu'un élément du groupe 1 (gère le cas — rare — de plusieurs assignations sur le
    // même bus, tant que leur ordre relatif se correspond entre les deux groupes).
    let mut by_bus: std::collections::HashMap<u8, std::collections::VecDeque<Block2Fields>> =
        std::collections::HashMap::new();
    for b2 in block2 {
        by_bus.entry(b2.bus).or_default().push_back(b2);
    }

    let mut out = Vec::new();
    for b1 in block1 {
        let Some(queue) = by_bus.get_mut(&b1.slot_bus) else {
            continue; // pas de contrepartie groupe 2 trouvée pour ce bus : on ignore plutôt que deviner.
        };
        let Some(b2) = queue.pop_front() else {
            continue;
        };
        out.push(ControllerAssignment {
            slot_bus: b1.slot_bus,
            kempline_slot_index: crate::helix::slot_bus_to_kempline_index(b1.slot_bus),
            param_name: b1.param_name,
            custom_name: b1.custom_name,
            momentary: b1.momentary,
            color_index: b1.color_index,
            source: b2.source,
            min_raw: b2.min_raw,
            max_raw: b2.max_raw,
            is_bypass: false,
        });
    }

    // Bypass suit un format à part (voir `try_decode_bypass_slots`) : pas de Bloc2, Source encodée
    // par la position dans un tableau de 8 emplacements (un par footswitch), scanné séparément.
    let mut pos = 0usize;
    while pos < data.len() {
        match try_decode_bypass_slots(data, pos) {
            Some((group, consumed)) => {
                out.extend(group);
                pos += consumed;
            }
            None => pos += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        hex.split(':')
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    /// Capture `Controllers_read_preset_with_controller.json` (Drive assigné à Footswitch 1) —
    /// payload utile isolé (frames 2909+2913, sans les 16 octets d'en-tête USB/ED03 de chaque
    /// frame). Min/Max ne sont pas à 0.0/1.0 « par défaut » : ce preset avait déjà été légèrement
    /// ajusté par un test précédent dans la même session (0.4/0.54), valeurs vérifiées ci-dessous.
    const BASELINE_DRIVE_FS1: &str = concat!(
        "95:81:0c:83:02:00:03:00:04:90:82:13:01:14:82:06:01:07:83:02:02:03:02:04:92:ca:3f:00:00:00:",
        "ca:c0:19:99:9a:82:13:02:14:83:0e:82:05:00:07:83:02:00:03:00:04:90:0f:84:08:cd:01:00:0d:00:",
        "0a:c3:07:83:02:02:03:02:04:92:ca:3f:00:00:00:c2:12:c2:82:13:08:14:c0:82:13:08:14:c0:82:13:",
        "08:14:c0:82:13:08:14:c0:82:13:08:14:c0:82:13:08:14:c0:82:13:08:14:c0:82:13:08:14:c0:82:13:",
        "03:14:83:10:82:06:00:07:83:02:00:03:00:04:90:11:84:08:cc:97:0d:00:0a:c3:07:83:02:06:03:06:",
        "04:96:ca:00:00:00:00:ca:3f:00:00:00:ca:00:00:00:00:ca:3f:00:00:00:c2:ca:00:00:00:00:12:c2:",
        "01:c0:03:82:07:02:08:98:91:87:0a:00:0b:87:00:02:05:a6:44:72:69:76:65:00:06:ce:00:07:10:0c:",
        "07:c2:08:01:02:00:09:83:1c:00:1d:00:29:c2:0c:c2:0e:a1:00:0d:c2:10:00:0f:c2:c0:c0:c0:c0:c0:",
        "c0:c0:04:9d:c0:c0:c0:91:82:00:00:01:89:00:03:01:04:02:ca:3e:cc:cc:cd:03:ca:3f:0a:3d:71:04:",
        "00:05:01:06:83:1c:00:1d:00:29:c2:07:00:0d:c2",
    );

    #[test]
    fn decodes_baseline_drive_footswitch1() {
        let data = hex_to_bytes(BASELINE_DRIVE_FS1);
        let found = scan_controller_assignments(&data);
        assert_eq!(found.len(), 1, "should find exactly one assignment");
        let a = &found[0];
        assert_eq!(a.slot_bus, 0x01);
        assert_eq!(a.kempline_slot_index, Some(0));
        assert_eq!(a.param_name, "Drive");
        assert_eq!(a.custom_name, None);
        assert!(!a.momentary);
        assert_eq!(a.color_index, 0);
        assert_eq!(a.source, 3); // Footswitch 1 (3 = 1 + 2, encodage unifié 0=None,1=EXP1,2=EXP2)
        assert!((a.min_raw - 0.4).abs() < 1e-4);
        assert!((a.max_raw - 0.54).abs() < 1e-4);
    }

    /// Capture `Controllers_read_preset_with_controller_name_Test123.json` (frames 1983+1987) —
    /// Customize renommé, vérifie que `param_name` reste "Drive" (immuable) et `custom_name`
    /// reflète le renommage.
    const RENAMED_TEST123: &str = concat!(
        "91:87:0a:00:0b:87:00:02:05:a6:44:72:69:76:65:00:06:ce:00:07:10:0c:07:c2:08:01:02:00:09:83:",
        "1c:00:1d:00:29:c2:0c:c2:0e:a8:54:65:73:74:31:32:33:00:0d:c3:10:00:0f:c2:c0:c0:c0:c0:c0:c0:",
        "c0:04:9d:c0:c0:c0:91:82:00:00:01:89:00:03:01:04:02:ca:3f:00:00:00:03:ca:3f:33:33:33:04:00:",
        "05:01:06:83:1c:00:1d:00:29:c2:07:00:0d:c2",
    );

    #[test]
    fn keeps_real_param_name_when_custom_name_set() {
        let data = hex_to_bytes(RENAMED_TEST123);
        let found = scan_controller_assignments(&data);
        assert_eq!(found.len(), 1);
        let a = &found[0];
        assert_eq!(a.param_name, "Drive");
        assert_eq!(a.custom_name.as_deref(), Some("Test123"));
        assert!((a.min_raw - 0.5).abs() < 1e-6);
        assert!((a.max_raw - 0.7).abs() < 1e-5);
    }

    /// Capture `Controllers_read_preset_with_2controllers_same_FS.json` (frames 1661+1665) — même
    /// switch, 2 paramètres (tableau `92:87:...:87:...` / `92:82:...:82:...`). Vérifié
    /// octet-par-octet : l'ordre des éléments du groupe 2 (Drive, Gain) est **inversé** par
    /// rapport au groupe 1 (Gain, Drive) — d'où l'appariement par bus plutôt que par position.
    const TWO_PARAMS_SAME_SWITCH: &str = concat!(
        "92:87:0a:07:0b:87:00:02:05:a5:47:61:69:6e:00:06:ce:00:07:10:0c:07:c2:08:05:02:00:09:83:1c:",
        "00:1d:00:29:c2:0c:c2:0e:a8:54:65:73:74:31:32:33:00:0d:c3:10:00:0f:c2:",
        "87:0a:00:0b:87:00:02:05:a6:44:72:69:76:65:00:06:ce:00:07:10:0c:07:c2:08:01:02:00:09:83:1c:",
        "00:1d:00:29:c2:0c:c2:0e:a8:54:65:73:74:31:32:33:00:0d:c3:10:00:0f:c2:",
        "92:82:00:00:01:89:00:03:01:04:02:ca:3f:00:00:00:03:ca:3f:33:33:33:04:00:05:01:06:83:1c:00:",
        "1d:00:29:c2:07:00:0d:c2:",
        "82:00:01:01:89:00:03:01:04:02:ca:c1:70:00:00:03:ca:41:70:00:00:04:00:05:05:06:83:1c:00:1d:",
        "00:29:c2:07:00:0d:c2",
    );

    #[test]
    fn decodes_two_params_on_same_switch_with_reversed_order() {
        let data = hex_to_bytes(TWO_PARAMS_SAME_SWITCH);
        let found = scan_controller_assignments(&data);
        assert_eq!(found.len(), 2, "should find both assignments from the size-2 arrays");
        let gain = found.iter().find(|a| a.param_name == "Gain").expect("Gain assignment");
        assert_eq!(gain.slot_bus, 0x05);
        assert_eq!(gain.custom_name.as_deref(), Some("Test123"));
        assert_eq!(gain.source, 3);
        assert!((gain.min_raw - (-15.0)).abs() < 1e-3);
        assert!((gain.max_raw - 15.0).abs() < 1e-3);

        let drive = found.iter().find(|a| a.param_name == "Drive").expect("Drive assignment");
        assert_eq!(drive.slot_bus, 0x01);
        assert_eq!(drive.custom_name.as_deref(), Some("Test123"));
        assert_eq!(drive.source, 3);
        assert!((drive.min_raw - 0.5).abs() < 1e-6);
        assert!((drive.max_raw - 0.7).abs() < 1e-5);
    }

    /// Capture `preset_control_bypass.json` (2026-07-09) — Bypass assigné à Footswitch 1 (premier
    /// emplacement du tableau de 8), nom personnalisé "MonByPass". Format à part, pas de Bloc2 :
    /// tableau `98` (8 emplacements), le premier peuplé, les 7 autres `nil`.
    const BYPASS_FOOTSWITCH1_CUSTOM_NAME: &str = concat!(
        "98:91:87:0a:07:0b:85:00:01:05:ac:44:65:6c:75:78:65:20:43:6f:6d:70:00:06:ce:00:84:ff:00:07:",
        "c3:08:01:0c:c2:0e:aa:4d:6f:6e:42:79:50:61:73:73:00:0d:c3:10:00:0f:c2:c0:c0:c0:c0:c0:c0:c0",
    );

    #[test]
    fn decodes_bypass_on_footswitch1_with_custom_name() {
        let data = hex_to_bytes(BYPASS_FOOTSWITCH1_CUSTOM_NAME);
        let found = scan_controller_assignments(&data);
        assert_eq!(found.len(), 1, "should find exactly the bypass assignment");
        let a = &found[0];
        assert!(a.is_bypass);
        assert_eq!(a.param_name, "Bypass");
        assert_eq!(a.slot_bus, 0x01);
        assert_eq!(a.kempline_slot_index, Some(0));
        assert_eq!(a.custom_name.as_deref(), Some("MonByPass"));
        assert!(!a.momentary);
        assert_eq!(a.color_index, 0);
        assert_eq!(a.source, 3); // Footswitch 1 (position 0 dans le tableau)
    }

    /// Capture `preset_control_bypass_FS5.json` (2026-07-09) — même Bypass, sans nom personnalisé,
    /// déplacé au 5e emplacement du tableau (index 4) — confirme que la Source vient bien de la
    /// POSITION dans le tableau, pas d'un champ dédié (contrairement aux vrais paramètres).
    const BYPASS_FOOTSWITCH5_NO_CUSTOM_NAME: &str = concat!(
        "98:c0:c0:c0:c0:91:87:0a:07:0b:85:00:01:05:ac:44:65:6c:75:78:65:20:43:6f:6d:70:00:06:ce:00:",
        "84:ff:00:07:c3:08:01:0c:c2:0e:a1:00:0d:c2:10:00:0f:c2:c0:c0:c0",
    );

    #[test]
    fn decodes_bypass_on_footswitch5_from_array_position() {
        let data = hex_to_bytes(BYPASS_FOOTSWITCH5_NO_CUSTOM_NAME);
        let found = scan_controller_assignments(&data);
        assert_eq!(found.len(), 1);
        let a = &found[0];
        assert!(a.is_bypass);
        assert_eq!(a.param_name, "Bypass");
        assert_eq!(a.slot_bus, 0x01);
        assert_eq!(a.custom_name, None);
        assert_eq!(a.source, 7); // Footswitch 5 (position 4 dans le tableau, 4 + 3 = 7)
    }

    /// Le même tableau de 8 emplacements peut contenir un VRAI paramètre (kind nested `0x00=2`,
    /// déjà couvert par `BASELINE_DRIVE_FS1`) : vérifie qu'un tel emplacement n'est pas
    /// (ré-)interprété comme du Bypass et ne produit pas de doublon.
    #[test]
    fn real_param_inside_footswitch_array_is_not_treated_as_bypass() {
        let data = hex_to_bytes(BASELINE_DRIVE_FS1);
        let found = scan_controller_assignments(&data);
        assert!(
            !found.iter().any(|a| a.is_bypass),
            "no bypass entry expected, only the real Drive parameter"
        );
    }
}
