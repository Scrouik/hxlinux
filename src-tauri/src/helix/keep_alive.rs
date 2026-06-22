// ===========================================================
// helix/keep_alive.rs
// Keep-alive idle tri-lane — aligné capture HX Edit (`stomp_running_start_hxedit.json`).
//
// HX Edit en idle (t ≥ ~9 s, 21 cycles observés) n'envoie PAS un seul poll mais
// TROIS polls `sub=10` par cycle (~1,047 s), dans l'ordre `f0 → ed → ef`, chaque
// lane avançant son propre compteur octet 9 (+1/cycle), avec un trailer FIXE :
//
//   f0:03 (02:10)  cnt=x2_cnt   tail = 09 10 00 00   (+0 ms dans le cycle)
//   ed:03 (80:10)  cnt=x80_cnt  tail = 7e 1c 00 00   (+~453 ms)
//   ef:03 (01:10)  cnt=x1_cnt   tail = e0 1d 00 00   (+~266 ms)  → +~328 ms → cycle suivant
//
// Le device NE ré-écho PAS le trailer (il le transforme : 09:10→09:02, 7e:1c→ae:02,
// l'octet 13 passe à 02). Donc le trailer n'est pas un tag de corrélation strict :
// le Stomp l'accepte et répond. Les trois trailers sont donc figés en littéraux.
//
// Compteurs : partagés avec le reste (x2 ↔ firmware_scroll_ack, x1 ↔ RequestPresetNames).
// En idle pur (pas de scroll, pas de requête host), seules ces trois lanes consomment
// leurs compteurs → séquence +1/cycle propre, comme HX. Le saut observé sur
// `one_scroll_hxlinux` n'apparaît QUE pendant un scroll, où HX intercale aussi ses
// ACK f0 sub=08 sur la même lane : comportement attendu, pas une régression.
//
// IMPORTANT : c'est le POLL idle (sub=10). Les ARM bootstrap (`amorcage`) et les
// ACK scroll (`firmware_scroll_ack`) restent sur sub=08 / 09:10 — ne pas confondre.
//
// ⚠ LANE ed:03 FIGÉE (`TAIL_ED = 7e 1c`) vs HX DYNAMIQUE — voir `keepalive_ed_lane`.
// Sur la capture HX `add_dual_cab_soup_pro_2x12bluebell_HXEdit`, le keepalive ed:03
// n'est PAS figé : il porte la lane modèle COURANTE (`8a 1c` pendant l'op, puis `9b 1c`
// après l'écriture). HX a UNE seule lane host (keepalive = focus = ed:08 − 0x11) qui
// avance de 0x11 à chaque écriture. Le device valide l'`IN 21` sur la continuité
// `ed:08 == dernière lane keepalive + 0x11`. Ici on fige `7e 1c` (suffisant en idle car
// le device est laxiste sur le trailer du poll) ; le focus Cab 2 reprend donc cette même
// lane figée (`keepalive_ed_lane`) pour que l'ed:08 = `7e 1c + 0x11 = 8f 1c` raccorde et
// débloque l'`IN 21`. Refonte propre (lane host unique qui avance comme HX) = chantier §5.
//
// FERMETURE GRACIEUSE (sub=02) : voir `graceful_close_packets` plus bas. À l'instant
// du close, HX Edit envoie un DERNIER tour sur les 3 lanes avec `byte 11 = 0x02`
// (le sous-type du *subscribe*), chacun ACKé par le Stomp, puis silence HID. C'est
// le « désabonnement éditeur » qui rend l'écran au Stomp. Sans lui, le device reste
// abonné : écran figé, moteur audio encore actif. (Capture `08_close_HXEdit.json`.)
//
// COMPTEUR DU CLOSE — règle confirmée sur 2 captures (HX Edit + close_linux) : le
// Stomp lit un OUT dont l'octet 9 ÉGALE son compteur courant comme un simple ACK
// (host accuse réception du keep-alive device) → il NE traite PAS le désabonnement.
// Un OUT avec un compteur DIFFÉRENT est lu comme une requête → il traite + répond.
// Le close doit donc porter `dernier compteur IN device sur la lane + 1`. Réutiliser
// l'écho keep-alive (x80/x2/x1) retombe pile sur le compteur device → ACK → ignoré
// (bug observé sur ed/f0 ; ef passait par hasard de phase).
// ===========================================================

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::helix::HelixState;
use crate::helix::packet::OutPacket;

