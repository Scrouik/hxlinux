//! Résolution `chainHexHint` → métadonnées modèle depuis `HX_ModelUsbAssign.json`.
//! Ordre des params UI : fichiers `.models` (le catalogue n’est plus chargé côté TS).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

const HX_MODEL_USB_ASSIGN_JSON: &str = include_str!("../../resources/HX_ModelUsbAssign.json");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainHexCatalogEntry {
    pub chain_hex: String,
    pub name: String,
    pub category: String,
    pub sub_category: String,
    pub model_id: String,
    pub variant: String,
}

/// Variantes picker dont le `chainHexHint` duplique l’entrée `amp` — non indexées seules.
pub fn chain_hex_hint_shared_with_amp(variant: &str) -> bool {
    matches!(
        variant.trim().to_ascii_lowercase().as_str(),
        "amp+cab" | "amp+cab-legacy"
    )
}

/// Index hex scroll : seules les variantes dont le hint est propre au fil USB.
pub fn chain_hex_hint_index_eligible(variant: &str) -> bool {
    !chain_hex_hint_shared_with_amp(variant)
}

/// Priorité lorsqu’un même `chainHexHint` est partagé (mono/stéréo, single/dual cab).
pub fn usb_assign_variant_priority(variant: &str) -> i32 {
    match variant.trim().to_ascii_lowercase().as_str() {
        "amp" => 100,
        "preamp" => 95,
        "stereo" => 50,
        "dual" => 49,
        "mono" => 48,
        "single" => 47,
        "legacy" => 46,
        _ => 1,
    }
}

static CHAIN_HEX_TO_ENTRY: OnceLock<HashMap<String, ChainHexCatalogEntry>> = OnceLock::new();
static HEX_TO_MODEL_ID: OnceLock<HashMap<String, String>> = OnceLock::new();
static MODULE_BY_HEX: OnceLock<HashMap<String, [String; 2]>> = OnceLock::new();

fn load_usb_assign_entries() -> Vec<ChainHexCatalogEntry> {
    let assign: Value =
        serde_json::from_str(HX_MODEL_USB_ASSIGN_JSON).expect("HX_ModelUsbAssign.json invalide");
    let Some(arr) = assign.get("entries").and_then(|x| x.as_array()) else {
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
        let Some(hint) = e
            .get("chainHexHint")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_lowercase())
        else {
            continue;
        };
        if hint.is_empty() {
            continue;
        }
        let name = e
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let category = e
            .get("category")
            .and_then(|x| x.as_str())
            .unwrap_or("Unknown")
            .trim()
            .to_string();
        let sub_category = e
            .get("subCategory")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let entry = ChainHexCatalogEntry {
            chain_hex: hint,
            name,
            category,
            sub_category,
            model_id: id,
            variant,
        };
        if chain_hex_hint_index_eligible(&entry.variant) {
            out.push(entry);
        }
    }
    out
}

fn insert_with_priority<K, V>(
    map: &mut HashMap<K, V>,
    priority_map: &mut HashMap<K, i32>,
    key: K,
    value: V,
    priority: i32,
) where
    K: std::hash::Hash + Eq + Clone,
{
    let existing = priority_map.get(&key).copied().unwrap_or(-1);
    if priority >= existing {
        map.insert(key.clone(), value);
        priority_map.insert(key, priority);
    }
}

fn chain_hex_to_entry_map() -> &'static HashMap<String, ChainHexCatalogEntry> {
    CHAIN_HEX_TO_ENTRY.get_or_init(|| {
        let entries = load_usb_assign_entries();
        let mut map = HashMap::new();
        let mut priority_by_hex: HashMap<String, i32> = HashMap::new();
        for entry in entries {
            let pri = usb_assign_variant_priority(&entry.variant);
            insert_with_priority(
                &mut map,
                &mut priority_by_hex,
                entry.chain_hex.clone(),
                entry,
                pri,
            );
        }
        map
    })
}

/// ID module (`chainHexHint`) → `[catégorie, nom]` pour le parseur preset / scroll.
pub fn module_by_hex_map() -> &'static HashMap<String, [String; 2]> {
    MODULE_BY_HEX.get_or_init(|| {
        let entries = load_usb_assign_entries();
        let mut map = HashMap::new();
        let mut priority_by_hex: HashMap<String, i32> = HashMap::new();
        for entry in entries {
            let pri = usb_assign_variant_priority(&entry.variant);
            insert_with_priority(
                &mut map,
                &mut priority_by_hex,
                entry.chain_hex.clone(),
                [entry.category.clone(), entry.name.clone()],
                pri,
            );
        }
        map
    })
}

