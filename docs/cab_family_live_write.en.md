# HXLinux — Parameter writes and cab changes: the 4 families

> **Français:** [cab_family_live_write.md](cab_family_live_write.md)

> **Scope.** This document describes the live-write protocol (`write_live_param`) and cab
> changes for the **four cab families in standalone Cab / dual Cab slots** on the HX Stomp
> XL: **single modern**, **dual modern**, **single legacy**, **dual legacy**. Everything
> comes from Wireshark/usbmon captures (*capture-first* method — no speculation). Each
> behavior is guarded by an **env-var flag** (default ON; `=0` restores the old behavior /
> witness).

> **Out of scope — Amp+Cab legacy hybrid.** **Amp+Cab** pairs (`assignVariant:
> amp+cab-legacy`) follow a different path: cab focus **`1b`**, live write **PP `0x08`**,
> selectors **`0x25+`** / compact tables — not the IR `23`/`27` builder described here. See
> [Amp_cab_fonctionnement_legacy.md](Amp_cab_fonctionnement_legacy.md) /
> [Amp_cab_operation_legacy.md](Amp_cab_operation_legacy.md). **Amp+Cab IR** (modern cab in
> a pair) is covered by
> [Amp_cab_fonctionnement_no_legacy.md](Amp_cab_fonctionnement_no_legacy.md).

### Code map (Rust + UI)

| Area | Files |
|------|-------|
| Live-write routing | `src-tauri/src/helix/live_write.rs`, `src-tauri/src/lib.rs` (`write_live_param`) |
| Single cab / legacy hints | `src-tauri/src/helix/amp_cab_live_write.rs` |
| Dual cab IR + legacy route | `src-tauri/src/helix/cab_dual_live_write.rs` |
| Dual legacy cab2 replace | `src-tauri/src/helix/cab_dual/legacy/wire.rs`, `cab_dual_cab2_replace.rs` |
| Legacy handshake (avoided) | `src-tauri/src/helix/legacy_cab_param_commit.rs` |
| UI params + cab picker | `src/models.ts` (`appendModelsParamRows`, `renderModelsParamsDualTabs`, `applyCabDualCabFromPickerListClick`) |

### Capture index (quick reference)

| Family | Main captures (`captures/usb-wireshark/Save/`) |
|--------|------------------------------------------------|
| Single modern | `cab single.json` |
| Dual modern | `IR Dual.json`, `add_dual_cab_modif_param_cab2.json`, `add_dual_cab_soup_pro_2x12bluebell_HXEdit.json` |
| Single legacy | `cab single legacy.json` |
| Dual legacy | `cab dual legacy.json`, `add_dual_legacy_change_cab2.json`, `add_dual_legacy_change_cab2_&_dual.json` |

See also the inventory: [captures/usb-wireshark/README.md](../captures/usb-wireshark/README.md).

---

## 0. Vocabulary and frame landmarks

A parameter write frame (`23` discrete/bool, `27` float) ends with a model block of the form:

```
83 66 cd <KK> <tag> 64 <op> 65 <85|82> 62 <bus> 1d <VT> 1a <CAB> 1c <pSel> 77 <val> 00
            └─KK─┘                                     │      │       │
            03 = single / cab1                         │      │       └─ pSel: parameter selector
            04 = dual / cab2                           │      └─ CAB: 00 = cab1, 01 = cab2 (dual)
                                                       └─ VT: value-type marker
                                                              c2 = legacy discrete
                                                              c3 = float (and modern discrete via static replay)
```

Three bytes carry most of the differentiation logic between families:

| Byte | Role | Values |
|------|------|--------|
| `cd <KK>` | model block | `cd 03` = single / cab1 · `cd 04` = dual / cab2 |
| `<VT>` (right after `1d`) | value type on the wire | `c2` = legacy discrete · `c3` = float / modern discrete |
| `<CAB>` (right after `1a`) | cab index in a dual | `00` = cab1 · `01` = cab2 |

**Assignment marker `c2:19` vs legacy marker.** Watch out for a trap: the `c2:19` present in
the **assignment bulk** is a “**single cab**” marker — it is also carried by modern MicIr
(`c2 19 cd03xx`). The real *legacy* discriminator is not this `c2:19`, but the **shape of the
cab field**: a legacy cab has a **1-byte hint**, a modern cab has a **3-byte `cd 03 xx`
block**.

---

## 1. Cab single modern

> **Status: RESOLVED.** Parameters + `@mic` work.

