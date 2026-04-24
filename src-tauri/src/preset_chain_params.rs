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

fn parse_c219_block_at(h: &str, c219_start: usize) -> Option<(Vec<ChainParamValue>, usize)> {
    let slice = h.get(c219_start..)?;
    let rel09 = slice.find("09")?;
    if rel09 < 4 {
        return None;
    }
    let _type_hex = &slice[4..rel09];
    let mut br = c219_start + rel09;
    // Comme Python `user_slot_reader` : après le type, `bytes_read` est sur le **premier** caractère
    // du délimiteur `09` ; le premier `bytes_read += 4` saute `09` + 2 hex suivants.
    br += 4;
    br += 4;
    br += 6;
    if br + 2 > h.len() {
        return None;
    }
    let num_hex = h.get(br..br + 2)?;
    let num_params = usize::from_str_radix(num_hex, 16).ok()?;
    br += 2;
    br += 8;
    // Certains blocs (ex. ampli basse SVT-4 Pro, `chainHex` cd0207) insèrent après l’entête
    // standard quatre caractères hex `00` + répétition du compteur (`00` + même byte que
    // `num_params`) avant la séquence `read_params`. Sans ce recul, le premier token est
    // lu comme deux `UInt` au lieu du premier `ca` (knob).
    if let Some(pad) = h.get(br..br + 4) {
        if pad.len() == 4 && pad.get(..2) == Some("00") && pad.get(2..4) == Some(num_hex) {
            br += 4;
        }
    }
    let (params, consumed) = read_params_hex(h.get(br..)?, num_params)?;
    Some((params, br + consumed))
}

fn parse_info_slot_block_value_bytes(value: &[u8]) -> Option<Vec<ChainParamValue>> {
    if value.len() < 8 {
        return None;
    }
    let num_params = value[2] as usize;
    if num_params == 0 {
        return Some(Vec::new());
    }
    // Comme Kempline `next_gen_slot_parser`: `num_params = value[2]`, puis `read_params(value[7:])`.
    let mut raw_params = value.get(7..)?;
    // Même anomalie que certains blocs `c219` : préfixe `00` + répétition du compteur
    // (`num_params`) avant la séquence `read_params`.
    if raw_params.len() >= 2 && raw_params[0] == 0x00 && raw_params[1] == value[2] {
        raw_params = &raw_params[2..];
    }
    let hex = hex_lower(raw_params);
    let (params, _) = read_params_hex(&hex, num_params)?;
    Some(params)
}

/// Segments I/O de flux (`slot_type` Kempline 00/01/02/03) :
/// les paramètres sont portés par un marqueur `0x07` puis un bloc « info » (`num_params = value[2]`, `read_params(value[7:])`).
///
/// Plusieurs `0x07` peuvent apparaître (surtout **Split** `0x02`) : le premier bloc est parfois un
/// en-tête avec `num_params == 0` → liste vide. On essaie donc **tous** les `0x07` et on retient le
/// décodage **non vide** le plus long (en cas d’égalité, le **dernier** gagne — bloc paramètres en fin de segment).
pub fn parse_flow_io_segment_params(seg: &[u8]) -> Option<Vec<ChainParamValue>> {
    if !matches!(seg.first().copied(), Some(0x00 | 0x01 | 0x02 | 0x03)) {
        return None;
    }
    let mut best: Option<Vec<ChainParamValue>> = None;
    let mut best_len = 0usize;
    for (abs, _) in seg.iter().enumerate().filter(|(_, &b)| b == 0x07) {
        let Some(value) = seg.get(abs + 1..) else {
            continue;
        };
        let Some(v) = parse_info_slot_block_value_bytes(value) else {
            continue;
        };
        if v.is_empty() {
            continue;
        }
        if v.len() >= best_len {
            best_len = v.len();
            best = Some(v);
        }
    }
    best
}

