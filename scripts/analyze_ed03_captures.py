#!/usr/bin/env python3
"""Compare HX Edit vs HXLinux Wireshark JSON exports — ED03 counters & ACK patterns."""

from __future__ import annotations

import json
import re
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

ED03 = bytes([0x80, 0x10, 0xED, 0x03])
ED03_REV = bytes([0xED, 0x03, 0x80, 0x10])


def parse_capdata(raw: str | None) -> bytes | None:
    if not raw:
        return None
    try:
        return bytes(int(x, 16) for x in raw.split(":"))
    except ValueError:
        return None


def has_ed03(data: bytes) -> bool:
    return ED03 in data or ED03_REV in data


def ed03_anchor(data: bytes) -> int:
    i = data.find(ED03)
    if i >= 0:
        return i
    return data.find(ED03_REV)


def extract_fields(data: bytes) -> dict[str, Any]:
    n = len(data)
    out: dict[str, Any] = {"len": n}
    if n < 8:
        return out

    # Wire format: first 4 bytes often LE length tag (19/08/0c/21…)
    out["tag"] = f"{data[0]:02x}:{data[1]:02x}:{data[2]:02x}:{data[3]:02x}"

    ai = ed03_anchor(data)
    if ai >= 0:
        out["ed03_at"] = ai
        if ai + 8 <= n:
            out["cnt"] = data[ai + 4]
        if ai + 7 <= n:
            out["sub"] = data[ai + 6]
        # sub for 16/36 with layout 80:10:ed:03 @4 → sub @ byte 10
        if ai == 4 and n >= 11:
            out["sub_b10"] = data[10]

    # 36-byte preset request / assign (Kempline layout)
    if n >= 30:
        idx = data.find(bytes([0x83, 0x66, 0xCD]))
        if idx >= 0 and idx + 3 < n:
            out["cd"] = data[idx + 3]
            if idx + 5 < n:
                out["sess_id"] = data[idx + 4]
        if n >= 30:
            out["double_28_29"] = f"{data[28]:02x}:{data[29]:02x}"
            out["u16_28_29"] = data[28] | (data[29] << 8)

    # 19/36-byte phase: session + double after sub (bytes 12-14 typical)
    if n >= 16:
        out["double_12_13"] = f"{data[12]:02x}:{data[13]:02x}"
        out["u16_12_13"] = data[12] | (data[13] << 8)
        out["byte12"] = data[12]
        out["byte13"] = data[13]

    if n >= 16:
        out["double_13_14"] = f"{data[13]:02x}:{data[14]:02x}"
        out["u16_13_14"] = data[13] | (data[14] << 8)

    # Short 16-byte: ACK chunk (sub=08), keep-alive (sub=10), etc.
    if n == 16 and ai >= 0:
        sub10 = data[10] if n > 10 else None
        sub11 = data[11] if n > 11 else None
        if sub11 == 0x08:
            out["lane"] = "ack_chunk"
            out["ack_session"] = data[12]
            out["ack_double"] = f"{data[13]:02x}:{data[14]:02x}"
        elif sub10 == 0x10:
            out["lane"] = "keepalive"
            out["ka_double"] = f"{data[12]:02x}:{data[13]:02x}"

    return out


def iter_packets(path: Path):
    with path.open(encoding="utf-8") as f:
        arr = json.load(f)
    for pkt in arr:
        layers = pkt.get("_source", {}).get("layers", {})
        usb = layers.get("usb", {})
        cap = parse_capdata(
            layers.get("usb.capdata") or usb.get("usb.capdata")
        )
        if cap is None:
            continue
        direction = "OUT" if usb.get("usb.src") == "host" else "IN"
        frame = layers.get("frame", {})
        yield {
            "num": int(frame.get("frame.number", 0)),
            "rel": float(frame.get("frame.time_relative", 0) or 0),
            "dir": direction,
            "data": cap,
            "fields": extract_fields(cap),
        }


def analyze_file(path: Path, ed03_only: bool = True) -> list[dict]:
    rows = []
    for p in iter_packets(path):
        if ed03_only and not has_ed03(p["data"]):
            continue
        row = {**p, "hex": p["data"].hex(":")}
        del row["data"]
        rows.append(row)
    return rows


def summarize(path: Path) -> dict[str, Any]:
    rows = analyze_file(path)
    doubles_28: list[str] = []
    doubles_12: list[str] = []
    ack_doubles: list[str] = []
    subs: defaultdict[int, int] = defaultdict(int)
    lens: defaultdict[int, int] = defaultdict(int)

    for r in rows:
        f = r["fields"]
        lens[f["len"]] += 1
        if "sub" in f:
            subs[f["sub"]] += 1
        if "double_28_29" in f and f.get("cd") is not None:
            doubles_28.append(f["double_28_29"])
        if r["dir"] == "OUT" and f["len"] in (36, 25):
            doubles_28.append(f.get("double_28_29", ""))
        if "double_12_13" in f and f["len"] in (36, 25, 19):
            doubles_12.append(f["double_12_13"])
        if f.get("lane") == "ack_chunk" and r["dir"] == "OUT":
            ack_doubles.append(f.get("ack_double", ""))
        if f.get("lane") == "keepalive" and r["dir"] == "OUT":
            pass  # tracked separately if needed

    def lane_stats(vals: list[str]) -> dict:
        vals = [v for v in vals if v and v != "00:00"]
        u16s = []
        for v in vals:
            a, b = (int(x, 16) for x in v.split(":"))
            u16s.append(a | (b << 8))
        hi64 = sum(1 for u in u16s if (u >> 8) == 0x64 or (u & 0xFF) == 0x64)
        return {
            "count": len(vals),
            "first5": vals[:5],
            "last5": vals[-5:],
            "unique": len(set(vals)),
            "in_64xx_lane": hi64,
        }

    return {
        "file": path.name,
        "ed03_packets": len(rows),
        "by_len": dict(sorted(lens.items())),
        "by_sub": dict(sorted(subs.items())),
        "double_28_29": lane_stats(doubles_28),
        "double_12_13": lane_stats(doubles_12),
        "ack_chunk_double": lane_stats(ack_doubles),
        "out_36": sum(1 for r in rows if r["dir"] == "OUT" and r["fields"]["len"] == 36),
        "out_16_ack_chunk": sum(
            1 for r in rows if r["dir"] == "OUT" and r["fields"].get("lane") == "ack_chunk"
        ),
        "out_16_keepalive": sum(
            1 for r in rows if r["dir"] == "OUT" and r["fields"].get("lane") == "keepalive"
        ),
        "in_272": sum(1 for r in rows if r["dir"] == "IN" and r["fields"]["len"] == 272),
    }


