//! Machine a etats phase 4 — version passive (etape 2).
//! Aucun paquet OUT envoye. Logs seulement pour valider la sequence IN.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase4Step {
    #[default]
    Idle,
    // Phase 4 bootstrap
    Waiting92o,
    Waiting1fA,
    Waiting68o,
    WaitingDump,
    Waiting1fB,
    // Dialogue post-1a (étape 3A — passif)
    PostArm,
    WaitAck2,
    WaitIn1f,
    WaitIn1b26,
    WaitPresetAck,
    // Terminal
    Done,
}

impl Phase4Step {
    pub fn is_active(self) -> bool {
        !matches!(self, Phase4Step::Idle | Phase4Step::Done)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Waiting92o => "Waiting92o",
            Self::Waiting1fA => "Waiting1fA",
            Self::Waiting68o => "Waiting68o",
            Self::WaitingDump => "WaitingDump",
            Self::Waiting1fB => "Waiting1fB",
            Self::PostArm => "PostArm",
            Self::WaitAck2 => "WaitAck2",
            Self::WaitIn1f => "WaitIn1f",
            Self::WaitIn1b26 => "WaitIn1b26",
            Self::WaitPresetAck => "WaitPresetAck",
            Self::Done => "Done",
        }
    }
}

/// Appele depuis usb_listener juste avant le check trailer.
/// Version passive v2 : gere les deux chemins (HX Edit Windows et Linux/Stomp XL).
pub fn handle_in_passive(step: &mut Phase4Step, data: &[u8]) {
    if !step.is_active() { return; }

    let prev = *step;
    let len = data.len();
    let h = data.first().copied().unwrap_or(0);
    let ep = data.get(4..8).unwrap_or(&[]);
    let sub = data.get(11).copied().unwrap_or(0);

    // Exclure keepalive sub=10/00.
    let is_keepalive = len == 16
        && data.starts_with(&[0x08, 0x00, 0x00, 0x18])
        && (sub == 0x10 || sub == 0x00);

    let next: Option<Phase4Step> = match *step {
        Phase4Step::Waiting92o => {
            if !is_keepalive
                && (80..=116).contains(&len)
                && matches!(h, 0x54 | 0x53 | 0x4e | 0x4f | 0x55)
                && ep.first().copied() == Some(0xed)
            {
                Some(Phase4Step::Waiting1fA)
            } else {
                None
            }
        }
        Phase4Step::Waiting1fA => {
            if len == 40
                && h == 0x1f
                && ep.get(0).copied() == Some(0xed)
                && ep.get(1).copied() == Some(0x03)
            {
                Some(Phase4Step::Waiting68o)
            } else if len == 68
                && matches!(h, 0x39 | 0x3c)
                && ep.first().copied() == Some(0xed)
            {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[phase4_fsm] Waiting1fA — 68o direct (Linux) head={:02x}",
                    h
                ));
                Some(Phase4Step::WaitingDump)
            } else {
                None
            }
        }
        Phase4Step::Waiting68o => {
            if len == 68
                && matches!(h, 0x39 | 0x3c)
                && ep.first().copied() == Some(0xed)
            {
                Some(Phase4Step::WaitingDump)
            } else {
                None
            }
        }
        Phase4Step::WaitingDump => {
            if (len == 132 && h == 0x7a || len == 116 && h == 0x6a)
                && ep.first().copied() == Some(0xed)
            {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[phase4_fsm] trailer {}o head={:02x} → PostArm",
                    len,
                    h
                ));
                Some(Phase4Step::PostArm)
            } else if len == 216 && h == 0xcf && ep.first().copied() == Some(0xed) {
                Some(Phase4Step::Waiting1fB)
            } else {
                None
            }
        }
        Phase4Step::Waiting1fB => {
            if len == 40
                && h == 0x1f
                && ep.get(0).copied() == Some(0xed)
                && ep.get(1).copied() == Some(0x03)
            {
                Some(Phase4Step::PostArm)
            } else {
                None
            }
        }
        Phase4Step::PostArm => {
            if is_keepalive {
                return;
            }
            if len == 36 && h == 0x19
                && ep.get(0).copied() == Some(0xef)
                && ep.get(1).copied() == Some(0x03)
            {
                let dbl = (
                    data.get(28).copied().unwrap_or(0),
                    data.get(29).copied().unwrap_or(0),
                );
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[post1a] PostArm -> WaitAck2 (IN 19/36o ef double={:02x}:{:02x})",
                    dbl.0, dbl.1
                ));
                Some(Phase4Step::WaitAck2)
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::WaitAck2 => {
            if is_keepalive {
                return;
            }
            if len == 16 && data.starts_with(&[0x08, 0x00, 0x00, 0x18]) && sub == 0x08 {
                let lane = if data.len() >= 14 {
                    let lo = data[12];
                    let hi = data[13];
                    format!("{:02x}{:02x}", hi, lo)
                } else {
                    "????".to_string()
                };
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[post1a] WaitAck2 - ACK sub=08 ep={:02x}{:02x} lane={}",
                    ep.first().copied().unwrap_or(0),
                    ep.get(1).copied().unwrap_or(0),
                    lane
                ));
                if ep.first().copied() == Some(0xed) {
                    crate::helix::init_trace::trace("[post1a] WaitAck2 -> WaitIn1f (2 ACK reçus)");
                    Some(Phase4Step::WaitIn1f)
                } else {
                    None
                }
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::WaitIn1f => {
            if is_keepalive {
                return;
            }
            if len == 40 && h == 0x1f && ep.first().copied() == Some(0xed) {
                let dbl = (
                    data.get(28).copied().unwrap_or(0),
                    data.get(29).copied().unwrap_or(0),
                );
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[post1a] WaitIn1f -> WaitIn1b26 (IN 1f/40o ed double={:02x}:{:02x})",
                    dbl.0, dbl.1
                ));
                Some(Phase4Step::WaitIn1b26)
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::WaitIn1b26 => {
            if is_keepalive {
                return;
            }
            // Chemin HX Edit Windows : 1b/36o ed + 26/48o ef
            if len == 36 && h == 0x1b && ep.first().copied() == Some(0xed) {
                crate::helix::init_trace::trace("[post1a] WaitIn1b26 — 1b/36o ed (1/2)");
                None
            } else if len == 48 && h == 0x26 && ep.first().copied() == Some(0xef) {
                crate::helix::init_trace::trace(
                    "[post1a] WaitIn1b26 → WaitPresetAck (26/48o ef)",
                );
                Some(Phase4Step::WaitPresetAck)
            // Chemin Linux : 2× 68o head=3c ou 39 (ef puis ed)
            } else if len == 68 && h == 0x3c && ep.first().copied() == Some(0xef) {
                crate::helix::init_trace::trace("[post1a] WaitIn1b26 — 68o ef head=3c (1/2 Linux)");
                None
            } else if len == 68 && h == 0x3c && ep.first().copied() == Some(0xed) {
                crate::helix::init_trace::trace(
                    "[post1a] WaitIn1b26 → WaitPresetAck (68o ed head=3c Linux)",
                );
                Some(Phase4Step::WaitPresetAck)
            } else if len == 68 && h == 0x39 && ep.first().copied() == Some(0xef) {
                crate::helix::init_trace::trace("[post1a] WaitIn1b26 — 68o ef head=39 (1/2 Linux)");
                None
            } else if len == 68 && h == 0x39 && ep.first().copied() == Some(0xed) {
                crate::helix::init_trace::trace(
                    "[post1a] WaitIn1b26 → WaitPresetAck (68o ed head=39 Linux)",
                );
                Some(Phase4Step::WaitPresetAck)
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::WaitPresetAck => {
            if is_keepalive {
                return;
            }
            if len == 16
                && data.starts_with(&[0x08, 0x00, 0x00, 0x18])
                && sub == 0x08
                && ep.first().copied() == Some(0xef)
            {
                let lane = if data.len() >= 14 {
                    let lo = data[12];
                    let hi = data[13];
                    format!("{:02x}{:02x}", hi, lo)
                } else {
                    "????".to_string()
                };
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[post1a] WaitPresetAck -> Done (ACK ef sub=08 lane={})",
                    lane
                ));
                Some(Phase4Step::Done)
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::Idle | Phase4Step::Done => None,
    };

    if let Some(next_step) = next {
        *step = next_step;
        if !matches!(
            next_step,
            Phase4Step::PostArm
                | Phase4Step::WaitAck2
                | Phase4Step::WaitIn1f
                | Phase4Step::WaitIn1b26
                | Phase4Step::WaitPresetAck
        ) {
            crate::helix::init_trace::trace_fmt(format_args!(
                "[phase4_fsm] {} -> {} (IN len={} head={:02x})",
                prev.label(),
                next_step.label(),
                len,
                h
            ));
        }
    } else {
        if matches!(
            *step,
            Phase4Step::Waiting92o | Phase4Step::Waiting1fA | Phase4Step::WaitingDump
        ) && ep.first().copied() == Some(0xed)
            && len >= 16
            && !is_keepalive
        {
            if *step == Phase4Step::Waiting1fA {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[phase4_fsm] Waiting1fA — IN ignoré len={} head={:02x}",
                    len,
                    h
                ));
            } else {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[phase4_fsm] {} - IN ed ignore len={} head={:02x}",
                    step.label(),
                    len,
                    h
                ));
            }
        }
    }
}