static KEEPALIVE_TRACE: AtomicBool = AtomicBool::new(false);

pub fn init_from_env() {
    if !std::env::var("HX_KEEPALIVE_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return;
    }
    KEEPALIVE_TRACE.store(true, Ordering::Relaxed);
    eprintln!("[KeepAliveTrace] activé — HX_KEEPALIVE_TRACE=1 (OUT ed sub=10 uniquement)");
}

fn trace_ed_poll(cnt: u8) {
    if !KEEPALIVE_TRACE.load(Ordering::Relaxed) {
        return;
    }
    eprintln!(
        "[KeepAliveTrace] OUT ed sub=10 cnt={cnt:02x} tail={:02x}:{:02x}",
        TAIL_ED[0], TAIL_ED[1]
    );
}

/// Période totale d'un cycle (somme des trois deltas inter-lanes). HX ≈ 1,047 s.
pub const KEEP_ALIVE_CYCLE_MS: u64 = 1047;

/// Délais intra-cycle observés sur HX Edit idle (`f0 → ed → ef → f0`).
/// f0 est l'origine du cycle (+0 ms) ; les sleeps séparent les envois suivants.
const DELAY_F0_TO_ED_MS: u64 = 453;
const DELAY_ED_TO_EF_MS: u64 = 266;
const DELAY_EF_TO_NEXT_F0_MS: u64 = 328; // 453 + 266 + 328 = 1047

/// Délai HX Edit après bootstrap phase 4 avant le 1er poll et `RequestPresetNames`.
pub const POST_PHASE4_SETTLE_MS: u64 = 700;

/// Trailers idle figés (octets 12–15) relevés sur HX Edit, constants sur 21 cycles.
/// Seule référence 3-lanes idle connue-bonne ; le device les transforme sans rejeter.
const TAIL_F0: [u8; 4] = [0x09, 0x10, 0x00, 0x00];
const TAIL_ED: [u8; 4] = [0x7e, 0x1c, 0x00, 0x00];
const TAIL_EF: [u8; 4] = [0xe0, 0x1d, 0x00, 0x00];

/// Sous-type « poll idle » (octet 11).
pub const POLL_SUB: u8 = 0x10;

/// Sous-type « fermeture / désabonnement éditeur » (octet 11), relevé sur
/// `08_close_HXEdit.json` : c'est le sous-type du *subscribe*, réutilisé au close.
pub const CLOSE_SUB: u8 = 0x02;

/// Lane ed:03 (octets 12-13) que le keepalive idle déclare au device (`TAIL_ED` = `7e 1c`).
///
/// Le focus Cab 2 (`1d`) et l'ed:08 du handshake DOIVENT reprendre cette lane : sur la
/// capture HX `add_dual_cab_soup_pro_2x12bluebell_HXEdit`, le device renvoie l'`IN 21`
/// uniquement quand `ed:08 == dernière lane keepalive + 0x11` (`8a 1c` → `9b 1c`). La lane
/// d'écho du device (`ec 02` côté HX, `5f 03` côté Stomp) est INDÉPENDANTE et ne sert pas
/// à valider l'ed:08. Comme notre keepalive est figé sur `7e 1c`, le focus doit porter
/// `7e 1c` → ed:08 `8f 1c`. Si un jour le keepalive devient dynamique (lane host unique qui
/// avance comme HX, chantier §5), cette fonction restera la source unique à interroger.
pub fn keepalive_ed_lane() -> [u8; 2] {
    [TAIL_ED[0], TAIL_ED[1]]
}

