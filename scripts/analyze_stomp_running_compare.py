#!/usr/bin/env python3
"""Compare captures stomp_running / connect — amorçage ARM + dialogue fond scroll.

Usage:
  python3 scripts/analyze_stomp_running_compare.py \\
    captures/usb-wireshark/stomp_running_start_hxedit.json \\
    captures/usb-wireshark/stomp_running_start_hxlinux.json

Référence: docs/todo-scroll-hw.md (jalon OUT [7–12], fond idle ≥200 IN 1d / ~27s).
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator


SCROLL_HEAD = bytes([0xF0, 0x03, 0x02, 0x10])
F0_ANCHOR = bytes([0x02, 0x10, 0xF0, 0x03])
ED_ANCHOR = bytes([0x80, 0x10, 0xED, 0x03])
EF_ANCHOR = bytes([0x01, 0x10, 0xEF, 0x03])


def parse_capdata(raw: str | None) -> bytes | None:
    if not raw:
        return None
    try:
        return bytes(int(x, 16) for x in raw.split(":"))
    except ValueError:
        return None


@dataclass
class Pkt:
    num: int
    rel: float
    direction: str
    data: bytes


def iter_packets(path: Path) -> Iterator[Pkt]:
    with path.open(encoding="utf-8") as f:
        arr = json.load(f)
    for pkt in arr:
        layers = pkt.get("_source", {}).get("layers", {})
        usb = layers.get("usb", {})
        cap = parse_capdata(layers.get("usb.capdata") or usb.get("usb.capdata"))
        if cap is None:
            continue
        direction = "OUT" if usb.get("usb.src") == "host" else "IN"
        frame = layers.get("frame", {})
        yield Pkt(
            num=int(frame.get("frame.number", 0)),
            rel=float(frame.get("frame.time_relative", 0) or 0),
            direction=direction,
            data=cap,
        )


def is_scroll_fond_in(data: bytes) -> bool:
    return (
        len(data) == 40
        and data[0] in (0x1D, 0x1F)
        and len(data) >= 8
        and data[4:8] == SCROLL_HEAD
    )


def classify_out_arm(data: bytes) -> str | None:
    if len(data) != 16 or data[11] != 0x08 or data[12:14] != bytes([0x09, 0x10]):
        return None
    if data[4:8] == ED_ANCHOR:
        return "ARM_ed"
    if data[4:8] == F0_ANCHOR:
        return "ARM_f0"
    if data[4:8] == EF_ANCHOR:
        return "ARM_ef"
    return None


def classify_out_f0_ack(data: bytes) -> str | None:
    if len(data) != 16 or data[4:8] != F0_ANCHOR:
        return None
    sub = data[11] if len(data) > 11 else None
    if sub == 0x08:
        return "f0_ack08"
    if sub == 0x10:
        return "f0_poll10"
    return None


def first_arm_sequence(outs: list[Pkt], max_arms: int = 8) -> list[dict[str, Any]]:
    seq: list[dict[str, Any]] = []
    for p in outs:
        kind = classify_out_arm(p.data)
        if kind:
            seq.append(
                {
                    "frame": p.num,
                    "rel_ms": round(p.rel * 1000),
                    "kind": kind,
                    "double": f"{p.data[12]:02x}:{p.data[13]:02x}",
                }
            )
        if len(seq) >= max_arms:
            break
    return seq


def scroll_fond_stats(pkts: list[Pkt]) -> dict[str, Any]:
    in_1d = in_1f = 0
    out_ack08 = 0
    out_poll10 = 0
    first_1d_frame: int | None = None
    first_ack_frame: int | None = None
    ack_doubles: list[str] = []

    for p in pkts:
        if p.direction == "IN" and is_scroll_fond_in(p.data):
            if p.data[0] == 0x1D:
                in_1d += 1
            else:
                in_1f += 1
            if first_1d_frame is None:
                first_1d_frame = p.num
        if p.direction == "OUT":
            k = classify_out_f0_ack(p.data)
            if k == "f0_ack08":
                out_ack08 += 1
                ack_doubles.append(f"{p.data[12]:02x}:{p.data[13]:02x}")
                if first_ack_frame is None:
                    first_ack_frame = p.num
            elif k == "f0_poll10":
                out_poll10 += 1

    ratio = round(out_ack08 / in_1d, 3) if in_1d else None
    return {
        "in_1d": in_1d,
        "in_1f": in_1f,
        "out_f0_sub08": out_ack08,
        "out_f0_sub10": out_poll10,
        "ack_per_1d": ratio,
        "first_in_1d_frame": first_1d_frame,
        "first_ack08_frame": first_ack_frame,
        "first5_ack_double": ack_doubles[:5],
        "last3_ack_double": ack_doubles[-3:] if ack_doubles else [],
    }


def bootstrap_window(outs: list[Pkt], arms: list[dict]) -> list[dict[str, Any]]:
    """Premiers OUT host (16/36 o) jusqu’au 2e ARM_ef ou 40 paquets."""
    if not arms:
        return []
    end_rel = None
    arm_ef_count = 0
    for a in arms:
        if a["kind"] == "ARM_ef":
            arm_ef_count += 1
            if arm_ef_count >= 1:
                end_rel = (a["rel_ms"] / 1000.0) + 1.2
                break
    if end_rel is None:
        end_rel = outs[-1].rel if outs else 0

    rows: list[dict[str, Any]] = []
    for p in outs:
        if p.rel > end_rel:
            break
        if len(p.data) not in (16, 36, 17, 25, 28, 44):
            continue
        tag = f"{p.data[0]:02x}:{p.data[1]:02x}:{p.data[2]:02x}:{p.data[3]:02x}"
        arm = classify_out_arm(p.data)
        f0k = classify_out_f0_ack(p.data)
        rows.append(
            {
                "frame": p.num,
                "rel_ms": round(p.rel * 1000),
                "len": len(p.data),
                "tag": tag,
                "arm": arm,
                "f0": f0k,
            }
        )
        if len(rows) >= 25:
            break
    return rows


def analyze(path: Path) -> dict[str, Any]:
    pkts = sorted(iter_packets(path), key=lambda p: (p.rel, p.num))
    outs = [p for p in pkts if p.direction == "OUT"]
    duration_s = max((p.rel for p in pkts), default=0.0)
    arms = first_arm_sequence(outs)
    return {
        "file": path.name,
        "duration_s": round(duration_s, 2),
        "packets": len(pkts),
        "arm_sequence": arms,
        "bootstrap_out": bootstrap_window(outs, arms),
        "scroll_fond": scroll_fond_stats(pkts),
    }


def print_report(r: dict[str, Any]) -> None:
    print(f"\n{'=' * 60}")
    print(f"  {r['file']}  ({r['duration_s']}s, {r['packets']} pkts capdata)")
    sf = r["scroll_fond"]
    print(
        f"  Fond scroll: IN 1d={sf['in_1d']} 1f={sf['in_1f']}  "
        f"OUT f0/08={sf['out_f0_sub08']} f0/10={sf['out_f0_sub10']}  "
        f"ratio ACK/1d={sf['ack_per_1d']}"
    )
    if sf["first_in_1d_frame"]:
        print(
            f"    1er IN 1d fond #{sf['first_in_1d_frame']}  "
            f"1er OUT ack08 #{sf['first_ack08_frame']}"
        )
        print(f"    ack doubles: {sf['first5_ack_double']} … {sf['last3_ack_double']}")
    print("  Séquence ARM (premiers):")
    for a in r["arm_sequence"]:
        print(
            f"    #{a['frame']:5d} +{a['rel_ms']:6d}ms  {a['kind']:8s}  lane {a['double']}"
        )
    print("  Bootstrap OUT (extrait):")
    for row in r["bootstrap_out"][:18]:
        extra = row["arm"] or row["f0"] or ""
        print(
            f"    #{row['frame']:5d} +{row['rel_ms']:6d}ms  len={row['len']:2d}  "
            f"{row['tag']}  {extra}"
        )


def compare_pair(hx: dict, lx: dict) -> None:
    print(f"\n{'=' * 60}")
    print("  COMPARAISON HX Edit vs Linux")
    ha = [a["kind"] for a in hx["arm_sequence"]]
    la = [a["kind"] for a in lx["arm_sequence"]]
    print(f"  ARM order HX: {' → '.join(ha) or '(aucun)'}")
    print(f"  ARM order LX: {' → '.join(la) or '(aucun)'}")
    if ha == la:
        print("  ARM order: OK (identique)")
    else:
        print("  ARM order: ÉCART — priorité fix amorçage")

    for key in ("in_1d", "out_f0_sub08", "ack_per_1d"):
        h, l = hx["scroll_fond"][key], lx["scroll_fond"][key]
        print(f"  {key}: HX={h}  LX={l}")

    h1d = hx["scroll_fond"]["in_1d"] or 0
    l1d = lx["scroll_fond"]["in_1d"] or 0
    if h1d >= 200 and l1d >= 200:
        print("  Fond idle: OK (≥200 IN 1d)")
    elif h1d >= 200 and l1d < 50:
        print("  Fond idle: KO — Stomp ne pousse pas (ou fil pas EditorReady)")
    else:
        print("  Fond idle: capture courte ou procédure ≠ stomp_running (~30s idle)")


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    paths = [Path(p) for p in sys.argv[1:]]
    for p in paths:
        if not p.is_file():
            print(f"Missing: {p}", file=sys.stderr)
            return 1
    reports = [analyze(p) for p in paths]
    for r in reports:
        print_report(r)
    if len(reports) == 2:
        compare_pair(reports[0], reports[1])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
