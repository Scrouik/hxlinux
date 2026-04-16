// ===========================================================
// helix/modes/request_preset_names.rs
// Lecture des 125 noms de presets
// Traduction fidèle de request_preset_names.py de kempline
// ===========================================================

use std::thread;
use std::time::Duration;
use std::sync::mpsc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use crate::helix::{Mode, HelixState, ModeRequest};
use crate::helix::packet::{OutPacket, byte_cmp};
use crate::helix::modes::standard::Standard;
use crate::pattern;

const EXPECTED_PRESET_COUNT: usize = 125;

static DEBUG_EXTRACT_PRESET_INDEX_FAILURES: AtomicUsize = AtomicUsize::new(0);

pub struct RequestPresetNames {
    preset_names_stream:          Vec<u8>,
    stream_parse_idx:             usize,
    decoded_names_by_index:       std::collections::HashMap<usize, String>,
    decoded_names_fallback:       Vec<String>,
    transfer_complete:            bool,
    watchdog_cancel_tx:           Option<mpsc::Sender<()>>,
    mode_tx:                      Option<mpsc::Sender<ModeRequest>>,
}

impl RequestPresetNames {
    pub fn new() -> Self {
        Self {
            preset_names_stream:    Vec::new(),
            stream_parse_idx:       0,
            decoded_names_by_index: std::collections::HashMap::new(),
            decoded_names_fallback: Vec::new(),
            transfer_complete:      false,
            watchdog_cancel_tx:     None,
            mode_tx:                None,
        }
    }

    /// Arme le watchdog — kempline : _arm_idle_watchdog()
    /// Si aucun paquet reçu pendant 750ms → on finalise
    fn arm_watchdog(&mut self) {
        self.cancel_watchdog();
        if let Some(mode_tx) = self.mode_tx.clone() {
            let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
            self.watchdog_cancel_tx = Some(cancel_tx);
            thread::spawn(move || {
                match cancel_rx.recv_timeout(Duration::from_millis(750)) {
                    Ok(_)  => {}
                    Err(_) => {
                        let _ = mode_tx.send(ModeRequest::Standard);
                    }
                }
            });
        }
    }