**Code:** `live_write.rs` (`force_discrete_c2_marker`, `standalone_legacy_assign_is_one_byte_hint`),
`amp_cab_live_write.rs` (single IR blocks).

**Captures:** `captures/usb-wireshark/Save/cab single.json` (assign + MicIr params, `cd 03 xx` block).

### Particularity

Single modern writes its discretes with the `c3` marker (from **static replay** of the
template), *not* `c2`. That is the exact opposite of single legacy.

### The problem we hit

While tuning single legacy, we forced a `c2` marker on discretes via
`force_discrete_c2_marker()`. That forcing, applied unconditionally, **broke single modern**:
its discrete `@mic` went out as `c2` instead of the expected `c3`, and the device ignored it.

The decisive test was `HX_CAB_DISCRETE_C2=0` (forcing disabled):

> With forcing OFF, single **modern** worked again, single **legacy** broke again. Direct
> proof that `c2` is **for legacy only** and **wrong for modern**.

### The solution

Gate the `c2` placement on the legacy signature — i.e. the **1-byte cab hint**. Helper
`standalone_legacy_assign_is_one_byte_hint()`: the `c2` flag is set **only** if the cab field
is 1 byte (legacy). Modern (3-byte `cd 03 xx` block) keeps its `c3`.

*Single modern therefore has no specific code: it is “what remains” when we do not set `c2`.
Lesson: a legacy fix must never be applied unconditionally.*

---

## 2. Dual cab single modern (dual modern)

> **Status: RESOLVED** (earlier sessions). All params + `@mic`, cab1 and cab2.

**Code:** `cab_dual_live_write.rs` (`resolve_cab_dual_live_write_route`,
`build_cab_dual_minimal_param_packets_from_state`, `ed:08` arming),
`cab_dual_cab2_replace.rs` (cab2 IR replace).

**Captures:** `IR Dual.json`, `add_dual_cab_modif_param_cab2.json`,
`add_dual_cab_soup_pro_2x12bluebell_HXEdit.json`, `cab dual change right.json` (focus / IR replace).

### Particularities

Dual modern introduces two mechanisms absent from single:

1. **cab1 / cab2 distinction via the model block**: cab1 = `cd 03`, cab2 = `cd 04`.
2. **Cab index in the byte after `1a`**: `00` for cab1, `01` for cab2.

### Points that had to be solved

| Point | Device requirement |
|-------|---------------------|
| cab2 models | dual context `cd031c` / `c3 19` (mixing with single fails) |
| Arming | **`ed:08` required before each `23`/`27`** (otherwise write silently ignored) |
| Model block | `cd:03` (cab1) / `cd:04` (cab2), not `cd:04` everywhere |
| Cab index | in the byte after `1a` (`00`/`01`) |
| Param index | **local (0-based) per cab**, not one continuous global index |

> **`ed:08` is the golden rule for dual.** The device emits no error if a `23`/`27` arrives
> without prior arming — it ignores it silently. Every dual write must be preceded by its
> `ed:08`.

*Dual modern is the “reference shape”: the two other (legacy) families are routed through its
builder once their specifics are handled.*

---

## 3. Cab single legacy

> **Status: RESOLVED.** Parameters + `@mic` work.

**Code:** `live_write.rs`, `amp_cab_live_write.rs` (legacy `23`/`27`/`57` burst),
`legacy_cab_param_commit.rs` (async handshake — **disabled** when `HX_LEGACY_SINGLE_IR_PARAM`),
`models.ts` (`wireLocal` via `cabAssignVariant === "legacy"`).

**Captures:** `captures/usb-wireshark/Save/cab single legacy.json` (scroll, `@mic`, `c2` discretes, `57` floats).

### Particularity

A single legacy is **not** written with a minimal burst: it uses the **standard IR frame**
`23` (discrete) / `27` (float), at `cd:03`, but with two quirks — the discrete `c2` marker,
and a **wire-local** parameter selector.

Observed discrete form:

```
83 66 cd 03 <tag> 64 1e 65 85 62 <bus> 1d c2 1a 00 1c <pSel> 77 <payload>
                                          └c2┘        └wire-local pSel┘
```

### Three stacked bugs (and their fixes)

**Bug 1 — Wrong routing (parasitic async handshake).**
A test `standalone_legacy_assign_uses_cd03ff` fired incorrectly and routed single legacy to
an incorrect async handshake path.
**Fix:** `HX_LEGACY_SINGLE_IR_PARAM` flag takes priority — all single legacy go through the
standard IR path (`route_override = None`).

