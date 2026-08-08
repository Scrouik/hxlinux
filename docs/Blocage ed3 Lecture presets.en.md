# Report — Reverse-engineering the `ed:03` protocol (HXLinux)

> **Context.** HXLinux is a native Linux editor for the Line 6 HX Stomp XL (Rust/Tauri). There is no Line 6 documentation: all understanding of the `ed:03` protocol comes from my own USB captures (usbmon / Wireshark → JSON). Reference implementation: `kempline/helix_usb`.
>
> **Working principle.** *Capture first, never speculate.* Every hypothesis is validated on trace before any code change; every behavior change is kept behind a witness flag (`=0` restores the old behavior); false leads are documented and closed the same way as confirmed findings.
>
> **Français:** [Blocage ed3 Lecture presets.md](./Blocage%20ed3%20Lecture%20presets.md)

---

## Overview

| # | Problem | Root cause | Fix | Flag (default ON) | Status |
|---|---|---|---|---|---|
| 1 | Bootstrap stalls, 0 presets read | Phase 4 FSM `Waiting68o` stuck on non-standard preamble shapes (snapshot presets) | Recognition **by nature** (partial chunk) instead of hardcoded heads/lengths | — (structural) | ✅ Resolved (125/125) |
| 2 | Freeze on multi-notch scroll | ED03 saturation + poorly coupled lane | Live `double`+`ctr` coupling + settling throttle | `HX_PULL_COUPLE_LANE` | ✅ Resolved |
| 3 | Drop past ~256 chunks (**BUG C**) | 16-bit chunk counter; carry ignored (byte 14 stuck at 0) | Byte 14 = true high byte, carry on byte 13 overflow | `HX_LANE_B14_CARRY` | ✅ Resolved (field confirmed) |
| 4 | Editor double desync on wrap (**BUG A**) | HX skips value `lo=0x00` on double wrap, we did not | Skip `0x00` on editor double `lo` | `HX_EDITOR_DOUBLE_SKIP_00` | ✅ Resolved |
| 5 | Drop at page turn `05→06` (**§5**) | “Live lane” subscription never armed: incomplete PHASE B closure | Commit `1b 0c f1` + wait for `23 04`; FSM without premature `Done` on `26 ef` | `HX_PHASEB_COMMIT` | 🟡 Handshake **log-validated**; field **pending** |
| 7 | Preset read freeze at ~19-21 reads | ED03 read transactions left **open**: end-of-dump trailer never ACKed | ACK the dump-end trailer (`OUT 08 80:10:ed:03 sub=08`) like HX → closes each transaction | `HX_ACK_DUMP_TRAILER` | ✅ **Resolved** (field: 60 reads, 0 freeze) |

---

## 1. Blocked bootstrap — `Waiting68o` and snapshot presets

**Symptom.** On connect, phase 4 FSM stalled in `Waiting68o`, hit timeout (3500 ms → forced settle), editor never went “live”, and **no preset names were read** (`125 slots, 0 non-empty`). Intermittent: worked on some presets, not others.

**Why.** Just before a dump, the device sends a “preamble” whose **size and head vary with the active preset**:
- classic preset: 68 o, head `39`/`3c`;
- snapshot preset: 68 o head `3b`, or 72 o head `3e`, etc.

Old code recognized this preamble via a **hardcoded list of heads/lengths**. When the active preset had a shape missing from the list, the FSM never switched to `WaitingDump` → stall. Same “frozen trailer” trap we had seen elsewhere.

**How.** We now recognize the preamble **by nature, not by values**: a **partial** `ed` chunk (`sub=0x04`, `17 ≤ len < 272`), excluding keepalive. The FSM resolves preamble vs trailer ambiguity not by shape (they are structurally identical) but by **position**: preamble arrives in `Waiting68o` (before any 272 chunk), trailer in `WaitingDump` (after). Safety net: if shape is unexpected but the first real 272 chunk (`08:01`) arrives, we still switch to `WaitingDump`.

> *General lesson, reused everywhere since: an FSM predicate must identify by nature, not frozen values.* Confirmed result: **125 slots, 125 non-empty**.

---

## 2. Freeze on multi-notch scroll

**Symptom.** Scrolling models (multi-notch scroll) caused freezes on fast multi-notch moves.

**Why.** Two coupled causes:
1. **Incomplete lane coupling.** Aligning *only one* of two coupled variables (editor `double` **and** `ctr`) is enough to fail — and worse, to draw a false root-cause conclusion. `ctr` had to be initialized to `0x6cbd` and `double` (`editor_ed03_double`) kept live.
2. **ED03 saturation.** Fast notches stacked transactions faster than the device drained its window (~300–400 ms), causing the freeze.

