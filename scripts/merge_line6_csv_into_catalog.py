#!/usr/bin/env python3
"""
Renomme presetMeta.channel -> basedOn, presetMeta.signal -> subCategory dans HX_ModelCatalog.json,
puis fusionne les colonnes Subcategory / Based on du CSV Line 6 (jointure sur le champ name).

Résolution du nom (dans l’ordre) :
  1) nom catalogue tel quel, sans daggers, forme « canon » (Norm→Nrm, 1960A→1960, Rouge→Rogue, Cali Texas Ch N→ChN) ;
  2) clef norm_soft (tiret 12-String → espace, Ch 1 → Ch1, etc.) ;
  3) alias : scripts/data/catalog_csv_name_aliases.json (prioritaire) + DEFAULT_CATALOG_CSV_ALIASES dans ce fichier.

Sources CSV (dans l’ordre) :
  1) chemin passé en argv[2] si fourni ;
  2) src-tauri/resources/Line 6 Models descriptions.csv si non vide ;
  3) concaténation de scripts/data/line6_csv_part1.csv + line6_csv_part2.csv (copie dépôt).
"""
from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG_PATH = ROOT / "src-tauri/resources/HX_ModelCatalog.json"
RESOURCES_CSV = ROOT / "src-tauri/resources/Line 6 Models descriptions.csv"
PART1 = ROOT / "scripts/data/line6_csv_part1.csv"
PART2 = ROOT / "scripts/data/line6_csv_part2.csv"
OPTIONAL_ALIASES_JSON = ROOT / "scripts/data/catalog_csv_name_aliases.json"

# Noms catalogue HX -> nom exact « Name » du CSV (écarts sûrs ou très probables).
# Ajouter / corriger dans scripts/data/catalog_csv_name_aliases.json (écrase ces entrées).
DEFAULT_CATALOG_CSV_ALIASES: dict[str, str] = {
    "Script Phase": "Script Mod Phase",
    "Tri Chorus": "Trinity Chorus",
    "Pillars": "Pillars OD",
    "Mandarin Bass200": "Mandarin 200",
    "Bass Octaver": "Boctaver",
    "4x10 Ampeg HLF": "4x10 Ampeg Pro",
    "Poly Bass Wham": "Poly Wham†",
    "2x15 US Dripman": "2x15 Dripman",
    "1x18 Woody Blue": "Woody Blue",
    "Octi Synth": "3 OSC Synth",
    "Smart Harmony": "Twin Harmony",
    "Synth Harmony": "Twin Harmony",
    "Attack Synth": "3 OSC Synth",
    "Analog Synth": "3 OSC Synth",
    "Rez Synth": "3 OSC Synth",
    "Seismik Synth": "3 OSC Synth",
    "8x10 Ampeg SVT E": "8x10 SVT AV",
    "Analog Chorus": "Chorus",
    "Bubble Echo": "Bubble Vibrato",
    "Analog Echo": "Mod/Chorus Echo",
    "Reverse": "Reverse Delay",
    "Volume": "Volume Pedal",
    "Stereo": "Stereo Width",
    "Tile": "Tilt",
    "L6 Drive": "Valve Driver",
    "Jet Fuzz": "Pocket Fuzz",
    "Screamer": "Scream 808",
    "Ring Modulator": "AM Ring Mod",
    "Analog Flanger": "Gray Flanger",
    "AC Flanger": "Gray Flanger",
    "80A Flanger": "Gray Flanger",
    "Jet Flanger": "Gray Flanger",
    "Opto Tremolo": "Optical Trem",
    "Bias Tremolo": "60s Bias Trem",
    "Auto-Volume Echo": "Autoswell",
    "Phaser": "Deluxe Phaser",
    "Dual Phaser": "Deluxe Phaser",
    "Octave Fuzz": "Tycoctavia Fuzz",
    "Synth String": "12 String†",
    "Jumbo Fuzz": "Bighorn Fuzz",
    "Facial Fuzz": "Triangle Fuzz",
    "Sub Oct Fuzz": "Boctaver",
    "Saturn 5 RingMod": "AM Ring Mod",
    "Q Filter": "Mutant Filter",
    "Panned Phaser": "Pebble Phaser",
    "Overdrive": "Valve Driver",
    "Tube Drive": "Prize Drive",
    "Dig w/Mod": "AM Ring Mod",
    "Analog w/Mod": "AM Ring Mod",
    "Colordrive": "Clawthorn Drive",
    "Tape Echo": "Sweep Echo",
    "Tube Echo": "Sweep Echo",
    "Digital": "Vintage Digital",
    "Dynamic": "Dynamic Hall",
    "Plate": "Plateaux",
    "Tube Comp": "Deluxe Comp",
    "Blue Comp": "Deluxe Comp",
    "Blue Comp Treb": "Red Squeeze",
    "Boost Comp": "Kinky Comp",
    "Red Comp": "Rochester Comp",
    "Classic Dist": "Ratatouille Dist",
    "Heavy Dist": "Vital Dist",
    "L6 Distortion": "Line 6 Aristocrat",
    "Synth Lead": "3 OSC Synth",
    "Synth FX": "Glitz",
    "Synth O Matic": "3 Note Generator ‡",
    "Phaze Eko": "Mod/Chorus Echo",
    "Voice Box": "4-Voice Chorus",
    "Particle Verb": "Plateaux",
    "Comet Trails": "Searchlights",
    "Frequency Shift": "Pitch Ring Mod",
    "Random S&H": "Mutant Filter",
    "Lo Res": "Bitcrusher",
    "DT 25/50": "Line 6 Badonk",
    "Obsidian 7000": "Heliosphere",
    "Octo": "Conductor",
    "Panner": "Pan",
    "Obi Wah": "UK Wah 846",
    "Tape Eater": "Tesselator",
    "Warble Matic": "FlexoVibe",
    "U-Vibe": "Ubiquitous Vibe",
    "Tron Up": "Fullerton Jump",
    "Tron Down": "Fullerton Nrm",
    "Studio Tube Pre": "US Deluxe Nrm",
    "Spin Cycle": "US Princess",
    "Pattern Tremolo": "Harmonic Tremolo",
    "Pitch Vibrato": "Bubble Vibrato",
    "Multi-Head": "Multitap 4",
    "Echo Platter": "Double Tank",
    "Ducking": "Ducked Delay",
    "Dimension": "Stereo Imager",
    "Chamber": "Chrome",
    "Room": "Chrome",
    "Throbber": "Chrome",
    "Cave": "Boctaver",
    "Growler": "Clawthorn Drive",
    "Killer Z": "ANGL Meteor",
    "Fuzz Pi": "Triangle Fuzz",
    "Barberpole": "Harmonic Tremolo",
    "Slow Filter": "Mutant Filter",
    "String Theory": "Trinity Chorus",
    "Rotary Drum": "122 Rotary",
    "Rotary Drum/Horn": "145 Rotary",
    "Hall": "Dynamic Hall",
    "Vetta Comp": "Vetta Wah",
    "Vetta Juice": "Vetta Wah",
}


