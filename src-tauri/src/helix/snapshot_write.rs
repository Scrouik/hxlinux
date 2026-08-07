//! Activation d'un snapshot (bascule entre les 4 snapshots d'un preset).
//!
//! Décodé sur `captures/usb-wireshark/Snapshot/change snapshot_1_2_3_4_1.json` (2026-07-13) :
//! chaque changement de snapshot dans HX Edit envoie un groupe de 4 paquets, mais **UN SEUL porte
//! le numéro de snapshot** — c'est celui-ci (`term=0x58`, payload `81 5c <index0based> 00`). Les 3
//! autres (`0x17`/`0x16` payload `c0` nil, `0x18` payload `81 76 49` constant) ne portent PAS le
//! snapshot et sont, à ce stade, considérés comme de la tuyauterie non reproduite (approche
//! minimale : on envoie d'abord le seul `0x58`, à valider device ; on ajoutera les compagnons si la
//! bascule est incomplète).
//!
//! `snapshot_index` est **0-based** : Snap 1 = 0, Snap 2 = 1, …, Snap 4 = 3 (confirmé byte-exact
//! sur les 4 trames : switch→Snap2=`01`, Snap3=`02`, Snap4=`03`, retour Snap1=`00`).

use crate::helix::HelixState;

/// Construit le paquet d'activation du snapshot `snapshot_index` (0-based, 0..=3).
/// `ctr +0x11`/`yy +1` (mêmes deltas que les autres écritures mono-champ). `pp=0x03` (même
/// simplification que le reste du codebase : le passage à `0x04` au wrap de `yy` n'est pas géré,
/// cas rare pour une action manuelle — cf `command_center_write.rs`).
pub fn build_snapshot_activate_packet(state: &mut HelixState, snapshot_index: u8) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;
    let pp: u8 = 0x03;

    let packet = vec![
        0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, 0x58, 0x65,
        0x81, 0x5c, snapshot_index, 0x00,
    ];

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x11);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    state.active_snapshot_index = snapshot_index; // suivi de l'actif (pour l'activation conditionnelle du renommage)
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Octets réels capturés (`change snapshot_1_2_3_4_1.json`) — 3 des 4 trames `0x58` (celles où
    /// `pp=0x03`, la 4e ayant `pp=0x04` par wrap de la lane `cd`, non géré comme partout ailleurs).
    #[test]
    fn builds_expected_bytes_for_snapshot_activate() {
        // Frame 1649 : activer Snap 2 (index 1). ctr=0x28e9, yy=0xf5.
        let mut state = HelixState::new();
        state.live_write_ctr = 0x28e9;
        state.live_write_yy = 0xf5;
        let p = build_snapshot_activate_packet(&mut state, 1);
        assert_eq!(p.len(), 36);
        assert_eq!(p[0], 0x1b);
        assert_eq!(&p[10..14], &[0x00, 0x04, 0xe9, 0x28]);
        assert_eq!(p[20], 0x0b);
        assert_eq!(&p[24..32], &[0x83, 0x66, 0xcd, 0x03, 0xf5, 0x64, 0x58, 0x65]);
        assert_eq!(&p[32..], &[0x81, 0x5c, 0x01, 0x00]);
        assert_eq!(state.live_write_ctr, 0x28fa);
        assert_eq!(state.live_write_yy, 0xf6);

        // Frame 3171 : activer Snap 3 (index 2). ctr=0x3504, yy=0xf9.
        let mut state = HelixState::new();
        state.live_write_ctr = 0x3504;
        state.live_write_yy = 0xf9;
        let p = build_snapshot_activate_packet(&mut state, 2);
        assert_eq!(&p[24..32], &[0x83, 0x66, 0xcd, 0x03, 0xf9, 0x64, 0x58, 0x65]);
        assert_eq!(&p[32..], &[0x81, 0x5c, 0x02, 0x00]);

        // Frame 4517 : activer Snap 4 (index 3). ctr=0x411f, yy=0xfd.
        let mut state = HelixState::new();
        state.live_write_ctr = 0x411f;
        state.live_write_yy = 0xfd;
        let p = build_snapshot_activate_packet(&mut state, 3);
        assert_eq!(&p[24..32], &[0x83, 0x66, 0xcd, 0x03, 0xfd, 0x64, 0x58, 0x65]);
        assert_eq!(&p[32..], &[0x81, 0x5c, 0x03, 0x00]);
    }

    /// Retour Snap 1 = index 0.
    #[test]
    fn snapshot1_is_index_zero() {
        let mut state = HelixState::new();
        let p = build_snapshot_activate_packet(&mut state, 0);
        assert_eq!(&p[32..], &[0x81, 0x5c, 0x00, 0x00]);
    }
}
