#!/usr/bin/env python3
"""Render the sample photographs the release screenshot is taken with.

    python3 tools/screenshot/make_samples.py

The JPEGs next to this script are committed output: the screenshot workflow
copies them into the folder the harness opens, so the sheet shows a shoot
instead of an empty directory. Everything is drawn here from scratch, so the
set carries no third-party licence. Run this again only to change the set, and
keep it small: the sheet shows them at thumbnail size.
"""

import math
import os
import random

from PIL import Image, ImageChops, ImageDraw, ImageFilter
from PIL.ExifTags import IFD

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "samples")


def vgrad(w, h, stops):
    """Vertical gradient with smoothstep between (position, rgb) stops."""
    col = Image.new("RGB", (1, h))
    px = col.load()
    for y in range(h):
        t = y / (h - 1)
        for i in range(len(stops) - 1):
            t0, c0 = stops[i]
            t1, c1 = stops[i + 1]
            if t <= t1 or i == len(stops) - 2:
                f = min(max((t - t0) / max(t1 - t0, 1e-6), 0), 1)
                f = f * f * (3 - 2 * f)
                px[0, y] = tuple(round(c0[k] + (c1[k] - c0[k]) * f) for k in range(3))
                break
    return col.resize((w, h))


def ramp(w, h, y0, y1):
    """L mask: 255 above y0 fading to 0 at y1."""
    m = Image.new("L", (1, h))
    px = m.load()
    for y in range(h):
        t = min(max((y - y0) / max(y1 - y0, 1e-6), 0), 1)
        px[0, y] = round(255 * (1 - t))
    return m.resize((w, h))


def poly_mask(w, h, pts, soft=1.0):
    m = Image.new("L", (w, h), 0)
    ImageDraw.Draw(m).polygon(pts, fill=255)
    return m.filter(ImageFilter.GaussianBlur(soft)) if soft else m


def glow(img, cx, cy, r, color, blur=None):
    w, h = img.size
    layer = Image.new("RGB", (w, h), (0, 0, 0))
    ImageDraw.Draw(layer).ellipse((cx - r, cy - r, cx + r, cy + r), fill=color)
    return ImageChops.screen(img, layer.filter(ImageFilter.GaussianBlur(blur or r * 0.5)))


def finish(img, vig=0.26):
    """Vignette and a little grain, so the tiles read as photographs."""
    w, h = img.size
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).ellipse((-w * 0.22, -h * 0.30, w * 1.22, h * 1.30), fill=255)
    mask = mask.filter(ImageFilter.GaussianBlur(min(w, h) * 0.16))
    dark = Image.new("RGB", (w, h), (0, 0, 0))
    return grain(Image.composite(img, Image.blend(img, dark, vig), mask))


def grain(img, sigma=3.4):
    try:
        import numpy as np
    except Exception:
        return img
    a = np.asarray(img).astype(np.int16)
    n = np.random.default_rng(5).normal(0, sigma, a.shape[:2])[..., None]
    return Image.fromarray(np.clip(a + n, 0, 255).astype("uint8"))


def ridge(d, w, h, base, tone, amp, phase, n=9):
    pts = [(0, h)]
    pts += [
        (w * i / n, h * (base - amp * math.sin(i * 1.35 + phase) - amp * 0.5 * math.cos(i * 2.7 + phase)))
        for i in range(n + 1)
    ]
    pts.append((w, h))
    d.polygon(pts, fill=tone)


def dawn(w=900, h=600):
    img = vgrad(w, h, [(0, (10, 16, 40)), (0.28, (38, 46, 90)), (0.50, (118, 84, 118)),
                       (0.62, (222, 120, 84)), (0.70, (255, 186, 116)), (0.745, (255, 226, 176)),
                       (0.80, (48, 36, 46)), (1, (22, 18, 26))])
    img = glow(img, w * 0.62, h * 0.705, 240, (255, 206, 138), 100)
    d = ImageDraw.Draw(img)
    d.ellipse((w * 0.62 - 22, h * 0.675 - 22, w * 0.62 + 22, h * 0.675 + 22), fill=(255, 246, 220))
    for base, tone, amp, ph in [(0.80, (74, 54, 68), 0.085, 0), (0.86, (44, 34, 48), 0.07, 2.1),
                                (0.93, (20, 17, 26), 0.05, 4.2)]:
        ridge(d, w, h, base, tone, amp, ph)
    return finish(img)


