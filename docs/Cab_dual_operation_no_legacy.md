# Cab dual & cab2 replacement — USB `ed:03` protocol behavior

**HXLinux — HX Stomp XL**  
*Technical reference ( **IR** Mic IR / WithPan cabs only — not pre-3.50 **Legacy** hybrid cabs, which use a distinct `c319` wire with `1a 30:00` suffix). Based on usbmon/Wireshark captures of HX Edit compared to HXLinux traffic.*

> **Version française :** [Cab_dual_fonctionnement_no_legacy.md](Cab_dual_fonctionnement_no_legacy.md)  
> **Legacy (hybrid):** [Cab_dual_operation_legacy.md](Cab_dual_operation_legacy.md) — cab2 replace: `0x23` (1-byte hint) or `0x25` (`cd02xx`, 48 B)

---

> **One-line summary.** Changing cab2 in a “Cab dual” slot is neither a blocking handshake nor a dump round-trip: it is a **fire-and-forget three-frame sequence** (`focus → ed:08 → bulk`) on **one consistent lane** (`live_write`), where the bulk must carry a **dual-context cab2** (`cd031c` / `c3 19`, never the single `cd031b` / `c2 19`) and a **clean header** (byte 14 zeroed).

---

## 1. What is a “Cab dual”?

On the HX Stomp XL, a “Cab” block can host **two cabinets at once** (cab1 + cab2), mixed via pan. The firmware calls this a *Cab dual* (`Dual` subcategory), as opposed to a *Cab single* (one cabinet).

The subtlety that cost the most time: **a “dual” cabinet and the “single” cabinet with the same display name are two different models**, with distinct `bulkHex` values and wire IDs. “Soup Pro Ellipse” exists in two forms:

| Form | `id` | `chainHexHint` | `bulkKind` | Marker | Head |
|------|------|----------------|------------|--------|------|
| **Dual** (for a dual slot) | `HD2_CabMicIr_SoupProEllipseWithPan` | `cd031c` | `assign48_cd0a` | `c3 19` | `0x27` |
| **Single** (one cabinet) | `HD2_CabMicIr_SoupProEllipse` | `cd031b` | `assign48_cd09` | `c2 19` | `0x25` |

Same UI name, but `cd031c` ≠ `cd031b`. **In an IR dual slot, cab1 and cab2 both use the dual wire form.** That was the key to the last bug (see §6.2).

---

## 2. Anatomy of an `80:10:ed:03` frame

All model commands go through USB HID frames whose first 8 bytes are:

```
[head] 00 00 18 80 10 ed 03
  │              └──────────── opcode “ed:03” (chain/model manipulation)
  └───────────────────────── head byte: operation kind
```

Head bytes encountered:

| Head | Meaning |
|------|---------|
| `0x1d` | **focus** — points at the element about to be modified |
| `0x08` | **short ed:08** — “arms” the bulk that follows |
| `0x27` | **dual replace bulk** (occupied slot, 48 B, `cd:0a` → reframe `cd:04`) |
| `0x31` | **dual create bulk** (empty slot, 60 B, `cd:03`) |
| `0x25` | **single assign bulk** (`cd:09`) |
| `0x21` | IN 21 — ack/handshake **from the device** |
| `0x19` / `0x93` / `0x38` | dumps from the device |

After the 8-byte prefix comes the **lane header** (bytes 8–15), then the **body** (model block).

---

## 3. Lanes — the core of the protocol

### 3.1. Lane counter

Bytes **12–13** of each `ed:03` frame carry a **16-bit little-endian counter**: the *lane*. Read it as `[page][offset]` — the high byte (13) is the “page”, the low byte (12) the offset within the page.

```
… 80 10 ed 03  00 [seq] 00 [sub]  [ctr_lo] [ctr_hi]  [14] [15] …
                   │         │      └────── lane (u16 LE) ──────┘
                   │         └── sub-opcode (0x04 in live)
                   └── sequence number
```

