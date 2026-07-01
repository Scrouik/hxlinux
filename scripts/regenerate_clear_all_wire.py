#!/usr/bin/env python3
"""Regenerate src-tauri/src/helix/clear_all_preset_blocks_wire.rs from Wireshark JSON."""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CAPTURE = ROOT / "captures/usb-wireshark/clear_all_block.json"
OUT = ROOT / "src-tauri/src/helix/clear_all_preset_blocks_wire.rs"
FRAME_IDS = [
    4171, 4173, 4177, 4179, 4183, 4185, 4189, 4193, 4195, 4199, 4203, 4355, 4357, 4367,
]


def parse_capdata(raw: str | None) -> bytes | None:
    if not raw:
        return None
    try:
        return bytes(int(x, 16) for x in raw.split(":"))
    except ValueError:
        return None


def main() -> None:
    with CAPTURE.open(encoding="utf-8") as f:
        arr = json.load(f)
    by_frame: dict[int, bytes] = {}
    for pkt in arr:
        layers = pkt.get("_source", {}).get("layers", {})
        usb = layers.get("usb", {})
        if usb.get("usb.src") != "host":
            continue
        cap = parse_capdata(layers.get("usb.capdata") or usb.get("usb.capdata"))
        if cap is None:
            continue
        frame = int(layers.get("frame", {}).get("frame.number", 0))
        if frame in FRAME_IDS:
            by_frame[frame] = cap

    missing = [f for f in FRAME_IDS if f not in by_frame]
    if missing:
        raise SystemExit(f"Missing frames in {CAPTURE}: {missing}")

    lines = [
        "// Generated from captures/usb-wireshark/clear_all_block.json — do not edit by hand.",
        "// Regenerate: python3 scripts/regenerate_clear_all_wire.py",
        "",
    ]
    for i, fr in enumerate(FRAME_IDS):
        cap = by_frame[fr]
        name = f"CLEAR_ALL_PKT_{i:02d}_FR_{fr}"
        lines.append(f"pub const {name}: &[u8] = &{list(cap)};")
        lines.append("")
    lines.append("pub const CLEAR_ALL_OUT_PACKETS: &[&[u8]] = &[")
    for i, fr in enumerate(FRAME_IDS):
        lines.append(f"    CLEAR_ALL_PKT_{i:02d}_FR_{fr},")
    lines.append("];")
    lines.append("")
    lines.append("/// Index du premier paquet après lequel la lane `live_write_ctr` avance de `0x11`.")
    lines.append("pub const CLEAR_ALL_LANE_BUMP_AFTER_PACKET: usize = 6;")
    lines.append("pub const CLEAR_ALL_LANE_BUMP_DELTA: u16 = 0x11;")
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Wrote {OUT} ({len(FRAME_IDS)} packets)")


if __name__ == "__main__":
    main()
