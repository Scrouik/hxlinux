# Scroll → model dump (HX Stomp XL) — Handoff ADDENDUM (June 2026)

> This addendum **updates**
> [Scroll_model_pull_handoff.en.md](./Scroll_model_pull_handoff.en.md). It records
> hardware-validated understanding after switching to **grab-53 mode**. It **corrects**
> handoff sections §0 (status), §5 (`ctr` rule), §6 (freeze), and §12 (operational
> decision). Anything not contradicted here remains valid.
>
> **Original handoff (historical):**
> [Scroll_model_pull_handoff.md](./Scroll_model_pull_handoff.md) ·
> [Scroll_model_pull_handoff.en.md](./Scroll_model_pull_handoff.en.md)
>
> **French (source):** [scroll_model_pull_handoff_addendum.md](./scroll_model_pull_handoff_addendum.md)
>
> **Revised status: single-notch RESOLVED and stable.** Fast multi-notch: **MITIGATED**
> (coalescing + 500 ms throttle — see
> [Addendum_section_gel_multinotch.md](./Addendum_section_gel_multinotch.md) §10).
>
> **Supplements:**
> [§9 parser decoupling](./addendum_section_decrochage_38.en.md) ·
> [§9 FR](./addendum_section_decrochage_38.md) ·
> [§10 multi-notch freeze](./Addendum_section_gel_multinotch.en.md) ·
> [§10 FR](./Addendum_section_gel_multinotch.md)

---

## 1. What unblocked the problem: we only read `IN 53`

We only need the model **chainHex**, and it is **entirely in `IN 53`**
(the dump frame, ~84–116 bytes, pattern `… 19 <id> 1a …`). The `272` bulk carries only
model **parameters** — useless here.

**grab-53 mode:**
```
IN 1f → OUT 1b (+ OUT 08 f0 interstitial) → IN <dump> → extract chainHex → DONE.
        We NEVER send 19, we NEVER continue into 272.
```

**Effect, verified on captures + runs:** removing the `19/272` chain fixed **both**
walls at once:
- **no more freeze** (hardware stays alive across dozens of detents);
- **intermittent rejects collapse** (from ~50% to ~3–8%).

The `19/272` chain was therefore the common cause: it “poisoned” the ED03 session
(incomplete transaction on the host side + ~2.3 s device delay → collision with the next
detent). By stopping at `53`, the device is free again on every `1f`.

---

## 2. CORRECTION to §5 — it is a CONTINUATION, not a “page”

Handoff §5 concluded “the device only dumps on page `0x6c`, never on page `0x1c`”.
**That is wrong / too narrow.** Single-notch run (~38 detents): pull `ctr` climbed

```
0x6cbd → 0x6d08 → … → 0x7794      (+0x4b per 1b sent, ONE 1b per notch in grab-53)
```

i.e. **8 pages crossed** (`6c, 6d, 6e, 6f, 70, 71, … 77`), **dumping all the way**.

➜ Corrected rule: the device accepts the pull as long as its `ctr` is a **monotonic
continuation from a valid seed** (`0x6cbd` set once per session). It is NOT a fixed page
value. The `0x1c7e` / live HX lane value rejected for us because it was not a continuation
of **our** session, not because of the page itself.

- **double** `cd:03`: `+1` per OUT, `hi` fixed at `0x64`, wrap `cd 03→04` when `lo`
  passes above `0xff` (observed: log `double wrap bas → cd lane 04` at pull `ctr=0x708c`).
- No page ceiling observed up to `0x77`.

---

## 3. Validated single-notch anatomy

**Healthy cycle** (~92% of detents):
```
IN 1d pre-scroll (scroll lane advanced, NO ACK)
  → IN 1f trigger
  → OUT 1b + OUT 08 (f0 interstitial)
  → IN <dump> (variable head 53/54/56/4c/4e/6c, len 84–116) → chainHex extracted immediately
  → IN 21 (POST-dump assignment notify, ignored)
  → IN 1d (ACKed)
```

> **Key point on `21`:** on a successful detent, `IN 21` arrives **AFTER** the dump — it is
> NOT a reject, it is the hardware assignment notification. It is a *reject* only when it
> arrives **before any dump** (step 1). Do not conflate the two (the handoff and older
> analyses called both cases “21”).

