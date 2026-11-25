# SPDX-License-Identifier: MIT
"""
Pure-NumPy + stdlib zlib procedural AA PBR Texture & Sky Dome generator for AETHER DRIFT v2.0.
Run via Blender 5.2's bundled Python (has numpy):
  "C:\Program Files\Blender Foundation\Blender 5.2\5.2\python\bin\python.exe" gen_textures.py

Outputs both viewable PNGs and compact deflate-compressed .tex (u32 w, u32 h, raw deflate RGBA):
  - track_albedo   (1024x1024) : Wet metallic asphalt deck, seams, bolted curb edges, tire streaks
  - track_puddle   (1024x1024) : Organic rain puddle mirror-reflection mask + damp sheen + micro-normals
  - billboard_atlas(1024x1024) : NEO-KYOTO, VORTEX ENERGY, CHRONOS TECN, AETHER-1 + Magenta/Cyan Chevrons
  - city_facade    (1024x1024) : Cyberpunk megacity illuminated skyscraper windows & vertical holo-panels
  - sky_twilight   (2048x1024) : Dramatic twilight storm-cloud sky dome with glowing sunset horizon
"""

import math
import os
import struct
import zlib
import numpy as np

OUT_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "assets", "textures"))
os.makedirs(OUT_DIR, exist_ok=True)


def save_png_and_tex(name, rgba: np.ndarray):
    h, w, c = rgba.shape
    assert c == 4 and rgba.dtype == np.uint8

    # 1. Save standard viewable PNG
    def chunk(tag, data):
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    raw_rows = b"".join(b"\x00" + rgba[y].tobytes() for y in range(h))
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw_rows, level=6))
        + chunk(b"IEND", b"")
    )
    png_path = os.path.join(OUT_DIR, f"{name}.png")
    with open(png_path, "wb") as f:
        f.write(png)

    # 2. Save compact .tex (u32 w, u32 h, zlib-compressed raw RGBA8) for instant Rust loading
    comp = zlib.compress(rgba.tobytes(), level=7)
    tex_path = os.path.join(OUT_DIR, f"{name}.tex")
    with open(tex_path, "wb") as f:
        f.write(struct.pack("<II", w, h))
        f.write(comp)
    print(f"[TEX] {name}: {w}x{h} -> .png ({len(png)//1024} KB), .tex ({len(comp)//1024} KB)")


def tileable_fbm(w, h, octaves=(4, 8, 16, 32), weights=(0.50, 0.26, 0.15, 0.09), seed=42):
    rng = np.random.default_rng(seed)
    acc = np.zeros((h, w), dtype=np.float32)
    y_grid, x_grid = np.mgrid[0:h, 0:w]
    for freq, wt in zip(octaves, weights):
        gw = max(2, freq)
        gh = max(2, int(freq * h / w))
        grid = rng.random((gh, gw), dtype=np.float32)
        fx = x_grid * (gw / w)
        fy = y_grid * (gh / h)
        x0 = np.floor(fx).astype(np.int32) % gw
        y0 = np.floor(fy).astype(np.int32) % gh
        x1 = (x0 + 1) % gw
        y1 = (y0 + 1) % gh
        tx = fx - np.floor(fx)
        ty = fy - np.floor(fy)
        tx = tx * tx * (3.0 - 2.0 * tx)
        ty = ty * ty * (3.0 - 2.0 * ty)
        val = (
            (grid[y0, x0] * (1 - tx) + grid[y0, x1] * tx) * (1 - ty)
            + (grid[y1, x0] * (1 - tx) + grid[y1, x1] * tx) * ty
        )
        acc += val * wt
    acc = (acc - acc.min()) / max(1e-5, acc.max() - acc.min())
    return acc


def blur_box(arr, radius=3):
    """Fast separable 2-pass box blur on float32 (H,W,C) array."""
    out = arr.copy()
    k = 2 * radius + 1
    for _ in range(2):
        # Horizontal
        cs = np.cumsum(np.pad(out, ((0, 0), (radius + 1, radius), (0, 0)), mode="edge"), axis=1)
        out = (cs[:, k:, :] - cs[:, :-k, :]) / k
        # Vertical
        cs = np.cumsum(np.pad(out, ((radius + 1, radius), (0, 0), (0, 0)), mode="edge"), axis=0)
        out = (cs[k:, :, :] - cs[:-k, :, :]) / k
    return out


