#!/usr/bin/env python3
"""
Synchronise / génère `HX_ModelUsbAssign.json` depuis `HX_ModelCatalog.json`.

Modes :
  --fill-all   Régénère toutes les entrées catalogue (ordre HX_ModelCatalog.json).
               Préserve bulkHex / notes / bulkKind / edOpcode des captures existantes.
               Les entrées sans capture reçoivent bulkHex: "" (assign USB inactif jusqu'à capture).
  (défaut)     Met à jour uniquement les entrées déjà présentes (basedOn, image, chainHexHint…).

Usage :
  python3 scripts/sync_usb_assign_from_catalog.py --fill-all
  python3 scripts/sync_usb_assign_from_catalog.py
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSIGN_PATH = ROOT / "src-tauri/resources/HX_ModelUsbAssign.json"
CATALOG_PATH = ROOT / "src-tauri/resources/HX_ModelCatalog.json"
AUDIT_PATH = ROOT / "src-tauri/resources/HX_ModelUsbAssign_chainhex-audit.txt"

VARIANT_DEFAULT_SUB = {
    "mono": "Mono",
    "stereo": "Stereo",
    "legacy": "Legacy",
    "single": "Single",
    "dual": "Dual",
    "amp": "Guitar",
    "amp+cab": "Guitar",
    "preamp": "Guitar",
}
AMP_VARIANT = "amp"
PREAMP_VARIANT = "preamp"
AMP_CAB_VARIANT = "amp+cab"
AMP_CAB_LEGACY_VARIANT = "amp+cab-legacy"
AMP_CAB_CATEGORY = "Amp+Cab"
AMP_CAB_LEGACY_CATEGORY = "Amp+Cab Legacy"
AMP_CATEGORY = "Amp"
PREAMP_CATEGORY = "Preamp"
SEND_RETURN_CATEGORY = "Send/Return"
SEND_RETURN_VARIANT = "sendReturn"

SUB_TO_VARIANT: dict[str, str] = {
    "mono": "mono",
    "stereo": "stereo",
    "stéréo": "stereo",
    "legacy": "legacy",
    "single": "single",
    "dual": "dual",
    "guitar": "mono",
    "bass": "mono",
    "mic": "mono",
}

PRESERVE_KEYS = ("bulkHex", "notes", "bulkKind", "edOpcode")

STUB_NOTE = "bulkHex à capturer — généré depuis HX_ModelCatalog.json"

# Bases `.models` (sans extension) par catégorie picker — source de vérité, recopiée en tête du JSON assign.
MODELS_FILE_BY_CATEGORY: dict[str, str | list[str]] = {
    "Amp": "amp",
    "Preamp": "preamp",
    "Amp+Cab": ["amp", "cab", "cabmicirs", "cabmicirswithpan", "preamp"],
    "Amp+Cab Legacy": ["amp", "cab", "cabmicirs", "cabmicirswithpan", "preamp"],
    "Cab": ["cab", "cabmicirs", "cabmicirswithpan"],
    "IR": ["fixed", "cabmicirs", "cabmicirswithpan"],
    "Looper": "fixed",
    "Delay": "delay",
    "Reverb": "reverb",
    "Dynamics": ["compressor", "gate"],
    "EQ": "eq",
    "Modulation": "modulation",
    "Distortion": "distortion",
    "Filter": "filter",
    "Wah": "wah",
    "Pitch/Synth": "pitch-synth",
    "Volume/Pan": "volumepan",
    "Send/Return": "sendreturn",
    "Input": "io",
    "Output": "io",
    "Split": "io",
    "Merge": "io",
    "Connected Devices": "io",
}

# Exceptions id → fichier(s) ; prioritaire sur modelsFileByCategory.
MODELS_FILE_BY_ID: dict[str, str | list[str]] = {}

# Catégories présentes dans entries[] (scroll / métadonnées) mais absentes du picker assign slot FX.
# Routing / périphériques : jamais dans le picker FX. Input/Output : picker verrouillé sur le slot I/O Path 1.
PICKER_EXCLUDED_CATEGORIES: list[str] = [
    "Split",
    "Merge",
    "Connected Devices",
]


def ensure_models_file_maps(assign: dict) -> None:
    """Met à jour modelsFileByCategory / modelsFileById sans écraser les ids ajoutés à la main."""
    assign["modelsFileByCategory"] = dict(MODELS_FILE_BY_CATEGORY)
    by_id = dict(MODELS_FILE_BY_ID)
    existing = assign.get("modelsFileById")
    if isinstance(existing, dict):
        for k, v in existing.items():
            if k not in by_id:
                by_id[k] = v
    assign["modelsFileById"] = by_id
    fg = assign.setdefault("fieldGuide", {})
    if isinstance(fg, dict):
        fg.setdefault(
            "modelsFileByCategory",
            {
                "runtime": "TypeScript (panneau paramètres)",
                "purpose": "Catégorie picker → base(s) `.models` (sans extension), string ou tableau.",
            },
        )
        fg.setdefault(
            "modelsFileById",
            {
                "runtime": "TypeScript (panneau paramètres)",
                "purpose": "Exceptions par id catalogue (prioritaire sur modelsFileByCategory).",
            },
        )


def ensure_picker_excluded_categories(assign: dict) -> None:
    assign["pickerExcludedCategories"] = list(PICKER_EXCLUDED_CATEGORIES)
    fg = assign.setdefault("fieldGuide", {})
    if isinstance(fg, dict):
        fg.setdefault(
            "pickerExcludedCategories",
            {
                "runtime": "TypeScript (picker assign)",
                "purpose": "Catégories hors picker FX (split, merge, …). Input/Output restent dans le picker mais verrouillés sur les slots Path 1 dédiés.",
            },
        )


def load_catalog_models() -> list[dict]:
    data = json.loads(CATALOG_PATH.read_text(encoding="utf-8"))
    models = data.get("models")
    if not isinstance(models, list):
        return []
    return [m for m in models if isinstance(m, dict)]


def normalize_list_field(val: object) -> list[str]:
    if isinstance(val, list):
        return [str(x).strip() for x in val if str(x).strip()]
    if isinstance(val, str) and val.strip():
        return [val.strip()]
    return []


def sub_to_variant(sub: str) -> str:
    return SUB_TO_VARIANT.get(sub.strip().lower(), "mono")


def catalog_variant_for_sub(pm: dict, sub: str) -> str:
    """Amp / Preamp / Send-Return : variants dédiés (pas mono/stéréo USB)."""
    cat = (pm.get("categoryName") or "").strip()
    if cat == AMP_CATEGORY:
        return AMP_VARIANT
    if cat == PREAMP_CATEGORY:
        return PREAMP_VARIANT
    if cat == SEND_RETURN_CATEGORY:
        return SEND_RETURN_VARIANT
    return sub_to_variant(sub)


def expand_catalog_variants(pm: dict) -> list[tuple[str, str, str]]:
    """(variant, chainHexHint, subCategory display) pour chaque entrée assign."""
    chs = normalize_list_field(pm.get("chainHex"))
    subs = normalize_list_field(pm.get("subCategory"))

    if len(chs) > 1 and len(subs) == len(chs):
        return [
            (catalog_variant_for_sub(pm, subs[i]), chs[i].lower(), subs[i])
            for i in range(len(chs))
        ]

    if len(chs) == 1:
        sub = subs[0] if subs else "Mono"
        hint = chs[0].lower()
        # Cab Legacy : même chainHex, deux modes matériel (1 voie vs L/R) → deux bulks à capturer.
        if (
            (pm.get("categoryName") or "").strip() == "Cab"
            and sub.lower() == "legacy"
            and hint
        ):
            return [
                ("single", hint, "Legacy"),
                ("dual", hint, "Legacy"),
            ]
        return [(catalog_variant_for_sub(pm, sub), hint, sub)]

    if len(chs) > 1:
        cat = (pm.get("categoryName") or "").strip()
        # Cab IR : parfois chainHex [single, dual] sur une ligne catalogue alors que le dual
        # a son propre id (*WithPan) — ne pas fabriquer une 2ᵉ entrée `mono`.
        if (
            cat == "Cab"
            and len(subs) == 1
            and subs[0].strip().lower() in ("single", "legacy")
        ):
            sub = subs[0]
            return [(catalog_variant_for_sub(pm, sub), chs[0].lower(), sub)]
        out: list[tuple[str, str, str]] = []
        for i, ch in enumerate(chs):
            sub = subs[i] if i < len(subs) else "Mono"
            out.append((catalog_variant_for_sub(pm, sub), ch.lower(), sub))
        return out

    sub = subs[0] if subs else "Mono"
    return [(catalog_variant_for_sub(pm, sub), "", sub)]


def pick_parallel(pm: dict, variant: str, field: str) -> str:
    vals = normalize_list_field(pm.get(field))
    if not vals:
        return ""
    v = variant.strip().lower()
    if v in ("mono", "amp", "preamp", "amp+cab") and vals:
        first = vals[0]
        return first.lower() if field == "chainHex" else first
    if v == "stereo" and len(vals) >= 2:
        return vals[1]
    if v == "legacy":
        if field == "subCategory":
            for s in vals:
                if s.lower() == "legacy":
                    return s
        subs = normalize_list_field(pm.get("subCategory"))
        if field == "chainHex" and subs:
            for i, s in enumerate(subs):
                if s.lower() == "legacy" and i < len(vals):
                    return vals[i].lower()
        return vals[-1].lower() if field == "chainHex" else vals[-1]
    if v == "single":
        for i, s in enumerate(normalize_list_field(pm.get("subCategory"))):
            if s.lower() == "single" and i < len(vals):
                return vals[i].lower() if field == "chainHex" else vals[i]
    if v == "dual":
        for i, s in enumerate(normalize_list_field(pm.get("subCategory"))):
            if s.lower() == "dual" and i < len(vals):
                return vals[i].lower() if field == "chainHex" else vals[i]
    first = vals[0]
    return first.lower() if field == "chainHex" else first


def build_entry_from_catalog(
    catalog_row: dict,
    variant: str,
    chain_hint: str,
    sub_display: str,
) -> dict:
    pm = catalog_row.get("presetMeta")
    if not isinstance(pm, dict):
        pm = {}
    vid = variant.strip().lower()
    sub = sub_display.strip() or VARIANT_DEFAULT_SUB.get(vid, vid.title())
    category = (pm.get("categoryName") or "Unknown").strip() or "Unknown"
    name = (catalog_row.get("name") or catalog_row.get("id") or "").strip()
    based_on = (pm.get("basedOn") or "").strip()
    image = (catalog_row.get("image") or "").strip()

    entry: dict = {
        "id": catalog_row["id"],
        "variant": vid,
        "bulkHex": "",
        "name": name,
        "category": category,
        "subCategory": sub,
    }
    if chain_hint:
        entry["chainHexHint"] = chain_hint
    if based_on:
        entry["basedOn"] = based_on
    if image:
        entry["image"] = image
    entry["notes"] = STUB_NOTE
    return entry


def sync_entry_fields(entry: dict, catalog_row: dict) -> dict[str, object]:
    pm = catalog_row.get("presetMeta")
    if not isinstance(pm, dict):
        pm = {}
    variant = (entry.get("variant") or "mono").strip().lower()
    updates: dict[str, object] = {}

    if variant in (AMP_CAB_VARIANT, AMP_CAB_LEGACY_VARIANT):
        if entry.get("chainHexHint"):
            updates["chainHexHint"] = ""
    else:
        hint = pick_parallel(pm, variant, "chainHex")
        if hint and (entry.get("chainHexHint") or "").strip().lower() != hint:
            updates["chainHexHint"] = hint
        elif not hint and entry.get("chainHexHint"):
            updates["chainHexHint"] = ""

    name = (catalog_row.get("name") or "").strip()
    if name and entry.get("name") != name:
        updates["name"] = name

    if variant == AMP_CAB_LEGACY_VARIANT:
        if (entry.get("category") or "").strip() != AMP_CAB_LEGACY_CATEGORY:
            updates["category"] = AMP_CAB_LEGACY_CATEGORY
    elif variant == AMP_CAB_VARIANT:
        if (entry.get("category") or "").strip() != AMP_CAB_CATEGORY:
            updates["category"] = AMP_CAB_CATEGORY
    elif (entry.get("category") or "").strip() not in (
        AMP_CAB_CATEGORY,
        AMP_CAB_LEGACY_CATEGORY,
    ):
        category = (pm.get("categoryName") or "").strip()
        if category and entry.get("category") != category:
            updates["category"] = category

    if variant in (AMP_CAB_VARIANT, AMP_CAB_LEGACY_VARIANT):
        sc = (entry.get("subCategory") or "").strip().lower()
        want = "Bass" if sc == "bass" or "bass" in sc else "Guitar"
        if (entry.get("subCategory") or "").strip() != want:
            updates["subCategory"] = want
    else:
        sub = pick_parallel(pm, variant, "subCategory")
        if not sub:
            sub = VARIANT_DEFAULT_SUB.get(variant, variant.title())
        if sub and entry.get("subCategory") != sub:
            updates["subCategory"] = sub

    based_on = (pm.get("basedOn") or "").strip()
    if based_on and entry.get("basedOn") != based_on:
        updates["basedOn"] = based_on

    image = (catalog_row.get("image") or "").strip()
    if image and entry.get("image") != image:
        updates["image"] = image

    return updates


def existing_index(entries: list[dict]) -> dict[tuple[str, str], dict]:
    out: dict[tuple[str, str], dict] = {}
    for e in entries:
        if not isinstance(e, dict):
            continue
        mid = (e.get("id") or "").strip()
        if not mid:
            continue
        var = (e.get("variant") or "mono").strip().lower()
        out[(mid, var)] = e
    return out


def lookup_prev_entry(
    prev_by_key: dict[tuple[str, str], dict],
    mid: str,
    variant: str,
    category: str,
) -> dict | None:
    hit = prev_by_key.get((mid, variant))
    if hit:
        return hit
    # Migration : anciennes entrées Amp / Preamp utilisaient variant `mono`.
    if category == AMP_CATEGORY and variant == AMP_VARIANT:
        return prev_by_key.get((mid, "mono"))
    if category == PREAMP_CATEGORY and variant == PREAMP_VARIANT:
        return prev_by_key.get((mid, "mono"))
    return None


def amp_cab_subcategory_variant_pairs(amp_sub: str) -> list[tuple[str, str, str]]:
    """
    Rubriques picker Amp+Cab (Helix FW 3.50+) :
    Guitar / Bass sous `Amp+Cab` (IR) ou `Amp+Cab Legacy` (hybrid) — même libellé subCategory,
    la catégorie picker distingue moderne vs legacy ; `variant` reste la clé bulk Rust.
    """
    s = (amp_sub or "Guitar").strip().lower()
    if s == "bass":
        return [
            ("Bass", AMP_CAB_VARIANT, AMP_CAB_CATEGORY),
            ("Bass", AMP_CAB_LEGACY_VARIANT, AMP_CAB_LEGACY_CATEGORY),
        ]
    return [
        ("Guitar", AMP_CAB_VARIANT, AMP_CAB_CATEGORY),
        ("Guitar", AMP_CAB_LEGACY_VARIANT, AMP_CAB_LEGACY_CATEGORY),
    ]


def clone_amp_cab_block_entries(
    amp_entry: dict, prev_by_key: dict[tuple[str, str], dict]
) -> list[dict]:
    """Jumelles Amp → Amp+Cab moderne + Amp+Cab legacy (même id, bulks distincts)."""
    mid = (amp_entry.get("id") or "").strip()
    out: list[dict] = []
    for sub_display, variant, category in amp_cab_subcategory_variant_pairs(
        str(amp_entry.get("subCategory") or "Guitar")
    ):
        clone: dict = {
            "id": mid,
            "variant": variant,
            "category": category,
            "bulkHex": "",
            "name": (amp_entry.get("name") or mid).strip(),
            "subCategory": sub_display,
            "notes": STUB_NOTE,
        }
        # chainHexHint : seule l’entrée `amp` le porte (fil USB = bloc ampli).
        # amp+cab / legacy : variante picker + bulk — résolution via categoryHint + cab.
        for key in ("basedOn", "image"):
            val = amp_entry.get(key)
            if val:
                clone[key] = val
        out.append(merge_preserved(clone, prev_by_key.get((mid, variant))))
    return out


def merge_preserved(stub: dict, prev: dict | None) -> dict:
    if not prev:
        return stub
    merged = dict(stub)
    for key in PRESERVE_KEYS:
        val = prev.get(key)
        if val is not None and val != "":
            merged[key] = val
    # Garde un bulkHex existant même si stub vide
    prev_bulk = (prev.get("bulkHex") or "").strip()
    if prev_bulk:
        merged["bulkHex"] = prev_bulk
    return merged


def fill_all_from_catalog(assign: dict) -> list[dict]:
    prev_by_key = existing_index(assign.get("entries") or [])
    new_entries: list[dict] = []

    for row in load_catalog_models():
        mid = (row.get("id") or "").strip()
        if not mid or mid == "None":
            continue
        pm = row.get("presetMeta") or {}
        for variant, chain_hint, sub_display in expand_catalog_variants(pm):
            stub = build_entry_from_catalog(row, variant, chain_hint, sub_display)
            cat_name = (pm.get("categoryName") or "").strip()
            merged = merge_preserved(
                stub, lookup_prev_entry(prev_by_key, mid, variant, cat_name)
            )
            new_entries.append(merged)
            if cat_name == AMP_CATEGORY and variant == AMP_VARIANT:
                new_entries.extend(clone_amp_cab_block_entries(merged, prev_by_key))

    return new_entries


def write_audit(entries: list[dict]) -> None:
    missing_hint = sum(1 for e in entries if not (e.get("chainHexHint") or "").strip())
    with_bulk = sum(1 for e in entries if (e.get("bulkHex") or "").strip())
    lines = [
        "# HX_ModelUsbAssign — audit chainHexHint (généré depuis HX_ModelUsbAssign.json)",
        f"# {len(entries)} entrées — {missing_hint} sans chainHexHint — {with_bulk} avec bulkHex capturé",
        "# Format : une ligne par entrée (ordre picker = ordre du JSON)",
        "# Vérifier terrain : scroll Stomp + HX_SCROLL_CHAINHEX=1 → comparer le hex USB au chainHexHint",
        "#",
    ]
    for e in entries:
        hint = (e.get("chainHexHint") or "").strip()
        name = (e.get("name") or e.get("id") or "").strip()
        cat = (e.get("category") or "").strip()
        sub = (e.get("subCategory") or "").strip()
        bulk = "bulk" if (e.get("bulkHex") or "").strip() else "stub"
        lines.append(
            f'[{bulk}] "chainHexHint": "{hint}", "name": "{name}", '
            f'"category": "{cat}", "subCategory": "{sub}", "variant": "{e.get("variant", "")}"'
        )
    AUDIT_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--fill-all",
        action="store_true",
        help="Génère toutes les entrées depuis le catalogue (préserve les bulkHex existants).",
    )
    parser.add_argument(
        "--audit-only",
        action="store_true",
        help="Régénère seulement le fichier d'audit.",
    )
    args = parser.parse_args()

    assign = json.loads(ASSIGN_PATH.read_text(encoding="utf-8"))

    if args.audit_only:
        entries = assign.get("entries") or []
        write_audit(entries)
        print(f"Audit écrit : {AUDIT_PATH}")
        return 0

    if args.fill_all:
        entries = fill_all_from_catalog(assign)
        assign["entries"] = entries
        desc = assign.get("description")
        if isinstance(desc, str):
            assign["description"] = (
                desc.rstrip()
                + " Entrées sans bulkHex : placeholders en attente de capture USB."
            )
    else:
        entries = assign.get("entries")
        if not isinstance(entries, list):
            print("HX_ModelUsbAssign.json : tableau entries manquant.", file=sys.stderr)
            return 1
        by_id = {m["id"]: m for m in load_catalog_models() if m.get("id")}
        changed = 0
        missing_catalog: list[str] = []
        for entry in entries:
            if not isinstance(entry, dict):
                continue
            mid = (entry.get("id") or "").strip()
            if not mid:
                continue
            row = by_id.get(mid)
            if not row:
                missing_catalog.append(mid)
                continue
            updates = sync_entry_fields(entry, row)
            if updates:
                entry.update(updates)
                changed += 1
        print(f"Entrées mises à jour : {changed} / {len(entries)}")
        if missing_catalog:
            print(f"Ids assign absents du catalogue : {len(missing_catalog)}", file=sys.stderr)

    ensure_models_file_maps(assign)
    ensure_picker_excluded_categories(assign)
    ordered: dict = {}
    for key in (
        "schemaVersion",
        "description",
        "modelsFileByCategory",
        "modelsFileById",
        "pickerExcludedCategories",
        "ioSources",
        "splitSources",
        "fieldGuide",
        "entries",
    ):
        if key in assign:
            ordered[key] = assign[key]
    for key, val in assign.items():
        if key not in ordered:
            ordered[key] = val
    ASSIGN_PATH.write_text(
        json.dumps(ordered, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    write_audit(assign["entries"])

    with_bulk = sum(1 for e in assign["entries"] if (e.get("bulkHex") or "").strip())
    print(f"Total entrées : {len(assign['entries'])} ({with_bulk} avec bulkHex)")
    print(f"Écrit : {ASSIGN_PATH}")
    print(f"Audit : {AUDIT_PATH}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