**Bug 2 — Frozen `c3` marker instead of `c2`.**
`assemble_23_bool_write` hard-coded `c3` for discretes, while the legacy device expects `c2`.
**Fix:** `force_discrete_c2_marker()` rewrites byte `start+12` of the `83 66 cd…` block to
`c2`, **post-finalization**. Flag `HX_CAB_DISCRETE_C2`.
**Essential condition** (see §1): `c2` is set **only** if the cab hint is 1 byte
(`standalone_legacy_assign_is_one_byte_hint()`), so single modern is not broken.

**Bug 3 — Global parameter selector instead of wire-local.**
HXLinux sent `pSel` = flat global index (mic counted in the total). The device numbers `pSel`
**0-based, separately per wave-type group** (discrete vs float).
**Symptom:** the 1st parameter was injected into the 2nd, and so on — a `+1` offset.
**Fix:** **wire-local** selector in `liveWriteParamIndexForRow` (`wireLocal` param),
localStorage flag `models_wire_local_param_selector` (`=0` disables wire-local).
Wire-local applies **only to single legacy**: `renderModelsParamsPane` passes
`cabAssignVariant === "legacy"` as the 13th argument to `appendModelsParamRows`.

### Single legacy summary table

| Symptom | Cause | Fix | Flag |
|---------|-------|-----|------|
| Parasitic async handshake | false positive `..._uses_cd03ff` | standard IR path priority | `HX_LEGACY_SINGLE_IR_PARAM` |
| Mute `@mic` (outputs `c3`) | `assemble_23_bool_write` codes `c3` | `force_discrete_c2_marker()` post-finalize | `HX_CAB_DISCRETE_C2` |
| 1st param → 2nd (+1 offset) | global `pSel` instead of local | wire-local selector | `models_wire_local_param_selector` |

*Single legacy was the lab: that is where we learned the discrete `c2` marker and the
wire-local principle — both reused later for dual legacy.*

---

## 4. Cab dual legacy

> **Status: RESOLVED.** cab2 change + all params + `@mic` (cab1 and cab2). UI display for
> cab2 change fixed.

**Code:** `cab_dual/legacy/wire.rs` (`build_legacy_cab2_replace_bulk`,
`CAB_DUAL_LEGACY_CAB2_REPLACE_23_TEMPLATE`), `cab_dual_live_write.rs` (`dual_legacy_standard_param_enabled`,
`discrete_wants_c2`), `live_write.rs`, `models.ts` (`applyCabDualCabFromPickerListClick`,
`renderModelsParamsDualTabs`).

**Captures:** `cab dual legacy.json` (ADD, params, `71` floats),
`add_dual_legacy_change_cab2.json` (cab2 replace compact `23` 44 B),
`add_dual_legacy_change_cab2_&_dual.json` (full session change + params).

This is the most complex family: it combines dual modern constraints (cab1/cab2, `ed:08`, cab
index) **and** legacy ones (`c2` discrete, wire-local), plus its own frame shape for cab2
change.

### 4.1 Wire truth (captures)

**ADD** (`2d`, 56 bytes) — dual marker `c3 19`, two 1-byte hints:

```
… 83 66 cd 04 <tag> 64 27 65 82 62 <bus> 63 82 13 06 14 83 18 83 17 c3 19 <cab1=33> 1a <cab2=30> 09 10 0a c3 …
                                                                      └c3 19┘ (dual legacy marker)
```

**FOCUS cab2** (`1d`, 40 bytes):

```
… 83 66 cd 04 <tag> 64 4e 65 82 62 <bus> 1a 01 00 00 00
```

**CAB2 CHANGE** (`23`, 44 bytes — **compact** frame, *not* a bulk):

```
83 66 cd 04 <tag> 64 28 65 82 62 <bus> 64 83 17 c3 19 <cab1> 1a <cab2new> 00
                                                 └c3 19┘     └1a┘└new cab2┘
```

**PARAM** (`23`, 44 bytes — discrete) — **same as dual modern**, with `c2` discrete, cab
index after `1a`, wire-local `pSel`:

```
83 66 cd 04 <tag> 64 1e 65 85 62 <bus> 1d c2 1a <00|01> 1c <pSel> 77 <val> 00
                                          └c2┘   └cab┘      └wire-local┘
```

> The dual legacy float `27` form was **not** observed in captures; it is inferred by symmetry
> with the discrete. Validate if a float case misbehaves.

### 4.2 cab2 change

