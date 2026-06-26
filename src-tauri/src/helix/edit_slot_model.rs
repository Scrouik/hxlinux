// ===========================================================
// edit_slot_model.rs
// Séquences USB « changement de modèle dans un slot » calquées sur les captures
// HX Edit (voir `src/Paquets Json/Add model HXEdit.json` et `Change Model HXEdit.json`).
//
// **Historique projet** (ordre d’intégration) :
// 1. Test : un seul **bulkHex** issu de `Change Model HXEdit.json`, renvoyé **quel que soit** le
//    modèle cliqué dans la liste, pour valider la séquence USB (courts + bulk + clôture).
// 2. Prod : bulks par modèle dans `resources/HX_ModelUsbAssign.json` (`resolve_usb_assign_bulk`).
//    Pour rejouer l’étape 1 sans modifier le code : `HX_SLOT_PROBE_USE_CHANGE_MODEL_TEST_BULK=1`.
//
// Statut : **sonde** — envoi best-effort pour comparaison bus Linux vs Windows.
// Les compteurs de session (x80, CTR…) sont injectés depuis `HelixState` comme pour
// `live_write`. Si un `chainHex` catalogue est fourni, on recopie depuis le premier
// `83 66 cd` jusqu’à la fin du bulk (troncature si trop long) puis on ré-applique le
// `slot_bus` sur `82 62 **`.
//
// **CRÉATION Amp+Cab legacy (juin 2026)** : le bulkHex `amp+cab-legacy` (head=23, cd:07, 44 o)
// est une forme compacte sans segment `82 13 06 14 83 18` ni trailer `09 12 0a c3`.
// Sur slot VIDE, HX Edit crée avec head=2d (56 o, cd:03). Voir `build_amp_cab_legacy_create_bulk`
// (flag `HX_AMP_CAB_LEGACY_CREATE_HEAD2D`, défaut ON).
//
// **CRÉATION dual (juin 2026)** : le bulkHex `assign48_cd0a` de `HX_ModelUsbAssign.json` est
// en réalité une forme REMPLACEMENT (head=27, sans le segment de création `82 13 06 14 83 18`).
// Sur slot VIDE, HX Edit envoie un head=31 complet (capture `add_dual_cab_soup_pro_2x12bluebell`
// fr1853) qui enregistre cab2 comme élément focusable. Sans lui, cab2 n'est jamais modifiable.
// Voir `CAB_DUAL_CREATE_BULK60_HEAD31_TEMPLATE` + `build_cab_dual_create_bulk` (flag
// `HX_CAB_DUAL_CREATE_HEAD31`, défaut ON).
//
// **Hex long déjà présent dans vos captures** (fichiers `Paquets Json/`) :
// - **Replace / swap modèle (Preset33, mai 2026)** — même preset, slots 1–3 : bulk **48 o**,
//   tout en **`80:10:ed:03`** (cf. `Preset33 Slot1 cd0184 to cd01fe.json` …). Après `83 66 cd 04`,
//   l’octet « voie » = **`2 * index_kempline`** (0,2,4… sur path 1 slots 0–2) ; `82 62 <slot_bus> 64`.
// - Add (56 o, slot vide) — toujours la capture « Add model HXEdit » plus ancienne :
//   `2f0000188010ed0300830004cc2a0000010006001f0000008366cd03fb642765826204638213061483188317c219cd01fe1aff09010ac300`
// - Variante « Change model » **`03:10` + 44 o** (`cd:03`) : bulk de référence embarqué + env
//   `HX_SLOT_PROBE_USE_CHANGE_MODEL_TEST_BULK=1` (voir `CHANGE_MODEL_HXEDIT_REPLACE_TEST_BULK_HEX`).
// Les `chainHex` courts de `HX_ModelCatalog.json` ne remplacent pas ce corps (voir garde
// `MIN_CATALOG_CHAIN_USB_PATCH_LEN` + préfixe `83 66 cd`).
// ===========================================================

use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Bulk « replace » 44 o extrait de `src/Paquets Json/Change Model HXEdit.json` (host → 0x01,
/// `83:66:cd:03`, slot bus **0x04** dans la capture). Sert de **référence de test** (voir entête).
const CHANGE_MODEL_HXEDIT_REPLACE_TEST_BULK_HEX: &str = concat!(
    "230000180310ed0300550004482a00000100060013000000",
    "8366cd03f6642865826204648317c219641aff00"
);

/// Renverra le bulk de test HX Edit (`Change Model`) au lieu de `HX_ModelUsbAssign.json`.
pub fn slot_probe_use_change_model_test_bulk() -> bool {
    std::env::var_os("HX_SLOT_PROBE_USE_CHANGE_MODEL_TEST_BULK").is_some_and(|v| {
        v.to_str()
            .is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes"))
    })
}

/// Octets du bulk de test (même envoi que pendant la phase de validation initiale).
pub fn change_model_hxedit_replace_test_bulk() -> Vec<u8> {
    static CACHED: OnceLock<Vec<u8>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            parse_hex_bytes(CHANGE_MODEL_HXEDIT_REPLACE_TEST_BULK_HEX)
                .expect("CHANGE_MODEL_HXEDIT_REPLACE_TEST_BULK_HEX")
        })
        .clone()
}

/// Opération de sonde (slot cible = octet `82 62 **slot**` dans le bulk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotModelProbeOp {
    /// Slot vide : famille `80 10 ed 03` (capture « Add model »).
    AddToEmpty,
    /// Slot occupé : séquence **`80:10`** + bulk **48 o** `83:66:cd:04` (captures Preset33, mai 2026).
    ReplaceOccupied,
    /// Slot occupé : suppression du modèle (capture `Delete Model HXEdit.json`, bulk 36 o).
    RemoveFromOccupied,
}

/// Bulk « add » 56 octets — `src/Paquets Json/Add model HXEdit.json` frame host #25.
const ADD_MODEL_BULK_TEMPLATE: [u8; 56] = [
    0x2f, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x83, 0x00, 0x04, 0xcc, 0x2a, 0x00, 0x00,
    0x01, 0x00, 0x06, 0x00, 0x1f, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0xfb, 0x64, 0x27, 0x65,
    0x82, 0x62, 0x04, 0x63, 0x82, 0x13, 0x06, 0x14, 0x83, 0x18, 0x83, 0x17, 0xc2, 0x19, 0xcd, 0x01,
    0xfe, 0x1a, 0xff, 0x09, 0x01, 0x0a, 0xc3, 0x00,
];

/// Bulk « assignation modèle » 48 octets — `Preset33 Slot1 cd0184 to cd01fe.json` (host OUT).
/// Corps modèle = **`cd:01:fe`** (exemple capture) ; `slot_bus` + octet voie après `cd 04` sont patchés.
const REPLACE_MODEL_BULK48_CD04_TEMPLATE: [u8; 48] = [
    0x25, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xd1, 0x00, 0x04, 0x8f, 0x35, 0x00, 0x00,
    0x01, 0x00, 0x06, 0x00, 0x15, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04, 0x00, 0x64, 0x28, 0x65,
    0x82, 0x62, 0x01, 0x64, 0x83, 0x17, 0xc2, 0x19, 0xcd, 0x01, 0xfe, 0x1a, 0xff, 0x00, 0x00, 0x00,
];

/// Bulk « remove » 36 octets — `Delete Model HXEdit.json` (host OUT, frame 637).
const REMOVE_MODEL_BULK36_CD03_TEMPLATE: [u8; 36] = [
    0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x5c, 0x00, 0x04, 0xbb, 0x2a, 0x00, 0x00,
    0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0xfa, 0x64, 0x1c, 0x65,
    0x81, 0x62, 0x04, 0x00,
];

