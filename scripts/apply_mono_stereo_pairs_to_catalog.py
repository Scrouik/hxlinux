#!/usr/bin/env python3
"""
Pour chaque paire mono/stéréo dans modules_by_id.json (même catégorie, même base sans
suffixe final (mono)|(stereo)|(stéréo), même indice Guitar/Bass si présent),
met à jour la fiche catalogue HX (modèle avec « name ») :
  presetMeta.chainHex = [hex_mono, hex_stereo]
  presetMeta.signal   = ["mono", "stereo"]

Réutilise les heuristiques de enrich_catalog_preset_meta.py.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MODULES_PATH = ROOT / "src-tauri/resources/modules_by_id.json"
CATALOG_PATH = ROOT / "src-tauri/resources/HX_ModelCatalog.json"

SUFFIX_RE = re.compile(r"\s*\((mono|stereo|stéréo)\)\s*$", re.I)


def norm_signal_label(kind: str) -> str:
    k = kind.lower()
    if k in ("stéréo", "stereo"):
        return "stereo"
    if k == "mono":
        return "mono"
    return k


def split_mono_stereo(name_str: str) -> tuple[str | None, str | None, str]:
    m = SUFFIX_RE.search(name_str.strip())
    if not m:
        return None, None, name_str.strip()
    kind = norm_signal_label(m.group(1))
    if kind not in ("mono", "stereo"):
        return None, None, name_str.strip()
    base = name_str[: m.start()].strip()
    return base, kind, name_str.strip()


def instrument_hint(name_long: str) -> str:
    w = f" {name_long} ".lower()
    if " guitar " in w:
        return "guitar"
    if " bass " in w:
        return "bass"
    return ""


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


def build_mono_stereo_pairs(
    entries: list[tuple[str, str, str]],
) -> list[tuple[str, str, str, str, str, str, str]]:
    """
    (cat_display, base_lower, inst_hint, hex_mono, hex_stereo, name_mono_full, name_stereo_full)
    """
    buckets: dict[tuple[str, str, str], dict] = {}
    for hex_k, cat, name in entries:
        base, kind, full = split_mono_stereo(name)
        if base is None or kind is None:
            continue
        key = (cat.strip().lower(), base.lower(), instrument_hint(full))
        if key not in buckets:
            buckets[key] = {"cat_display": cat, "mono": None, "stereo": None}
        slot = "mono" if kind == "mono" else "stereo"
        prev = buckets[key].get(slot)
        if prev is None or len(full) > len(prev[1]):
            buckets[key][slot] = (hex_k, full)

    out: list[tuple[str, str, str, str, str, str, str]] = []
    for key, d in buckets.items():
        if d.get("mono") is None or d.get("stereo") is None:
            continue
        hm, nm = d["mono"]
        hs, ns = d["stereo"]
        out.append((d["cat_display"], key[1], key[2], hm, hs, nm, ns))
    return out


def walk_catalog_and_apply(
    data: dict,
    pairs: list[tuple[str, str, str, str, str, str, str]],
) -> tuple[int, int]:
    updated = 0
    no_match = 0

    for cat_display, _base_l, _inst, hex_m, hex_s, nm, ns in pairs:

        def model_matches(cat_name: str, sub_name: str | None, m: dict) -> bool:
            if not isinstance(m, dict) or "name" not in m:
                return False
            cn = m["name"].strip()
            if not cn:
                return False
            if cat_name.strip() != cat_display.strip():
                return False
            if not name_long_matches_catalog_name(nm, cn):
                return False
            if not name_long_matches_catalog_name(ns, cn):
                return False
            if not subcategory_fits_name_long(sub_name, nm):
                return False
            if not subcategory_fits_name_long(sub_name, ns):
                return False
            return True

        best_m: dict | None = None
        best_len = -1

        for cat in data.get("categories", []) or []:
            if not isinstance(cat, dict):
                continue
            cname = cat.get("name")
            if not isinstance(cname, str) or cname.strip() != cat_display.strip():
                continue

            def scan(sub_name: str | None, models: list | None) -> None:
                nonlocal best_m, best_len
                if not models:
                    return
                for m in models:
                    if not isinstance(m, dict):
                        continue
                    if not model_matches(cname, sub_name, m):
                        continue
                    cn = m["name"].strip()
                    if len(cn) > best_len:
                        best_len = len(cn)
                        best_m = m

            scan(None, cat.get("models"))
            for sub in cat.get("subcategories") or []:
                if not isinstance(sub, dict):
                    continue
                sn = sub.get("name") if isinstance(sub.get("name"), str) else None
                scan(sn, sub.get("models"))

        if best_m is None:
            no_match += 1
            print(f"[skip] pas de fiche catalogue pour paire {cat_display!r} : mono={nm[:52]}…")
            continue

        pm = best_m.setdefault("presetMeta", {})
        pm["chainHex"] = [hex_m, hex_s]
        pm["signal"] = ["mono", "stereo"]
        parsed = parse_suffix(nm, best_m["name"].strip())
        for k, v in parsed.items():
            if v and (not pm.get(k) or str(pm.get(k, "")).strip() == ""):
                pm[k] = v
        updated += 1

    return updated, no_match


def main() -> int:
    path = CATALOG_PATH
    if len(sys.argv) > 1:
        path = Path(sys.argv[1])
    entries = load_modules_entries()
    pairs = build_mono_stereo_pairs(entries)
    print(f"Paires mono+stéréo détectées dans modules_by_id.json : {len(pairs)}")
    data = json.loads(path.read_text(encoding="utf-8"))
    up, miss = walk_catalog_and_apply(data, pairs)
    print(f"Fiches catalogue mises à jour : {up}, sans correspondance : {miss}")
    path.write_text(json.dumps(data, ensure_ascii=False, indent=4) + "\n", encoding="utf-8")
    print(f"Écrit {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
