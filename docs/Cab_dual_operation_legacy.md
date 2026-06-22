# Cab dual Legacy — USB `ed:03` protocol

**HXLinux — HX Stomp XL**  
*Pre-3.50 **Legacy** hybrid cabs — companion to [Cab_dual_operation_no_legacy.md](Cab_dual_operation_no_legacy.md) (IR / WithPan).*

> **Version française :** [Cab_dual_fonctionnement_legacy.md](Cab_dual_fonctionnement_legacy.md)

**Captures (2026-06-22):**
- `captures/usb-wireshark/add_dual_legacy_change_cab2.json` — isolated cab2 replace
- `captures/usb-wireshark/add_dual_legacy_change_cab2_&_dual.json` — full scenario (11 exploitable model frames)

---

> **Summary.** Legacy dual cabs share **exactly the same transport** as IR duals (`live_write` lane, `+0x11` offset, byte 14 zeroed, fire-and-forget) — reuse IR doc §3/§5 as-is. Divergence is **entirely in the bulk body**: different heads, longer scaffold, **1-byte** cab hints (most cabs) or **`cd02xx` 3-byte** hints (long hybrid sub-family).

---

## 1. Transport: identical to IR

```
focus 0xd350 → bulk   0xd361   (+0x11)   cab2 replace
focus 0xd49e → bulk   0xd4af   (+0x11)   dual parent replace (head 0x25)
focus 0xd51a → bulk   0xd52b   (+0x11)   cab2 replace (first of a series)
```

Fire logic (`focus → ed:08 → bulk` on `live_write`) is reused unchanged (`cab_dual/replace_fire.rs`).

---

## 2. Legacy head map

| Head | Operation | cab2 / note |
|------|-----------|-------------|
| `0x2d` | Assign / create dual | full descriptor `30 09 10 0a c3` |
| `0x25` | Replace **dual parent** (full slot picker) | factory default `cd 01 63` (3 B) |
| `0x25` | Replace **cab2** when hint is `cd02xx` / `cd01xx` | `c3 19 <cab1:1B> 1a cd02xx 00 00 00` — **48 B** |
| `0x23` | Replace **cab2** when hint is **1 byte** | `c3 19 <cab1> 1a <cab2> 00` — **44 B** |
| `0x1d` | Focus | — |

*IR reminder: `0x31` create, `0x27` cab2 replace; `0x25` = **single** assign (not dual).*

> **Documented trap (June 2026, HW validated).** Never send a **`0x23` bulk at 46 B** (3-byte `cd02xx` cab2 patched into a 44 B template) — the Stomp ignores it. HX Edit never uses this shape in captures.

---

## 3. Cab field: two encoding families

```
IR      :  c3 19  cd 03 1c   1a  cd 03 1b   00
Legacy  :  c3 19  33         1a  33         00     (1-byte, head 0x23)
Legacy  :  c3 19  30         1a  cd 02 4e  00 00 00   (3-byte cab2, head 0x25)
```

### 3.1 Cab2 replace — 1-byte selector (`head 0x23`, 44 B)

`chainHexHint` with **2 hex digits** (e.g. `2e`, `31`, `30`):

```
c3 19  <cab1: 1 byte>  1a  <cab2: 1 byte>  00
```

HW-validated examples (Lead 80 dual, cab1 = `30`): Celest 12H (`2e`), US Deluxe (`31`), Field Coil (`2f`).

### 3.2 Cab2 replace — `cd02xx` hint (`head 0x25`, 48 B)

`chainHexHint` with **6 hex digits** (`cd024e`, `cd0228`, `cd0227`, …) — catalog entry `variant: dual` with `bulkKind: assign48`:

```
c3 19  <cab1: 1 B>  1a  cd 02 xx  00 00 00
```

HW-validated examples: Princess Blue (`cd024e`), Grammatico (`cd0228`), Fullerton (`cd0227`).

**HXLinux build** (`build_legacy_cab2_replace_bulk` in `cab_dual/legacy/wire.rs`):

1. Start from the picked cab’s **dual** `0x25` bulk (48 B skeleton).
2. Replace **cab1** with the parent dual’s cab1 (e.g. `30` = Lead 80).
3. Replace **cab2** with the 3-byte hint (`cd02xx`) — cab1/cab2 length swap keeps **total size** at 48 B.

Do **not** patch a 3-byte hint into a parent `0x23` (44 B) template — that yields 46 B and fails silently on hardware.

### 3.3 Quick catalog rule

| `chainHexHint` | dual `bulkKind` (catalog) | cab2 replace head |
|----------------|---------------------------|-------------------|
| `2e`, `31`, `33`, … (≤ 2 hex) | `assign44_cd04_…` | **`0x23`** (44 B) |
| `cd024e`, `cd0227`, … (6 hex) | `assign48_cd04_…` | **`0x25`** (48 B) |

