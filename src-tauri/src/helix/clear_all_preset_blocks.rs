//! Vide tous les blocs FX du preset actif — séquence HX Edit `clear_all_block.json`.
//!
//! 14 paquets OUT rejoués depuis la capture (frames 4171→4367). Seuls les compteurs
//! lane / `x80` / double `cd:03` du 1ᵉʳ paquet sont patchés depuis l'état courant.
//!
//! # Handshake (modèle `matrix_routing_dd`)
//!
//! La capture HX Edit **n'est pas** une rafale : l'hôte envoie un groupe de chunks,
//! **attend l'ack device** (`ed:03:80:10` sub=`08`, ou l'IN `19`/36o sub=`04` du groupe C),
//! puis envoie le groupe suivant. 7 points de synchronisation stricts :
//!
//! | Groupe | OUT idx        | Ack device attendu                 | Δt device |
//! |--------|----------------|------------------------------------|-----------|
//! | A      | 0, 1           | `ed:03:80:10` sub=08 (fr 4175)     | 1.5 ms    |
//! | B      | 2, 3           | `ed:03:80:10` sub=08 (fr 4181)     | 1.4 ms    |
//! | C      | 4, 5           | `19`/36o sub=04       (fr 4187)     | 1.7 ms    |
//! | D      | 6              | `ed:03:80:10` sub=08 (fr 4191)     | 0.3 ms    |
//! | E      | 7, 8           | `ed:03:80:10` sub=08 (fr 4196)     | 0.2 ms    |
//! | F      | 9, 10 (ARM 08) | `ed:03:80:10` sub=08 (fr 4353)     | **105 ms**|
//! | G      | 11, 12         | `ed:03:80:10` sub=08 (fr 4365)*    | ~2 ms     |
//! | H      | 13             | (queue, hors fenêtre — best-effort)| —         |
//!
//! (*) Le groupe G émet aussi un `f0:03:02:10` (fr 4358, compteur de page) que le
//! handshake **ne consomme pas** : il retombe dans la FSM de page via le pipeline.
//!
//! # Déblocage app
//!
//! L'ancienne version prenait `&mut HelixState` et l'appelant Tauri tenait le mutex
//! pendant toute la boucle → le thread `usb_listener` ne pouvait plus notifier → gel dur.
//! Ici on passe par `Arc<Mutex<HelixState>>` et on **relâche le lock** pendant chaque
//! `wait_ack`, exactement comme `execute_routing_marker_dd`.
//!
//! # Garde `HXL_CLEAR_ALL_HANDSHAKE`
//!
//! - `=1` : handshake complet (7 attentes d'ack).
//! - absent / `=0` (**témoin**) : envoie les 14 OUT sans attendre les IN, mais lock
//!   toujours relâché entre chaque `send`. Isole la part « handshake » de la part « lock ».

use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::helix::clear_all_preset_blocks_wire::{
    CLEAR_ALL_LANE_BUMP_AFTER_PACKET, CLEAR_ALL_LANE_BUMP_DELTA, CLEAR_ALL_OUT_PACKETS,
};
use crate::helix::packet::OutPacket;
use crate::helix::scroll_model_pull;
use crate::helix::HelixState;

/// Timeout d'un ack device (groupe F mesuré à ~105 ms → 500 ms de marge).
const CLEAR_ALL_ACK_TIMEOUT_MS: u64 = 500;
/// Granularité de scrutation du canal de notification.
const CLEAR_ALL_POLL_MS: u64 = 5;

/// Attente d'ack après chaque OUT idx (dérivé de `clear_all_block.json`).
/// `true` aux frontières de groupe A..G ; groupe H (idx 13) = best-effort, pas d'attente.
//                                          0      1     2      3     4      5     6     7      8     9     10    11     12    13
pub const CLEAR_ALL_AWAIT_ACK_AFTER: [bool; 14] = [
    false, true, false, true, false, true, true, false, true, false, true, false, true, false,
];

/// Un seul type d'événement : les deux formes d'ack (`ed:03` sub=08 et `19`/36o) sont
/// non ambiguës dans ce dialogue strictement séquentiel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearAllInEvent {
    DeviceAck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClearAllAwaiting {
    Idle,
    Ack,
}

