"""Draws the pictures of the demo scene: a landscape photo and a link thumbnail.

Usage: art.py <output directory>
"""
import math, os, random, sys
from PIL import Image, ImageDraw, ImageFilter

def lerp(a, b, t): return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))

def landscape(w, h, sky_top, sky_bottom, ridges, sun):
    img = Image.new("RGB", (w, h))
    d = ImageDraw.Draw(img)
    for y in range(h):
        d.line([(0, y), (w, y)], fill=lerp(sky_top, sky_bottom, y / (h * 0.7)))
    sx, sy, sr = sun
    glow = Image.new("RGB", (w, h), (0, 0, 0)); g = ImageDraw.Draw(glow)
    g.ellipse([sx - sr * 3, sy - sr * 3, sx + sr * 3, sy + sr * 3], fill=(90, 60, 30))
    glow = glow.filter(ImageFilter.GaussianBlur(sr))
    img = Image.blend(img, Image.composite(Image.new("RGB", (w, h), (255, 220, 160)), img, glow.convert("L")), 0.55)
    d = ImageDraw.Draw(img)
    d.ellipse([sx - sr, sy - sr, sx + sr, sy + sr], fill=(255, 238, 200))
    rnd = random.Random(7)
    for base, amp, color in ridges:
        pts = [(0, h)]
        phase = rnd.random() * 10
        for x in range(0, w + 8, 8):
            y = base + amp * (math.sin(x / w * 5 + phase) * 0.6 + math.sin(x / w * 13 + phase * 2) * 0.25 + math.sin(x / w * 29 + phase) * 0.08)
            pts.append((x, y))
        pts.append((w, h))
        d.polygon(pts, fill=color)
    return img

photo = landscape(1200, 800, (70, 110, 170), (250, 190, 140),
                  [(430, 90, (120, 120, 150)), (500, 80, (80, 95, 120)), (600, 60, (45, 70, 80)), (700, 40, (25, 45, 45))],
                  (820, 330, 46))
d = ImageDraw.Draw(photo)
rnd = random.Random(3)
for _ in range(60):
    x = rnd.randint(0, 1200); y = rnd.randint(700, 790); s = rnd.randint(40, 90)
    d.polygon([(x, y - s * 2), (x - s // 2, y), (x + s // 2, y)], fill=(18, 38, 36))
out = sys.argv[1]
photo.save(os.path.join(out, "summit.jpg"), quality=90)

thumb = landscape(400, 210, (110, 150, 190), (240, 215, 170),
                  [(120, 30, (110, 130, 150)), (150, 25, (70, 100, 110)), (180, 20, (40, 70, 70))],
                  (300, 80, 18))
thumb.save(os.path.join(out, "link.jpg"), quality=88)
