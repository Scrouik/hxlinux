//! Commande console — replace **Cab 2** sur slot dual, version **lane cohérente**.
//!
//! La version figée (corps HX Edit `cab dual change right.json`) changeait bien cab2 MAIS
//! plantait le HW : compteurs incohérents (ed:08 figé, bulk = L au lieu de L+0x11) + 15 ACK
//! figés post-bulk qui désynchronisent la session.
//!
//! Preuve apportée par cette console : la **lane modèle = `live_write_ctr`** (le device accepte
//! focus/bulk patchés sur ce compteur), et **ni dump ni IN 21 ne sont requis** — on tire la
//! séquence. Donc ici on rend tout cohérent sur `live_write` :
//!
//!   focus(`1d`) = **L** · ed:08(`08`) = **L+0x11** · bulk(`27`) = **L+0x11**   (exactement HX :
//!   `6e7d` · `6e8e` · `6e8e`), puis on **laisse la session vivante** gérer la suite — on
//!   n'envoie PAS les 15 ACK figés (`6f19`→`7ac0`) qui causaient le crash.
//!
//! Bulk cœur : cab1/cab2 `cd031c`/`cd031c` (capture HX `cab dual change right.json`). Test
//! `cd031b` en cab2 : pas d'assign + UI HW buguée (sans crash) — le device veut l'identité dual.
//!
//! ```js
//! await change_cab2(0)              // focus + ed:08 + bulk48 cohérents, sans post-séquence figée
//! ```
//! Témoin `HX_CONSOLE_CAB2_FULL=1` : renvoie aussi la post-séquence (compteurs cohérents), pour
//! comparer si le device en a besoin. À n'utiliser que si le minimal ne « commit » pas le cab2.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::helix::packet::OutPacket;
use crate::helix::{kempline_index_to_slot_bus, HelixState};

/// Corps HX Edit `cab dual change right.json`. Compteurs RE-CALCULÉS à l'envoi (cf. patch).
/// #0 focus `1d` · #1 ed:08 `08` · #2 bulk replace Cab 2 `27` (48 o) · #3.. post-séquence.
const CHANGE_CAB2_SEQUENCE: &[(&[u8], u64)] = &[
    (
        &[
            0x1d, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x99, 0x00, 0x04, 0x7d, 0x6e, 0x00,
            0x00, 0x01, 0x00, 0x06, 0x00, 0x0d, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04, 0x59, 0x64,
            0x4e, 0x65, 0x82, 0x62, 0x01, 0x1a, 0x01, 0x00, 0x00, 0x00,
        ],
        0,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x9a, 0x00, 0x08, 0x8e, 0x6e, 0x00,
            0x00,
        ],
        93,
    ),
    (
        &[
            0x27, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x9c, 0x00, 0x04, 0x8e, 0x6e, 0x00,
            0x00, 0x01, 0x00, 0x06, 0x00, 0x17, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04, 0x5a, 0x64,
            0x28, 0x65, 0x82, 0x62, 0x01, 0x64, 0x83, 0x17, 0xc3, 0x19, 0xcd, 0x03, 0x1c, 0x1a, 0xcd,
            0x03, 0x1c, 0x00,
        ],
        400,
    ),
    // ── post-séquence (envoyée seulement si HX_CONSOLE_CAB2_FULL=1) ──
    (
        &[
            0x19, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x9d, 0x00, 0x0c, 0x19, 0x6f, 0x00,
            0x00, 0x01, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04, 0x5b, 0x64,
            0x17, 0x65, 0xc0, 0x00, 0x00, 0x00,
        ],
        140,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x02, 0x10, 0xf0, 0x03, 0x00, 0x7c, 0x00, 0x08, 0x38, 0x29, 0x00,
            0x00,
        ],
        0,
    ),
    (
        &[
            0x19, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x9e, 0x00, 0x0c, 0x4a, 0x6f, 0x00,
            0x00, 0x01, 0x00, 0x06, 0x00, 0x09, 0x00, 0x00, 0x00, 0x83, 0x66, 0xcd, 0x04, 0x5c, 0x64,
            0x16, 0x65, 0xc0, 0x00, 0x00, 0x00,
        ],
        32,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0x9f, 0x00, 0x08, 0x4a, 0x70, 0x00,
            0x00,
        ],
        15,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa0, 0x00, 0x08, 0x4a, 0x71, 0x00,
            0x00,
        ],
        3,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa1, 0x00, 0x08, 0x4a, 0x72, 0x00,
            0x00,
        ],
        0,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa2, 0x00, 0x08, 0x4a, 0x73, 0x00,
            0x00,
        ],
        1,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa3, 0x00, 0x08, 0x4a, 0x74, 0x00,
            0x00,
        ],
        0,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa4, 0x00, 0x08, 0x4a, 0x75, 0x00,
            0x00,
        ],
        1,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa5, 0x00, 0x08, 0x4a, 0x76, 0x00,
            0x00,
        ],
        0,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa6, 0x00, 0x08, 0x4a, 0x77, 0x00,
            0x00,
        ],
        1,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa7, 0x00, 0x08, 0x4a, 0x78, 0x00,
            0x00,
        ],
        0,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa8, 0x00, 0x08, 0x4a, 0x79, 0x00,
            0x00,
        ],
        1,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xa9, 0x00, 0x08, 0x4a, 0x7a, 0x00,
            0x00,
        ],
        0,
    ),
    (
        &[
            0x08, 0x00, 0x00, 0x18, 0x80, 0x10, 0xed, 0x03, 0x00, 0xaa, 0x00, 0x08, 0xc0, 0x7a, 0x00,
            0x00,
        ],
        7,
    ),
];

