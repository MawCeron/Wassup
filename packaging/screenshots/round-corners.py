"""Rounds the corners of a raw capture, as GNOME draws a window, and saves it as PNG.

Usage: round-corners.py <capture> <output.png>
"""
import sys
from PIL import Image, ImageDraw
src, dst, radius, scale = sys.argv[1], sys.argv[2], 12, 4
img = Image.open(src).convert("RGBA")
w, h = img.size
mask = Image.new("L", (w * scale, h * scale), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, w * scale - 1, h * scale - 1], radius * scale, fill=255)
img.putalpha(mask.resize((w, h), Image.LANCZOS))
img.save(dst, optimize=True)