/// Variante Amp/Preamp "dual-slot" observée sur certains segments:
/// `... 0b <info_slot_a> 0c 83 <info_slot_b ...>` sans blocs `c219`.
/// Kempline lit les params depuis `info_slot_a` et `info_slot_b` via `value[2]` + `value[7:]`.
fn parse_dual_slot_info_param_blocks(seg: &[u8]) -> Option<Vec<Vec<ChainParamValue>>> {
    let body = seg.get(1..)?;
    let start = body.windows(4).position(|w| w == [0x85, 0x18, 0x83, 0x17])?;
    let tail = body.get(start + 4..)?;
    let pos_0b = tail.iter().position(|&b| b == 0x0b)?;
    let info_a_start = pos_0b + 1;
    let mut out: Vec<Vec<ChainParamValue>> = Vec::new();

    // Séparateur attendu entre A/B: `0c 83` (vu dans le dump utilisateur et côté Kempline).
    let pos_0c83_rel = tail
        .get(info_a_start..)?
        .windows(2)
        .position(|w| w == [0x0c, 0x83]);
    if let Some(rel) = pos_0c83_rel {
        let pos_0c83 = info_a_start + rel;
        let info_a = tail.get(info_a_start..pos_0c83).unwrap_or(&[]);
        if let Some(params_a) = parse_info_slot_block_value_bytes(info_a) {
            out.push(params_a);
        }
        // Kempline consomme le marqueur `0x0c`, donc `info_slot_b` commence sur `0x83`.
        let info_b = tail.get(pos_0c83 + 1..).unwrap_or(&[]);
        if let Some(params_b) = parse_info_slot_block_value_bytes(info_b) {
            out.push(params_b);
        }
    } else {
        // Fallback: un seul bloc info après `0x0b`.
        let info = tail.get(info_a_start..).unwrap_or(&[]);
        if let Some(params) = parse_info_slot_block_value_bytes(info) {
            out.push(params);
        }
    }

    if out.is_empty() { None } else { Some(out) }
}