/// Template **CRÉATION dual** — capture HX Edit `add_dual_cab_soup_pro_2x12bluebell_HXEdit.json`
/// frame 1853 (head=**31**, 60 o). Contient la structure de création complète
/// (`82 13 06 14 83 18` + trailer `09 20 0a c3`) ABSENTE du bulkHex `assign48_cd0a` de
/// `HX_ModelUsbAssign.json` (forme REMPLACEMENT head=27, sans enregistrement de cab2). Sur slot
/// vide, HX Edit envoie CE head=31 ; sans lui, le device crée un dual où cab2 n'est jamais
/// focusable → le focus `1d` n'est jamais honoré. Variable par dual : le cab1 (identité),
/// repatché depuis le bulkHex `dual`. cab2 = défaut `cd 02 d6` (Jazz Rivet, comme HX Edit).
/// `slot_bus`/compteurs patchés au runtime.
const CAB_DUAL_CREATE_BULK60_HEAD31_TEMPLATE: [u8; 60] = [
    0x31, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x24, 0x00, 0x04, 0xff, 0x1b, 0x00, 0x00,
    0x01, 0x00, 0x06, 0x00, 0x21, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03, 0xf1, 0x64, 0x27, 0x65,
    0x82, 0x62, 0x01, 0x63, 0x82, 0x13, 0x06, 0x14, 0x83, 0x18, 0x83, 0x17, 0xc3, 0x19, 0xcd, 0x03,
    0x1c, 0x1a, 0xcd, 0x02, 0xd6, 0x09, 0x20, 0x0a, 0xc3, 0x00, 0x00, 0x00,
];

fn patch_short_ed03_16(
    out: &mut [u8; 16],
    op: [u8; 4],
    byte11: u8,
    seq_x80: u8,
    ctr: u16,
) {
    out[0..4].copy_from_slice(&[0x08, 0x00, 0x00, 0x18]);
    out[4..8].copy_from_slice(&op);
    out[8] = 0x00;
    out[9] = seq_x80;
    out[10] = 0x00;
    out[11] = byte11;
    out[12] = (ctr & 0xff) as u8;
    out[13] = ((ctr >> 8) & 0xff) as u8;
    out[14] = 0x00;
    out[15] = 0x00;
}

fn patch_bulk_header_counters(buf: &mut [u8], seq_x80: u8, ctr: u16) {
    if buf.len() >= 14 {
        buf[9] = seq_x80;
        buf[12] = (ctr & 0xff) as u8;
        buf[13] = ((ctr >> 8) & 0xff) as u8;
    }
}

/// Bulk `HX_ModelUsbAssign.json` : seul le compteur **x80** (octet 9) est dynamique.
/// Les octets 12–15 portent l’identité modèle capturée (`d1 a7 02 00` WhoWatt, etc.) — ne pas
/// y écrire `live_write_ctr` (sinon le device ignore le replace cab).
fn patch_bulk_header_seq_x80_only(buf: &mut [u8], seq_x80: u8) {
    if buf.len() > 9 {
        buf[9] = seq_x80;
    }
}

/// Octet du bus slot dans le segment `82 62 **slot** …` (même convention que `live_write`).
fn patch_slot_bus_in_bulk(buf: &mut [u8], slot_bus: u8) {
    for i in 0..buf.len().saturating_sub(2) {
        if (buf[i] == 0x82 || buf[i] == 0x81) && buf[i + 1] == 0x62 {
            buf[i + 2] = slot_bus;
            return;
        }
    }
    // Fallback : offset fixe des captures HX Edit (`82 62` à l’index 32).
    if buf.len() > 34 {
        buf[34] = slot_bus;
    }
}

/// Octet après `83 66 cd 04` ou `83 66 cd 03` : chemins catalogue / template hérités.
fn patch_kempline_lane_after_cd03_or_cd04(bulk: &mut [u8], kempline_index: usize) {
    let lane = (kempline_index as u8).saturating_mul(2);
    for i in 0..bulk.len().saturating_sub(5) {
        if bulk[i] == 0x83
            && bulk[i + 1] == 0x66
            && bulk[i + 2] == 0xcd
            && (bulk[i + 3] == 0x04 || bulk[i + 3] == 0x03)
        {
            bulk[i + 4] = lane;
            return;
        }
    }
}

fn cd_lane_offset_after_cd03_or_cd04(bulk: &[u8]) -> Option<usize> {
    for i in 0..bulk.len().saturating_sub(5) {
        if bulk[i] == 0x83
            && bulk[i + 1] == 0x66
            && bulk[i + 2] == 0xcd
            && (bulk[i + 3] == 0x04 || bulk[i + 3] == 0x03)
        {
            return Some(i + 4);
        }
    }
    None
}

/// Octet tag session après `83 66 cd 08` (replace slot occupé — capture `amp_cab legacy bass.json` 1775).
fn cd_lane_tag_offset_after_cd08(bulk: &[u8]) -> Option<usize> {
    for i in 0..bulk.len().saturating_sub(5) {
        if bulk[i] == 0x83 && bulk[i + 1] == 0x66 && bulk[i + 2] == 0xcd && bulk[i + 3] == 0x08 {
            return Some(i + 4);
        }
    }
    None
}

/// Opcode ED03 des courts 16 o dérivé du bulk assignation.
///
/// Captures:
/// - bulk `80 10 ed 03` -> short `ed 03 80 10`
/// - bulk `03 10 ed 03` -> short `ed 03 03 10`
fn ed_op_from_assign_bulk_prefix(bulk: &[u8]) -> [u8; 4] {
    if bulk.len() >= 6 {
        return [0xed, 0x03, bulk[4], bulk[5]];
    }
    [0xed, 0x03, 0x80, 0x10]
}

/// `HX_CAB_DUAL_CREATE_HEAD31` (défaut ON) : sur slot vide, créer le dual avec le head=31
/// complet au lieu du bulkHex `assign48_cd0a` (head=27, sans enregistrement de cab2).
/// `=0` → témoin : ancien comportement (head=27, cab2 non modifiable).
fn cab_dual_create_head31_enabled() -> bool {
    match std::env::var("HX_CAB_DUAL_CREATE_HEAD31").as_deref() {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Reconnaît un bulkHex de dual (`assign48_cd0a`) : présence de `83 66 cd 0a`.
fn bulk_is_cab_dual_cd0a(bulk: &[u8]) -> bool {
    bulk.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x0a])
}

/// Construit la commande de CRÉATION dual (head=31) à partir du bulkHex `assign48_cd0a`.
/// On reprend le cab1 (identité du dual, entre `c3 19` et `1a`) et on l'insère dans le template
/// de création. cab2 reste le défaut `cd 02 d6` (Jazz Rivet) — modifiable ensuite par le
/// handshake focus → ed:08 → IN 21 → bulk `27`.
pub fn build_cab_dual_create_bulk(dual_assign_bulk: &[u8]) -> Option<Vec<u8>> {
    let (c1s, c1e) = cab_dual_cab1_field_range_in_bulk(dual_assign_bulk)?;
    let cab1: Vec<u8> = dual_assign_bulk[c1s..c1e].to_vec();
    let mut create = CAB_DUAL_CREATE_BULK60_HEAD31_TEMPLATE.to_vec();
    let (t1s, t1e) = cab_dual_cab1_field_range_in_bulk(&create)?;
    if cab1.len() == t1e - t1s {
        create[t1s..t1e].copy_from_slice(&cab1);
    } else {
        create.splice(t1s..t1e, cab1.iter().copied());
    }
    Some(create)
}

/// `HX_AMP_CAB_LEGACY_CREATE_HEAD2D` (défaut ON) : sur slot vide, créer l’Amp+Cab legacy avec
/// head=2d (56 o, cd:03) au lieu du bulkHex assign (head=23, cd:07, 44 o).
fn amp_cab_legacy_create_head2d_enabled() -> bool {
    match std::env::var("HX_AMP_CAB_LEGACY_CREATE_HEAD2D").as_deref() {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Bulk assign `amp+cab-legacy` compact (head `0x23`, lane `cd:07`, marqueur `c3:19`).
fn bulk_is_amp_cab_legacy_assign(bulk: &[u8]) -> bool {
    bulk.first() == Some(&0x23)
        && bulk.len() == 44
        && bulk.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x07])
        && bulk.windows(AMP_CAB_BULK_MARKER.len())
            .any(|w| w == AMP_CAB_BULK_MARKER)
}

const AMP_CAB_LEGACY_CREATE_SEGMENT: [u8; 6] = [0x82, 0x13, 0x06, 0x14, 0x83, 0x18];
const AMP_CAB_LEGACY_CREATE_TRAILER: [u8; 7] = [0x09, 0x12, 0x0a, 0xc3, 0x00, 0x00, 0x00];

