//! Valeurs de paramètres PAR SNAPSHOT (« Snapshot Control »).
//!
//! Reverse-engineeré 2026-07-13 (captures `add_drive_to_snapshot`, `Drive_SS1_..`,
//! `Snapshot_2_param`). Modèle : un paramètre devient « contrôlé par snapshot » quand il est
//! assigné à la **source "Snapshot"** dans le Command Center (énumération source : 0=None, 1/2=EXP,
//! 3-10=FS1-8, **12=Snapshot**). Ces assignations n'ont QUE le groupe 2 (`9N:82`, pas de groupe 1 :
//! ni nom ni couleur — ce n'est pas un footswitch). Chaque élément porte : source (nested 0x00),
//! bus (0x05), Min/Max (0x02/0x03), et **index de param** (nested 0x06 → sous-clé 0x1d).
//!
//! Les VALEURS par snapshot sont stockées dans chaque bloc `SNAPSHOT N` du dump, sous forme d'un
//! **tableau positionnel de triplets** `93 [bool, type, valeur_f32|nil]` (le run se termine juste
//! avant le marqueur `SNAPSHOT N\0`). **La position N ↔ le Nᵉ paramètre source=Snapshot** (ordre
//! d'apparition dans le groupe 2). Vérifié vérité terrain : Level slot2/param5(idx4) en pos0,
//! Tone slot1/param2(idx1) en pos1 ; valeurs 2/4/6/8 (Level, dB brut) et 0.2/0.4/0.6/0.8 (Tone).

use crate::msgpack_lite::{map_get, parse_value, Value};
use serde::Serialize;

/// Valeur de la source « Snapshot » dans l'énumération source (champ `4a` des assignations, ou
/// nested `0x00` du groupe 2). Confirmé capture `add_drive_to_snapshot` (`4a=0x0c`).
pub const SNAPSHOT_SOURCE: u8 = 12;
const SNAPSHOT_COUNT: usize = 4;

/// Un paramètre contrôlé par snapshot + ses 4 valeurs (une par snapshot).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotParamValues {
    /// Bus Kempline du bloc contrôlé.
    pub slot_bus: u8,
    /// Index grille Kempline (0..15) du bloc, `None` si bus spécial.
    pub kempline_slot_index: Option<usize>,
    /// Index wire du paramètre (même convention que `param_selector`).
    pub param_index: u8,
    pub min_raw: f32,
    pub max_raw: f32,
    /// Valeur brute par snapshot (4 entrées, snapshots 1..4). `None` = pas de valeur propre à ce
    /// snapshot (nil dans le triplet = utilise la valeur de base).
    pub values: Vec<Option<f32>>,
}

/// Réf d'un paramètre source=Snapshot dans le groupe 2 (ordre d'apparition = ordre position triplet).
struct SnapshotSourceRef {
    bus: u8,
    param_index: u8,
    min_raw: f32,
    max_raw: f32,
}

/// Décode un élément `82` du groupe 2 comme réf Snapshot — `None` si la source n'est pas Snapshot.
fn decode_snapshot_source_element(el: &Value) -> Option<SnapshotSourceRef> {
    let outer = el.as_map()?;
    let nested = map_get(outer, 0x01)?.as_map()?;
    if map_get(nested, 0x00)?.as_int()? as u8 != SNAPSHOT_SOURCE {
        return None;
    }
    let bus = map_get(nested, 0x05)?.as_int()? as u8;
    let min_raw = map_get(nested, 0x02)?.as_float()?;
    let max_raw = map_get(nested, 0x03)?.as_float()?;
    // Index de param : nested clé 0x06 est une map dont la sous-clé 0x1d (29) porte l'index.
    let idx_map = map_get(nested, 0x06)?.as_map()?;
    let param_index = map_get(idx_map, 0x1d)?.as_int()? as u8;
    Some(SnapshotSourceRef {
        bus,
        param_index,
        min_raw,
        max_raw,
    })
}

