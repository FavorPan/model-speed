#!/usr/bin/env python3
"""Generate unified app icons: dark rounded-square bg + accent lightning bolt.
Colors match src/index.html design tokens:
  bg     #0d0e12 (off-black surface)
  border #2a2d38
  accent #62a0ea (restrained cyan-blue, the single app accent)
"""
from PIL import Image, ImageDraw
import os, math

BG       = (13, 14, 18, 255)      # #0d0e12
BG2      = (28, 30, 38, 255)      # subtle gradient end
BORDER   = (42, 45, 56, 255)      # #2a2d38
ACCENT   = (98, 160, 234, 255)    # #62a0ea
ACCENT_HI= (140, 190, 245, 255)   # bolt highlight

OUT = os.path.dirname(os.path.abspath(__file__))

def rounded_gradient(size, radius):
    """Dark rounded-square with a soft vertical gradient + border."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    grad = Image.new("RGBA", (size, size))
    for y in range(size):
        t = y / max(1, size - 1)
        r = int(BG[0] + (BG2[0] - BG[0]) * t)
        g = int(BG[1] + (BG2[1] - BG[1]) * t)
        b = int(BG[2] + (BG2[2] - BG[2]) * t)
        for x in range(size):
            grad.putpixel((x, y), (r, g, b, 255))
    # mask for rounded rect
    mask = Image.new("L", (size, size), 0)
    md = ImageDraw.Draw(mask)
    md.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    img.paste(grad, (0, 0), mask)
    # border
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, outline=BORDER, width=max(1, size // 64))
    return img

def bolt_path(size, scale=0.58):
    """Lightning bolt polygon centered in the square. scale < 1 = smaller bolt
    with more padding around it."""
    s = size
    # bolt bounds (normalized 0..1), centered around (0.5, 0.5)
    pts_norm = [
        (0.54, 0.16),
        (0.26, 0.55),
        (0.45, 0.55),
        (0.34, 0.84),
        (0.74, 0.42),
        (0.52, 0.42),
        (0.62, 0.16),
    ]
    # scale around center (0.5, 0.5) to shrink the bolt and add padding
    out = []
    for x, y in pts_norm:
        nx = 0.5 + (x - 0.5) * scale
        ny = 0.5 + (y - 0.5) * scale
        out.append((nx * s, ny * s))
    return out

def draw_bolt(img, size):
    d = ImageDraw.Draw(img)
    pts = bolt_path(size)
    # soft glow
    for w, alpha in [(size // 12, 26), (size // 20, 40)]:
        glow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        gd = ImageDraw.Draw(glow)
        gd.polygon(pts, fill=(ACCENT[0], ACCENT[1], ACCENT[2], alpha))
        img.alpha_composite(glow)
    # main bolt
    d.polygon(pts, fill=ACCENT)
    # highlight (smaller inner polygon offset up-left)
    hi = [(x * 0.96 + size * 0.01, y * 0.96 + size * 0.005) for x, y in pts_norm(pts)]
    d.polygon(hi, fill=ACCENT_HI) if False else None  # keep simple: skip inner
    return img

def pts_norm(pts):
    return pts

def make_icon(size, radius_ratio=0.22):
    radius = int(size * radius_ratio)
    img = rounded_gradient(size, radius)
    img = draw_bolt(img, size)
    return img

def make_tray(size=44):
    """Template-style tray icon: transparent bg, light bolt for menu bar."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # 托盘图标保持较大占比（菜单栏小图标需清晰），不受 app 图标缩小影响
    pts = bolt_path(size, scale=0.78)
    # shift bolt to center (bolt bounds ~0.16..0.84 already centered)
    d.polygon(pts, fill=(235, 238, 245, 255))
    return img

if __name__ == "__main__":
    import subprocess
    tmp = os.path.join(OUT, "_tmp")
    os.makedirs(tmp, exist_ok=True)

    # PNGs — save() works (file encoder path)
    png_files = {}
    for name, size in [("32x32.png", 32), ("128x128.png", 128),
                       ("128x128@2x.png", 256), ("icon.png", 512)]:
        make_icon(size).save(os.path.join(OUT, name))
        print("wrote", name, size)

    # Write temp PNGs at all sizes for icns/ico assembly via ImageMagick
    all_sizes = [16, 32, 64, 128, 256, 512, 1024]
    tmp_pngs = {}
    for s in all_sizes:
        p = os.path.join(tmp, f"icon_{s}.png")
        make_icon(s).save(p)
        tmp_pngs[s] = p

    # ICNS via magick (iconutil would need a .iconset dir; magick writes icns directly)
    icns_src = tmp_pngs[512]
    r = subprocess.run(["magick", icns_src, os.path.join(OUT, "icon.icns")],
                       capture_output=True, text=True)
    if r.returncode != 0:
        # fallback: build .iconset + iconutil
        iconset = os.path.join(tmp, "ModelSpeed.iconset")
        os.makedirs(iconset, exist_ok=True)
        mapping = {
            "icon_16x16": 16, "icon_16x16@2x": 32, "icon_32x32": 32,
            "icon_32x32@2x": 64, "icon_128x128": 128, "icon_128x128@2x": 256,
            "icon_256x256": 256, "icon_256x256@2x": 512, "icon_512x512": 512,
            "icon_512x512@2x": 1024,
        }
        for name, s in mapping.items():
            make_icon(s).save(os.path.join(iconset, name + ".png"))
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o",
                        os.path.join(OUT, "icon.icns")], check=True)
    print("wrote icon.icns")

    # ICO via magick (multi-size)
    r = subprocess.run(
        ["magick", tmp_pngs[256], "-define", "icon:auto-resize=16,32,48,64,128,256",
         os.path.join(OUT, "icon.ico")],
        capture_output=True, text=True)
    if r.returncode != 0:
        # fallback: single-size ico
        subprocess.run(["magick", tmp_pngs[256], os.path.join(OUT, "icon.ico")], check=True)
    print("wrote icon.ico")

    # Tray (menu bar) — transparent bg, light bolt
    make_tray(44).save(os.path.join(OUT, "tray-icon.png"))
    print("wrote tray-icon.png")

    # cleanup
    import shutil
    shutil.rmtree(tmp, ignore_errors=True)
    print("done.")