def print_timeline(path: Path, max_rows: int = 80, focus_64: bool = False):
    rows = analyze_file(path)
    print(f"\n=== Timeline {path.name} (max {max_rows}) ===")
    shown = 0
    for r in rows:
        f = r["fields"]
        d28 = f.get("double_28_29", "")
        d12 = f.get("double_12_13", "")
        if focus_64:
            u28 = f.get("u16_28_29")
            u12 = f.get("u16_12_13")
            if not (
                (u28 and ((u28 >> 8) == 0x64 or (u28 & 0xFF) == 0x64))
                or (u12 and ((u12 >> 8) == 0x64 or (u12 & 0xFF) == 0x64))
            ):
                continue
        sub = f.get("sub")
        cd = f.get("cd")
        extra = []
        if sub is not None:
            extra.append(f"sub={sub:02x}")
        if cd is not None:
            extra.append(f"cd={cd:02x}")
        if d28:
            extra.append(f"28-29={d28}")
        if d12:
            extra.append(f"12-13={d12}")
        if f.get("ack_double"):
            extra.append(f"ack={f['ack_double']}")
        print(
            f"  #{r['num']:5d} {r['rel']:8.3f}s {r['dir']:3s} "
            f"len={f['len']:3d} tag={f.get('tag','?')} {' '.join(extra)}"
        )
        shown += 1
        if shown >= max_rows:
            break


def dump_ack_sequences(path: Path, label: str):
    rows = analyze_file(path)
    acks = [
        r["fields"]["ack_double"]
        for r in rows
        if r["dir"] == "OUT" and r["fields"].get("lane") == "ack_chunk"
    ]
    d28 = [
        r["fields"].get("double_28_29")
        for r in rows
        if r["dir"] == "OUT"
        and r["fields"]["len"] == 36
        and r["fields"].get("cd") == 3
    ]
    print(f"\n--- {label} ACK chunk doubles ({len(acks)}): ---")
    print("  " + " ".join(acks[:40]))
    if len(acks) > 40:
        print("  ... +" + str(len(acks) - 40))
    print(f"--- {label} OUT36 cd=03 doubles ({len(d28)}): ---")
    print("  " + " ".join(d28))


def compare_pair(hx: Path, linux: Path):
    sh = summarize(hx)
    sl = summarize(linux)
    print(f"\n{'='*72}")
    print(f"COMPARE: {hx.name}  vs  {linux.name}")
    print(f"{'='*72}")
    keys = [
        "ed03_packets",
        "out_36",
        "out_16_ack_chunk",
        "out_16_keepalive",
        "in_272",
        "by_len",
        "by_sub",
        "double_28_29",
        "double_12_13",
        "ack_chunk_double",
    ]
    for k in keys:
        print(f"  {k}:")
        print(f"    HX:    {sh.get(k)}")
        print(f"    Linux: {sl.get(k)}")


def main(argv: list[str]) -> int:
    base = Path(__file__).resolve().parents[1] / "src/Paquets Json"
    pairs = [
        ("Start_Model_change.json", "Start_Model_change_Linux.json"),
    ]
    singles = ["Change_preset.json"]

    for hx_name, lx_name in pairs:
        hx = base / hx_name
        lx = base / lx_name
        if hx.exists() and lx.exists():
            compare_pair(hx, lx)
            print_timeline(hx, 60, focus_64=True)
            print_timeline(lx, 60, focus_64=True)
            dump_ack_sequences(hx, "HX")
            dump_ack_sequences(lx, "Linux")

    for name in singles:
        p = base / name
        if p.exists():
            print(f"\n--- Single: {name} ---")
            print(json.dumps(summarize(p), indent=2))
            print_timeline(p, 50, focus_64=False)

    # All JSON in folder — quick overview
    print(f"\n{'='*72}\nOVERVIEW all captures\n{'='*72}")
    for p in sorted(base.glob("*.json")):
        try:
            s = summarize(p)
            d28 = s["double_28_29"]
            ack = s["ack_chunk_double"]
            print(
                f"{p.name:42s} ed03={s['ed03_packets']:4d} "
                f"out36={s['out_36']:3d} ack08={s['out_16_ack_chunk']:3d} "
                f"in272={s['in_272']:3d} "
                f"28-29_n={d28['count']:3d} 64xx={d28['in_64xx_lane']:3d} "
                f"ack_n={ack['count']:3d}"
            )
        except Exception as e:
            print(f"{p.name}: ERROR {e}")

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
