//! Résolution `chainHex` → nom modèle depuis `HX_ModelCatalog.json`.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

const HX_MODEL_CATALOG_JSON: &str = include_str!("../../resources/HX_ModelCatalog.json");

static CHAIN_HEX_TO_NAME: OnceLock<HashMap<String, String>> = OnceLock::new();

fn chain_hex_to_name_map() -> &'static HashMap<String, String> {
    CHAIN_HEX_TO_NAME.get_or_init(|| {
        let catalog: Value =
            serde_json::from_str(HX_MODEL_CATALOG_JSON).expect("HX_ModelCatalog.json invalide");
        let mut map = HashMap::new();
        insert_models_from_catalog(&catalog, &mut map);
        map
    })
}

fn insert_hex_entry(map: &mut HashMap<String, String>, hex_v: &Value, name: &str) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    match hex_v {
        Value::String(s) => {
            let h = s.trim().to_lowercase();
            if !h.is_empty() {
                map.insert(h, name);
            }
        }
        Value::Array(a) => {
            for x in a {
                if let Some(s) = x.as_str() {
                    let h = s.trim().to_lowercase();
                    if !h.is_empty() {
                        map.insert(h, name.clone());
                    }
                }
            }
        }
        _ => {}
    }
}

fn insert_model_list(map: &mut HashMap<String, String>, models: Option<&Value>) {
    let Some(arr) = models.and_then(|m| m.as_array()) else {
        return;
    };
    for m in arr {
        let Some(obj) = m.as_object() else { continue };
        let Some(name) = obj.get("name").and_then(|x| x.as_str()) else { continue };
        let Some(pm) = obj.get("presetMeta").and_then(|p| p.as_object()) else { continue };
        let Some(hex_v) = pm.get("chainHex") else { continue };
        insert_hex_entry(map, hex_v, name);
    }
}

fn insert_models_from_catalog(catalog: &Value, map: &mut HashMap<String, String>) {
    if let Some(models) = catalog.get("models").and_then(|m| m.as_array()) {
        for m in models {
            let Some(obj) = m.as_object() else { continue };
            let Some(name) = obj.get("name").and_then(|x| x.as_str()) else { continue };
            let Some(pm) = obj.get("presetMeta").and_then(|p| p.as_object()) else { continue };
            let Some(hex_v) = pm.get("chainHex") else { continue };
            insert_hex_entry(map, hex_v, name);
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

/// Retourne `(chainHex catalogue, nom modèle)` si trouvé dans `HX_ModelCatalog.json`.
pub fn resolve_chain_hex_and_name(module_hex: &str) -> Option<(String, String)> {
    let map = chain_hex_to_name_map();
    for cand in chain_hex_lookup_candidates(module_hex) {
        if let Some(name) = map.get(&cand) {
            return Some((cand, name.clone()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_heir_apparent_by_chain_hex() {
        let (chain, name) = resolve_chain_hex_and_name("cd0223").expect("cd0223");
        assert_eq!(chain, "cd0223");
        assert_eq!(name, "Heir Apparent");
    }

    #[test]
    fn resolves_from_usb_module_id_embedded() {
        let (chain, name) =
            resolve_chain_hex_and_name("19231a09cd0223").expect("embedded cd0223");
        assert_eq!(chain, "cd0223");
        assert_eq!(name, "Heir Apparent");
    }
}