The 1-byte branch of `build_legacy_cab2_replace_bulk` (wire.rs) was replaced by a
*capture-grounded* template `CAB_DUAL_LEGACY_CAB2_REPLACE_23_TEMPLATE` (44 B):

- cab1 at index 40, cab2 at index 42, bus at index 34 (patched by wrapper
  `build_slot_model_probe_packets`).
- Flag `HX_DUAL_LEGACY_CAB2_23_TEMPLATE`. **→ Hardware validated.**

### 4.3 Parameters: routing via the dual modern builder

Rather than a specific hybrid burst, dual legacy borrows the dual modern builder
(`build_cab_dual_minimal_param_packets_from_state`).

- `write_live_param`: guard `route_is_dual_legacy_cab(&route) && !dual_legacy_standard_param_enabled()`,
  flag `HX_DUAL_LEGACY_STD_PARAM`.
- `resolve_cab_dual_live_write_route`: in `standard_legacy`, uses the modern IR block
  (`build_cab_dual_cab1/cab2_ir_param_model_block`, `cd 03`), `param_selector = param_index`
  local, bypasses echo cache.

> **Current HW-accepted state (`cd 03` vs `cd 04`).** The §4.1 capture shows dual legacy
> discrete params on **`cd 04`**. The `standard_legacy` path (flag ON) sometimes emits
> **`cd 03`** on the model block — notably `@mic` (§4.5). The device accepts it today; do not
> “fix” without a failure capture. If a write is ignored, try a `cd 04` override on
> `model_block` before any other change.

### 4.4 Wire-local selector (+1 offset)

Same bug as single legacy: `pSel` was sent as a global index → the 1st param was injected
into the 2nd.

**Fix:** `renderModelsParamsDualTabs` passes a 13th argument
`cabDualLegacyWireLocal = dualSlotKind === "cab_dual" && cabDualAssignVariant === "dual-legacy"`
to `appendModelsParamRows`. Dual **modern** keeps `"dual"` → wire-local OFF → global index
unchanged.

```typescript
const cabDualLegacyWireLocal =
  dualSlotKind === "cab_dual" &&
  (cabDualAssignVariant ?? "").trim().toLowerCase() === "dual-legacy";
```

### 4.5 `@mic`: fragile shared flag → context carried by the route

This was the last — and most instructive — bug. Symptom: *“everything works except the mic”*,
on cab1 **and** cab2.

**Byte-level diagnosis.** `@mic` did output `opcode=23`, `pSel=00`, correct `1a 00`/`1a 01` —
**but with `c3` instead of `c2`**:

```
cab1: … 83 66 cd 03 19 … 1d c3 1a 00 1c 00 77 08 00
cab2: … 83 66 cd 03 1a … 1d c3 1a 01 1c 00 77 0b 00
                            └c3┘ ← should be c2
```

**Root cause.** Dual builder `c2` depended on the shared state flag
`force_discrete_c2_for_legacy_single`. That flag is **consumed** (reset to `false`) by **every**
write — including a preceding **float** write. When you move a float slider then the mic, the
float consumes the flag → the following discrete `@mic` no longer sees it → outputs `c3`. Other
params (floats) still work because they do not need `c2`: only discrete `@mic` suffered.
Hence *“everything works except the mic”*.

**Solution.** Stop depending on fragile shared state: **carry legacy context in the route**.

1. Field `discrete_wants_c2: bool` added to `LiveWriteRouteOverride` (init `false` everywhere).
2. `resolve_cab_dual_live_write_route`: `discrete_wants_c2: standard_legacy` in non-echo return,
   `false` in echo-cache branch.
3. `build_cab_dual_minimal_param_packets_from_state`:
   ```rust
   let force_c2 = route.discrete_wants_c2 || state.force_discrete_c2_for_legacy_single;
   ```

Thus `c2` is decided by the `@mic` route itself, on every call, without depending on write
order. **→ Hardware validated** (cab1 + cab2).

> **Note `cd 03` vs `cd 04`.** `@mic` `frame_b` outputs `cd 03` (not `cd 04` like the dual
> legacy capture). The device accepts it — not changed because it works. If a future case
> fails, force `cd 04` via `model_block` override.

### 4.6 UI display on cab2 change

> **Guiding principle (never regress): a cab change reads NOTHING from hardware.** The new
> cab’s parameters are the **defaults from the chosen cab’s `.models`** (JSON). The wire
> (`moduleHex`) must never drive this resolution.

**Symptom.** On cab2 change, the UI kept the **old** cab2’s parameters (hardware did change
correctly).

