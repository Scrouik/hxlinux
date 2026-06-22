# Amp+Cab Legacy — USB protocol (hybrid cab)

**HXLinux — HX Stomp XL**  
*Legacy **hybrid** cab in an amp+cab pair (`assignVariant: amp+cab-legacy`). Complements [Amp_cab_operation_no_legacy.md](Amp_cab_operation_no_legacy.md) (IR).*

> **French:** [Amp_cab_fonctionnement_legacy.md](Amp_cab_fonctionnement_legacy.md)  
> **Captures:** `captures/usb-wireshark/Save/amp_cab legacy guitar.json`, `amp_cab legacy bass.json`

---

> **One-line summary.** Same **`c319`** wire as IR, but the cab is **legacy hybrid** (model suffix `64:83:17:c3:19`, short cab token on wire). Cab focus = **`1b`** frame. Cab params: **PP `0x08`**, selectors **`0x25+`** (guitar) or **`0x00+`** (compact) from amp param count.

---

## 1. IR vs Legacy on the same wire

| | **IR** `amp+cab` | **Legacy** `amp+cab-legacy` |
|--|------------------|------------------------------|
| Cab on wire | `cd:03:xx` | Often **2 nibbles** (`47:00`, …) |
| Param model block | `85:62` … `1d:c3:1a:01:1c` | `82:62` … `c3:19` |
| Cab tab focus | `1d`, `cd:03`, `1a:01` | `1b`, `cd:08` |
| Live write PP | `0x03` | `0x08` |

---

## 2. Legacy cab selector tables

Router uses `ampCabAmpParamCount` (visible amp params from UI), **not** `preset_data`.

| Amp block size (proxy) | Table | Example 1st cab param |
|------------------------|-------|------------------------|
| **≥ 10** (guitar) | `LEGACY_GUITAR_CAB_ROUTES` | `pSel=0x25`, tag `0x05` |
| **< 10** (compact) | `LEGACY_COMPACT_CAB_ROUTES` | `pSel=0x00`, tag `0xcb` |

Code: `legacy_cab_wire_pair` in `amp_cab_live_write.rs`.

---

## 3. Legacy cab focus (`1b`)

Sent on Cab tab click and before first cab `write_live_param` if not yet focused. See `build_amp_cab_cab_focus_packet`.

---

## 4. HXLinux UI

| Field | Value |
|-------|--------|
| `dualPart` | `amp` / `cab` |
| `ampCabAssignVariant` | `"amp+cab-legacy"` |
| `ampCabAmpParamCount` | Visible amp param count |
| Cab picker replace | amp variant **legacy** + cab single entry |

---

## 5. Code map

| File | Role |
|------|------|
| `amp_cab_live_write.rs` | Legacy blocks, tables, `1b` focus |
| `edit_slot_model.rs` | `build_amp_cab_replace_cab_bulk` |
| `models.ts` | Tabs, picker, variant resolution |

---

## 6. vs Cab dual legacy

- Cab dual: `dualPart` `cab1`/`cab2`, variant `dual` / `dual-legacy`
- Amp+Cab: `dualPart` `amp`/`cab`, variant `amp+cab` / `amp+cab-legacy`

Use `build_amp_cab_replace_cab_bulk` for cab-only replace, not cab2 dual builders.
