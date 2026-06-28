#!/usr/bin/env python3
"""
Inject bulkHex from Wireshark JSON captures into HX_ModelUsbAssign.json.

Matches entries by category + variant + chainHexHint.

Example:
  python3 scripts/inject_bulk_from_captures.py EQ \\
    mono:captures/usb-wireshark/eq\\ Mono.json \\
    stereo:captures/usb-wireshark/eq\\ stereo.json

  python3 scripts/inject_bulk_from_captures.py Modulation \\
    mono:captures/usb-wireshark/modulation\\ mono.json \\
    stereo:captures/usb-wireshark/modulation\\ stereo.json \\
    legacy:captures/usb-wireshark/modulation\\ legacy.json

  python3 scripts/inject_bulk_from_captures.py Cab --allow-partial \\
    legacy-single:captures/usb-wireshark/cab\\ single\\ legacy.json \\
    legacy-dual:captures/usb-wireshark/cab\\ dual\\ legacy.json
"""
from __future__ import annotations

import argparse
import json
import re
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSIGN_PATH = ROOT / "src-tauri/resources/HX_ModelUsbAssign.json"


# OUT assign HX Edit : 23/24/25/27, opcode 80:10:ed:03, corps 83:66:cd:03…07.
ASSIGN_OUT_PREFIXES = (
    "230000188010ed03",
    "240000188010ed03",
    "250000188010ed03",
    "270000188010ed03",  # Amp+Cab IR (48 o)
)

# Clé capture → (variant assign, subCategory) par catégorie.
CAPTURE_VARIANT_ALIASES_BY_CATEGORY: dict[str, dict[str, tuple[str, str | None]]] = {
    "Amp": {"guitar": ("amp", "Guitar"), "bass": ("amp", "Bass")},
    "Amp+Cab": {"guitar": ("amp+cab", "Guitar"), "bass": ("amp+cab", "Bass")},
    "Amp+Cab Legacy": {
        "guitar": ("amp+cab-legacy", "Guitar"),
        "bass": ("amp+cab-legacy", "Bass"),
    },
    "Preamp": {
        "guitar": ("preamp", "Guitar"),
        "bass": ("preamp", "Bass"),
        "mic": ("preamp", "Mic"),
    },
    "Cab": {
        "single": ("single", "Single"),
        "dual": ("dual", "Dual"),
        "legacy-single": ("single", "Legacy"),
        "legacy-dual": ("dual", "Legacy"),
    },
    "Send/Return": {
        "mono": ("sendReturn", "Mono"),
        "stereo": ("sendReturn", "Stereo"),
    },
}


def chain_hex_hint_from_c219_module(hex_tail: str) -> str:
    """
    Module après marqueur c219 dans le bulk assign.
    - Delays « courts » : 501aff… → chainHexHint `50` (2 nibbles avant `1a`).
    - Module long : cd011d1aff… → `cd011d`.
    """
    t = hex_tail.lower()
    if t.startswith("cd"):
        i = t.find("1aff")
        return t[:i] if i > 0 else t[:6]
    if len(t) >= 4 and t[2:4] == "1a":
        return t[:2]
    i = t.find("1a")
    return t[:i] if i > 0 else t


def extract_assign_bulks(path: Path) -> dict[str, tuple[str, int]]:
    """chainHexHint (lowercase) -> (bulkHex, wireshark frame number)."""
    data = json.loads(path.read_text())
    out: dict[str, tuple[str, int]] = {}
    for pkt in data:
        layers = pkt.get("_source", {}).get("layers", {})
        cap = layers.get("usb.capdata")
        if not cap or layers.get("usb", {}).get("usb.src") != "host":
            continue
        hexs = cap.replace(":", "").lower()
        if not hexs.startswith(ASSIGN_OUT_PREFIXES):
            continue
        if "8366cd" not in hexs:
            continue
        frame = int(layers["frame"]["frame.number"])
        m = re.search(r"c219([0-9a-f]+)", hexs)
        if not m:
            continue
        hint = chain_hex_hint_from_c219_module(m.group(1))
        if not hint:
            continue
        if hint in out:
            print(f"WARN: duplicate hint {hint} in {path.name} (frame {frame}, keeping first)")
            continue
        out[hint] = (hexs, frame)
    return out


