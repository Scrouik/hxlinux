# HXLinux

**EN:** Open-source HX Stomp XL editor for Linux (Tauri — Rust + TypeScript).  
**FR:** Éditeur open source pour HX Stomp XL sous Linux (Tauri — Rust + TypeScript).

> **Development in progress · Développement toujours en cours.**

---

## Status · État du projet

# HXLinux — Feature inventory (first usable version)

> **Scope:** HX Stomp XL on Linux · post-commit `First_usable_version` (June 2026).  
> This document is intended as a basis for the GitHub README and communication around the first usable release.

---

## Summary

HXLinux is an open-source editor for Line 6 Helix on Linux (Tauri + Rust + TypeScript). On **HX Stomp XL**, it lets you:

- browse and rename **125 presets**;
- view and edit the **signal chain** (stomp matrix + parameter panel);
- **assign, replace, copy, paste, and move** FX blocks over live USB;
- manage **Amp+Cab** and **Cab Dual** models (tabs + cab replace);
- control **Input** and **Split** Path 1 (live write + hardware scroll);

… all over **native Linux USB**, without HX Edit.

---

## 1. Platform & connection

| Capability | Detail |
|------------|--------|
| Linux desktop app | Tauri 2 (Rust + TypeScript/Vite), 2 windows: presets + models |
| Native USB | HX Stomp XL detection, handshake, protocol state machine |
| Stable session | Clean shutdown without leaving the hardware stuck in editor mode |
| Window geometry | Save/restore position and size |

---

## 2. Main window — presets (125 slots)

| Capability | Detail |
|------------|--------|
| Name list | Reads all 125 presets from the device |
| Preset activation | Click → sent to the HX |
| Rename | Right-click → Rename, inline edit, USB send + optimistic UI update |
| Save to hardware | Right-click → Save (active preset only, non-empty slot) |
| Visual selection | Active preset highlighted |
| List drag & drop | Reordering **UI only** — not yet sent to the HX |
| Load from disk | Menu present, **not implemented** |

---

## 3. Models window — overview

| Capability | Detail |
|------------|--------|
| Stomp matrix grid | 16 FX slots + structural blocks (Input, Output, Split, Merge) |
| Visual routing | Split/merge columns, paths 1 and 2 |
| Preset loading | One-shot USB dump → grid hydration + session parameter cache |
| Loading overlay | UI lock while preset loads |
| Preset banner | Name + “modified” indicator |
| Active HW slot sync | Stomp wheel / buttons → aligned UI selection |

---

## 4. Matrix — FX slot editing

| Capability | Detail |
|------------|--------|
| Slot selection | Click → hardware focus + parameter panel |
| Copy | Right-click on filled slot → model + parameter snapshot |
| Paste | Onto any empty FX cell (same path, other path, **other preset**) |
| Move (DnD) | Pointer Events (not HTML5 DnD): copy → paste → clear source |
| Move v1 constraint | Same path only (0–7 ↔ 0–7, 8–15 ↔ 8–15), empty destination |
| Remove slot | Clears the slot on the hardware |
| Session parameter cache | `preset_data` read **once** on load; then cache + live overrides |
| Move Split/Merge | Routing marker move (partially implemented) |

**Not yet:** inter-path DnD with auto split/merge, free Split/Merge drag, DSP budget.

---

## 5. Picker — model change

| Capability | Detail |
|------------|--------|
| Scrollable list | Assignable FX models (`HX_ModelUsbAssign.json` + catalog) |
| USB assign | `bulkHex` frames captured from HX Edit |
| Remove slot | Same mechanism (clear slot) |
| HW model scroll | Wheel on active slot → USB pull, without systematic re-dump |
| Locked picker | Input, Output, Split, Merge: category fixed by structural bus |
| Exclusions | Non-assignable categories (e.g. Split in the FX list) |

---

## 6. Parameter panel — live editing

### Control types

