#!/usr/bin/env python3
"""rgba2png.py <file-<size>.rgba>... : raw square RGBA -> PNG (stdlib only)"""
import re, struct, sys, zlib

def png(path):
    size = int(re.search(r"-(\d+)\.rgba$", path).group(1))
    raw = open(path, "rb").read()
    rows = b"".join(b"\x00" + raw[y * size * 4:(y + 1) * size * 4] for y in range(size))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
    out = path[:-5] + ".png"
    open(out, "wb").write(data)
    print(out)

for p in sys.argv[1:]:
    png(p)
