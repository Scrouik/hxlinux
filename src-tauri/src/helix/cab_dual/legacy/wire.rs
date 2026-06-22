//! Wire legacy Cab dual — patch cab2 (head `0x23`), dual parent (`0x25`), create (`0x2d`).

/// Replace cab2 sur slot dual legacy occupé.
pub const CAB2_REPLACE_HEAD_LEGACY: u8 = 0x23;

/// Replace dual parent (picker slot entier).
pub const DUAL_PARENT_REPLACE_HEAD: u8 = 0x25;

/// Assign / create dual sur slot vide.
pub const DUAL_CREATE_HEAD: u8 = 0x2d;

pub use crate::helix::cab_dual::ir::CAB2_REPLACE_HEAD as CAB2_REPLACE_HEAD_IR;

/// Têtes bulk pour le fire replace cab2 (IR + legacy 1 o / 3 o).
pub fn accepted_cab2_replace_heads() -> &'static [u8] {
    &[
        CAB2_REPLACE_HEAD_IR,
        CAB2_REPLACE_HEAD_LEGACY,
        DUAL_PARENT_REPLACE_HEAD,
    ]
}

pub fn is_legacy_dual_bulk_head(head: u8) -> bool {
    matches!(head, CAB2_REPLACE_HEAD_LEGACY | DUAL_PARENT_REPLACE_HEAD)
}

pub fn is_legacy_cab2_replace_bulk(bulk: &[u8]) -> bool {
    bulk.first() == Some(&CAB2_REPLACE_HEAD_LEGACY)
}

/// Slot occupé : reframe `cd:0a` → `cd:04` (identique IR).
pub fn reframe_cd0a_to_cd04(bulk: &mut [u8]) {
    for i in 0..bulk.len().saturating_sub(4) {
        if bulk[i] == 0x83 && bulk[i + 1] == 0x66 && bulk[i + 2] == 0xcd && bulk[i + 3] == 0x0a {
            bulk[i + 3] = 0x04;
            return;
        }
    }
}

/// Prépare un bulk replace cab2 avant envoi (reframe lane + cohérence transport).
pub fn prepare_cab2_replace_bulk(bulk: &mut [u8]) {
    reframe_cd0a_to_cd04(bulk);
}

/// Replace cab2 legacy : sélecteur **1 o** → template parent `0x23` (44 o) ;
/// hint **3 o** (`cd02xx`) → template dual pick `0x25` (48 o) avec swap cab1/cab2 (longueur fixe HX).
pub fn build_legacy_cab2_replace_bulk(
    parent_dual_bulk: &[u8],
    picked_cab_dual_bulk: &[u8],
    cab_field: &[u8],
) -> Result<Vec<u8>, String> {
    use crate::helix::edit_slot_model::{
        cab_dual_cab1_field_range_in_bulk, patch_cab_dual_bulk_cab_field,
    };

    if cab_field.len() == 1 {
        let mut bulk = parent_dual_bulk.to_vec();
        patch_cab_dual_bulk_cab_field(&mut bulk, 1, cab_field)?;
        prepare_cab2_replace_bulk(&mut bulk);
        if bulk.first() != Some(&CAB2_REPLACE_HEAD_LEGACY) || bulk.len() != 44 {
            return Err(format!(
                "legacy cab2 1 o : attendu head 0x23 / 44 o, reçu head={:#02x} len={}",
                bulk.first().copied().unwrap_or(0),
                bulk.len()
            ));
        }
        return Ok(bulk);
    }
    if cab_field.len() != 3 {
        return Err(format!(
            "legacy cab2 : hint {} octet(s) — attendu 1 ou 3",
            cab_field.len()
        ));
    }
    if picked_cab_dual_bulk.first() != Some(&DUAL_PARENT_REPLACE_HEAD) {
        return Err(
            "legacy cab2 3 o (cd02xx) : entrée dual variant head 0x25 requise dans HX_ModelUsbAssign"
                .into(),
        );
    }
    let (p1s, p1e) = cab_dual_cab1_field_range_in_bulk(parent_dual_bulk)
        .ok_or_else(|| "parent dual sans cab1 exploitable".to_string())?;
    let parent_cab1 = parent_dual_bulk[p1s..p1e].to_vec();

    let mut bulk = picked_cab_dual_bulk.to_vec();
    patch_cab_dual_bulk_cab_field(&mut bulk, 0, &parent_cab1)?;
    patch_cab_dual_bulk_cab_field(&mut bulk, 1, cab_field)?;
    prepare_cab2_replace_bulk(&mut bulk);
    if bulk.first() != Some(&DUAL_PARENT_REPLACE_HEAD) || bulk.len() != 48 {
        return Err(format!(
            "legacy cab2 3 o : attendu head 0x25 / 48 o, reçu head={:#02x} len={}",
            bulk.first().copied().unwrap_or(0),
            bulk.len()
        ));
    }
    Ok(bulk)
}