/// Nombre de paquets « cœur » (focus + ed:08 + bulk) toujours envoyés.
const CORE_LEN: usize = 3;

/// `HX_CONSOLE_CAB2_FULL=1` : envoie aussi la post-séquence (compteurs cohérents).
fn console_cab2_full() -> bool {
    matches!(
        std::env::var("HX_CONSOLE_CAB2_FULL").as_deref(),
        Ok(v) if matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
    )
}

fn patch_slot_bus_only(buf: &mut [u8], slot_bus: u8) {
    for i in 0..buf.len().saturating_sub(2) {
        if (buf[i] == 0x82 || buf[i] == 0x81) && buf[i + 1] == 0x62 {
            buf[i + 2] = slot_bus;
            return;
        }
    }
}

fn patch_lane_after_cd03_or_cd04(buf: &mut [u8], kempline_index: usize) {
    let lane = (kempline_index as u8).saturating_mul(2);
    for i in 0..buf.len().saturating_sub(5) {
        if buf[i] == 0x83
            && buf[i + 1] == 0x66
            && buf[i + 2] == 0xcd
            && (buf[i + 3] == 0x03 || buf[i + 3] == 0x04)
        {
            buf[i + 4] = lane;
            return;
        }
    }
}

/// Compteur cohérent sur la **lane modèle = `live_write_ctr` (= L)** :
///   focus `1d`  → L
///   ed:08 `08`  → L + 0x11
///   bulk  `27`  → L + 0x11   (= ed:08, exactement HX `6e7d`/`6e8e`/`6e8e`)
/// Tout autre `80:10:ed:03` (post-séquence `19`/`08`) → L + 0x11 par défaut.
fn ed03_ctr_for_head(state: &HelixState, head: u8) -> u16 {
    let l = state.live_write_ctr;
    match head {
        0x1d => l,
        _ => l.wrapping_add(0x11),
    }
}