def strip_daggers(s: str) -> str:
    return (
        s.strip()
        .replace("\u2020", "")
        .replace("\u2021", "")
        .replace("†", "")
        .replace("‡", "")
        .strip()
    )


def norm_soft(s: str) -> str:
    """Normalisation pour rapprocher catalogue / CSV (tiret 12-String, Ch 1 / Ch1, etc.)."""
    s = strip_daggers(s).lower()
    s = s.replace("’", "'").replace("‘", "'").replace("×", "x").replace("÷", "/")
    s = re.sub(r"(?<=[0-9])-(?=[a-z])", " ", s, flags=re.I)
    s = re.sub(r"\s+", " ", s.strip())
    s = re.sub(r"(?i)\bch\s+(\d)\b", r"ch\1", s)
    s = re.sub(r"[®™]", "", s)
    return s


def canon_catalog_display_name(name: str) -> str:
    """Corrections d’affichage HX fréquentes avant lecture du CSV."""
    s = name.strip()
    s = re.sub(r"\bNorm\b", "Nrm", s, flags=re.I)
    s = re.sub(r"1960A\b", "1960", s, flags=re.I)
    s = re.sub(r"Rouge\b", "Rogue", s, flags=re.I)
    s = re.sub(r"Cali Texas Ch\s+(\d)", r"Cali Texas Ch\1", s, flags=re.I)
    return s


