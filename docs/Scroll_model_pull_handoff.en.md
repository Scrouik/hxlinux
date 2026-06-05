# Scroll → model dump (HX Stomp XL) — Handoff / dead end

> **June 2026 addendum (grab-53):** sections **§0, §5, §6, and §12** in this document are
> **superseded** by
> [scroll_model_pull_handoff_addendum.en.md](./scroll_model_pull_handoff_addendum.en.md).
> Keep **both files** side by side: this handoff preserves the original reasoning and
> dead-end analysis; the addendum records hardware-validated state.
>
> **French (source):** [scroll_model_pull_handoff_addendum.md](./scroll_model_pull_handoff_addendum.md)

> **Status: SUSPENDED.** The feature “reflect in the editor, in real time, the model
> changed via the Stomp encoder wheel” is **disabled by default**. The code remains in the
> repo (behind `HX_PULL_COUPLE_LANE`, OFF by default) but is **not wired for production**.
> This document replaces and closes the `scroll_dump_analysis_1..5.md` line
> (all contained partially or fully wrong conclusions — see §9).
>
> Last revision: June 2026. Author: Scrouik + reverse-engineering assistance.
>
> **Analysis commit (workstream closure):** [`Scroll_model_pull_handoff`](https://github.com/Scrouik/hxlinux/commit/b94508e39d6536702159275cb689b8de351e38a8)
> — branch `fix/none-sur-3894283` (`b94508e`).
>
> **Witness commit** (scroll pull that sometimes dumps, 272 ACK): `d6eb2b1` —
> `fix(helix): pull scroll modèle HW, ACK flux 272 et garde standard`. Extracted archives:
> `docs/reference/*_d6eb2b1.rs`.
>
> **French (source):** [Scroll_model_pull_handoff.md](./Scroll_model_pull_handoff.md)
>
> **Addendum (June 2026, grab-53):**
> [scroll_model_pull_handoff_addendum.en.md](./scroll_model_pull_handoff_addendum.en.md) ·
> [scroll_model_pull_handoff_addendum.md](./scroll_model_pull_handoff_addendum.md)
>
> **Analysis lineage (historical — partially wrong conclusions; useful for intellectual
> trajectory):** read in order
> [1](./scroll_dump_analysis_1.md) →
> [2](./scroll_dump_analysis_2.md) →
> [3](./scroll_dump_analysis_3.md) →
> [4](./scroll_dump_analysis_4.md) →
> [5](./scroll_dump_analysis_5.md), then this handoff, then the addendum and
> [§9 parser](./addendum_section_decrochage_38.en.md) /
> [§9 FR](./addendum_section_decrochage_38.md) /
> [§10 multi-notch](./Addendum_section_gel_multinotch.en.md) /
> [§10 FR](./Addendum_section_gel_multinotch.md) for validated state.

---

## 0. TL;DR for whoever picks this up

- We can **trigger** a model dump (device answers with `IN 53` + model-id), but **only
  intermittently and unstably**: depending on a lane counter we cannot derive correctly, the
  device either **dumps**, answers `IN 21` (reject), or **freezes** (reboot required).
- Root cause: **internal device session state** (ED03 lane counters) we cannot reconstruct
  reliably **without Line 6 specs** (we will never have them) and that **kempline does not
  cover** (its analysis stops well before this).
- Pragmatic decision: **do not emit the pull**. The editor will not reflect model changes
  made *on the Stomp hardware* while connected. User guideline: **do not operate Stomp
  controls while the editor is connected** (change models from the editor). No pull emitted →
  no freeze.

---

## 1. Intended feature

When the user turns the “model” wheel on the HX Stomp XL, the device pushes a notification
(`IN 1d` then `IN 1f` on lane `f0:03:02:10`). The editor would then **read the new model**
to refresh the on-screen grid by emitting a **pull** (`OUT 1b` on lane `ed`, `80:10:ed:03`).
The device **should** respond with a dump (`IN 53` ~92 B containing the model-id, then
possibly `272` B bulks).

This is a primitive **not covered by kempline**: kempline reads presets/names and renames,
but does not live-read the model on scroll. This part was **fully reverse-engineered** from
HX Edit captures (Windows).

---

## 2. Symptom

```
Wheel → IN 1d (pre-scroll) → IN 1f (trigger)
  → HXLinux: OUT 1b (pull) + OUT 08 (f0 interstitial)
  → Device, depending on case:
      (a) IN 53 (dump, contains model-id)  → SUCCESS
      (b) IN 21 (44 B, notify)             → REJECT, no dump
      (c) … then, after several dumps: nothing more → FREEZE (device reboot required)
```

All three behaviors coexist in one session depending on a counter value (see §5). We **never**
reached “dumps every time, no freeze”.

---

## 3. Pull anatomy (what we emit)

Target sequence (mirrored from HX Edit `stomp_running_start_hxedit_one_notch.json`):

```
OUT 1b  80:10:ed:03  … ctr(12-13) … 83:66:cd:03 <lo> 64 … 2d:65:81:62 <slot> 00   (trigger)
OUT 08  02:10:f0:03  … (f0 interstitial)
IN  53  ed:03:80:10  … 83:66:cd:03 <lo> 67 … 19 <model-id> 1a …                   (DUMP)
IN  21  / IN 1d      (device notifications)
OUT 19  80:10:ed:03  … (response #1)
OUT 19  80:10:ed:03  … (response #2)
IN  272 …            (follow-up bulks, echo of last double)
```

Two counters travel in each OUT `1b`/`19`, **both on ED03 lane**:

| Field | Bytes | Role | Observed step (HX) |
|---|---|---|---|
| `double cd:03` | 28-29 | `<lo>:64`; device re-echoes `<lo>:67` | **+1 per OUT** (f1→f2→f3) |
| `ctr` (ED03 lane) | 12-13 | transaction position on `ed03` lane | `+0x4b` after `1b`, `+0x31` after `19` |

HX one_notch, byte-accurate (canonical reference):

```
[1b] ctr=0x1c7e  double=f1:64   → dump (IN 53, echo f1:67) starts on THIS 1b
[19] ctr=0x1cc9  double=f2:64   (+0x4b ; +1)
[19] ctr=0x1cfa  double=f3:64   (+0x31 ; +1)
```

The device **tolerates the absolute double value** (a pull that dumps can start at `f1` or
`f8`). What matters is **consistency with its session**. The `ctr` is the hard part (§5).

**Operational note (53 vs 272):** On the wire, `IN 53` arrives **before** any `272`. The model-id
is already in the 53. However, **stopping after 53 without closing the transaction** (19 pulls,
272 drain/ACK as HX does) **freezes the hardware** per project experience — not “no 53 without
272 on the bus”, but “no safe 53-only handling”.

---

## 4. HXLinux runtime state (`scroll_model_pull.rs`)

- Pipeline: `ScrollModelPull` layer **before** `FirmwareScroll`. Non-None `IN 1f` →
  `Consumed` (emits pull), `IN 1d`/`IN 21` → `Ignored`.
- Counters in `HelixState`:
  - `editor_ed03_double` (shared cd:03 double, ≈ `0x64f2` after PHASE B),
  - `editor_ed03_lane` (shared ED03 lane, bytes 12-13; anchor `0x1009`, `+0x17`/command in
    PHASE B),
  - `hw_model_pull_ed03_double` / `hw_model_pull_ctr` (local pull seeds).
- **Valid fixes to keep** if someone resumes:
  - **+1 per actually emitted OUT** on the double (HX pattern), `hi` fixed `0x64`, wrap
    `cd 03→04` when `lo` passes `0xff`. (Old “+3 between pulls” was an artifact: 3 OUT × +1.)
  - **Clean abort on `IN 21`** at step 1: no pending `1b` transaction (avoids stacked unclosed
    ED03 transactions).

---

## 5. WALL #1 — `ctr` rule (bytes 12-13) is unknown

This is where everything stalls. Pull `ctr` partly decides dump, reject, or freeze. Three
families tested:

| `1b` `ctr` | Page | Dump? | Freeze? | Comment |
|---|---|---|---|---|
| `0x6cbd` (from `live_write_ctr`) | `0x6c` | **yes, intermittent** | **yes** | sometimes dumps (f3,f6,f9…) but rejects others (f5,f8); freezes after a few notches |
| `0x1c7e` (constant, = HX one_notch) | `0x1c` | **never** | no | systematic reject but stable session |
| live `editor_ed03_lane` (≈ `0x1c10`) | `0x1c` | **never** | no | same: follows our real lane, systematic reject |
| **HX Edit**: `0x1c7e` | `0x1c` | **YES** | no | HX dumps on page `0x1c` with the same value we reject |

**Insoluble contradiction:** HX dumps on page `0x1c` (`0x1c7e`); we reject on page `0x1c` but
dump (intermittently) on page `0x6c`. With **identical double** (e.g. `f3`), changing only
`ctr` from `0x6cbd` → `0x1c10` flips device from **dump** to **reject**. Therefore:

1. Bytes 12-13 **are part of** the discriminant (proved: same double, different ctr → opposite
   outcome).
2. But the **expected value** follows no rule we could derive: not “page `0x1c` like HX”, not
   “continue our `editor_ed03_lane`”, not a constant.

**Most likely hypothesis (unprovable without specs):** device compares pull `ctr` to an
**internal ED03 lane register** whose evolution depends on **all** session ED03 history (PHASE
B + preset reads + …). Our `editor_ed03_lane` does not mirror that register faithfully — and HX
Edit’s `0x1c7e` was simply **its** session value at that instant, not a universal constant. Our
session ends in a different internal state we do not observe on the wire.

**Proof it is not just pull bytes:** value `f8` **dumped** in one run and was **rejected** in
another. Behavior depends on device state invisible in bulk traffic.

---

## 6. WALL #2 — freeze is in the post-dump chain

Decisive observation: **freeze only appears when dumps happen** (page `0x6c`). On page `0x1c`
(zero dump), session survives indefinitely (25+ notches, no hang).

Device capture of a run that dumps then freezes:

```
OUT 1b → IN 53 (dump) → OUT 19#1 → IN 39 (echo of 19#1) → OUT 19#2 → … nothing more
   → infinite 16 B keep-alive loop → FREEZE (reboot required)
```

- Device **re-echoes our double with `hi=0x67`**, **FIFO** and **delayed**: echo from pull N
  sometimes arrives during pull N+1 (~2.3 s lag observed).
- Sending `19#2` too early (both `19` back-to-back on first response) freezes the device right
  after. HX waits post-`53` notifications (`IN 21`, `IN 1d`) before its `19`. Our ordering/cadence
  differs.

So **two distinct issues:** (a) `ctr` rule (§5), (b) post-dump chain (19 ordering, 272 ACK,
device lag) leading to freeze. Fixing (a) without (b) would bring back freeze.

---

## 7. Everything tried (timeline)

Historical docs (v1→v5, **all refuted**):

1. Missing `0c`/`11` subscribe on f0 lane → refuted (present, same as HX).
2. f0 ARM timing (too late) → refuted (`HX_F0_ARM_EARLY`, no effect).
3. “Live editor state” = background `1d` stream → refuted (`1d` comes from wheel gesture, not
   spontaneous subscribe: 0 background `1d` without scroll, 41 with one notch).
4. `cd:03` double (f3 vs f1) → refuted (aligned with HX, still `21`).
5. “double + ctr decoupled” (v5 claimed **SOLVED**) → refuted: dumped on cold-boot witness
   commit but did not hold; relied on `ctr=0x6cbd` later proved wrong.

This session (HX captures + Linux traces):

6. **+1 per OUT vs blind `+3` on finalize** → correct (`+3` advanced even on failed pull →
   desync). **Keep.**
7. **`cd 03→04` wrap** in all modes (before: `hi` rolled to `0x65`). **Keep.**
8. **Clean abort on `IN 21`**. **Keep.**
9. **Both `19` sent back-to-back** (HX mirror) → **regression**: freeze on first notch (`19#1`
   before device `IN 21`, device answers `39` then freezes on `19#2`). → revert to **one `19`
   per device response** if resumed.
10. **`ctr`: `0x6cbd` → `0x1c7e` → `editor_ed03_lane`** → §5 wall. `0x6cbd` dump+freeze,
    page `0x1c` always reject.

---

## 8. Established facts (verified — do not re-litigate)

- Model-id is read from `IN 53` (~92 B) via `… 19 <id> 1a …` (e.g. `cd01fe`). `IN 21`
  (44 B) **never** contains the model-id.
- Device re-echoes host double with `hi=0x67` (host sends `hi=0x64`).
- Double tolerates absolute value; advances **+1 per OUT** on HX.
- `ctr` advances `+0x4b` (after `1b`) / `+0x31` (after `19`) — deltas confirmed on HX.
- Freeze correlates with dumps (post-dump chain), not `ctr` alone.
- Background `1d` is **caused by wheel gesture**, not subscription.
- No special USB control requests (HX only `GET_DESCRIPTOR` + `SET_CONFIGURATION`).

---

## 9. Definitively refuted (do not re-explore)

- Missing f0 subscribe; f0 ARM timing; “background 1d stream” prerequisite; double alone;
  `SET_INTERFACE`/USB control; “ctr = page 0x1c constant (0x1c7e)”; “ctr = naive continuation
  of `editor_ed03_lane`”. All tested, all insufficient.
- Idea that pull bytes alone are enough: false (`f8` dumps OR rejects per session).

---

## 10. Why we are truly blocked

- **No Line 6 specs** (and we never will). Pure reverse engineering.
- **kempline does not go this far**: its Python analysis helped bootstrap (handshake, name read,
  rename) but was wrong on session details many times. It **does not implement** model pull.
- Dump/reject discriminant lives in **internal ED03 session state**, not visible on bulk wire,
  **dependent on full session history**. Exact reconstruction needs specs or a much larger
  capture campaign (matrix {ctr value} × {precise lane state} × device response).

---

## 11. Concrete paths for a successor

If someone resumes this specific topic:

1. **Capture a REJECT run** (page `0x1c`) and dissect `IN 21` (44 B): is there a counter/echo
   revealing the lane value the device **expected**? Most direct way to learn §5 rule vs guessing.
2. **Trace device state**: device echoes double with `hi=0x67`; its responses have their own
   `ctr` on page `0x03` (device→host). Map relation between our emitted `ctr` (host→device,
   ED03) and device’s might reveal internal register.
3. **Controlled matrix**: fixed double, sweep `1b` `ctr` (different pages/values), note
   dump/reject — empirical window for **this** session.
4. **If dump obtained**: tackle §6 (freeze) — **one `19` per device response** (not both at
   once), wait post-`53` `IN 21`/`IN 1d` like HX, verify full `272` bulk ACK.
5. **Capture on Linux/usbmon** (never macOS/Windows VM — already decided). Wireshark filter
   `usb.idVendor == 0x0e41`.

---

## 12. Operational decision (shipped state)

- Scroll model pull is **disconnected** (`HX_PULL_COUPLE_LANE` OFF; no `1b` on `IN 1f`).
  **No pull → no freeze.**
- Editor **does not reflect** model changes made *on the Stomp* hardware.
- **User guideline:** do not operate Stomp controls while editor is connected; change models
  from the editor. (Put in README.)
- Code and valid fixes (§4) remain for a future owner.

---

## 13. Reference captures

- `stomp_running_start_hxedit_one_notch.json` — **HX Edit, pull that dumps** (canonical
  protocol: `1b` ctr=`0x1c7e`, double `f1`, → `IN 53`).
- `stomp_running_start_linux_multi_notch_crash.json` — HXLinux, successive runs (intermittent
  dump + freeze on page `0x6c`; stable reject on page `0x1c`).
- Prior analysis line (hypothesis chronology — **do not treat conclusions as final**):
  [scroll_dump_analysis_1.md](./scroll_dump_analysis_1.md) ·
  [2](./scroll_dump_analysis_2.md) ·
  [3](./scroll_dump_analysis_3.md) ·
  [4](./scroll_dump_analysis_4.md) ·
  [5](./scroll_dump_analysis_5.md)

## 14. Key files / code entry points

- `src-tauri/src/helix/scroll_model_pull.rs` — full pull logic (`1b`/`19` builders,
  `ingest_pull_capture` state machine, `IN 21` abort, wrap, counters).
- `src-tauri/src/helix/mod.rs` — `HelixState`: `editor_ed03_double`, `editor_ed03_lane`,
  `hw_model_pull_*`.
- Enable flag: `HX_PULL_COUPLE_LANE=1`. Debug: `HX_SCROLL_PULL_DEBUG=1`, `HX_INIT_TRACE=1`.

---

*General lesson: in reverse without specs, verify the witness run BEFORE theorizing, change one
variable at a time, and accept some firmware states are out of reach. This wall is one of them.*
