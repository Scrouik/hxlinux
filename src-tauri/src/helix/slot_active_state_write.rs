//! Écriture de l'état actif/inactif d'un bloc entier (bus Kempline), format Command Center.
//!
//! Distinct du format normal de `live_write.rs` (`85:62:<bus>:1d:<c2|c3>:1a:00:1c:<pSel>:77:...`,
//! une paire de trames `04`/`0c`) : ici un **paquet unique**, tail `82:62:<bus>:3b:<c2|c3>`, sans
//! `param_selector` ni bloc modèle. Format vérifié par capture différentielle (2 mêmes presets,
//! seul l'état actif/inactif d'un slot diffère) — voir mémoire projet 2026-07-09.
//!
//! Inconnues restantes (un seul bus testé au moment de l'écriture) : `pp`/`term` sont peut-être
//! fixes, peut-être dépendants du slot/modèle — à confirmer par test utilisateur sur d'autres bus.

use crate::helix::HelixState;
use serde::Serialize;

/// Charge utile de l'évènement `models:slot-active-state-changed` (bascule faite sur le device).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotActiveStateChangedPayload {
    pub slot_bus: u8,
    pub active: bool,
}

/// Construit le paquet d'écriture actif/inactif pour `slot_bus`. Avance les compteurs partagés
/// (`live_write_ctr`/`live_write_yy`) comme les autres écritures live, pour rester cohérent avec
/// la séquence attendue par le firmware sur les écritures suivantes.
pub fn build_slot_active_state_write_packet(
    state: &mut HelixState,
    slot_bus: u8,
    active: bool,
) -> Vec<u8> {
    let seq = state.next_x80_cnt();
    let ctr = state.live_write_ctr;
    let yy = state.live_write_yy;
    // Valeurs observées identiques sur les 2 seules écritures capturées (même bus) — à reconfirmer
    // sur d'autres slots/modèles avant de les considérer comme universelles.
    let pp: u8 = 0x04;
    let term: u8 = 0x29;
    let value: u8 = if active { 0xc3 } else { 0xc2 };

    let packet = vec![
        0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03,
        0x00, seq, 0x00, 0x04,
        (ctr & 0xff) as u8, ((ctr >> 8) & 0xff) as u8,
        0x00, 0x00,
        0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, pp, yy, 0x64, term, 0x65,
        0x82, 0x62, slot_bus, 0x3b, value, 0x00, 0x00, 0x00,
    ];

    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x1f);
    state.live_write_yy = state.live_write_yy.wrapping_add(1);
    packet
}

/// Détecte un changement d'état actif/inactif fait **directement sur le device** (canal `f0:03`,
/// même tag `82:62:<bus>:3b:<c2|c3>` que l'écriture ci-dessus). Retourne `(slot_bus, active)`.
/// Vérifié par capture (2026-07-09, `activ_unactiv_from_device.json` : 4 appuis physiques,
/// alternance propre `c3`/`c2`).
pub fn ingest_slot_active_state_wire_in(data: &[u8]) -> Option<(u8, bool)> {
    if data.get(4..8)? != [0xf0, 0x03, 0x02, 0x10] {
        return None;
    }
    if data.get(36..38)? != [0x82, 0x62] {
        return None;
    }
    let bus = *data.get(38)?;
    if data.get(39).copied()? != 0x3b {
        return None;
    }
    match data.get(40).copied()? {
        0xc3 => Some((bus, true)),
        0xc2 => Some((bus, false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paquet réel capturé (t=1.021s, bus=01, actif).
    const IN_ACTIVE: &[u8] = &[
        0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x89, 0x00, 0x04, 0x09, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x82, 0x69, 0x31, 0x6a, 0x84, 0x52,
        0x00, 0x44, 0x05, 0x79, 0x11, 0x6a, 0x82, 0x62, 0x01, 0x3b, 0xc3, 0x00, 0x14, 0xc3,
    ];
    /// Idem, appui suivant (t=2.343s), inactif.
    const IN_INACTIVE: &[u8] = &[
        0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x8b, 0x00, 0x04, 0x09, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x82, 0x69, 0x31, 0x6a, 0x84, 0x52,
        0x00, 0x44, 0x05, 0x79, 0x11, 0x6a, 0x82, 0x62, 0x01, 0x3b, 0xc2, 0x00, 0x14, 0xc3,
    ];

    #[test]
    fn ingest_detects_active_and_inactive_from_real_device_packets() {
        assert_eq!(ingest_slot_active_state_wire_in(IN_ACTIVE), Some((0x01, true)));
        assert_eq!(ingest_slot_active_state_wire_in(IN_INACTIVE), Some((0x01, false)));
    }

    #[test]
    fn ingest_ignores_unrelated_packets() {
        assert_eq!(ingest_slot_active_state_wire_in(&[0x08, 0x00, 0x00, 0x18]), None);
        let mut other = IN_ACTIVE.to_vec();
        other[39] = 0x66; // mauvais tag (Type Command Center, pas actif/inactif)
        assert_eq!(ingest_slot_active_state_wire_in(&other), None);
    }

    #[test]
    fn builds_expected_static_bytes_for_active_and_inactive() {
        let mut state = HelixState::new();
        let active_pkt = build_slot_active_state_write_packet(&mut state, 0x07, true);
        assert_eq!(active_pkt.len(), 40);
        assert_eq!(&active_pkt[0..8], &[0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03]);
        assert_eq!(&active_pkt[24..27], &[0x83, 0x66, 0xcd]);
        assert_eq!(&active_pkt[32..37], &[0x82, 0x62, 0x07, 0x3b, 0xc3]);
        assert_eq!(&active_pkt[37..40], &[0x00, 0x00, 0x00]);

        let mut state2 = HelixState::new();
        let inactive_pkt = build_slot_active_state_write_packet(&mut state2, 0x07, false);
        assert_eq!(&inactive_pkt[32..37], &[0x82, 0x62, 0x07, 0x3b, 0xc2]);
    }

    #[test]
    fn advances_shared_live_write_counters() {
        let mut state = HelixState::new();
        let ctr_before = state.live_write_ctr;
        let yy_before = state.live_write_yy;
        let _ = build_slot_active_state_write_packet(&mut state, 0x01, true);
        assert_eq!(state.live_write_ctr, ctr_before.wrapping_add(0x1f));
        assert_eq!(state.live_write_yy, yy_before.wrapping_add(1));
    }
}