/// Session d'attente IN pendant un clear-all (notif via `usb_in_pipeline`).
pub struct ClearAllWait {
    pub notify_tx: SyncSender<ClearAllInEvent>,
    awaiting: ClearAllAwaiting,
}

impl ClearAllWait {
    fn arm_await(&mut self) {
        self.awaiting = ClearAllAwaiting::Ack;
    }
}

// ---------------------------------------------------------------------------
// Prédicats IN
// ---------------------------------------------------------------------------

/// Écho court DEV→HOST `ed:03` sub=`08` après un groupe de chunks (fr 4175/4181/…/4365).
/// Discrimine du heartbeat idle qui porte sub=`10` (compteur figé `e5:02`).
pub fn is_clear_all_ed03_ack08(data: &[u8]) -> bool {
    data.len() == 16
        && data.get(0..4) == Some(&[0x08, 0x00, 0x00, 0x18])
        && data.get(4..8) == Some(&[0xed, 0x03, 0x80, 0x10])
        && data.get(11) == Some(&0x08)
}

/// IN `19`/36o sub=`04` du groupe C (fr 4187), juste avant le bump de lane.
pub fn is_clear_all_in19(data: &[u8]) -> bool {
    data.len() == 36
        && data.first() == Some(&0x19)
        && data.get(4..8) == Some(&[0xed, 0x03, 0x80, 0x10])
        && data.get(11) == Some(&0x04)
}

/// IN à absorber par la couche clear-all. **Exclut** volontairement le `f0:03:02:10`
/// (compteur de page) pour qu'il retombe dans la FSM phase 4.
pub fn is_clear_all_pipeline_in(data: &[u8]) -> bool {
    is_clear_all_ed03_ack08(data) || is_clear_all_in19(data)
}

/// Notifie le thread clear-all si un ack attendu arrive. Retourne `true` si notifié.
/// (Le handler pipeline consomme de toute façon tout `is_clear_all_pipeline_in`.)
pub fn try_notify_clear_all_in(state: &mut HelixState, data: &[u8]) -> bool {
    let Some(wait) = state.clear_all_wait.as_mut() else {
        return false;
    };
    if wait.awaiting != ClearAllAwaiting::Ack {
        return false;
    }
    if !is_clear_all_pipeline_in(data) {
        return false;
    }
    wait.awaiting = ClearAllAwaiting::Idle;
    let _ = wait.notify_tx.try_send(ClearAllInEvent::DeviceAck);
    true
}

// ---------------------------------------------------------------------------
// Patch des templates
// ---------------------------------------------------------------------------

