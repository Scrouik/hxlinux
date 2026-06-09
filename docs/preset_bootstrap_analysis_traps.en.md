# HX Linux — Preset bootstrap: analysis traps and field lessons

*One-shot per connection — no second chance if the phase 4 envelope is misrecognized.*

> **French (source):** [preset_bootstrap_analysis_traps.md](./preset_bootstrap_analysis_traps.md) · **Graceful USB close:** [quitter_sans_figer_hardware.en.md](./quitter_sans_figer_hardware.en.md)

> **In one sentence**: full preset reading is a **one-shot bootstrap**. The complete dump (272×N) and the list of 125 names are read **only once per USB connection**, in a fixed sequence. After that, the editor runs in `Standard` mode and performs only **targeted** reads (body of the active preset on change). Direct consequence: a failure in that single sequence is **fatal for the entire session** — there is no automatic re-dump. Corollary (see §7): **any hard-coded envelope recognition** (preamble or trailer) is a time bomb, because envelope shape depends on the **active preset at plug-in**.

## 1. The sequence, and what happens only once

Bootstrap chain (one run per plug-in):

```
Connect → ReconfigureX1 → amorcage (phase 4 + settle)
        → RequestPresetNames → RequestPresetName → RequestPreset → Standard
```

| Step | Role | Frequency |
|---|---|---|
| phase 4 (272×N dump + trailer + PHASE B) | bring the editor “live”, clear initial state | **once / connection** |
| RequestPresetNames | read the 125 names | **once / connection** |
| RequestPresetName / RequestPreset | name + body of active preset | once at bootstrap, then **on each preset change** |
| Standard | runtime: ACK, HW/UI events, targeted reads | continuous |

So “reading happens only once” means **the bootstrap dump + the name list**. A preset **body** is re-read on each selection — but in a targeted way, not as a full re-dump.

## 2. The phase 4 dump and its trailer

The device pushes the dump as a burst of 272-byte chunks, **closed by a partial chunk** (the “trailer”):

```
preamble : 92o(54) → 40o(1f) → 68o(3x)         ctr 1a:02 … 3f:02   (pre-dump handshake)
dump      : N × 272o   head=08  sub=04           ctr 50:02
trailer   : 1 × <272o  head=xx  sub=04  len<272  ctr 50:02          ← END OF DUMP
```

**Trailer size depends on the active preset** (= total size modulo the chunk boundary). Shapes seen in captures: `140/84`, `132/7a`, `116/6a`, but also `104/5f`, `224/d7`, `28/14`, … It must therefore be recognized **by its nature** — a data chunk (`sub=0x04`) shorter than 272 — never by a hard-coded length, or you get preset-dependent intermittency. **The same rule applies to the preamble** (see §7): its size and head vary too.

**Shared pattern (preamble or trailer)** — structural predicate, not a head list:

```
ed:03:80:10, sub[11]=0x04, 17 ≤ len < 272, not keepalive 16o (sub=10/00)
```

Implemented in `phase4_state.rs` (FSM) and `is_preset_dump_stream_chunk_in` (`preset_dump_stream_ack.rs`) for full 272 chunks.

## 3. “Go-live”: why the trailer is critical

The trailer triggers **PHASE B** (post-dump editor dialogue: `1b 76:0e`, `1c 76:cc`, `1a`, `19 ed/ef`…). PHASE B **wakes** the device into editor mode. Until it has run:

- the device stays **alive** (it pings `50:02` / `09:02`),
- but it **ignores** read requests (names via `1d`, body via `19`).

So: **trailer recognized → PHASE B → go-live → reads served.** Trailer missed → device silent on reads, even though it looks alive.

> **Note (see §8.2)**: the trailer is **not ACKed** by the host (its head — `84`, `5f`, … — does not match the ACK pattern `08:01`). Go-live therefore does **not** depend on a terminal ACK: it depends on **FSM recognition** of the trailer. Classic trailers are not ACKed either, and go-live still succeeds.

## 4. Why a failure is fatal (and looked “intermittent”)

Because everything is **one-shot**, there is no second attempt in the session: if the **preamble or** trailer of the active preset is not recognized, bootstrap times out (phase 4 gate, 3500 ms), settle is forced, the editor never goes “live”, names come back **empty**, and body reads spin on the watchdog.

Hence the symptom “works N times then suddenly nothing”: each connection gets **only one try**, and the outcome depends on the **active preset** at plug-in. A preset whose envelope (preamble/trailer) matches a recognized shape passes; another freezes the whole session. Rebooting the Stomp does not help — it was a **host-side** recognition gap.

## 5. Counters (ED03 lane)

The entire editor dialogue — **requests and** chunk ACKs — must run on **a single lane**:

| Lane (bytes 12–13) | Usage | Progression |
|---|---|---|
| `editor_ed03_lane` | `19`/`1b`/`1c` requests + dump chunk ACKs | `9d:10 → 9d:11 → … → 9d:1b` (lo fixed, hi +1/chunk) |

Past bug: acknowledging chunks on a **separate** lane stuck at `f4:1d`. The device, which strictly validates `19` traffic, rejects lane discontinuity. **Aligning both sub-counters (lo + hi) at the same time is mandatory** — fixing only one fails silently.

**Witness revert (HW debug only)**: `HX_DUMP_ACK_LANE=f4` forces dump ACKs onto the fixed lane `f4:1d` instead of `editor_ed03_lane`. Useful to compare a historically broken run; do not leave enabled in production.

## 6. Rules not to re-learn

- **Trailer = partial chunk**, never a hard-coded length (otherwise preset-dependent intermittency).
- **Preamble = partial chunk too** (see §7), never a hard-coded head/len list — that was the last place still listing heads (`Waiting68o`).
- **One-shot = no safety net**: protect bootstrap, because no re-dump recovers a mid-session failure.
- **Go-live first**: the device serves reads only after PHASE B; a device that pings is not a device that is ready. Go-live does **not** require a trailer ACK (see §3, §8.2).
- **Single lane**: dump requests and ACKs on `editor_ed03_lane`, lo + hi aligned together.
- **One shared definition of “272 dump chunk”** between the FSM and `preset_dump_stream_ack` (`is_preset_dump_stream_chunk_in`) — not two diverging patterns.

## 7. Snapshot variant: the preamble varies too

> **In one sentence**: a snapshot preset uses the **same phase 4 envelope** (partial preamble → N×272 → partial trailer), but the **preamble** can take an unexpected shape. The `Waiting68o` gate only recognized `68o head=39|3c`; missing variants meant the FSM never reached `WaitingDump`, the trailer (already recognized structurally) was never evaluated → no PHASE B → one-shot session lost.

### 7.1 Symptom

Boot with an active snapshot preset (e.g. **TX WOODY BLUE**, index 27) → `125 slots, 0 non-empty`, empty UI, `WARN timeout phase4 (3500 ms)`. The dump arrived **fully** on the wire (272 chunks ACKed `9d:10 … 9d:1b`), the trailer too — but the FSM stayed stuck in `Waiting68o`, silently (that state did not even log ignored IN packets).

### 7.2 Observed shapes

On two runs, **three preamble shapes** and several trailers — all partial `ed` chunks (`sub=0x04`, `len<272`), only size and head change:

| Element | Classic | Observed variants |
|---|---|---|
| Preamble (post-`1f`) | `68o head=39\|3c` | `68o head=3b`, `72o head=3e` |
| Trailer (end of dump) | `140/84`, `132/7a`, `116/6a` | `104/5f`, `224/d7`, `28/14` |

A hard-coded head/len list misses them one after another — exactly the trap already identified for the trailer (§2), but this time on the **preamble**.

### 7.3 Trace (snapshot run, after fix)

**Verbatim logs** (`HX_INIT_TRACE=1`) — `[phase4_fsm]` lines remain French in the binary:

```
[phase4_fsm] Waiting1fA -> Waiting68o (IN len=40 head=1f)
IN 68o : 3b 00 00 18 ed 03 80 10 00 06 00 04 …            ← preamble (sub=04, partial)
[phase4_fsm] Waiting68o — préambule 68o head=3b (chunk partiel ed) → WaitingDump
N × IN 272o : 08 01 00 18 ed 03 80 10 … 04 …             ← dump chunks (ACKed 9d:10…9d:1b)
IN 104o : 5f 00 00 18 ed 03 80 10 … 04 … (…SNAPSHOT 4…)  ← trailer (partial)
[phase4_fsm] trailer 104o head=5f (chunk partiel) → PostArm (PHASE B proactive)
…
[RequestPresetNames] finish_transfer: 125 slots, 125 non-empty
```

Before the fix, the same preset could push a `224o head=d7` trailer — already structurally valid, but never reached because the FSM stayed in `Waiting68o`.

### 7.4 The fix (structural, not a head list)

In `phase4_state.rs`, from `Waiting68o`:

| State | Old rule (fragile) | New rule (structural) |
|---|---|---|
| `Waiting68o → WaitingDump` | `len==68 && head∈{39,3c}` | **(a)** partial `ed` chunk: `sub==0x04`, `17≤len<272`, not keepalive; **OR (b)** first **272 chunk** recognized via `is_preset_dump_stream_chunk_in` (shared predicate) |