fn log_ignored(step: &Phase4Step, len: usize, h: u8, ep: &[u8]) {
    crate::helix::init_trace::trace_fmt(format_args!(
        "[post1a] {} - IN ignoré len={} head={:02x} ep={:02x}{:02x}",
        step.label(),
        len,
        h,
        ep.first().copied().unwrap_or(0),
        ep.get(1).copied().unwrap_or(0),
    ));
}

/// Arme la FSM au demarrage de la phase 4.
pub fn arm(step: &mut Phase4Step) {
    *step = Phase4Step::Waiting92o;
    crate::helix::init_trace::trace("[phase4_fsm] armee -> Waiting92o");
}

pub fn on_enter_post_arm(state: &mut crate::helix::HelixState) {
    let cnt = state.next_x1_cnt();
    state.send(crate::helix::packet::OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt, 0x00, 0x08,
        0x1a, 0x10, 0x00, 0x00,
    ]));
    crate::helix::init_trace::trace_fmt(format_args!(
        "[post1a] ARM_ef envoyé (3B) cnt={:02x} lane=1a10",
        cnt
    ));
}

pub fn on_enter_wait_in_1f(state: &mut crate::helix::HelixState) {
    let cnt = state.next_x80_cnt();
    let before = state.editor_ed03_double;
    let d = state.next_editor_ed03_double();
    crate::helix::init_trace::trace_fmt(format_args!(
        "[post1a] 1b OUT ed envoyé (3C) cnt={:02x} double={:02x}:{:02x} (editor_ed03_double avant={:#06x})",
        cnt,
        d[0],
        d[1],
        before
    ));
    state.send(crate::helix::packet::OutPacket::new(vec![
        0x1b, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt, 0x00, 0x0c,
        0x64, 0x1c, 0x00, 0x00,
        0x01, 0x00, 0x06, 0x00,
        0x0b, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        d[0], d[1],
        0x18, 0x65, 0x81, 0x76, 0x0e, 0x00,
    ]));
}

