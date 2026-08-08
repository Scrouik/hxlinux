//! Rename / save preset sur le HX (captures HX Edit).
//!
//! Lane `ed:03` (`80:10`), compteur lane = `live_write_ctr`.
//! - **Rename** : `cd:03`, sous-commande `02`, suffixe `06:65`
//! - **Save** : `cd:04`, sous-commande `06`, suffixe `47:65`

use crate::helix::packet::OutPacket;
use crate::helix::HelixState;

const LABEL_LANE_LO_DELTA: u16 = 0x11;

struct PresetLabelWire {
    cd: u8,
    cmd: u8,
    suffix: [u8; 2],
}

const RENAME_WIRE: PresetLabelWire = PresetLabelWire {
    cd: 0x03,
    cmd: 0x02,
    suffix: [0x06, 0x65],
};

// `cd=0x03` = compteur de SESSION ed:03 (PAS 0x04). Confirmé capture HX Edit `rename_snap_and_save`
// (f5389 SAVE : `83 66 cd 03 fc 64 47 65 …`, double fc monotone après le rename fb, MÊME session
// cd=0x03 que toutes les ops). Bug 2026-08-07 : on hardcodait cd=0x04 → device rejette le save →
// lane ed:03 assommée → lectures suivantes bloquées (0 o). MÊME famille que le bug rename snapshot.
const SAVE_WIRE: PresetLabelWire = PresetLabelWire {
    cd: 0x03,
    cmd: 0x06,
    suffix: [0x47, 0x65],
};

/// Nom preset ASCII (16 caractères max), comme l’UI.
pub fn preset_label_ascii_bytes(name: &str) -> Vec<u8> {
    name.chars()
        .filter(|c| c.is_ascii())
        .take(16)
        .map(|c| c as u8)
        .collect()
}

/// Octet après le tag `6d` : `0xa1 + len(nom)` (capture HX Edit).
fn preset_label_length_tag(name_len: usize) -> u8 {
    0xa1u8.wrapping_add(name_len as u8)
}

fn build_preset_label_packet(
    preset_index: u8,
    text: &[u8],
    lane_lo: u8,
    lane_hi: u8,
    cnt: u8,
    double: [u8; 2],
    wire: PresetLabelWire,
) -> Vec<u8> {
    let msg_size_byte = 0x20u8.wrapping_add(text.len() as u8);
    let second_length_byte = msg_size_byte.wrapping_sub(0x10);
    let length_tag = preset_label_length_tag(text.len());

    let mut data = vec![
        msg_size_byte,
        0x00,
        0x00,
        0x18,
        0x80,
        0x10,
        0xed,
        0x03,
        0x00,
        cnt,
        0x00,
        0x04,
        lane_lo,
        lane_hi,
        0x00,
        0x00,
        0x01,
        0x00,
        wire.cmd,
        0x00,
        second_length_byte,
        0x00,
        0x00,
        0x00,
        0x83,
        0x66,
        0xcd,
        wire.cd,
        double[0],
        double[1],
        wire.suffix[0],
        wire.suffix[1],
        0x83,
        0x6b,
        0x00,
        0x6c,
        preset_index,
        0x6d,
        length_tag,
    ];
    data.extend_from_slice(text);
    while data.len() < (msg_size_byte as usize) + 10 {
        data.push(0x00);
    }
    data
}

fn send_preset_label(
    state: &mut HelixState,
    preset_index: usize,
    name: &str,
    wire: PresetLabelWire,
    disabled_msg: &str,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        return Err(disabled_msg.to_string());
    }
    if preset_index > 0xff {
        return Err(format!("index preset invalide: {preset_index}"));
    }
    let text = preset_label_ascii_bytes(name);
    if text.is_empty() {
        return Err("nom preset vide".to_string());
    }

    let ctr = state.live_write_ctr;
    let lane_lo = (ctr & 0xff) as u8;
    let lane_hi = ((ctr >> 8) & 0xff) as u8;
    let cnt = state.next_x80_cnt();
    let double = state.next_editor_ed03_double();

    let data = build_preset_label_packet(
        preset_index as u8,
        &text,
        lane_lo,
        lane_hi,
        cnt,
        double,
        wire,
    );
    // DIAG chantier C : octets du paquet label (save ET rename preset) — cd/lane/double.
    eprintln!(
        "[PresetLabel][sent] term={:#04x} cd={:#04x} lane(b12_13)={:02x}:{:02x} double(b28_29)={:02x}:{:02x} cnt={:02x}",
        data.get(30).copied().unwrap_or(0), data.get(27).copied().unwrap_or(0),
        lane_lo, lane_hi, double[0], double[1], cnt
    );
    state.send(OutPacket::new(data));
    state.live_write_ctr = state.live_write_ctr.wrapping_add(LABEL_LANE_LO_DELTA);
    Ok(())
}

