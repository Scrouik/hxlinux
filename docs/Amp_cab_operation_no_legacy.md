# Amp+Cab IR — USB behavior (cab params & focus)

**HXLinux — HX Stomp XL**  
*Reference for **Amp + IR Cab** pairs (`assignVariant: amp+cab`). Legacy hybrid: [Amp_cab_operation_legacy.md](Amp_cab_operation_legacy.md).*

> **French:** [Amp_cab_fonctionnement_no_legacy.md](Amp_cab_fonctionnement_no_legacy.md)  
> **Captures:** `captures/usb-wireshark/Save/amp_cab guitar.json`, `amp_cab bass.json`

---

> **One-line summary.** An Amp+Cab slot is **two models** (amp + cab) in **one bulk** (`…85188317c319…` + `<amp> 1a <cab>`). The UI targets the sub-block with `dualPart` (`amp` / `cab`) and a **local param index**. IR cab live writes use **PP `0x03`**, model suffix **`1d:c3:1a:01:1c`**, Cab tab focus = **`1d` `cd:03` `1a:01`** (same as Cab dual cab2 tab focus).

---

## 1. What is “Amp+Cab”?

| Concept | Detail |
|--------|--------|
| Matrix slot | **One** cell (one `slot_bus`) |
| Content | Linked **amp** + **IR cab** (not two slots) |
| Catalog | `HX_ModelUsbAssign` entry variant **`amp+cab`** |
| UI | **Amp** / **Cab** tabs; Cab tab picker locked to **Cab / Single** |
| Preset wire | **`c319`** marker then **`<amp_hex> 1a <cab_hex>`** |

Not **Cab dual** (two cabs in a Cab slot, `dualPart` `cab1`/`cab2`, variant `dual`).

---

## 2. HXLinux model (no `preset_data` during session)

| Layer | Rule |
|--------|------|
| Sub-model | `dualPart: "amp"` or `"cab"` on `write_live_param` |
| Param index | **Always local** to the active tab (`paramIndexBase = 0`) |
| Assign variant | `ampCabAssignVariant: "amp+cab"` (IR) or `"amp+cab-legacy"` |
| Cab route | `resolve_cab_live_write_route` in `amp_cab_live_write.rs` — catalog + UI counts, not preset dump |
| Display values | Session cache + live overrides; preset dump only on preset load |

---

## 3. **Cab** tab focus (hardware)

| Variant | Head | Model block (excerpt) | Tauri |
|---------|------|------------------------|-------|
| **IR** `amp+cab` | `0x1d` | `83:66:cd:03` … `1a:01` | `focus_amp_cab_usb_part` → `build_amp_cab_ir_cab_focus_packet` |
| **Legacy** `amp+cab-legacy` | `0x1b` | `83:66:cd:08` … | `build_amp_cab_cab_focus_packet` |

IR focus reuses `build_cab_dual_cab2_tab_focus_packet` (`cd:03`, `1a:01`).

---

## 4. Live **cab** param write (IR)

| Field | IR value |
|-------|----------|
| `dualPart` | `cab` |
| `param_index` | Local (e.g. Mic = `0`) |
| PP | **`0x03`** |
| `param_selector` | = local index |
| 16-byte model block | `…85 62 bus 1d c3 1a 01 1c` |

Path: `write_live_param` → generic `build_live_write_frames_from_state` (not Cab dual minimal path).

Logs: `ppSource=amp_cab:ir_capture`, `pSelSource=amp_cab:ir_local_index`.

---

## 5. **Cab-only** replace (picker)

Use `probe_slot_model_usb` with amp id + `cabCatalogModelId`, variant **`amp+cab`**. Rust: `build_amp_cab_replace_cab_bulk` patches only the cab field after `c319`/`1a`.

Lane rule: same `focus → ed:08 → bulk` coherence as Cab dual ([Cab_dual_operation_no_legacy.md](Cab_dual_operation_no_legacy.md) §3).

---

## 6. Code map

| File | Role |
|------|------|
| `src-tauri/src/helix/amp_cab_live_write.rs` | IR/legacy blocks, focus, route resolver |
| `src-tauri/src/lib.rs` | `focus_amp_cab_usb_part`, `write_live_param` |
| `src/models.ts` | Amp/Cab tabs, variants, picker |
| `src/hxModelCatalogMeta.ts` | `amp+cab` variant helpers |

---

## 7. Regression checklist

- [ ] Cab tab click → HW focuses on cab (IR and legacy)
- [ ] Cab params → local `pSel`, `pp=03` (IR)
- [ ] Picker cab change → slot stays Amp+Cab
- [ ] No per-slider `preset_data` parse in session