/// Construit les 3 paquets de fermeture gracieuse (un par lane, `sub=0x02`).
///
/// Compteur (octet 9) = **dernier compteur IN reçu du device sur la lane + 1**
/// (cf. en-tête : c'est ce qui fait une REQUÊTE que le device traite, et pas un ACK
/// qu'il ignore). Source : `HelixState::dev_keepalive_cnt_{ed,ef,f0}`, alimentés par
/// `ingest_hw_slot_notify_in`. Fallback sur l'écho host si aucune IN n'a été observée
/// (ne devrait pas arriver en session active).
///
/// Trailers = ceux des polls idle (le device les accepte sans validation stricte) ;
/// seul l'octet 11 passe à `0x02`. `graceful_close_packets` renvoie `[f0, ed, ef]` ;
/// `lib.rs::graceful_helix_close` réordonne en `ed → f0 → ef` (ordre HX Edit).
///
/// À envoyer AVANT `release_interface`, tant que l'USB est vivant. Sur quit
/// d'application, l'appel DOIT être synchrone (cf. `lib.rs::graceful_helix_close`).
pub fn graceful_close_packets(state: &mut HelixState) -> [Vec<u8>; 3] {
    let c_f0 = state
        .dev_keepalive_cnt_f0
        .map(|c| c.wrapping_add(1))
        .unwrap_or_else(|| state.next_x2_cnt());
    let c_ed = state
        .dev_keepalive_cnt_ed
        .map(|c| c.wrapping_add(1))
        .unwrap_or_else(|| state.next_x80_cnt());
    let c_ef = state
        .dev_keepalive_cnt_ef
        .map(|c| c.wrapping_add(1))
        .unwrap_or_else(|| state.next_x1_cnt());

    if KEEPALIVE_TRACE.load(Ordering::Relaxed) {
        eprintln!(
            "[KeepAliveTrace] close cnt (device_last+1) f0={c_f0:02x} ed={c_ed:02x} ef={c_ef:02x} \
             (dev_last f0={:?} ed={:?} ef={:?})",
            state.dev_keepalive_cnt_f0, state.dev_keepalive_cnt_ed, state.dev_keepalive_cnt_ef
        );
    }

    [
        vec![
            0x08, 0x00, 0x00, 0x18,
            0x02, 0x10, 0xf0, 0x03,
            0x00, c_f0, 0x00, CLOSE_SUB,
            TAIL_F0[0], TAIL_F0[1], 0x00, 0x00,
        ],
        vec![
            0x08, 0x00, 0x00, 0x18,
            0x80, 0x10, 0xed, 0x03,
            0x00, c_ed, 0x00, CLOSE_SUB,
            TAIL_ED[0], TAIL_ED[1], 0x00, 0x00,
        ],
        vec![
            0x08, 0x00, 0x00, 0x18,
            0x01, 0x10, 0xef, 0x03,
            0x00, c_ef, 0x00, CLOSE_SUB,
            TAIL_EF[0], TAIL_EF[1], 0x00, 0x00,
        ],
    ]
}

pub struct KeepAliveManager {
    stop_ordered: Arc<AtomicBool>,
}

impl KeepAliveManager {
    pub fn new() -> Self {
        Self {
            stop_ordered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cycle idle tri-lane `f0 → ed → ef` (sub=10), trailers figés HX.
    /// Un seul thread : l'ordre et les deltas inter-lanes sont sous contrôle direct
    /// (≠ trois timers libres qui dériveraient).
    pub fn start_ordered(&self, state: Arc<Mutex<HelixState>>) {
        let stop = Arc::clone(&self.stop_ordered);
        stop.store(false, Ordering::SeqCst);

        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                // Pendant une lecture de corps de preset (`preset_content_only`),
                // le host n'émet pas de poll proactif : on saute le cycle entier.
                let skip_cycle = {
                    let s = state.lock().unwrap();
                    s.preset_content_only
                };
                if skip_cycle {
                    thread::sleep(Duration::from_millis(KEEP_ALIVE_CYCLE_MS));
                    continue;
                }

                // ── Lane f0 (origine du cycle, +0 ms) ─────────────────────────
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x2_cnt();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x02, 0x10, 0xf0, 0x03,
                        0x00, cnt, 0x00, POLL_SUB,
                        TAIL_F0[0], TAIL_F0[1], TAIL_F0[2], TAIL_F0[3],
                    ]);
                    s.send(pkt);
                }
                if Self::sleep_or_stop(&stop, DELAY_F0_TO_ED_MS) {
                    break;
                }

