#!/usr/bin/env python3
"""Generate Wynamp's original icon set (no external dependencies).

Design: phosphor-green play triangle on a dark rounded square — the app's
own retro mark, not derived from any existing player's assets.

Outputs (into app/icons/):
  32x32.png, 128x128.png, 128x128@2x.png, icon.png   — Tauri defaults
  icon.ico                                            — Windows (PNG-in-ICO)
  icon.icns                                           — macOS (via iconutil)
  iconset/*.png                                       — icns source of truth

Usage: python3 scripts/generate_icons.py
"""

from __future__ import annotations

import struct
import subprocess
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "app" / "icons"
ICONSET = ICONS / "iconset.iconset"

BG = (27, 27, 31, 255)        # --bg  #1b1b1f
BORDER = (58, 58, 66, 255)    # panel edge
GREEN = (57, 255, 20, 255)    # --accent #39ff14

SIZES = [16, 32, 64, 128, 256, 512, 1024]


def in_rounded_rect(x: int, y: int, w: int, h: int, inset: int, radius: float) -> bool:
    """Pixel inside rounded rectangle, measured from the pixel's top-left."""
    left, top = inset, inset
    right, bottom = w - 1 - inset, h - 1 - inset
    if x < left or x > right or y < top or y > bottom:
        return False
    r = radius
    # Corner circle checks
    if x < left + r and y < top + r:
        return (x - (left + r)) ** 2 + (y - (top + r)) ** 2 <= r * r
    if x > right - r and y < top + r:
        return (x - (right - r)) ** 2 + (y - (top + r)) ** 2 <= r * r
    if x < left + r and y > bottom - r:
        return (x - (left + r)) ** 2 + (y - (bottom - r)) ** 2 <= r * r
    if x > right - r and y > bottom - r:
        return (x - (right - r)) ** 2 + (y - (bottom - r)) ** 2 <= r * r
    return True


def in_triangle(px: float, py: float) -> bool:
    """Point-in-triangle for the play mark (normalized 0..1 coordinates)."""
    ax, ay = 0.36, 0.26   # top-left
    bx, by = 0.36, 0.74   # bottom-left
    cx, cy = 0.76, 0.50   # tip (right)

    def sign(x1: float, y1: float, x2: float, y2: float, x3: float, y3: float) -> float:
        return (x1 - x3) * (y2 - y3) - (x2 - x3) * (y1 - y3)

    d1 = sign(px, py, ax, ay, bx, by)
    d2 = sign(px, py, bx, by, cx, cy)
    d3 = sign(px, py, cx, cy, ax, ay)
    has_neg = d1 < 0 or d2 < 0 or d3 < 0
    has_pos = d1 > 0 or d2 > 0 or d3 > 0
    return not (has_neg and has_pos)


def render(size: int) -> bytes:
    """Render one icon as raw RGBA bytes."""
    radius = max(2.0, size * 0.18)
    border = max(1, size // 24)
    px = bytearray()
    for y in range(size):
        for x in range(size):
            if not in_rounded_rect(x, y, size, size, 0, radius):
                px.extend((0, 0, 0, 0))  # transparent corner
                continue
            # Play triangle (drawn only inside the inner face)
            nx, ny = (x + 0.5) / size, (y + 0.5) / size
            inner = in_rounded_rect(x, y, size, size, border, max(1.0, radius - border))
            if inner and in_triangle(nx, ny):
                px.extend(GREEN)
            elif not inner:
                px.extend(BORDER)
            else:
                px.extend(BG)
    return bytes(px)


def png_bytes(size: int, rgba: bytes) -> bytes:
    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    raw = b"".join(b"\x00" + rgba[y * size * 4 : (y + 1) * size * 4] for y in range(size))
    idat = zlib.compress(raw, 9)
    return sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def write_png(path: Path, size: int) -> None:
    path.write_bytes(png_bytes(size, render(size)))
    print(f"  wrote {path.relative_to(ROOT)} ({size}x{size})")


def write_ico(path: Path) -> None:
    """ICO container with PNG-compressed entries (valid for 16..256 px)."""
    sizes = [16, 32, 64, 256]
    blobs = [png_bytes(s, render(s)) for s in sizes]
    header = struct.pack("<HHH", 0, 1, len(sizes))
    offset = len(header) + 16 * len(sizes)
    entries = b""
    for s, blob in zip(sizes, blobs):
        dim = 0 if s >= 256 else s
        entries += struct.pack(
            "<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset
        )
        offset += len(blob)
    path.write_bytes(header + entries + b"".join(blobs))
    print(f"  wrote {path.relative_to(ROOT)} (sizes {sizes})")


def write_icns() -> None:
    """Build icon.icns from a .iconset via the macOS iconutil tool."""
    ICONSET.mkdir(parents=True, exist_ok=True)
    mapping = [
        (16, "icon_16x16"),
        (32, "icon_16x16@2x"),
        (32, "icon_32x32"),
        (64, "icon_32x32@2x"),
        (128, "icon_128x128"),
        (256, "icon_128x128@2x"),
        (256, "icon_256x256"),
        (512, "icon_256x256@2x"),
        (512, "icon_512x512"),
        (1024, "icon_512x512@2x"),
    ]
    for size, name in mapping:
        write_png(ICONSET / f"{name}.png", size)
    icns = ICONS / "icon.icns"
    if icns.exists():
        icns.unlink()
    result = subprocess.run(
        ["iconutil", "-c", "icns", str(ICONSET), "-o", str(icns)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f"iconutil failed: {result.stderr.strip()}", file=sys.stderr)
        sys.exit(1)
    print(f"  wrote {icns.relative_to(ROOT)}")


def main() -> None:
    ICONS.mkdir(parents=True, exist_ok=True)
    print("Generating Wynamp icons…")
    write_png(ICONS / "32x32.png", 32)
    write_png(ICONS / "128x128.png", 128)
    write_png(ICONS / "128x128@2x.png", 256)
    write_png(ICONS / "icon.png", 512)
    write_ico(ICONS / "icon.ico")
    write_icns()
    print("Done.")


if __name__ == "__main__":
    main()