/// Template création Amp+Cab legacy (56 o, head `2d`, `cd:03`) depuis le bulk assign (44 o, `cd:07`).
/// Même logique que `build_cab_dual_create_bulk` : segment création + trailer après le cab.
pub fn build_amp_cab_legacy_create_bulk(assign_bulk: &[u8]) -> Option<Vec<u8>> {
    if assign_bulk.len() != 44 || assign_bulk.first()? != &0x23 {
        return None;
    }
    if !bulk_is_amp_cab_legacy_assign(assign_bulk) {
        return None;
    }

    let mut out = assign_bulk.to_vec();
    out[0] = 0x2d;

    for i in 0..out.len().saturating_sub(4) {
        if out[i..i + 4] == [0x83, 0x66, 0xcd, 0x07] {
            out[i + 3] = 0x03;
            break;
        }
    }

    for i in 0..out.len().saturating_sub(4) {
        if out[i] == 0x82 && out[i + 1] == 0x62 && out[i + 3] == 0x64 {
            out[i + 3] = 0x63;
            break;
        }
    }

    let insert_pos = out.windows(2).position(|w| w == [0x83, 0x17])?;
    out.splice(
        insert_pos..insert_pos,
        AMP_CAB_LEGACY_CREATE_SEGMENT.iter().copied(),
    );

    let (_cab_s, cab_e) = amp_cab_cab_field_range_in_bulk(&out)?;
    out.truncate(cab_e);
    out.extend_from_slice(&AMP_CAB_LEGACY_CREATE_TRAILER);

    if out.len() >= 21 {
        // Octet 20 = longueur payload hors en-tête 24 o, moins les 3 octets `00` finaux du trailer.
        out[20] = out.len().saturating_sub(24 + 3) as u8;
    }
    Some(out)
}

#[derive(Debug, Clone)]
struct UsbAssignEntry {
    id: String,
    variant: String,
    bulk: Vec<u8>,
    chain_hex_hint: String,
}

static USB_ASSIGN_ENTRIES: OnceLock<Vec<UsbAssignEntry>> = OnceLock::new();

fn load_usb_assign_entries() -> Vec<UsbAssignEntry> {
    const JSON_STR: &str = include_str!("../../resources/HX_ModelUsbAssign.json");
    let v: Value = serde_json::from_str(JSON_STR).expect("HX_ModelUsbAssign.json parse");
    let Some(arr) = v.get("entries").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for e in arr {
        let Some(id) = e.get("id").and_then(|x| x.as_str()).map(|s| s.trim().to_string()) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let variant = e
            .get("variant")
            .and_then(|x| x.as_str())
            .unwrap_or("mono")
            .trim()
            .to_ascii_lowercase();
        let Some(hex) = e.get("bulkHex").and_then(|x| x.as_str()) else {
            continue;
        };
        let chain_hex_hint = e
            .get("chainHexHint")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if let Some(bulk) = parse_hex_bytes(hex) {
            if bulk.len() >= 32 {
                out.push(UsbAssignEntry {
                    id,
                    variant,
                    bulk,
                    chain_hex_hint,
                });
            }
        }
    }
    out
}

/// `chainHexHint` catalogue pour `id` + `variant` (ex. cab legacy `33` ou `cd024d`).
pub fn resolve_usb_assign_chain_hex_hint(model_id: &str, variant: &str) -> Option<String> {
    let id = model_id.trim();
    if id.is_empty() {
        return None;
    }
    let v = variant.trim().to_ascii_lowercase();
    let entries = USB_ASSIGN_ENTRIES.get_or_init(load_usb_assign_entries);
    for e in entries {
        if e.id == id && e.variant == v {
            let h = e.chain_hex_hint.trim();
            if h.is_empty() {
                return None;
            }
            return Some(h.to_string());
        }
    }
    None
}

/// Bulk complet issu de `HX_ModelUsbAssign.json` pour `id` + `variant` (`mono` | `stereo` | `legacy`).
pub fn resolve_usb_assign_bulk(model_id: &str, variant: &str) -> Option<Vec<u8>> {
    let id = model_id.trim();
    if id.is_empty() {
        return None;
    }
    let v = variant.trim().to_ascii_lowercase();
    let entries = USB_ASSIGN_ENTRIES.get_or_init(load_usb_assign_entries);
    for e in entries {
        if e.id == id && e.variant == v {
            return Some(e.bulk.clone());
        }
    }
    None
}

fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if s.is_empty() || s.len() % 2 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        out.push(u8::from_str_radix(&s[i..i + 2], 16).ok()?);
    }
    Some(out)
}

fn first_chain_hex_bytes_from_preset_meta(pm: &serde_json::Map<String, Value>) -> Option<Vec<u8>> {
    let hex_v = pm.get("chainHex")?;
    let candidates: Vec<String> = match hex_v {
        Value::String(st) => {
            let t = st.trim().to_string();
            if t.is_empty() {
                return None;
            }
            vec![t]
        }
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        _ => return None,
    };
    if candidates.is_empty() {
        return None;
    }
    // Préférer la chaîne qui donne le plus d’octets (ex. variante Stereo vs Mono plus longue).
    let mut best: Option<Vec<u8>> = None;
    for s in candidates {
        if let Some(b) = parse_hex_bytes(&s) {
            if best.as_ref().map_or(true, |cur| b.len() > cur.len()) {
                best = Some(b);
            }
        }
    }
    best
}

fn id_to_chain_bytes_map(catalog: &Value) -> HashMap<String, Vec<u8>> {
    let mut m = HashMap::new();
    let mut scan_models = |models: &[Value]| {
        for mo in models {
            let Some(oid) = mo.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let id = oid.trim();
            if id.is_empty() {
                continue;
            }
            let Some(pm) = mo.get("presetMeta").and_then(|p| p.as_object()) else {
                continue;
            };
            if let Some(bytes) = first_chain_hex_bytes_from_preset_meta(pm) {
                m.insert(id.to_string(), bytes);
            }
        }
    };
    if let Some(models) = catalog.get("models").and_then(|v| v.as_array()) {
        if !models.is_empty() {
            scan_models(models);
            return m;
        }
    }
    if let Some(categories) = catalog.get("categories").and_then(|v| v.as_array()) {
        for cat in categories {
            let Some(co) = cat.as_object() else {
                continue;
            };
            if let Some(arr) = co.get("models").and_then(|v| v.as_array()) {
                scan_models(arr);
            }
            if let Some(subs) = co.get("subcategories").and_then(|v| v.as_array()) {
                for sub in subs {
                    let Some(so) = sub.as_object() else {
                        continue;
                    };
                    if let Some(arr) = so.get("models").and_then(|v| v.as_array()) {
                        scan_models(arr);
                    }
                }
            }
        }
    }
    m
}

static CATALOG_ID_CHAIN_BYTES: OnceLock<HashMap<String, Vec<u8>>> = OnceLock::new();

/// Premier `presetMeta.chainHex` (chaîne ou 1er élément du tableau) pour l’`id` catalogue.
pub fn resolve_catalog_model_chain_bytes(model_id: &str) -> Option<Vec<u8>> {
    let id = model_id.trim();
    if id.is_empty() {
        return None;
    }
    let map = CATALOG_ID_CHAIN_BYTES.get_or_init(|| {
        const JSON: &str = include_str!("../../resources/HX_ModelCatalog.json");
        let v: Value = serde_json::from_str(JSON).expect("HX_ModelCatalog.json");
        id_to_chain_bytes_map(&v)
    });
    map.get(id).cloned()
}

/// Longueur minimale pour traiter `chainHex` comme un segment assignable USB (pas un id court type `cd0171`).
const MIN_CATALOG_CHAIN_USB_PATCH_LEN: usize = 12;

/// Recopie `chain` sur le suffixe bulk à partir du premier `83 66 cd` (troncature si nécessaire).
///
/// Beaucoup d’entrées `HX_ModelCatalog.json` n’ont qu’un **id module court** (`chainHex`: `["cd0171","cd0176"]`).
/// Ce n’est **pas** le bloc binaire HX Edit : l’écraser sur le template casse le paquet (`83 66 cd` → `cd 01 71`…).
/// On ne fusionne donc que si `chain` ressemble déjà à un préfixe Kempline (`83 66 cd …`) et est assez long.
fn patch_catalog_chain_into_bulk(bulk: &mut [u8], chain: &[u8]) -> bool {
    if chain.len() < MIN_CATALOG_CHAIN_USB_PATCH_LEN {
        return false;
    }
    if !chain.starts_with(&[0x83, 0x66, 0xcd]) {
        return false;
    }
    let Some(start) = bulk
        .windows(3)
        .position(|w| w == [0x83, 0x66, 0xcd])
    else {
        return false;
    };
    let max = bulk.len().saturating_sub(start);
    let n = max.min(chain.len());
    if n > 0 {
        bulk[start..start + n].copy_from_slice(&chain[..n]);
    }
    true
}

