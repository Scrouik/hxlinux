//! États bloc par snapshot (`tone.snapshotN.blocks.dsp0`) depuis `preset_data`.
//!
//! Chaque enregistrement `SNAPSHOT N` est précédé d'une suite de triplets `92 c2 c3` (true)
//! ou `92 c2 c2` (false). Ordre wire observé (Helix Stomp) :
//! `split`, `block0`, `block1`, `block2`, `block4`, `block5`, `block3`.

const SNAPSHOT_COUNT: usize = 4;

fn triplet_bool(data: &[u8], at: usize) -> Option<bool> {
    if at + 2 >= data.len() {
        return None;
    }
    if data[at] != 0x92 {
        return None;
    }
    match (data[at + 1], data[at + 2]) {
        (0xc2, 0xc3) => Some(true),
        (0xc2, 0xc2) => Some(false),
        _ => None,
    }
}

/// Parse les triplets `92 c2 c2/c3` dans `chunk`.
fn parse_92_bool_triplets(chunk: &[u8]) -> Vec<bool> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 2 < chunk.len() {
        if let Some(v) = triplet_bool(chunk, i) {
            out.push(v);
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}

/// Cherche le début des triplets snapshot juste avant `SNAPSHOT N` (`00 14` puis `92…`).
fn snapshot_triplet_chunk_before_marker(data: &[u8], marker_at: usize) -> Option<&[u8]> {
    if marker_at == 0 {
        return None;
    }
    let search_from = marker_at.saturating_sub(120);
    let window = &data[search_from..marker_at];
    let rel = window.iter().rposition(|&b| b == 0x14)?;
    let abs = search_from + rel;
    if abs + 1 >= marker_at {
        return None;
    }
  // Sauter l'octet de longueur après `00 14` (souvent `92` directement).
    let chunk_start = if abs > 0 && data[abs - 1] == 0x00 {
        abs + 1
    } else {
        abs + 1
    };
    if chunk_start >= marker_at {
        return None;
    }
    Some(&data[chunk_start..marker_at])
}

fn snapshot_marker_pos(data: &[u8], snap_num: usize) -> Option<usize> {
    if snap_num == 0 || snap_num > SNAPSHOT_COUNT {
        return None;
    }
    let needle: Vec<u8> = format!("SNAPSHOT {snap_num}\0").into_bytes();
    data.windows(needle.len()).position(|w| w == needle)
}

/// Décode les 7 booléens (`split` + `block0…5`) depuis les triplets d'un snapshot.
fn decode_snapshot_dsp0_block_run(triplets: &[bool]) -> Option<Vec<bool>> {
    if triplets.len() < 8 {
        return None;
    }
    // Octet 0 = en-tête (souvent false) ; indices 1…7 = états.
    Some(vec![
        triplets[1],
        triplets[2],
        triplets[3],
        triplets[4],
        triplets[7],
        triplets[5],
        triplets[6],
    ])
}

/// Lit les états `split` + `block0…` pour chaque snapshot (jusqu'à 4).
/// `block_count` = nombre de blocs FX exportés (`block0`, `block1`, …).
pub fn try_parse_snapshot_dsp0_block_states(
    data: &[u8],
    block_count: usize,
) -> Option<Vec<Vec<bool>>> {
    if block_count == 0 || data.len() < 256 {
        return None;
    }
    let run_len = block_count + 1;
    let mut runs: Vec<Vec<bool>> = Vec::new();

    for snap_num in 1..=SNAPSHOT_COUNT {
        let marker_at = match snapshot_marker_pos(data, snap_num) {
            Some(p) => p,
            None if snap_num > 1 && !runs.is_empty() => {
                runs.push(runs.last()?.clone());
                continue;
            }
            None => return if runs.is_empty() { None } else { Some(runs) },
        };
        let chunk = snapshot_triplet_chunk_before_marker(data, marker_at)?;
        let triplets = parse_92_bool_triplets(chunk);
        let mut run = decode_snapshot_dsp0_block_run(&triplets)?;
        if run.len() > run_len {
            run.truncate(run_len);
        } else if run.len() < run_len {
            run.resize(run_len, true);
        }
        runs.push(run);
    }

    if runs.is_empty() {
        return None;
    }
    while runs.len() < SNAPSHOT_COUNT {
        runs.push(runs.last()?.clone());
    }
    Some(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_willis_fixture() -> Vec<u8> {
        let hex: String = std::fs::read_to_string("/tmp/willis_preset.hex")
            .expect("run python export to /tmp/willis_preset.hex first")
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect();
        let mut out = Vec::new();
        for i in (0..hex.len()).step_by(2) {
            let b = u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
            out.push(b);
        }
        out
    }

    #[test]
    fn willis_snapshot_block3_off_only_in_snapshot0() {
        let data = load_willis_fixture();
        let runs = try_parse_snapshot_dsp0_block_states(&data, 6).expect("parse");
        assert_eq!(runs.len(), 4);
        // split + 6 blocs : block3 = index 4.
        assert_eq!(runs[0][4], false, "snapshot0 block3");
        assert_eq!(runs[1][4], true, "snapshot1 block3");
        assert_eq!(runs[2][4], true, "snapshot2 block3");
        assert_eq!(runs[3][4], true, "snapshot3 block3 (fallback)");
    }
}
