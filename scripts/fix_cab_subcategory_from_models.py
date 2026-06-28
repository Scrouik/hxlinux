#!/usr/bin/env python3
"""
Corrige `presetMeta.subCategory` pour les modèles Cab dans HX_ModelCatalog.json :

- id présent dans `cab.models` → `Legacy` (ancien bloc HD2_Cab*)
- id présent dans `cabmicirswithpan.models` → `Dual`
- id présent dans `cabmicirs.models` → `Single`

La présence d’un HD2_Cab* legacy dans `cab.models` ne propage pas Legacy vers les
CabMicIr associés : chaque id est classé selon le fichier où il apparaît.

Usage : python3 scripts/fix_cab_subcategory_from_models.py [chemin HX_ModelCatalog.json]
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG_PATH = ROOT / "src-tauri/resources/HX_ModelCatalog.json"
MODELS_DIR = ROOT / "src-tauri/resources/models"


def load_model_ids(path: Path) -> set[str]:
    data = json.loads(path.read_text(encoding="utf-8"))
    return {
        m["symbolicID"]
        for m in data
        if isinstance(m, dict) and m.get("category") is not None and "symbolicID" in m
    }


def cab_sub_category(
    model_id: str,
    cab_ids: set[str],
    mic_ids: set[str],
    pan_ids: set[str],
) -> str | None:
    if model_id in cab_ids:
        return "Legacy"
    if model_id in pan_ids:
        return "Dual"
    if model_id in mic_ids:
        return "Single"
    return None


def main() -> int:
    path = CATALOG_PATH if len(sys.argv) <= 1 else Path(sys.argv[1])

    cab_ids = load_model_ids(MODELS_DIR / "cab.models")
    mic_ids = load_model_ids(MODELS_DIR / "cabmicirs.models")
    pan_ids = load_model_ids(MODELS_DIR / "cabmicirswithpan.models")

    data = json.loads(path.read_text(encoding="utf-8"))
    models = data.get("models")
    if not isinstance(models, list):
        print("Catalogue sans tableau `models`.", file=sys.stderr)
        return 1

    changed = 0
    unmapped: list[str] = []
    for m in models:
        if not isinstance(m, dict):
            continue
        pm = m.get("presetMeta")
        if not isinstance(pm, dict) or pm.get("categoryName") != "Cab":
            continue
        mid = m.get("id")
        if not isinstance(mid, str) or not mid:
            continue
        new_sc = cab_sub_category(mid, cab_ids, mic_ids, pan_ids)
        if new_sc is None:
            unmapped.append(mid)
            continue
        if pm.get("subCategory") != new_sc:
            pm["subCategory"] = new_sc
            changed += 1

    path.write_text(json.dumps(data, ensure_ascii=False, indent=4) + "\n", encoding="utf-8")
    print(f"Écrit {path}")
    print(f"subCategory Cab mis à jour : {changed}")
    if unmapped:
        print(f"Sans fichier .models ({len(unmapped)}) :", ", ".join(unmapped), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