/// Construit la liste de paquets OUT (endpoint 0x01) pour une sonde.
///
/// Séquence (simplifiée par rapport à HX Edit) :
/// 1. ED03 court `… 00 10 …` (ouvre / aligne comme les captures avant le bulk).
/// 2. Bulk modèle (template capture + `slot_bus` + compteurs).
/// 3. ED03 court `… 00 08 …` (clôture observée après le bulk).
///
/// Si `usb_assign_full_bulk` est fourni (Add ou Replace), les courts 16 o reprennent les
/// octets ED du bulk (`80 10` vs `03 10`) et le corps est celui du JSON (captures Preset33) —
/// pas de fusion `chainHex` court catalogue.
pub fn build_slot_model_probe_packets(
    state: &mut super::HelixState,
    op: SlotModelProbeOp,
    kempline_index: usize,
    slot_bus: u8,
    catalog_chain_bytes: Option<&[u8]>,
    usb_assign_full_bulk: Option<&[u8]>,
    // Replace cab 2 après focus `1d` : pas de préambule `ef`/`f0` (capture `cab dual change right.json`).
    cab_dual_cab2_replace_after_focus: bool,
) -> Vec<Vec<u8>> {
    // -------------------------------------------------------------------------
    // NOTE MAINTENANCE (importante)
    //
    // Cette fonction ne doit PAS router selon `category` / `subCategory` métier.
    // Le routage doit rester basé sur le profil TRANSPORT USB observé dans les
    // captures (opcode ED03 + gabarit bulk), sinon on introduit des règles UI
    // arbitraires qui cassent à la moindre variation catalogue.
    //
    // Ici, les deux profils principaux vus en captures:
    // - 80:10:ed:03  -> flux "classique" (stereo/legacy majoritairement).
    // - 03:10:ed:03  -> flux alternatif (notamment Distortion mono observé).
    //
    // Le choix de séquence (pré-contexte, patch compteurs, etc.) doit donc
    // dépendre des octets du bulk transporté (ou d'un champ structuré équivalent
    // comme `edOpcode`/`bulkKind`), jamais du nom de modèle.
    // -------------------------------------------------------------------------
    let mut packets: Vec<Vec<u8>> = Vec::new();

    // CRÉATION dual sur slot VIDE : le bulkHex `assign48_cd0a` est une forme REMPLACEMENT
    // (head=27, sans le segment de création `82 13 06 14 83 18`). HX Edit crée le dual avec un
    // head=31 complet qui enregistre cab2 comme focusable. On substitue ici la commande de
    // création (le cab1 = identité du dual est repris depuis le bulkHex d'origine).
    let cab_dual_create_owned: Option<Vec<u8>> = if matches!(op, SlotModelProbeOp::AddToEmpty)
        && cab_dual_create_head31_enabled()
        && matches!(usb_assign_full_bulk, Some(b) if bulk_is_cab_dual_cd0a(b))
    {
        let made = usb_assign_full_bulk.and_then(build_cab_dual_create_bulk);
        if let Some(ref c) = made {
            eprintln!(
                "[SlotModelProbe] cab dual CREATE head=31 ({} o) substitué au bulkHex assign48 (head=27) — \
                 enregistre cab2 comme focusable",
                c.len()
            );
        }
        made
    } else {
        None
    };
    let amp_cab_legacy_create_owned: Option<Vec<u8>> = if matches!(op, SlotModelProbeOp::AddToEmpty)
        && amp_cab_legacy_create_head2d_enabled()
        && cab_dual_create_owned.is_none()
        && matches!(usb_assign_full_bulk, Some(b) if bulk_is_amp_cab_legacy_assign(b))
    {
        let made = usb_assign_full_bulk.and_then(build_amp_cab_legacy_create_bulk);
        if let Some(ref c) = made {
            eprintln!(
                "[SlotModelProbe] amp+cab legacy CREATE head=2d ({} o) substitué au bulkHex assign (head=23, cd:07)",
                c.len()
            );
        }
        made
    } else {
        None
    };
    let usb_assign_full_bulk: Option<&[u8]> = cab_dual_create_owned
        .as_deref()
        .or(amp_cab_legacy_create_owned.as_deref())
        .or(usb_assign_full_bulk);

    let (_op_short, bulk_template): ([u8; 4], Cow<'_, [u8]>) = match (op, usb_assign_full_bulk) {
        (_, Some(b)) if b.len() >= 32 => (ed_op_from_assign_bulk_prefix(b), Cow::Borrowed(b)),
        (SlotModelProbeOp::AddToEmpty, _) => (
            [0x80, 0x10, 0xed, 0x03],
            Cow::Borrowed(&ADD_MODEL_BULK_TEMPLATE[..]),
        ),
        (SlotModelProbeOp::RemoveFromOccupied, _) => (
            [0x80, 0x10, 0xed, 0x03],
            Cow::Borrowed(&REMOVE_MODEL_BULK36_CD03_TEMPLATE[..]),
        ),
        _ => (
            [0x80, 0x10, 0xed, 0x03],
            Cow::Borrowed(&REPLACE_MODEL_BULK48_CD04_TEMPLATE[..]),
        ),
    };

    let use_json_bulk = matches!(usb_assign_full_bulk, Some(b) if b.len() >= 32);

    // Plus de routage conditionnel par opcode ED03 (0310/8010) sur le chemin JSON:
    // on garde une seule cinématique de sonde pour éviter les faux diagnostics.
    //
    // Préambule unifié: deux courts de contexte (ef puis f0) avant le bulk.
    // Cela évite les transitions de session "bloquées" après un envoi 0310 qui
    // peuvent ensuite faire ignorer les bulks 8010 (et inversement).
    if use_json_bulk && !cab_dual_cab2_replace_after_focus {
        let mut pre_ef = [0u8; 16];
        let mut pre_f0 = [0u8; 16];
        let ctr0 = state.live_write_ctr;
        let ctr1 = ctr0.wrapping_add(0x1f);
        patch_short_ed03_16(
            &mut pre_ef,
            // Windows capture: ef packet appears as `... ef 03 01 10 ...`
            [0xef, 0x03, 0x01, 0x10],
            // Windows capture: byte11 is 0x10 for this ef preamble.
            0x10,
            state.next_x2_cnt(),
            ctr0,
        );
        patch_short_ed03_16(
            &mut pre_f0,
            [0x02, 0x10, 0xf0, 0x03],
            0x10,
            state.next_x2_cnt(),
            ctr1,
        );
        packets.push(pre_ef.to_vec());
        packets.push(pre_f0.to_vec());
    }
    if !use_json_bulk && matches!(op, SlotModelProbeOp::RemoveFromOccupied) {
        // Capture remove HX Edit: court `... 00 10 ...` juste avant le bulk.
        let mut short1 = [0u8; 16];
        let seq1 = state.next_x80_cnt();
        let ctr0 = state.live_write_ctr;
        patch_short_ed03_16(&mut short1, _op_short, 0x10, seq1, ctr0);
        packets.push(short1.to_vec());
    }

    // 1) Bulk (mode "replay strict" pour les captures JSON: pas d'enveloppe courte ajoutée).
    let mut bulk = bulk_template.to_vec();
    if use_json_bulk {
        // Chemin JSON unifié:
        // - seq x80 (octet 9) seulement — identité modèle octets 12–15 figée capture,
        // - lane séquentiel quand un marqueur `83 66 cd 03|04` est présent.
        let bulk_seq_used = state.next_x80_cnt();
        patch_bulk_header_seq_x80_only(&mut bulk, bulk_seq_used);

        if let Some(off) = cd_lane_offset_after_cd03_or_cd04(&bulk) {
            let next_lane = state.slot_model_lane_seq.unwrap_or(bulk[off]);
            bulk[off] = next_lane;
            state.slot_model_lane_seq = Some(next_lane.wrapping_add(1));
        } else if matches!(op, SlotModelProbeOp::ReplaceOccupied) {
            if let Some(off) = cd_lane_tag_offset_after_cd08(&bulk) {
                // Replace Amp+Cab legacy occupé : tag session dynamique (≠ `fb` figé de l’assign cd:07).
                let tag = state.slot_model_lane_seq.unwrap_or(state.live_write_yy);
                bulk[off] = tag;
                let next = tag.wrapping_add(1);
                state.slot_model_lane_seq = Some(next);
                state.live_write_yy = next;
            }
        }
    } else {
        let ctr0 = state.live_write_ctr;
        let seq2 = state.next_x80_cnt();
        patch_bulk_header_counters(&mut bulk, seq2, ctr0);
    }
    patch_slot_bus_in_bulk(&mut bulk, slot_bus);
    if use_json_bulk {
        // IMPORTANT: pour un bulk capturé depuis `HX_ModelUsbAssign.json`, on ne doit PAS
        // réécrire l'octet qui suit `83 66 cd 04`.
        // Sur les captures mono complètes, cet octet suit une progression de session
        // (ex. 0x26, 0x27, …), pas un simple `2 * slotIndex`. L'écraser casse la trame.
        // On ne patche donc ici que le `slot_bus`.
        patch_slot_bus_in_bulk(&mut bulk, slot_bus);
    } else if let Some(ch) = catalog_chain_bytes {
        if !patch_catalog_chain_into_bulk(&mut bulk, ch) {
            eprintln!(
                "[SlotModelProbe] chainHex catalogue ignoré pour fusion USB (trop court ou sans préfixe 83 66 cd ; ids courts Mono/Stereo non utilisables) — corps du template capture conservé, slot_bus seulement."
            );
        } else {
            patch_slot_bus_in_bulk(&mut bulk, slot_bus);
        }
        if matches!(op, SlotModelProbeOp::ReplaceOccupied | SlotModelProbeOp::RemoveFromOccupied) {
            patch_kempline_lane_after_cd03_or_cd04(&mut bulk, kempline_index);
            patch_slot_bus_in_bulk(&mut bulk, slot_bus);
        }
    } else if matches!(op, SlotModelProbeOp::ReplaceOccupied | SlotModelProbeOp::RemoveFromOccupied) {
        patch_kempline_lane_after_cd03_or_cd04(&mut bulk, kempline_index);
        patch_slot_bus_in_bulk(&mut bulk, slot_bus);
    }
    let bulk_len = bulk.len();
    packets.push(bulk);
    if !use_json_bulk {
        // 2) Short 16 — byte11 = 0x08 (clôture post-bulk sur le chemin template historique).
        let ctr0 = state.live_write_ctr;
        let ctr_delta = if matches!(op, SlotModelProbeOp::ReplaceOccupied) {
            match bulk_len {
                44 => 0x44u16,
                48 => 0x3eu16,
                _ => 0x1fu16,
            }
        } else if matches!(op, SlotModelProbeOp::RemoveFromOccupied) {
            // Capture `Delete Model HXEdit.json`: 0x2abb -> 0x2acc (+0x11) entre bulk et court de clôture.
            0x11u16
        } else {
            0x1f
        };
        let ctr1 = ctr0.wrapping_add(ctr_delta);
        let seq3 = state.next_x80_cnt();
        let mut short2 = [0u8; 16];
        patch_short_ed03_16(&mut short2, _op_short, 0x08, seq3, ctr1);
        packets.push(short2.to_vec());
        state.live_write_ctr = state.live_write_ctr.wrapping_add(ctr_delta);
    } else if let Some(b) = packets.first() {
        // Chemin JSON capturé: rejouer le bulk seul (compteurs dynamiques déjà patchés).
        // Les captures ne montrent pas une clôture ED03 immédiate de façon fiable pour tous
        // les modèles; l'ajouter systématiquement peut invalider l'assignation.
        let ctr_delta = match b.len() {
            44 => 0x44u16,
            48 => 0x3eu16,
            _ => 0x1fu16,
        };
        state.live_write_ctr = state.live_write_ctr.wrapping_add(ctr_delta);
    }

    packets
}

