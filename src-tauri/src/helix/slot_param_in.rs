//! Parse passif des trames IN bulk : bloc paramètre live `85:62:SS:1d:c3:1a:00:1c:PP:77:XX`.
//!
//! Captures de référence : `src/Paquets Json/Slot0_Change_param_#0.json` (et #1, #2).

use serde::Serialize;
use std::collections::HashMap;

use crate::helix::slot_bus_to_kempline_index;

const ANCHOR: [u8; 5] = [0x1d, 0xc3, 0x1a, 0x00, 0x1c];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SlotParamWireKind {
    Float,
    BoolOff,
    BoolOn,
    Discrete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlotParamWireSample {
    pub slot_bus: u8,
    pub param_selector: u8,
    pub kind: SlotParamWireKind,
    /// Float normalisé (souvent 0…1) ou index discret selon `kind`.
    pub numeric_value: f32,
}

impl SlotParamWireSample {
    pub fn value_type_str(self) -> &'static str {
        match self.kind {
            SlotParamWireKind::Float => "float",
            SlotParamWireKind::BoolOff | SlotParamWireKind::BoolOn => "bool",
            SlotParamWireKind::Discrete => "discrete",
        }
    }

    pub fn discrete_index(&self) -> Option<u8> {
        match self.kind {
            SlotParamWireKind::Discrete => Some(self.numeric_value.round() as u8),
            _ => None,
        }
    }

    fn fingerprint_key(&self) -> (u8, u8) {
        (self.slot_bus, self.param_selector)
    }

    fn fingerprint_value(&self) -> u64 {
        let tag = match self.kind {
            SlotParamWireKind::Float => 0u8,
            SlotParamWireKind::BoolOff => 1,
            SlotParamWireKind::BoolOn => 2,
            SlotParamWireKind::Discrete => 3,
        };
        let bits = self.numeric_value.to_bits();
        (tag as u64) << 32 | (bits as u64)
    }
}

/// Cherche toutes les occurrences du motif paramètre dans un buffer IN.
pub fn scan_slot_param_samples(buf: &[u8]) -> Vec<SlotParamWireSample> {
    let mut out = Vec::new();
    if buf.len() < 11 {
        return out;
    }
    let mut i = 0usize;
    while i + 11 <= buf.len() {
        if buf[i] != 0x85 || buf[i + 1] != 0x62 {
            i += 1;
            continue;
        }
        let slot_bus = buf[i + 2];
        if buf.get(i + 3..i + 8) != Some(&ANCHOR) {
            i += 1;
            continue;
        }
        let pp = buf[i + 8];
        if buf.get(i + 9) != Some(&0x77) {
            i += 1;
            continue;
        }
        let tag = buf[i + 10];
        let tail_end = match tag {
            0xca => i + 15,
            _ => i + 11,
        };
        if tail_end > buf.len() {
            i += 1;
            continue;
        }
        if let Some(sample) = decode_tag(slot_bus, pp, tag, buf, i) {
            out.push(sample);
            i += if tag == 0xca { 15 } else { 11 };
            continue;
        }
        i += 1;
    }
    out
}

