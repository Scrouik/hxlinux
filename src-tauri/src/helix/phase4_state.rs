//! Machine à états phase 4 + dialogue éditeur post-trailer (PHASE B).
//!
//! Phase 4 (passive) : Waiting92o → … → WaitingDump → trailer → PostArm.
//! PHASE B (réactive, dialogue requête→réponse, fidèle HX `stomp_running`) :
//!   PostArm(=ARM_ef + 1b 76:0e) → PbWait49(=1b 76:49) → PbWaitCc(=1c 76:cc:88)
//!     → PbWait1a(=1a ef) → PbWait1b(=1b 76:1b) → WaitIn1b26(=19 ed+ef) → Done
//!
//! DÉCLENCHEMENT PROACTIF : à l'entrée de `PostArm` (trailer reçu), le host
//! envoie le 1er `1b 76:0e` sans attendre d'`IN 19` (HX fait pareil ~20 ms
//! après le trailer). Chaque requête suivante part à réception de l'`IN 1f`
//! (ou `IN 19 ef` pour le 1a). Les `IN 1d`/ACK 08 entrelacés sont ignorés.
//!
//! Octets 12-13 = `editor_ed03_lane` (continuité depuis 9d, +0x17/commande —
//! option A : valeur absolue ≠ HX mais device tolérant). Doubles cd:03
//! = `editor_ed03_double` (ec→f0). Le `1a` réutilise e8:64 (lane fixe 09:10),
//! le `19 ef` final réutilise e9:64 (lane fixe 1a:10).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase4Step {
    #[default]
    Idle,
    // Phase 4 bootstrap (passif)
    Waiting92o,
    Waiting1fA,
    Waiting68o,
    WaitingDump,
    Waiting1fB,
    // PHASE B (dialogue éditeur post-trailer, proactif)
    PostArm,    // on_enter: ARM_ef + 1b 76:0e (ec) ; attend IN 1f
    PbWait49,   // on_enter: 1b 76:49 (ed)          ; attend IN 1f
    PbWaitCc,   // on_enter: 1c 76:cc:88 (ee)       ; attend IN 1f
    PbWait1a,   // on_enter: 1a ef                  ; attend IN 19 ef
    PbWait1b,   // on_enter: 1b 76:1b (ef')         ; attend IN 1f
    WaitIn1b26, // on_enter: 19 ed + 19 ef (f0/e9)  ; attend IN 1b + IN 26
    // Terminal
    Done,
}

impl Phase4Step {
    pub fn is_active(self) -> bool {
        !matches!(self, Phase4Step::Idle | Phase4Step::Done)
    }

    /// `true` pendant la PHASE B (post-trailer) : utilisé par `amorcage` pour
    /// ne pas lancer `RequestPresetNames` tant que le dialogue éditeur tourne.
    pub fn is_phase_b(self) -> bool {
        matches!(
            self,
            Phase4Step::PostArm
                | Phase4Step::PbWait49
                | Phase4Step::PbWaitCc
                | Phase4Step::PbWait1a
                | Phase4Step::PbWait1b
                | Phase4Step::WaitIn1b26
        )
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
            Self::PbWait49 => "PbWait49",
            Self::PbWaitCc => "PbWaitCc",
            Self::PbWait1a => "PbWait1a",
            Self::PbWait1b => "PbWait1b",
            Self::WaitIn1b26 => "WaitIn1b26",
            Self::Done => "Done",
        }
    }
}

/// Réponse IN 1f/40o sur lane ed (réponse à une requête 76).
fn is_in_1f_ed(data: &[u8]) -> bool {
    data.len() == 40
        && data.first() == Some(&0x1f)
        && data.get(4..8) == Some(&[0xed, 0x03, 0x80, 0x10])
}

/// Réponse IN 19/36o sur lane ef (réponse au 1a).
fn is_in_19_ef(data: &[u8]) -> bool {
    data.len() == 36
        && data.first() == Some(&0x19)
        && data.get(4).copied() == Some(0xef)
        && data.get(5).copied() == Some(0x03)
}

