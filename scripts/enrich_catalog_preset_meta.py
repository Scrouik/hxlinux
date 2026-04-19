#!/usr/bin/env python3
"""
Enrichit HX_ModelCatalog.json avec un objet `presetMeta` par fiche modèle complète
(champ `name` présent), en croisant modules_by_id.json.

Règles (heuristiques) :
- Une entrée modules [catégorie, nomLong] matche un modèle catalogue si :
  - la catégorie modules == le nom de la catégorie HX (ex. Amp, Preamp) ;
  - nomLong correspond au début du nom catalogue (égalité, ou préfixe + espace, ou + '(') ;
  - si la sous-catégorie HX est Guitar ou Bass, on exige la présence de " Guitar " ou " Bass "
    dans le nomLong (espaces autour, insensible à la casse), pour limiter les ambiguïtés.
- On retient la correspondance au nomLong le plus long (la plus spécifique).
- chainHex = clé hex dans modules_by_id.json.
- Découpage du suffixe après le nom catalogue : premier mot Guitar/Bass -> instrument ;
  texte avant la première '(' -> emulationName ; parenthèses : "channel" ou "chanel" -> channel ;
  mono / stereo / stéréo -> signal.

Les entrées catalogue sans correspondance reçoivent presetMeta avec des chaînes vides.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MODULES_PATH = ROOT / "src-tauri/resources/modules_by_id.json"
CATALOG_PATH = ROOT / "src-tauri/resources/HX_ModelCatalog.json"


def load_modules_entries() -> list[tuple[str, str, str]]:
    raw = json.loads(MODULES_PATH.read_text(encoding="utf-8"))
    out: list[tuple[str, str, str]] = []
    for hex_key, pair in raw.items():
        if not isinstance(pair, list) or len(pair) != 2:
            continue
        cat, name_str = str(pair[0]).strip(), str(pair[1]).strip()
        if cat and name_str:
            out.append((hex_key.strip().lower(), cat, name_str))
    return out


def name_long_matches_catalog_name(name_long: str, catalog_name: str) -> bool:
    nl = name_long.strip().lower()
    cn = catalog_name.strip().lower()
    if not nl or not cn:
        return False
    if nl == cn:
        return True
    if nl.startswith(cn + " ") or nl.startswith(cn + "("):
        return True
    return False


def subcategory_fits_name_long(sub_name: str | None, name_long: str) -> bool:
    if not sub_name:
        return True
    s = sub_name.strip()
    if s in ("Guitar", "Bass"):
        needle = f" {s} "
        return needle.lower() in f" {name_long} ".lower()
    return True


def best_modules_match(
    entries: list[tuple[str, str, str]],
    catalog_category: str,
    catalog_subcategory: str | None,
    model_name: str,
) -> tuple[str, str, str] | None:
    best: tuple[str, str, str] | None = None
    best_len = -1
    for hex_k, mod_cat, name_long in entries:
        if mod_cat.strip() != catalog_category.strip():
            continue
        if not name_long_matches_catalog_name(name_long, model_name):
            continue
        if not subcategory_fits_name_long(catalog_subcategory, name_long):
            continue
        if len(name_long) > best_len:
            best_len = len(name_long)
            best = (hex_k, mod_cat, name_long)
    return best


def parse_suffix(name_long: str, catalog_name: str) -> dict[str, str]:
    nl = name_long.strip()
    cn = catalog_name.strip()
    if nl.lower().startswith(cn.lower() + " "):
        rest = nl[len(cn) + 1 :].strip()
    elif nl.lower() == cn.lower():
        rest = ""
    elif nl.lower().startswith(cn.lower() + "("):
        rest = nl[len(cn) :].strip()
    else:
        rest = nl

    instrument = ""
    emulation_name = ""
    channel = ""
    signal = ""

    work = rest
    parts = work.split(None, 1)
    if parts and parts[0].lower() in ("guitar", "bass"):
        instrument = parts[0].capitalize()
        work = parts[1].strip() if len(parts) > 1 else ""

    if "(" in work:
        emulation_name = work[: work.index("(")].strip()
        for inner in re.findall(r"\(([^)]*)\)", work):
            low = inner.lower()
            if "channel" in low or "chanel" in low:
                channel = inner.strip()
            if low in ("mono", "stereo", "stéréo") or "mono" in low or "stereo" in low or "stéréo" in low:
                signal = inner.strip()
    else:
        emulation_name = work.strip()

    return {
        "instrument": instrument,
        "emulationName": emulation_name,
        "channel": channel,
        "signal": signal,
    }


def empty_preset_meta() -> dict[str, str]:
    return {
        "chainHex": "",
        "instrument": "",
        "emulationName": "",
        "channel": "",
        "signal": "",
    }


def walk_models(
    entries: list[tuple[str, str, str]],
    catalog_category: str,
    sub_name: str | None,
    models: list | None,
) -> None:
    if not models:
        return
    for m in models:
        if not isinstance(m, dict):
            continue
        if "name" not in m or not isinstance(m["name"], str):
            continue
        model_name = m["name"].strip()
        if not model_name:
            continue
        match = best_modules_match(entries, catalog_category, sub_name, model_name)
        meta = empty_preset_meta()
        if match:
            hex_k, _mc, name_long = match
            meta["chainHex"] = hex_k
            parsed = parse_suffix(name_long, model_name)
            meta.update(parsed)
        m["presetMeta"] = meta


def process_catalog(data: dict) -> None:
    entries = load_modules_entries()
    for cat in data.get("categories", []) or []:
        if not isinstance(cat, dict):
            continue
        cname = cat.get("name")
        if not isinstance(cname, str):
            continue
        walk_models(entries, cname, None, cat.get("models"))
        for sub in cat.get("subcategories") or []:
            if not isinstance(sub, dict):
                continue
            sname = sub.get("name")
            sn = sname if isinstance(sname, str) else None
            walk_models(entries, cname, sn, sub.get("models"))


def main() -> int:
    path = CATALOG_PATH
    if len(sys.argv) > 1:
        path = Path(sys.argv[1])
    data = json.loads(path.read_text(encoding="utf-8"))
    process_catalog(data)
    path.write_text(json.dumps(data, ensure_ascii=False, indent=4) + "\n", encoding="utf-8")
    print(f"Wrote {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