fn decode_tag(
    slot_bus: u8,
    pp: u8,
    tag: u8,
    buf: &[u8],
    i: usize,
) -> Option<SlotParamWireSample> {
    match tag {
        0xca => {
            let b = buf.get(i + 11..i + 15)?;
            let float_be = [b[0], b[1], b[2], b[3]];
            let v = f32::from_be_bytes(float_be);
            if !v.is_finite() {
                return None;
            }
            Some(SlotParamWireSample {
                slot_bus,
                param_selector: pp,
                kind: SlotParamWireKind::Float,
                numeric_value: v,
            })
        }
        0xc2 => Some(SlotParamWireSample {
            slot_bus,
            param_selector: pp,
            kind: SlotParamWireKind::BoolOff,
            numeric_value: 0.0,
        }),
        0xc3 => Some(SlotParamWireSample {
            slot_bus,
            param_selector: pp,
            kind: SlotParamWireKind::BoolOn,
            numeric_value: 1.0,
        }),
        0x00..=0x0f => Some(SlotParamWireSample {
            slot_bus,
            param_selector: pp,
            kind: SlotParamWireKind::Discrete,
            numeric_value: tag as f32,
        }),
        _ => None,
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotParamChangedPayload {
    pub sequence: u32,
    pub slot_index: u32,
    pub slot_bus: u8,
    /// Index paramètre wire (`PP` après `00:1c`), aligné `param_selector` / live write.
    pub param_index: u32,
    pub value_type: String,
    pub value: serde_json::Value,
}

/// Garde la dernière empreinte émise par `(slot_bus, PP)` pour éviter le spam identique.
pub struct SlotParamEmitState {
    last_fingerprint: HashMap<(u8, u8), u64>,
    pub sequence: u32,
}

impl Default for SlotParamEmitState {
    fn default() -> Self {
        Self {
            last_fingerprint: HashMap::new(),
            sequence: 0,
        }
    }
}

impl SlotParamEmitState {
    pub fn ingest_buffer(&mut self, buf: &[u8]) -> Vec<SlotParamChangedPayload> {
        let mut payloads = Vec::new();
        for sample in scan_slot_param_samples(buf) {
            if let Some(p) = self.try_emit_sample(sample) {
                payloads.push(p);
            }
        }
        payloads
    }

    fn try_emit_sample(&mut self, sample: SlotParamWireSample) -> Option<SlotParamChangedPayload> {
        let slot_index = slot_bus_to_kempline_index(sample.slot_bus)?;
        let key = sample.fingerprint_key();
        let fp = sample.fingerprint_value();
        if self.last_fingerprint.get(&key) == Some(&fp) {
            return None;
        }
        self.last_fingerprint.insert(key, fp);
        self.sequence = self.sequence.wrapping_add(1);

        let slot_bus = sample.slot_bus;
        let param_selector = sample.param_selector;
        let value = match sample.kind {
            SlotParamWireKind::Float => serde_json::json!(sample.numeric_value),
            SlotParamWireKind::BoolOff => serde_json::json!(false),
            SlotParamWireKind::BoolOn => serde_json::json!(true),
            SlotParamWireKind::Discrete => {
                serde_json::json!(sample.discrete_index().unwrap_or(0))
            }
        };

        Some(SlotParamChangedPayload {
            sequence: self.sequence,
            slot_index: slot_index as u32,
            slot_bus,
            param_index: param_selector as u32,
            value_type: sample.value_type_str().to_string(),
            value,
        })
    }

    pub fn clear(&mut self) {
        self.last_fingerprint.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        hex.split(|c: char| c == ':' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| u8::from_str_radix(s, 16).unwrap())
            .collect()
    }

    #[test]
    fn parses_slot0_param0_float_from_capture() {
        let hex = "2b:00:00:18:f0:03:02:10:00:f5:00:04:09:02:00:00:00:00:04:00:1b:00:00:00:82:69:1e:6a:84:52:00:44:06:79:14:6a:85:62:01:1d:c3:1a:00:1c:00:77:ca:3e:dc:28:f5:40";
        let buf = hex_to_bytes(hex);
        let samples = scan_slot_param_samples(&buf);
        assert_eq!(samples.len(), 1);
        let s = &samples[0];
        assert_eq!(s.slot_bus, 0x01);
        assert_eq!(s.param_selector, 0x00);
        assert_eq!(s.kind, SlotParamWireKind::Float);
        assert!((s.numeric_value - 0.43).abs() < 0.02);
    }

    #[test]
    fn emit_dedupes_identical_samples() {
        let hex = "85:62:01:1d:c3:1a:00:1c:00:77:ca:3f:80:00:00";
        let buf = hex_to_bytes(hex);
        let mut st = SlotParamEmitState::default();
        let a = st.ingest_buffer(&buf);
        assert_eq!(a.len(), 1);
        let b = st.ingest_buffer(&buf);
        assert!(b.is_empty());
    }

    #[test]
    fn parses_bool_and_discrete_tags() {
        let off = hex_to_bytes("85:62:02:1d:c3:1a:00:1c:01:77:c2");
        let on = hex_to_bytes("85:62:02:1d:c3:1a:00:1c:02:77:c3");
        let disc = hex_to_bytes("85:62:02:1d:c3:1a:00:1c:03:77:05");
        assert_eq!(scan_slot_param_samples(&off)[0].kind, SlotParamWireKind::BoolOff);
        assert_eq!(scan_slot_param_samples(&on)[0].kind, SlotParamWireKind::BoolOn);
        assert_eq!(scan_slot_param_samples(&disc)[0].kind, SlotParamWireKind::Discrete);
        assert_eq!(scan_slot_param_samples(&disc)[0].numeric_value, 5.0);
    }
}
