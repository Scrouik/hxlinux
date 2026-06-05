## 9. Decoupling around the ~38th detent: host-side parser bug (RESOLVED) — corrects §2 and §5

> **Status: RESOLVED.** The systematic drop-out around the 38th/39th detent was **neither** a
> `ctr` ceiling, **nor** a reject, **nor** a device freeze. The pedal sent a perfectly valid
> `IN 53`; our **model-id extractor** missed it. A one-function fix
> (`extract_first_module_hex_from_bulk`) is enough, verified on real bytes. Single-notch
> scroll is now **unlimited**.
>
> **French (source):** [addendum_section_decrochage_38.md](./addendum_section_decrochage_38.md)
>
> **Reference capture:** `stomp_running_start_linux_multi_one_notch.json` (single-notch run
> that “dropped out” on the 39th pull). **Code:**
> `src-tauri/src/helix/scroll_model_pull.rs`, function `extract_first_module_hex_from_bulk`.
> **Test:** `echo_double_0x19_does_not_mask_model_id`.

---

### 9.1. The decisive fact: the pedal dumps 39/39

On the reference run, the device returns an exploitable dump on **every** one of the 39
pulls, including the one we called “failed”. On the failing pull (`ctr=0x77df`), frame
**#3875**, the device sends a well-formed 92-byte `IN 53` containing model-id `cd0209`.
Right after, keep-alives resume normally on all lanes. **The hardware neither froze nor
rejected.** The “drop-out” is entirely on the host side.

Replayed on **all 39 dump frames** in the capture: the old parser fails on **only one**
(#3875); the fixed parser reads all 39, including `cd0209`.

### 9.2. Mechanism: collision with the double echo

Right after the assignment marker `83 66 cd <cd_lane>`, the device places the **echo of our
double** as `<double_lo> 67 00 68 …` (structure also confirmed on the HX Edit capture). The
old `extract_first_module_hex_from_bulk` searched for “the first `0x19`” in the **entire**
frame:

```
[24]  83 66 cd 04        ← marker
[28]  19 67 00 68 …       ← double echo: double_lo == 0x19 !
[45]  19 cd 02 09 1a      ← the REAL model-id (cd0209)
```

When `double_lo == 0x19`, byte [28] *is* a `0x19`. The parser treated it as a model-id
marker, searched for the following `1a` **anywhere in the buffer**, landed on the `1a` of the
real model-id ~20 bytes later ([49]), judged the “id” too long (>12 bytes) and gave up — but
the cursor had **already skipped past** the real marker at [45]. Result: `None` → `finalize`
via timeout → “pull failed (no assignable bulk)”, while the dump was good.

### 9.3. Why always around the 38th/39th detent (deterministic)

The double is seeded at `0xf2` (`editor_ed03_double` after PHASE B), first OUT at `0xf3`,
then **+1 per detent** (one `1b` per detent in grab-53). It reaches `0x19` after exactly
**38 increments** → the **39th pull** triggers it. Session seed aside, drop-out always lands
in the same place (hence the “~35–38” feel).

### 9.4. CORRECTION to §2 and §5 — there is no “0x7794 ceiling”

§5 quoted the reference run as “`ctr` 0x6cbd→**0x7794**” and §2 noted “no page ceiling up
to 0x77”. Stopping at `0x7794` was **not** a device limit: it is simply the **last detent
before `double_lo` reaches `0x19`**.

| Detent | `ctr` | `double_lo` | Device dump | Old parser |
|--------|-------|-------------|-------------|------------|
| 38th (last “OK”) | `0x7794` | `0x18` | `cd0122` (Bitcrusher) | read |
| 39th (“drop-out”) | `0x77df` | `0x19` | `cd0209` | **miss → None** |

➜ The reference run in §5 stopped **exactly for this reason**, not for a lane ceiling. The
“monotonic continuation / `ctr` window” framing in §2 still holds for what *makes* the device
dump, but it **does not govern** the terminal drop-out — that was purely host-side.

### 9.5. The fix

In `extract_first_module_hex_from_bulk`, two guardrails:

1. search for `1a` is **bounded** to `MODEL_ID_MAX_LEN = 8` bytes (a real model-id is
   ≤ ~3 bytes: `cd0209`, `cd0122`, `64`…);
2. if no `1a` is found in the window, this `0x19` is a collision → advance **only by 1** (no
   longer skip over a marker further ahead).

The fix lives in the parser, so it also covers **future recurrences**: the double will pass
through `0x19` every 256 detents without re-triggering the bug. No counter or protocol field
is touched. Regression test `echo_double_0x19_does_not_mask_model_id` added with real bytes
from #3875.

### 9.6. Reference figures (revised)

| Run | Pulls | Device dumps | Read (old) | Read (fixed) | Freeze |
|-----|-------|--------------|------------|--------------|--------|
| 1-notch (`multi_one_notch`) | 39 | **39 (100%)** | 38 | **39** | 0 |

The ~8% of **real** device rejects (`IN 21` with no following dump, §4) remain a distinct,
real phenomenon; this run had **none**. As it stands, grab-53 single-notch scroll is much
more solid than the addendum suggested: the only flaw was this read bug.

---

*Summary: the “38th detent wall” did not exist on the hardware. The pedal dumped 39/39; a
double-echo `0x19` masqueraded as a model-id marker and hid the real one. One fixed function,
one test on real bytes, and single-notch scroll becomes unlimited. Lesson: before theorizing
a firmware ceiling, verify the expected byte is actually read — the wall was in our parser,
not in ED03.*
