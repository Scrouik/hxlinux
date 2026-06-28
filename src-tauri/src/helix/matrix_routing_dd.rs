//! Handshake USB split/merge D&D (captures `d&d_split.json`, `d&d_merge.json`).
//!
//! Séquence HX Edit par geste (Path 1, colonne frontière 0..8) — pas de focus `cd:03` :
//!   arm `ed`+`f0` sub=`10`
//!   write #1 `1d` `cd:03` → IN `19` → ACK `ed` → IN `21` → ACK `f0`
//!   write #2 `1b` `cd:03` → IN `19` → ACK `ed` → IN `25` → ACK `f0`
//!
//! Le focus structurel `cd:03` (`focus_split_merge.json`) est réservé au clic split/merge UI.
//!
//! « write #2 / branche B » = 2ᵉ paquet USB (`1b`), pas la rangée Path 2 de la grille UI.

use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::helix::matrix_routing_move::{build_matrix_routing_marker_move_packet, RoutingMarkerKind};
use crate::helix::packet::OutPacket;
use crate::helix::path1_io_live_write::{
    build_f0_dd_post_commit_sub08, build_post_1d_ack08, send_matrix_dd_drag_arm,
};
use crate::helix::HelixState;

const DD_TIMEOUT_MS: u64 = 800;
const DD_POLL_MS: u64 = 5;
const DD_DELAY_AFTER_ARM_MS: u64 = 35;
const DD_INTER_ACK_MS: u64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdInEvent {
    In19,
    In21,
    In25,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DdAwaiting {
    Idle,
    In19,
    In21,
    In25,
}

/// Session d'attente IN pendant un move routing (notif via `usb_in_pipeline`).
pub struct MatrixRoutingDdWait {
    pub notify_tx: SyncSender<DdInEvent>,
    awaiting: DdAwaiting,
}

impl MatrixRoutingDdWait {
    fn arm_await(&mut self, kind: DdAwaiting) {
        self.awaiting = kind;
    }
}

/// Tout IN `19`/`21`/`25` du dialogue routing D&D (cd:04).
pub fn is_routing_dd_session_in(data: &[u8]) -> bool {
    is_routing_dd_in19(data) || is_routing_dd_in21(data) || is_routing_dd_in25(data)
}

pub fn is_routing_dd_in19(data: &[u8]) -> bool {
    data.len() == 36
        && data.first() == Some(&0x19)
        && data.get(4..8) == Some(&[0xed, 0x03, 0x80, 0x10])
        && data.get(11) == Some(&0x04)
}

pub fn is_routing_dd_in21(data: &[u8]) -> bool {
    data.len() == 44
        && data.first() == Some(&0x21)
        && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
        && data.get(11) == Some(&0x04)
}

/// Dump post-commit après le 2ᵉ write `1b` (`d&d_split.json`, `d&d_merge.json`).
pub fn is_routing_dd_in25(data: &[u8]) -> bool {
    data.len() == 48
        && data.first() == Some(&0x25)
        && data.get(4..8) == Some(&[0xf0, 0x03, 0x02, 0x10])
        && data.get(11) == Some(&0x04)
}

/// Écho court DEV→HOST `ed:03` sub=`08` après un bulk routing (capture `d&d_split_linux.json` #173).
pub fn is_routing_dd_device_ed03_ack08(data: &[u8]) -> bool {
    data.len() == 16
        && data.get(0..4) == Some(&[0x08, 0x00, 0x00, 0x18])
        && data.get(4..8) == Some(&[0xed, 0x03, 0x80, 0x10])
        && data.get(11) == Some(&0x08)
}

/// IN à absorber par la couche routing D&D (dialogue + échos device).
pub fn is_routing_dd_pipeline_in(data: &[u8]) -> bool {
    is_routing_dd_session_in(data) || is_routing_dd_device_ed03_ack08(data)
}

/// Notifie le thread move si un IN attendu arrive.
pub fn try_notify_routing_dd_in(state: &mut HelixState, data: &[u8]) -> bool {
    let Some(wait) = state.matrix_routing_dd_wait.as_mut() else {
        return false;
    };
    let event = match wait.awaiting {
        DdAwaiting::In19 if is_routing_dd_in19(data) => Some(DdInEvent::In19),
        DdAwaiting::In21 if is_routing_dd_in21(data) => Some(DdInEvent::In21),
        DdAwaiting::In25 if is_routing_dd_in25(data) => Some(DdInEvent::In25),
        _ => None,
    };
    let Some(ev) = event else {
        return false;
    };
    wait.awaiting = DdAwaiting::Idle;
    let _ = wait.notify_tx.try_send(ev);
    true
}

pub fn build_matrix_routing_path2_commit_packet(
    state: &mut HelixState,
    kind: RoutingMarkerKind,
    dest_boundary_col: u8,
    path1_lane: [u8; 3],
) -> Vec<u8> {
    // HX Edit : un `next_editor_ed03_double()` par bulk `1d` **et** par bulk `1b`
    // (`d&d_split.json` : tags `f2` puis `f3`). Ne pas réutiliser `tag_1d + 1` sans avancer
    // l'état — sinon le 2ᵉ D&D repart sur le tag du `1b` précédent.
    let tag = state.next_editor_ed03_double()[0];
    let mid_byte = match kind {
        RoutingMarkerKind::Split => 0x2e,
        RoutingMarkerKind::Merge => 0x2f,
    };
    // `dd_split.json` : b[34] = colonne destination sur le fil (= `splitCol` 0..8 + 1).
    let dest_wire = dest_boundary_col.wrapping_add(1);
    let cnt = state.next_x80_cnt();
    let lane_lo = path1_lane[0].wrapping_add(0x11);
    vec![
        0x1b,
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
        path1_lane[1],
        path1_lane[2],
        0x00,
        0x01,
        0x00,
        0x06,
        0x00,
        0x0b,
        0x00,
        0x00,
        0x00,
        0x83,
        0x66,
        0xcd,
        0x03,
        tag,
        0x64,
        mid_byte,
        0x65,
        0x81,
        0x62,
        dest_wire,
        0x00,
    ]
}

struct RoutingDdScope {
    helix_arc: Arc<Mutex<HelixState>>,
    rx: Receiver<DdInEvent>,
}

impl RoutingDdScope {
    fn open(helix_arc: Arc<Mutex<HelixState>>) -> Self {
        let (tx, rx) = sync_channel::<DdInEvent>(4);
        {
            let mut s = helix_arc.lock().unwrap();
            s.usb_host_transaction_hold = true;
            s.matrix_routing_dd_wait = Some(MatrixRoutingDdWait {
                notify_tx: tx,
                awaiting: DdAwaiting::Idle,
            });
        }
        Self { helix_arc, rx }
    }

    fn arm_in19_on(state: &mut HelixState) {
        if let Some(w) = state.matrix_routing_dd_wait.as_mut() {
            w.arm_await(DdAwaiting::In19);
        }
    }

    fn arm_in21_on(state: &mut HelixState) {
        if let Some(w) = state.matrix_routing_dd_wait.as_mut() {
            w.arm_await(DdAwaiting::In21);
        }
    }

    fn arm_in25_on(state: &mut HelixState) {
        if let Some(w) = state.matrix_routing_dd_wait.as_mut() {
            w.arm_await(DdAwaiting::In25);
        }
    }

    fn wait_in(&self, expect: DdInEvent, label: &str) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(DD_TIMEOUT_MS);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let poll = remaining.min(Duration::from_millis(DD_POLL_MS));
            match self.rx.recv_timeout(poll) {
                Ok(got) if got == expect => {
                    eprintln!("[MatrixRoutingDd] {label} ← {:?}", expect);
                    return Ok(());
                }
                Ok(got) => {
                    return Err(format!(
                        "routing D&D {label} : IN inattendu {:?} (attendu {:?})",
                        got, expect
                    ));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(format!("routing D&D {label} : canal IN fermé"));
                }
            }
        }
        Err(format!(
            "routing D&D {label} : timeout {DD_TIMEOUT_MS} ms sans {:?}",
            expect
        ))
    }
}

