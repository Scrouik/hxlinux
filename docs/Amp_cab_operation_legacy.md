# Amp+Cab Legacy — USB protocol (hybrid cab)

**HXLinux — HX Stomp XL**  
*Legacy **hybrid** cab in an amp+cab pair (`assignVariant: amp+cab-legacy`). Complements [Amp_cab_operation_no_legacy.md](Amp_cab_operation_no_legacy.md) (IR).*

> **French:** [Amp_cab_fonctionnement_legacy.md](Amp_cab_fonctionnement_legacy.md)  
> **Key captures:**
> - `captures/usb-wireshark/Save/amp_cab legacy guitar.json` — assign, scroll, bulk families
> - `captures/usb-wireshark/ampcab_legacy_switch_tab.json` — Amp / Cab tab focus (`1d`)
> - `captures/usb-wireshark/ampcab_legacy_change_cab_HXEdit.json` — cab replace WhoWatt → Soup Pro (#4401)  
> **Standalone Cab single legacy** (`c2:19`): see [Cab_single_operation_legacy.md](Cab_single_operation_legacy.md) — **not** this document.

---

> **Summary (Jun 2026, HW validated).** Same wire marker **`c3:19`** as IR, **legacy hybrid** cab (short hint or `cd:02:xx`). Assign and model replace use bulk from **`HX_ModelUsbAssign.json`** (`variant: amp+cab-legacy`), **not** `preset_data`. Model block lane: **`cd:07`** on assign, **`cd:03`** on cab replace and tab focus. UI focus = **`1d`** frame (not `1b`) + **`ed:08`**. Picker rule: **any legacy cab** on **Amp+Cab Legacy**; **any IR cab** on **Amp+Cab** IR.

---

## 1. IR vs Legacy on the same wire

| | **IR** `amp+cab` | **Legacy** `amp+cab-legacy` |
|--|------------------|------------------------------|
| Cab on wire | `cd:03:xx` (3 B, MicIr) | **1 byte** (`33`, `47`…) or **`cd:02:xx`** (3 B) depending on amp |
| Param model block (live write) | `85:62` … `1d:c3:1a:01:1c`, PP **`0x03`** | `82:62` … `64:83:17:c3:19`, PP **`0x08`** |
| Model focus / cab replace | `1d`, `cd:03`, `1a:01` → `ed:08` → bulk | **same** (`1d`, not `1b`) |
| Picker | Cab **Single** (IR) | Cab **Single Legacy** |
| USB variant | `amp+cab` | `amp+cab-legacy` |

Catalog entries: Guitar/Bass pairs injected (`sync_usb_assign_from_catalog.py`), bulk with **`83:17:c3:19`** + amp/cab pair on wire.

---

## 2. Lane `cd:07` (assign) vs `cd:03` (replace / focus)

Catalog bulk **`amp+cab-legacy`** embeds model block `83:66:cd:**07**:TAG` (e.g. tag `fb` on WhoWatt, frame **#1259**).

| Phase | `cd` lane | Capture example |
|-------|-----------|-----------------|
| **Assign** empty slot (`AddToEmpty`) | **`07`** | `amp_cab legacy guitar.json` #1259 — head `23`, 44 B |
| **Cab-only replace** | **`03`** | `ampcab_legacy_change_cab_HXEdit.json` #4401 — reframe `07→03` before send |
| **Amp / Cab tab focus** | **`03`** | `ampcab_legacy_switch_tab.json` — suffix `1a:00` (Amp) or `1a:01` (Cab) |

**Fixed pitfall:** sending assign bulk as-is (`cd:07`) on cab replace → app logs “OK”, **HW ignores**.  
**Implementation:** `reframe_legacy_replace_cd07_to_cd03` in `amp_cab_cab_replace.rs`; same reframe for optional create template `2d` (see §5).

Session tag (byte after `cd:XX`) stored per slot:
- **Assign:** `(amp_tag, cab_tag)` from bulk (`cd:07` or `cd:03`), e.g. `fb`
- **Cab replace:** updates **`cab_tag`** only
- **Amp tab focus:** use **`amp_tag`** (do not reuse `live_write_yy` after replace — wrong sub-block, e.g. Soup Pro shown instead of WhoWatt)

---

## 3. Amp / Cab tab focus — `1d` frame

HX Edit (and HXLinux since Jun 2026) switches Amp/Cab tabs with **`1d`**, not `1b`.

### 3.1 Envelope (capture `ampcab_legacy_switch_tab.json`)

```text
1d … 80:10:ed:03 … sub=04 … 83:66:cd:03:TAG:64:4e:65:82:62:bus:1a:SUFFIX:00:00:00
                                                      ↑              ↑
                                                   focus lane    00=Amp, 01=Cab
```

Then **`ed:08`** (~93 ms), then **`f0`** poke:
- **Cab** tab: `f0:08`
- **Amp** tab: `f0:10` then `f0:08` (frame **#2659**)

Tags observed after assign / tab switches: `fb` → `fc` → `fd` → `fe` (session progression; Amp focus must keep **amp** tag from assign).

### 3.2 Cab replace — cab focus **required** before bulk

HW-validated sequence (`execute_amp_cab_cab_replace`):

```text
1d cab focus (cd:03, 1a:01)  →  ed:08  →  ~1100 ms  →  replace bulk (head 23/25/27)
```

**Why:** without prior cab focus, device is not on cab sub-block; bulk logs “OK” but HW does not change cab (or corrupts amp state).

Legacy and IR share this **model replace** sequence; only bulk content differs.

### 3.3 `1b` / `cd:08` (historical)

Older captures (`amp_cab legacy guitar.json`) show **`1b`** + `cd:08` for cab **param** live writes. **Model** path (assign / replace / UI tabs) uses **`1d` + `cd:03`**. Do not mix the two.

---

## 4. Initial assign (first click on empty slot)

### 4.1 Bulk to send

| Attempt | HW result |
|---------|-----------|
| head **`2d`**, 56 B, `cd:03` (create template) | ❌ first click ignored; 2nd click (replace) only worked |
| head **`23`**, 44 B, `cd:07` (catalog assign bulkHex) | ✅ frame **#1259** |

**Fix:** `HX_AMP_CAB_LEGACY_CREATE_HEAD2D` **OFF** by default — first click sends same **`23` / `cd:07`** catalog bulk, not `2d` template.

Bulk bytes **14–15:** **`02 00`** on heads `23` / `25` / `27` — **do not overwrite** to `00 00`.

### 4.2 UI

- After probe add: `suppressNextAmpCabFocusUsb` to avoid spurious `1d` focus on re-render
- Pinned variant: **`amp+cab-legacy`** (do not fall back to IR on HW scroll)

---

## 5. Cab-only replace (Cab tab picker)

### 5.1 Bulk construction

`build_amp_cab_replace_cab_bulk`:
1. Copy parent **amp** bulk (`amp+cab-legacy`)
2. Patch **only** cab field after `c3:19` / `1a`
3. **Keep amp wire** before `1a` (e.g. WhoWatt `2c`) — **never** copy from another catalog entry sharing the same cab hint

**Fixed pitfall (“similar names”):** catalog lookup “amp+cab-legacy with same cab” replaced wire `2c` with `23` (Soup Pro) → returning to Amp tab showed wrong amp.  
HX Edit capture #4401: WhoWatt + Soup Pro = **`2c 1a 33`**, not `23 1a 33`.

### 5.2 Cab encoding — compact vs long

Two shapes on wire `… c3:19 <wire> 1a <cab> …`:

| Amp family (ex.) | Head | Amp wire | Default cab | Ex. |
|------------------|------|----------|-------------|-----|
| **Compact** | `23` (44 B) | 1 B (`2c`, `2b`…) | 1 B (`47`, `34`…) | WhoWatt, US Small Tweed, Tuck’n Go |
| **Long** | `27` (48 B) | 3 B `cd:02:xx` | 3 B `cd:02:xx` | Fullerton Jump, US Princess |
| **Mixed** | `25` (48 B) | 3 B | 1 B | Voltage Queen, US Super |

**Product picker rule:** on Amp+Cab Legacy, **any legacy cab**; on Amp+Cab IR, **any IR cab**.

**Wire adaptation** (`cab_field_bytes_for_amp_cab_replace`):

| Parent slot | Catalog cab | Field sent |
|-------------|-------------|------------|
| 1 B | hint `33` | `33` |
| 3 B | hint `33` | `cd:02:33` |
| 1 B | hint `cd024e` | `4e` (3rd byte of `cd:02:4e`) |
| 3 B | hint `cd024e` | `cd:02:4e` |

Do **not** reject `cd02xx` cab on compact slot — HX Edit allows it (e.g. US Small Tweed + 1x12 US Princess).

### 5.3 Probe / UI

| Field | Value |
|-------|--------|
| `dualPart` | `amp` / `cab` |
| Amp `assignVariant` | `"amp+cab-legacy"` |
| Probe | `replace` + `catalogModelId` (amp) + `cabCatalogModelId` + `cabAssignVariant` (`single` / `legacy`) |
| Optimistic UI | merge grace; no `preset_data` re-read |

---

## 6. Legacy cab param live-write tables

Router receives `ampCabAmpParamCount` = **visible** amp panel params.

| Amp block size (proxy) | Table | Example cab Level |
|------------------------|-------|-------------------|
| **≥ 10** (guitar) | `LEGACY_GUITAR_CAB_ROUTES` | `pSel=0x25`, tag `0x05` |
| **< 10** (compact / bass) | `LEGACY_COMPACT_CAB_ROUTES` | `pSel=0x00`, tag `0xcb` |

Code: `legacy_cab_wire_pair` in `amp_cab_live_write.rs`.

---

## 7. Bugs encountered and fixes

| Symptom | Cause | Fix |
|---------|-------|-----|
| First assign click ignored by HW | bulk `2d` / `cd:03` instead of `23` / `cd:07` | head `23` by default |
| Cab replace “OK” in logs, HW unchanged | no cab focus; or `cd:07` instead of `cd:03` | `1d` → `ed:08` → bulk; reframe `07→03` |
| Cab works when name similar to amp; amp switches on Amp tab | amp wire overwritten via catalog (e.g. `2c→23`) | keep parent amp wire |
| Soup Pro on Amp tab after replace | Amp focus with cab session tag / `live_write_yy` | **amp** tag from assign |
| Fullerton + Soup Pro: cab size error | hint `33` not expanded to `cd:02:33` | compact ↔ long conversion |
| Small Tweed + Princess: “hybrid short” rejection | HXLinux guard (not HX Edit) | hint `cd024e` → byte `4e` |
| Bytes 14–15 set to `00 00` | aggressive replace finalize | keep `02 00` on known heads |

---

## 8. Code map

| File | Role |
|------|------|
| `amp_cab_cab_replace.rs` | Cab replace: focus `1d` → `ed:08` → bulk; reframe `cd:07→cd:03` |
| `amp_cab_live_write.rs` | Tab focus `1d`, session tags, legacy PP tables, assign/replace record |
| `edit_slot_model.rs` | `build_amp_cab_replace_cab_bulk`, assign head `23`, compact/long cab encoding |
| `cab_dual/legacy/wire.rs` | `legacy_compact_hint_to_cd02_field`, `legacy_cd02_field_to_compact_hint` |
| `models.ts` | Picker, `applyAmpCabCabFromPicker`, tab focus, `suppressNextAmpCabFocusUsb` |
| `lib.rs` | `probe_slot_model_usb`, `focus_amp_cab_usb_part`, `record_amp_cab_assign_session` |

---

## 9. Legacy regression checklist

- [x] First assign on empty slot → HW reacts (bulk `23`, `cd:07`)
- [x] Cab picker replace → cab focus then bulk; HW changes cab
- [x] Cab replace: amp wire unchanged (WhoWatt `2c` + Soup Pro `33`)
- [x] Return to Amp tab after replace → correct amp (amp tag, not cab tag)
- [x] Compact + `cd02xx` cab (Princess on Small Tweed)
- [x] Long + compact cab (Fullerton + Soup Pro → `cd:02:33`)
- [ ] Cab param live write → `pp=08`, guitar/compact selector consistent
- [ ] Picker stays Legacy after HW scroll
- [ ] No IR fallback (`amp+cab`) on legacy slot

---

## 10. vs Cab dual legacy

Legacy duals share `c3:19` and hybrid hints, but:

- Cab dual → `dualPart` `cab1`/`cab2`, variant `dual` / `dual-legacy`
- Amp+Cab → `dualPart` `amp`/`cab`, variant `amp+cab` / `amp+cab-legacy`

Do not reuse cab2 dual replace builders for Amp+Cab cab: use **`build_amp_cab_replace_cab_bulk`**.