| Type | Examples |
|------|----------|
| Numeric slider | Gain, Level, Time, etc. (float and integer) |
| Stepped discrete slider | Ratio, Clipping, Wave shape, Compress/Limit Type |
| Boolean toggle | Bright, Fuzz, EQ on/off, polarity |
| Mic combo | Mic selection (dedicated displayType) |
| Graphic EQ | Bands hidden when master EQ is off |
| Value formatting | `HelixControls.json` (units, labels, steps) |

### Display rules (hardware-aligned)

- Hide `stereo-only` parameters in mono
- Hide internal booleans (`@enabled`, `@stereo` — `valueType: 2` without `displayType`)
- Special scales: pan 0…1 → −100…+100, split A/B, etc.
- Wire order: ascending `assign` then JSON order (correct live-write)

### USB writes

- `write_live_param` (float / bool / discrete)
- `write_live_param_midi_cc` when needed

---

## 7. Dual models — Amp+Cab & Cab Dual

| Capability | Amp+Cab | Cab Dual |
|------------|---------|----------|
| Tabs | Amp \| Cab | Cab 1 \| Cab 2 |
| Params per tab | Yes | Yes |
| Secondary tab picker | Cab locked to Single IR | Cab 2 locked to Single IR |
| Cab-only replace | Modern bulk or legacy sequence | Dual bulk hint `c319` |
| USB part focus | Yes | Yes |
| HW scroll | Linked amp + cab detection | Cab1 + cab2 detection |

---

## 8. Path 1 — structural blocks

| Block | Picker | Live write | HW scroll |
|-------|--------|------------|-----------|
| **Input** | Locked | Yes | Yes |
| **Split** | Locked | Yes | Yes (Y/A/B encoding inverted: scroll vs select) |
| **Output** | Locked + focus | No | No |
| **Merge** | Locked + focus | No | No |

I/O and flow params (Split/Merge): read from `preset_data` + panel display.

---

## 9. Hardware ↔ UI sync

| Event | Trigger |
|-------|---------|
| `models:hardware-slot-changed` | Active slot change on the HX |
| `models:slot-model-changed` | Scroll / model change |
| `models:slot-param-changed` | Knob twist on hardware |
| `models:slot-content-changed` | Slot content watch |
| `models:path1-input-source-changed` | Input scroll / echo |
| `models:path1-split-type-changed` | Split scroll / echo |
| `models:preset-saved` | After preset save |

Soft-sync: no full re-parse between dumps when unnecessary.

---

## 10. USB infrastructure (under the hood)

- Preset read ed:03 (16-bit counter bugs, double-editor issues fixed)
- Phase B handshake (editor commit)
- Keep-alive and coupled lanes
- Multi-notch scroll without ED03 freeze
- Preset reader recovery
- Chain value parsing (Amp+Cab, Cab Dual, `c319`/`c219`)
- Stomp layout

---

## 11. Not implemented or partial

| Area | Status |
|------|--------|
| Helix LT / Floor | Not supported (4-path topology, 2 DSP) |
| DSP budget (`load` in `.models`) | Not calculated |
| Output / Merge live write + scroll | Partial |
| Inter-path DnD with auto split/merge | Planned |
| Preset reordering on HX | UI only |
| Preset file import/export | Not done |
| Load preset from disk | Stub |
| `bulkHex` campaign | Partial catalog coverage |

---

## Requirements

- Linux (tested on Ubuntu/Debian family)
- Line 6 **HX Stomp XL** connected via USB
- **HX Edit** installed (to provide model metadata files)

## Run the application

```bash
npm run tauri dev    # development
npm run tauri build  # production build
```

## Credits

USB reverse engineering inspired by [kempline/helix_usb](https://github.com/kempline/helix_usb).

## Project notes · Notes projet

- Rust mode state machine for USB protocol phases
- Async USB (listener / writer threads + channels)
- Protocol reference: [`docs/Référence protocole USB HX Stomp XL.md`](docs/Référence%20protocole%20USB%20HX%20Stomp%20XL.md)

---