> **Bytes 14–15 must be `0x00` in live operation.** They are not part of the useful counter; a non-zero byte 14 shifts the lane to a stray page and the device ignores the frame. (See trap §6.1.)

### 3.2. Coherence rule: `focus → ed:08 → bulk`, offset `+0x11`

This is **the** protocol law, proven on paired HX Edit captures:

> **focus, ed:08, and bulk share one coherent lane.**  
> `ed:08 = focus + 0x11` · `bulk = ed:08`

With `L` = model lane counter at fire time:

```
focus   →  ctr = L
ed:08   →  ctr = L + 0x11
bulk    →  ctr = L + 0x11
```

The device **strictly validates** ed:08 and bulk against the focus lane. If the three are not on the same page with the correct offset, it rejects silently.

### 3.3. HX Edit (1 lane) vs HXLinux (4 fragmented lanes)

This is where the multithread refactor trapped us. **HX Edit keeps everything on one page** and advances it as a block:

```
HX Edit:  focus 0x6e7d  →  ed:08 0x6e8e  →  bulk 0x6e8e        (page 0x6e, +0x11)
```

**HXLinux split counters across four disjoint pages**, one per thread “role”:

| Lane | Role | Example (real capture) |
|------|------|------------------------|
| `live_write` | live write / **model bulk** | `0x6cbd` |
| `sq` / auto-ACK | automatic acks | `0x1ef4` |
| `editor` | editor focus (tab click) | `0x3255` |
| `keepalive` | heartbeat (hardcoded `7e1c`) | `0x1c7e` |

The danger: building focus on the `editor` lane and bulk on `live_write` violates §3.2 — four pages instead of one. **The correct model lane is `live_write`.** Focus, ed:08, and bulk must all be anchored there.

---

## 4. Anatomy of a Cab dual bulk

Take the raw dual `bulkHex` for “Soup Pro Ellipse” (catalog, 2026-06-12 capture) and decode it:

```
27 00 00 18  80 10 ed 03   ← head 0x27 + ed:03 opcode
00 3a 00 04                ← seq=3a, sub=04
99 8b                      ← ctr (lane) = 0x8b99
05 00                      ← bytes 14-15  ⚠ byte14=0x05 = capture residue (should be 00)
01 00 06 00 17 00 00 00    ← header scaffold
83 66 cd 0a                ← model block start, kind = cd:0a (dual)
75                         ← tag (internal lane byte in block)
64 28 65 82 62 01 64 83 17 ← fixed scaffold; 82 62 01 = slot bus (01)
c3 19                      ← DUAL MARKER
cd 03 1c                   ← cab1 = Soup Pro Ellipse (dual) = cd031c
1a                         ← cab1/cab2 separator
cd 02 d6                   ← cab2 = Jazz Rivet (default) = cd02d6
00                         ← end
```

General dual model block layout:

```
83 66 cd <0a|04> <tag> 64 28 65 82 62 <bus> 64 83 17  c3 19  <cab1> 1a <cab2> 00
         │                                            │       │        │
         │                                            │       │        └── 2nd cabinet
         │                                            │       └── 1st cabinet
         │                                            └── dual marker (single = c2 19)
         └── 0a = assign template; 04 = live “replace” reframe
```

**`cd:0a` → `cd:04` (reframe).** The catalog template uses `cd:0a` (assignment). For a live *replace* (change a cab in an existing slot), we reframe `0a` to `04`. This is only the block sub-kind; cab1/cab2 payload is unchanged.

For comparison, the **single** `bulkHex` for the same cabinet:

```
25 … 8366 cd 09 … c2 19  cd 03 1b  1a ff 00 00 00
        │          │      │        │
        │          │      │        └── ff = no 2nd cab
        │          │      └── module = Soup Pro Ellipse (single) = cd031b
        │          └── SINGLE marker
        └── kind cd:09 (single)
```

Single = head `0x25`, kind `cd:09`, marker `c2 19`, id `cd031b`, and `1a ff` (no cab2). It is a **different object** from dual.