/// Scanne tous les groupes `9N:82` du dump et retourne, DANS L'ORDRE d'apparition, les éléments dont
/// la source = Snapshot. Balayage tolérant (position par position), comme `scan_controller_assignments`.
fn scan_snapshot_source_params(data: &[u8]) -> Vec<SnapshotSourceRef> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let tag = data[pos];
        if (0x91..=0x9f).contains(&tag) {
            let n = (tag & 0x0f) as usize;
            let mut cursor = pos + 1;
            let mut elems = Vec::with_capacity(n);
            let mut shape_ok = true;
            for _ in 0..n {
                if data.get(cursor).copied() != Some(0x82) {
                    shape_ok = false;
                    break;
                }
                match parse_value(data, &mut cursor) {
                    Some(v) => elems.push(v),
                    None => {
                        shape_ok = false;
                        break;
                    }
                }
            }
            // On ne considère le groupe que si TOUS ses éléments sont des `82` bien formés ET qu'au
            // moins un est une source Snapshot (évite d'absorber un `9N:82` non lié).
            if shape_ok && !elems.is_empty() {
                let refs: Vec<SnapshotSourceRef> = elems
                    .iter()
                    .filter_map(decode_snapshot_source_element)
                    .collect();
                if !refs.is_empty() {
                    out.extend(refs);
                    pos = cursor;
                    continue;
                }
            }
        }
        pos += 1;
    }
    out
}

/// Décode un champ nom de snapshot à la position `pos` si `pos` pointe sur la clé `0x04` d'un champ
/// `04 <fixstr tag 0xa1..0xbf> <texte> 00 0e` — Some((nom_sans_nul, position_après_0x0e)).
///
/// Cette signature STRUCTURELLE est name-agnostic ET tempo-agnostic : elle ne dépend ni du texte du
/// nom (défaut "SNAPSHOT N" ou custom), ni des octets de payload qui suivent (tempo, triplets…).
/// L'ancienne ancre « octets fixes » (`05 ca 42 f0 … 88 00 c3 01 9e`) était FAUSSE : ses 5 derniers
/// octets appartenaient au PAYLOAD du snapshot (identique pour les snaps 1/2/3 non modifiés, mais
/// DIFFÉRENT pour un snapshot modifié — ex. Level=-30 sur le snap4 de `read_snapshot_preset_linux`),
/// donc elle ratait tout snapshot modifié. Vérifié : la signature structurelle trouve exactement 4
/// champs (snap 1..4) sur `read_snapshot_preset(_linux)` et `read_preset_named_snap`.
fn decode_snapshot_name_field_at(data: &[u8], pos: usize) -> Option<(String, usize)> {
    if data.get(pos).copied() != Some(0x04) {
        return None;
    }
    let tag = *data.get(pos + 1)?;
    if !(0xa1..=0xbf).contains(&tag) {
        return None;
    }
    let declared_len = (tag - 0xa0) as usize; // longueur fixstr, nul final inclus
    if declared_len == 0 {
        return None;
    }
    let null_pos = pos + 1 + declared_len; // dernier octet du fixstr = nul terminateur
    if data.get(null_pos).copied() != Some(0x00) {
        return None;
    }
    // La clé suivante DOIT être 0x0e (signature du bloc nom de snapshot) — écarte les autres fixstr.
    if data.get(null_pos + 1).copied() != Some(0x0e) {
        return None;
    }
    let text_bytes = &data[pos + 2..null_pos];
    let name = std::str::from_utf8(text_bytes).ok()?.to_string();
    Some((name, null_pos + 2))
}

/// Positions (clé `0x04`) de tous les champs nom de snapshot du dump, dans l'ordre d'apparition —
/// la Nᵉ position correspond au snapshot N (1-based), l'ordre physique des 4 blocs étant fixe.
fn snapshot_name_field_positions(data: &[u8]) -> Vec<usize> {
    if data.len() < 4 {
        return Vec::new();
    }
    (0..data.len())
        .filter(|&i| decode_snapshot_name_field_at(data, i).is_some())
        .collect()
}

/// Position du bloc `SNAPSHOT n` (n = 1..4) via la signature structurelle du champ nom. `None` si
/// absent. Retourne la position de la clé `0x04` (le run de triplets du bloc est AVANT, cf
/// `snapshot_block_triplet_values` qui balaye `[pos-450, pos)`).
fn snapshot_marker_pos(data: &[u8], snap_num: usize) -> Option<usize> {
    snapshot_name_field_positions(data)
        .get(snap_num.checked_sub(1)?)
        .copied()
}

