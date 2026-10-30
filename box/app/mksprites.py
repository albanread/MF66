#!/usr/bin/env python3
"""mksprites.py -- !F66's sprites: the application (!f66, sm!f66) and the
Forth source file (file_a77, small_a77), drawn here as 32 bpp sprites with
1 bpp masks at 90 dpi (new-format mode word, type 6).

    python3 box/app/mksprites.py      writes box/app/!F66/!Sprites,ff9
"""
import os
import struct

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "!F66", "!Sprites,ff9")

# 5x7 glyphs, rows top to bottom
FONT = {
    "F": ["11111", "10000", "10000", "11110", "10000", "10000", "10000"],
    "6": ["00110", "01000", "10000", "11110", "10001", "10001", "01110"],
    "4": ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
}

BLUE, DARK, WHITE, YELLOW, PAGE, EDGE = (
    (30, 60, 150), (10, 20, 60), (255, 255, 255), (255, 210, 40), (250, 250, 240), (90, 90, 90))


def canvas(w, h):
    return [[None] * w for _ in range(h)]


def text(c, s, x, y, scale, col):
    for ch in s:
        for r, row in enumerate(FONT[ch]):
            for k, bit in enumerate(row):
                if bit == "1":
                    for dy in range(scale):
                        for dx in range(scale):
                            c[y + r * scale + dy][x + k * scale + dx] = col
        x += 5 * scale + 1


def app_icon(n):
    c = canvas(n, n)
    r = n // 6
    for y in range(n):
        for x in range(n):
            # rounded square: outside the corner circles is transparent
            cx = min(max(x, r), n - 1 - r)
            cy = min(max(y, r), n - 1 - r)
            d = (x - cx) ** 2 + (y - cy) ** 2
            if d <= r * r:
                edge = d > (r - 1.5) ** 2 or x in (0, n - 1) or y in (0, n - 1)
                c[y][x] = DARK if edge else BLUE
    if n >= 34:
        text(c, "F66", 1, 9, 2, WHITE)
        for x in range(4, n - 4):          # a yellow underline: the prompt
            c[27][x] = c[28][x] = YELLOW
    else:
        text(c, "F", 6, 5, 1, WHITE)
    return c


def file_icon(n):
    c = canvas(n, n)
    x0, x1, fold = n // 8, n - n // 8, n // 4
    for y in range(n):
        for x in range(x0, x1):
            if x > x1 - 1 - fold + y:      # the folded corner
                continue
            edge = x in (x0, x1 - 1) or y in (0, n - 1) or x == x1 - 1 - fold + y
            c[y][x] = EDGE if edge else PAGE
    if n >= 34:
        text(c, "F", 6, 12, 2, BLUE)
        text(c, "4", 18, 12, 2, BLUE)
    else:
        text(c, "F", 6, 5, 1, BLUE)
    return c


def sprite(name, c):
    h, w = len(c), len(c[0])
    mode = (6 << 27) | (90 << 14) | (90 << 1) | 1
    img = b"".join(struct.pack("<I", 0 if p is None else (p[2] << 16) | (p[1] << 8) | p[0])
                   for row in c for p in row)
    mwords = (w + 31) // 32
    mask = b""
    for row in c:
        bits = 0
        for x, p in enumerate(row):
            if p is not None:
                bits |= 1 << x
        mask += bits.to_bytes(mwords * 4, "little")
    hdr = 44
    size = hdr + len(img) + len(mask)
    return struct.pack("<I12siiiiiii", size, name.encode().ljust(12, b"\0"), w - 1, h - 1, 0, 31,
                       hdr, hdr + len(img), mode) + img + mask


def main():
    sprites = [sprite("!f66", app_icon(34)), sprite("sm!f66", app_icon(17)),
               sprite("file_a77", file_icon(34)), sprite("small_a77", file_icon(17))]
    body = b"".join(sprites)
    area = struct.pack("<iii", len(sprites), 16, 16 + len(body)) + body
    with open(OUT, "wb") as f:
        f.write(area)
    print(f"mksprites: {OUT} ({len(area)} bytes, {len(sprites)} sprites)")


if __name__ == "__main__":
    main()