const AMP_CAB_BULK_MARKER: [u8; 2] = [0xc3, 0x19];
const C219_BULK_MARKER: [u8; 2] = [0xc2, 0x19];

/// Champ module (`<hex…>`) après un marqueur `c219` dans un bulk cab `single` / `legacy`.
pub fn module_field_bytes_after_c219(bulk: &[u8]) -> Option<Vec<u8>> {
    let pos = bulk
        .windows(C219_BULK_MARKER.len())
        .position(|w| w == C219_BULK_MARKER)?;
    let start = pos + C219_BULK_MARKER.len();
    let tail = bulk.get(start..)?;
    let end = tail.iter().position(|&b| b == 0x1a).unwrap_or(tail.len());
    if end == 0 {
        return None;
    }
    Some(tail[..end].to_vec())
}

/// Bornes `[start, end)` du fil wire entre `c319` et le séparateur `1a` (ex. `2c` ou `23`).
fn amp_cab_wire_range_before_1a_in_bulk(bulk: &[u8]) -> Option<(usize, usize)> {
    let pos = bulk
        .windows(AMP_CAB_BULK_MARKER.len())
        .position(|w| w == AMP_CAB_BULK_MARKER)?;
    let cursor = pos + AMP_CAB_BULK_MARKER.len();
    let sep = bulk.get(cursor..)?.iter().position(|&b| b == 0x1a)?;
    let start = cursor;
    let end = cursor + sep;
    if end <= start {
        return None;
    }
    Some((start, end))
}

/// Reprend le fil avant `1a` depuis une entrée `amp+cab-legacy` catalogue qui embarque ce cab.
fn legacy_amp_cab_wire_before_1a_for_cab_field(cab_field: &[u8]) -> Option<Vec<u8>> {
    let entries = USB_ASSIGN_ENTRIES.get_or_init(load_usb_assign_entries);
    for e in entries {
        if e.variant != "amp+cab-legacy" {
            continue;
        }
        let (cs, ce) = amp_cab_cab_field_range_in_bulk(&e.bulk)?;
        if &e.bulk[cs..ce] != cab_field {
            continue;
        }
        let (ws, we) = amp_cab_wire_range_before_1a_in_bulk(&e.bulk)?;
        return Some(e.bulk[ws..we].to_vec());
    }
    None
}

/// Bornes `[start, end)` du champ cab dans un bulk assign `amp+cab` / `amp+cab-legacy`.
fn amp_cab_cab_field_range_in_bulk(bulk: &[u8]) -> Option<(usize, usize)> {
    let pos = bulk
        .windows(AMP_CAB_BULK_MARKER.len())
        .position(|w| w == AMP_CAB_BULK_MARKER)?;
    let cursor = pos + AMP_CAB_BULK_MARKER.len();
    let sep = bulk.get(cursor..)?.iter().position(|&b| b == 0x1a)?;
    let cab_start = cursor + sep + 1;
    if cab_start >= bulk.len() {
        return None;
    }
    let tail = &bulk[cab_start..];
    let cab_end = if let Some(p) = tail.iter().position(|&b| b == 0x09) {
        cab_start + p
    } else if tail.first() == Some(&0xcd) && tail.len() >= 3 {
        cab_start + 3
    } else if tail.first() == Some(&0x00) {
        return None;
    } else {
        cab_start + 1
    };
    if cab_end <= cab_start {
        return None;
    }
    Some((cab_start, cab_end))
}

/// Replace legacy : patch le fil avant `1a` (ex. `2c`→`23`) en plus du cab après `1a`.
fn patch_amp_cab_bulk_legacy_cab_wire(bulk: &mut Vec<u8>, cab_field: &[u8]) -> Result<(), String> {
    let wire_before = legacy_amp_cab_wire_before_1a_for_cab_field(cab_field).ok_or_else(|| {
        format!(
            "wire legacy avant 1a introuvable pour cab {:?} — aucune entrée amp+cab-legacy catalogue",
            cab_field
        )
    })?;
    let (ws, we) = amp_cab_wire_range_before_1a_in_bulk(bulk)
        .ok_or_else(|| "bulk ampli sans marqueur amp+cab (c319/1a)".to_string())?;
    let old_len = we - ws;
    if wire_before.len() == old_len {
        bulk[ws..we].copy_from_slice(&wire_before);
        return Ok(());
    }
    bulk.splice(ws..we, wire_before.iter().copied());
    Ok(())
}

/// Remplace la partie cab (`… 1a <cab> …`) dans un bulk Amp+Cab existant.
pub fn patch_amp_cab_bulk_cab_field(bulk: &mut Vec<u8>, new_cab: &[u8]) -> Result<(), String> {
    if new_cab.is_empty() {
        return Err("champ cab vide".into());
    }
    let (start, end) =
        amp_cab_cab_field_range_in_bulk(bulk).ok_or_else(|| "marqueur amp+cab (c319/1a) introuvable".to_string())?;
    let old_len = end - start;
    if new_cab.len() == old_len {
        bulk[start..end].copy_from_slice(new_cab);
        return Ok(());
    }
    bulk.splice(start..end, new_cab.iter().copied());
    Ok(())
}

