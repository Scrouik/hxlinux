#!/usr/bin/env python3
"""
Post-traitement de HX_ModelCatalog.json (source unique : le catalogue lui-même).

Anciennement ce script croisait un export hex → noms ; tout passe désormais par le catalogue.
- Complète les champs texte vides de `presetMeta` (instrument, emulationName, channel, signal)
  en analysant le seul champ `name` du modèle (`parse_suffix`).
- Ne touche pas à `chainHex` : à renseigner manuellement ou via d’autres outils dans le JSON.

Usage : python3 scripts/enrich_catalog_preset_meta.py [chemin HX_ModelCatalog.json]
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG_PATH = ROOT / "src-tauri/resources/HX_ModelCatalog.json"


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


def chain_hex_is_empty(pm: dict) -> bool:
    ch = pm.get("chainHex")
    if ch is None:
        return True
    if isinstance(ch, str):
        return not ch.strip()
    if isinstance(ch, list):
        return not any(isinstance(x, str) and x.strip() for x in ch)
    return True


def walk_and_repair_preset_meta(data: dict) -> tuple[int, int]:
    """Retourne (nb fiches sans chainHex, nb champs presetMeta complétés)."""
    missing_hex = 0
    filled = 0

    def visit(models: list | None) -> None:
        nonlocal missing_hex, filled
        if not models:
            return
        for m in models:
            if not isinstance(m, dict):
                continue
            model_name = (m.get("name") or "").strip()
            if not model_name:
                continue
            pm = m.get("presetMeta")
            if not isinstance(pm, dict):
                pm = empty_preset_meta()
                m["presetMeta"] = pm
            if chain_hex_is_empty(pm):
                missing_hex += 1
            parsed = parse_suffix(model_name, model_name)
            for k, v in parsed.items():
                if not v:
                    continue
                cur = pm.get(k)
                if cur is None or str(cur).strip() == "":
                    pm[k] = v
                    filled += 1

    models_root = data.get("models")
    if isinstance(models_root, list) and len(models_root) > 0:
        visit(models_root)
        return missing_hex, filled

    for cat in data.get("categories", []) or []:
        if not isinstance(cat, dict):
            continue
        cname = cat.get("name")
        if not isinstance(cname, str):
            continue
        visit(cat.get("models"))
        for sub in cat.get("subcategories") or []:
            if isinstance(sub, dict):
                visit(sub.get("models"))
    return missing_hex, filled


def main() -> int:
    path = CATALOG_PATH
    if len(sys.argv) > 1:
        path = Path(sys.argv[1])
    data = json.loads(path.read_text(encoding="utf-8"))
    missing, filled = walk_and_repair_preset_meta(data)
    path.write_text(json.dumps(data, ensure_ascii=False, indent=4) + "\n", encoding="utf-8")
    print(f"Écrit {path}")
    print(f"Fiches sans chainHex (à compléter à la main) : {missing}")
    print(f"Champs presetMeta complétés depuis `name` : {filled}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