                // ── Lane ed (+~453 ms) ────────────────────────────────────────
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x80_cnt();
                    trace_ed_poll(cnt);
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x80, 0x10, 0xed, 0x03,
                        0x00, cnt, 0x00, POLL_SUB,
                        TAIL_ED[0], TAIL_ED[1], TAIL_ED[2], TAIL_ED[3],
                    ]);
                    s.send(pkt);
                }
                if Self::sleep_or_stop(&stop, DELAY_ED_TO_EF_MS) {
                    break;
                }

                // ── Lane ef (+~266 ms) ────────────────────────────────────────
                {
                    let mut s = state.lock().unwrap();
                    let cnt = s.next_x1_cnt();
                    let pkt = OutPacket::new(vec![
                        0x08, 0x00, 0x00, 0x18,
                        0x01, 0x10, 0xef, 0x03,
                        0x00, cnt, 0x00, POLL_SUB,
                        TAIL_EF[0], TAIL_EF[1], TAIL_EF[2], TAIL_EF[3],
                    ]);
                    s.send(pkt);
                }
                if Self::sleep_or_stop(&stop, DELAY_EF_TO_NEXT_F0_MS) {
                    break;
                }
            }
        });
    }

    /// Sleep interruptible : retourne `true` si un arrêt a été demandé pendant l'attente
    /// (granularité ~20 ms pour rester réactif au shutdown sans busy-loop).
    fn sleep_or_stop(stop: &Arc<AtomicBool>, total_ms: u64) -> bool {
        const STEP_MS: u64 = 20;
        let mut remaining = total_ms;
        while remaining > 0 {
            if stop.load(Ordering::SeqCst) {
                return true;
            }
            let step = remaining.min(STEP_MS);
            thread::sleep(Duration::from_millis(step));
            remaining -= step;
        }
        stop.load(Ordering::SeqCst)
    }

    pub fn stop_all(&self) {
        self.stop_ordered.store(true, Ordering::SeqCst);
    }

    /// Fermeture gracieuse via le writer asynchrone : stoppe d'abord le cycle idle
    /// (pour qu'aucun poll `sub=10` ne s'intercale APRÈS), laisse le thread sortir de
    /// son sleep, puis émet le tour `sub=0x02` sur les 3 lanes et patiente pour laisser
    /// le Stomp traiter le désabonnement + le writer drainer.
    ///
    /// À utiliser sur une déconnexion où l'USB est ENCORE présent (ex. erreur interne).
    /// **PAS** sur quit d'application (`exit(0)` tue le process avant) — voir
    /// `lib.rs::graceful_helix_close`, qui réutilise `graceful_close_packets`.
    #[allow(dead_code)] // réservé au désabonnement hors-quit (déconnexion/erreur)
    pub fn stop_graceful(&self, state: &Arc<Mutex<HelixState>>) {
        self.stop_ordered.store(true, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(40));
        {
            let mut s = state.lock().unwrap();
            if !s.connected {
                return;
            }
            let pkts = graceful_close_packets(&mut s);
            for p in pkts.iter() {
                s.send(OutPacket::new(p.clone()));
            }
        }
        if KEEPALIVE_TRACE.load(Ordering::Relaxed) {
            eprintln!("[KeepAliveTrace] StopGraceful — tour sub=02 envoyé (f0/ed/ef)");
        }
        thread::sleep(Duration::from_millis(80));
    }
}