# Clean geometric 5x7 futuristic block font for crisp LED billboards
FONT_5X7 = {
    "A": ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
    "C": ["01111", "10000", "10000", "10000", "10000", "10000", "01111"],
    "E": ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
    "G": ["01111", "10000", "10000", "10111", "10001", "10001", "01110"],
    "H": ["10001", "10001", "10001", "11111", "10001", "10001", "10001"],
    "I": ["11111", "00100", "00100", "00100", "00100", "00100", "11111"],
    "K": ["10001", "10010", "10100", "11000", "10100", "10010", "10001"],
    "L": ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
    "N": ["10001", "11001", "10101", "10011", "10001", "10001", "10001"],
    "O": ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    "P": ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
    "R": ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
    "S": ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
    "T": ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
    "U": ["10001", "10001", "10001", "10001", "10001", "10001", "01110"],
    "V": ["10001", "10001", "10001", "10001", "10001", "01010", "00100"],
    "X": ["10001", "10001", "01010", "00100", "01010", "10001", "10001"],
    "Y": ["10001", "10001", "01010", "00100", "00100", "00100", "00100"],
    "1": ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
    "2": ["01110", "10001", "00001", "00110", "01000", "10000", "11111"],
    "0": ["01110", "10011", "10101", "10101", "11001", "10001", "01110"],
    "9": ["01110", "10001", "10001", "01111", "00001", "00010", "01100"],
    "-": ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
    " ": ["00000", "00000", "00000", "00000", "00000", "00000", "00000"],
}


def draw_text(buf, x0, y0, text, sx, sy, col):
    cx = x0
    for ch in text.upper():
        g = FONT_5X7.get(ch, FONT_5X7[" "])
        for r, row in enumerate(g):
            for c, bit in enumerate(row):
                if bit == "1":
                    px0 = int(cx + c * sx)
                    py0 = int(y0 + r * sy)
                    px1 = int(px0 + sx)
                    py1 = int(py0 + sy)
                    buf[max(0, py0):min(buf.shape[0], py1), max(0, px0):min(buf.shape[1], px1), :3] = col
        cx += 6 * sx


def draw_thick_line(buf, x0, y0, x1, y1, thickness, col):
    steps = int(max(abs(x1 - x0), abs(y1 - y0)) * 2) + 2
    ts = np.linspace(0.0, 1.0, steps)
    r = max(1, int(thickness * 0.5))
    H, W, _ = buf.shape
    for t in ts:
        x = int(x0 + (x1 - x0) * t)
        y = int(y0 + (y1 - y0) * t)
        buf[max(0, y - r):min(H, y + r + 1), max(0, x - r):min(W, x + r + 1), :3] = col


