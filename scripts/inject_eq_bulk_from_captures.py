#!/usr/bin/env python3
"""Shorthand: inject EQ mono/stereo bulkHex (delegates to inject_bulk_from_captures.py)."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/inject_bulk_from_captures.py"


def main() -> None:
    args = [
        sys.executable,
        str(SCRIPT),
        "EQ",
        f"mono:{ROOT / 'captures/usb-wireshark/eq Mono.json'}",
        f"stereo:{ROOT / 'captures/usb-wireshark/eq stereo.json'}",
    ]
    raise SystemExit(subprocess.call(args))


if __name__ == "__main__":
    main()