### 4.1. Create (`head=0x31`) vs replace (`head=0x27`)

This document focuses on live **replace** (cab1/cab2 in an existing dual slot). **Initial assign** on an empty slot uses a different bulk:

| Operation | Slot | Bulk head | Size | Block kind | Role |
|-----------|------|-----------|------|------------|------|
| **create** / `add` | empty | `0x31` | **60 B** | `cd:03` | Installs dual parent; factory cab2 = `cd02d6` (Jazz Rivet) |
| **replace** | occupied | `0x27` | **48 B** | `cd:0a` → reframe `cd:04` | Changes cab1 or cab2 (`cab_index` 0 or 1) |

Typical flow after create: factory cab2 (`cd02d6`) is changed via the §5 sequence (`focus → ed:08 → bulk 0x27`).

In HXLinux, `build_cab_dual_create_bulk` (`edit_slot_model.rs`) builds the 60 B bulk from the catalog `assign48_cd0a` template by copying cab1 identity (`c319`…`1a`). Enabled by default via `HX_CAB_DUAL_CREATE_HEAD31` (set `=0` for legacy head=27 behavior without registered cab2).

### 4.2. Degenerate case when reading captures

Some **cab2 replace** captures show the **same dual hint twice**:

```
… c3 19  cd 03 1c  1a  cd 03 1c  00   ← cab1 and cab2 = Soup Pro (cd031c)
```

Example: `captures/usb-wireshark/Save/cab dual change right.json` — re-selecting the **same** Soup Pro Ellipse on cab2.

> **This is not the general rule.** On create (`captures/usb-wireshark/add_dual_cab.json`) or after scrolling, cab2 is often a **different** hint (e.g. factory `cd02d6`, then `cd0322`, `cd02d1`, etc.). Do not infer “cab2 = copy of cab1” from this degenerate case.

---

## 5. Cab replace sequence — the “fire”

The same pattern applies to **cab1** or **cab2** (`cab_index` 0 or 1); this document focuses on cab2 as the most tested case.

We long believed a blocking handshake was required: `focus → wait dump → ed:08 → wait IN 21 → done`. **Wrong.** Captures show a **fire-and-forget sequence**: emit three frames on the `live_write` lane; the live session handles the rest. No dump wait, no frozen IN 21 wait, no replayed ACKs.

With `L = live_write_ctr` at fire time:

```
1.  focus   head=0x1d  lane=live_write  ctr = L          cd=0x04  src=LiveWrite
        ↓  (~93 ms)
2.  ed:08   head=0x08                   ctr = L + 0x11    ← arms bulk
        ↓  (~400 ms)
3.  bulk    head=0x27                   ctr = L + 0x11    ← cab2 model
```

Details that matter:

- **The internal block lane tag** (byte right after `cd:04`) also follows a sequence: `focus = tag Y`, `bulk = tag Y+1` (`slot_model_lane_seq`). In the validated capture, focus→`Y`, bulk→`0x19`.
- **Bulk bytes 14–15 are forced to `0x00`** before send (see §6.1).
- **Bulk cab2 must be a dual cab** (`cd031c` / `c3 19`), not single (see §6.2).
- During the sequence the device emits a burst of `IN head=0x1d` notifications we **ACK normally**; expected, not an error.

Real trace of a successful fire (`live_write` lane, `L=0x6cbd`):

```
cab_dual_focus  cd=0x04 src=LiveWrite  → focus ctr = 0x6cbd
FIRE  L=0x6cbd  (focus=L, ed08/bulk=0x6cce)
OUT  head=0x1d                                    ← focus
IN   head=0x21                                    ← device engages (IN 21)
IN   head=0x1d  (×N)  → ACK_1d send               ← notifications, ACKed
OUT  head=0x27  bulk len=48 …                     ← model bulk
OK   L=0x6cbd  model=0x6cce
```

---

## 6. Traps encountered (symptom / cause / fix)

