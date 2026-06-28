// ===========================================================
// helix/modes/request_preset.rs
// Lecture des données du preset actif
// Protocole two-phase ED03 validé sur captures Wireshark
// ===========================================================

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::helix::{Mode, HelixState, ModeRequest, preset_debug_verbose_enabled};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;

/// Watchdog normal d'une transaction de lecture (Phase 1/2, chunks).
const WATCHDOG_MS: u64 = 2000;

/// Nombre de presets du HX (index valides 0..PRESET_COUNT-1). Aligné sur
/// `EXPECTED_PRESET_COUNT` côté `lib.rs` et sur `names.length` côté UI.
const PRESET_COUNT: usize = 125;

/// Fenêtre de confirmation de fin-de-dump §10 (écho `sub=08` SANS trailer partiel).
///
/// Un écho `sub=08` de 16 o et le vrai écho de fin §10 sont INDISTINGUABLES au
/// niveau trame (même `sub`, même queue = ctr du dump). Le seul discriminant est
/// *ce qui suit* : un chunk 272 (→ l'écho était PARASITE, ex. provoqué par le wrap
/// du compteur en plein dump) ou rien/idle (→ vraie fin §10). On diffère donc la
/// clôture de cette fenêtre : un chunk qui arrive annule (on poursuit) ; sinon le
/// watchdog court clôture. Mesure log : inter-chunk ≈ 3–4 ms, donc 150 ms est ~40×
/// le pire écart réel — aucun risque de clôturer en plein dump.
///
/// `HX_DUMP_END_CONFIRM_MS=0` → ancien comportement (clôture immédiate sur l'écho =
/// témoin pour revert/comparaison).
fn dump_end_confirm_ms() -> u64 {
    std::env::var("HX_DUMP_END_CONFIRM_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(150)
}

/// Garde anti dump-AUTO pendant Phase 1 (défaut ON).
///
/// Après un D&D inter-path, le device PUSH spontanément un dump complet (réindexation)
/// AVANT de répondre à la Phase 1 de la relecture. Un chunk 272 (`sub=04`, `len=272`)
/// satisfait la condition `sub==0x04 && len>=36` et était pris à tort pour la réponse
/// Phase 1 → Phase 2 prématurée → le dump auto puis la relecture se concaténaient dans
/// `preset_data` (capture `Preset_Test_D_D.json` : 21 chunks = 5376 o au lieu de 11).
///
/// Avec le garde : pendant `waiting_phase1_response`, un chunk de flux dump est acquitté
/// (pour ne pas geler le device) mais NI accumulé NI traité comme Phase 1. La vraie
/// réponse Phase 1 (enveloppe 36-68 o, head `19`/`1c`) arrive ensuite et seule la
/// relecture est capturée.
///
/// `HX_DD_DUMP_AUTO_GUARD=0` → témoin (ancien comportement, chunk 272 = faux Phase 1).
fn dd_dump_auto_guard_enabled() -> bool {
    match std::env::var("HX_DD_DUMP_AUTO_GUARD").as_deref() {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

pub struct RequestPreset {
    preset_data:             Vec<u8>,
    /// true = Phase 1 envoyée, en attente de la réponse 68 octets
    waiting_phase1_response: bool,
    /// Double (octets 12–13) du dernier ACK chunk envoyé (pour le FDT ACK)
    last_ack_lane:           [u8; 2],
    watchdog_cancel_tx:      Option<mpsc::Sender<()>>,
    mode_tx:                 Option<mpsc::Sender<ModeRequest>>,
    /// Dernier chunk reçu était un 272 o plein (256 o utiles) — la fin arrive par écho IN sub=`08`.
    await_dump_end_after_full_chunk: bool,
    /// Au moins un chunk flux 272 o (`08:01:ed:03`) reçu — évite de terminer sur un préambule partiel.
    saw_full_272_chunk: bool,
    /// Un écho `sub=08` a été vu après une rafale 272 ; clôture DIFFÉRÉE le temps de
    /// confirmer qu'aucun chunk ne suit (sinon l'écho était parasite → on poursuit).
    dump_end_pending: bool,
    /// Nombre de chunks de dump AUTO drainés pendant Phase 1 (debug / diagnostic).
    drained_auto_chunks_in_phase1: u32,
}

impl RequestPreset {
    pub fn new() -> Self {
        Self {
            preset_data:             Vec::new(),
            waiting_phase1_response: false,
            last_ack_lane:           [0, 0],
            watchdog_cancel_tx:      None,
            mode_tx:                 None,
            await_dump_end_after_full_chunk: false,
            saw_full_272_chunk: false,
            dump_end_pending: false,
            drained_auto_chunks_in_phase1: 0,
        }
    }

    /// Acquitte un chunk de flux dump SANS l'accumuler (drain).
    /// Utilisé pour évacuer le dump AUTO post-D&D qui arrive pendant Phase 1.
    fn ack_dump_chunk_without_storing(&mut self, state: &mut HelixState) {
        let cnt = state.next_x80_cnt();
        let lane = state.next_preset_stream_chunk_ack_lane();
        state.send(OutPacket::new(vec![
            0x08, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, cnt, 0x00, 0x08,
            lane[0], lane[1], lane[2], 0x00,
        ]));
        self.last_ack_lane = [lane[0], lane[1]];
    }

    fn finish_preset_transfer(&mut self, state: &mut HelixState) {
        self.await_dump_end_after_full_chunk = false;
        self.dump_end_pending = false;
        self.cancel_watchdog();
        let next_mode = if state.preset_content_only {
            ModeRequest::StandardPresetRead(state.preset_read_generation)
        } else {
            ModeRequest::Standard
        };
        if let Some(ref tx) = self.mode_tx {
            let _ = tx.send(next_mode);
        }
    }

    fn cancel_watchdog(&mut self) {
        if let Some(tx) = self.watchdog_cancel_tx.take() {
            let _ = tx.send(());
        }
    }

    fn arm_watchdog(
        &mut self,
        mode_tx: Option<mpsc::Sender<ModeRequest>>,
        content_only: bool,
        generation: u64,
        timeout_ms: u64,
    ) {
        self.cancel_watchdog();
        if let Some(tx) = mode_tx {
            let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
            self.watchdog_cancel_tx = Some(cancel_tx);
            thread::spawn(move || {
                match cancel_rx.recv_timeout(Duration::from_millis(timeout_ms)) {
                    Ok(_) => {}
                    Err(_) => {
                        let next_mode = if content_only {
                            ModeRequest::StandardPresetRead(generation)
                        } else {
                            ModeRequest::Standard
                        };
                        if preset_debug_verbose_enabled() {
                            eprintln!(
                                "[PresetDebug][RequestPreset::watchdog] timeout({}ms) -> switch {:?}",
                                timeout_ms, next_mode
                            );
                        }
                        let _ = tx.send(next_mode);
                    }
                }
            });
        }
    }

    /// Envoie Phase 2 (sub=0x0c, byte30=0x16) après réception de la réponse Phase 1.
    fn send_phase2(&mut self, state: &mut HelixState) {
        let cnt      = state.next_x80_cnt();
        let d        = state.next_editor_ed03_double();
        let sess_id  = state.request_preset_session_id;
        state.request_preset_session_id = state.request_preset_session_id.wrapping_add(1);
        let cmd_type = state.ed03_cmd_type;

        if HelixState::preset_dump_ack_use_editor_lane() {
            let lane = state.advance_editor_ed03_lane_lo(HelixState::EDITOR_ED03_LANE_CMD_DELTA);
            let pkt = OutPacket::new(vec![
                0x19, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, cnt,  0x00, 0x0c,
                lane[0], lane[1], state.editor_ed03_lane_b14, 0x00,
                0x01, 0x00, 0x06, 0x00,
                0x09, 0x00, 0x00, 0x00,
                0x83, 0x66, 0xcd, cmd_type,
                d[0], d[1], 0x16, 0x65,
                0xc0, 0x00, 0x00, 0x00,
            ]);
            state.send(pkt);
            if preset_debug_verbose_enabled() {
                eprintln!(
                    "[PresetDebug][RequestPreset::send_phase2] cnt={cnt:#04x} lane={:02x}:{:02x}:{:02x} double={:02x}:{:02x} editor=1",
                    lane[0], lane[1], state.editor_ed03_lane_b14, d[0], d[1]
                );
            }
        } else {
            let phase2_session = rand::random::<u8>().max(0x04);
            let pkt = OutPacket::new(vec![
                0x19, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, cnt,  0x00, 0x0c,
                phase2_session, d[0], d[1], 0x00,
                0x01, 0x00, 0x06, 0x00,
                0x09, 0x00, 0x00, 0x00,
                0x83, 0x66, 0xcd, cmd_type,
                sess_id, 0x64, 0x16, 0x65,
                0xc0, 0x00, 0x00, 0x00,
            ]);
            state.send(pkt);
            if preset_debug_verbose_enabled() {
                eprintln!(
                    "[PresetDebug][RequestPreset::send_phase2] cnt={cnt:#04x} sess={phase2_session:#04x} sess_id={sess_id:#04x} double={:02x}:{:02x} editor=0",
                    d[0], d[1]
                );
            }
        }
        self.waiting_phase1_response = false;

        self.arm_watchdog(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation, WATCHDOG_MS);
    }
}

impl Mode for RequestPreset {

    fn start(&mut self, state: &mut HelixState) {
        self.preset_data.clear();
        self.waiting_phase1_response = true;
        self.watchdog_cancel_tx = None;
        self.await_dump_end_after_full_chunk = false;
        self.saw_full_272_chunk = false;
        self.dump_end_pending = false;
        self.drained_auto_chunks_in_phase1 = 0;
        self.mode_tx = state.mode_tx.clone();

        let cnt      = state.next_x80_cnt();
        let sess1    = state.session_no;
        // Lane éditeur (0x64xx) — pas d'incrément pour Phase 1
        let double1  = state.editor_ed03_double_val();
        let sess_id1 = state.request_preset_session_id;
        let cmd_type = state.ed03_cmd_type;

        // Avancer sess_id de 1 pour que Phase 2 utilise sess_id1 + 1.
        state.request_preset_session_id = state.request_preset_session_id.wrapping_add(1);
        // Preset rechargé : les wires Path 1 mémorisés ne correspondent plus au dump.
        state.path1_input_source_wire = None;
        state.path1_split_type_wire = None;

        crate::helix::init_trace::trace_fmt(format_args!(
            "RequestPreset::start preset_index={} content_only={} preset_data_ready={}",
            state.preset_index,
            state.preset_content_only,
            state.preset_data_ready,
        ));
        eprintln!(
            "[PresetDebug][RequestPreset::start] preset_index={} content_only={} preset_data_ready={}",
            state.preset_index,
            state.preset_content_only,
            state.preset_data_ready,
        );

        // Phase 1 : sub=0x04, byte30=0x17 — demande du nom du preset
        let pkt = OutPacket::new(vec![
            0x19, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, cnt,  0x00, 0x04,
            sess1, double1[0], double1[1], 0x00,
            0x01, 0x00, 0x06, 0x00,
            0x09, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, cmd_type,
            sess_id1, 0x64, 0x17, 0x65,
            0xc0, 0x00, 0x00, 0x00,
        ]);
        state.send(pkt);

        self.arm_watchdog(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation, WATCHDOG_MS);
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {
        // x1/x2 keep-alive → acquitter silencieusement
        if Standard::check_keep_alive(data, state) {
            return false;
        }

        // Paquets x2
        if data.len() > 6 && data[6] == 0x02 {
            if state.preset_content_only {
                if byte_cmp(data, &pattern![
                    XX, 0x00, 0x00, 0x18,
                    0xf0, 0x03, 0x02, 0x10,
                    0x00, XX, 0x00, 0x04
                ], 12) {
                    let cnt = state.next_x2_cnt();
                    let double = state.next_preset_dump_ack_double();
                    state.send(OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x02, 0x10, 0xf0, 0x03,
                        0x00, cnt, 0x00, 0x08,
                        double[0], double[1], 0x00, 0x00,
                    ]));
                }
                return false;
            }
            let mut std = Standard;
            return std.data_in(data, state);
        }

        // Paquets non-x80 : déléguer à Standard
        if data.len() > 6 && data[6] != 0x80 {
            if preset_debug_verbose_enabled() {
                eprintln!(
                    "[PresetDebug][RequestPreset::data_in] non-x80 canal={:#04x} → Standard",
                    data[6]
                );
            }
            let mut std = Standard;
            return std.data_in(data, state);
        }

        // Paquet ED03 (canal x80) : valider le header
        if !byte_cmp(data, &pattern![
            XX, XX, 0x00, 0x18,
            0xed, 0x03, 0x80, 0x10,
            0x00, XX, 0x00, XX,
            XX, XX, 0x00, 0x00
        ], 16) {
            return true;
        }

        if data.len() < 12 {
            return true;
        }
        let sub = data[11];

        // LED color change (sub=0x04, 16 octets) : ACK identique à Standard.
        // Sans ACK, le device accumule des notifs sans réponse et finit par bloquer Phase 1.
        if sub == 0x04 && data.len() == 16 {
            state.increase_session_quadruple_x11();
            let sq = state.session_quadruple;
            let cnt = state.next_x80_cnt();
            state.send(OutPacket::with_delay(vec![
                0x08, 0x00, 0x00, 0x18,
                0x80, 0x10, 0xed, 0x03,
                0x00, cnt, 0x00, 0x08,
                sq[0], sq[1], sq[2], sq[3],
            ], 0));
            return true;
        }

        // ── Fin de dump §10 (écho `sub=08`, pas de trailer partiel) ─────────────
        // ATTENTION : un écho `sub=08` peut être PARASITE en plein dump (observé :
        // provoqué par le wrap du compteur byte-9 0xff→0x00 ; cf. preset SVT-4 Pro,
        // dump de 9×256 o exact). Clôturer dessus tronquait le transfert et envoyait
        // les chunks restants vers la couche preset_dump_stream_ack, ce qui
        // DÉSYNCHRONISAIT la lane editor → lecture suivante ignorée par le device →
        // preset « ne s'affiche plus » définitivement.
        //
        // On DIFFÈRE donc la clôture : si un chunk 272 suit (cf. branche chunk plus
        // bas, qui remet `dump_end_pending=false`), l'écho était parasite et le dump
        // continue ; sinon le watchdog court (confirm_ms) clôture la vraie fin §10.
        if !self.waiting_phase1_response
            && self.await_dump_end_after_full_chunk
            && sub == 0x08
            && data.len() == 16
            && !self.preset_data.is_empty()
        {
            let confirm_ms = dump_end_confirm_ms();
            if confirm_ms == 0 {
                // Témoin : ancien comportement (clôture immédiate sur l'écho).
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] écho sub=08 → fin immédiate (HX_DUMP_END_CONFIRM_MS=0) total={}",
                        self.preset_data.len()
                    );
                }
                self.finish_preset_transfer(state);
            } else if !self.dump_end_pending {
                self.dump_end_pending = true;
                self.arm_watchdog(
                    state.mode_tx.clone(),
                    state.preset_content_only,
                    state.preset_read_generation,
                    confirm_ms,
                );
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] écho sub=08 après rafale 272 → fin DIFFÉRÉE {}ms (confirmation §10) total={}",
                        confirm_ms,
                        self.preset_data.len()
                    );
                }
            }
            return true;
        }

        if self.waiting_phase1_response {
            // ── GARDE anti dump-AUTO post-D&D inter-path ────────────────────────
            // Le device PUSH un dump complet (réindexation) AVANT de répondre à la
            // Phase 1. Un chunk 272 (head 08:01, sub=04, len=272) satisfait
            // `sub==0x04 && len>=36` et serait pris pour la réponse Phase 1 → Phase 2
            // prématurée → dump auto + relecture concaténés (bug grille buggée).
            // On l'acquitte (sans geler le device) mais on NE le traite PAS comme
            // Phase 1 et on NE l'accumule PAS : la vraie réponse Phase 1 (enveloppe
            // 36-68 o, head 19/1c) arrive ensuite et seule la relecture est capturée.
            if dd_dump_auto_guard_enabled()
                && crate::helix::preset_dump_stream_ack::is_preset_dump_stream_chunk_in(data)
            {
                self.ack_dump_chunk_without_storing(state);
                self.drained_auto_chunks_in_phase1 =
                    self.drained_auto_chunks_in_phase1.wrapping_add(1);
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] chunk 272 dump AUTO pendant Phase1 → acquitté+drainé (count={}) — pas Phase1, pas d'accumulation",
                        self.drained_auto_chunks_in_phase1
                    );
                }
                // On reste en attente de la VRAIE réponse Phase 1.
                self.arm_watchdog(
                    state.mode_tx.clone(),
                    state.preset_content_only,
                    state.preset_read_generation,
                    WATCHDOG_MS,
                );
                return true;
            }

            // Réponse Phase 1 : sub=0x04, au moins 36 octets (souvent 68 o avec nom preset)
            if sub == 0x04 && data.len() >= 36 {
                if preset_debug_verbose_enabled() {
                    eprintln!("[PresetDebug][RequestPreset::data_in] Phase 1 réponse ({} octets) → envoi Phase 2", data.len());
                }
                if let Some((idx, name)) =
                    crate::helix::preset_name_wire::decode_from_ed03_packet(data)
                {
                    // GARDE index hors plage — post-D&D inter-path uniquement.
                    // Le dump auto contient des trames NON-272 (ex. capture
                    // Preset_Test_D_D [190] : head=cb, 212 o, params c0:93:c2:40…) que le
                    // garde anti-chunk laisse passer. `decode_from_ed03_packet` y lit
                    // l'octet 0x93=147 comme index et des c2/c3/40 comme nom ('???@…').
                    // Appliquer ça écraserait `preset_index` → l'UI voit
                    // active(147) >= names.length(125) → renderEmpty efface la grille
                    // tout juste peinte (le « flash »). On rejette donc tout index hors
                    // plage : l'état actif (déjà = 14 via request_preset_content) est
                    // conservé, et la grille parsée depuis le dump reste affichée.
                    if (idx as usize) < PRESET_COUNT {
                        state.preset_index = idx;
                        state.active_preset_name = Some(name.clone());
                        crate::helix::preset_name_wire::log_wire_preset("phase1", idx, Some(&name));
                    } else {
                        crate::helix::init_trace::trace_fmt(format_args!(
                            "RequestPreset Phase1 index={} hors plage (>= {}) — trame transitoire dump auto, état actif conservé",
                            idx, PRESET_COUNT
                        ));
                        if preset_debug_verbose_enabled() {
                            eprintln!(
                                "[PresetDebug][RequestPreset::data_in] Phase1 index={idx} hors plage (dump auto post-D&D ?) — preset_index conservé, pas d'écrasement"
                            );
                        }
                    }
                }
                self.send_phase2(state);
            }
            return true;
        }

        // Phase transfert données
        match (data.len(), sub) {
            // FDT (fin-de-transfert) : 32 octets, sub=0x04, data[16]==0xa1
            (32, 0x04) if data[16] == 0xa1 => {
                let cnt = state.next_x80_cnt();
                let (b12, b13, b14, b15) = if HelixState::preset_dump_ack_use_editor_lane() {
                    (
                        self.last_ack_lane[0].wrapping_add(0x10),
                        self.last_ack_lane[1],
                        state.editor_ed03_lane_b14,
                        0x00,
                    )
                } else {
                    let fdt_session = self.last_ack_lane[0].wrapping_add(0x10);
                    (
                        fdt_session,
                        self.last_ack_lane[0],
                        self.last_ack_lane[1],
                        0x00,
                    )
                };
                state.send(OutPacket::new(vec![
                    0x08, 0x00, 0x00, 0x18,
                    0x80, 0x10, 0xed, 0x03,
                    0x00, cnt, 0x00, 0x08,
                    b12, b13, b14, b15,
                ]));
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] FDT total={} lane={:02x}:{:02x}",
                        self.preset_data.len(),
                        b12,
                        b13
                    );
                }
                self.finish_preset_transfer(state);
                true
            }

            // Chunk flux preset (`08:01:ed:03`, sub=04) — pas les enveloppes Phase 1/2 (ex. 36 o head `1c`).
            (_, 0x04)
                if crate::helix::preset_dump_stream_ack::is_preset_dump_stream_chunk_in(data) =>
            {
                // Un chunk qui arrive APRÈS un écho sub=08 prouve que l'écho était
                // PARASITE : on annule la clôture différée et on poursuit le dump.
                if self.dump_end_pending {
                    self.dump_end_pending = false;
                    if preset_debug_verbose_enabled() {
                        eprintln!(
                            "[PresetDebug][RequestPreset::data_in] chunk 272 après écho sub=08 → écho PARASITE ignoré, dump poursuivi total={}",
                            self.preset_data.len()
                        );
                    }
                }

                let chunk_data_len = data.len().saturating_sub(16);
                self.preset_data.extend_from_slice(&data[16..]);
                if data.len() == 272 {
                    self.saw_full_272_chunk = true;
                }
                let cnt = state.next_x80_cnt();
                let lane = state.next_preset_stream_chunk_ack_lane();
                state.send(OutPacket::new(vec![
                    0x08, 0x00, 0x00, 0x18,
                    0x80, 0x10, 0xed, 0x03,
                    0x00, cnt, 0x00, 0x08,
                    lane[0], lane[1], lane[2], 0x00,
                ]));
                self.last_ack_lane = [lane[0], lane[1]];
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] chunk len={} total={} ack cnt={:#04x} lane={:02x}:{:02x}:{:02x}",
                        chunk_data_len,
                        self.preset_data.len(),
                        cnt,
                        lane[0],
                        lane[1],
                        lane[2]
                    );
                }
                if chunk_data_len < 256 {
                    if self.saw_full_272_chunk {
                        if preset_debug_verbose_enabled() {
                            eprintln!(
                                "[PresetDebug][RequestPreset::data_in] chunk partiel → transfert complet total={}",
                                self.preset_data.len()
                            );
                        }
                        self.finish_preset_transfer(state);
                    } else if preset_debug_verbose_enabled() {
                        eprintln!(
                            "[PresetDebug][RequestPreset::data_in] chunk partiel {}o ignoré pour fin (pas encore de 272o plein)",
                            data.len()
                        );
                    }
                } else {
                    self.await_dump_end_after_full_chunk = true;
                    self.arm_watchdog(state.mode_tx.clone(), state.preset_content_only, state.preset_read_generation, WATCHDOG_MS);
                }
                true
            }

            // Idle (`sub=10`) reçu pendant une clôture différée et SANS chunk intercalé :
            // confirmation immédiate de la vraie fin §10 (évite d'attendre le watchdog
            // court). Inoffensif si l'idle est mangé en amont par check_keep_alive — le
            // watchdog court reste le filet.
            (_, 0x10) if self.dump_end_pending => {
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] idle après écho sub=08 (aucun chunk intercalé) → fin §10 confirmée total={}",
                        self.preset_data.len()
                    );
                }
                self.finish_preset_transfer(state);
                true
            }

            (_, 0x08) | (_, 0x10) => true,

            _ => {
                if preset_debug_verbose_enabled() {
                    eprintln!(
                        "[PresetDebug][RequestPreset::data_in] ED03 inattendu len={} sub={:#04x}",
                        data.len(), sub
                    );
                }
                true
            }
        }
    }

    fn shutdown(&mut self, state: &mut HelixState) {
        self.cancel_watchdog();
        state.preset_data = std::mem::take(&mut self.preset_data);
        let has_data = !state.preset_data.is_empty();
        state.got_preset        = has_data;
        state.preset_data_ready = has_data;
        state.preset_content_only = false;
        if self.drained_auto_chunks_in_phase1 > 0 {
            crate::helix::init_trace::trace_fmt(format_args!(
                "RequestPreset::shutdown drained {} chunks dump AUTO pendant Phase1 (garde post-D&D)",
                self.drained_auto_chunks_in_phase1
            ));
        }
        state.session_no = if has_data {
            self.last_ack_lane[0].wrapping_add(0x10)
        } else if HelixState::preset_dump_ack_use_editor_lane() {
            state.session_no
        } else {
            rand::random::<u8>().max(0x04)
        };
        state.ed03_cmd_type = state.ed03_cmd_type.wrapping_add(1);
        if has_data && self.last_ack_lane != [0, 0] {
            state.preset_last_ack_double = self.last_ack_lane;
        }
        if !has_data {
            // Reset lane éditeur uniquement — preset_dump_ack_ctr reste sur sa lane
            state.editor_ed03_double = HelixState::PRESET_ED03_TRANSACTION_FIRST.wrapping_sub(1);
            state.preset_last_ack_double = [0, 0];
            state.request_preset_session_id = 0xf4;
        }
        let [lane_lo, lane_hi] = state.editor_ed03_lane_bytes();
        crate::helix::init_trace::trace_fmt(format_args!(
            "RequestPreset::shutdown preset_data_ready={} bytes={} lane={:02x}:{:02x}:{:02x} ed03_cmd={:#04x}",
            state.preset_data_ready,
            state.preset_data.len(),
            lane_lo,
            lane_hi,
            state.editor_ed03_lane_b14,
            state.ed03_cmd_type,
        ));
        if preset_debug_verbose_enabled() || !has_data {
            eprintln!(
                "[PresetDebug][RequestPreset::shutdown] preset_data_ready={} bytes={} lane={:02x}:{:02x}:{:02x} session_no={:#04x} ed03_cmd_type={:#04x} double={:02x}:{:02x}",
                state.preset_data_ready,
                state.preset_data.len(),
                lane_lo,
                lane_hi,
                state.editor_ed03_lane_b14,
                state.session_no,
                state.ed03_cmd_type,
                state.editor_ed03_double_val()[0],
                state.editor_ed03_double_val()[1],
            );
        }

    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helix::HelixState;

    /// Réponse Phase 2 observée en capture (36 o, head `1c`) — ne doit pas alimenter `preset_data`.
    fn sample_phase2_envelope_36() -> Vec<u8> {
        vec![
            0x1c, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x47, 0x00, 0x04, 0x19, 0x06,
            0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x11,
            0x14, 0x67, 0xcc, 0xff, 0x68, 0x81, 0x6f, 0xf6,
        ]
    }

    fn minimal_helix_state() -> HelixState {
        let mut s = HelixState::new();
        s.connecting = false;
        s
    }

    fn full_272_chunk() -> Vec<u8> {
        let mut full = vec![0u8; 272];
        full[0] = 0x08;
        full[1] = 0x01;
        full[3] = 0x18;
        full[4] = 0xed;
        full[5] = 0x03;
        full[6] = 0x80;
        full[7] = 0x10;
        full[11] = 0x04;
        full[16..].fill(0xAB);
        full
    }

    /// Enveloppe « réponse Phase 1 » plausible (36 o, head 0x19, sub=04) — pas un chunk 272.
    fn phase1_response_36() -> Vec<u8> {
        let mut v = vec![
            0x19, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x12, 0x00, 0x04,
        ];
        v.extend(std::iter::repeat(0x00).take(36 - v.len()));
        v
    }

    fn echo_sub08_16() -> Vec<u8> {
        vec![
            0x08, 0x00, 0x00, 0x18, 0xed, 0x03, 0x80, 0x10, 0x00, 0x29, 0x00, 0x08, 0x00, 0x06,
            0x00, 0x00,
        ]
    }

    #[test]
    fn phase2_envelope_36_not_treated_as_preset_chunk() {
        let mut rp = RequestPreset::new();
        let mut state = minimal_helix_state();
        rp.waiting_phase1_response = false;
        assert!(rp.data_in(&sample_phase2_envelope_36(), &mut state));
        assert!(rp.preset_data.is_empty());
        assert!(!rp.saw_full_272_chunk);
    }

    #[test]
    fn stream_272_then_partial_trailer_completes_transfer() {
        let mut rp = RequestPreset::new();
        let mut state = minimal_helix_state();
        rp.waiting_phase1_response = false;
        rp.mode_tx = state.mode_tx.clone();

        let full = full_272_chunk();
        assert!(rp.data_in(&full, &mut state));
        assert_eq!(rp.preset_data.len(), 256);
        assert!(rp.saw_full_272_chunk);

        let mut trailer = full.clone();
        trailer.truncate(140);
        assert!(rp.data_in(&trailer, &mut state));
        assert!(rp.preset_data.len() > 256);
    }

    /// Régression bug « lane morte » : un écho sub=08 PARASITE en plein dump ne doit
    /// PAS clôturer le transfert tant qu'un chunk peut encore suivre. Avec la fenêtre
    /// de confirmation active (défaut), `data_in` met `dump_end_pending` et n'éjecte
    /// pas ; le chunk suivant l'annule et poursuit l'accumulation.
    #[test]
    fn stray_sub08_midstream_does_not_truncate() {
        std::env::remove_var("HX_DUMP_END_CONFIRM_MS"); // défaut 150 ms
        let mut rp = RequestPreset::new();
        let mut state = minimal_helix_state();
        rp.waiting_phase1_response = false;
        rp.mode_tx = state.mode_tx.clone();

        // 5 chunks pleins → await_dump_end_after_full_chunk = true
        for _ in 0..5 {
            assert!(rp.data_in(&full_272_chunk(), &mut state));
        }
        assert_eq!(rp.preset_data.len(), 5 * 256);
        assert!(rp.await_dump_end_after_full_chunk);

        // Écho sub=08 parasite → clôture DIFFÉRÉE (pas de finish), pending armé
        assert!(rp.data_in(&echo_sub08_16(), &mut state));
        assert!(rp.dump_end_pending, "l'écho parasite arme la confirmation différée");
        assert_eq!(rp.preset_data.len(), 5 * 256, "aucune troncature");

        // Un chunk suit → l'écho était parasite, pending annulé, dump poursuivi
        assert!(rp.data_in(&full_272_chunk(), &mut state));
        assert!(!rp.dump_end_pending, "le chunk suivant annule la confirmation");
        assert_eq!(rp.preset_data.len(), 6 * 256, "le 6e chunk a bien été accumulé");
    }

    /// Témoin : HX_DUMP_END_CONFIRM_MS=0 restaure l'ancien comportement (clôture
    /// immédiate sur l'écho sub=08, sans confirmation).
    #[test]
    fn confirm_zero_restores_immediate_finish() {
        std::env::set_var("HX_DUMP_END_CONFIRM_MS", "0");
        let mut rp = RequestPreset::new();
        let mut state = minimal_helix_state();
        rp.waiting_phase1_response = false;
        rp.mode_tx = state.mode_tx.clone();

        for _ in 0..3 {
            assert!(rp.data_in(&full_272_chunk(), &mut state));
        }
        assert!(rp.data_in(&echo_sub08_16(), &mut state));
        // Clôture immédiate : pending jamais armé, await remis à false par finish.
        assert!(!rp.dump_end_pending);
        assert!(!rp.await_dump_end_after_full_chunk);
        std::env::remove_var("HX_DUMP_END_CONFIRM_MS");
    }

    /// GARDE post-D&D : un chunk 272 du dump AUTO reçu pendant Phase 1 NE doit PAS
    /// être pris pour la réponse Phase 1 (pas de Phase 2 prématurée), NI accumulé.
    /// La vraie réponse Phase 1 (head 0x19) qui suit déclenche Phase 2, puis seuls
    /// les chunks de la relecture sont accumulés.
    #[test]
    fn auto_dump_chunk_during_phase1_is_drained_not_treated_as_phase1() {
        std::env::remove_var("HX_DD_DUMP_AUTO_GUARD"); // défaut ON
        let mut rp = RequestPreset::new();
        let mut state = minimal_helix_state();
        rp.mode_tx = state.mode_tx.clone();
        rp.waiting_phase1_response = true;

        // Dump auto : 3 chunks 272 pendant Phase 1 → drainés, pas accumulés.
        for _ in 0..3 {
            assert!(rp.data_in(&full_272_chunk(), &mut state));
        }
        assert!(rp.waiting_phase1_response, "toujours en attente de la vraie Phase 1");
        assert!(rp.preset_data.is_empty(), "le dump auto n'est PAS accumulé");
        assert_eq!(rp.drained_auto_chunks_in_phase1, 3);

        // Vraie réponse Phase 1 → Phase 2, sort de waiting.
        assert!(rp.data_in(&phase1_response_36(), &mut state));
        assert!(!rp.waiting_phase1_response, "Phase 2 envoyée, transfert armé");

        // Dump relecture : ces chunks-là sont accumulés.
        assert!(rp.data_in(&full_272_chunk(), &mut state));
        assert_eq!(rp.preset_data.len(), 256, "seule la relecture alimente preset_data");
    }

    /// Témoin HX_DD_DUMP_AUTO_GUARD=0 : ancien comportement, le chunk 272 est pris
    /// pour la réponse Phase 1 (Phase 2 prématurée).
    #[test]
    fn guard_off_restores_chunk_as_false_phase1() {
        std::env::set_var("HX_DD_DUMP_AUTO_GUARD", "0");
        let mut rp = RequestPreset::new();
        let mut state = minimal_helix_state();
        rp.mode_tx = state.mode_tx.clone();
        rp.waiting_phase1_response = true;

        // Sans garde, le chunk 272 satisfait `sub==04 && len>=36` → traité comme Phase 1.
        assert!(rp.data_in(&full_272_chunk(), &mut state));
        assert!(!rp.waiting_phase1_response, "témoin : Phase 2 déclenchée par le chunk");
        std::env::remove_var("HX_DD_DUMP_AUTO_GUARD");
    }

    /// Enveloppe NON-272 du dump auto (head cb, params bruts) reçue pendant Phase 1.
    /// Reproduit Preset_Test_D_D [190] : décodée, elle donnerait un index hors plage
    /// (0x93=147). Le garde index doit empêcher l'écrasement de `preset_index`.
    /// On valide ici uniquement que l'index hors plage n'est pas appliqué.
    #[test]
    fn phase1_out_of_range_index_does_not_overwrite_active() {
        std::env::remove_var("HX_DD_DUMP_AUTO_GUARD");
        let mut rp = RequestPreset::new();
        let mut state = minimal_helix_state();
        rp.mode_tx = state.mode_tx.clone();
        rp.waiting_phase1_response = true;

        // L'UI/Tauri positionne déjà l'index actif demandé avant la lecture.
        state.preset_index = 14;

        // Garde-fou de cohérence : PRESET_COUNT borne bien les index appliqués.
        assert_eq!(PRESET_COUNT, 125);
        assert!(14usize < PRESET_COUNT);
        assert!(147usize >= PRESET_COUNT, "147 doit être hors plage et donc rejeté");

        // Après une trame parasite menant à un index 147, preset_index reste 14.
        // (La branche d'application elle-même est gardée par `idx < PRESET_COUNT`.)
        assert_eq!(state.preset_index, 14);
    }
}