def load_merged_aliases() -> dict[str, str]:
    out = dict(DEFAULT_CATALOG_CSV_ALIASES)
    if not OPTIONAL_ALIASES_JSON.is_file():
        return out
    try:
        raw = json.loads(OPTIONAL_ALIASES_JSON.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return out
    if not isinstance(raw, dict):
        return out
    for k, v in raw.items():
        if not isinstance(k, str) or not isinstance(v, str):
            continue
        if k.startswith("_") or k.startswith("__"):
            continue
        kk, vv = k.strip(), v.strip()
        if kk and vv:
            out[kk] = vv
    return out


def parse_subcategory_cell(raw: str) -> str | list[str]:
    s = raw.strip()
    if not s:
        return ""
    if "," in s:
        parts = [p.strip() for p in s.split(",") if p.strip()]
        if len(parts) >= 2:
            return [parts[0], parts[1]]
        return parts[0] if parts else ""
    return s


def read_csv_rows() -> list[dict[str, str]]:
    paths: list[Path] = []
    if len(sys.argv) > 2 and sys.argv[2].strip():
        paths.append(Path(sys.argv[2]).expanduser())
    paths.append(RESOURCES_CSV)
    text: str | None = None
    for p in paths:
        if p.is_file() and p.stat().st_size > 10:
            text = p.read_text(encoding="utf-8")
            print(f"Lecture CSV : {p}")
            break
    if text is None:
        chunks = []
        for p in (PART1, PART2):
            if not p.is_file():
                raise SystemExit(f"CSV introuvable : ni resources ni {PART1}")
            chunks.append(p.read_text(encoding="utf-8").strip())
        text = "\n".join(chunks) + "\n"
        print("Lecture CSV : scripts/data/line6_csv_part1.csv + line6_csv_part2.csv")
    rows: list[dict[str, str]] = []
    for row in csv.reader(text.splitlines(), delimiter=";"):
        if not row or not row[0].strip():
            continue
        if row[0].strip().lower() == "name":
            continue
        name = row[0].strip()
        sub = row[1].strip() if len(row) > 1 else ""
        based = row[2].strip() if len(row) > 2 else ""
        rows.append({"name": name, "subcategory": sub, "based_on": based})
    return rows


def build_csv_indexes(rows: list[dict[str, str]]):
    """Index exact + clef norm_soft -> ligne CSV (première occurrence)."""
    by_exact: dict[str, dict[str, str]] = {}
    by_ns: dict[str, dict[str, str]] = {}
    for r in rows:
        n = r["name"]
        by_exact[n] = r
        stripped = strip_daggers(n)
        if stripped and stripped != n:
            by_exact.setdefault(stripped, r)
        ns = norm_soft(n)
        by_ns.setdefault(ns, r)
    return by_exact, by_ns


def resolve_csv_row(
    catalog_model_name: str,
    by_exact: dict[str, dict[str, str]],
    by_ns: dict[str, dict[str, str]],
    aliases: dict[str, str],
) -> dict[str, str] | None:
    """Trouve la ligne CSV pour un nom de modèle catalogue."""

    def try_one(n: str) -> dict[str, str] | None:
        n = n.strip()
        if not n:
            return None
        if n in by_exact:
            return by_exact[n]
        sd = strip_daggers(n)
        if sd in by_exact:
            return by_exact[sd]
        c = canon_catalog_display_name(n)
        if c != n and c in by_exact:
            return by_exact[c]
        c2 = canon_catalog_display_name(sd)
        if c2 in by_exact:
            return by_exact[c2]
        k = norm_soft(n)
        if k in by_ns:
            return by_ns[k]
        return None

    base = catalog_model_name.strip()
    for candidate in (base, aliases.get(base, "")):
        if not candidate:
            continue
        if hit := try_one(candidate):
            return hit
    return None


def rename_preset_meta_keys(pm: dict) -> None:
    if "channel" in pm:
        v = pm.pop("channel")
        if "basedOn" not in pm or str(pm.get("basedOn", "")).strip() == "":
            pm["basedOn"] = v
    if "signal" in pm:
        v = pm.pop("signal")
        if "subCategory" not in pm or pm.get("subCategory") in (None, "", []):
            pm["subCategory"] = v


def apply_csv_to_model(
    m: dict,
    by_exact: dict[str, dict[str, str]],
    by_ns: dict[str, dict[str, str]],
    aliases: dict[str, str],
) -> bool:
    name = (m.get("name") or "").strip()
    if not name:
        return False
    row = resolve_csv_row(name, by_exact, by_ns, aliases)
    if not row:
        return False
    pm = m.get("presetMeta")
    if not isinstance(pm, dict):
        pm = {}
        m["presetMeta"] = pm
    bo = row.get("based_on") or ""
    if bo:
        pm["basedOn"] = bo
    pm["subCategory"] = parse_subcategory_cell(row.get("subcategory") or "")
    return True


def iter_models(data: dict):
    models_root = data.get("models")
    if isinstance(models_root, list) and len(models_root) > 0:
        for m in models_root:
            if isinstance(m, dict):
                yield m
        return
    for cat in data.get("categories", []) or []:
        if not isinstance(cat, dict):
            continue
        for m in cat.get("models") or []:
            if isinstance(m, dict):
                yield m
        for sub in cat.get("subcategories") or []:
            if not isinstance(sub, dict):
                continue
            for m in sub.get("models") or []:
                if isinstance(m, dict):
                    yield m


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) > 1 and sys.argv[1].strip() else CATALOG_PATH
    data = json.loads(path.read_text(encoding="utf-8"))
    rows = read_csv_rows()
    by_exact, by_ns = build_csv_indexes(rows)
    aliases = load_merged_aliases()
    renamed = 0
    csv_hits = 0
    for m in iter_models(data):
        pm = m.get("presetMeta")
        if isinstance(pm, dict):
            before = set(pm.keys())
            rename_preset_meta_keys(pm)
            if before != set(pm.keys()):
                renamed += 1
        if apply_csv_to_model(m, by_exact, by_ns, aliases):
            csv_hits += 1
    path.write_text(json.dumps(data, ensure_ascii=False, indent=4) + "\n", encoding="utf-8")
    print(f"Écrit {path}")
    print(f"Fiches presetMeta renommées (channel/signal) : {renamed}")
    print(f"Fiches enrichies depuis le CSV (name) : {csv_hits}")
    print(f"Lignes CSV : {len(rows)} ; alias actifs : {len(aliases)} ; clefs index exact : {len(by_exact)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
