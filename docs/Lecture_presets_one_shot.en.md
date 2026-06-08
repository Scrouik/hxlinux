# HX Linux — Preset reading: one-shot bootstrap

> **In one sentence**: full preset reading is a **one-shot bootstrap**. The complete dump (272×N) and the list of 125 names are read **only once per USB connection**, in a fixed sequence. After that, the editor runs in `Standard` mode and performs only **targeted** reads (body of the active preset on change). Direct consequence: a failure in that single sequence is **fatal for the entire session** — there is no automatic re-dump.

**French (source):** [Lecture_presets_one_shot.md](./Lecture_presets_one_shot.md)

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
preamble : 92o(54) → 40o(1f) → 68o(39)        ctr 1a:02 … 3f:02   (pre-dump handshake)
dump      : 11 × 272o   head=08                 ctr 50:02
trailer   : 1 × 140o    head=84  sub=04  len<272 ctr 50:02          ← END OF DUMP
```

**Trailer size depends on the active preset** (= total size modulo the chunk boundary). It was `140/84` on one run, but elsewhere it will be `132/7a`, `116/6a`, … It must therefore be recognized **by its nature** — a data chunk (`sub=0x04`) shorter than 272 — never by a hard-coded length, or you get preset-dependent intermittency.

## 3. “Go-live”: why the trailer is critical

The trailer triggers **PHASE B** (post-dump editor dialogue: `1b 76:0e`, `1c 76:cc`, `1a`, `19 ed/ef`…). PHASE B **wakes** the device into editor mode. Until it has run:

- the device stays **alive** (it pings `50:02` / `09:02`),
- but it **ignores** read requests (names via `1d`, body via `19`).

So: **trailer recognized → PHASE B → go-live → reads served.** Trailer missed → device silent on reads, even though it looks alive.

## 4. Why a failure is fatal (and looked “intermittent”)

Because everything is **one-shot**, there is no second attempt in the session: if the active preset’s trailer is not recognized, bootstrap times out (phase 4 gate, 3500 ms), settle is forced, the editor never goes “live”, names come back **empty**, and body reads spin on the watchdog.

Hence the symptom “works N times then suddenly nothing”: each connection gets **only one try**, and the outcome depends on the **active preset** at plug-in. A preset whose dump ends on a recognized size passes; another freezes the whole session. Rebooting the Stomp does not help — it was a **host-side** recognition gap.

## 5. Counters (ED03 lane)

The entire editor dialogue — **requests and** chunk ACKs — must run on **a single lane**:

| Lane (bytes 12–13) | Usage | Progression |
|---|---|---|
| `editor_ed03_lane` | `19`/`1b`/`1c` requests + dump chunk ACKs | `9d:10 → 9d:11 → … → 9d:1b` (lo fixed, hi +1/chunk) |

Past bug: acknowledging chunks on a **separate** lane stuck at `f4:1d`. The device, which strictly validates `19` traffic, rejects lane discontinuity. **Aligning both sub-counters (lo + hi) at the same time is mandatory** — fixing only one fails silently.

## 6. Rules not to re-learn

- **Trailer = partial chunk**, never a hard-coded length (otherwise preset-dependent intermittency).
- **One-shot = no safety net**: protect bootstrap, because no re-dump recovers a mid-session failure.
- **Go-live first**: the device serves reads only after PHASE B; a device that pings is not a device that is ready.
- **Single lane**: dump requests and ACKs on `editor_ed03_lane`, lo + hi aligned together.

---

*Preset reading is not an operation you restart: it is a unique initiation rite per connection. Anything that can break this sequence must be treated as critical, because there will be no second chance until the next plug-in.*
