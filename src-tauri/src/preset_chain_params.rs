//! Valeurs de paramètres lues dans le segment binaire d’un slot assignable (grille Kempline),
//! en reprenant la logique hex de `Kempline/utils/simple_filter.py` (`user_slot_reader` + `read_params`).

use serde::Serialize;
use std::fmt::Write as _;

const PAT_85188317: &str = "85188317";

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ChainParamValue {
    Bool(bool),
    Number(f64),
    UInt(u8),
    RawHex(String),
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

/// Décode la suite `read_params` sur une chaîne **hex** (deux caractères par octet), comme le Python.
fn read_params_hex(mut cur: &str, num_params: usize) -> Option<(Vec<ChainParamValue>, usize)> {
    let start = cur.len();
    let mut out = Vec::with_capacity(num_params);
    while out.len() < num_params {
        if cur.len() < 2 {
            return None;
        }
        if cur.starts_with("c2") {
            out.push(ChainParamValue::Bool(false));
            cur = &cur[2..];
        } else if cur.starts_with("c3") {
            out.push(ChainParamValue::Bool(true));
            cur = &cur[2..];
        } else if cur.starts_with("ca") {
            if cur.len() < 10 {
                return None;
            }
            let v = &cur[2..10];
            let raw = u32::from_str_radix(v, 16).ok()?;
            let f = f32::from_bits(raw) as f64;
            out.push(ChainParamValue::Number(f));
            cur = &cur[10..];
        } else if cur.len() >= 2 {
            let b = u8::from_str_radix(&cur[0..2], 16).ok()?;
            out.push(ChainParamValue::UInt(b));
            cur = &cur[2..];
        } else {
            return None;
        }
    }
    if cur.starts_with("1bda") && cur.len() >= 8 {
        let sz = usize::from_str_radix(&cur[4..6], 16).ok()? * 16
            + usize::from_str_radix(&cur[6..8], 16).ok()?;
        let need = 8 + sz * 2;
        if cur.len() < need {
            return None;
        }
        let blob = &cur[8..8 + sz * 2];
        out.push(ChainParamValue::RawHex(blob.to_string()));
        cur = &cur[need..];
    }
    let consumed = start - cur.len();
    Some((out, consumed))
}

/// `seg` = segment 8213… pour un slot `06` (sans le préfixe `8213` global ; le contenu commence souvent par `06`).
pub fn parse_standard_assignable_segment(seg: &[u8]) -> Option<Vec<ChainParamValue>> {
    if seg.first().copied() != Some(0x06) {
        return None;
    }
    let h = hex_lower(&seg[1..]);
    let slot_info_start = h.find(PAT_85188317)?;
    let mut br = slot_info_start + PAT_85188317.len();

    let tail = h.get(br..)?;
    if tail.starts_with("c319") {
        return None;
    }
    if !tail.starts_with("c219") {
        return None;
    }
    let c219_start = br;
    let slice = h.get(c219_start..)?;
    let rel09 = slice.find("09")?;
    if rel09 < 4 {
        return None;
    }
    let _type_hex = &slice[4..rel09];
    // Comme Python `user_slot_reader` : après le type, `bytes_read` est sur le **premier** caractère
    // du délimiteur `09` ; le premier `bytes_read += 4` saute `09` + 2 hex suivants (pas `09` puis +4 séparément).
    br = c219_start + rel09;

    br += 4;
    br += 4;
    br += 6;
    if br + 2 > h.len() {
        return None;
    }
    let num_params = usize::from_str_radix(&h[br..br + 2], 16).ok()?;
    br += 2;
    br += 8;
    let (params, _consumed) = read_params_hex(h.get(br..)?, num_params)?;
    Some(params)
}