impl Drop for RoutingDdScope {
    fn drop(&mut self) {
        let mut s = self.helix_arc.lock().unwrap();
        s.matrix_routing_dd_wait = None;
        s.usb_host_transaction_hold = false;
    }
}

fn send_ed_ack(state: &mut HelixState, out_pkt: &[u8], ack_hi_plus_one: bool) {
    let ack_lo = out_pkt[12].wrapping_add(0x11);
    let ack_hi = if ack_hi_plus_one {
        out_pkt[13].wrapping_add(1)
    } else {
        out_pkt[13]
    };
    let post_ed = build_post_1d_ack08(state, ack_lo, ack_hi);
    state.send(OutPacket::new(post_ed));
    eprintln!(
        "[MatrixRoutingDd] ACK ed {:02x}{:02x} (hi+1={ack_hi_plus_one})",
        ack_lo, ack_hi
    );
}

fn send_f0_ack(state: &mut HelixState) {
    let post_f0 = build_f0_dd_post_commit_sub08(state);
    state.send(OutPacket::with_delay(post_f0, DD_INTER_ACK_MS));
}

/// 1er write `1d` : IN `19` → ACK `ed` → (IN `21` → ACK `f0` si dump).
fn commit_1d_with_handshake(
    scope: &RoutingDdScope,
    helix_arc: &Arc<Mutex<HelixState>>,
    pkt: Vec<u8>,
    label: &str,
) -> Result<(), String> {
    {
        let mut s = helix_arc.lock().unwrap();
        RoutingDdScope::arm_in19_on(&mut s);
        s.send(OutPacket::new(pkt.clone()));
    }
    scope.wait_in(DdInEvent::In19, label)?;
    {
        let mut s = helix_arc.lock().unwrap();
        send_ed_ack(&mut s, &pkt, false);
        RoutingDdScope::arm_in21_on(&mut s);
    }
    if scope
        .wait_in(DdInEvent::In21, &format!("{label} f0 dump"))
        .is_ok()
    {
        let mut s = helix_arc.lock().unwrap();
        send_f0_ack(&mut s);
    }
    Ok(())
}

