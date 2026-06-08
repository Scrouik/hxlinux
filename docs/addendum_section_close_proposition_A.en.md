## 11. Proposal A (HX Edit-style closure) — TESTED, failure on synthetic lane

> **Status: ABANDONED (hardware), code tested locally then removed / not committed
> (see §11.6; intended flag `HX_PULL_CLOSE_LIKE_HXEDIT`, OFF by default).** We implemented and tested on the pedal HX Edit-style transaction
> closure (`19#1` at computed `ctr` → 68-byte echo → `19#2`). Result without appeal:
> **the echo never arrives** (the device rejects our `19` on the synthetic lane) and the
> orphan **`19#1` accelerates the freeze**. This is **the §5 wall coming back**: as long as
> the `1b` is not on the device's live lane, the `19` is unreceivable. We **stay on grab-53 +
> throttle** (§10), the validated stable config. The closure formula is correct and remains
> documented below for whoever cracks §5.
>
> **French (source):** [addendum_section_close_proposition_A.md](./addendum_section_close_proposition_A.md)
>
> **Complements:** §6 (post-dump freeze), §10 (throttle mitigation). **Captures:**
> `stomp_running_start_hxedit_2_multi_notch.json` (HX reference), §10.6
> `stomp_running_start_linux_multi_notch.json` (stable grab-53). **Close-on run:** dev
> session terminal logs (May 2026), flags below — no archived JSON capture.

---

### 11.1. What was attempted

Close each notch like HX, to remove the **root cause** of freeze (never-closed ED03
transactions, §6/§10) instead of only capping their rate. FSM:

```text
dump → read dump[20] → 19#1 at ctr = (1b ctr) + dump[20] + 8
     → wait for 68-byte echo (head 0x39, lane ed:03:80:10)
     → 19#2 at ctr +0x31 → finalize
```

The model is emitted **before** `19#1` (so no display regression even if closure fails).
Clean abort if the echo is missing within the window.

### 11.2. The `19` `ctr` formula — RESOLVED and verified (established)

Contrary to handoff §5/§11 which believed it out of reach, the `19` `ctr` **can be derived**:
the device places in **dump byte 20** the length the host must acknowledge.

```text
delta(1b → 19#1) = dump[20] + 8        (verified 10/10 on both HX captures)
delta(19#1 → 19#2) = 0x31              (constant)
double: +1 per OUT
```

Determinism test: same dump content → same delta (model `cd0246` ×4 → always `0x4d`). Apparent
variation came from **different** models (two 92-byte dumps → `0x4c` vs `0x4b`). **The formula
is NOT the blocker** — close-on run logs confirm it; arithmetic hits every detent. Unit test:
`close_19_1_ctr_matches_hx_formula`.

### 11.3. Why it still fails: the `1b` / `19` asymmetry

The device treats the two packets **differently**:

| Packet | Role | `ctr` validation | On synthetic lane (`0x6cbd`) |
|--------|------|------------------|------------------------------|
| `1b` | dump *request* | **loose** | dumps ~92% (grab-53 works) |
| `19` | transaction *continuation* | **strict** (real internal lane) | **never an echo** |

The `1b` is served even off the real lane; the `19` is checked against the device's real
transaction state → on our synthetic lane, **no 68-byte echo**. This is **the §5 wall applied
to closure**: HX closes because its **entire** session (`1b` + `19`) is on the live lane the
device tracks; our synthetic `1b` forbids the `19`.

### 11.4. Hardware trace (close-on run)

```text
model → "64"; "Minotaur"
[close] 19#1 ctr=6dc8 (=1b 6d84 + dump[20] 0x3c + 8) double=f7:64
[close] 68-byte echo absent → clean abort (no 19#2)        ← 19 rejected
  …1b ctr=6df9 → pull failed (no assignable bulk)          ← “Teemah” detent: §4 reject
  …1b ctr=6e44
model → "cd0223"; "Heir Apparent"
[close] 19#1 ctr=6e92 (=1b 6e44 + dump[20] 0x46 + 8) double=fa:64
[close] 68-byte echo absent → clean abort
```

Two symptoms, two distinct causes:

- **“Teemah” skipped** = ordinary reject (§4, ~8% benign): the `1b` at `ctr=6df9` did not
  dump; the UI skips one detent and realigns on the next. **Not a code bug** — close mode
  simply does not see that detent.
- **Freeze** = close **strictly worse** than grab-53. Each abort leaves a transaction **half
  open** (`1b` + failed `19#1`, never closed) → stacks *more* unclosed state than grab-53
  (which only leaves the `1b`) → saturates and freezes **faster**. Measured correlation in
  §11.4.1.

#### 11.4.1. Measured correlation (same throttle, close ON vs OFF)

Same fast multi-notch protocol as §10.6; only the close flag changes. Close-on repro:

```text
HX_PULL_COUPLE_LANE=1 HX_PULL_CLOSE_LIKE_HXEDIT=1 HX_PULL_COALESCE_LAST=1
HX_PULL_SETTLING_MS=500 HX_SCROLL_PULL_DEBUG=1
```

| Run | `CLOSE` | Throttle | Pulls before freeze | `ff→00` wrap | Freeze |
|-----|:-------:|---------:|--------------------:|:------------:|:------:|
| grab-53 (§10.6) | OFF | 500 ms | **24** (10 sweeps) | crossed | **no** |
| proposal A | ON | 500 ms | ~8–12 (3rd–4th sweep) | not reached | **yes** |

Trace §11.4 is from the close-on run (dev session, May 2026): `ctr` arithmetic is correct at
every attempted detent, but each `[close] 68-byte echo absent → clean abort` leaves one more
half-open transaction than pure grab-53.

### 11.5. Decision

- **`HX_PULL_CLOSE_LIKE_HXEDIT` stays OFF** (default). Back to grab-53 + 500 ms throttle
  (§10), stable and freeze-free.
- Proposal A code (FSM, formula, echo detection, tests) was **validated locally** but is
  **not on the current repo branch** (see §11.6). To re-enable usefully, **crack §5** first
  (place the `1b` on the device's live lane) — then the §11.2 closure recipe applies as-is.
- **User guideline (README):** do not operate pedal controls while the editor is connected;
  make changes **from the editor**. HX Edit allows simultaneous use, but for HX Linux it
  makes no sense given the constraint: live model read on hardware scroll is neither reliable
  nor safe, and avoiding it removes freeze risk upfront.

### 11.6. Code state (handoff)

The flag `HX_PULL_CLOSE_LIKE_HXEDIT`, closure FSM (§11.1), and unit test
`close_19_1_ctr_matches_hx_formula` are **not present on the current repo branch**
(`fix/none-sur-3894283` at time of writing — `grep` returns nothing in
`scroll_model_pull.rs`). The implementation was **tested locally** then **removed / not
committed** to avoid accidentally enabling a path that makes freeze worse. Anyone replaying A
must reintroduce the patch from their session history, or rewrite from the §11.1–11.2 recipe.
**Do not merge** without resolving §5 (live lane) first.

---

*Summary: HX-style closure was the right idea (and the `ctr` formula, long thought
impossible, finally fell into place: `dump[20] + 8`). But it hits the same wall as the whole
workstream — without the `1b` on the live lane, the device rejects our `19`s, and trying
makes freeze worse. We stop at a sufficient, stable state (grab-53 + throttle, hardware
scroll discouraged in the UI), and park A, ready if §5 ever yields.*