/// `chainHexHint` → `id` modèle (symbolicID Line 6).
pub fn model_id_by_hex_map() -> &'static HashMap<String, String> {
    HEX_TO_MODEL_ID.get_or_init(|| {
        let entries = load_usb_assign_entries();
        let mut map = HashMap::new();
        let mut priority_by_hex: HashMap<String, i32> = HashMap::new();
        for entry in entries {
            let pri = usb_assign_variant_priority(&entry.variant);
            insert_with_priority(
                &mut map,
                &mut priority_by_hex,
                entry.chain_hex.clone(),
                entry.model_id.clone(),
                pri,
            );
        }
        map
    })
}

/// Candidats pour joindre l’assign (hex complet, préfixe avant `1a`, sous-chaînes `cdXXXX`).
pub fn chain_hex_lookup_candidates(hex_norm: &str) -> Vec<String> {
    let h = hex_norm.trim().to_lowercase();
    if h.is_empty() {
        return Vec::new();
    }
    let mut out = vec![h.clone()];
    if let Some(i) = h.find("1a") {
        let prefix = h.get(..i).unwrap_or("").to_string();
        if !prefix.is_empty() && prefix != h {
            out.push(prefix);
        }
    }
    let b = h.as_bytes();
    if b.len() >= 6 {
        for i in 0..=b.len() - 6 {
            if b[i] == b'c' && b[i + 1] == b'd' {
                out.push(String::from_utf8_lossy(&b[i..i + 6]).into_owned());
            }
        }
    }
    out.sort_by(|a, b| b.len().cmp(&a.len()));
    out.dedup();
    out
}

/// Retourne les métadonnées assign pour un `chainHex` extrait du dump USB.
pub fn resolve_chain_hex_entry(module_hex: &str) -> Option<ChainHexCatalogEntry> {
    let map = chain_hex_to_entry_map();
    for cand in chain_hex_lookup_candidates(module_hex) {
        if let Some(entry) = map.get(&cand) {
            return Some(entry.clone());
        }
    }
    None
}

/// Retourne `(chainHexHint, nom modèle)` si trouvé dans `HX_ModelUsbAssign.json`.
pub fn resolve_chain_hex_and_name(module_hex: &str) -> Option<(String, String)> {
    resolve_chain_hex_entry(module_hex).map(|e| (e.chain_hex, e.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_heir_apparent_by_chain_hex() {
        let e = resolve_chain_hex_entry("cd0223").expect("cd0223");
        assert_eq!(e.chain_hex, "cd0223");
        assert_eq!(e.name, "Heir Apparent");
        assert_eq!(e.category, "Distortion");
        assert_eq!(e.sub_category, "Mono");
    }

    #[test]
    fn resolves_from_usb_module_id_embedded() {
        let e = resolve_chain_hex_entry("19231a09cd0223").expect("embedded cd0223");
        assert_eq!(e.chain_hex, "cd0223");
        assert_eq!(e.name, "Heir Apparent");
    }

    #[test]
    fn resolves_stereo_variant_by_chain_hex_index() {
        let e = resolve_chain_hex_entry("cd027a").expect("cd027a");
        assert_eq!(e.name, "Tesselator");
        assert_eq!(e.category, "Delay");
        assert_eq!(e.sub_category, "Stereo");
    }

    #[test]
    fn resolves_mono_variant_by_chain_hex_index() {
        let e = resolve_chain_hex_entry("cd0279").expect("cd0279");
        assert_eq!(e.name, "Tesselator");
        assert_eq!(e.sub_category, "Mono");
    }

    #[test]
    fn resolves_short_hex_minotaur_stereo() {
        let e = resolve_chain_hex_entry("70").expect("70");
        assert_eq!(e.name, "Minotaur");
        assert_eq!(e.sub_category, "Stereo");
    }

    #[test]
    fn amp_hint_indexed_not_amp_cab_clone() {
        let map = module_by_hex_map();
        let pair = map.get("2c").expect("2c");
        assert_eq!(pair[0], "Amp");
        assert_eq!(pair[1], "WhoWatt 100");
    }
}