**How.**
- Coupling enabled via `HX_PULL_COUPLE_LANE=1` (live double + `ctr` init `0x6cbd`).
- Settling throttle: `post_pull_settling_ms()` defaults to **500 ms** when coalescing is active (`PULL_THROTTLE_SETTLING_MS=500`) — ~1.3–1.6× margin on the device drain window.

> *“Proposal A” (close transactions like HX Edit) was parked here: the device validates `19` strictly against its live lane, which made it impossible without cracking §5. Same wall we eventually attacked head-on in item 5.*

---

## 3. BUG C — byte 14 carry (chunk counter)

**Symptom.** Past a certain cumulative chunk count (~256), a read dropped hard mid-dump.

**Why.** During a dump, the host ACKs each 272 o chunk with an OUT `08 ed03 sub=08`. Three bytes carry position:

| byte | role |
|---|---|
| **12** | transaction position — **independent**, fixed during dump |
| **13** | chunk counter, **low** byte (lo) |
| **14** | chunk counter, **high** byte (hi) |

The chunk counter is **not** byte 13 alone: it is a **16-bit little-endian value on byte13 (lo) + byte14 (hi)**. HXLinux had byte14 **hardcoded to `0x00`** in all three ED03 ACK builders (`RequestPreset`, `preset_dump_stream_ack`, FDT). While byte13 stayed under `0xff`, it worked. But crossing `fe → ff → 00`, the counter must **carry into byte14**; stuck at 0, it fell back to a low value → desync → device aborts.

**Capture proof (HX).** byte12 fixed, carry in byte14:

```
95 fe 00      byte12=95   byte13=fe   byte14=00
95 ff 00                  byte13=ff   byte14=00
95 00 01   ← byte13 wraps to 00, byte14 CARRIES → 01
```

**How.** byte14 becomes a real high byte, incremented on each byte13 overflow (`ff→00`). Counter `advance` returns `[byte12, byte13, byte14]`. Behind `HX_LANE_B14_CARRY` (`=0` = byte14 stuck at 0 = original bug).

> *Field confirmed: crossing passed packet-for-packet (`c5:ff:00 → c5:00:01`), full dump. Two false leads closed by the same capture: carry did **not** go in byte12, and there was **no** `0x00` skip on byte13.*

> *Do not confuse with §5: BUG C = chunk counter past `0xff`; §5 = live lane subscription at connect.*

---

## 4. BUG A — skipping `0x00` on editor double

**Symptom.** Editor `double` desync at wrap, causing read failure a few notches too early.

**Why.** Editor `double` is a 16-bit value with `hi` pinned at `0x64`. On `lo` wrap, **HX skips value `0x00`**: it goes `0x64ff → 0x6401`, never `0x6400`. HXLinux emitted `0x6400` → one-notch offset from what the device expects.

**How.** On `lo` wrap, if `lo == 0x00` then `lo = 0x01`. Behind `HX_EDITOR_DOUBLE_SKIP_00` (and hi pin `0x64` via `HX_EDITOR_DOUBLE_PIN_HI`).

> *Field confirmed: without skip, failure at read 19; with skip, reads survive to 23+. **This was not** the underlying drop cause — HX fidelity alignment, distinct from §5.*

---

## 5. §5 — “live lane” and drop at page turn

**Symptom.** Once BUG C and BUG A were fixed, reads crossed the chunk counter wrap but **still dropped** (historically observed) when the device page counter (IN lane, byte13) went **`05 → 06`** — regardless of read count, double value, or preset content.

**Why.** Measured on HX captures:
- HX receives **exactly one heartbeat `19 04` per read** (25 reads = 25 heartbeats), on the device live lane (`hi = 0x67`);
- HXLinux received **none** until PHASE B commit was correctly completed.

OUT packets in read regime (Phase-1/2, ACK, keepalive) are **byte-for-byte identical** between HX and HXLinux. §5 is therefore **not** a missing per-read packet: it is a **mode/subscription to arm once at connect**. Without it, lane `0x67` sleeps, no heartbeats, and the device drops at page turn.

**What captures show (not local cache).** HX Edit does not consult a table of all 125 preset bodies in RAM: on each UI preset switch, capture `02_change_preset_*_HXEdit.json` shows the same two-phase `RequestPreset` sequence (`19 ed:03` phase 1 then phase 2 + chunks). Only **phase 4 bootstrap dump** and **125-name list** are one-shot per connection (see [`preset_bootstrap_analysis_traps.md`](./preset_bootstrap_analysis_traps.md)).

### 5.1 Initial diagnosis — missing commit

PHASE B was reached, but the old FSM ended too early:

| frame | dir | packet | double | note |
|---|---|---|---|---|
| f389 | OUT | `1b` sub=**04** | 0x64ec | `ec`, `76:0e` — HX uses `sub=0c` (divergence 1) |
| f413 | OUT | `19` sub=0c | 0x64**f0** | ed finalization |
| f415 | OUT | `19` sub=04 | 0x64e9 | ef finalization |
| **f417** | **IN** | **`1b` sub=04 ep=ed** | **0x67f0** | **device `1b 04 f0`** |
| f419 | IN | `26` sub=04 ep=ef | 0x67e9 | → old FSM: `Done` here ❌ |

On HX, on f417, the host **replies** with `1b 0c f1` (queue `81 76 0f 00`) and waits for **`IN 23 04 ed`**. **This closure round = commit that arms persistent subscription.**

**First fix wave** (`HX_PHASEB_COMMIT`, default ON):
1. proactive `ec` in `sub=0c` (like HX and `ed`/`ee` siblings);
2. `PbCommit` state: emit `1b 0c f1` on device `IN 1b 04 ed`;
3. alternate Linux path: `IN 68o ed` → `PbCommit` (instead of direct `Done`).

Files: `phase4_state.rs`, `usb_listener.rs`.

### 5.2 Second wave — the `26 ef` trap (design flaw fixed)

After the first wave, commit **was sent** but the FSM still closed **without subscription confirmation** — reproducing §5 via another path.

**Log symptom (field).** ~2 ms after sending commit:

```
WaitIn1b26 -> PbCommit (IN 1b 04 ed device)
OUT 1b 0c f1 (commit)
PbCommit -> Done (IN 26/48o ef)    ← false positive
```

**Reasoning.** `IN 26 ef` (f419) is the **echo of `19 ef` finalization**, often already in flight when the device emits its `1b 04 f0`. It is **not** commit confirmation. Accepting it as a “safety net” in `WaitIn1b26` **or** `PbCommit` sent commit then closed PHASE B on the echo → subscription **unconfirmed** — exactly the §5 we wanted to kill.

**Fixes (June 2026, commit `f09f12c`)**:

| State | Before | After |
|---|---|---|
| `WaitIn1b26` + `IN 26 ef` | immediate `Done` (or before device `1b`) | **Stay** waiting; log “1/2, commit pending device 1b” |
| `WaitIn1b26` + `IN 1b 04 ed` | partial | → `PbCommit` → `OUT 1b 0c f1` |
| `PbCommit` + `IN 26 ef` | `Done` (safety net) | **Ignored** — explicit log |
| `PbCommit` + `IN 23 04 ed` | — | **`Done`** (HX confirmation) |
| Fallback timeout | armed mainly at `PostArm` | re-armed at `PostArm`, `WaitIn1b26`, `PbCommit` (2 s) |

**Hardware-validated sequence (`InitTrace` log)**:

```
WaitIn1b26 -> PbCommit (IN 1b 04 ed device, HX commit)
OUT 1b 0c f1 (commit) lane=10:1e double=f1:64
PbCommit — IN 26 ef ignored (waiting 23 04 ed, f419 echo?)
PbCommit -> Done (IN 23 04 ed, commit confirmed)   ~11 ms after commit
```

➜ **PHASE B handshake faithful to HX, validated packet-for-packet.** This proves commit is well formed and recognized by the device. **It does not yet prove** that `19 04` heartbeats (lane `0x67`) appear on each read or that `05→06` holds.

### 5.3 §5 confidence levels

| Level | Claim | Status |
|---|---|---|
| Log proven | `1b 0c f1` sent; `26 ef` ignored; `23 04 ed` received; clean `Done` | ✅ |
| Strong hypothesis | Live lane subscription armed → heartbeats + `05→06` hold | 🟡 **field confirmation pending** |
| Watch | `23 04` must arrive on **every** connect (else 2 s timeout) | ⚠️ monitor |
| Watch | Linux path `68o ed → PbCommit`: hardcoded heads/len like old `Waiting68o` | ⚠️ structural recognition if shape observed |

**Decisive test (still pending).** Connect + ~25 reads. Three criteria:
1. `1b 0c f1` + `23 04` on every connect;
2. `IN 19 04` heartbeats lane `0x67` (one per read);
3. `05→06` passage without drop.

Until (2) and (3) are green, we do not declare §5 **operationally** fixed — only the **handshake** validated.

---

## 6. False safety nets and false leads (closed)

| Lead | Verdict |
|---|---|
| Chunk carry on **byte 12** (old `HX_LANE_HI_CARRY`) | ❌ Refuted — `out_only.txt` capture |
| “Skip 0x00” on byte13 alone | ❌ Refuted — same capture |
| HX Edit = cache of all 125 presets in RAM | ❌ Captures show `RequestPreset` on each UI switch |
| `reset_editor_ed03_lane()` in `force_recover` | ❌ Removed — chunk counter (13+14) is **global**; host-only reset worsens desync (§3) |
| Perceived slowness = USB protocol | ❌ Mostly host latency (200 ms poll, throttles); USB dump comparable to HX |