/// Nom du snapshot n (1..4) : le nom custom s'il est décodable (non vide), sinon le défaut
/// `"Snapshot N"` (HX Edit affiche par défaut `"SNAPSHOT N"`, on garde la casse de notre UI).
pub fn snapshot_name(data: &[u8], snap_num: usize) -> String {
    snapshot_marker_pos(data, snap_num)
        .and_then(|pos| decode_snapshot_name_field_at(data, pos))
        .map(|(name, _)| name)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("Snapshot {snap_num}"))
}

/// Les 4 noms de snapshot du preset actif (index 0 = Snapshot 1 … index 3 = Snapshot 4).
pub fn scan_snapshot_names(data: &[u8]) -> Vec<String> {
    (1..=SNAPSHOT_COUNT).map(|n| snapshot_name(data, n)).collect()
}

/// Snapshot actif (0-based, 0..=3) stocké dans l'en-tête du preset : map top-level → clé `0x68`
/// (métadonnées preset) → clé `0x5c`. C'est la MÊME clé `0x5c` que les paquets d'activation/renommage
/// (`81 5c <idx>` / `82 5c <idx>`). Vérifié sur captures : preset neuf (`no_snap`) = 0 (Snapshot 1) ;
/// "Snap 4" = 2/3 selon l'état. `None` si l'en-tête n'est pas décodable — l'appelant retombe alors
/// sur 0 (Snapshot 1). HXLinux supposait AVANT toujours 0, d'où l'UI positionnée à tort sur Snap 1
/// alors que le device était parqué ailleurs (bug signalé 2026-08-06).
pub fn active_snapshot_index(data: &[u8]) -> Option<u8> {
    // Le début du dump content_only contient PLUSIEURS petits frames `83 66 cd .. 67 .. 68 ..` : les
    // premiers sont des PRÉAMBULES (`0x68` = nil `c0`, ou sous-map `82` sans `0x5c`) ; la VRAIE map
    // d'en-tête (avec le nom `0x6d` et `0x5c`) arrive PLUS LOIN (offset ~112 vérifié sur dump réel de
    // notre app, pas dans les 64 premiers octets). On itère donc tous les fixmap `8N` de 1re clé `0x66`
    // dans une large fenêtre et on retient le PREMIER dont `0x68` est une map contenant `0x5c` (0..=3).
    // (Bug 2026-08-06 : fenêtre 64o + prendre le 1er frame → `0x68=nil`/absent → None → UI figée Snap 1.)
    let limit = data.len().saturating_sub(1).min(2048);
    for i in 0..limit {
        if !((0x81..=0x8f).contains(&data[i]) && data[i + 1] == 0x66) {
            continue;
        }
        let mut pos = i;
        let Some(top) = parse_value(data, &mut pos) else {
            continue;
        };
        let Some(top_map) = top.as_map() else { continue };
        let Some(inner) = map_get(top_map, 0x68).and_then(|v| v.as_map()) else {
            continue;
        };
        if let Some(idx) = map_get(inner, 0x5c).and_then(|v| v.as_int()) {
            if (0..=3).contains(&idx) {
                return Some(idx as u8);
            }
        }
    }
    None
}

/// Parse un run de triplets `93 [scalaire, scalaire, valeur]` à partir de `start`, jusqu'à ce que
/// l'octet courant ne soit plus `0x93`. Retourne (valeurs par position, curseur de fin).
fn parse_triplet_run(data: &[u8], start: usize) -> Option<(Vec<Option<f32>>, usize)> {
    let mut values = Vec::new();
    let mut cursor = start;
    while data.get(cursor).copied() == Some(0x93) {
        cursor += 1; // saut du tag fixarray-3
        let _e1 = parse_value(data, &mut cursor)?; // bool (flag)
        let _e2 = parse_value(data, &mut cursor)?; // int (type)
        let e3 = parse_value(data, &mut cursor)?; // valeur f32 OU nil
        values.push(match e3 {
            Value::Float(f) => Some(f),
            _ => None,
        });
    }
    Some((values, cursor))
}

