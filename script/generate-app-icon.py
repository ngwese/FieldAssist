#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Greg Wuller
# SPDX-License-Identifier: MIT
"""Rasterize assets/logo/04-bands.svg into Windows .ico and macOS .icns.

Requires: pip install resvg_py
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

try:
    import resvg_py
except ImportError:
    sys.stderr.write("error: resvg_py is required (pip install resvg_py)\n")
    sys.exit(1)

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "crates" / "field-assist" / "assets"
SVG_PATH = ASSETS / "logo" / "04-bands.svg"
OUT_DIR = ASSETS / "app-icon"

ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
# ostype -> pixel size. PNG payloads, modern macOS.
ICNS_TYPES = (
    (b"ic11", 32),  # 16@2x
    (b"ic12", 64),  # 32@2x
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic13", 256),  # 128@2x
    (b"ic09", 512),
    (b"ic14", 512),  # 256@2x
    (b"ic10", 1024),
)


def render_png(svg: str, size: int) -> bytes:
    png = resvg_py.svg_to_bytes(svg_string=svg, width=size, height=size)
    if not png:
        raise RuntimeError(f"resvg_py returned no PNG for {size}px")
    return png


def write_ico(path: Path, images: dict[int, bytes]) -> None:
    count = len(ICO_SIZES)
    offset = 6 + 16 * count
    entries = b""
    blobs = b""
    for size in ICO_SIZES:
        data = images[size]
        width = 0 if size >= 256 else size
        height = 0 if size >= 256 else size
        entries += struct.pack(
            "<BBBBHHII",
            width,
            height,
            0,
            0,
            1,
            32,
            len(data),
            offset,
        )
        blobs += data
        offset += len(data)
    path.write_bytes(struct.pack("<HHH", 0, 1, count) + entries + blobs)


def write_icns(path: Path, images: dict[int, bytes]) -> None:
    chunks = b""
    for ostype, size in ICNS_TYPES:
        data = images[size]
        chunks += ostype + struct.pack(">I", 8 + len(data)) + data
    path.write_bytes(b"icns" + struct.pack(">I", 8 + len(chunks)) + chunks)


def main() -> None:
    svg = SVG_PATH.read_text(encoding="utf-8")
    sizes = sorted(set(ICO_SIZES) | {size for _, size in ICNS_TYPES} | {512})
    images = {size: render_png(svg, size) for size in sizes}

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    ico = OUT_DIR / "app-icon.ico"
    icns = OUT_DIR / "AppIcon.icns"
    png = OUT_DIR / "app-icon.png"
    write_ico(ico, images)
    write_icns(icns, images)
    png.write_bytes(images[512])
    print(f"wrote {ico.relative_to(ROOT)}")
    print(f"wrote {icns.relative_to(ROOT)}")
    print(f"wrote {png.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