    /// Annule le watchdog courant
    fn cancel_watchdog(&mut self) {
        if let Some(tx) = self.watchdog_cancel_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Parse les noms dans le stream accumulé.
    ///
    /// IMPORTANT:
    /// Les captures montrent des records de nom à longueur variable (et parfois
    /// coupés entre 2 paquets USB). On ne peut donc pas découper par taille fixe.
    /// On segmente de marqueur `81 cd 00` à marqueur suivant.
    fn parse_preset_names(&mut self, finalize: bool) -> usize {
        let marker = [0x81u8, 0xcd, 0x00];

        loop {
            let len = self.preset_names_stream.len();
            if len < marker.len() {
                break;
            }

            // Trouver le prochain marqueur à partir de stream_parse_idx
            let search_start = self.stream_parse_idx.min(len.saturating_sub(marker.len()));
            let marker_idx = self.preset_names_stream[search_start..]
                .windows(marker.len())
                .position(|w| w == marker)
                .map(|p| p + search_start);

            let marker_idx = match marker_idx {
                Some(i) => i,
                None => {
                    // Conserver une petite queue pour gérer un marqueur qui chevauche 2 paquets.
                    if !finalize {
                        self.stream_parse_idx = len.saturating_sub(marker.len() - 1);
                    } else {
                        self.stream_parse_idx = len;
                    }
                    break;
                }
            };

            // Chercher le marqueur suivant => fin du record courant
            let next_search_start = marker_idx + marker.len();
            let next_marker_idx = if next_search_start + marker.len() <= len {
                self.preset_names_stream[next_search_start..]
                    .windows(marker.len())
                    .position(|w| w == marker)
                    .map(|p| p + next_search_start)
            } else {
                None
            };

            let record_end = match next_marker_idx {
                Some(i) => i,
                None => {
                    if !finalize {
                        // Record potentiellement incomplet (coupé entre paquets)
                        self.stream_parse_idx = marker_idx;
                        break;
                    }
                    len
                }
            };

            if record_end <= marker_idx || record_end > len {
                break;
            }

            let record = &self.preset_names_stream[marker_idx..record_end];

            // Le nom démarre après `6d xx` et se termine au premier `00`.
            let name = if let Some(pos_6d) = record.iter().position(|&b| b == 0x6d) {
                let start = pos_6d.saturating_add(2);
                if start < record.len() {
                    let end_rel = record[start..]
                        .iter()
                        .position(|&b| b == 0x00)
                        .unwrap_or(record.len() - start);
                    let end = start + end_rel;
                    record[start..end]
                        .iter()
                        .map(|&b| if (32..=126).contains(&b) { b as char } else { '?' })
                        .collect::<String>()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            if !name.is_empty() {
                // Tentative d'indexation (souvent absente sur ce flux), sinon fallback séquentiel.
                let preset_idx = self.extract_preset_index(record);
                match preset_idx {
                    Some(idx) => {
                        self.decoded_names_by_index.entry(idx).or_insert(name);
                    }
                    None => {
                        self.decoded_names_fallback.push(name);
                    }
                }
            }

            self.stream_parse_idx = record_end;
        }

        self.decoded_names_by_index.len() + self.decoded_names_fallback.len()
    }

    /// Extrait l'index du preset depuis un record
    /// Kempline : _extract_record_preset_index()
    fn extract_preset_index(&self, record: &[u8]) -> Option<usize> {
        if record.len() < 9 {
            return None;
        }
        let metadata = &record[3..9];
        let mut idx_6b: i32 = -1;
        let mut idx_6c: i32 = -1;

        for i in 0..metadata.len() {
            if metadata[i] == 0x6b && i + 1 < metadata.len() {
                idx_6b = metadata[i + 1] as i32;
            } else if metadata[i] == 0x6c && i + 1 < metadata.len() {
                idx_6c = metadata[i + 1] as i32;
            }
        }

        if idx_6b < 0 || idx_6c < 0 {
            let n = DEBUG_EXTRACT_PRESET_INDEX_FAILURES.fetch_add(1, AtomicOrdering::Relaxed);
            if n < 12 {
                let mut pos_6b: Vec<usize> = Vec::new();
                let mut pos_6c: Vec<usize> = Vec::new();
                for (i, b) in record.iter().enumerate() {
                    if *b == 0x6b {
                        pos_6b.push(i);
                    } else if *b == 0x6c {
                        pos_6c.push(i);
                    }
                }
                eprintln!(
                    "[PresetDebug][extract_preset_index] fail n={} record_prefix={:02x?} metadata={:02x?} idx_6b={} idx_6c={} pos6b={:?} pos6c={:?}",
                    n,
                    &record.iter().take(12).cloned().collect::<Vec<u8>>(),
                    metadata,
                    idx_6b,
                    idx_6c,
                    pos_6b,
                    pos_6c
                );
            }
            return None;
        }

        let candidate = (idx_6b * 25 + idx_6c) as usize;
        if candidate < EXPECTED_PRESET_COUNT {
            Some(candidate)
        } else {
            let n = DEBUG_EXTRACT_PRESET_INDEX_FAILURES.fetch_add(1, AtomicOrdering::Relaxed);
            if n < 12 {
                let metadata = &record[3..9];
                eprintln!(
                    "[PresetDebug][extract_preset_index] candidate_oob n={} idx_6b={} idx_6c={} candidate={} metadata={:02x?} record_prefix={:02x?}",
                    n,
                    idx_6b,
                    idx_6c,
                    candidate,
                    metadata,
                    &record.iter().take(12).cloned().collect::<Vec<u8>>()
                );
            }
            None
        }
    }

    /// Construit la liste finale alignée par index
    /// Kempline : _build_aligned_preset_names()
    fn build_aligned_names(&self) -> Vec<String> {
        let placeholder = "<empty>".to_string();
        let mut aligned = vec![placeholder.clone(); EXPECTED_PRESET_COUNT];

        for (&idx, name) in &self.decoded_names_by_index {
            if idx < EXPECTED_PRESET_COUNT {
                aligned[idx] = name.clone();
            }
        }

        // Safety-first:
        // Quand on ne peut pas extraire l'index d'un record, on stocke son nom dans `decoded_names_fallback`.
        // Le remplissage "dans l'ordre" peut créer un décalage si le flux a des trous (presets manquants) au
        // milieu de la liste. Pour garder l'alignement strict index↔preset, on ne consomme le fallback que si
        // les seules entrées manquantes (<empty>) sont un suffixe en fin de liste.
        let last_filled = aligned.iter().rposition(|s| s != &placeholder);
        if let Some(last_filled) = last_filled {
            let suffix_start = last_filled + 1;
            let has_placeholder_in_middle = aligned[..suffix_start]
                .iter()
                .any(|s| s == &placeholder);

            if has_placeholder_in_middle {
                eprintln!(
                    "[PresetDebug][request_preset_names] fallback skipped (holes in middle). \
decoded_by_index={} fallback_count={} suffix_start={}",
                    self.decoded_names_by_index.len(),
                    self.decoded_names_fallback.len(),
                    suffix_start
                );
                return aligned;
            }

            let suffix_len = EXPECTED_PRESET_COUNT.saturating_sub(suffix_start);
            if self.decoded_names_fallback.len() != suffix_len {
                eprintln!(
                    "[PresetDebug][request_preset_names] fallback not used (count mismatch). \
decoded_by_index={} fallback_count={} suffix_len={}",
                    self.decoded_names_by_index.len(),
                    self.decoded_names_fallback.len(),
                    suffix_len
                );
                return aligned;
            }

            let mut fallback_iter = self.decoded_names_fallback.iter();
            for slot in aligned[suffix_start..].iter_mut() {
                if slot == &placeholder {
                    if let Some(name) = fallback_iter.next() {
                        *slot = name.clone();
                    }
                }
            }
        } else {
            // Cas observé sur certains dumps: aucun index 6b/6c exploitable,
            // mais une liste fallback quasi complète est disponible.
            // On rétablit alors un mapping séquentiel (best-effort) pour ne pas
            // afficher une liste entièrement vide.
            eprintln!(
                "[PresetDebug][request_preset_names] no indexable names; using sequential fallback (fallback_count={})",
                self.decoded_names_fallback.len()
            );
            for (idx, name) in self.decoded_names_fallback.iter().enumerate() {
                if idx >= EXPECTED_PRESET_COUNT {
                    break;
                }
                aligned[idx] = name.clone();
            }
        }

        aligned
    }

    /// Finalise le transfert
    /// Kempline : _finish_transfer()
    fn finish_transfer(&mut self, state: &mut HelixState) {
        if self.transfer_complete {
            return;
        }
        self.transfer_complete = true;
        self.cancel_watchdog();
        self.parse_preset_names(true);

        let names = self.build_aligned_names();

        // Trace utile pour comprendre l'ordre de remplissage quand l'indexation échoue.
        // (On limite volontairement au début pour ne pas spammer.)
        for idx in 0..13 {
            let n = names.get(idx).cloned().unwrap_or_default();
            eprintln!(
                "[PresetDebug][request_preset_names] aligned idx={} name='{}'",
                idx, n
            );
        }

        state.preset_names     = names;
        state.got_preset_names = true;
        state.just_fetched_preset_names = true;
        state.new_session_no();
        // Plus de switch_mode ici — c'est lib.rs qui switche
    }

    /// Ajoute le payload d'un paquet au stream
    /// Kempline : _append_name_packet_payload()
    fn append_payload(&mut self, data: &[u8]) {
        if data.len() > 16 {
            self.preset_names_stream.extend_from_slice(&data[16..]);
        }
    }
}

impl Mode for RequestPresetNames {

    fn start(&mut self, state: &mut HelixState) {
        self.preset_names_stream.clear();
        self.decoded_names_by_index.clear();
        self.decoded_names_fallback.clear();
        self.stream_parse_idx       = 0;
        self.transfer_complete      = false;
        self.watchdog_cancel_tx     = None;
        self.mode_tx                = state.mode_tx.clone();

        let cnt     = state.next_x1_cnt();

        // Kempline : data = [0x1d, 0x0, 0x0, 0x18, 0x1, 0x10, 0xef, 0x3, 0x0, "XX", 0x0, 0xc,
        //                    0x38, 0x10, 0x0, 0x0, 0x1, 0x0, 0x2, 0x0, 0xd, 0x0, 0x0, 0x0,
        //                    0x83, 0x66, 0xcd, 0x3, 0xea, 0x64, 0x1, 0x65, 0x82, 0x6b, 0x0, 0x65,
        //                    0x2, 0x0, 0x0, 0x0]
        let pkt = OutPacket::new(vec![
            0x1d, 0x00, 0x00, 0x18,
            0x01, 0x10, 0xef, 0x03,
            0x00, cnt,  0x00, 0x0c,
            0x38, 0x10, 0x00, 0x00,
            0x01, 0x00, 0x02, 0x00,
            0x0d, 0x00, 0x00, 0x00,
            0x83, 0x66, 0xcd, 0x03,
            0xea, 0x64, 0x01, 0x65,
            0x82, 0x6b, 0x00, 0x65,
            0x02, 0x00, 0x00, 0x00,
        ]);
        state.send(pkt);

        // Armer le watchdog
        self.arm_watchdog();
    }

    fn data_in(&mut self, data: &[u8], state: &mut HelixState) -> bool {
        self.arm_watchdog();

        // Keep-alive → acquitter silencieusement
        if Standard::check_keep_alive(data, state) {
            return false;
        }

        // Paquet avec un byte de payload (0x8, 0x1, ...)
        // Kempline : my_byte_cmp([0x8, 0x1, 0x0, 0x18, 0xef, 0x3, 0x1, 0x10,
        // 0x0, "XX", 0x0, 0x4, "XX", 0x2, 0x0, 0x0, "XX"], length=17)
        if byte_cmp(data, &pattern![
            0x08, 0x01, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, XX,   0x00, 0x04,
            XX,   0x02, 0x00, 0x00, XX
        ], 17) {
            self.append_payload(data);

            // Ack
            let ack_cnt  = data[9].wrapping_add(9);
            let next_cnt = state.next_x1_cnt();
            let pkt = OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, next_cnt, 0x00, 0x08,  // ← next_x1_cnt() comme kempline
                0x38, ack_cnt,  0x00, 0x00,
            ]);
            state.send(pkt);

            // Réarmer le watchdog
            self.arm_watchdog();
            

            // Vérifier si on a tous les noms
            let count = self.parse_preset_names(false);
            if count >= EXPECTED_PRESET_COUNT {
                self.finish_transfer(state);
            }

            return true;
        }

        // Paquet intermédiaire ou final
        // Kempline : my_byte_cmp(["XX", 0x0, 0x0, 0x18, 0xef, 0x3, 0x1, 0x10,
        //                          0x0, "XX", 0x0, 0x4, "XX", 0x2, 0x0, 0x0], length=16)
        if byte_cmp(data, &pattern![
            XX,   0x00, 0x00, 0x18,
            0xef, 0x03, 0x01, 0x10,
            0x00, XX,   0x00, 0x04,
            XX,   0x02, 0x00, 0x00
        ], 16) {
            // Ack
            let ack_cnt  = data[9].wrapping_add(9);
            let next_cnt = state.next_x1_cnt();
            let pkt = OutPacket::new(vec![
                0x08, 0x00, 0x00, 0x18,
                0x01, 0x10, 0xef, 0x03,
                0x00, next_cnt, 0x00, 0x08,  // ← next_x1_cnt() comme kempline
                0x38, ack_cnt,  0x00, 0x00,
            ]);
            state.send(pkt);

            self.append_payload(data);

            // Réarmer le watchdog
            self.arm_watchdog();

            let count = self.parse_preset_names(false);
            if count >= EXPECTED_PRESET_COUNT {
                self.finish_transfer(state);
            }

            return true;
        }

        true
    }

    fn shutdown(&mut self, state: &mut HelixState) {
        self.cancel_watchdog();
        state.got_preset = false;
        // Finaliser si pas encore fait (cas du watchdog timeout)
        if !self.transfer_complete {
            self.finish_transfer(state);
        }
}
}