/// Extrait un ou plusieurs blocs `read_params` d’un segment assignable (cas standard et Amp+Cab).
pub fn parse_assignable_segment_param_blocks(seg: &[u8]) -> Option<Vec<Vec<ChainParamValue>>> {
    // Même convention que `try_parse_preset_kempline_grid` : en-tête de segment `06` ou `08`.
    if !matches!(seg.first().copied(), Some(0x06 | 0x08)) {
        return None;
    }
    let h = hex_lower(&seg[1..]);
    let slot_info_start = h.find(PAT_85188317)?;
    let br = slot_info_start + PAT_85188317.len();
    let tail = h.get(br..)?;
    let mut out: Vec<Vec<ChainParamValue>> = Vec::new();

    // Cas standard : un ou plusieurs blocs `c219` consécutifs (sans préfixe `c319`).
    // Un seul bloc ne suffit pas pour certains amplis « riches » (ex. basse + EQ intégré,
    // `chainHex` cd0207 / HD2_AmpSVT4Pro) : le firmware enchaîne plusieurs séquences `read_params`.
    if tail.starts_with("c219") {
        let mut search = br;
        while search < h.len() {
            let Some(rel) = h.get(search..)?.find("c219") else {
                break;
            };
            let c219_start = search + rel;
            let Some((params, next)) = parse_c219_block_at(&h, c219_start) else {
                break;
            };
            if next <= c219_start {
                break;
            }
            out.push(params);
            search = next;
        }
        return if out.is_empty() { None } else { Some(out) };
    }

    // Cas Amp+Cab (`c319`) : plusieurs blocs `c219` peuvent suivre.
    if tail.starts_with("c319") {
        let mut search = br + 4;
        while search < h.len() {
            let Some(rel) = h.get(search..)?.find("c219") else {
                break;
            };
            let c219_start = search + rel;
            let Some((params, next)) = parse_c219_block_at(&h, c219_start) else {
                break;
            };
            out.push(params);
            if next <= c219_start {
                break;
            }
            search = next;
        }
        if !out.is_empty() {
            return Some(out);
        }
        // Variante Amp/Preamp sans `c219`: décoder via blocs info `0b/0c`.
        if let Some(v) = parse_dual_slot_info_param_blocks(seg) {
            return Some(v);
        }
        return None;
    }

    // Tolérance : rechercher un `c219` plus loin si la tête contient des métadonnées inattendues.
    if let Some(rel) = tail.find("c219") {
        let (params, _) = parse_c219_block_at(&h, br + rel)?;
        out.push(params);
        return Some(out);
    }
    parse_dual_slot_info_param_blocks(seg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assignable_seg_from_ascii_hex(hex_lower: &str) -> Vec<u8> {
        let mut out = vec![0x06u8];
        let h = hex_lower.as_bytes();
        for i in (0..h.len()).step_by(2) {
            let b = u8::from_str_radix(std::str::from_utf8(&h[i..i + 2]).unwrap(), 16).unwrap();
            out.push(b);
        }
        out
    }

    /// En-tête `85188317` / `c219` identique à la capture SVT-4 Pro (`cd0207`) : après `num_params`,
    /// quatre hex `00` + répétition du compteur précèdent les `ca…`.
    #[test]
    fn c219_skips_zero_dup_count_before_read_params() {
        let prefix = "85188317c219cd02071aff09110ac30b830215031504dc0015";
        let one_float = "ca3f800000";
        let params_hex: String = std::iter::repeat(one_float).take(21).collect();
        let seg = assignable_seg_from_ascii_hex(&format!("{prefix}{params_hex}"));
        let blocks = parse_assignable_segment_param_blocks(&seg).expect("parse");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].len(), 21);
        for v in &blocks[0] {
            match v {
                ChainParamValue::Number(x) => assert!((x - 1.0f64).abs() < 1e-6),
                _ => panic!("expected float"),
            }
        }
    }

    #[test]
    fn c319_dual_slot_info_blocks_without_c219_are_decoded() {
        let seg = assignable_seg_from_ascii_hex(
            "1485188317c319061acd02cf09210ac30b83020c030c049cca3ee66666ca3f266666ca3f000000ca3f000000ca3f333333ca3f800000ca3f800000ca3f000000ca3f000000ca3f000000ca3f28f5c3ca3f0000000c830207030704970aca3f400000ca41100000ca42340000ca419f3333ca469d0800ca00000000",
        );
        let blocks = parse_assignable_segment_param_blocks(&seg).expect("parse");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].len(), 12);
        assert_eq!(blocks[1].len(), 7);
    }

    #[test]
    fn c319_dual_slot_amp_eq_skips_zero_dup_count_prefix() {
        let seg = assignable_seg_from_ascii_hex(
            "1485188317c319cd02071acd02f009210ac30b830215031504dc0015ca3f000000ca3f000000ca3f00000001ca3f000000ca3f800000ca3f400000c2c2c2c3ca00000000ca00000000ca00000000ca00000000ca00000000ca00000000ca00000000ca00000000ca00000000ca000000000c8302070307049706ca3e75c28fca3f800000ca42340000ca419f3333ca4657a000ca00000000",
        );
        let blocks = parse_assignable_segment_param_blocks(&seg).expect("parse");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].len(), 21);
        assert_eq!(blocks[1].len(), 7);
        match blocks[0][0] {
            ChainParamValue::Number(v) => assert!((v - 0.5f64).abs() < 1e-6),
            _ => panic!("expected first amp param float"),
        }
        match blocks[0][1] {
            ChainParamValue::Number(v) => assert!((v - 0.5f64).abs() < 1e-6),
            _ => panic!("expected second amp param float"),
        }
    }

    #[test]
    fn io_flow_segment_00_decodes_params_from_marker_07_until_end() {
        // Segment type 00 (Input upper), params blob après marqueur 0x07.
        let seg = vec![
            0x00, 0x13, 0x00, 0x14, 0x82, 0x05, 0x01, 0x07, 0x83, 0x02, 0x02, 0x03, 0x04, 0x97,
            0x00, 0xca, 0x3f, 0x40, 0x00, 0x00, 0xca, 0x3f, 0x80, 0x00, 0x00,
        ];
        let vals = parse_flow_io_segment_params(&seg).expect("parse io");
        assert_eq!(vals.len(), 2);
        match vals[0] {
            ChainParamValue::Number(v) => assert!((v - 0.75f64).abs() < 1e-6),
            _ => panic!("expected float"),
        }
        match vals[1] {
            ChainParamValue::Number(v) => assert!((v - 1.0f64).abs() < 1e-6),
            _ => panic!("expected float"),
        }
    }

    #[test]
    fn io_flow_segment_02_skips_empty_first_07_marker() {
        // Split `0x02` : un premier `0x07` peut annoncer `num_params == 0` ; les vrais params suivent.
        let seg = vec![
            0x02, 0x07, 0x83, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x83, 0x02, 0x02, 0x03,
            0x04, 0x97, 0x00, 0xca, 0x3f, 0x40, 0x00, 0x00, 0xca, 0x3f, 0x80, 0x00, 0x00,
        ];
        let vals = parse_flow_io_segment_params(&seg).expect("parse split flow");
        assert_eq!(vals.len(), 2);
    }
}