1-byte selectors seen in captures (`add_dual_legacy_change_cab2_&_dual.json`):

| Slot | Values | Model (`chainHexHint`) |
|------|--------|------------------------|
| cab1 | `0x33`, `0x30` | Soup Pro Ellipse, 1x12 Lead 80 |
| cab2 | `0x33`, `0x2e`, `0x38`, `0x47` | Soup Pro, Celest 12H, Jazz Rivet, WhoWatt 100 |

The `cd031b`/`cd031c` trap does **not** apply — here it is **`c219` single** vs **`c319` dual**. Device expands selector in IN echoes: `33 1a 2e` → `33 1a 2e 09 10 0a c3 …`.

**Catalog mapping:** wire hints from `chainHexHint` in `HX_ModelUsbAssign.json` (`variant: dual`). Do not patch from **single** `c219` bulk.

---

## 4. Default `cd0163` on dual parent replace (`head 0x25`)

```
head 0x25 :  c3 19  <cab1: 1 B>  1a  cd 01 63  00 00 00
```

Legacy equivalent of IR factory cab2 `cd02d6`. Variable field length → `amp_cab_cab_field_range_in_bulk` in `edit_slot_model.rs`.

**Open point:** confirm `cd0163` is always the `0x25` factory cab2 (capture: change dual → read without touching cab2).

---

## 5. Shared focus across consecutive cab2 replaces

After switching to Lead 80 dual, one focus `#17059` (`d51a`) precedes three `0x23` bulks with advancing lane: `d52b` → `d594` → `d5fd`. Do not always re-focus if UI context unchanged.

---

## 6. Decoded timeline (`add_dual_legacy_change_cab2_&_dual.json`)

```
CREATE       #5361   2d   c3 19 33 1a [30 09 10 0a c3]
CAB2 ×2      #6643 focus → #8707 33 1a 33 → #10027 33 1a 2e
DUAL CHANGE  #10837 25 33 1a cd0163 → #13055 focus → #15387 30 1a cd0163
CAB2 ×3      #17059 focus → #18223 2e → #19153 38 → #20045 47
```

(cab1 prefix `30` on last three = Lead 80)

---

## 7. IR vs Legacy summary

| Aspect | IR WithPan | Legacy |
|--------|------------|--------|
| Transport | same | same |
| Cab2 bulk (1 B hint) | `0x27` (48 B) | **`0x23`** (44 B) |
| Cab2 bulk (`cd02xx`) | — | **`0x25`** (48 B) |
| Dual parent head | `0x27`/`0x31` | **`0x25`**/`0x2d` |
| Cab id | 3-byte `cd03xx` | **1 B** or **`cd02xx`** |
| Factory cab2 | `cd02d6` | `30` / **`cd0163`** |
| Cab2 replace trap | — | **never** `0x23` + 3 B cab2 (46 B) |

---

## 8. Quick reference

```
LEGACY CAB2 REPLACE
  Transport: same as IR (+0x11, byte14=0, live_write)

  1-byte hint (chainHexHint ≤ 2 hex):
    head 0x23, 44 B — c3 19 <cab1> 1a <cab2> 00

  cd02xx hint (6 hex):
    head 0x25, 48 B — c3 19 <cab1> 1a cd02xx 00 00 00
    build: picked cab dual 0x25 template + swap parent cab1 / cab2 hint

  FORBIDDEN: head 0x23 + 3-byte cab2 → 46 B → HW ignores

  Dual parent (full slot): head 0x25, factory cab2 cd0163
  Create empty slot: head 0x2d
```

---

## 9. HXLinux implementation (`cab_dual/`)

| Module | Role |
|--------|------|
| `cab_dual/replace_fire.rs` | Shared fire; bulk `0x23` / `0x25` (legacy) or `0x27` (IR) |
| `cab_dual/legacy/wire.rs` | `build_legacy_cab2_replace_bulk` — 1 B → `0x23`, `cd02xx` → `0x25` |
| `edit_slot_model.rs` | `build_cab_dual_replace_cab_bulk` delegates to legacy wire when needed |
| `src/models.ts` | Picker → `variant: dual` (no WithPan) |

---

## 10. Checklist

- [x] Transport, cab1≠cab2, head map, `chainHexHint`, shared focus
- [x] Cab2 replace: `0x23` (1 B) vs `0x25` (`cd02xx`) routing — **HW validated June 2026**
- [ ] Confirm `cd0163` default on `0x25` dual parent
- [ ] cab1-only replace; legacy create (`0x2d`)

---

*Merged from `dual_legacy_part.md` (June 2026).*
