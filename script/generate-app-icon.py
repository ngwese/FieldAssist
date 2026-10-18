#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Greg Wuller
# SPDX-License-Identifier: MIT
"""Resize field-assist-icon2.png into Windows .ico, macOS .icns, and a 512px PNG.

Requires: pip install pillow
"""

from __future__ import annotations

import io
import struct
import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    sys.stderr.write("error: pillow is required (pip install pillow)\n")
    sys.exit(1)

ROOT = Path(__file__).resolve().parent.parent
OUT_DIR = ROOT / "crates" / "field-assist" / "assets" / "app-icon"
SOURCE = OUT_DIR / "field-assist-icon2.png"

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


def render_png(image: Image.Image, size: int) -> bytes:
    if image.size == (size, size):
        resized = image
    else:
        resized = image.resize((size, size), Image.Resampling.LANCZOS)
    buffer = io.BytesIO()
    resized.save(buffer, format="PNG", optimize=True)
    return buffer.getvalue()


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
    if not SOURCE.is_file():
        sys.stderr.write(f"error: missing {SOURCE.relative_to(ROOT)}\n")
        sys.exit(1)

    with Image.open(SOURCE) as opened:
        image = opened.convert("RGBA")
    if image.width != image.height:
        sys.stderr.write(
            f"error: {SOURCE.name} must be square, got {image.width}x{image.height}\n"
        )
        sys.exit(1)

    sizes = sorted(set(ICO_SIZES) | {size for _, size in ICNS_TYPES} | {512})
    images = {size: render_png(image, size) for size in sizes}

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