**Reject cycle** (~8%, benign):
```
IN 1f → OUT 1b → IN 21 (before any dump) → clean abort (no pending transaction)
   → this detent is not read → UI keeps previous model → NEXT detent realigns
```

---

## 4. Single-notch reject characterization (verified on logs)

- **Sporadic, ~8%** (3 rejects out of 38 pulls in the reference run).
- **Independent of cadence**: all 3 rejects occurred at normal pace
  (~900–1100 ms between detents), **not** during fast scrolling. Residual device
  intermittency (unobservable internal ED03 state), not a host timing issue.
- **No recoverable dump after `21`**: only `1d` frames follow. The device simply did not
  dump for **that** `1b`. To recover the detent, we would need to **re-send a fresh `1b`**
  (see §6, “retry-on-reject” track — not implemented yet).
- **UI consequence**: one detent behind, self-corrected on the next detent. This is the
  “lag that realigns” observed around notch 33.

---

## 5. Reference figures

| Run | Pulls | Dumps | Rejects | Freeze | Notes |
|-----|-------|-------|---------|--------|-------|
| 1-notch (slow pace, ~52 s) | 38 | 35 (~92%) | 3 (clean) | 0 | `ctr` 0x6cbd→0x7794 |
| multi-notch (pbUi) | 31 | 29 (~94%) | 1 | 0 | + ~5 `1f` lost during fast scroll |

---

## 6. What remains / supplements

1. **(1-notch) retry-once-on-reject** — on step-1 `IN 21`, re-send **one** fresh `1b`
   (normal `ctr`/double continuation). The device very likely dumps on the 2nd attempt
   → detent recovered, no visible lag. Low risk in grab-53 (back-to-back pulls OK).
   *Status: proposed, not implemented.* (Note: since June 2026 captures, pre-dump `IN 21` is
   no longer treated as a reject — this track is probably **obsolete**; revalidate before
   implementing.)
2. **(multi-notch) coalesce `1f` during settling** — *formerly “designed, not implemented”
   here.* **→ Superseded and detailed in
   [Addendum_section_gel_multinotch.md](./Addendum_section_gel_multinotch.md) §10**:
   coalescing ON by default (`HX_PULL_COALESCE_LAST=0` for legacy behavior), 500 ms
   throttle, `tick_hw_model_pull` from `usb_listener`. *Status: **implemented and validated**
   (24 pulls / 10 sweeps, 0 freeze; device freeze mitigated, not cured).*

Item 1 remains an optional **host-side** polish. Item 2 is covered by supplement §10 (no
`19/272` chain).

---

## 7. Code state (reminder)

- `helix/scroll_model_pull.rs` — grab-53 mode behind `HX_PULL_COUPLE_LANE=1`.
  - Seed: live `double = editor_ed03_double`, `ctr = 0x6cbd`; then `+0x4b`/1b.
  - `ingest_pull_capture`: finalize on first dump (chainHex extraction), **no `19`**.
  - `send_pull_both_19s` kept as `#[allow(dead_code)]` (full handshake ref if we ever
    want `272` parameters).
  - Clean abort on step-1 `IN 21`.
- Debug: `HX_SCROLL_PULL_DEBUG=1`, `HX_INIT_TRACE=1`.

## 8. Reference captures

- `stomp_running_start_hxedit_one_notch.json` — HX (protocol reference).
- `stomp_running_start_linux_multi_notch_pbUi.json` — grab-53 multi-notch (29/31 dumps, 0 freeze).
- Single-notch run (logs) — 35/38 dumps, 3 clean rejects, 0 freeze, `ctr` 0x6cbd→0x7794.

---

*Summary: scroll-time model read went from “suspended / unstable / freeze” to
“single-notch stable, zero freeze” (§9: parser) and “fast multi-notch mitigated” (§10:
coalescing + throttle). The 19/272 chain was not needed for chainHex. What remains is mainly
retry-on-reject (§6.1, probably obsolete) and HX-style closure (§10.3, blocked on `19`
`ctr`).*