### 6.1. Stale byte 14

> **Symptom.** Fire runs cleanly (IN 21 received, no crash, coherent lane), logs look perfect — but cab2 does not change on hardware.
>
> **Cause.** Catalog `bulkHex` `assign48_cd0a` carries **byte 14 = `0x05`**, residue from the capture session. `patch_bulk_header_counters` rewrites ctr (bytes 12–13) but **leaves byte 14 intact**. On the wire, that `0x05` shifts the lane to a stray page → device ignores the bulk.
>
> ```
> emitted bulk (FAIL): 27 … 00 04  ce 6c  05 00  01 …   ← byte14 = 05
> HX Edit      (OK)  : 27 … 00 04  8e 6e  00 00  01 …   ← byte14 = 00
> ```
>
> **Fix.** After isolating the `head=0x27` pack, force `bulk[14] = 0x00; bulk[15] = 0x00;`. HX Edit and create `head=31` do the same.

### 6.2. cab2 single instead of cab2 dual — **the final bug**

> **Symptom.** The same frozen `bulkHex` in a console command changed cab2 perfectly; UI code with identical structure failed. Only variable: console hard-coded cab2, UI chose it via the picker.
>
> **Cause.** The picker fed cab2 the **single** `bulkHex` (`cd031b`, `c2 19`). In a dual slot, **the device only accepts dual-context cabs** (`cd031c`, `c3 19`) in both positions. Console worked because its hard-coded cab2 was `cd031c` (dual). UI sent `cd031b` (single) → silent reject.
>
> ```
> console (OK): … c3 19  cd 03 1c  1a  cd 03 1c     ← cab2 = cd031c (dual)
> UI      (FAIL): … c3 19  cd 03 1c  1a  cd 03 1b 00  ← cab2 = cd031b (single)
> ```
>
> **Fix (clean version, June 2026).** Cab 2 picker stays **Single IR** for the user (same cabinet name as elsewhere). On click, `resolveCabDualCab2UsbWireFromPicker` maps single id → **dual** assign entry (`HD2_CabMicIr_FooWithPan`, `variant: dual`); `build_cab_dual_replace_cab_bulk` extracts the `c319` hint (e.g. `cd031c`, cab1 field from the WithPan bulk) and patches cab2 — not the single bulk `c219` / `cd031b`.

### 6.3. Picker context stuck after dual

> **Symptom.** After a successful cab2 change, picking a **Distortion** (or any other block) from the picker: assign fails and UI reverts to the original Cab dual.
>
> **Cause.** Cab dual context (`cabDualPickerSync` / `lastCabDualTabPanesContext`) stayed active: every click was routed to `applyCabDualCabFromPickerListClick` (sub-cab replace) instead of full-slot `probe_slot_model_usb` replace.
>
> **Fix.** `isCabDualSubCabPickerPick`: sub-cab **only** when picker category is Cab and variant is single/legacy; otherwise `exitCabDualPickerModeForFullSlotReplace()` then standard assign flow.

### 6.4. Empty picker list for “Single Legacy” / “Dual Legacy”

> **Symptom.** Cab Legacy subcategories shown but model list empty.
>
> **Cause.** `usbAssignVariantFromPickerSub` mapped “Single Legacy” / “Dual Legacy” to `variant: legacy`, while `HX_ModelUsbAssign.json` stores those cabs as `variant: single` or `dual` (outside IR scope of this doc, same picker mechanism).
>
> **Fix.** Cab + “Single Legacy” → `single`; Cab + “Dual Legacy” → `dual`.

### 6.5. Fragmented lanes (recap §3.3)

> **Cause.** Multithread refactor split HX’s single counter into four lanes (`live_write`, `sq`, `editor`, `keepalive`). Building focus on one lane and bulk on another violates the `+0x11` rule.
>
> **Fix.** Anchor focus + ed:08 + bulk on **`live_write` only**, with focus=`L`, ed:08/bulk=`L+0x11`.