def bokeh(w=900, h=600):
    img = vgrad(w, h, [(0, (11, 12, 20)), (0.55, (22, 20, 30)), (1, (32, 22, 24))])
    layer = Image.new("RGB", (w, h), (0, 0, 0))
    d = ImageDraw.Draw(layer)
    rnd = random.Random(7)
    pal = [(196, 148, 96), (150, 104, 84), (98, 116, 142), (188, 176, 150), (120, 92, 96)]
    for _ in range(46):
        r = rnd.randint(26, 120)
        x, y = rnd.uniform(-40, w + 40), rnd.uniform(-40, h + 40)
        k = rnd.uniform(0.30, 0.62)
        d.ellipse((x - r, y - r, x + r, y + r), fill=tuple(int(v * k) for v in rnd.choice(pal)))
    img = ImageChops.screen(img, layer.filter(ImageFilter.GaussianBlur(34)))
    d = ImageDraw.Draw(img)
    for _ in range(10):
        r = rnd.randint(3, 9)
        x, y = rnd.uniform(0, w), rnd.uniform(0, h)
        d.ellipse((x - r, y - r, x + r, y + r),
                  fill=rnd.choice([(255, 214, 156), (232, 226, 208), (206, 170, 120)]))
    return finish(img.filter(ImageFilter.GaussianBlur(0.6)), 0.36)


def dunes(w=900, h=600):
    img = vgrad(w, h, [(0, (200, 196, 196)), (0.30, (230, 208, 182)), (0.48, (238, 198, 158)),
                       (1, (196, 148, 104))])
    img = glow(img, w * 0.16, h * 0.22, 190, (255, 244, 216), 120)
    layers = [(0.48, (240, 206, 166), (200, 148, 104), 0.055),
              (0.60, (230, 182, 134), (176, 120, 82), 0.045),
              (0.73, (208, 156, 112), (146, 96, 64), 0.065),
              (0.86, (172, 122, 86), (112, 74, 52), 0.05)]
    for k, (base, top, bot, amp) in enumerate(layers):
        pts = [(0, h)]
        pts += [(w * i / 72, h * (base + amp * math.sin(i * 0.17 + k * 2.1)
                                  + amp * 0.4 * math.sin(i * 0.53 + k * 1.3))) for i in range(73)]
        pts.append((w, h))
        img.paste(vgrad(w, h, [(0, top), (1, bot)]), (0, 0), poly_mask(w, h, pts, 1.2))
        shade = Image.new("L", (w, h), 0)
        ImageDraw.Draw(shade).line([(x, y + 9) for x, y in pts[1:-1]], fill=130, width=13)
        img = Image.composite(Image.blend(img, Image.new("RGB", (w, h), (88, 56, 40)), 0.34),
                              img, shade.filter(ImageFilter.GaussianBlur(11)))
    return finish(img, 0.20)


def sea(w=900, h=600):
    img = vgrad(w, h, [(0, (88, 104, 140)), (0.30, (158, 156, 170)), (0.45, (226, 176, 148)),
                       (0.495, (240, 204, 170)), (0.512, (70, 76, 94)), (0.75, (38, 44, 60)),
                       (1, (20, 24, 36))])
    img = glow(img, w * 0.70, h * 0.48, 170, (255, 214, 158), 80)
    d = ImageDraw.Draw(img)
    d.ellipse((w * 0.70 - 20, h * 0.475 - 20, w * 0.70 + 20, h * 0.475 + 20), fill=(255, 238, 196))
    streak = Image.new("RGB", (w, h), (0, 0, 0))
    sd = ImageDraw.Draw(streak)
    rnd = random.Random(19)
    for _ in range(110):
        t = rnd.random()
        y = h * (0.53 + 0.47 * t ** 1.5)
        x = w * 0.70 + rnd.gauss(0, 30 + 150 * t)
        ln = rnd.uniform(4, 30) * (0.4 + t)
        sd.line((x - ln, y, x + ln, y), fill=(150, 132, 116), width=max(1, int(1 + t * 3)))
    img = ImageChops.screen(img, streak.filter(ImageFilter.GaussianBlur(1.6)))
    d = ImageDraw.Draw(img)
    for _ in range(3):
        x, y = rnd.uniform(w * 0.12, w * 0.5), rnd.uniform(h * 0.10, h * 0.26)
        d.arc((x - 7, y - 4, x, y + 3), 200, 340, fill=(62, 60, 68), width=2)
        d.arc((x, y - 4, x + 7, y + 3), 200, 340, fill=(62, 60, 68), width=2)
    return finish(img, 0.24)


