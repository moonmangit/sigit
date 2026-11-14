"""Draws sigit's app icon and writes every format the builds need.

    python assets/make_icon.py

Writes, next to this script:
  icon.png       1024 px master (README, Linux, notifications)
  icon-256.png   for desktop notifications
  icon.ico       Windows: the .exe, taskbar, installer (16-256 px)
  icon.icns      macOS: the .app bundle

The mark is the same git-branch glyph as the title bar logo (src/ui/mod.rs, I_LOGO), on a
rounded square with an indigo-to-blue gradient. Needs Pillow.
"""
import math
import os

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
S = 4096  # drawn large, scaled down for smooth edges
TOP = (99, 77, 255)      # indigo
BOTTOM = (34, 139, 255)  # blue


def gradient(size):
    """Diagonal top-left to bottom-right gradient."""
    small = 256
    img = Image.new("RGB", (small, small))
    px = img.load()
    for y in range(small):
        for x in range(small):
            t = (x + y) / (2 * (small - 1))
            px[x, y] = tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM))
    return img.resize((size, size), Image.BICUBIC)


def draw(size=S):
    # macOS-style margins: the rounded square fills ~80% of the canvas.
    pad = round(size * 0.1)
    box = size - 2 * pad
    radius = round(box * 0.225)
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle([pad, pad, pad + box, pad + box], radius=radius, fill=255)
    icon = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    icon.paste(gradient(size), (0, 0), mask)

    # The glyph on a 16-unit grid, as in I_LOGO: nodes at (4.5,4) (4.5,12) (11.5,6.5),
    # a trunk from the top node to the bottom one, and a branch curving from the right node
    # into the trunk.
    d = ImageDraw.Draw(icon)
    # The glyph spans 2.75..13.25 across and 2.25..13.75 down (centre 8, 8): scale it to fill
    # about 68% of the square and centre it.
    unit = box * 0.68 / 11.5
    ox = pad + box / 2 - 8 * unit
    oy = pad + box / 2 - 8 * unit
    p = lambda x, y: (ox + x * unit, oy + y * unit)
    white = (255, 255, 255, 255)
    stroke = round(unit * 1.05)
    r = unit * 1.85

    def line(points):
        d.line(points, fill=white, width=stroke, joint="curve")
        for pt in (points[0], points[-1]):
            d.ellipse([pt[0] - stroke / 2, pt[1] - stroke / 2, pt[0] + stroke / 2, pt[1] + stroke / 2], fill=white)

    # trunk, between the two left rings
    line([p(4.5, 4 + 1.75), p(4.5, 12 - 1.75)])
    # branch: cubic from under the right ring down into the trunk
    a, c1, c2, b = (11.5, 8.25), (11.5, 10.6), (8.2, 10.4), (6.1, 11.1)
    curve = []
    for i in range(41):
        t = i / 40
        x = (1 - t) ** 3 * a[0] + 3 * (1 - t) ** 2 * t * c1[0] + 3 * (1 - t) * t ** 2 * c2[0] + t ** 3 * b[0]
        y = (1 - t) ** 3 * a[1] + 3 * (1 - t) ** 2 * t * c1[1] + 3 * (1 - t) * t ** 2 * c2[1] + t ** 3 * b[1]
        curve.append(p(x, y))
    line(curve)
    # rings
    for cx, cy in ((4.5, 4), (4.5, 12), (11.5, 6.5)):
        x, y = p(cx, cy)
        d.ellipse([x - r, y - r, x + r, y + r], outline=white, width=stroke)
    return icon


# Every image macOS looks for, as Apple's iconutil writes them: (type, pixel size). The 1x
# 16 and 32 px ones (icp4, icp5) are what Finder lists, Spotlight and menus use on non-Retina
# screens; Pillow's ICNS writer leaves them out.
ICNS_TYPES = [
    ("icp4", 16), ("icp5", 32), ("icp6", 64),
    ("ic07", 128), ("ic08", 256), ("ic09", 512), ("ic10", 1024),
    ("ic11", 32), ("ic12", 64), ("ic13", 256), ("ic14", 512),
]


def write_icns(master, path):
    """An .icns of PNG images (macOS 10.7+): 'icns', total length, then type/length/data."""
    import io
    import struct

    chunks = b""
    for kind, px in ICNS_TYPES:
        buf = io.BytesIO()
        master.resize((px, px), Image.LANCZOS).save(buf, format="PNG")
        data = buf.getvalue()
        chunks += kind.encode("ascii") + struct.pack(">I", 8 + len(data)) + data
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", 8 + len(chunks)) + chunks)


def main():
    big = draw()
    master = big.resize((1024, 1024), Image.LANCZOS)
    master.save(os.path.join(HERE, "icon.png"))
    master.resize((256, 256), Image.LANCZOS).save(os.path.join(HERE, "icon-256.png"))
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    master.save(os.path.join(HERE, "icon.ico"), sizes=[(s, s) for s in sizes])
    write_icns(master, os.path.join(HERE, "icon.icns"))
    print("wrote icon.png, icon-256.png, icon.ico, icon.icns")


if __name__ == "__main__":
    main()