def amp_hint_from_c319_module(hex_tail: str) -> str:
    """
    Partie ampli dans un bulk Amp+Cab (`8317c319` + `<amp> 1a <cab> …`).
    Un seul bulk par entrée — amp et cab dans le même paquet.
    - IR : cab en `cd:03:xx` → séparateur `1a` suivi de `cd`.
    - Legacy hybrid : cab courte `47:00` → `1a` suivi de 2 nibbles puis `00`.
    """
    t = hex_tail.lower()
    pos = 0
    while pos < len(t):
        idx = t.find("1a", pos)
        if idx < 0:
            break
        after = t[idx + 2 :]
        if after.startswith("cd"):
            return t[:idx]
        if (
            len(after) >= 4
            and after[2:4] == "00"
            and re.match(r"^[0-9a-f]{2}", after)
        ):
            return t[:idx]
        pos = idx + 2
    if len(t) >= 4 and t[2:4] == "1a":
        return t[:2]
    return ""


def cab_dual_hint_from_c319_module(hex_tail: str) -> str:
    """Cab IR dual : `<hint> 1a cd02d6` — voie droite défaut (Jazz Rivet) après c319."""
    t = hex_tail.lower()
    suffix = "1acd02d6"
    if suffix in t:
        return t[: t.index(suffix)]
    return amp_hint_from_c319_module(t)


def cab_dual_legacy_hint_from_c319_module(hex_tail: str) -> str:
    """Cab Legacy dual : `<hint> 1a 30:00` — voie droite défaut 1x12 Lead 80 (`30`)."""
    t = hex_tail.lower()
    for suffix in ("1a30000000", "1a3000"):
        if suffix in t:
            return t[: t.index(suffix)]
    return amp_hint_from_c319_module(t)


def extract_cab_dual_bulks(path: Path, *, legacy: bool = False) -> dict[str, tuple[str, int]]:
    """chainHexHint dual → (bulkHex c319 complet, frame)."""
    hint_fn = cab_dual_legacy_hint_from_c319_module if legacy else cab_dual_hint_from_c319_module
    data = json.loads(path.read_text())
    out: dict[str, tuple[str, int]] = {}
    for pkt in data:
        layers = pkt.get("_source", {}).get("layers", {})
        cap = layers.get("usb.capdata")
        if not cap or layers.get("usb", {}).get("usb.src") != "host":
            continue
        hexs = cap.replace(":", "").lower()
        if not hexs.startswith(ASSIGN_OUT_PREFIXES):
            continue
        if "8366cd" not in hexs or "8317c319" not in hexs:
            continue
        frame = int(layers["frame"]["frame.number"])
        m = re.search(r"c319([0-9a-f]+)", hexs)
        if not m:
            continue
        hint = hint_fn(m.group(1))
        if not hint:
            continue
        if hint in out:
            print(f"WARN: duplicate cab dual hint {hint} in {path.name} (frame {frame}, keeping first)")
            continue
        out[hint] = (hexs, frame)
    return out


def extract_amp_cab_bulks(path: Path) -> dict[str, tuple[str, int]]:
    """Hint ampli (comme variante `amp`) → (bulkHex Amp+Cab complet, frame)."""
    data = json.loads(path.read_text())
    out: dict[str, tuple[str, int]] = {}
    for pkt in data:
        layers = pkt.get("_source", {}).get("layers", {})
        cap = layers.get("usb.capdata")
        if not cap or layers.get("usb", {}).get("usb.src") != "host":
            continue
        hexs = cap.replace(":", "").lower()
        if not hexs.startswith(ASSIGN_OUT_PREFIXES):
            continue
        if "8366cd" not in hexs or "8317c319" not in hexs:
            continue
        frame = int(layers["frame"]["frame.number"])
        m = re.search(r"c319([0-9a-f]+)", hexs)
        if not m:
            continue
        hint = amp_hint_from_c319_module(m.group(1))
        if not hint:
            continue
        if hint in out:
            print(f"WARN: duplicate amp hint {hint} in {path.name} (frame {frame}, keeping first)")
            continue
        out[hint] = (hexs, frame)
    return out