def peaks(w=600, h=900):
    img = vgrad(w, h, [(0, (26, 36, 68)), (0.24, (56, 78, 124)), (0.42, (132, 136, 160)),
                       (0.53, (224, 174, 152)), (0.60, (244, 206, 174)), (0.68, (78, 72, 88)),
                       (1, (36, 34, 46))])
    img = glow(img, w * 0.28, h * 0.58, 180, (255, 200, 154), 85)
    for base, top, bot, jag, ph in [(0.54, (94, 106, 136), (62, 74, 102), 0.14, 0.0),
                                    (0.62, (68, 78, 104), (44, 52, 76), 0.16, 1.4),
                                    (0.72, (44, 48, 66), (24, 26, 38), 0.13, 2.7)]:
        n = 11
        pts = [(0, h)]
        for i in range(n + 1):
            peak = 1.0 if i % 2 else 0.32
            pts.append((w * i / n, h * (base - jag * peak * (1 + math.sin(i * 1.9 + ph)) * 0.7)))
        pts.append((w, h))
        img.paste(vgrad(w, h, [(0, top), (1, bot)]), (0, 0), poly_mask(w, h, pts, 1.2))
        caps = Image.new("L", (w, h), 0)
        cd = ImageDraw.Draw(caps)
        for i in range(1, len(pts) - 1):
            ax, ay = pts[i]
            lx, ly = pts[i - 1]
            rx, ry = pts[i + 1]
            if ay <= ly or ay <= ry:
                continue
            drop = (max(ly, ry) - ay) * 0.42
            tl, tr = drop / (ly - ay), drop / (ry - ay)
            cd.polygon([(ax, ay), (ax + (rx - ax) * tr, ay + drop),
                        (ax + (lx - ax) * tl, ay + drop)], fill=225)
        img.paste(vgrad(w, h, [(0, (252, 252, 255)), (1, (198, 210, 228))]),
                  (0, 0), caps.filter(ImageFilter.GaussianBlur(2.2)))
    haze = Image.composite(vgrad(w, h, [(0, (0, 0, 0)), (1, (128, 136, 154))]),
                           Image.new("RGB", (w, h), (0, 0, 0)), ramp(w, h, h * 0.72, h * 1.05))
    return finish(ImageChops.screen(img, haze.filter(ImageFilter.GaussianBlur(22))), 0.28)


def night(w=900, h=600):
    img = vgrad(w, h, [(0, (5, 9, 24)), (0.45, (14, 22, 48)), (0.75, (24, 30, 54)), (1, (16, 18, 30))])
    band = Image.new("RGB", (w, h), (0, 0, 0))
    ImageDraw.Draw(band).polygon([(0, h * 0.82), (w * 0.42, 0), (w * 0.60, 0), (w * 0.22, h * 0.86)],
                                 fill=(54, 56, 84))
    img = ImageChops.screen(img, band.filter(ImageFilter.GaussianBlur(60)))
    stars = Image.new("RGB", (w, h), (0, 0, 0))
    sd = ImageDraw.Draw(stars)
    rnd = random.Random(11)
    for _ in range(650):
        x, y = rnd.uniform(0, w), rnd.uniform(0, h * 0.92)
        r, v = rnd.uniform(0.5, 1.4), rnd.randint(80, 255)
        sd.ellipse((x - r, y - r, x + r, y + r), fill=(v, v, min(255, int(v * 1.06))))
    img = ImageChops.screen(img, stars)
    ridge(ImageDraw.Draw(img), w, h, 0.90, (10, 12, 20), 0.04, 1.3, 7)
    return finish(img, 0.32)