fn patch_packet_live(state: &mut HelixState, pkt: &mut [u8], slot_bus: u8, kempline_index: usize) {
    patch_slot_bus_only(pkt, slot_bus);

    // Tous les `80:10:ed:03` (focus, ed:08, bulk) : compteur cohérent sur live_write.
    if pkt.len() >= 14 && pkt[4..8] == [0x80, 0x10, 0xed, 0x03] {
        pkt[9] = state.next_x80_cnt();
        let ctr = ed03_ctr_for_head(state, pkt[0]);
        pkt[12] = (ctr & 0xff) as u8;
        pkt[13] = ((ctr >> 8) & 0xff) as u8;
        if pkt.len() > 16 {
            patch_lane_after_cd03_or_cd04(pkt, kempline_index);
        }
        return;
    }

    // Courts ACK firmware (`f0:03` / `ef:03`) : on ne touche QUE le seq (lane firmware propre).
    if pkt.len() == 16 && pkt[0] == 0x08 {
        let seq = match &pkt[4..8] {
            [0x02, 0x10, 0xf0, 0x03] => state.next_x2_cnt(),
            [0x01, 0x10, 0xef, 0x03] => state.next_x1_cnt(),
            [0xef, 0x03, 0x01, 0x10] => state.next_x2_cnt(),
            _ => state.next_x80_cnt(),
        };
        pkt[9] = seq;
    }
}

fn send_timed_sequence(
    state: &mut HelixState,
    slot_bus: u8,
    kempline_index: usize,
    seq: &[(&[u8], u64)],
) -> u32 {
    let mut sent = 0u32;
    // Le bulk porte le compteur modèle (L+0x11) ; on aligne live_write_ctr dessus pour
    // que la session vivante reprenne sur la bonne lane après le bulk.
    for (data, delay_ms) in seq {
        if *delay_ms > 0 {
            thread::sleep(Duration::from_millis(*delay_ms));
        }
        let mut pkt = data.to_vec();
        patch_packet_live(state, &mut pkt, slot_bus, kempline_index);
        eprintln!(
            "[HxConsole] OUT #{} head=0x{:02x} len={} ctr={:02x}{:02x}",
            sent + 1,
            pkt.first().copied().unwrap_or(0),
            pkt.len(),
            pkt.get(13).copied().unwrap_or(0),
            pkt.get(12).copied().unwrap_or(0),
        );
        state.send(OutPacket::new(pkt));
        sent += 1;
    }
    // Après le bulk, la lane modèle est à L+0x11 : on y cale live_write_ctr.
    state.live_write_ctr = state.live_write_ctr.wrapping_add(0x11);
    sent
}

fn slot_bus_for_index(slot_index: u32) -> Result<u8, String> {
    if slot_index >= 16 {
        return Err("slotIndex hors plage (0..15)".to_string());
    }
    kempline_index_to_slot_bus(slot_index as usize).ok_or_else(|| "slotIndex invalide".to_string())
}

fn helix_from_app(state: &Arc<Mutex<crate::AppState>>) -> Result<Arc<Mutex<HelixState>>, String> {
    let app = state.lock().map_err(|e| e.to_string())?;
    app.helix_state
        .clone()
        .ok_or_else(|| "HX non connecté".to_string())
}

/// Slot **dual** déjà présent : replace Cab 2, lane cohérente sur live_write.
#[tauri::command]
pub fn hx_console_change_cab2(
    state: tauri::State<Arc<Mutex<crate::AppState>>>,
    slot_index: u32,
) -> Result<String, String> {
    let slot_bus = slot_bus_for_index(slot_index)?;
    let helix = helix_from_app(state.inner())?;
    let full = console_cab2_full();
    let n = if full { CHANGE_CAB2_SEQUENCE.len() } else { CORE_LEN };
    eprintln!(
        "[HxConsole] change_cab2 slot_index={slot_index} slot_bus=0x{slot_bus:02x} full={full} packets={n}"
    );
    let sent = thread::spawn(move || {
        let mut st = helix.lock().unwrap();
        let l = st.live_write_ctr;
        eprintln!(
            "[HxConsole] lane modèle L=live_write_ctr={:#06x} -> focus={:#06x} ed08/bulk={:#06x}",
            l,
            l,
            l.wrapping_add(0x11)
        );
        send_timed_sequence(&mut st, slot_bus, slot_index as usize, &CHANGE_CAB2_SEQUENCE[..n])
    })
    .join()
    .map_err(|_| "thread change_cab2 interrompu".to_string())?;
    Ok(format!(
        "change_cab2 OK — {sent} paquets OUT cohérents (focus=L, ed:08/bulk=L+0x11), slot_bus=0x{slot_bus:02x}, full={full}"
    ))
}