/// Décode `chainHexHint` catalogue en octets champ cab (1 o legacy, 2–3 o IR/hybrid long).
pub fn chain_hint_to_cab_field_bytes(chain_hex_hint: &str) -> Option<Vec<u8>> {
    let h = chain_hex_hint.trim();
    if h.is_empty() {
        return None;
    }
    if h.len() <= 2 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        return u8::from_str_radix(h, 16).ok().map(|b| vec![b]);
    }
    if !h.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(h.len() / 2);
    for i in (0..h.len()).step_by(2) {
        let byte = u8::from_str_radix(&h[i..i + 2], 16).ok()?;
        out.push(byte);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::edit_slot_model::build_cab_dual_replace_cab_bulk;

    #[test]
    fn chain_hint_one_byte_legacy() {
        assert_eq!(chain_hint_to_cab_field_bytes("33"), Some(vec![0x33]));
        assert_eq!(chain_hint_to_cab_field_bytes("2e"), Some(vec![0x2e]));
    }

    #[test]
    fn chain_hint_three_byte_ir() {
        assert_eq!(
            chain_hint_to_cab_field_bytes("cd031b"),
            Some(vec![0xcd, 0x03, 0x1b])
        );
    }

    #[test]
    fn legacy_cab2_replace_bulk_head_23() {
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_Cab1x6x9SoupProEllipse",
            "HD2_Cab1x12Celest12H",
            "dual",
            1,
        )
        .expect("legacy cab2 bulk");
        assert_eq!(bulk[0], CAB2_REPLACE_HEAD_LEGACY);
        assert_eq!(bulk.len(), 44);
        assert!(is_legacy_cab2_replace_bulk(&bulk));
    }

    #[test]
    fn legacy_cab2_patch_keeps_cab1_one_byte() {
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_Cab1x6x9SoupProEllipse",
            "HD2_Cab1x12Celest12H",
            "dual",
            1,
        )
        .expect("bulk");
        let body = bulk
            .windows(5)
            .any(|w| w == [0xc3, 0x19, 0x33, 0x1a, 0x2e]);
        assert!(body, "attendu c3 19 33 1a 2e (cab1 soup, cab2 celest)");
    }

    #[test]
    fn legacy_cab2_cd02xx_uses_head_25_48_bytes() {
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_Cab1x12Lead80",
            "HD2_Cab1x12PrincessBlue",
            "dual",
            1,
        )
        .expect("princess cab2 on lead80");
        assert_eq!(bulk[0], DUAL_PARENT_REPLACE_HEAD);
        assert_eq!(bulk.len(), 48);
        assert!(
            bulk.windows(8).any(|w| w == [0xc3, 0x19, 0x30, 0x1a, 0xcd, 0x02, 0x4e, 0x00]),
            "attendu c3 19 30 1a cd 02 4e"
        );
    }

    #[test]
    fn legacy_cab2_us_deluxe_stays_head_23() {
        let bulk = build_cab_dual_replace_cab_bulk(
            "HD2_Cab1x12Lead80",
            "HD2_Cab1x12USDeluxe",
            "dual",
            1,
        )
        .expect("deluxe");
        assert_eq!(bulk[0], CAB2_REPLACE_HEAD_LEGACY);
        assert_eq!(bulk.len(), 44);
        assert!(bulk.windows(5).any(|w| w == [0xc3, 0x19, 0x30, 0x1a, 0x31]));
    }

    #[test]
    fn reframe_cd0a_to_cd04_only_touches_lane_byte() {
        let mut bulk = vec![
            0x23, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x83, 0x66, 0xcd, 0x0a, 0x59,
        ];
        reframe_cd0a_to_cd04(&mut bulk);
        assert_eq!(&bulk[8..12], &[0x83, 0x66, 0xcd, 0x04]);
    }
}