**Cause.** Since `cabDualWireParts` also parses legacy, the rebuild started from the
**optimistic wire** (`optimisticSlot.moduleHex`), which still carried the old cab2 until the
next dump. The chain `resolveCabDualTabPanes` → `buildDualTabPanesFromCabDualWire` rebuilt
pane2 from the old cab2.

**Solution (two parts).**

1. Derive the hex of the **actually chosen** cab (`cab2PickerCatalogId`) via
   `moduleHexForUsbVariant(...)`, instead of the optimistic wire, in
   `applyCabDualCabFromPickerListClick` (`tab === 1` branch).
2. Force a **full panel rebuild** after `finally`, invalidating the “values-only patch” path:
   ```typescript
   if (tab === 1 && selectedParamsKemplineSlotIndex === ki &&
       lastProbePickerAssignContext?.ki === ki) {
     selectedParamsInPlaceUpdater = null;   // invalidates canPatchValuesOnly
     selectedParamsInPlaceSlotKey = null;
     selectedParamsValuesSig = null;
     const slotNow = lastHwSyncNormalizedSlots?.[ki] ?? optimisticSlot;
     await loadAndShowModelsParamsForSlot(slotNow, ki);
   }
   ```
   All three `= null` are essential: without them, `loadAndShowModelsParamsForSlot` goes
   through `canPatchValuesOnly`, which only updates cab1.

*The right rebuild anchor is the **ID of the freshly chosen cab2**
(`lastProbePickerAssignContext.cabDualCab2ModelId` → `probeCab2Hint`), not the wire — faithful
to “no HW read on cab change”.*

---

## 5. Cross-family summary table

| | Single modern | Dual modern | Single legacy | Dual legacy |
|---|---|---|---|---|
| Model block | `cd 03` | `cd 03`/`cd 04` | `cd 03` | `cd 04` (params: `cd 03` tolerated) |
| Discrete marker | `c3` (replay) | `c3` | **`c2`** | **`c2`** |
| Float marker | `c3` | `c3` | `c3` | `c3` |
| Cab index (`1a …`) | — | `00`/`01` | `00` | `00`/`01` |
| `pSel` selector | global | global | **wire-local** | **wire-local** |
| `ed:08` arming | — | **required** | — | **required** |
| Cab hint | `cd 03 xx` block (3 B) | block (3 B) | **1 byte** | **1 byte** ×2 |
| Parameter builder | single frames | dual minimal | single frames (IR) | dual minimal (`discrete_wants_c2` route) |

---

## 6. Flag reference

| Flag | Default | Effect (`=0` = witness / old behavior) |
|------|---------|----------------------------------------|
| `HX_LEGACY_SINGLE_IR_PARAM` | ON | Single legacy via standard IR path (no parasitic async handshake) |
| `HX_CAB_DISCRETE_C2` | ON | Force `c2` on legacy discretes (`force_discrete_c2_marker`) |
| `models_wire_local_param_selector` (localStorage) | ON | Wire-local `pSel` selector (discrete/float 0-based separately) |
| `HX_DUAL_LEGACY_CAB2_23_TEMPLATE` | ON | Dual legacy cab2 change via capture-grounded `23` 44 B template |
| `HX_DUAL_LEGACY_STD_PARAM` | ON | Dual legacy params routed via dual modern builder |

---

## 7. Key principles (cross-cutting)

- **Wire-type-local, 0-based `pSel` selectors**: the device numbers discretes and floats
  separately. A flat global index causes a silent offset, with no device error.
- **`c2` = legacy discrete · `c3` = float** (and modern discrete via static replay). The
  value-type marker is the byte **right after `1d`**, before `1a`.
- **`cd 04` + cab index after `1a`** for dual; **`cd 03`** for single. (Dual legacy `@mic` also
  works on `cd 03`.)
- **Assignment bulk `c2:19` = “single cab”**, *not* legacy. Legacy is recognized by the
  **1-byte hint** vs modern’s 3-byte `cd 03 xx` block.
- **`ed:08` mandatory** before each `23`/`27` in dual (otherwise write silently ignored).
- **`@mic` lesson**: a **shared** state flag between writes
  (`force_discrete_c2_for_legacy_single`) is fragile — a prior write consumes it. Prefer
  carrying context in the **route** (`discrete_wants_c2`).
- **Cab change = no HW read**: params come from the chosen cab’s `.models` defaults. Never
  drive resolution from the optimistic wire.
- **Capture-first method**: never code blind. Require `[LiveWrite][sent]` or `[SlotModelProbe]`
  to decide byte-for-byte.