pub fn on_enter_wait_in_1b26(state: &mut crate::helix::HelixState) {
    let cnt1 = state.next_x80_cnt();
    let d1 = state.next_editor_ed03_double();
    state.send(crate::helix::packet::OutPacket::new(vec![
        0x19, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt1, 0x00, 0x0c,
        0x6c, 0x10, 0x00, 0x00,
        0x01, 0x00, 0x06, 0x00,
        0x09, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        d1[0], d1[1],
        0x17, 0x65, 0xc0, 0x00, 0x00, 0x00,
    ]));

    let cnt2 = state.next_x1_cnt();
    let d2 = state.next_editor_ed03_double();
    state.send(crate::helix::packet::OutPacket::new(vec![
        0x19, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt2, 0x00, 0x04,
        0x09, 0x10, 0x00, 0x00,
        0x01, 0x00, 0x06, 0x00,
        0x09, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        d2[0], d2[1],
        0x17, 0x65, 0xc0, 0x00, 0x00, 0x00,
    ]));

    crate::helix::init_trace::trace_fmt(format_args!(
        "[post1a] 19 OUT ed+ef envoyés (3D) cnt={:02x}/{:02x} double={:02x}:{:02x}/{:02x}:{:02x}",
        cnt1, cnt2, d1[0], d1[1], d2[0], d2[1]
    ));
}