/// 2ᵉ write `1b` (branche B) : IN `19` → ACK `ed` → IN `25` → ACK `f0`.
/// ACK `ed` merge : octet 13 = `out[13]+1` (`d&d_merge.json` #1545).
fn commit_1b_with_handshake(
    scope: &RoutingDdScope,
    helix_arc: &Arc<Mutex<HelixState>>,
    pkt: Vec<u8>,
    kind: RoutingMarkerKind,
    label: &str,
) -> Result<(), String> {
    let ack_hi_plus_one = kind == RoutingMarkerKind::Merge;
    {
        let mut s = helix_arc.lock().unwrap();
        RoutingDdScope::arm_in19_on(&mut s);
        s.send(OutPacket::new(pkt.clone()));
    }
    scope.wait_in(DdInEvent::In19, label)?;
    {
        let mut s = helix_arc.lock().unwrap();
        send_ed_ack(&mut s, &pkt, ack_hi_plus_one);
        RoutingDdScope::arm_in25_on(&mut s);
    }
    if scope
        .wait_in(DdInEvent::In25, &format!("{label} dump"))
        .is_ok()
    {
        let mut s = helix_arc.lock().unwrap();
        send_f0_ack(&mut s);
        eprintln!("[MatrixRoutingDd] {label} ok (In25)");
    } else {
        eprintln!(
            "[MatrixRoutingDd] {label} ok sans In25 (timeout {DD_TIMEOUT_MS} ms — commit 1b déjà ACK ed)"
        );
    }
    Ok(())
}