### 6.6. Frozen ACKs (crash)

> **Cause.** An early console replayed 15 post-bulk ACKs **frozen** from an HX Edit capture and left ed:08 stuck (incoherent counter). Those acks desynced the live session → device freeze.
>
> **Fix.** Coherent counters (focus=`L`, ed:08/bulk=`L+0x11`) **and** remove frozen post-ACK replay: the live session handles what follows.

---

## 7. Why it works now

Conditions that must all hold at once:

| Condition | Correct value | Trap avoided |
|-----------|---------------|--------------|
| Single coherent lane | focus=`L`, ed:08/bulk=`L+0x11`, page `live_write` | fragmented lanes (§6.5) |
| Clean bulk header | bytes 14–15 = `00 00` | `0x05` residue (§6.1) |
| cab2 context | dual cab `cd031c` / `c3 19` | single `cd031b` (§6.2) |
| UI picker routing | exit dual context for other categories | mistaken sub-cab (§6.3) |
| Sub-kind | `cd:0a` reframed to `cd:04` | — |
| No blocking handshake | fire-and-forget, live session | unnecessary dump/IN 21 wait |
| No replayed ACKs | no frozen post-ACK | crash (§6.6) |

> *The root error was not handshake, lane, or timing — those were right. It was a catalog assumption: treating cab2 in a dual as “the same cabinet in single form” on the USB wire. The device does not see a name: it sees `cd031b` (single object, `c2 19`) where it expects a dual hint `cd031c` / `c3 19`. The picker can stay Single IR: only the assign path (`WithPan` + `variant=dual`) matters for the bulk.*

---

## 8. Quick reference

```
DUAL SLOT — CHANGE CAB2
───────────────────────
L = live_write_ctr

focus  : head 1d · lane live_write · ctr = L        · cd 04 · src LiveWrite · tag Y
ed:08  : head 08 ·                   ctr = L + 0x11
bulk   : head 27 ·                   ctr = L + 0x11  · cd 04 (reframe from cd 0a) · tag Y+1
         └─ bytes 14-15 = 00 00
         └─ body: … c3 19  <cab1 dual cd03xx>  1a  <cab2 DUAL cd03xx>  00

GOLDEN RULES
  • one lane only (live_write), +0x11 offset between focus and ed:08/bulk
  • bulk byte 14 = 0x00 (never capture residue)
  • cab1 AND cab2 = DUAL cabs (cd031c / c3 19), never single (cd031b / c2 19)
  • fire-and-forget: no dump wait, no frozen IN 21, no replayed ACKs
```

---

## 9. HXLinux implementation

| File | Role |
|------|------|
| [`src-tauri/src/helix/cab_dual_cab2_replace.rs`](../src-tauri/src/helix/cab_dual_cab2_replace.rs) | Fire sequence `focus → ed:08 → bulk`; forces `bulk[14..15]=0` |
| [`src-tauri/src/helix/cab_dual_live_write.rs`](../src-tauri/src/helix/cab_dual_live_write.rs) | Builds `head=0x1d` focus, `live_write` lane |
| [`src-tauri/src/helix/edit_slot_model.rs`](../src-tauri/src/helix/edit_slot_model.rs) | `build_cab_dual_replace_cab_bulk`, `build_cab_dual_create_bulk`, `c319` patch |
| [`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs) | `probe_slot_model_usb` — routes cab2 replace when `cabDualCabIndex == 1` |
| [`src/models.ts`](../src/models.ts) | Cab 1/2 picker, `resolveCabDualCab2UsbWireFromPicker`, dual exit routing |
| [`src-tauri/resources/HX_ModelUsbAssign.json`](../src-tauri/resources/HX_ModelUsbAssign.json) | Catalog `bulkHex` single vs WithPan / dual |

Reference captures: `captures/usb-wireshark/Save/cab dual.json`, `captures/usb-wireshark/Save/cab dual change right.json`, `captures/usb-wireshark/add_dual_cab.json`.
