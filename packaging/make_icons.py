#!/usr/bin/env python3
"""Draws the Apeiron icon: an open ring, for what has no bound.

Usage: make_icons.py <folder>. Writes icon-<size>.png and apeiron.ico there.
It needs nothing but Python, so that no drawing tool or binary file is kept
in the repository, and gives the same bytes every time.
"""

import math
import struct
import sys
import zlib
from pathlib import Path

BACKGROUND = (31, 78, 121)
RING = (255, 255, 255)


def covered(x, y, size):
    """How much of a pixel the shape covers, for the corners and the ring."""
    n = 4
    background = ring = 0
    for i in range(n):
        for j in range(n):
            u = (x + (i + 0.5) / n) / size
            v = (y + (j + 0.5) / n) / size
            # A square with rounded corners.
            r = 0.2
            dx = max(abs(u - 0.5) - (0.5 - r), 0)
            dy = max(abs(v - 0.5) - (0.5 - r), 0)
            if dx * dx + dy * dy <= r * r:
                background += 1
                # A ring with an opening at the upper right.
                cx, cy = u - 0.5, v - 0.5
                distance = math.hypot(cx, cy)
                angle = math.degrees(math.atan2(-cy, cx)) % 360
                if 0.2 <= distance <= 0.32 and not 25 <= angle <= 65:
                    ring += 1
    return background / (n * n), ring / (n * n)


def png(size):
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            background, ring = covered(x, y, size)
            mix = ring / background if background else 0
            colour = [round(b + (r - b) * mix) for b, r in zip(BACKGROUND, RING)]
            rows.extend(colour + [round(background * 255)])

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
        + chunk(b"IEND", b"")
    )


def ico(images):
    """An icon file holding PNG images, as Windows reads since Vista."""
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for size, data in images:
        side = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    return out + b"".join(data for _, data in images)


def main():
    folder = Path(sys.argv[1])
    folder.mkdir(parents=True, exist_ok=True)
    images = {}
    for size in (16, 32, 48, 64, 128, 256, 512, 1024):
        images[size] = png(size)
        (folder / f"icon-{size}.png").write_bytes(images[size])
    (folder / "apeiron.ico").write_bytes(ico([(s, images[s]) for s in (16, 32, 48, 256)]))


if __name__ == "__main__":
    main()