/// Séquence complète split/merge D&D (arm + `1d` + `1b`).
pub fn execute_routing_marker_dd(
    helix_arc: Arc<Mutex<HelixState>>,
    kind: RoutingMarkerKind,
    dest_boundary_col: u8,
) -> Result<String, String> {
    let scope = RoutingDdScope::open(Arc::clone(&helix_arc));

    let (pkt1d, path1_lane, tag_1d) = {
        let mut s = helix_arc.lock().unwrap();
        send_matrix_dd_drag_arm(&mut s)?;
        let (pkt, tag) = build_matrix_routing_marker_move_packet(&mut s, kind, dest_boundary_col)?;
        let lane = [pkt[12], pkt[13], pkt[14]];
        (pkt, lane, tag)
    };

    thread::sleep(Duration::from_millis(DD_DELAY_AFTER_ARM_MS));

    commit_1d_with_handshake(&scope, &helix_arc, pkt1d, "write#1 1d")?;
    eprintln!(
        "[MatrixRoutingDd] write#1 1d ok tag={tag_1d:#04x} lane={:02x}{:02x}{:02x}",
        path1_lane[0], path1_lane[1], path1_lane[2]
    );

    let tag_1b = {
        let mut s = helix_arc.lock().unwrap();
        let pkt =
            build_matrix_routing_path2_commit_packet(&mut s, kind, dest_boundary_col, path1_lane);
        let tag = pkt[28];
        (pkt, tag)
    };
    let (pkt1b, tag_1b) = tag_1b;

    commit_1b_with_handshake(&scope, &helix_arc, pkt1b, kind, "write#2 1b")?;
    eprintln!(
        "[MatrixRoutingDd] write#2 1b ok tag={tag_1b:#04x} col={dest_boundary_col} b34={:#04x} ({})",
        dest_boundary_col.wrapping_add(1),
        kind.label()
    );

    {
        let mut s = helix_arc.lock().unwrap();
        s.hw_active_slot_index = None;
        s.hw_active_slot_bus = Some(kind.slot_bus());
        s.hw_active_slot_sequence = s.hw_active_slot_sequence.wrapping_add(1);
    }

    Ok(format!(
        "routing_dd_{} col->{dest_boundary_col} bus {:#04x}",
        kind.label(),
        kind.slot_bus()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::matrix_routing_move::RoutingMarkerKind;

    #[test]
    fn path2_commit_split_shape() {
        let mut s = HelixState::new();
        s.session_no = 0x70;
        s.editor_ed03_double = 0x64f2;
        let pkt = build_matrix_routing_path2_commit_packet(
            &mut s,
            RoutingMarkerKind::Split,
            0,
            [0x70, 0x2a, 0x00],
        );
        assert_eq!(pkt.len(), 36);
        assert_eq!(pkt[0], 0x1b);
        assert_eq!(pkt[12], 0x81);
        assert_eq!(pkt[20], 0x0b);
        assert_eq!(pkt[27], 0x03);
        assert_eq!(pkt[28], 0xf3);
        assert_eq!(&pkt[29..32], &[0x64, 0x2e, 0x65]);
        assert_eq!(&pkt[32..36], &[0x81, 0x62, 0x01, 0x00]);
    }

    #[test]
    fn path2_commit_merge_shape() {
        let mut s = HelixState::new();
        s.editor_ed03_double = 0x64f2;
        let pkt = build_matrix_routing_path2_commit_packet(
            &mut s,
            RoutingMarkerKind::Merge,
            6,
            [0xe7, 0x2a, 0x00],
        );
        assert_eq!(pkt[27], 0x03);
        assert_eq!(pkt[28], 0xf3);
        assert_eq!(pkt[32], 0x81);
        assert_eq!(pkt[34], 0x07);
        assert_eq!(pkt[30], 0x2f);
    }

    #[test]
    fn path2_commit_split_dest_col_wire_byte() {
        let mut s = HelixState::new();
        s.editor_ed03_double = 0x64f1;
        // splitCol 4 → colonne grille 9 ; `dd_split.json` geste 1 : b[34]=0x05
        let pkt = build_matrix_routing_path2_commit_packet(
            &mut s,
            RoutingMarkerKind::Split,
            4,
            [0x9e, 0x1c, 0x00],
        );
        assert_eq!(pkt[34], 0x05);
        assert_eq!(pkt[30], 0x2e);
    }

    #[test]
    fn in19_in21_in25_matchers() {
        let in19 = [
            0x19, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x13, 0x00, 0x04, 0x86, 0x1d,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04,
            0x12, 0x67, 0x00, 0x68, 0xc0, 0x79, 0x13, 0x6a,
        ];
        assert!(is_routing_dd_in19(&in19));
        let in21 = [
            0x21, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0x36, 0x00, 0x04, 0x09, 0x02,
            0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x11, 0x00, 0x00, 0x00, 0x82, 0x69, 0x27, 0x6a,
            0x84, 0x52, 0x01, 0x44, 0x03, 0x79, 0x13, 0x6a, 0x82, 0x62, 0x0a, 0x1a, 0x00, 0xab,
            0x4e, 0x65,
        ];
        assert!(is_routing_dd_in21(&in21));
        let in25 = [
            0x25, 0x00, 0x00, 0x18, 0xf0, 0x03, 0x02, 0x10, 0x00, 0xdd, 0x00, 0x04, 0x09, 0x02,
            0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x15, 0x00, 0x00, 0x00, 0x82, 0x69, 0x1a, 0x6a,
            0x84, 0x52, 0x00, 0x44, 0x03, 0x79, 0x13, 0x6a, 0x82, 0x62, 0x0a, 0x1a, 0x00, 0xab,
            0x4e, 0x65, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(is_routing_dd_in25(&in25));
        assert!(!is_routing_dd_in21(&in25));
        let dev_ed08 = [
            0x08, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x49, 0x00, 0x08, 0x2b, 0x03,
            0x00, 0x00,
        ];
        assert!(is_routing_dd_device_ed03_ack08(&dev_ed08));
        assert!(is_routing_dd_pipeline_in(&dev_ed08));
        assert!(!is_routing_dd_device_ed03_ack08(&in19));
    }

    #[test]
    fn merge_1b_ack_hi_is_out_byte13_plus_one() {
        let out_1b: [u8; 36] = [
            0x1b, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x4d, 0x00, 0x04, 0xf8, 0x2a,
            0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04,
            0x1a, 0x64, 0x2f, 0x65, 0x81, 0x62, 0x07, 0x00,
        ];
        let ack_lo = out_1b[12].wrapping_add(0x11);
        let ack_hi_merge = out_1b[13].wrapping_add(1);
        assert_eq!(ack_lo, 0x09);
        assert_eq!(ack_hi_merge, 0x2b);
        assert_eq!(out_1b[13].wrapping_add(0), 0x2a);
    }
}