def resolve_chain_hex_hint(entry: dict, entries: list[dict]) -> str:
    hint = (entry.get("chainHexHint") or "").strip().lower()
    if hint:
        return hint
    model_id = entry.get("id")
    variant = (entry.get("variant") or "").strip().lower()
    if variant in ("amp+cab", "amp+cab-legacy") and model_id:
        for e in entries:
            if e.get("id") == model_id and (e.get("variant") or "").strip().lower() == "amp":
                return (e.get("chainHexHint") or "").strip().lower()
    return ""


def bulk_meta(bulk_hex: str) -> tuple[str, str]:
    n = len(bulk_hex) // 2
    ed = bulk_hex[8:16] if len(bulk_hex) >= 16 else ""
    if "8366cd0a" in bulk_hex:
        cd_tag = "cd0a"
    elif "8366cd09" in bulk_hex:
        cd_tag = "cd09"
    elif "8366cd08" in bulk_hex:
        cd_tag = "cd08"
    elif "8366cd07" in bulk_hex:
        cd_tag = "cd07"
    elif "8366cd06" in bulk_hex:
        cd_tag = "cd06"
    elif "8366cd05" in bulk_hex:
        cd_tag = "cd05"
    elif "8366cd04" in bulk_hex:
        cd_tag = "cd04"
    else:
        cd_tag = "cd03"
    kind = f"assign{n}_{cd_tag}_{ed}" if ed else f"assign{n}"
    return kind, ed


def entry_matches_capture_variant(
    entry: dict, capture_key: str, category: str
) -> bool:
    """True if assign row should receive bulks from this capture file key."""
    variant = (entry.get("variant") or "").strip().lower()
    aliases = CAPTURE_VARIANT_ALIASES_BY_CATEGORY.get(category, {})
    if capture_key in aliases:
        assign_variant, sub = aliases[capture_key]
        if (entry.get("category") or "").strip() != category:
            return False
        if variant != assign_variant:
            return False
        if sub is not None and (entry.get("subCategory") or "").strip() != sub:
            return False
        return True
    return variant == capture_key


def capture_key_for_entry(entry: dict, capture_keys: set[str], category: str) -> str | None:
    for key in capture_keys:
        if entry_matches_capture_variant(entry, key, category):
            return key
    return None