/// Bornes `[start, end)` du champ cab2 (`… 1a <cab2> …`) dans un bulk Cab dual / Amp+Cab.
pub fn cab_dual_cab2_field_range_in_bulk(bulk: &[u8]) -> Option<(usize, usize)> {
    amp_cab_cab_field_range_in_bulk(bulk)
}

/// Bornes `[start, end)` du premier cab (`<cab1> 1a <cab2>`) dans un bulk Cab dual.
pub(crate) fn cab_dual_cab1_field_range_in_bulk(bulk: &[u8]) -> Option<(usize, usize)> {
    let pos = bulk
        .windows(AMP_CAB_BULK_MARKER.len())
        .position(|w| w == AMP_CAB_BULK_MARKER)?;
    let cursor = pos + AMP_CAB_BULK_MARKER.len();
    let sep = bulk.get(cursor..)?.iter().position(|&b| b == 0x1a)?;
    let cab_start = cursor;
    let cab_end = cursor + sep;
    if cab_end <= cab_start {
        return None;
    }
    Some((cab_start, cab_end))
}

/// Sur slot occupé après add dual : replace `83:66:cd:04` (Stomp XL + `cab dual change right.json`).
fn reframe_cab_dual_bulk_cd0a_to_cd04_for_replace(bulk: &mut [u8]) {
    for i in 0..bulk.len().saturating_sub(4) {
        if bulk[i] == 0x83
            && bulk[i + 1] == 0x66
            && bulk[i + 2] == 0xcd
            && bulk[i + 3] == 0x0a
        {
            bulk[i + 3] = 0x04;
            return;
        }
    }
}

/// Remplace cab1 (avant `1a`) ou cab2 (après `1a`) dans un bulk Cab dual existant.
pub fn patch_cab_dual_bulk_cab_field(
    bulk: &mut Vec<u8>,
    cab_index: u8,
    new_cab: &[u8],
) -> Result<(), String> {
    if new_cab.is_empty() {
        return Err("champ cab vide".into());
    }
    let range = match cab_index {
        0 => cab_dual_cab1_field_range_in_bulk(bulk),
        1 => amp_cab_cab_field_range_in_bulk(bulk),
        _ => None,
    }
    .ok_or_else(|| format!("marqueur cab dual (c319/1a) introuvable pour cab {cab_index}"))?;
    let (start, end) = range;
    let old_len = end - start;
    if new_cab.len() == old_len {
        bulk[start..end].copy_from_slice(new_cab);
        return Ok(());
    }
    bulk.splice(start..end, new_cab.iter().copied());
    Ok(())
}

/// Bulk `replace` Cab dual : même entrée `dual`, un seul cab patché (`cab_index` 0 ou 1).
pub fn build_cab_dual_replace_cab_bulk(
    dual_model_id: &str,
    cab_model_id: &str,
    cab_variant: &str,
    cab_index: u8,
) -> Result<Vec<u8>, String> {
    if cab_index > 1 {
        return Err(format!("cab_index attendu 0 ou 1, reçu {cab_index}"));
    }
    let mut bulk = resolve_usb_assign_bulk(dual_model_id, "dual").ok_or_else(|| {
        format!(
            "Pas d'entrée HX_ModelUsbAssign pour cab dual {:?} variant dual",
            dual_model_id
        )
    })?;
    let cab_v = cab_variant.trim();
    let cab_bulk = resolve_usb_assign_bulk(cab_model_id, cab_v).ok_or_else(|| {
        format!(
            "Pas d'entrée HX_ModelUsbAssign pour cab {:?} variant {:?}",
            cab_model_id, cab_v
        )
    })?;
    // Replace cab2 : hint dual wire (`c319` cab1 du bulk WithPan / legacy dual), pas `c219` single.
    let cab_field = if cab_v.eq_ignore_ascii_case("dual") {
        let (start, end) = cab_dual_cab1_field_range_in_bulk(&cab_bulk).ok_or_else(|| {
            format!("bulk cab dual sans c319 cab1 exploitable ({cab_model_id})")
        })?;
        cab_bulk[start..end].to_vec()
    } else {
        module_field_bytes_after_c219(&cab_bulk)
            .ok_or_else(|| format!("bulk cab sans bloc c219 exploitable ({cab_model_id})"))?
    };

    let legacy_cab2 = cab_index == 1
        && (crate::helix::cab_dual::legacy::wire::bulk_is_legacy_dual_hybrid(&bulk)
            || (cab_v.eq_ignore_ascii_case("dual")
                && crate::helix::cab_dual::legacy::wire::bulk_is_legacy_dual_hybrid(&cab_bulk)));

    if legacy_cab2 {
        bulk = crate::helix::cab_dual::legacy::wire::build_legacy_cab2_replace_bulk(
            &bulk,
            &cab_bulk,
            &cab_field,
        )?;
    } else {
        patch_cab_dual_bulk_cab_field(&mut bulk, cab_index, &cab_field)?;
        reframe_cab_dual_bulk_cd0a_to_cd04_for_replace(&mut bulk);
    }
    Ok(bulk)
}

/// Champ cab pour replace Amp+Cab : longueur = slot parent ; legacy = `chainHexHint` catalogue.
fn cab_field_bytes_for_amp_cab_replace(
    parent_amp_bulk: &[u8],
    amp_cab_variant: &str,
    cab_model_id: &str,
    cab_variant: &str,
    cab_bulk: &[u8],
) -> Result<Vec<u8>, String> {
    use crate::helix::cab_dual::legacy::wire::chain_hint_to_cab_field_bytes;

    let target_len = {
        let (start, end) = amp_cab_cab_field_range_in_bulk(parent_amp_bulk)
            .ok_or_else(|| "bulk ampli sans marqueur amp+cab (c319/1a)".to_string())?;
        end - start
    };

    if amp_cab_variant.eq_ignore_ascii_case("amp+cab-legacy") {
        if let Some(hint) = resolve_usb_assign_chain_hex_hint(cab_model_id, cab_variant) {
            if let Some(field) = chain_hint_to_cab_field_bytes(&hint) {
                if field.len() == target_len {
                    return Ok(field);
                }
                if target_len == 1 && field.len() == 3 {
                    return Err(format!(
                        "cab legacy {:?} (hint {hint}) incompatible avec ampli compact legacy \
                         (cab 1 o) — choisir un cab hybrid court",
                        cab_model_id
                    ));
                }
            }
        }
    }

    let from_c219 = module_field_bytes_after_c219(cab_bulk)
        .ok_or_else(|| format!("bulk cab sans bloc c219 exploitable ({cab_model_id})"))?;
    if from_c219.len() == target_len || amp_cab_variant.eq_ignore_ascii_case("amp+cab") {
        return Ok(from_c219);
    }
    Err(format!(
        "cab {:?} : champ {from_c219:?} ({} o) incompatible avec le slot parent ({} o)",
        cab_model_id,
        from_c219.len(),
        target_len
    ))
}

/// Têtes bulk replace cab Amp+Cab (IR `0x27`, legacy court `0x23`, legacy long `0x25`).
pub fn accepted_amp_cab_cab_replace_heads() -> &'static [u8] {
    &[0x27, 0x23, 0x25]
}

