//! Layout Stomp XL : 8 colonnes + positions Split / Merge pour la grille Kempline 16 cases.
//! - **Occupation** : même règle que `computeRoutingJunctionColumns` côté TS (référence stable).
//! - **grid_x** (optionnel) : si toutes les cases occupées ont un `grid_x` et la rangée B n’est pas vide,
//!   on affine surtout le **merge** (dernière colonne du ou des blocs les plus « tard » dans l’ordre `grid_x`)
//!   et on peut décaler le **split** vers la droite quand la chaîne A reste entièrement avant `min(grid_x)` sur B.

use serde::Serialize;

/// Cellule de la grille Kempline (16 segments : 8 path 1A puis 8 path 1B).
#[derive(Clone, Debug)]
pub struct KemplineCell {
    pub category: String,
    pub name: String,
    pub grid_x: Option<u8>,
    pub grid_y: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePresetRoutingLayout {
    pub split_after_col: u8,
    pub merge_after_col: u8,
    pub inferred_from: String,
    /// `true` si le parseur a fourni exactement 16 segments grille ; `false` si données partielles.
    pub kempline_grid_ok: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StompChainEntry {
    pub index: u8,
    pub kind: String,
    pub top_category: String,
    pub top_name: String,
    pub bottom_category: String,
    pub bottom_name: String,
    pub top_grid_x: Option<u8>,
    pub top_grid_y: Option<u8>,
    pub bottom_grid_x: Option<u8>,
    pub bottom_grid_y: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePresetStompLayout {
    pub routing: ActivePresetRoutingLayout,
    pub chain: Vec<StompChainEntry>,
}

fn is_kempline_empty(cell: &KemplineCell) -> bool {
    cell.category.is_empty() && cell.name == "<empty>"
}

fn leading_empty_row(grid: &[KemplineCell], row_start: usize) -> u8 {
    let mut n: u8 = 0;
    for i in 0..8 {
        if is_kempline_empty(&grid[row_start + i]) {
            n = n.saturating_add(1);
        } else {
            break;
        }
    }
    n
}

fn last_filled_row_index(grid: &[KemplineCell], row_start: usize) -> i8 {
    for i in (0..8).rev() {
        if !is_kempline_empty(&grid[row_start + i]) {
            return i as i8;
        }
    }
    -1
}

/// Heuristique d’occupation (équivalent à `computeRoutingJunctionColumns` dans `src/models.ts`).
pub fn split_merge_from_occupancy(grid: &[KemplineCell]) -> (u8, u8) {
    let lead_a = leading_empty_row(grid, 0);
    let lead_b = leading_empty_row(grid, 8);
    let last_b = last_filled_row_index(grid, 8);
    let has_any_b = last_b >= 0;

    let mut split_col = if has_any_b {
        lead_a.max(lead_b)
    } else {
        lead_a
    };
    split_col = split_col.min(8);

    let last_a = last_filled_row_index(grid, 0);
    let last_used = last_a.max(last_b);
    let mut merge_col = if last_used < 0 {
        8u8
    } else {
        ((last_used as u8) + 1).min(8)
    };
    if merge_col <= split_col {
        merge_col = (split_col + 1).min(8);
    }
    (split_col, merge_col)
}

fn all_nonempty_have_grid_x(grid: &[KemplineCell]) -> bool {
    if grid.len() != 16 {
        return false;
    }
    grid.iter().all(|c| is_kempline_empty(c) || c.grid_x.is_some())
}

fn min_grid_x_on_row_b(grid: &[KemplineCell]) -> Option<u8> {
    let mut m: Option<u8> = None;
    for i in 0..8 {
        let c = &grid[8 + i];
        if is_kempline_empty(c) {
            continue;
        }
        let gx = c.grid_x?;
        m = Some(m.map_or(gx, |v| v.min(gx)));
    }
    m
}

/// Raffine split/merge quand chaque case occupée a un `grid_x` et qu’il existe au moins un bloc sur B.
fn split_merge_from_grid_x(grid: &[KemplineCell], occ: (u8, u8)) -> Option<(u8, u8)> {
    if grid.len() != 16 {
        return None;
    }
    if !all_nonempty_have_grid_x(grid) {
        return None;
    }
    let min_b = min_grid_x_on_row_b(grid)?;

    let mut max_a_col_strictly_before_b: Option<u8> = None;
    for j in 0..8 {
        let a = &grid[j];
        if is_kempline_empty(a) {
            continue;
        }
        let gx = a.grid_x?;
        if gx < min_b {
            let col = j as u8;
            max_a_col_strictly_before_b = Some(max_a_col_strictly_before_b.map_or(col, |m| m.max(col)));
        }
    }
    let split_gx = max_a_col_strictly_before_b
        .map(|m| m.saturating_add(1).min(8))
        .unwrap_or(0);

    let mut global_max: Option<u8> = None;
    let mut merge_anchor_col: u8 = 0;
    for idx in 0..16 {
        let c = &grid[idx];
        if is_kempline_empty(c) {
            continue;
        }
        let gx = c.grid_x?;
        let col = if idx < 8 { idx as u8 } else { (idx - 8) as u8 };
        match global_max {
            None => {
                global_max = Some(gx);
                merge_anchor_col = col;
            }
            Some(m) if gx > m => {
                global_max = Some(gx);
                merge_anchor_col = col;
            }
            Some(m) if gx == m && col > merge_anchor_col => {
                merge_anchor_col = col;
            }
            _ => {}
        }
    }
    let _ = global_max?;
    let merge_gx = merge_anchor_col.saturating_add(1).min(8);

    let split = occ.0.max(split_gx).min(8);
    let mut merge = occ.1.max(merge_gx).min(8);
    if merge <= split {
        merge = (split + 1).min(8);
    }
    Some((split, merge))
}

/// Lit les colonnes split/merge depuis le corps preset USB.
///
/// Corps preset tel qu’accumulé par `RequestPreset` (octets **après** les 16 premiers
/// de chaque chunk ED03).
///
/// Sur captures usbmon (Stomp), quand il y a exactement deux motifs `0x0d XX 0x0a`,
/// le 1er `XX` encode le split et le 2e encode le merge, avec la convention `col = XX - 1`.
pub fn split_merge_from_usb_preset_body(data: &[u8]) -> Option<(u8, u8)> {
    let mut mids: Vec<u8> = Vec::new();
    for i in 0..data.len().saturating_sub(2) {
        if data[i] == 0x0d && data[i + 2] == 0x0a {
            mids.push(data[i + 1]);
        }
    }
    if mids.len() != 2 {
        return None;
    }
    let split_mid = mids[0];
    let merge_mid = mids[1];
    if split_mid == 0 || merge_mid == 0 {
        return None;
    }
    let split = split_mid.saturating_sub(1).min(8);
    let merge = merge_mid.saturating_sub(1).min(8);
    if merge <= split {
        return None;
    }
    Some((split, merge))
}

/// Compat helper: ne retourne que `merge_after_col`.
pub fn merge_after_col_from_usb_preset_body(data: &[u8]) -> Option<u8> {
    split_merge_from_usb_preset_body(data).map(|(_, merge)| merge)
}

fn build_chain(grid: &[KemplineCell]) -> Vec<StompChainEntry> {
    let mut out = Vec::with_capacity(10);
    for col in 0u8..8u8 {
        let top = &grid[col as usize];
        let bot = &grid[8 + col as usize];
        let kind = if is_kempline_empty(top) && is_kempline_empty(bot) {
            "empty"
        } else {
            "model"
        };
        out.push(StompChainEntry {
            index: col,
            kind: kind.to_string(),
            top_category: top.category.clone(),
            top_name: top.name.clone(),
            bottom_category: bot.category.clone(),
            bottom_name: bot.name.clone(),
            top_grid_x: top.grid_x,
            top_grid_y: top.grid_y,
            bottom_grid_x: bot.grid_x,
            bottom_grid_y: bot.grid_y,
        });
    }
    out.push(StompChainEntry {
        index: 8,
        kind: "split".to_string(),
        top_category: String::new(),
        top_name: String::new(),
        bottom_category: String::new(),
        bottom_name: String::new(),
        top_grid_x: None,
        top_grid_y: None,
        bottom_grid_x: None,
        bottom_grid_y: None,
    });
    out.push(StompChainEntry {
        index: 9,
        kind: "merge".to_string(),
        top_category: String::new(),
        top_name: String::new(),
        bottom_category: String::new(),
        bottom_name: String::new(),
        top_grid_x: None,
        top_grid_y: None,
        bottom_grid_x: None,
        bottom_grid_y: None,
    });
    out
}

/// Construit le layout à partir de 16 cellules grille (ordre Kempline : 8×1A puis 8×1B).
pub fn compute_stomp_layout_from_kempline_grid(grid: &[KemplineCell]) -> ActivePresetStompLayout {
    let grid_ok = grid.len() == 16;
    let grid_slice = if grid_ok {
        grid
    } else {
        &[]
    };

    let occ = if grid_ok {
        split_merge_from_occupancy(grid_slice)
    } else {
        (0u8, 8u8)
    };

    let (split_c, merge_c, inferred) = if grid_ok {
        if let Some((s, m)) = split_merge_from_grid_x(grid_slice, occ) {
            (s, m, "kempline_grid_x")
        } else {
            (occ.0, occ.1, "kempline_leading_slots")
        }
    } else {
        (0u8, 8u8, "fallback_empty_grid")
    };

    let routing = ActivePresetRoutingLayout {
        split_after_col: split_c,
        merge_after_col: merge_c,
        inferred_from: inferred.to_string(),
        kempline_grid_ok: grid_ok,
    };

    let chain = if grid_ok {
        build_chain(grid_slice)
    } else {
        Vec::new()
    };
    ActivePresetStompLayout { routing, chain }
}

/// Comme [`compute_stomp_layout_from_kempline_grid`], puis tente d’affiner split/merge depuis le flux USB
/// (`split_merge_from_usb_preset_body`) lorsque le motif est reconnu.
pub fn compute_stomp_layout_from_kempline_grid_with_usb(
    grid: &[KemplineCell],
    raw_preset: &[u8],
) -> ActivePresetStompLayout {
    let mut layout = compute_stomp_layout_from_kempline_grid(grid);
    if let Some((usb_split, usb_merge)) = split_merge_from_usb_preset_body(raw_preset) {
        layout.routing.split_after_col = usb_split;
        layout.routing.merge_after_col = usb_merge;
        let base = layout.routing.inferred_from.clone();
        layout.routing.inferred_from = format!("{base};usb_split_merge_od");
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(cat: &str, name: &str, gx: Option<u8>, gy: Option<u8>) -> KemplineCell {
        KemplineCell {
            category: cat.to_string(),
            name: name.to_string(),
            grid_x: gx,
            grid_y: gy,
        }
    }

    fn empty() -> KemplineCell {
        cell("", "<empty>", None, None)
    }

    #[test]
    fn occupancy_matches_two_leading_a_and_b() {
        let mut g = (0..16).map(|_| empty()).collect::<Vec<_>>();
        g[0] = empty();
        g[1] = empty();
        g[2] = cell("Amp", "A", Some(12), Some(12));
        g[8] = cell("Amp", "B", Some(12), Some(12));
        let (s, m) = split_merge_from_occupancy(&g);
        assert_eq!(s, 2);
        assert!(m > s);
        let layout = compute_stomp_layout_from_kempline_grid(&g);
        assert_eq!(layout.routing.split_after_col, 2);
        assert_eq!(layout.chain.len(), 10);
        assert_eq!(layout.chain[8].kind, "split");
        assert_eq!(layout.chain[9].kind, "merge");
    }

    #[test]
    fn grid_x_path_sets_inferred_from_kempline_grid_x() {
        let mut g = (0..16).map(|_| empty()).collect::<Vec<_>>();
        g[2] = cell("Amp", "A", Some(10), Some(10));
        g[8] = cell("Amp", "B", Some(20), Some(20));
        let layout = compute_stomp_layout_from_kempline_grid(&g);
        assert_eq!(layout.routing.inferred_from, "kempline_grid_x");
    }

    #[test]
    fn grid_x_merge_can_extend_past_last_occupied_column() {
        let mut g = (0..16).map(|_| empty()).collect::<Vec<_>>();
        g[0] = cell("Amp", "Early", Some(5), Some(5));
        g[7] = cell("Delay", "Late", Some(40), Some(40));
        g[8] = cell("Amp", "B", Some(10), Some(10));
        let occ = split_merge_from_occupancy(&g);
        let gx = split_merge_from_grid_x(&g, occ).expect("grid_x path");
        assert!(
            gx.1 >= occ.1,
            "merge from max gx should be at least occupancy merge"
        );
    }

    #[test]
    fn non_sixteen_grid_yields_fallback() {
        let g = vec![empty(); 8];
        let layout = compute_stomp_layout_from_kempline_grid(&g);
        assert!(!layout.routing.kempline_grid_ok);
        assert_eq!(layout.chain.len(), 0);
    }

    #[test]
    fn usb_od_triples_map_split_and_merge() {
        let mut v = vec![0u8; 500];
        v[100] = 0x0d;
        v[101] = 0x01;
        v[102] = 0x0a;
        v[200] = 0x0d;
        v[201] = 0x05;
        v[202] = 0x0a;
        assert_eq!(super::split_merge_from_usb_preset_body(&v), Some((0, 4)));
        assert_eq!(super::merge_after_col_from_usb_preset_body(&v), Some(4));
    }

    #[test]
    fn usb_od_triple_rejects_not_two_hits() {
        let v = vec![0x0du8, 3, 0x0a];
        assert!(super::merge_after_col_from_usb_preset_body(&v).is_none());
    }

    #[test]
    fn usb_od_triple_rejects_second_zero() {
        let mut v = vec![0u8; 50];
        v[0] = 0x0d;
        v[1] = 0x01;
        v[2] = 0x0a;
        v[10] = 0x0d;
        v[11] = 0x00;
        v[12] = 0x0a;
        assert!(super::merge_after_col_from_usb_preset_body(&v).is_none());
    }

    #[test]
    fn usb_od_triple_rejects_merge_not_after_split() {
        let mut v = vec![0u8; 50];
        v[0] = 0x0d;
        v[1] = 0x04;
        v[2] = 0x0a;
        v[10] = 0x0d;
        v[11] = 0x03;
        v[12] = 0x0a;
        assert!(super::split_merge_from_usb_preset_body(&v).is_none());
    }

    #[test]
    fn with_usb_overrides_merge_when_hint_valid() {
        let mut g = (0..16).map(|_| empty()).collect::<Vec<_>>();
        g[0] = empty();
        g[1] = empty();
        g[2] = cell("Amp", "A", Some(12), Some(12));
        g[8] = cell("Amp", "B", Some(12), Some(12));
        let mut raw = vec![0u8; 500];
        raw[100] = 0x0d;
        raw[101] = 0x01;
        raw[102] = 0x0a;
        raw[200] = 0x0d;
        raw[201] = 0x08;
        raw[202] = 0x0a;
        let layout = super::compute_stomp_layout_from_kempline_grid_with_usb(&g, &raw);
        assert_eq!(layout.routing.split_after_col, 0);
        assert_eq!(layout.routing.merge_after_col, 7);
        assert!(layout.routing.inferred_from.contains("usb_split_merge_od"));
    }
}
