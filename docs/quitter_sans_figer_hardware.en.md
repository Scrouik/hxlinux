# HX Linux — Quit the app without freezing the hardware display

> **Explicit unsubscribe** (`sub=0x02` on all 3 lanes), **synchronous** before `exit(0)`, **paced** ~150 ms between each lane.
>
> **Confirmed on capture** (`close_linux` final). On quit, HXLinux stopped polling and released the interface **without telling the hardware**, which stayed in editor mode (frozen display, audio engine still active). The fix: send, **before `exit(0)`** and **synchronously**, HX Edit's unsubscribe round — one `sub=0x02` packet on each of the 3 lanes (`f0`/`ed`/`ef`), **spaced ~150 ms apart**. The device processes each unsubscribe **on receipt**; an ACK is not required. Display restored without reboot.

**French (source):** [quitter_sans_figer_hardware.md](./quitter_sans_figer_hardware.md)

## 1. Symptom

After closing HXLinux: the Helix **display stops syncing** (frozen), but **controls still work** (footswitches, sound). **Persistent** state — reboot required. Signature of **editor mode never released**: the device still believes an editor is driving it and suspends display management; the audio engine runs independently.

## 2. Root cause

Teardown only did `stop_all()` on the poll then `release_interface`. No “the editor is leaving” message. Initial hypothesis “USB drop is enough” → **disproved**: releasing the interface does not exit editor mode; an **explicit unsubscribe** at the proprietary protocol level does.

## 3. HX Edit reference (`08_close_HXEdit.json`)

Throughout the session, keep-alives use `sub=0x10`. At close time, **one last round** on all 3 lanes switches to `sub=0x02` (the *subscribe* subtype), then HID silence:

```
OUT 80 10 ed 03  sub=02      OUT 02 10 f0 03  sub=02      OUT 01 10 ef 03  sub=02
IN  ed 03 80 10  sub=02 ✓    IN  f0 03 02 10  sub=02 ✓    IN  ef 03 01 10  sub=02 ✓
— then no more HID traffic —
```

On HX Edit all 3 are ACKed, but (see §5) the **ACK is not what matters**: it's spaced receipt of `sub=0x02`.

The close packet = that lane's idle poll with **one byte changed**: `byte 11` goes from `0x10` to `0x02`. (That's why the initial eyeball comparison found nothing.)

## 4. The fix — three ingredients

1. **Synchronous, before `exit(0)`.** On app close, the `CloseRequested` handler calls `app.exit(0)`, which **kills the process immediately**: no deferred teardown or channel message would run in time. The `0x02` round is sent **in the handler**, via direct `write_bulk`, before `exit(0)`.

2. **Paced (~150 ms between each lane).** This is the decisive ingredient (§6). Sent back-to-back, the hardware only processes one.

3. **Do not touch `helix_session_stop`.** That flag triggers `disconnect_helix_session` (`connected=false`, `tx=None`) on another thread → the close was aborted before send. Stop **only** the idle poll via the keep-alive channel (`KeepAliveCommand::StopAll`), never the session.

## 5. Dead ends (do not revisit)

- **“All 3 lanes must be ACKed.”** False. On a working HXLinux capture, **only `ef` responds**; `ed`/`f0` **never** respond (device-driven lanes — a host OUT is not a request they answer). Yet the display comes back. The device processes unsubscribe **on receipt**, not by replying. The `ef` ACK is a side effect (host-driven lane).

- **“It's a counter issue (collision / `device_last+1`).”** False, verified: after switching to `device_last+1`, `ed` was `40` (≠ device `3f`, so **no collision**) and still not ACKed, while pacing alone fixed everything. The counter is not the discriminant. *(The code keeps `device_last+1` as harmless, but it was not the cause.)*

## 6. Why pacing matters (evidence)

**Failure** (`close_linux` v1) — 3 closes in < 1 ms:
```
1.528 OUT f0 sub=02   1.528 OUT ed sub=02   1.528 OUT ef sub=02   1.529 IN ef sub=02 ✓
→ device processes only one (the last); display stays frozen.
```

**Success** (`close_linux` final) — closes spaced ~150 ms apart:
```
1.800 OUT ed sub=02
1.951 OUT f0 sub=02      (+151 ms)
2.101 OUT ef sub=02      (+150 ms)   2.101 IN ef sub=02 ✓
→ device processes all 3 on receipt; display restored.
```

The delay comes from `read_bulk(0x81, …, 150 ms)` after each send: on `ed`/`f0` (which return nothing) its **timeout** provides spacing; on `ef` it catches the ACK. **The timeout is the active ingredient** — hence the warning in §8.

## 7. Implementation

**`keep_alive.rs`** — single source for packets:

```rust
pub const CLOSE_SUB: u8 = 0x02;
pub fn graceful_close_packets(state: &mut HelixState) -> [Vec<u8>; 3] {
    // [f0, ed, ef]: each lane's idle poll, byte 11 = 0x02. (device_last+1 counter, not decisive)
}
```

**`lib.rs` — `graceful_helix_close(app)`**, called before `exit(0)`:

1. stop idle poll (`keepalive_tx → StopAll`) + listener; **not** `helix_session_stop`; then `sleep(80 ms)` so poll and listener release endpoint `0x81`;
2. reorder to `ed → f0 → ef` (HX Edit order) and, per lane: `write_bulk(0x01, close)` then `read_bulk(0x81, …, 150 ms)` — both send **and** spacing;
3. `release_interface(0/4)` + `attach_kernel_driver`.

## 8. Takeaways

- **Editor mode is a protocol subscription**, not a USB side effect: it must be **closed explicitly** (`sub=0x02` on all 3 lanes).
- **Pacing beats ACK.** Closes must be **spaced** (~150 ms confirmed working; device minimum not measured). Back-to-back → only one processed.
- **Do not “optimize” `read_bulk`.** On `ed`/`f0` it receives nothing, but its timeout **is** the spacing. Removing it breaks close.
- **`exit(0)` leaves no safety net**: all hardware cleanup on quit must be **synchronous**, before the call.
- **Never trigger `disconnect_helix_session` from the close path** (`helix_session_stop`): it sets `connected=false` / `tx=None` and aborts the send. Stop only the poll.

---

*Quitting cleanly means saying goodbye — slowly. The hardware expects an explicit unsubscribe on each lane before it takes back its display, and it needs time to process each one. Sent in a burst, it only hears one; spaced out, it hears all three. The ACK is incidental: what counts is the message received, not the reply.*