/// Valeurs des triplets du bloc `SNAPSHOT n`. Le run de triplets est situé quelques centaines
/// d'octets AVANT le marqueur (d'autres données — états bypass `92 c2 c2/c3` — le séparent du
/// marqueur), donc PAS collé au marqueur. On cherche dans une fenêtre en amont le run le PLUS LONG
/// débutant par `0x93` suivi d'un booléen (`c2`/`c3`) — la vraie table est un tableau de ~64
/// triplets, bien plus long qu'un éventuel `93` parasite. Vérifié byte-exact sur `Snapshot_2_param`.
fn snapshot_block_triplet_values(data: &[u8], snap_num: usize) -> Vec<Option<f32>> {
    let Some(marker) = snapshot_marker_pos(data, snap_num) else {
        return Vec::new();
    };
    let search_from = marker.saturating_sub(450);
    let mut best: Vec<Option<f32>> = Vec::new();
    let mut i = search_from;
    while i + 1 < marker {
        if data[i] == 0x93 && matches!(data[i + 1], 0xc2 | 0xc3) {
            if let Some((values, _end)) = parse_triplet_run(data, i) {
                // Longueur max, tie-break sur le run le plus TARDIF (`>=`, on balaye en croissant) :
                // robuste si deux blocs voisins tombent dans la même fenêtre (fixtures serrées).
                if values.len() >= best.len() && !values.is_empty() {
                    best = values;
                }
            }
        }
        i += 1;
    }
    best
}

