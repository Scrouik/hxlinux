//! Résolution `chainHex` → métadonnées modèle depuis `HX_ModelCatalog.json`.
//! Réservé au scroll / catalogue — utilisé par les tests unitaires pour l’instant.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

const HX_MODEL_CATALOG_JSON: &str = include_str!("../../resources/HX_ModelCatalog.json");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainHexCatalogEntry {
    pub chain_hex: String,
    pub name: String,
    pub category: String,
    pub sub_category: String,
}

static CHAIN_HEX_TO_ENTRY: OnceLock<HashMap<String, ChainHexCatalogEntry>> = OnceLock::new();

fn chain_hex_to_entry_map() -> &'static HashMap<String, ChainHexCatalogEntry> {
    CHAIN_HEX_TO_ENTRY.get_or_init(|| {
        let catalog: Value =
            serde_json::from_str(HX_MODEL_CATALOG_JSON).expect("HX_ModelCatalog.json invalide");
        let mut map = HashMap::new();
        insert_models_from_catalog(&catalog, &mut map);
        map
    })
}

fn sub_category_for_index(sub_v: &Value, index: usize) -> String {
    match sub_v {
        Value::String(s) => s.trim().to_string(),
        Value::Array(a) => a
            .get(index)
            .or_else(|| a.first())
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

fn insert_model_object(map: &mut HashMap<String, ChainHexCatalogEntry>, obj: &serde_json::Map<String, Value>) {
    let Some(name) = obj.get("name").and_then(|x| x.as_str()) else {
        return;
    };
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let Some(pm) = obj.get("presetMeta").and_then(|p| p.as_object()) else {
        return;
    };
    let Some(hex_v) = pm.get("chainHex") else {
        return;
    };
    let category = pm
        .get("categoryName")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let sub_v = pm
        .get("subCategory")
        .cloned()
        .unwrap_or(Value::String(String::new()));

    let mut insert_one = |hex: &str, index: usize| {
        let h = hex.trim().to_lowercase();
        if h.is_empty() {
            return;
        }
        map.insert(
            h.clone(),
            ChainHexCatalogEntry {
                chain_hex: h,
                name: name.to_string(),
                category: category.clone(),
                sub_category: sub_category_for_index(&sub_v, index),
            },
        );
    };

    match hex_v {
        Value::String(s) => insert_one(s, 0),
        Value::Array(a) => {
            for (index, x) in a.iter().enumerate() {
                if let Some(s) = x.as_str() {
                    insert_one(s, index);
                }
            }
        }
        _ => {}
    }
}

fn insert_model_list(map: &mut HashMap<String, ChainHexCatalogEntry>, models: Option<&Value>) {
    let Some(arr) = models.and_then(|m| m.as_array()) else {
        return;
    };
    for m in arr {
        let Some(obj) = m.as_object() else { continue };
        insert_model_object(map, obj);
    }
}

fn insert_models_from_catalog(catalog: &Value, map: &mut HashMap<String, ChainHexCatalogEntry>) {
    if let Some(models) = catalog.get("models").and_then(|m| m.as_array()) {
        for m in models {
            let Some(obj) = m.as_object() else { continue };
            insert_model_object(map, obj);
        }
        return;
    }
    let Some(categories) = catalog.get("categories").and_then(|c| c.as_array()) else {
        return;
    };
    for cat in categories {
        insert_model_list(map, cat.get("models"));
        if let Some(subs) = cat.get("subcategories").and_then(|s| s.as_array()) {
            for sub in subs {
                insert_model_list(map, sub.get("models"));
            }
        }
    }
}

/// Candidats pour joindre le catalogue (hex complet, préfixe avant `1a`, sous-chaînes `cdXXXX`).
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

/// Retourne les métadonnées catalogue pour un `chainHex` extrait du dump USB.
pub fn resolve_chain_hex_entry(module_hex: &str) -> Option<ChainHexCatalogEntry> {
    let map = chain_hex_to_entry_map();
    for cand in chain_hex_lookup_candidates(module_hex) {
        if let Some(entry) = map.get(&cand) {
            return Some(entry.clone());
        }
    }
    None
}

/// Retourne `(chainHex catalogue, nom modèle)` si trouvé dans `HX_ModelCatalog.json`.
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
}