- **(a)** catches the preamble regardless of shape — symmetric with the trailer rule in `WaitingDump`.
- **(b)** safety net: if the preamble has a totally unexpected shape but the first real `272` chunk (`08:01…`) arrives, switch to `WaitingDump` anyway — the device started the dump, we *must* be there to catch the trailer. A 272 chunk is unambiguous (never confused with the partial trailer).
- No preamble/trailer ambiguity: structurally identical, but distinguished by **position** (preamble in `Waiting68o`, trailer in `WaitingDump`). That is the FSM’s job.
- `Waiting68o` added to the “IN ed ignore” log list + `sub` traced, so this state never freezes silently again.

*No ACK-layer change: `preset_dump_stream_ack` is unchanged. Unblocking snapshot bootstrap ≠ implementing snapshot editing — two separate workstreams.*

### 7.5 Remaining vigilance point

`WaitIn1b26` (end of PHASE B) has the **same historical fragility**: `len==68 && head∈{3c,39}` on the Linux path. The snapshot run avoided it (PHASE B completed via the HX path `1b/36o → 26/48o → Done`), so it did not bite — but another snapshot using the Linux path with an offset shape would stall there. **Treat it the same way (structural recognition) once the shape is observed in a capture**, not before.

## 8. Open notes (non-blocking)

### 8.1 Name index: sequential fallback, grid labeling potentially misaligned

`extract_preset_index` fails on **all** list records (`idx_6b=-1 idx_6c=-1`); we fall back to **sequential ordering**. The grid is no longer empty, but the fallback assumes *transfer order = slot order* — which is **wrong**: the byte after `81 cd 00` jumps (`12, 0f, 10, 0d, 00, 01, 02, 13, 26 …`). Names can therefore land on the **wrong slots** in the grid.

This did not show up because (a) the grid is populated and (b) the **active** preset name comes from another path (`RequestPresetName` → `6c cd 00 1b` → 27 = `TX WOODY BLUE`, correct), not from the list.

Trap for refinement: the byte after `81 cd 00` is **not directly the slot either** — `TX WOODY BLUE` carries `…00 25` (37) in the list, while the active request says `…00 1b` (27). **Two distinct index spaces** (`81 cd …` list vs `6c cd …` active) to disentangle on a dedicated capture. Record format: `81 cd 00 <?> 84 cd 00 6d <len> <name…>`.

*Bottom line: **loading** a preset is correct (`RequestPreset` uses the real index); **grid labeling** still needs hardening. More “latent display bug” than pure cosmetics.*

### 8.2 Bootstrap trailer not ACKed

The trailer (`5f:…`, `14:…`, …) is **not** ACKed: its head does not match the `08:01` pattern in `is_preset_dump_stream_chunk_in`. The FSM is enough for go-live (see §3), and **classic** trailers (`84`, `7a`, `6a`) are never ACKed either — so low risk. **Watch** only if a preset ever misbehaves without a missing terminal ACK.

### 8.3 `WaitingDump - IN ed ignore len=272`: expected behavior

This log is **normal**, not an anomaly. In `WaitingDump`, a `272` chunk does not match the trailer rule (`272` is not `< 272`) → it falls through to `else` → logged as “ignored”. Full 272s are dump chunks, not trailers; only the terminal partial chunk triggers `PostArm`.

## 9. Code map and field debug

| File | Role |
|---|---|
| `src-tauri/src/helix/phase4_state.rs` | Phase 4 FSM: preamble (`Waiting68o`), trailer (`WaitingDump`), PHASE B |
| `src-tauri/src/helix/preset_dump_stream_ack.rs` | Dump chunk ACKs on `editor_ed03_lane`; `HX_DUMP_ACK_LANE` |
| `src-tauri/src/helix/amorcage.rs` | Bootstrap chain → name / active preset requests |
| `src-tauri/src/helix/mod.rs` | Session orchestration, phase 4 gate (3500 ms) |

Useful environment variables for diagnosis:

| Variable | Effect |
|---|---|
| `HX_INIT_TRACE=1` | Phase 4 FSM trace (`[phase4_fsm]`, ignored IN + `sub`) |
| `HX_PRESET_DUMP_STREAM_ACK_DEBUG=1` | Dump ACK detail (lane, counters) |
| `PRESET_DEBUG_VERBOSE=1` | Name list, index, preset body |
| `USB_PACKET_TRACE=1` / `USB_PACKET_TRACE_BOOT=1` | Raw USB packets (boot) |

---

*Preset reading is not an operation you restart: it is a unique initiation rite per connection. Anything that can break this sequence must be treated as critical, because there will be no second chance until the next plug-in. And since the phase 4 envelope — preamble **and** trailer — changes shape with the active preset, we never recognize it by a hard-coded size or head: only by its nature (partial `ed` chunk) and its position in the FSM.*