/// Retourne, par paramètre contrôlé par snapshot (ordre d'assignation), ses 4 valeurs (snapshots
/// 1..4). Combine : (a) l'ordre des params source=Snapshot du groupe 2 ↔ position dans les triplets,
/// (b) les valeurs des triplets par bloc snapshot.
pub fn scan_snapshot_param_values(data: &[u8]) -> Vec<SnapshotParamValues> {
    let refs = scan_snapshot_source_params(data);
    if refs.is_empty() {
        return Vec::new();
    }
    // Valeurs par snapshot : blocks[snap][position].
    let blocks: Vec<Vec<Option<f32>>> = (1..=SNAPSHOT_COUNT)
        .map(|n| snapshot_block_triplet_values(data, n))
        .collect();

    refs.iter()
        .enumerate()
        .map(|(position, r)| {
            let values: Vec<Option<f32>> = blocks
                .iter()
                .map(|b| b.get(position).copied().flatten())
                .collect();
            SnapshotParamValues {
                slot_bus: r.bus,
                kempline_slot_index: crate::helix::slot_bus_to_kempline_index(r.bus),
                param_index: r.param_index,
                min_raw: r.min_raw,
                max_raw: r.max_raw,
                values,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Élément `82` du groupe 2 pour une source Snapshot : bus + param_index + Min/Max.
    /// Reproduit fidèlement la structure réelle (`Snapshot_2_param.json`) :
    /// `82 00 <k0> 01 89 00 0c 01 04 02 ca<min> 03 ca<max> 04 00 05 <bus> 06 83 1c 00 1d <idx> 29 c2 07 00 0d c2`.
    fn snap_source_elem(bus: u8, param_index: u8, min: f32, max: f32) -> Vec<u8> {
        let mut v = vec![0x82, 0x00, 0x01, 0x01, 0x89, 0x00, 0x0c, 0x01, 0x04, 0x02, 0xca];
        v.extend_from_slice(&min.to_be_bytes());
        v.push(0x03);
        v.push(0xca);
        v.extend_from_slice(&max.to_be_bytes());
        v.extend_from_slice(&[0x04, 0x00, 0x05, bus, 0x06, 0x83, 0x1c, 0x00, 0x1d, param_index, 0x29, 0xc2, 0x07, 0x00, 0x0d, 0xc2]);
        v
    }

    /// Suite `04 <tag> <nom> 00 0e <c2> <ancre 13o>` reproduisant fidèlement la structure réelle
    /// (captures `read_snapshot_preset.json` / `read_preset_named_snap.json`, 2026-07-14) : le nom
    /// (par défaut "SNAPSHOT N" ou custom) : `04 <tag=0xa1+len> <texte> 00 0e` puis un octet de
    /// payload variable (`tail_byte`) — reproduit le fait que ce qui suit `0x0e` DIFFÈRE selon le
    /// snapshot (bug de l'ancienne ancre « octets fixes »).
    fn snapshot_name_suffix(name: &str, tail_byte: u8) -> Vec<u8> {
        let text = name.as_bytes();
        let mut v = vec![0x04, 0xa1u8.wrapping_add(text.len() as u8)];
        v.extend_from_slice(text);
        v.push(0x00); // nul terminateur (inclus dans la longueur fixstr déclarée)
        v.push(0x0e); // clé suivante (signature structurelle du champ nom)
        v.push(tail_byte); // octet de payload variable (ex. c2/c3, ou début de tableau)
        v
    }

    /// Bloc snapshot : run de triplets (`93 [bool,int,val|nil]`) puis champ nom.
    fn snap_block(values: &[Option<f32>], name: &str) -> Vec<u8> {
        snap_block_with_tail(values, name, 0xc2)
    }

    fn snap_block_with_tail(values: &[Option<f32>], name: &str, tail_byte: u8) -> Vec<u8> {
        let mut v = Vec::new();
        for (i, val) in values.iter().enumerate() {
            v.push(0x93);
            v.push(0xc2); // flag bool false
            v.push((i as u8) * 0x10); // "type" (arbitraire, non lu)
            match val {
                Some(f) => {
                    v.push(0xca);
                    v.extend_from_slice(&f.to_be_bytes());
                }
                None => v.push(0xc0),
            }
        }
        v.extend(snapshot_name_suffix(name, tail_byte));
        v
    }

    /// Fixture fidèle à `Snapshot_2_param.json` : 2 params source=Snapshot (Level bus2/idx4,
    /// Tone bus1/idx1) ; valeurs par snapshot pos0=Level(2/4/6/8), pos1=Tone(0.2/0.4/0.6/0.8).
    fn build_two_param_fixture() -> Vec<u8> {
        let mut d = Vec::new();
        // Groupe 2 : `92` (fixarray-2) + elem Level (bus2,idx4) + elem Tone (bus1,idx1).
        d.push(0x92);
        d.extend(snap_source_elem(0x02, 0x04, -60.0, 12.0));
        d.extend(snap_source_elem(0x01, 0x01, 0.0, 1.0));
        // 4 blocs snapshot : pos0 = Level, pos1 = Tone, pos2 = nil.
        let level = [2.0f32, 4.0, 6.0, 8.0];
        let tone = [0.2f32, 0.4, 0.6, 0.8];
        for n in 0..4 {
            d.extend(snap_block(
                &[Some(level[n]), Some(tone[n]), None],
                &format!("SNAPSHOT {}", n + 1),
            ));
        }
        d
    }

    #[test]
    fn decodes_two_snapshot_params_with_per_snapshot_values() {
        let data = build_two_param_fixture();
        let out = scan_snapshot_param_values(&data);
        assert_eq!(out.len(), 2, "deux params contrôlés par snapshot");

        // Position 0 = Level (bus2, idx4) — ordre du groupe 2.
        let level = &out[0];
        assert_eq!(level.slot_bus, 0x02);
        assert_eq!(level.param_index, 0x04);
        assert!((level.min_raw - (-60.0)).abs() < 1e-3);
        assert!((level.max_raw - 12.0).abs() < 1e-3);
        assert_eq!(level.values.len(), 4);
        let lv: Vec<f32> = level.values.iter().map(|v| v.unwrap()).collect();
        assert_eq!(lv, vec![2.0, 4.0, 6.0, 8.0]);

        // Position 1 = Tone (bus1, idx1).
        let tone = &out[1];
        assert_eq!(tone.slot_bus, 0x01);
        assert_eq!(tone.param_index, 0x01);
        let tv: Vec<f32> = tone.values.iter().map(|v| v.unwrap()).collect();
        assert_eq!(tv, vec![0.2, 0.4, 0.6, 0.8]);
    }

    #[test]
    fn empty_when_no_snapshot_source_param() {
        // Un groupe 2 avec une source Footswitch (3) ne doit produire aucun param snapshot.
        let mut d = vec![0x91];
        d.extend(snap_source_elem(0x01, 0x01, 0.0, 1.0));
        // patch la source 0x0c -> 0x03 (footswitch) dans l'élément
        let src_pos = d.iter().position(|&b| b == 0x0c).unwrap();
        d[src_pos] = 0x03;
        assert!(scan_snapshot_param_values(&d).is_empty());
    }

    /// Reproduit la signature structurelle vue dans `read_snapshot_preset.json`
    /// (`04 ab 53 4e 41 50 53 48 4f 54 20 31 00 0e`) — l'octet qui suit `0x0e` (ici `c2`) est du
    /// payload variable, hors signature.
    #[test]
    fn snapshot_name_field_matches_real_capture_bytes() {
        let suffix = snapshot_name_suffix("SNAPSHOT 1", 0xc2);
        let expected: Vec<u8> =
            bytes_from_hex_colon("04:ab:53:4e:41:50:53:48:4f:54:20:31:00:0e:c2");
        assert_eq!(suffix, expected);
    }

    fn bytes_from_hex_colon(s: &str) -> Vec<u8> {
        s.split(':').map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    #[test]
    fn scan_snapshot_names_reads_default_labels() {
        let mut d = Vec::new();
        for n in 1..=4 {
            d.extend(snap_block(&[None], &format!("SNAPSHOT {n}")));
        }
        assert_eq!(
            scan_snapshot_names(&d),
            vec!["SNAPSHOT 1", "SNAPSHOT 2", "SNAPSHOT 3", "SNAPSHOT 4"]
        );
    }

    #[test]
    fn scan_snapshot_names_survives_rename_name_agnostic() {
        // Le snapshot 1 est renommé (comme après `rename_first_snapshot.json`) : le texte littéral
        // "SNAPSHOT 1" a disparu — seule la signature structurelle permet de retrouver le bloc.
        let mut d = Vec::new();
        let names = ["FirstSnap", "SNAPSHOT 2", "SNAPSHOT 3", "SNAPSHOT 4"];
        for name in names {
            d.extend(snap_block(&[None], name));
        }
        assert_eq!(scan_snapshot_names(&d), names.to_vec());
    }

    /// RÉGRESSION (bug trouvé sur `read_snapshot_preset_linux.json`, snap4 modifié Level=-30) :
    /// l'octet de payload APRÈS `0x0e` diffère entre snapshots (identique 1/2/3 non modifiés, mais
    /// différent sur le snap4 modifié). Le décodage du nom ET des valeurs doit marcher malgré ça.
    #[test]
    fn snapshot_name_and_values_survive_divergent_payload_tail() {
        let mut d = Vec::new();
        // Un param source=Snapshot (pos0), valeurs 0/0/0/-30 (= le cas Deluxe Comp Level du capture).
        d.push(0x91);
        d.extend(snap_source_elem(0x02, 0x04, -60.0, 12.0));
        let vals = [0.0f32, 0.0, 0.0, -30.0];
        // Snap 1/2/3 : tail 0x88 (comme les blocs non modifiés) ; snap 4 : tail 0x0d (divergent).
        let tails = [0x88u8, 0x88, 0x88, 0x0d];
        for n in 0..4 {
            d.extend(snap_block_with_tail(&[Some(vals[n])], &format!("SNAPSHOT {}", n + 1), tails[n]));
        }
        // Noms : tous par défaut, snap4 inclus (l'ancienne ancre le ratait à cause du tail divergent).
        assert_eq!(
            scan_snapshot_names(&d),
            vec!["SNAPSHOT 1", "SNAPSHOT 2", "SNAPSHOT 3", "SNAPSHOT 4"]
        );
        // Valeurs : le snap4 (Level=-30) doit être décodé.
        let out = scan_snapshot_param_values(&d);
        assert_eq!(out.len(), 1);
        let got: Vec<Option<f32>> = out[0].values.clone();
        assert_eq!(got, vec![Some(0.0), Some(0.0), Some(0.0), Some(-30.0)]);
    }

    #[test]
    fn snapshot_name_defaults_when_dump_empty() {
        assert_eq!(snapshot_name(&[], 1), "Snapshot 1");
        assert_eq!(scan_snapshot_names(&[]), vec!["Snapshot 1", "Snapshot 2", "Snapshot 3", "Snapshot 4"]);
    }

    /// En-tête preset reproduisant la structure réelle (`read_snapshot_preset_linux` :
    /// top fixmap-3 {0x66:u16, 0x67:int, 0x68:{0x6b:u16, 0x6c:u16, 0x6d:nom, 0x75:bool, 0x53:arr, 0x5c:idx}}).
    fn build_header_with_active(active: u8) -> Vec<u8> {
        let mut d = vec![
            // préfixe de trame arbitraire (le décodeur cherche le fixmap `8N`+clé 0x66)
            0x00, 0x00, 0x06, 0x00, 0x24, 0x00, 0x00, 0x00,
            0x83, // top fixmap-3
            0x66, 0xcd, 0x03, 0xf8, // 0x66 = u16(1016)
            0x67, 0x00, // 0x67 = 0
            0x68, 0x86, // 0x68 = fixmap-6
            0x6b, 0xcd, 0x00, 0x00, // 0x6b = u16(0)
            0x6c, 0xcd, 0x00, 0x16, // 0x6c = u16(22)
            0x6d, 0xa7, b'S', b'n', b'a', b'p', b' ', b'4', 0x00, // 0x6d = "Snap 4\0"
            0x75, 0xc2, // 0x75 = false
            0x53, 0x92, 0xcd, 0x08, 0x88, 0x00, // 0x53 = arr[u16(2184), 0]
            0x5c, active, // 0x5c = active snapshot index
        ];
        d.push(0x00); // padding
        d
    }

    #[test]
    fn decodes_active_snapshot_from_header() {
        assert_eq!(active_snapshot_index(&build_header_with_active(0)), Some(0));
        assert_eq!(active_snapshot_index(&build_header_with_active(1)), Some(1));
        assert_eq!(active_snapshot_index(&build_header_with_active(2)), Some(2));
        assert_eq!(active_snapshot_index(&build_header_with_active(3)), Some(3));
        // Dump vide / illisible → None (l'appelant retombe sur 0).
        assert_eq!(active_snapshot_index(&[]), None);
        assert_eq!(active_snapshot_index(&[0x00, 0x11, 0x22]), None);
    }

    /// RÉGRESSION (capture `change_preset_actived_snap`, 2026-08-06) : le dump commence par un frame
    /// PRÉAMBULE `83 66 cd 04 f3 67 01 68 c0` (`0x68` = nil) AVANT la vraie map d'en-tête. Le décodeur
    /// doit SAUTER le préambule et lire le `0x5c` de la vraie map. Vérité terrain différentiel :
    /// "Active Snap 1..4" → 0x5c = 0/1/2/3.
    #[test]
    fn decodes_active_snapshot_skipping_nil_preamble() {
        // Préambule : fixmap-3 {0x66:u16, 0x67:1, 0x68:nil}.
        let preamble: Vec<u8> = vec![
            0x00, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, // frame prefix
            0x83, 0x66, 0xcd, 0x04, 0xf3, 0x67, 0x01, 0x68, 0xc0, // 0x68 = nil
        ];
        for active in 0u8..=3 {
            let mut d = preamble.clone();
            d.extend(build_header_with_active(active));
            assert_eq!(
                active_snapshot_index(&d),
                Some(active),
                "préambule nil doit être sauté (active={active})"
            );
        }
    }

    /// RÉGRESSION (dump content_only réel de notre app, 2026-08-07) : la vraie map d'en-tête arrive
    /// LOIN du début (offset ~112), précédée de plusieurs petits frames préambule (dont des sous-maps
    /// `82` sans `0x5c`). Le décodeur doit chercher au-delà des 64 premiers octets. Ici on préfixe
    /// ~150 octets de bruit + un frame `83 66 … 68 82 …` (sous-map sans 0x5c) avant le vrai en-tête.
    #[test]
    fn decodes_active_snapshot_header_far_from_start() {
        let mut d: Vec<u8> = Vec::new();
        // Bruit de préambule (frames divers, dont un `83 66 cd .. 67 .. 68 82 <map-2 sans 0x5c>`).
        d.extend(std::iter::repeat(0x00).take(90));
        d.extend([0x83, 0x66, 0xcd, 0x03, 0xe8, 0x67, 0x00, 0x68, 0x82, 0x76, 0xcd, 0x00, 0x80, 0x77, 0x00]);
        d.extend(std::iter::repeat(0x00).take(20));
        // Vrai en-tête (offset > 64) avec active=2.
        d.extend(build_header_with_active(2));
        assert_eq!(active_snapshot_index(&d), Some(2));
    }
}