def parse_capture_arg(spec: str) -> tuple[str, Path]:
    if ":" not in spec:
        raise argparse.ArgumentTypeError(f"expected variant:path, got {spec!r}")
    variant, rel = spec.split(":", 1)
    variant = variant.strip().lower()
    path = Path(rel)
    if not path.is_absolute():
        path = ROOT / path
    return variant, path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("category", help="HX_ModelUsbAssign category (ex. EQ, Modulation)")
    parser.add_argument(
        "captures",
        nargs="+",
        type=parse_capture_arg,
        metavar="variant:path",
        help="variant and Wireshark JSON path (ex. mono:captures/.../eq Mono.json)",
    )
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--allow-partial",
        action="store_true",
        help="Write matched entries even if some assign rows lack a capture bulk",
    )
    args = parser.parse_args()
    category = args.category.strip()

    bulks_by_variant: dict[str, dict[str, tuple[str, int]]] = {}
    capture_names: dict[str, str] = {}
    for variant, cap in args.captures:
        if not cap.is_file():
            raise SystemExit(f"missing capture: {cap}")
        if category in ("Amp+Cab", "Amp+Cab Legacy"):
            bulks = extract_amp_cab_bulks(cap)
        elif category == "Cab" and variant == "dual":
            bulks = extract_cab_dual_bulks(cap)
        elif category == "Cab" and variant == "legacy-dual":
            bulks = extract_cab_dual_bulks(cap, legacy=True)
        elif category == "Cab":
            bulks = extract_assign_bulks(cap)
        else:
            bulks = extract_assign_bulks(cap)
        bulks_by_variant[variant] = bulks
        capture_names[variant] = cap.name
        print(f"{variant}: {len(bulks_by_variant[variant])} bulk(s) in {cap.name}")

    assign = json.loads(ASSIGN_PATH.read_text())
    all_entries = assign.get("entries", [])
    targets = [
        e
        for e in assign.get("entries", [])
        if (e.get("category") or "").strip() == category
    ]
    expected = len(targets)
    updated = 0
    missing: list[str] = []
    today = date.today().isoformat()

    capture_keys = set(bulks_by_variant)

    for entry in targets:
        variant = (entry.get("variant") or "").strip().lower()
        hint = resolve_chain_hex_hint(entry, all_entries)
        name = entry.get("name") or entry.get("id") or "?"
        sub = (entry.get("subCategory") or "").strip()
        cap_key = capture_key_for_entry(entry, capture_keys, category)
        if cap_key is None:
            missing.append(f"{name} ({variant}/{sub}): no capture for variant")
            continue
        if not hint:
            missing.append(f"{name} ({variant}/{sub}): empty chainHexHint")
            continue
        hit = bulks_by_variant[cap_key].get(hint)
        if not hit:
            missing.append(f"{name} ({variant}/{sub}): hint {hint} not in capture")
            continue
        bulk_hex, frame = hit
        cap_name = capture_names[cap_key]
        entry["bulkHex"] = bulk_hex
        kind, ed = bulk_meta(bulk_hex)
        entry["bulkKind"] = kind
        entry["edOpcode"] = ed
        if category in ("Amp+Cab", "Amp+Cab Legacy"):
            amp_hint_note = f"amp chainHexHint={hint}"
        elif category == "Cab" and (entry.get("variant") or "").strip().lower() == "dual":
            sub = (entry.get("subCategory") or "").strip()
            if sub == "Legacy":
                amp_hint_note = f"cab dual legacy chainHexHint={hint} (c319 + voie droite 30 Lead 80)"
            else:
                amp_hint_note = f"cab dual chainHexHint={hint} (c319 + voie droite cd02d6)"
        else:
            amp_hint_note = f"chainHexHint={hint}"
        entry["notes"] = (
            f"{category} {variant} bulkHex capture Wireshark {today} ({cap_name} frame {frame}); "
            f"{amp_hint_note}"
        )
        updated += 1
        print(
            f"OK {name} ({variant}/{sub}) hint={hint} frame={frame} len={len(bulk_hex) // 2}"
        )

    unused: list[str] = []
    for cap_key, bulks in bulks_by_variant.items():
        used = {
            resolve_chain_hex_hint(e, all_entries)
            for e in targets
            if capture_key_for_entry(e, {cap_key}, category) == cap_key
        }
        for hint in bulks:
            if hint not in used:
                unused.append(f"{cap_key}:{hint}")

    if missing:
        print("\nMISSING:")
        for m in missing:
            print(f"  {m}")
    if unused:
        print("\nUNUSED bulks in capture (not matched to assign):")
        for u in unused:
            print(f"  {u}")

    if updated != expected and not args.allow_partial:
        raise SystemExit(
            f"expected {expected} updates for {category}, got {updated} "
            f"(use --allow-partial to inject matches only)"
        )
    if updated != expected and args.allow_partial:
        print(f"\npartial: {updated}/{expected} {category} entries updated")

    if args.dry_run:
        print(f"\ndry-run: would write {updated} entries")
        return

    ASSIGN_PATH.write_text(json.dumps(assign, indent=2, ensure_ascii=False) + "\n")
    print(f"\nwrote {ASSIGN_PATH} ({updated} {category} entries)")


if __name__ == "__main__":
    main()
