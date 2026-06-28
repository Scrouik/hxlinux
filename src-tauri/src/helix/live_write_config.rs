//! Configuration live write USB (`resources/HelixLiveWrite.json`), complémentaire à `HelixControls.json`.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelixLiveWriteCfg {
    #[serde(default = "default_pp")]
    pub pp_default: u8,
    #[serde(default = "default_bool_off")]
    pub bool_mark_off: u8,
    #[serde(default = "default_bool_on")]
    pub bool_mark_on: u8,
    #[serde(default = "default_bool_display_types")]
    pub bool_display_types: Vec<String>,
    /// `valueType` Line 6 autorisés pour la trame float `0x27` (hors bool `0x23`).
    #[serde(default = "default_allowed_float_value_types")]
    pub allowed_float_value_types: Vec<i32>,
    /// `displayType` → nombre de positions (HelixControls `format` / segmented) ; trame `23` avec octet après `77` = index 0..n-1 (captures HX Edit, ex. comp_ratio).
    #[serde(default)]
    pub discrete_23_display_types: HashMap<String, u8>,
    /// Overrides `PP` par `displayType` (octet après `83:66:cd`), utile pour les familles
    /// qui ne suivent pas `ppDefault` (ex. `wave_shape` capturé en `0x04`).
    #[serde(default)]
    pub pp_by_display_type: HashMap<String, u8>,
}

fn default_pp() -> u8 {
    0x03
}

fn default_bool_off() -> u8 {
    0xc2
}

fn default_bool_on() -> u8 {
    0xc3
}

fn default_bool_display_types() -> Vec<String> {
    vec!["off_on".to_string(), "polarity".to_string()]
}

fn default_allowed_float_value_types() -> Vec<i32> {
    vec![0, 1]
}

impl Default for HelixLiveWriteCfg {
    fn default() -> Self {
        Self {
            pp_default: default_pp(),
            bool_mark_off: default_bool_off(),
            bool_mark_on: default_bool_on(),
            bool_display_types: default_bool_display_types(),
            allowed_float_value_types: default_allowed_float_value_types(),
            discrete_23_display_types: HashMap::new(),
            pp_by_display_type: HashMap::new(),
        }
    }
}

static LIVE_WRITE_CFG: OnceLock<HelixLiveWriteCfg> = OnceLock::new();

pub fn live_write_cfg() -> &'static HelixLiveWriteCfg {
    LIVE_WRITE_CFG.get_or_init(|| {
        const JSON: &str = include_str!("../../resources/HelixLiveWrite.json");
        serde_json::from_str(JSON).unwrap_or_else(|e| {
            eprintln!("[LiveWrite][config] HelixLiveWrite.json parse error: {e} — defaults");
            HelixLiveWriteCfg::default()
        })
    })
}

/// `valueType` Line 6 : 2 = booléen ; sinon on regarde `displayType` (liste `boolDisplayTypes`).
/// Les `displayType` segmentés (`discrete23DisplayTypes`, ex. `comp_mode`) priment sur `valueType` 2.
pub fn infer_bool_wire_payload(display_type: Option<&str>, value_type: Option<i32>) -> bool {
    if discrete_23_step_count(display_type).is_some() {
        return false;
    }
    if matches!(value_type, Some(2)) {
        return true;
    }
    let cfg = live_write_cfg();
    let dt = display_type.map(str::trim).unwrap_or("").to_ascii_lowercase();
    if dt.is_empty() {
        return false;
    }
    cfg.bool_display_types
        .iter()
        .any(|x| x.to_ascii_lowercase() == dt)
}

/// Nombre de positions segmentées pour une trame `23` « index discret » (pas bool c2/c3), si présent dans la config.
pub fn discrete_23_step_count(display_type: Option<&str>) -> Option<u8> {
    let dt = display_type.map(str::trim).unwrap_or("");
    if dt.is_empty() {
        return None;
    }
    let key = dt.to_ascii_lowercase();
    let cfg = live_write_cfg();
    let n = cfg
        .discrete_23_display_types
        .iter()
        .find(|(k, _)| k.to_ascii_lowercase() == key)
        .map(|(_, &v)| v)?;
    if n >= 1 {
        Some(n)
    } else {
        None
    }
}

pub fn pp_override_for_display_type(display_type: Option<&str>) -> Option<u8> {
    let dt = display_type.map(str::trim).unwrap_or("");
    if dt.is_empty() {
        return None;
    }
    let key = dt.to_ascii_lowercase();
    let cfg = live_write_cfg();
    cfg.pp_by_display_type
        .iter()
        .find(|(k, _)| k.to_ascii_lowercase() == key)
        .map(|(_, &v)| v)
}

/// Refuse l’envoi USB si le couple métadonnées ne correspond pas à un chemin protocolaire connu
/// (bool `0x23` ou float `0x27` avec `valueType` autorisé). Limite le risque de figer le DSP avec une trame non capturée.
pub fn validate_usb_live_write_metadata(
    display_type: Option<&str>,
    value_type: Option<i32>,
) -> Result<(), String> {
    if infer_bool_wire_payload(display_type, value_type) {
        return Ok(());
    }
    if discrete_23_step_count(display_type).is_some() {
        return Ok(());
    }
    let vt = value_type.ok_or_else(|| {
        "Live write USB refusé : valueType absent. Ajoute valueType dans le .models ou désactive l’écriture USB pour ce param.".to_string()
    })?;
    let cfg = live_write_cfg();
    if cfg.allowed_float_value_types.contains(&vt) {
        return Ok(());
    }
    Err(format!(
        "Live write USB refusé : valueType={vt} hors liste allowedFloatValueTypes {:?} — le chemin float 0x27 n’est pas validé pour ce type. Capture HX Edit ou étends HelixLiveWrite.json ; sinon risque de comportement erratique sur le hardware.",
        cfg.allowed_float_value_types
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comp_mode_is_discrete_segmented_not_bool_markers() {
        assert_eq!(discrete_23_step_count(Some("comp_mode")), Some(2));
        assert!(!infer_bool_wire_payload(Some("comp_mode"), Some(2)));
    }

    #[test]
    fn off_on_stays_bool_even_with_value_type_2() {
        assert!(infer_bool_wire_payload(Some("off_on"), Some(2)));
    }
}
