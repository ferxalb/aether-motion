"""Generates the procedural textures of examples/textured-shaded-3d.html (brick, its normal map, runes, leaf).

Usage: python3 tools/make_example_textures.py examples/assets/textures
"""
import zlib, struct, math, random, sys
out = sys.argv[1]

def png(path, w, h, px):
    raw = b''.join(b'\x00' + bytes(c for p in px[y*w:(y+1)*w] for c in p) for y in range(h))
    def chunk(t, d):
        c = struct.pack('>I', len(d)) + t + d
        return c + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))

# Brick: 256x256, 4 rows of 4 bricks, offset every other row, mortar lines.
W = H = 256
rows, cols = 8, 4
bh, bw = H // rows, W // cols
mortar = 3
rnd = random.Random(7)
tint = {}
height = [[0.0] * W for _ in range(H)]
albedo = []
for y in range(H):
    r = y // bh
    off = (bw // 2) if r % 2 else 0
    for x in range(W):
        xx = (x + off) % W
        c = xx // bw
        inx, iny = xx % bw, y % bh
        if (r, c) not in tint:
            k = rnd.uniform(0.82, 1.12)
            tint[(r, c)] = k
        edge = min(inx, bw - 1 - inx, iny, bh - 1 - iny)
        if inx < mortar or iny < mortar:
            height[y][x] = 0.0
            n = rnd.randint(-6, 6)
            albedo.append((150 + n, 146 + n, 138 + n, 255))
        else:
            k = tint[(r, c)]
            grain = rnd.uniform(-10, 10)
            height[y][x] = min(1.0, 0.6 + 0.1 * min(edge, 4))
            albedo.append((int(min(255, max(0, 168 * k + grain))), int(min(255, max(0, 74 * k + grain))), int(min(255, max(0, 52 * k + grain))), 255))
png(out + '/brick.png', W, H, albedo)

# Normal map from the height field (OpenGL convention: +Y is up in the image).
normal = []
strength = 6.0
for y in range(H):
    for x in range(W):
        hl = height[y][(x - 1) % W]; hr = height[y][(x + 1) % W]
        hu = height[(y - 1) % H][x]; hd = height[(y + 1) % H][x]
        dx = (hl - hr) * strength
        dy = (hd - hu) * strength   # up in the image is -y, so a rise toward the top tilts +Y
        n = (dx, dy, 1.0)
        l = math.sqrt(sum(v * v for v in n))
        normal.append(tuple(int((v / l * 0.5 + 0.5) * 255 + 0.5) for v in n) + (255,))
png(out + '/brick-normal.png', W, H, normal)

# Runes: emissive glyph grid on black (128x128).
S = 128
rune = []
for y in range(S):
    for x in range(S):
        gx, gy = x % 32, y % 32
        on = (abs(gx - 16) < 2 and 6 < gy < 26) or (abs(gy - 16) < 2 and 6 < gx < 26) or (abs(gx - gy) < 2 and 8 < gx < 24 and (x // 32 + y // 32) % 2 == 0)
        rune.append((255, 170, 60, 255) if on else (0, 0, 0, 255))
png(out + '/runes.png', S, S, rune)

# Foliage-like cutout: opaque disc with transparent corners (64x64).
leaf = []
for y in range(64):
    for x in range(64):
        d = math.hypot(x - 31.5, (y - 31.5) * 0.8)
        a = 255 if d < 30 else 0
        g = 90 + int(60 * (1 - d / 32)) if d < 30 else 0
        leaf.append((40, max(0, g), 50, a))
png(out + '/leaf.png', 64, 64, leaf)
