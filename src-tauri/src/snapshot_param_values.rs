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

/// Position du marqueur `SNAPSHOT n\0` (n = 1..4). `None` si absent.
fn snapshot_marker_pos(data: &[u8], snap_num: usize) -> Option<usize> {
    let needle: Vec<u8> = format!("SNAPSHOT {snap_num}\0").into_bytes();
    data.windows(needle.len()).position(|w| w == needle)
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

    /// Bloc snapshot : run de triplets (`93 [bool,int,val|nil]`) puis marqueur `SNAPSHOT n\0`.
    fn snap_block(values: &[Option<f32>], n: usize) -> Vec<u8> {
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
        v.extend_from_slice(format!("SNAPSHOT {n}\0").as_bytes());
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
                n + 1,
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
}