fn patch_clear_all_packet(
    out: &mut [u8],
    seq_x80: u8,
    lane: u16,
    editor_double: Option<[u8; 2]>,
) {
    if out.len() >= 14 {
        out[9] = seq_x80;
        out[12] = (lane & 0xff) as u8;
        out[13] = ((lane >> 8) & 0xff) as u8;
    }
    if let Some(d) = editor_double {
        for i in 0..out.len().saturating_sub(6) {
            if out[i] == 0x83 && out[i + 1] == 0x66 && out[i + 2] == 0xcd {
                out[i + 4] = d[0];
                out[i + 5] = d[1];
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scope RAII (pose hold + wait, retire au drop)
// ---------------------------------------------------------------------------

struct ClearAllScope {
    helix_arc: Arc<Mutex<HelixState>>,
    rx: Receiver<ClearAllInEvent>,
}

impl ClearAllScope {
    fn open(helix_arc: Arc<Mutex<HelixState>>) -> Self {
        let (tx, rx) = sync_channel::<ClearAllInEvent>(4);
        {
            let mut s = helix_arc.lock().unwrap();
            s.usb_host_transaction_hold = true;
            s.clear_all_wait = Some(ClearAllWait {
                notify_tx: tx,
                awaiting: ClearAllAwaiting::Idle,
            });
        }
        Self { helix_arc, rx }
    }

    /// Vide les résidus du canal puis arme l'attente (à faire sous le même lock que le `send`).
    fn drain(&self) {
        while self.rx.try_recv().is_ok() {}
    }

    fn arm(state: &mut HelixState) {
        if let Some(w) = state.clear_all_wait.as_mut() {
            w.arm_await();
        }
    }

    /// Attend l'ack device du groupe `idx` (lock **non** tenu par l'appelant).
    fn wait_ack(&self, idx: usize) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(CLEAR_ALL_ACK_TIMEOUT_MS);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let poll = remaining.min(Duration::from_millis(CLEAR_ALL_POLL_MS));
            match self.rx.recv_timeout(poll) {
                Ok(ClearAllInEvent::DeviceAck) => {
                    return Ok(());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(format!("clear all idx={idx} : canal IN fermé"));
                }
            }
        }
        Err(format!(
            "clear all idx={idx} : timeout {CLEAR_ALL_ACK_TIMEOUT_MS} ms sans ack device"
        ))
    }
}

impl Drop for ClearAllScope {
    fn drop(&mut self) {
        let mut s = self.helix_arc.lock().unwrap();
        s.clear_all_wait = None;
        s.usb_host_transaction_hold = false;
    }
}

// ---------------------------------------------------------------------------
// Exécuteur
// ---------------------------------------------------------------------------

fn clear_all_handshake_enabled() -> bool {
    std::env::var("HXL_CLEAR_ALL_HANDSHAKE").ok().as_deref() == Some("1")
}

/// Envoie la séquence « clear all blocks » sur le preset actif.
///
/// Passe par `Arc<Mutex>` et relâche le lock pendant chaque `wait_ack` : c'est la
/// condition pour que `usb_listener` puisse notifier via `try_notify_clear_all_in`.
pub fn execute_clear_all_preset_blocks(helix_arc: Arc<Mutex<HelixState>>) -> Result<(), String> {
    {
        let s = helix_arc.lock().unwrap();
        if !s.editor_ready {
            return Err("Amorçage USB en cours — clear all indisponible".to_string());
        }
        if s.preset_content_only {
            return Err("Lecture preset en cours — reportez clear all".to_string());
        }
        if scroll_model_pull::hw_model_usb_busy(&s) {
            return Err("Scroll modèle hardware en cours — reportez clear all".to_string());
        }
    }

    let handshake = clear_all_handshake_enabled();

    // Snapshot lane de base + double editor (lock court).
    let (base_lane, editor_double) = {
        let s = helix_arc.lock().unwrap();
        (s.live_write_ctr, s.editor_ed03_double_val())
    };

    debug_assert_eq!(
        CLEAR_ALL_OUT_PACKETS.len(),
        CLEAR_ALL_AWAIT_ACK_AFTER.len(),
        "table d'attente désalignée des templates"
    );

    // Pose hold + wait ; retirés au drop de `scope` (y compris sur `?`).
    let scope = ClearAllScope::open(Arc::clone(&helix_arc));

    for idx in 0..CLEAR_ALL_OUT_PACKETS.len() {
        let await_after = handshake && CLEAR_ALL_AWAIT_ACK_AFTER[idx];

        if await_after {
            scope.drain(); // résidu éventuel avant d'armer
        }

        // build + patch + (arm) + send, tout sous un seul lock court.
        {
            let mut s = helix_arc.lock().unwrap();

            let mut pkt = CLEAR_ALL_OUT_PACKETS[idx].to_vec();
            // Bump de lane dès idx 6 (capture : `94:2a` → `a5:2a` = +0x11 à idx 6).
            let lane = if idx >= CLEAR_ALL_LANE_BUMP_AFTER_PACKET {
                base_lane.wrapping_add(CLEAR_ALL_LANE_BUMP_DELTA)
            } else {
                base_lane
            };
            let double = if idx == 0 { Some(editor_double) } else { None };
            let x80 = s.next_x80_cnt();
            patch_clear_all_packet(&mut pkt, x80, lane, double);

            // Armer AVANT de send, sous le même lock : l'IN ne peut pas être traité
            // tant qu'on tient le mutex, donc `awaiting` est déjà positionné.
            if await_after {
                ClearAllScope::arm(&mut s);
            }
            s.send(OutPacket::new(pkt));
        }

        // Attente sans lock : le listener peut prendre le mutex et notifier.
        if await_after {
            scope.wait_ack(idx)?;
        }
    }

    {
        let mut s = helix_arc.lock().unwrap();
        s.live_write_ctr = base_lane.wrapping_add(CLEAR_ALL_LANE_BUMP_DELTA);
    }

    Ok(())
    // `scope` drop ici → usb_host_transaction_hold = false, clear_all_wait = None.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_match_capture_lengths() {
        let lens: [usize; 14] = [512, 32, 512, 32, 512, 32, 24, 512, 24, 24, 16, 512, 32, 400];
        for (i, tpl) in CLEAR_ALL_OUT_PACKETS.iter().enumerate() {
            assert_eq!(tpl.len(), lens[i], "packet {i}");
            assert_eq!(&tpl[4..8], &[0x80, 0x10, 0xed, 0x03]);
        }
    }

    #[test]
    fn await_table_matches_template_count() {
        assert_eq!(CLEAR_ALL_OUT_PACKETS.len(), CLEAR_ALL_AWAIT_ACK_AFTER.len());
    }

    #[test]
    fn await_frontiers_are_group_boundaries() {
        // A..G = idx 1,3,5,6,8,10,12 ; groupe H (13) = best-effort.
        let expected = [1usize, 3, 5, 6, 8, 10, 12];
        for (idx, &flag) in CLEAR_ALL_AWAIT_ACK_AFTER.iter().enumerate() {
            assert_eq!(flag, expected.contains(&idx), "idx {idx}");
        }
    }

    #[test]
    fn patch_lane_and_double() {
        let mut pkt = CLEAR_ALL_OUT_PACKETS[0].to_vec();
        patch_clear_all_packet(&mut pkt, 0xab, 0xbeef, Some([0x11, 0x64]));
        assert_eq!(pkt[9], 0xab);
        assert_eq!(pkt[12], 0xef);
        assert_eq!(pkt[13], 0xbe);
        assert_eq!(pkt[28], 0x11);
        assert_eq!(pkt[29], 0x64);
    }

    #[test]
    fn bump_lane_starts_at_idx_6() {
        // idx 5 → base ; idx 6 → base+0x11 (frontière capture).
        let base: u16 = 0x2a94;
        for idx in 0..CLEAR_ALL_OUT_PACKETS.len() {
            let lane = if idx >= CLEAR_ALL_LANE_BUMP_AFTER_PACKET {
                base.wrapping_add(CLEAR_ALL_LANE_BUMP_DELTA)
            } else {
                base
            };
            let want = if idx >= 6 { 0x2aa5 } else { 0x2a94 };
            assert_eq!(lane, want, "idx {idx}");
        }
    }

    #[test]
    fn ed03_ack08_matcher() {
        let ack = [
            0x08, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x5d, 0x00, 0x08, 0xe5, 0x04,
            0x00, 0x00,
        ];
        assert!(is_clear_all_ed03_ack08(&ack));
        assert!(is_clear_all_pipeline_in(&ack));

        // Heartbeat idle : sub=0x10 → ne doit PAS matcher.
        let heartbeat = [
            0x08, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x5d, 0x00, 0x10, 0xe5, 0x02,
            0x00, 0x00,
        ];
        assert!(!is_clear_all_ed03_ack08(&heartbeat));
        assert!(!is_clear_all_pipeline_in(&heartbeat));
    }

    #[test]
    fn in19_matcher_group_c() {
        // fr 4187, tronqué à 36 o.
        let in19 = [
            0x19, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x5f, 0x00, 0x04, 0xed, 0x06,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x03,
            0xf4, 0x67, 0x01, 0x68, 0xc0, 0x93, 0xc2, 0x40,
        ];
        assert!(is_clear_all_in19(&in19));
        assert!(is_clear_all_pipeline_in(&in19));
    }

    #[test]
    fn f0_page_counter_is_not_absorbed() {
        // fr 4358 : `f0:03:02:10` → doit retomber dans la FSM de page, pas être consommé.
        let f0 = [
            0x08, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x3d, 0x00, 0x10, 0x09, 0x02,
            0x00, 0x00,
        ];
        assert!(!is_clear_all_pipeline_in(&f0));
    }
}