/// Témoin `HX_PRESET_RENAME_HW` (défaut ON) : rename preset sur le HX. `=0` désactive l’envoi.
pub fn preset_rename_hw_enabled() -> bool {
    match std::env::var("HX_PRESET_RENAME_HW").as_deref() {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
}

/// Témoin `HX_PRESET_SAVE_HW` (défaut ON) : sauvegarde preset sur le HX. `=0` désactive l’envoi.
pub fn preset_save_hw_enabled() -> bool {
    match std::env::var("HX_PRESET_SAVE_HW").as_deref() {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
}

/// Envoie le rename preset (`cd:03`, cmd `02`) sur le HX.
pub fn send_preset_rename(state: &mut HelixState, preset_index: usize, name: &str) -> Result<(), String> {
    send_preset_label(
        state,
        preset_index,
        name,
        RENAME_WIRE,
        "rename preset HX désactivé (HX_PRESET_RENAME_HW=0)",
        preset_rename_hw_enabled(),
    )
}

/// Envoie la sauvegarde preset sur le HX (`cd:03`, cmd `06`, suffixe `47:65`).
///
/// ⚠️ LIMITE CONNUE (2026-08-07, chantier « unification compteurs ed:03 ») : après un renommage de
/// snapshot, le SAVE désynchronise la lane ed:03 → les lectures de preset suivantes sont bloquées
/// (device muet, 0 o). Racine : HX Edit utilise UN compteur monotone unique (double+lane) pour TOUTES
/// les ops ed:03 (rename→save→lectures se suivent : double f4→f5→f6…, lane +0x11) ; notre archi a des
/// compteurs SÉPARÉS pour writes (`live_write_ctr`/`live_write_yy`) et lectures (`editor_ed03_double` +
/// lane dump). La lecture post-save doit CONTINUER le compteur du save, ce que notre lecture (compteur
/// séparé) ne fait pas. Fix = unifier tous les compteurs ed:03 (refactor dédié, cf mémoire). Le `cd=0x03`
/// est correct (confirmé `rename_snap_save_change_preset`). Contournement : `HX_PRESET_SAVE_HW=0`.
pub fn send_preset_save(state: &mut HelixState, preset_index: usize, name: &str) -> Result<(), String> {
    send_preset_label(
        state,
        preset_index,
        name,
        SAVE_WIRE,
        "sauvegarde preset HX désactivée (HX_PRESET_SAVE_HW=0)",
        preset_save_hw_enabled(),
    )
}

/// Renomme un snapshot (cmd `06`, suffixe `59:65`, map interne 2 clés
/// `{0x5c: index_snapshot_0based, 0x6d: nom}`).
///
/// `cd` = **compteur de SESSION ed:03** (octet après `83 66 cd`), PARTAGÉ par toutes les ops ed:03
/// de la session (lectures, activation, renommage) — ce n'est PAS une classe fixe. Preuve captures
/// HX Edit : `rename_first_snapshot` a `cd=0x04`, mais `rename_Third_snapshot_change_preset`
/// (session où activation+lectures+rename sont TOUS `cd=0x03`) a `cd=0x03`. Notre `send_snapshot_rename`
/// doit passer le MÊME `cd` que l'activation (`snapshot_write.rs` = 0x03) sinon le device REJETTE le
/// write (mismatch session) → lane ed:03 assommée → lectures suivantes KO. Bug trouvé 2026-08-06 : on
/// hardcodait `cd=0x04` (copié à tort de la classe « save » preset).
///
/// En-tête 2 octets plus court que le rename/save preset (pas de paire `0x6b:0`/`0x6c:preset_index`)
/// → `msg_size_byte`/padding décalés de 2 par rapport à `build_preset_label_packet`.
fn build_snapshot_label_packet(
    snapshot_index: u8,
    text: &[u8],
    lane_lo: u8,
    lane_hi: u8,
    cnt: u8,
    double: [u8; 2],
    cd: u8,
) -> Vec<u8> {
    let msg_size_byte = 0x1eu8.wrapping_add(text.len() as u8);
    let second_length_byte = msg_size_byte.wrapping_sub(0x10);
    let length_tag = preset_label_length_tag(text.len());

    let mut data = vec![
        msg_size_byte,
        0x00,
        0x00,
        0x18,
        0x80,
        0x10,
        0xed,
        0x03,
        0x00,
        cnt,
        0x00,
        0x04,
        lane_lo,
        lane_hi,
        0x00,
        0x00,
        0x01,
        0x00,
        0x06,
        0x00,
        second_length_byte,
        0x00,
        0x00,
        0x00,
        0x83,
        0x66,
        0xcd,
        cd,
        double[0],
        double[1],
        0x59,
        0x65,
        0x82,
        0x5c,
        snapshot_index,
        0x6d,
        length_tag,
    ];
    data.extend_from_slice(text);
    while data.len() < (msg_size_byte as usize) + 9 {
        data.push(0x00);
    }
    data
}

/// Témoin `HX_SNAPSHOT_RENAME_HW` (défaut ON) : renommage snapshot sur le HX. `=0` désactive l'envoi.
pub fn snapshot_rename_hw_enabled() -> bool {
    match std::env::var("HX_SNAPSHOT_RENAME_HW").as_deref() {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        Err(_) => true,
    }
}

/// Envoie le renommage d'un snapshot (`cd:04`, cmd `06`) sur le HX. `snapshot_index` 0-based (0..=3).
pub fn send_snapshot_rename(state: &mut HelixState, snapshot_index: u8, name: &str) -> Result<(), String> {
    if !snapshot_rename_hw_enabled() {
        return Err("renommage snapshot HX désactivé (HX_SNAPSHOT_RENAME_HW=0)".to_string());
    }
    if snapshot_index > 3 {
        return Err(format!("index snapshot invalide: {snapshot_index} (attendu 0..=3)"));
    }
    let text = preset_label_ascii_bytes(name);
    if text.is_empty() {
        return Err("nom snapshot vide".to_string());
    }

    let ctr = state.live_write_ctr;
    let lane_lo = (ctr & 0xff) as u8;
    let lane_hi = ((ctr >> 8) & 0xff) as u8;
    let cnt = state.next_x80_cnt();
    // IMPORTANT : le renommage de snapshot partage la lane ed:03 avec l'ACTIVATION de snapshot
    // (`snapshot_write.rs`) et tous les writes live, qui utilisent le compteur `live_write_yy` pour
    // l'octet-28 (`double[0]`, hi figé 0x64). L'utilisateur active un snapshot (→ `live_write_yy++`)
    // avant de le renommer ; si le renommage utilisait `next_editor_ed03_double()` (compteur SÉPARÉ,
    // avancé seulement par les labels), son double serait périmé → le device DROPPE le write et la
    // lane se désynchronise → les lectures preset suivantes échouent (bug confirmé sur
    // `rename_snap_save_ko` : renommages doubles f3/f4/f5 vs activations 1b/1c, device muet). On
    // s'aligne donc sur `live_write_yy`, exactement comme `build_snapshot_activate_packet`.
    let double = [state.live_write_yy, 0x64];
    // `cd` = compteur de session ed:03, MÊME valeur que l'activation snapshot (`snapshot_write.rs`
    // pp=0x03) et les lectures. Confirmé capture HX Edit `rename_Third_snapshot_change_preset`
    // (rename `cd=0x03`, comme toute la session). Hardcodé 0x03 comme l'activate (wrap yy→0x04 non
    // géré, cas rare — même simplification partout).
    let cd: u8 = 0x03;

    let data = build_snapshot_label_packet(snapshot_index, &text, lane_lo, lane_hi, cnt, double, cd);
    state.send(OutPacket::new(data));
    state.live_write_ctr = state.live_write_ctr.wrapping_add(LABEL_LANE_LO_DELTA);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAVE_CAPTURE: &str =
        "26:00:00:18:80:10:ed:03:00:b6:00:04:05:52:00:00:01:00:06:00:16:00:00:00:83:66:cd:04:1b:64:47:65:83:6b:00:6c:0d:6d:a7:52:65:6e:61:6d:65:00:00:00";

    fn bytes_from_hex_colon(s: &str) -> Vec<u8> {
        s.split(':').map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    #[test]
    fn save_body_matches_capture_layout() {
        let text = b"Rename".to_vec();
        let data = build_preset_label_packet(0x0d, &text, 0x05, 0x52, 0xb6, [0x1b, 0x64], SAVE_WIRE);
        let cap = bytes_from_hex_colon(SAVE_CAPTURE);
        assert_eq!(data.len(), cap.len());
        for (i, (&a, &b)) in data.iter().zip(cap.iter()).enumerate() {
            // Byte 27 = cd : compteur de SESSION (0x04 dans cette vieille capture, 0x03 dans
            // `rename_snap_and_save` et notre hardcode actuel) — skip car session-dépendant.
            if matches!(i, 9 | 12 | 13 | 27 | 28 | 29) {
                continue;
            }
            assert_eq!(a, b, "byte {i}");
        }
    }

    /// SAVE preset f5389 (`rename_snap_and_save`, 2026-08-07) : `… 83 66 cd 03 fc 64 47 65 83 6b 00
    /// 6c 19 6d ae "Active Snap 1" …`. Vérifie le fix : cd=0x03 (session, pas 0x04), suffixe 47:65,
    /// cmd=0x06, map preset `{0x6b,0x6c,0x6d:nom}`.
    #[test]
    fn save_matches_capture_cd03_session() {
        let text = b"Active Snap 1".to_vec();
        let data = build_preset_label_packet(0x19, &text, 0x34, 0x3f, 0xc1, [0xfc, 0x64], SAVE_WIRE);
        let cd_idx = data.windows(3).position(|w| w == [0x83, 0x66, 0xcd]).unwrap();
        assert_eq!(data[cd_idx + 3], 0x03, "cd session (pas 0x04)");
        assert_eq!(data[cd_idx + 4], 0xfc, "double[0]");
        assert_eq!(&data[cd_idx + 6..cd_idx + 8], &[0x47, 0x65], "suffixe save");
        assert_eq!(data[18], 0x06, "cmd save");
        // map preset : 83 6b 00 6c <idx> 6d <tag> <nom>
        assert_eq!(&data[cd_idx + 8..cd_idx + 12], &[0x83, 0x6b, 0x00, 0x6c]);
        assert_eq!(data[cd_idx + 12], 0x19, "preset index 25");
        assert!(data.windows(text.len()).any(|w| w == text.as_slice()));
    }

    #[test]
    fn rename_uses_cd03_cmd02_suffix_0665() {
        let text = b"New Name".to_vec();
        let data = build_preset_label_packet(0x20, &text, 0x05, 0x52, 0xb6, [0xed, 0x64], RENAME_WIRE);
        assert_eq!(&data[4..8], &[0x80, 0x10, 0xed, 0x03]);
        assert_eq!(data[18], 0x02);
        let cd_idx = data
            .windows(3)
            .position(|w| w == [0x83, 0x66, 0xcd])
            .unwrap();
        assert_eq!(data[cd_idx + 3], 0x03);
        assert_eq!(data[cd_idx + 6], 0x06);
        assert_eq!(data[cd_idx + 7], 0x65);
        assert!(data.windows(text.len()).any(|w| w == text.as_slice()));
    }

    const SNAPSHOT_RENAME_CAPTURE_SNAP1: &str =
        "27:00:00:18:80:10:ed:03:00:3e:00:04:73:08:01:00:01:00:06:00:17:00:00:00:83:66:cd:04:46:64:59:65:82:5c:00:6d:aa:46:69:72:73:74:53:6e:61:70:00:00";
    const SNAPSHOT_RENAME_CAPTURE_SNAP3: &str =
        "27:00:00:18:80:10:ed:03:00:9a:00:04:9b:42:00:00:01:00:06:00:17:00:00:00:83:66:cd:04:00:64:59:65:82:5c:02:6d:aa:54:68:69:72:64:53:6e:61:70:00:00";

    // Capture `rename_Third_snapshot_change_preset` f8190 : rename idx0 "Premier", session cd=0x03
    // (activation+lectures+rename tous cd=0x03) → PUIS le device répond et la lecture d'après RÉUSSIT.
    // C'est LA référence qui prouve que le rename doit porter le cd de session (0x03), pas 0x04.
    const SNAPSHOT_RENAME_CAPTURE_CD03: &str =
        "25:00:00:18:80:10:ed:03:00:ba:00:04:d0:4b:00:00:01:00:06:00:15:00:00:00:83:66:cd:03:fe:64:59:65:82:5c:00:6d:a8:50:72:65:6d:69:65:72:00:00:00:00";

    #[test]
    fn snapshot_rename_matches_capture_cd03_premier() {
        let text = b"Premier".to_vec();
        let data = build_snapshot_label_packet(0x00, &text, 0xd0, 0x4b, 0xba, [0xfe, 0x64], 0x03);
        let cap = bytes_from_hex_colon(SNAPSHOT_RENAME_CAPTURE_CD03);
        // La capture porte 2 nuls de padding USB en plus (48 vs 46 logiques, `msg_size+9`) — non
        // significatifs. On compare la portion logique, puis on vérifie que le reste = nuls.
        assert!(data.len() <= cap.len());
        for (i, (&a, &b)) in data.iter().zip(cap.iter()).enumerate() {
            // Octet 14 : 3e octet d'un compteur 32 bits de lane, 0 tant que <0x10000.
            if i == 14 {
                continue;
            }
            assert_eq!(a, b, "byte {i}");
        }
        assert!(cap[data.len()..].iter().all(|&b| b == 0x00), "padding non nul");
        // Vérifie explicitement le cd de session = 0x03 (le cœur du fix).
        let cd_idx = data.windows(3).position(|w| w == [0x83, 0x66, 0xcd]).unwrap();
        assert_eq!(data[cd_idx + 3], 0x03);
    }

    #[test]
    fn snapshot_rename_matches_capture_snap1_idx0() {
        let text = b"FirstSnap".to_vec();
        // Cette capture (session HX Edit distincte) a cd=0x04 — on le passe explicitement pour le test.
        let data = build_snapshot_label_packet(0x00, &text, 0x73, 0x08, 0x3e, [0x46, 0x64], 0x04);
        let cap = bytes_from_hex_colon(SNAPSHOT_RENAME_CAPTURE_SNAP1);
        assert_eq!(data.len(), cap.len());
        for (i, (&a, &b)) in data.iter().zip(cap.iter()).enumerate() {
            // Octet 14 : dynamique/non identifié, diffère entre les 2 captures (01 ici, 00 sur snap3).
            if i == 14 {
                continue;
            }
            assert_eq!(a, b, "byte {i}");
        }
    }

    #[test]
    fn snapshot_rename_matches_capture_snap3_idx2() {
        let text = b"ThirdSnap".to_vec();
        let data = build_snapshot_label_packet(0x02, &text, 0x9b, 0x42, 0x9a, [0x00, 0x64], 0x04);
        let cap = bytes_from_hex_colon(SNAPSHOT_RENAME_CAPTURE_SNAP3);
        assert_eq!(data.len(), cap.len());
        for (i, (&a, &b)) in data.iter().zip(cap.iter()).enumerate() {
            assert_eq!(a, b, "byte {i}");
        }
    }

    #[test]
    fn snapshot_rename_cmd06_suffix_5965_cd_param() {
        let text = b"MonCrunch".to_vec();
        // Runtime : cd=0x03 (compteur de session, comme l'activation).
        let data = build_snapshot_label_packet(0x01, &text, 0x05, 0x52, 0xb6, [0x1b, 0x64], 0x03);
        assert_eq!(&data[4..8], &[0x80, 0x10, 0xed, 0x03]);
        assert_eq!(data[18], 0x06);
        let cd_idx = data
            .windows(3)
            .position(|w| w == [0x83, 0x66, 0xcd])
            .unwrap();
        assert_eq!(data[cd_idx + 3], 0x03);
        assert_eq!(data[cd_idx + 6], 0x59);
        assert_eq!(data[cd_idx + 7], 0x65);
        // Map interne 2 clés {0x5c: index, 0x6d: nom} — pas de 0x6b/0x6c (contrairement au preset).
        let map_idx = cd_idx + 8;
        assert_eq!(data[map_idx], 0x82);
        assert_eq!(data[map_idx + 1], 0x5c);
        assert_eq!(data[map_idx + 2], 0x01);
        assert_eq!(data[map_idx + 3], 0x6d);
        assert!(data.windows(text.len()).any(|w| w == text.as_slice()));
    }
}