def gen_track_textures():
    S = 1024
    fine = tileable_fbm(S, S, (8, 16, 32, 64, 128), (0.35, 0.25, 0.18, 0.12, 0.10), seed=101)
    streaks = tileable_fbm(S, S, (32, 64), (0.6, 0.4), seed=202)
    streaks_v = np.repeat(np.mean(streaks.reshape(S // 8, 8, S), axis=1), 8, axis=0)

    r = 0.08 + fine * 0.065 + streaks_v * 0.03
    g = 0.10 + fine * 0.075 + streaks_v * 0.035
    b = 0.15 + fine * 0.095 + streaks_v * 0.045

    # Horizontal titanium expansion joints + specular bevel edge
    for y in range(0, S, 256):
        r[y:y+5, :] *= 0.30
        g[y:y+5, :] *= 0.30
        b[y:y+5, :] *= 0.34
        r[y+5:y+9, :] += 0.07
        g[y+5:y+9, :] += 0.09
        b[y+5:y+9, :] += 0.12

    # Longitudinal metal plate seams
    for x in [96, 320, 512, 704, 928]:
        r[:, x-2:x+2] *= 0.42
        g[:, x-2:x+2] *= 0.42
        b[:, x-2:x+2] *= 0.46

    # Shoulder neon guide trim painted into deck edge
    r[:, 22:42] = r[:, 22:42] * 0.35 + 0.16 * 0.65
    g[:, 22:42] = g[:, 22:42] * 0.35 + 0.85 * 0.65
    b[:, 22:42] = b[:, 22:42] * 0.35 + 0.96 * 0.65
    r[:, S-42:S-22] = r[:, S-42:S-22] * 0.35 + 0.90 * 0.65
    g[:, S-42:S-22] = g[:, S-42:S-22] * 0.35 + 0.24 * 0.65
    b[:, S-42:S-22] = b[:, S-42:S-22] * 0.35 + 0.76 * 0.65

    # Center dashed racing guide line
    for y0 in range(32, S, 128):
        r[y0:y0+58, 505:519] = 0.70
        g[y0:y0+58, 505:519] = 0.88
        b[y0:y0+58, 505:519] = 0.98

    # Polished twin wet hover-wake grooves
    xs = np.linspace(-1.0, 1.0, S, dtype=np.float32)[None, :]
    groove = np.exp(-((np.abs(xs) - 0.32) / 0.18) ** 2) * 0.18
    r = np.clip(r - groove * 0.14, 0.0, 1.0)
    g = np.clip(g - groove * 0.11, 0.0, 1.0)
    b = np.clip(b - groove * 0.07, 0.0, 1.0)

    albedo = np.stack([
        (r * 255).astype(np.uint8),
        (g * 255).astype(np.uint8),
        (b * 255).astype(np.uint8),
        np.full((S, S), 255, dtype=np.uint8),
    ], axis=-1)
    save_png_and_tex("track_albedo", albedo)

    # Organic rain puddle reflection mask (R = mirror puddle intensity, G = damp sheen, B = normal_x, A = normal_y)
    macro = tileable_fbm(S, S, (3, 6, 12, 24), (0.48, 0.28, 0.16, 0.08), seed=303)
    puddle = np.clip((macro - 0.32) * 2.8, 0.0, 1.0)
    puddle = puddle * puddle * (3.0 - 2.0 * puddle)
    puddle = np.clip(puddle * 0.80 + groove * 1.55 + 0.22, 0.0, 1.0)
    damp = np.clip(0.38 + fine * 0.48, 0.0, 1.0)

    gx = np.roll(fine, -1, axis=1) - np.roll(fine, 1, axis=1)
    gy = np.roll(fine, -1, axis=0) - np.roll(fine, 1, axis=0)
    rough_scale = (1.0 - puddle * 0.85) * 3.5
    nx = np.clip(0.5 + gx * rough_scale, 0.0, 1.0)
    ny = np.clip(0.5 + gy * rough_scale, 0.0, 1.0)

    puddle_img = np.stack([
        (puddle * 255).astype(np.uint8),
        (damp * 255).astype(np.uint8),
        (nx * 255).astype(np.uint8),
        (ny * 255).astype(np.uint8),
    ], axis=-1)
    save_png_and_tex("track_puddle", puddle_img)


def gen_billboard_atlas():
    W, H = 1024, 1024
    buf = np.zeros((H, W, 4), dtype=np.float32)
    buf[:, :, 3] = 1.0

    def fill_panel(x0, y0, pw, ph, c0, c1, border):
        ys = np.linspace(0.0, 1.0, ph, dtype=np.float32)[:, None, None]
        grad = np.array(c0, dtype=np.float32)[None, None, :] * (1 - ys) + np.array(c1, dtype=np.float32)[None, None, :] * ys
        buf[y0:y0+ph, x0:x0+pw, :3] = grad
        # Subtle LED scanlines
        buf[y0:y0+ph:6, x0:x0+pw, :3] *= 0.82
        # Crisp glowing neon frame border
        b = 6
        buf[y0:y0+b, x0:x0+pw, :3] = border
        buf[y0+ph-b:y0+ph, x0:x0+pw, :3] = border
        buf[y0:y0+ph, x0:x0+b, :3] = border
        buf[y0:y0+ph, x0+pw-b:x0+pw, :3] = border

    # Row 0 Left: NEO-KYOTO
    fill_panel(0, 0, 512, 256, (0.07, 0.09, 0.28), (0.24, 0.09, 0.38), (0.40, 0.92, 1.0))
    # Glowing geometric Fuji mountain silhouette
    for y in range(110, 248):
        span = int((y - 110) * 1.15)
        cx = 256
        buf[y, max(8, cx - span):min(504, cx + span), :3] = (0.15, 0.11, 0.34)
    draw_thick_line(buf, 100, 246, 256, 110, 5, (0.36, 0.90, 1.0))
    draw_thick_line(buf, 256, 110, 412, 246, 5, (0.95, 0.34, 0.84))
    draw_text(buf, 42, 76, "NEO-KYOTO", 8, 9, (0.92, 0.98, 1.0))
    draw_text(buf, 96, 168, "HYPER-CIRCUIT 2099", 3, 4, (0.45, 0.95, 1.0))

    # Row 0 Right: VORTEX ENERGY
    fill_panel(512, 0, 512, 256, (0.04, 0.14, 0.25), (0.06, 0.26, 0.36), (0.22, 0.96, 0.92))
    # Cyan Vortex Emblem
    cx, cy = 594, 128
    draw_thick_line(buf, cx - 40, cy - 42, cx + 40, cy - 42, 7, (0.25, 1.0, 0.96))
    draw_thick_line(buf, cx - 40, cy - 42, cx, cy + 48, 7, (0.25, 1.0, 0.96))
    draw_thick_line(buf, cx + 40, cy - 42, cx, cy + 48, 7, (0.25, 1.0, 0.96))
    draw_text(buf, 668, 62, "VORTEX", 9, 9, (0.94, 1.0, 1.0))
    draw_text(buf, 672, 146, "ENERGY", 7, 7, (0.35, 0.98, 0.95))

    # Row 1 Left: CHRONOS TECN
    fill_panel(0, 256, 512, 256, (0.11, 0.07, 0.24), (0.26, 0.09, 0.36), (0.85, 0.42, 1.0))
    hx, hy = 82, 384
    pts = [(hx + 42 * math.cos(math.radians(60 * k)), hy + 42 * math.sin(math.radians(60 * k))) for k in range(6)]
    for k in range(6):
        draw_thick_line(buf, pts[k][0], pts[k][1], pts[(k + 1) % 6][0], pts[(k + 1) % 6][1], 6, (0.90, 0.50, 1.0))
    draw_text(buf, 148, 318, "CHRONOS", 8, 8, (0.96, 0.94, 1.0))
    draw_text(buf, 152, 400, "TECN", 9, 8, (0.92, 0.48, 1.0))

    # Row 1 Right: AETHER-1
    fill_panel(512, 256, 512, 256, (0.04, 0.10, 0.22), (0.10, 0.24, 0.42), (0.35, 0.88, 1.0))
    draw_text(buf, 556, 326, "AETHER-1", 9, 9, (0.92, 0.98, 1.0))
    draw_text(buf, 560, 412, "ANTI-GRAVITY LEAGUE", 3, 4, (0.45, 0.92, 1.0))

    # Row 2 (512..768): MAGENTA Turn Chevrons <<<<<<<<
    fill_panel(0, 512, 1024, 256, (0.14, 0.04, 0.22), (0.24, 0.06, 0.34), (0.98, 0.38, 1.0))
    for k in range(8):
        bx = 40 + k * 124
        draw_thick_line(buf, bx + 86, 550, bx + 26, 640, 22, (1.0, 0.56, 1.0))
        draw_thick_line(buf, bx + 26, 640, bx + 86, 730, 22, (1.0, 0.56, 1.0))

    # Row 3 (768..1024): CYAN Turn Chevrons <<<<<<<<
    fill_panel(0, 768, 1024, 256, (0.03, 0.14, 0.22), (0.05, 0.24, 0.32), (0.28, 1.0, 0.96))
    for k in range(8):
        bx = 40 + k * 124
        draw_thick_line(buf, bx + 86, 806, bx + 26, 896, 22, (0.55, 1.0, 0.98))
        draw_thick_line(buf, bx + 26, 896, bx + 86, 986, 22, (0.55, 1.0, 0.98))

    # Soft LED bloom halo baked into texture
    glow = blur_box(buf, radius=5)
    final = np.clip(buf + glow * 0.42, 0.0, 1.0)
    final[:, :, 3] = 1.0
    save_png_and_tex("billboard_atlas", (final * 255).astype(np.uint8))


def gen_city_facade():
    S = 1024
    rng = np.random.default_rng(777)
    arr = np.zeros((S, S, 4), dtype=np.uint8)
    arr[:, :, 0] = 14
    arr[:, :, 1] = 20
    arr[:, :, 2] = 35
    arr[:, :, 3] = 90  # alpha encodes emissive mask (<128 = dark cladding, 255 = glowing window)

    for y in range(0, S, 32):
        arr[y:y+5, :, :3] = (8, 12, 22)
    for x in range(0, S, 64):
        arr[:, x:x+6, :3] = (9, 13, 24)

    for row in range(0, S, 32):
        floor_lit = rng.random() < 0.74
        for col in range(0, S, 16):
            if col % 64 < 6:
                continue
            if floor_lit and rng.random() < 0.54:
                pick = rng.random()
                if pick < 0.46:
                    c = (242, 198, 132)
                elif pick < 0.82:
                    c = (130, 228, 255)
                else:
                    c = (240, 118, 224)
                bright = rng.uniform(0.48, 1.0)
                c = tuple(int(v * bright) for v in c)
                arr[row+8:row+25, col+2:col+13, 0] = c[0]
                arr[row+8:row+25, col+2:col+13, 1] = c[1]
                arr[row+8:row+25, col+2:col+13, 2] = c[2]
                arr[row+8:row+25, col+2:col+13, 3] = 255

    # Vertical glowing cyberpunk neon holo-signs on tower walls
    for sx, col in [(140, (65, 245, 255)), (520, (255, 85, 205)), (840, (255, 185, 70))]:
        arr[120:460, sx:sx+34, 0] = col[0]
        arr[120:460, sx:sx+34, 1] = col[1]
        arr[120:460, sx:sx+34, 2] = col[2]
        arr[120:460, sx:sx+34, 3] = 255

    save_png_and_tex("city_facade", arr)


def gen_sky_twilight():
    W, H = 2048, 1024
    y = np.linspace(0.0, 1.0, H, dtype=np.float32)[:, None]
    x = np.linspace(0.0, 1.0, W, dtype=np.float32)[None, :]

    t_sky = np.clip(y / 0.52, 0.0, 1.0)
    r = 0.05 + 0.16 * (t_sky ** 1.4) + 0.42 * np.exp(-((y - 0.50) / 0.075) ** 2)
    g = 0.06 + 0.11 * (t_sky ** 1.5) + 0.19 * np.exp(-((y - 0.50) / 0.065) ** 2)
    b = 0.16 + 0.24 * (t_sky ** 1.1) + 0.16 * np.exp(-((y - 0.46) / 0.11) ** 2)

    az_glow = 0.5 + 0.5 * np.cos((x - 0.28) * 2.0 * math.pi)
    horizon_band = np.exp(-((y - 0.49) / 0.085) ** 2) * (az_glow ** 1.8)
    r = r + horizon_band * 0.48
    g = g + horizon_band * 0.24
    b = b + horizon_band * 0.08

    c1 = tileable_fbm(W, H, (4, 8, 16, 32, 64), (0.42, 0.26, 0.16, 0.10, 0.06), seed=909)
    c2 = tileable_fbm(W, H, (6, 12, 24, 48), (0.45, 0.28, 0.17, 0.10), seed=911)
    cloud_mask = np.clip((c1 * 0.65 + c2 * 0.35 - 0.26) * 1.95, 0.0, 1.0)
    above = np.clip((0.55 - y) * 10.0, 0.0, 1.0)
    cloud_mask *= above

    rim = np.clip((c2 - c1 + 0.26) * 1.85, 0.0, 1.0) * above
    cr = 0.11 + rim * (0.28 + horizon_band * 0.68)
    cg = 0.10 + rim * (0.18 + horizon_band * 0.36)
    cb = 0.23 + rim * (0.35 + horizon_band * 0.12)

    r = r * (1.0 - cloud_mask * 0.68) + cr * (cloud_mask * 0.68)
    g = g * (1.0 - cloud_mask * 0.68) + cg * (cloud_mask * 0.68)
    b = b * (1.0 - cloud_mask * 0.68) + cb * (cloud_mask * 0.68)

    below = np.clip((y - 0.51) / 0.18, 0.0, 1.0)
    r = r * (1.0 - below) + (0.04 + c1 * 0.04) * below
    g = g * (1.0 - below) + (0.08 + c1 * 0.06) * below
    b = b * (1.0 - below) + (0.15 + c1 * 0.09) * below

    sky = np.stack([
        (np.clip(r, 0.0, 1.0) * 255).astype(np.uint8),
        (np.clip(g, 0.0, 1.0) * 255).astype(np.uint8),
        (np.clip(b, 0.0, 1.0) * 255).astype(np.uint8),
        np.full((H, W), 255, dtype=np.uint8),
    ], axis=-1)
    save_png_and_tex("sky_twilight", sky)


if __name__ == "__main__":
    gen_track_textures()
    gen_billboard_atlas()
    gen_city_facade()
    gen_sky_twilight()
    print("[TEX] All PBR textures & sky dome generated!")