/// Bulk `replace` Amp+Cab : même ampli (`amp+cab` / `amp+cab-legacy`), cab issu d'une entrée cab.
pub fn build_amp_cab_replace_cab_bulk(
    amp_model_id: &str,
    amp_cab_variant: &str,
    cab_model_id: &str,
    cab_variant: &str,
) -> Result<Vec<u8>, String> {
    let amp_v = amp_cab_variant.trim().to_ascii_lowercase();
    if amp_v != "amp+cab" && amp_v != "amp+cab-legacy" {
        return Err(format!(
            "variante ampli attendue amp+cab ou amp+cab-legacy, reçu {:?}",
            amp_cab_variant
        ));
    }
    let mut bulk = resolve_usb_assign_bulk(amp_model_id, &amp_v).ok_or_else(|| {
        format!(
            "Pas d'entrée HX_ModelUsbAssign pour ampli {:?} variant {:?}",
            amp_model_id, amp_v
        )
    })?;
    let cab_bulk = resolve_usb_assign_bulk(cab_model_id, cab_variant.trim()).ok_or_else(|| {
        format!(
            "Pas d'entrée HX_ModelUsbAssign pour cab {:?} variant {:?}",
            cab_model_id, cab_variant
        )
    })?;
    let cab_field = cab_field_bytes_for_amp_cab_replace(
        &bulk,
        &amp_v,
        cab_model_id,
        cab_variant,
        &cab_bulk,
    )?;
    patch_amp_cab_bulk_cab_field(&mut bulk, &cab_field)?;
    if amp_v == "amp+cab-legacy" {
        patch_amp_cab_bulk_legacy_cab_wire(&mut bulk, &cab_field)?;
    }
    eprintln!(
        "[build_amp_cab_replace_cab_bulk] amp={} variant={} cab={} cab_variant={} → {:02x?}",
        amp_model_id, amp_cab_variant, cab_model_id, cab_variant, &bulk
    );
    Ok(bulk)
}

/// Préambule `ef` puis `f0` (16 o chacun) avant replace cab legacy occupé (captures HX Edit).
pub fn build_ef_f0_slot_preamble_packets(state: &mut super::HelixState) -> Vec<Vec<u8>> {
    let mut pre_ef = [0u8; 16];
    let mut pre_f0 = [0u8; 16];
    let ctr0 = state.live_write_ctr;
    let ctr1 = ctr0.wrapping_add(0x1f);
    patch_short_ed03_16(
        &mut pre_ef,
        [0xef, 0x03, 0x01, 0x10],
        0x10,
        state.next_x2_cnt(),
        ctr0,
    );
    patch_short_ed03_16(
        &mut pre_f0,
        [0x02, 0x10, 0xf0, 0x03],
        0x10,
        state.next_x2_cnt(),
        ctr1,
    );
    vec![pre_ef.to_vec(), pre_f0.to_vec()]
}

#[cfg(test)]
mod amp_cab_replace_cab_tests {
    use super::*;

    #[test]
    fn patches_who_watt_default_cab_with_greenback() {
        let mut bulk = resolve_usb_assign_bulk("HD2_AmpWhoWatt100", "amp+cab").expect("amp+cab bulk");
        let cab_bulk =
            resolve_usb_assign_bulk("HD2_CabMicIr_4x12Greenback20", "single").expect("cab bulk");
        let new_cab = module_field_bytes_after_c219(&cab_bulk).expect("cab field");
        assert_eq!(new_cab, vec![0xcd, 0x02, 0xf2]);
        patch_amp_cab_bulk_cab_field(&mut bulk, &new_cab).expect("patch");
        let (_, end) = amp_cab_cab_field_range_in_bulk(&bulk).expect("range");
        assert_eq!(&bulk[end - 3..end], &[0xcd, 0x02, 0xf2]);
    }

    #[test]
    fn build_replace_cab_bulk_keeps_amp_cab_frame() {
        let bulk = build_amp_cab_replace_cab_bulk(
            "HD2_AmpWhoWatt100",
            "amp+cab",
            "HD2_CabMicIr_4x12Greenback20",
            "single",
        )
        .expect("build");
        assert!(bulk.windows(2).any(|w| w == AMP_CAB_BULK_MARKER));
        let cab = module_field_bytes_after_c219(
            &resolve_usb_assign_bulk("HD2_CabMicIr_4x12Greenback20", "single").unwrap(),
        )
        .unwrap();
        let (s, e) = amp_cab_cab_field_range_in_bulk(&bulk).unwrap();
        assert_eq!(&bulk[s..e], cab.as_slice());
    }

    #[test]
    fn build_legacy_amp_cab_replace_cab_one_byte_hint() {
        let bulk = build_amp_cab_replace_cab_bulk(
            "HD2_AmpTucknGo",
            "amp+cab-legacy",
            "HD2_Cab1x6x9SoupProEllipse",
            "single",
        )
        .expect("build");
        assert_eq!(bulk.len(), 44);
        assert_eq!(bulk.first().copied(), Some(0x23));
        let (ws, we) = amp_cab_wire_range_before_1a_in_bulk(&bulk).expect("wire");
        assert_eq!(&bulk[ws..we], &[0x23]);
        let (s, e) = amp_cab_cab_field_range_in_bulk(&bulk).expect("range");
        assert_eq!(e - s, 1);
        assert_eq!(bulk[s], 0x33);
    }

    #[test]
    fn json_usb_assign_replace_cd08_patches_session_lane_tag() {
        let mut state = super::super::HelixState::new();
        state.live_write_yy = 0xb8;
        state.slot_model_lane_seq = Some(0xb8);
        let mut bulk = build_amp_cab_replace_cab_bulk(
            "HD2_AmpWhoWatt100",
            "amp+cab-legacy",
            "HD2_Cab1x6x9SoupProEllipse",
            "single",
        )
        .expect("build");
        assert_eq!(bulk[27], 0x07);
        bulk[27] = 0x08;
        let packs = build_slot_model_probe_packets(
            &mut state,
            SlotModelProbeOp::ReplaceOccupied,
            0,
            0x01,
            None,
            Some(&bulk),
            true,
        );
        let sent = packs.iter().find(|p| p.len() == 44).expect("bulk");
        assert_eq!(&sent[24..28], &[0x83, 0x66, 0xcd, 0x08]);
        assert_eq!(
            sent[28], 0xb8,
            "tag session après cd:08 (capture bass 1775 ≈ b9), pas fb de l'assign cd:07"
        );
        assert_eq!(state.live_write_yy, 0xb9);
    }

    #[test]
    fn json_usb_assign_probe_preserves_amp_model_wire_bytes() {
        let mut state = super::super::HelixState::new();
        state.live_write_ctr = 0x6d20;
        let bulk = build_amp_cab_replace_cab_bulk(
            "HD2_AmpWhoWatt100",
            "amp+cab-legacy",
            "HD2_Cab1x6x9SoupProEllipse",
            "single",
        )
        .expect("build");
        assert_eq!(&bulk[12..16], &[0xd1, 0xa7, 0x02, 0x00]);
        let packs = build_slot_model_probe_packets(
            &mut state,
            SlotModelProbeOp::ReplaceOccupied,
            0,
            0x01,
            None,
            Some(&bulk),
            true,
        );
        let sent = packs
            .iter()
            .find(|p| p.len() == 44 && p.first() == Some(&0x23))
            .expect("bulk 44 o");
        assert_eq!(
            &sent[12..16],
            &[0xd1, 0xa7, 0x02, 0x00],
            "octets 12–15 = identité ampli catalogue, pas live_write_ctr"
        );
    }

    #[test]
    fn build_who_watt_legacy_replace_soup_updates_wire_before_1a() {
        let bulk = build_amp_cab_replace_cab_bulk(
            "HD2_AmpWhoWatt100",
            "amp+cab-legacy",
            "HD2_Cab1x6x9SoupProEllipse",
            "single",
        )
        .expect("build");
        let (ws, we) = amp_cab_wire_range_before_1a_in_bulk(&bulk).expect("wire");
        assert_eq!(
            &bulk[ws..we],
            &[0x23],
            "capture amp_cab legacy guitar frame 1735 : c319 23 1a 33"
        );
        let (s, e) = amp_cab_cab_field_range_in_bulk(&bulk).expect("cab");
        assert_eq!(&bulk[s..e], &[0x33]);
    }

    #[test]
    fn legacy_long_cab_rejected_on_compact_amp_cab_slot() {
        let err = build_amp_cab_replace_cab_bulk(
            "HD2_AmpTucknGo",
            "amp+cab-legacy",
            "HD2_Cab1x10PrincessCopperhead",
            "single",
        )
        .expect_err("cd024d on 1-byte slot");
        assert!(err.contains("incompatible"));
    }

    #[test]
    fn patches_cab_dual_second_cab_without_touching_first() {
        let dual_id = "HD2_CabMicIr_SoupProEllipseWithPan";
        let mut bulk = resolve_usb_assign_bulk(dual_id, "dual").expect("dual bulk");
        let cab_bulk =
            resolve_usb_assign_bulk("HD2_CabMicIr_4x12Greenback20", "single").expect("cab bulk");
        let new_cab = module_field_bytes_after_c219(&cab_bulk).expect("cab field");
        let (c1s, c1e) = cab_dual_cab1_field_range_in_bulk(&bulk).expect("cab1");
        let cab1_before = bulk[c1s..c1e].to_vec();
        patch_cab_dual_bulk_cab_field(&mut bulk, 1, &new_cab).expect("patch cab2");
        assert_eq!(&bulk[c1s..c1e], cab1_before.as_slice());
        let (_, end) = amp_cab_cab_field_range_in_bulk(&bulk).expect("cab2 range");
        assert_eq!(&bulk[end - new_cab.len()..end], new_cab.as_slice());
    }