def city(w=900, h=600):
    img = vgrad(w, h, [(0, (34, 40, 74)), (0.28, (86, 80, 112)), (0.44, (206, 144, 124)),
                       (0.52, (240, 192, 152)), (0.56, (66, 62, 80)), (0.75, (30, 32, 44)),
                       (1, (18, 20, 28))])
    img = glow(img, w * 0.30, h * 0.53, 190, (255, 194, 148), 100)
    d = ImageDraw.Draw(img)
    rnd = random.Random(23)
    horizon = h * 0.56
    x = -40.0
    while x < w + 40:
        bw, bh = rnd.uniform(46, 120), rnd.uniform(70, 250)
        tone = rnd.choice([(28, 30, 42), (36, 36, 50), (22, 24, 34)])
        d.rectangle((x, horizon - bh, x + bw, horizon), fill=tone)
        for wy in range(int(horizon - bh) + 9, int(horizon) - 7, 13):
            for wx in range(int(x) + 6, int(x + bw) - 7, 12):
                if rnd.random() < 0.26:
                    d.rectangle((wx, wy, wx + 4, wy + 5),
                                fill=rnd.choice([(255, 214, 150), (255, 194, 120), (226, 206, 180)]))
        x += bw + rnd.uniform(2, 14)
    streak = Image.new("RGB", (w, h), (0, 0, 0))
    sd = ImageDraw.Draw(streak)
    for _ in range(150):
        t = rnd.random()
        y = horizon + 4 + (h - horizon) * t ** 1.4
        sx = w * 0.30 + rnd.gauss(0, 40 + 120 * t)
        ln = rnd.uniform(5, 34) * (0.4 + t)
        sd.line((sx - ln, y, sx + ln, y), fill=(120, 108, 96), width=max(1, int(1 + 2 * t)))
    return finish(ImageChops.screen(img, streak.filter(ImageFilter.GaussianBlur(1.8))), 0.26)


# Each sample carries the camera, lens and exposure the inspector reads: a
# shoot with no EXIF in it leaves the panel saying it is still reading. The
# dates match the file dates the screenshot workflow stamps on the copies.
SHOTS = [
    ("IMG_0412.jpg", dawn, ("Canon", "Canon EOS R5", "RF24-70mm F2.8 L IS USM", (1, 250), (28, 10), 200, (35, 1), "2026:09:12 06:14:22")),
    ("IMG_0427.jpg", bokeh, ("Sony", "ILCE-7M4", "FE 35mm F1.4 GM", (1, 80), (14, 10), 800, (35, 1), "2026:09:13 19:42:05")),
    ("IMG_0431.jpg", dunes, ("NIKON CORPORATION", "NIKON Z 8", "NIKKOR Z 24-120mm f/4 S", (1, 500), (80, 10), 100, (70, 1), "2026:09:14 16:05:40")),
    ("IMG_0444.jpg", sea, ("FUJIFILM", "X-T5", "XF16-55mmF2.8 R LM WR", (1, 320), (80, 10), 160, (45, 1), "2026:09:15 18:31:12")),
    ("IMG_0458.jpg", peaks, ("Canon", "Canon EOS R6m2", "RF70-200mm F2.8 L IS USM", (1, 400), (56, 10), 100, (150, 1), "2026:09:16 07:02:47")),
    ("IMG_0463.jpg", night, ("Sony", "ILCE-7M4", "FE 16-35mm F2.8 GM", (25, 1), (28, 10), 3200, (16, 1), "2026:09:17 23:18:33")),
    ("IMG_0471.jpg", city, ("NIKON CORPORATION", "NIKON Z 8", "NIKKOR Z 85mm f/1.8 S", (1, 125), (28, 10), 640, (85, 1), "2026:09:18 20:07:58")),
]


def exif_for(make, model, lens, exposure, fnumber, iso, focal, taken):
    ex = Image.Exif()
    ex[0x010F] = make
    ex[0x0110] = model
    ex[0x0132] = taken
    ifd = ex.get_ifd(IFD.Exif)
    ifd[0x9003] = taken
    ifd[0x829A] = exposure
    ifd[0x829D] = fnumber
    ifd[0x8827] = iso
    ifd[0xA434] = lens
    ifd[0x920A] = focal
    return ex


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    for name, fn, meta in SHOTS:
        im = fn()
        im.save(os.path.join(OUT, name), quality=80, optimize=True, progressive=True,
                exif=exif_for(*meta))
        print(f"{name} {im.size[0]}x{im.size[1]}")