---

## 7. Preset read freeze at ~19-21 reads — unclosed ED03 transactions (RESOLVED)

**Symptom.** After ~19-21 consecutive preset reads, the dump truncates then the device stops
responding to `19 sub=04` entirely (`bytes=0`), while staying alive (keep-alives keep flowing).
Only a full USB re-open (app restart) revives it. Independent of preset content and scroll
direction; there is no literal `20` anywhere in the code (host or frontend).

**Why (proven byte-for-byte — `change_preset_and_freeze_linux.json` vs `60_changes_hxedit.json`).**
After the last 272-byte chunk, the device sends an **end-of-dump trailer**:
`XX:00:00:18 ed:03:80:10 00:cc:00:04 …` (~224–240 bytes, head `XX` = a session byte ≠ `0x08`,
`data[1]==0x00` where a normal chunk is `08:01`). **HX Edit ACKs this trailer** with
`OUT 08 80:10:ed:03 sub=08` — that **closes** the read transaction. We did not (it fell into the
`_ =>` default; the dump ended via watchdog) → **every read left a transaction open** → the
device’s internal window **saturates at ~19–21 open transactions** and stops serving reads.

> **Device constraint (general).** The HX Stomp XL tolerates only **~19–21 unclosed ED03 read
> transactions**. Any operation that triggers an ED03 dump **must ACK its end-of-dump trailer**,
> or they accumulate and the device freezes. Same mechanism as the multi-notch scroll freeze
> (§2 / Addendum §10), here on the preset-read path.

**Fix (`HX_ACK_DUMP_TRAILER`, default ON).** A new arm in `RequestPreset::data_in` detects the
trailer (guarded by `await_dump_end_after_full_chunk`, so it never matches the 56-byte Phase 1
response), appends its payload (also fixes an occasionally-incomplete preset just before the
freeze), sends the closing ACK, and finishes the transfer. Isolated from the scroll path
(RequestPreset mode only; multi-notch uses the separate `ScrollModelPull` pipeline layer).

> *Field result: 60 reads, 62 presets loaded, **0 freeze, 0 recovery**. Multi-notch verified: no
> regression. This operationally achieves what §5 aimed at (sustained reads) via the actual
> missing signal — the trailer ACK — not the live-lane subscription. Lesson: measure the **whole**
> post-dump sequence (not just the request) before coding; a byte-diff cracked what ~10 blind
> attempts (poll cadence, cd/lane counters, endpoint `clear_halt`, reader, recovery) could not.*

---

## Incident — version skew (method lesson)

When applying the §5 fix, I delivered a `phase4_state.rs` based on a **stale on-disk copy**. Consequence: your **`Waiting68o` fix (item 1) was overwritten** → bootstrap stalled again, *“it reads nothing”*, back to `0 non-empty`.

The tell was a compile error: your `usb_listener.rs` called `handle_in_passive(&mut s, …)` (signature `&mut HelixState`), while my copy had `handle_in_passive(&mut Phase4Step, …)`. **That signature mismatch alone proved my base was older than yours.** Fix: rebase all additions on **your** exact current file.

> *Lesson: always start from the repo’s real file, never an older working copy. A signature mismatch between two delivered files is a version-skew signal to handle first, not to work around.*

---

## Consolidated principles

- **Capture first.** No hypothesis without trace; false leads are documented and formally closed.
- **Multi-variable coupling.** Aligning only one of two coupled variables fails *and* manufactures false root causes (demonstrated on scroll).
- **Device asymmetries.** It validates `19` strictly against live lane, but serves `1b` loosely — asymmetry that blocked Proposal A.
- **Systematic flag-gating** with witness (`=0` restores old behavior).
- **Explicit confidence levels.** Handshake validated ≠ operational behavior validated.
- **Predicates by nature, not frozen values** (generalization of `Waiting68o` fix).
- **A poorly chosen FSM “safety net” can recreate the bug** (`26 ef` case in `PbCommit`).

---

## Current status & next steps

| Topic | Status |
|---|---|
| BUG C (byte 14) | ✅ Closed, field confirmed |
| BUG A (0x00 skip) | ✅ Closed |
| `Waiting68o` block | ✅ Closed (125/125) |
| Scroll freeze | ✅ Closed |
| §5 handshake (`1b 0c f1` → `23 04`) | ✅ Field log validated |
| §5 operational (heartbeats, `05→06`, ~25 reads) | 🟡 **Pending** |
| Watch `23 04` on every connect | ⚠️ Monitor |
| Watch snapshot on `68o → PbCommit` path | ⚠️ If commit missing, check IN shape |

*Next action: chain ~25 preset changes; verify `19 04` heartbeats lane `0x67` and hold at `05→06` turn. If yes → §5 operationally closed; else diagnose on a base with a healthy handshake.*