    #[test]
    fn build_cab_dual_replace_deluxe_single_cab2_on_soup_dual() {
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_CabMicIr_SoupProEllipseWithPan",
            "HD2_CabMicIr_1x12USDeluxe",
            "single",
            1,
        )
        .expect("deluxe single cab2");
        assert!(bulk.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x04]));
    }

    #[test]
    fn cab_dual_cab2_replace_probe_skips_ef_f0_preamble() {
        let mut state = super::super::HelixState::new();
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_CabMicIr_SoupProEllipseWithPan",
            "HD2_CabMicIr_1x8SmallTweed",
            "single",
            1,
        )
        .expect("bulk");
        let packs = build_slot_model_probe_packets(
            &mut state,
            SlotModelProbeOp::ReplaceOccupied,
            0,
            0x01,
            None,
            Some(&bulk),
            true,
        );
        assert_eq!(packs.len(), 1, "cab2 replace = bulk seul après focus");
        assert_eq!(packs[0].len(), 48);
        assert_eq!(packs[0][0], 0x27);
    }

    #[test]
    fn cab_dual_legacy_cab2_replace_probe_head_23() {
        let mut state = super::super::HelixState::new();
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_Cab1x6x9SoupProEllipse",
            "HD2_Cab1x12Celest12H",
            "dual",
            1,
        )
        .expect("legacy bulk");
        let packs = build_slot_model_probe_packets(
            &mut state,
            SlotModelProbeOp::ReplaceOccupied,
            0,
            0x01,
            None,
            Some(&bulk),
            true,
        );
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].len(), 44);
        assert_eq!(packs[0][0], 0x23);
    }

    #[test]
    fn cab_dual_legacy_cab2_cd02xx_probe_head_25() {
        let mut state = super::super::HelixState::new();
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_Cab1x12Lead80",
            "HD2_Cab1x12PrincessBlue",
            "dual",
            1,
        )
        .expect("legacy princess bulk");
        let packs = build_slot_model_probe_packets(
            &mut state,
            SlotModelProbeOp::ReplaceOccupied,
            0,
            0x01,
            None,
            Some(&bulk),
            true,
        );
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].len(), 48);
        assert_eq!(packs[0][0], 0x25);
    }

    #[test]
    fn build_cab_dual_replace_cab2_bulk_keeps_dual_frame() {
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_CabMicIr_SoupProEllipseWithPan",
            "HD2_CabMicIr_4x12Greenback20",
            "single",
            1,
        )
        .expect("build");
        assert!(
            bulk.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x04]),
            "replace cab dual doit utiliser cd:04 (Stomp), pas cd:0a add"
        );
        assert!(bulk.windows(2).any(|w| w == AMP_CAB_BULK_MARKER));
        let cab = module_field_bytes_after_c219(
            &resolve_usb_assign_bulk("HD2_CabMicIr_4x12Greenback20", "single").unwrap(),
        )
        .unwrap();
        let (s, e) = amp_cab_cab_field_range_in_bulk(&bulk).unwrap();
        assert_eq!(&bulk[s..e], cab.as_slice());
    }

    #[test]
    fn amp_cab_legacy_create_head2d_keeps_cab_and_registers_structure() {
        let assign = resolve_usb_assign_bulk("HD2_AmpWhoWatt100", "amp+cab-legacy").expect("assign");
        let create = build_amp_cab_legacy_create_bulk(&assign).expect("create");
        assert_eq!(create[0], 0x2d);
        assert_eq!(create.len(), 56);
        assert!(create.windows(6).any(|w| w == AMP_CAB_LEGACY_CREATE_SEGMENT));
        assert!(create.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x03]));
        assert_eq!(create[20], 0x1d);
        assert_eq!(&create[create.len() - 7..], AMP_CAB_LEGACY_CREATE_TRAILER);
        let (as_s, as_e) = amp_cab_cab_field_range_in_bulk(&assign).unwrap();
        let (cs, ce) = amp_cab_cab_field_range_in_bulk(&create).unwrap();
        assert_eq!(&create[cs..ce], &assign[as_s..as_e]);
        assert_eq!(create[cs], 0x47);
    }

    #[test]
    fn amp_cab_legacy_create_bulk_matches_hxedit_frame() {
        let replace = resolve_usb_assign_bulk("HD2_AmpWhoWatt100", "amp+cab-legacy")
            .expect("bulk amp+cab-legacy");
        let create = build_amp_cab_legacy_create_bulk(&replace).expect("create 2d");

        // Structure attendue d'après capture HX Edit frame #338
        assert_eq!(create.len(), 56);
        assert_eq!(create[0], 0x2d, "head");
        assert_eq!(create[20], 0x1d, "payload len");
        assert!(
            create.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x03]),
            "lane cd:03 absente"
        );
        assert!(
            !create.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x07]),
            "cd:07 encore présent"
        );
        assert!(
            create.windows(4).any(|w| w[0] == 0x82 && w[1] == 0x62 && w[3] == 0x63),
            "slot type 0x63 absent"
        );
        assert!(
            create.windows(6).any(|w| w == [0x82, 0x13, 0x06, 0x14, 0x83, 0x18]),
            "segment création absent"
        );
        assert!(
            create.windows(4).any(|w| w == [0x09, 0x12, 0x0a, 0xc3]),
            "trailer absent"
        );
        assert!(
            create.windows(3).any(|w| w == [0x2c, 0x1a, 0x47]),
            "cab défaut WhoWatt absent"
        );
        assert_eq!(&create[53..56], &[0x00, 0x00, 0x00], "padding final");
    }

    #[test]
    fn add_to_empty_amp_cab_legacy_uses_head2d_create() {
        let mut state = super::super::HelixState::new();
        let assign = resolve_usb_assign_bulk("HD2_AmpWhoWatt100", "amp+cab-legacy").expect("assign");
        let packs = build_slot_model_probe_packets(
            &mut state,
            SlotModelProbeOp::AddToEmpty,
            0,
            0x01,
            None,
            Some(&assign),
            false,
        );
        let bulk = packs.iter().find(|p| p.len() == 56).expect("create bulk 56o");
        assert_eq!(bulk[0], 0x2d, "création amp+cab legacy = head=2d");
        assert!(bulk.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x03]));
    }

    #[test]
    fn cab_dual_create_head31_keeps_cab1_and_registers_structure() {
        let dual = resolve_usb_assign_bulk("HD2_CabMicIr_SoupProEllipseWithPan", "dual")
            .expect("dual bulk");
        let create = build_cab_dual_create_bulk(&dual).expect("create");
        // head=31, 60 o, segment de création présent, cd:03 (pas cd:0a)
        assert_eq!(create[0], 0x31);
        assert_eq!(create.len(), 60);
        assert!(create.windows(6).any(|w| w == [0x82, 0x13, 0x06, 0x14, 0x83, 0x18]));
        assert!(create.windows(4).any(|w| w == [0x83, 0x66, 0xcd, 0x03]));
        // cab1 conservé : identique au bulk d'origine.
        let (s, e) = cab_dual_cab1_field_range_in_bulk(&dual).unwrap();
        let (cs, ce) = cab_dual_cab1_field_range_in_bulk(&create).unwrap();
        assert_eq!(&create[cs..ce], &dual[s..e]);
    }

    #[test]
    fn add_to_empty_dual_uses_head31_create() {
        let mut state = super::super::HelixState::new();
        let dual = resolve_usb_assign_bulk("HD2_CabMicIr_SoupProEllipseWithPan", "dual")
            .expect("dual bulk");
        let packs = build_slot_model_probe_packets(
            &mut state,
            SlotModelProbeOp::AddToEmpty,
            0,
            0x01,
            None,
            Some(&dual),
            false,
        );
        // Le bulk de création doit être un head=31 (et non le head=27 d'origine).
        let bulk = packs.iter().find(|p| p.len() >= 48).expect("bulk");
        assert_eq!(bulk[0], 0x31, "création dual = head=31");
    }
}