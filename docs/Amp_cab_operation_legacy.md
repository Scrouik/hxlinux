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

## 4. **Cab-only** replace (picker) — HW validated Jun 2026

Bulk is built from **`HX_ModelUsbAssign.json`** (`build_amp_cab_replace_cab_bulk`); only the cab field after `c319`/`1a` is patched. **`preset_data` is not used** on this path.

### 4.1 Legacy USB sequence (≠ IR, ≠ Cab dual)

| Step | Legacy `amp+cab-legacy` | IR `amp+cab` |
|------|-------------------------|--------------|
| Preamble | **`ef` → `f0`** (16 B each) | `1d` cab focus → **`ed:08`** |
| Bulk | head **`0x23`** (44 B) or **`0x25`** (48 B) | **`0x27`** / `0x25` per catalog |
| Bulk bytes **14–15** | keep **`02 00`** | `0x27`: may zero; `0x25`: keep `02 00` |

**Fixed pitfall:** `focus → ed:08 → bulk` or zeroing bytes 14–15 logged “OK” but the device ignored the replace. Legacy must match **initial assign** (`AddToEmpty`: `ef/f0/bulk`). Ref. capture: `amp_cab legacy bass.json` frame **1357**.

Code: `execute_amp_cab_cab_replace` in `amp_cab_cab_replace.rs` (`legacy=true`).

### 4.2 Compact cab token

1-byte legacy slots use the cab entry’s **`chainHexHint`** (`33`, `37`, …), not full IR `cd02xx`. Oversized cabs are rejected before send.

### 4.3 UI

| Field | Value |
|-------|--------|
| `dualPart` | `amp` / `cab` |
| `ampCabAssignVariant` | `"amp+cab-legacy"` |
| `ampCabAmpParamCount` | Visible amp param count |
| Cab picker | **Cab / Single Legacy**; probe `replace` + `cabCatalogModelId` |

`1b` focus (§3) is for the Cab tab and **param** live writes, not the cab **model** replace sequence.

---

## 5. Code map

| File | Role |
|------|------|
| `amp_cab_cab_replace.rs` | Replace fire: `ef/f0/bulk` (legacy) vs `focus/ed:08/bulk` (IR) |
| `amp_cab_live_write.rs` | Legacy blocks, tables, `1b` focus |
| `edit_slot_model.rs` | `build_amp_cab_replace_cab_bulk`, `chainHexHint`, 1-byte cab field |
| `models.ts` | Picker variant, `applyAmpCabCabFromPicker` |
| `hxModelCatalogMeta.ts` | Amp family scroll keeps Legacy variant |

---

## 6. vs Cab dual legacy

- Cab dual: `dualPart` `cab1`/`cab2`, variant `dual` / `dual-legacy`
- Amp+Cab: `dualPart` `amp`/`cab`, variant `amp+cab` / `amp+cab-legacy`

Use `build_amp_cab_replace_cab_bulk` for cab-only replace, not cab2 dual builders.
