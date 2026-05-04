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

/// Octet du bus slot dans le segment `82 62 **slot** …` (même convention que `live_write`).
fn patch_slot_bus_in_bulk(buf: &mut [u8], slot_bus: u8) {
    for i in 0..buf.len().saturating_sub(2) {
        if buf[i] == 0x82 && buf[i + 1] == 0x62 {
            buf[i + 2] = slot_bus;
            return;
        }
    }
    // Fallback : offset fixe des captures HX Edit (`82 62` à l’index 32).
    if buf.len() > 34 {
        buf[34] = slot_bus;
    }
}

/// Octet après `83 66 cd 04` ou `83 66 cd 03` : sur les captures Preset33, **`2 * index_kempline`**.
/// Même formule pour les indices 8–15 (path 2) tant qu’on n’a pas de contre-exemple USB.
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

/// Octets `… ed 03` dans les courts 16 o (indices 4..8 du bulk assignation : `80 10` ou `03 10`).
fn ed_op_from_assign_bulk_prefix(bulk: &[u8]) -> [u8; 4] {
    if bulk.len() >= 8 {
        return [bulk[4], bulk[5], bulk[6], bulk[7]];
    }
    [0x80, 0x10, 0xed, 0x03]
}

#[derive(Debug, Clone)]
struct UsbAssignEntry {
    id: String,
    variant: String,
    bulk: Vec<u8>,
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
        if let Some(bulk) = parse_hex_bytes(hex) {
            if bulk.len() >= 32 {
                out.push(UsbAssignEntry { id, variant, bulk });
            }
        }
    }
    out
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
/// Si `usb_assign_full_bulk` est fourni (**Replace** uniquement), les courts 16 o reprennent les
/// octets ED du bulk (`80 10` vs `03 10`) et le corps est celui du JSON (captures Preset33) —
/// pas de fusion `chainHex` court catalogue.
pub fn build_slot_model_probe_packets(
    state: &mut super::HelixState,
    op: SlotModelProbeOp,
    kempline_index: usize,
    slot_bus: u8,
    catalog_chain_bytes: Option<&[u8]>,
    usb_assign_full_bulk: Option<&[u8]>,
) -> Vec<Vec<u8>> {
    let ctr0 = state.live_write_ctr;
    let mut packets: Vec<Vec<u8>> = Vec::new();

    let (op_short, bulk_template): ([u8; 4], Cow<'_, [u8]>) = match (op, usb_assign_full_bulk) {
        (SlotModelProbeOp::ReplaceOccupied, Some(b)) if b.len() >= 32 => {
            (ed_op_from_assign_bulk_prefix(b), Cow::Borrowed(b))
        }
        (SlotModelProbeOp::AddToEmpty, _) => (
            [0x80, 0x10, 0xed, 0x03],
            Cow::Borrowed(&ADD_MODEL_BULK_TEMPLATE[..]),
        ),
        _ => (
            [0x80, 0x10, 0xed, 0x03],
            Cow::Borrowed(&REPLACE_MODEL_BULK48_CD04_TEMPLATE[..]),
        ),
    };

    let use_json_replace = matches!(
        (op, usb_assign_full_bulk),
        (SlotModelProbeOp::ReplaceOccupied, Some(b)) if b.len() >= 32
    );

    // 1) Short 16 — byte11 = 0x10
    let seq1 = state.next_x80_cnt();
    let mut short1 = [0u8; 16];
    patch_short_ed03_16(&mut short1, op_short, 0x10, seq1, ctr0);
    packets.push(short1.to_vec());

    // 2) Bulk
    let seq2 = state.next_x80_cnt();
    let mut bulk = bulk_template.to_vec();
    patch_bulk_header_counters(&mut bulk, seq2, ctr0);
    patch_slot_bus_in_bulk(&mut bulk, slot_bus);
    if use_json_replace {
        patch_kempline_lane_after_cd03_or_cd04(&mut bulk, kempline_index);
        patch_slot_bus_in_bulk(&mut bulk, slot_bus);
    } else if let Some(ch) = catalog_chain_bytes {
        if !patch_catalog_chain_into_bulk(&mut bulk, ch) {
            eprintln!(
                "[SlotModelProbe] chainHex catalogue ignoré pour fusion USB (trop court ou sans préfixe 83 66 cd ; ids courts Mono/Stereo non utilisables) — corps du template capture conservé, slot_bus seulement."
            );
        } else {
            patch_slot_bus_in_bulk(&mut bulk, slot_bus);
        }
        if matches!(op, SlotModelProbeOp::ReplaceOccupied) {
            patch_kempline_lane_after_cd03_or_cd04(&mut bulk, kempline_index);
            patch_slot_bus_in_bulk(&mut bulk, slot_bus);
        }
    } else if matches!(op, SlotModelProbeOp::ReplaceOccupied) {
        patch_kempline_lane_after_cd03_or_cd04(&mut bulk, kempline_index);
        patch_slot_bus_in_bulk(&mut bulk, slot_bus);
    }
    packets.push(bulk);

    // 3) Short 16 — byte11 = 0x08 (clôture post-bulk sur les captures).
    // CTR +0x1f : même pas qu’entre deux jambes `27` dans `live_write.rs` (comportement historique stable).
    let ctr1 = ctr0.wrapping_add(0x1f);
    let seq3 = state.next_x80_cnt();
    let mut short2 = [0u8; 16];
    patch_short_ed03_16(&mut short2, op_short, 0x08, seq3, ctr1);
    packets.push(short2.to_vec());

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);

    packets
}
