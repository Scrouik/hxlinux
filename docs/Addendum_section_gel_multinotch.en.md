## 10. Fast multi-notch scroll freeze: ED03 saturation (MITIGATED by throttle)

> **Status: MITIGATED, not cured.** During *fast* multi-notch scrolling, the UI froze after
> 3–4 sweeps. This is neither the parser bug (§9) nor a `ctr` ceiling: it is **ED03
> saturation** from accumulated pull transactions never closed (grab-53 sends no `19`). The
> trigger is **cadence**, not detent count. 100% host-side mitigation: **coalesce to last
> detent + throttle** pull rate. Validated: 24 pulls / 10 sweeps, `ff→00` wrap crossed,
> **0 freeze**. The real remedy (close the transaction like HX Edit) remains blocked on the
> `19` `ctr` rule (see §5, §11).
>
> **French (source):** [Addendum_section_gel_multinotch.md](./Addendum_section_gel_multinotch.md)
>
> **Supersedes** item 2 of
> [scroll_model_pull_handoff_addendum.md](./scroll_model_pull_handoff_addendum.md) §6
> (“coalescing — designed, not implemented”).
>
> **Captures:** `stomp_running_start_linux_multi_notch.json` (Linux runs),
> `stomp_running_start_hxedit_2_multi_notch.json` (Windows reference).
> **Code:** `scroll_model_pull.rs` (`coalesce_last_enabled`, `post_pull_settling_ms`,
> `tick_hw_model_pull`), `usb_listener.rs` (tick on every IN).

---

### 10.1. Symptom and proof the device is alive

The UI freezes after a few fast sweeps. In the capture, **after the last pull the pedal
keeps answering keep-alives** on all lanes (ed/ef/f0) for several seconds, but emits no more
scroll `1d`/`1f`. That is the exact signature documented in handoff §6: live keep-alive loop,
dead scroll subsystem (a footswitch press on the pedal unblocks it). The freeze is therefore
**internal to the device**, triggered by our traffic — not a host crash.

### 10.2. Cause: unclosed ED03 transactions piling up

grab-53 reads the dump then **stops**: no closing `19`. Each pull therefore leaves an ED03
transaction open on the device side, which drains only slowly (echo lag ~2.3 s, handoff §6).
In *slow* scroll (~1 pull/s) each transaction has time to clear → the session holds (38
detents, §9). In *fast* scroll, pulls arrive every ~80–110 ms: open transactions stack faster
than they drain, the internal window saturates, and scroll freezes.

➜ **The trigger is pull throughput, not detent count.** Measured on the run that freezes:
up to **8 pulls in a 2.3 s sliding window** (intervals `103, 93, 93, 110, 77, 97` ms). The
slow run that survived 38 detents was ~1 pull/s.

### 10.3. What HX Edit does (and why it never saturates)

Windows reference, per detent: `1b → model dump → (21) → 19#1 → 68-byte echo (head 39) → 19#2`.
Two **light** `19`s + 68-byte echoes, **no `272`** during scroll. HX therefore **closes** each
transaction 1:1 with no settling blackout → its ED03 lane never accumulates, at any cadence.

The real fix (proposal A) would replicate that closure. It remains **blocked**: `19` `ctr` is a
position counter **tied to dump content** (measured: not `1b→19#1` =
`0x46/0x44/0x4c/0x64/0x4b/0x4d` for dumps of 88/84/92/116/92/96 bytes, and the device does not
re-encode it in the dump). Cannot reconstruct to ±0 without Line 6 specs — the §5 wall applied
to closure. Sending a `19` at a guessed `ctr` would replay the freeze.

### 10.4. Mitigation: coalescing + throttle (pure host-side)

Without touching the device (no new packets), we **cap pull rate**:

- **Coalescing**: a `1f` received during the settling window is no longer discarded but
  remembered (LAST wins, `hw_model_pull_pending_slot_bus`). One deferred pull fires at end
  of settling via `tick_hw_model_pull` from `usb_listener` **on every IN** (the only tick
  point outside capture). We always read the FINAL model of the sweep. The tick **simulates
  the missing `1f` lane advance** (`advance_firmware_scroll_lane(0x1f)`) before sending the
  deferred `1b` — same contract as the immediate pull in `handle_in_layer_trigger`.
- **Throttle**: the settling window sets cadence. At 500 ms, we no longer exceed ~2–3
  transactions in flight → below the saturation threshold.

### 10.5. Measured device threshold

Sweeping `HX_PULL_SETTLING_MS` under sustained fast scroll:

| `HX_PULL_SETTLING_MS` | Result |
|----------------------:|--------|
| 50 (historical default) | freezes (3–4 sweeps) |
| 300 | still freezes |
| 500 | **stable** |

➜ The device ED03 drain window is **~300–400 ms**. We fixed **500 ms** by default (~1.3–1.6×
margin). Responsiveness/stability trade-off: the UI settles on the final model ~0.5 s after
the wheel stops.

### 10.6. Result

| Run | Throttle | Pulls | `ff→00` wrap | Freeze |
|-----|---------:|------:|:------------:|:------:|
| fast multi-notch | 50 ms | ~12 (freeze on 3rd–4th sweep) | not reached | **yes** |
| fast multi-notch | 500 ms | **24** over 10 sweeps | crossed | **no** |

### 10.7. Code state

- `coalesce_last_enabled()`: **ON by default** (`HX_PULL_COALESCE_LAST=0` for legacy behavior).
- `post_pull_settling_ms()`: **500 ms** by default when coalescing is active, 50 ms otherwise;
  `HX_PULL_SETTLING_MS=<n>` override takes priority (tuning / threshold measurement).
- `tick_hw_model_pull(state)`: fires deferred pull at end of settling, called from
  `usb_listener` on every IN, independent of capture in progress.
- All of this remains under `HX_PULL_COUPLE_LANE` (the scroll pull feature itself).

---

*Summary: multi-notch freeze was not a new mystery but the known device freeze (§6), reached
faster because fast scroll stacks never-closed ED03 transactions. Unable to close them (`19`
`ctr` out of reach, §5), we cap their rate: coalescing + 500 ms throttle, host-side, zero
device risk. Fast multi-notch scroll stable. HX Edit-style closure (proposal A) remains the
target if `ctr` becomes tractable someday.*
