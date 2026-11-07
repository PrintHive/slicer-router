"""Write the Slicer Router icon masters and every bundle icon made from them.

icon.svg draws 64 px and up; icon-small.svg has heavier strokes for 16 and 32 px.
Needs rsvg-convert (`brew install librsvg`) and macOS iconutil. Run from anywhere.
"""
import math
import struct
import subprocess
import tempfile
from pathlib import Path

INK, NEON, BONE, EDGE = "#0A0A0A", "#39FF14", "#F0EEE9", "#222525"


def hexagon(cx, cy, r):
    return " ".join(
        f"{cx + r * math.cos(math.radians(a)):.1f},{cy + r * math.sin(math.radians(a)):.1f}"
        for a in range(0, 360, 60)
    )


def icon(src_r, src_w, dst_r, dst_w, branch_w, spread, dim):
    sx, sy, dx = 318, 512, 726
    join = sx + src_r + 70
    parts = [
        f'<rect x="100" y="100" width="824" height="824" rx="186" fill="{INK}"/>',
        f'<rect x="102" y="102" width="820" height="820" rx="184" fill="none" stroke="{EDGE}" stroke-width="4"/>',
        f'<polygon points="{hexagon(sx, sy, src_r)}" fill="none" stroke="{BONE}" stroke-width="{src_w}"/>',
    ]
    for y, chosen in ((sy - spread, False), (sy, True), (sy + spread, False)):
        colour, opacity = (NEON, 1) if chosen else (BONE, dim)
        parts.append(
            f'<path d="M{sx + src_r} {sy} L{join} {sy} L{join + 48} {y} L{dx - dst_r} {y}" fill="none" '
            f'stroke="{colour}" stroke-opacity="{opacity}" stroke-width="{branch_w}" stroke-linejoin="miter"/>'
        )
        fill = NEON if chosen else "none"
        stroke = "" if chosen else f' stroke="{BONE}" stroke-opacity="{dim}" stroke-width="{dst_w}"'
        parts.append(f'<polygon points="{hexagon(dx, y, dst_r)}" fill="{fill}"{stroke}/>')
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">{"".join(parts)}</svg>\n'


here = Path(__file__).parent


def render(svg, size, out):
    subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), str(svg), "-o", str(out)], check=True)


def master(size):
    return here / ("icon-small.svg" if size <= 32 else "icon.svg")


def write_ico(sizes, out):
    """An .ico whose entries are PNGs, which Windows Vista and later read."""
    images = []
    with tempfile.TemporaryDirectory() as tmp:
        for size in sizes:
            png = Path(tmp) / f"{size}.png"
            render(master(size), size, png)
            images.append((size, png.read_bytes()))
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries, data = b"", b""
    for size, png in images:
        side = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(png), offset + len(data))
        data += png
    out.write_bytes(header + entries + data)


def main():
    (here / "icon.svg").write_text(icon(src_r=118, src_w=38, dst_r=76, dst_w=28, branch_w=30, spread=222, dim=0.42))
    (here / "icon-small.svg").write_text(icon(src_r=130, src_w=72, dst_r=96, dst_w=56, branch_w=60, spread=238, dim=0.5))
    icons = here.parent / "src-tauri" / "icons"
    for name, size in (("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)):
        render(master(size), size, icons / name)
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "icon.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            render(master(size), size, iconset / f"icon_{size}x{size}.png")
            render(master(size * 2), size * 2, iconset / f"icon_{size}x{size}@2x.png")
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(icons / "icon.icns")], check=True)
    write_ico((16, 24, 32, 48, 64, 128, 256), icons / "icon.ico")


if __name__ == "__main__":
    main()