/// Appelé depuis usb_listener juste avant le check trailer.
pub fn handle_in_passive(state: &mut crate::helix::HelixState, data: &[u8]) {
    let step = &mut state.phase4_step;
    if !step.is_active() {
        return;
    }

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
            // Le « préambule » qui suit l'IN 1f précède immédiatement le dump.
            // Sa TAILLE et son HEAD varient selon le preset actif :
            //   classique : 68o head=39|3c
            //   snapshot  : 68o head=3b (un run), 72o head=3e (run snapshot XL), …
            // On le reconnaît donc par sa NATURE — chunk ed PARTIEL (sub=0x04,
            // 17≤len<272), hors keepalive — exactement comme le trailer en
            // WaitingDump, JAMAIS par une liste de head/len en dur (sinon
            // intermittence par preset, même piège que l'ancien trailer figé).
            //
            // Ici aucune ambiguïté préambule/trailer : ils sont structurellement
            // identiques (partiel ed/sub04) mais distingués par la POSITION —
            // le préambule arrive en Waiting68o (avant tout chunk 272), le trailer
            // en WaitingDump (après les 272). C'est tout le rôle de la FSM.
            //
            // Filet de sécurité : si le préambule a une forme inattendue (ni 68/72o
            // ni partiel ed) mais que le 1er VRAI chunk 272 (08:01) arrive, on
            // bascule quand même — le device a commencé le dump, il FAUT être en
            // WaitingDump pour capter le trailer final. Le chunk 272 est sans
            // équivoque (il ne peut pas être confondu avec le trailer, partiel) et
            // partage sa définition avec la couche ACK (preset_dump_stream_ack).
            if is_keepalive {
                None
            } else if ep.first().copied() == Some(0xed)
                && sub == 0x04
                && (17..272).contains(&len)
            {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[phase4_fsm] Waiting68o — préambule {}o head={:02x} (chunk partiel ed) → WaitingDump",
                    len,
                    h
                ));
                Some(Phase4Step::WaitingDump)
            } else if crate::helix::preset_dump_stream_ack::is_preset_dump_stream_chunk_in(data) {
                crate::helix::init_trace::trace(
                    "[phase4_fsm] Waiting68o — 1er chunk dump 272 vu (préambule manqué ?) → WaitingDump",
                );
                Some(Phase4Step::WaitingDump)
            } else {
                None
            }
        }
        Phase4Step::WaitingDump => {
            // Variante deux-étages (216o cf → 1f → PostArm) : inchangée, testée
            // AVANT la règle générique pour ne pas l'absorber.
            if len == 216 && h == 0xcf && ep.first().copied() == Some(0xed) {
                Some(Phase4Step::Waiting1fB)
            // Fin de dump = dernier chunk PARTIEL du flux (sub=0x04, 16 < len < 272),
            // quelle que soit sa taille : couvre 132/7a, 116/6a, 140/84, 28/14, … La
            // taille du chunk final dépend du preset actif — c'est ce qui codait
            // l'intermittence quand on listait les longueurs en dur. Ici aucun risque
            // de faux positif sur le préambule (92o/68o/72o, mêmes sub=04) : on est
            // DÉJÀ passé en WaitingDump, donc seuls les chunks (272 puis le partiel
            // terminal) arrivent encore.
            } else if ep.first().copied() == Some(0xed)
                && sub == 0x04
                && (17..272).contains(&len)
            {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[phase4_fsm] trailer {}o head={:02x} (chunk partiel) → PostArm (PHASE B proactive)",
                    len,
                    h
                ));
                state.phase4_dump_full_272_count = 0;
                Some(Phase4Step::PostArm)
            } else if crate::helix::preset_dump_stream_ack::is_preset_dump_stream_chunk_in(data)
                && len == 272
            {
                state.phase4_dump_full_272_count =
                    state.phase4_dump_full_272_count.saturating_add(1);
                None
            } else if state.phase4_dump_full_272_count > 0
                && crate::helix::preset_dump_stream_ack::is_preset_dump_stream_ack_echo_in(data)
            {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[phase4_fsm] fin dump (écho ACK sub=08 après {}×272o) → PostArm",
                    state.phase4_dump_full_272_count
                ));
                state.phase4_dump_full_272_count = 0;
                Some(Phase4Step::PostArm)
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
        // ── PHASE B : chaîne requête→réponse. PostArm a déjà émis le 1b 76:0e
        //    (on_enter_post_arm) ; on attend ici l'IN 1f de réponse. ───────────
        Phase4Step::PostArm => {
            if is_keepalive {
                return;
            }
            if is_in_1f_ed(data) {
                crate::helix::init_trace::trace("[PhaseB] PostArm -> PbWait49 (IN 1f, rép. 76:0e)");
                Some(Phase4Step::PbWait49)
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::PbWait49 => {
            if is_keepalive {
                return;
            }
            if is_in_1f_ed(data) {
                crate::helix::init_trace::trace("[PhaseB] PbWait49 -> PbWaitCc (IN 1f, rép. 76:49)");
                Some(Phase4Step::PbWaitCc)
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::PbWaitCc => {
            if is_keepalive {
                return;
            }
            if is_in_1f_ed(data) {
                crate::helix::init_trace::trace("[PhaseB] PbWaitCc -> PbWait1a (IN 1f, rép. 76:cc)");
                Some(Phase4Step::PbWait1a)
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::PbWait1a => {
            if is_keepalive {
                return;
            }
            if is_in_19_ef(data) {
                crate::helix::init_trace::trace("[PhaseB] PbWait1a -> PbWait1b (IN 19 ef, rép. 1a)");
                Some(Phase4Step::PbWait1b)
            } else {
                log_ignored(step, len, h, ep);
                None
            }
        }
        Phase4Step::PbWait1b => {
            if is_keepalive {
                return;
            }
            if is_in_1f_ed(data) {
                crate::helix::init_trace::trace("[PhaseB] PbWait1b -> WaitIn1b26 (IN 1f, rép. 76:1b)");
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
            // Chemin HX Edit : 1b/36o ed (1/2, log) puis 26/48o ef → Done.
            if len == 36 && h == 0x1b && ep.first().copied() == Some(0xed) {
                crate::helix::init_trace::trace("[PhaseB] WaitIn1b26 — IN 1b/36o ed (1/2)");
                None
            } else if len == 48 && h == 0x26 && ep.first().copied() == Some(0xef) {
                crate::helix::init_trace::trace("[PhaseB] WaitIn1b26 -> Done (IN 26/48o ef)");
                Some(Phase4Step::Done)
            // Chemin Linux : 2× 68o head=3c|39 (ef puis ed) → Done sur le 2ᵉ.
            // NOTE (vigilance snapshot) : même fragilité de head/len en dur qu'ex-
            // Waiting68o. Si un preset snapshot répond ici avec une autre forme
            // (ex. 72o/3e), la PHASE B calera. À traiter de la même façon
            // (reconnaissance structurelle) UNE FOIS la forme observée en capture —
            // ce run n'a jamais atteint la PHASE B, on ne devine pas.
            } else if len == 68 && matches!(h, 0x3c | 0x39) && ep.first().copied() == Some(0xef) {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[PhaseB] WaitIn1b26 — IN 68o ef head={:02x} (1/2 Linux)",
                    h
                ));
                None
            } else if len == 68 && matches!(h, 0x3c | 0x39) && ep.first().copied() == Some(0xed) {
                crate::helix::init_trace::trace_fmt(format_args!(
                    "[PhaseB] WaitIn1b26 -> Done (IN 68o ed head={:02x} Linux)",
                    h
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
        // Les transitions PHASE B sont déjà tracées ci-dessus ; on loggue ici
        // seulement les transitions phase 4 passives.
        if !next_step.is_phase_b() && next_step != Phase4Step::Done {
            crate::helix::init_trace::trace_fmt(format_args!(
                "[phase4_fsm] {} -> {} (IN len={} head={:02x})",
                prev.label(),
                next_step.label(),
                len,
                h
            ));
        }
    } else if matches!(
        *step,
        Phase4Step::Waiting92o
            | Phase4Step::Waiting1fA
            | Phase4Step::Waiting68o
            | Phase4Step::WaitingDump
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
                "[phase4_fsm] {} - IN ed ignore len={} head={:02x} sub={:02x}",
                step.label(),
                len,
                h,
                sub
            ));
        }
    }
}

fn log_ignored(step: &Phase4Step, len: usize, h: u8, ep: &[u8]) {
    crate::helix::init_trace::trace_fmt(format_args!(
        "[PhaseB] {} - IN ignoré len={} head={:02x} ep={:02x}{:02x}",
        step.label(),
        len,
        h,
        ep.first().copied().unwrap_or(0),
        ep.get(1).copied().unwrap_or(0),
    ));
}

/// Arme la FSM au démarrage de la phase 4.
pub fn arm(step: &mut Phase4Step) {
    *step = Phase4Step::Waiting92o;
    crate::helix::init_trace::trace("[phase4_fsm] armée -> Waiting92o");
}

// ============================================================================
// on_enter_* : OUT émis À L'ENTRÉE de chaque état PHASE B (appelés par usb_listener)
// ============================================================================

/// Construit un OUT requête 76 sur lane ed (`1b` ou `1c`) :
/// - octets 12-13 = `editor_ed03_lane` courant (puis +0x17),
/// - double cd:03 = `editor_ed03_double` courant (puis +1),
/// - queue `18 65 81 76 <id> <arg>`.
fn send_phase_b_76(
    state: &mut crate::helix::HelixState,
    head: u8,
    sub: u8,
    last_field: u8, // octet 20 : 0x0b (1b) ou 0x0c (1c)
    id76: u8,
    arg76: u8,
    label: &str,
) {
    let cnt = state.next_x80_cnt();
    let lane = state.advance_editor_ed03_lane_lo(crate::helix::HelixState::EDITOR_ED03_LANE_CMD_DELTA);
    let d = state.next_editor_ed03_double();
    state.send(crate::helix::packet::OutPacket::new(vec![
        head, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt, 0x00, sub,
        lane[0], lane[1], 0x00, 0x00,
        0x01, 0x00, 0x06, 0x00,
        last_field, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        d[0], d[1], 0x18, 0x65,
        0x81, 0x76, id76, arg76,
    ]));
    crate::helix::init_trace::trace_fmt(format_args!(
        "[PhaseB] {} cnt={:02x} lane={:02x}:{:02x} double={:02x}:{:02x} (76:{:02x}:{:02x})",
        label, cnt, lane[0], lane[1], d[0], d[1], id76, arg76
    ));
}

/// Entrée PostArm (trailer reçu) : ARM_ef puis le 1er `1b 76:0e` (ec) proactif.
/// Le `1b` part avec un léger délai writer (≈ HX : ~15 ms après les ACK), sans
/// bloquer le thread listener (delay porté par le writer).
pub fn on_enter_post_arm(state: &mut crate::helix::HelixState) {
    // Fin de dump atteinte (trailer = chunk partiel, détecté par la FSM en WaitingDump) :
    // on signale ICI la complétion phase 4, depuis la FSM (stateful, déjà passé le
    // préambule), au lieu de dépendre du détecteur stateless is_phase4_bootstrap_trailer_in
    // (qui ne connaissait que 7a/132 et 6a/116 et timeoutait à 3500 ms sur les autres
    // tailles, ex. 84/140 → settle forcé, éditeur jamais « vivant », presets non lus).
    // Idempotent : no-op si phase4_bootstrap_active est déjà false.
    state.note_phase4_bootstrap_complete();

    // ARM_ef (08 ef sub=08 lane=1a10) — inchangé.
    let cnt = state.next_x1_cnt();
    state.send(crate::helix::packet::OutPacket::new(vec![
        0x08, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt, 0x00, 0x08,
        0x1a, 0x10, 0x00, 0x00,
    ]));
    crate::helix::init_trace::trace_fmt(format_args!(
        "[PhaseB] PostArm on_enter — ARM_ef cnt={:02x} lane=1a10 puis 1b 76:0e",
        cnt
    ));

    // Transition dump → PHASE B : la 1re commande PHASE B est la transaction ed03
    // qui SUIT le dernier ACK chunk. Comme chaque ACK chunk a fait +1 sur le HI
    // (preset_dump_stream_ack, gardé phase4_bootstrap_active : 10 → … → 1b sur 11
    // chunks), ce +1 supplémentaire amène le HI à 1c — exactement comme HX
    // (ec à …:1c). Ce n'est pas un correctif : c'est le cran de HI de la
    // transaction suivante, identique au +1 de chaque ACK.
    let _ = state.advance_editor_ed03_lane_hi();

    // 1b 76:0e (ec) — 1re requête PHASE B, proactive (~15 ms de délai writer).
    let cnt2 = state.next_x80_cnt();
    let lane = state.advance_editor_ed03_lane_lo(crate::helix::HelixState::EDITOR_ED03_LANE_CMD_DELTA);
    let d = state.next_editor_ed03_double();
    state.send(crate::helix::packet::OutPacket::with_delay(
        vec![
            0x1b, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, cnt2, 0x00, 0x04,
            lane[0], lane[1], 0x00, 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x0b, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            d[0], d[1], 0x18, 0x65,
            0x81, 0x76, 0x0e, 0x00,
        ],
        15,
    ));
    crate::helix::init_trace::trace_fmt(format_args!(
        "[PhaseB] ec 1b 76:0e cnt={:02x} lane={:02x}:{:02x} double={:02x}:{:02x}",
        cnt2, lane[0], lane[1], d[0], d[1]
    ));
}

/// ed — 1b 76:49 (sub=0c).
pub fn on_enter_pb_wait49(state: &mut crate::helix::HelixState) {
    send_phase_b_76(state, 0x1b, 0x0c, 0x0b, 0x49, 0x00, "ed 1b 76:49");
}

/// ee — 1c 76:cc:88 (sub=0c).
pub fn on_enter_pb_waitcc(state: &mut crate::helix::HelixState) {
    send_phase_b_76(state, 0x1c, 0x0c, 0x0c, 0xcc, 0x88, "ee 1c 76:cc:88");
}

/// 1a ef — réutilise e8:64, lane fixe 09:10 (octets 12-13 NON tirés du compteur),
/// payload `cc fe 65 80`. Émis après le 1c ee, comme HX.
pub fn on_enter_pb_wait1a(state: &mut crate::helix::HelixState) {
    let cnt = state.next_x1_cnt();
    // Le 1a porte TOUJOURS e8:64 (valeur figée HX), pas le compteur courant.
    const E8: u16 = crate::helix::HelixState::PRESET_ED03_TRANSACTION_FIRST; // 0x64e8
    let d_e8: [u8; 2] = [(E8 & 0xff) as u8, ((E8 >> 8) & 0xff) as u8];
    state.send(crate::helix::packet::OutPacket::new(vec![
        0x1a, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt, 0x00, 0x04,
        0x09, 0x10, 0x00, 0x00,
        0x01, 0x00, 0x02, 0x00,
        0x0a, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        d_e8[0], d_e8[1], 0xcc, 0xfe,
        0x65, 0x80, 0x00, 0x00,
    ]));
    crate::helix::init_trace::trace_fmt(format_args!(
        "[PhaseB] 1a ef cnt={:02x} double={:02x}:{:02x} payload=cc:fe:65:80",
        cnt, d_e8[0], d_e8[1]
    ));
}

/// ef' — 1b 76:1b (sub=04).
pub fn on_enter_pb_wait1b(state: &mut crate::helix::HelixState) {
    send_phase_b_76(state, 0x1b, 0x04, 0x0b, 0x1b, 0x00, "ef 1b 76:1b");
}

/// f0 — finalisation : 19 ed (lane, double f0) + 19 ef (lane fixe 1a:10, double e9).
pub fn on_enter_wait_in_1b26(state: &mut crate::helix::HelixState) {
    // 19 ed — double f0:64, octets 12-13 = compteur lane (+0x17), payload 63 65 80.
    let cnt1 = state.next_x80_cnt();
    let lane1 = state.advance_editor_ed03_lane_lo(crate::helix::HelixState::EDITOR_ED03_LANE_CMD_DELTA);
    let d1 = state.next_editor_ed03_double();
    state.send(crate::helix::packet::OutPacket::new(vec![
        0x19, 0x00, 0x00, 0x18,
        0x80, 0x10, 0xed, 0x03,
        0x00, cnt1, 0x00, 0x0c,
        lane1[0], lane1[1], 0x00, 0x00,
        0x01, 0x00, 0x06, 0x00,
        0x09, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        d1[0], d1[1],
        0x63, 0x65, 0x80, 0x00, 0x00, 0x00,
    ]));

    // 19 ef — double e9:64 (réutilise e9), lane fixe 1a:10, payload 00 65 c0.
    let cnt2 = state.next_x1_cnt();
    const E9: u16 = 0x64e9;
    let d2: [u8; 2] = [(E9 & 0xff) as u8, ((E9 >> 8) & 0xff) as u8];
    state.send(crate::helix::packet::OutPacket::new(vec![
        0x19, 0x00, 0x00, 0x18,
        0x01, 0x10, 0xef, 0x03,
        0x00, cnt2, 0x00, 0x04,
        0x1a, 0x10, 0x00, 0x00,
        0x01, 0x00, 0x06, 0x00,
        0x09, 0x00, 0x00, 0x00,
        0x83, 0x66, 0xcd, 0x03,
        d2[0], d2[1],
        0x00, 0x65, 0xc0, 0x00, 0x00, 0x00,
    ]));

    crate::helix::init_trace::trace_fmt(format_args!(
        "[PhaseB] finalisation 19 ed+ef cnt={:02x}/{:02x} lane_ed={:02x}:{:02x} dbl_ed={:02x}:{:02x} dbl_ef={:02x}:{:02x}",
        cnt1, cnt2, lane1[0], lane1[1], d1[0], d1[1], d2[0], d2[1]
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::HelixState;

    #[test]
    fn waiting_dump_ends_on_ack_echo_after_full_272_rafale() {
        let mut state = HelixState::new();
        state.phase4_dump_full_272_count = 2;
        let echo = [
            0x08, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x13, 0x00, 0x08, 0x50, 0x02,
            0x00, 0x00,
        ];
        state.phase4_step = Phase4Step::WaitingDump;
        handle_in_passive(&mut state, &echo);
        assert_eq!(state.phase4_step, Phase4Step::PostArm);
        assert_eq!(state.phase4_dump_full_272_count, 0);
    }
}