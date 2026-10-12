#!/usr/bin/env python3
"""Generate the Git Tidy app icon as a high-resolution PNG."""
from PIL import Image, ImageDraw
import math
import os

SIZE = 1024
BG_TOP = (240, 248, 255)
BG_BOTTOM = (225, 240, 253)
ACCENT = (0, 122, 255)
ACCENT_DARK = (0, 89, 200)


def lerp_color(c1, c2, t):
    return tuple(int(c1[i] + (c2[i] - c1[i]) * t) for i in range(3))


def main():
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    margin = 80
    radius = 230

    for y in range(SIZE):
        t = y / SIZE
        color = lerp_color(BG_TOP, BG_BOTTOM, t)
        draw.line([(0, y), (SIZE, y)], fill=color + (255,))

    mask = Image.new("L", (SIZE, SIZE), 0)
    mask_draw = ImageDraw.Draw(mask)
    mask_draw.rounded_rectangle(
        [margin, margin, SIZE - margin, SIZE - margin],
        radius=radius,
        fill=255,
    )

    result = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    result.paste(img, (0, 0), mask)
    img = result
    draw = ImageDraw.Draw(img)

    cx, cy = SIZE // 2, SIZE // 2
    g_radius = 300
    g_thickness = 52
    cut_color = lerp_color(BG_TOP, BG_BOTTOM, 0.5) + (255,)

    # G ring
    draw.ellipse(
        [cx - g_radius, cy - g_radius, cx + g_radius, cy + g_radius],
        outline=ACCENT,
        width=g_thickness,
    )

    # Cut G opening (right-bottom quadrant)
    for i in range(60):
        angle_deg = 40 * i / 60
        angle_rad = math.radians(angle_deg)
        px = cx + g_radius * math.cos(angle_rad)
        py = cy + g_radius * math.sin(angle_rad)
        draw.ellipse(
            [px - g_thickness // 2 - 6, py - g_thickness // 2 - 6,
             px + g_thickness // 2 + 6, py + g_thickness // 2 + 6],
            fill=cut_color,
        )

    # G horizontal bar
    bar_y = cy + int(g_radius * math.sin(math.radians(20)))
    bar_length = int(g_radius * math.cos(math.radians(20))) + 20
    draw.rectangle(
        [cx - 5, bar_y - g_thickness // 2, cx + bar_length, bar_y + g_thickness // 2],
        fill=ACCENT,
    )

    # T vertical stem
    t_stroke = 52
    t_height = 280
    t_top = cy - 140
    t_x = cx - 10
    draw.rectangle(
        [t_x - t_stroke // 2, t_top, t_x + t_stroke // 2, t_top + t_height],
        fill=ACCENT,
    )

    # T crossbar
    t_cross_width = 200
    draw.rectangle(
        [t_x - t_cross_width // 2, t_top,
         t_x + t_cross_width // 2, t_top + t_stroke],
        fill=ACCENT,
    )

    # Git commit dots inside G bottom
    dot_r = 14
    dots = [
        (cx - 120, cy + 130),
        (cx, cy + 175),
        (cx + 120, cy + 130),
    ]
    for i in range(len(dots) - 1):
        x1, y1 = dots[i]
        x2, y2 = dots[i + 1]
        draw.line([x1, y1, x2, y2], fill=ACCENT_DARK, width=4)
    for dx, dy in dots:
        draw.ellipse(
            [dx - dot_r, dy - dot_r, dx + dot_r, dy + dot_r],
            fill=ACCENT_DARK,
        )

    # Outer subtle border
    draw.rounded_rectangle(
        [margin, margin, SIZE - margin, SIZE - margin],
        radius=radius,
        outline=(200, 210, 220, 128),
        width=2,
    )

    out = os.path.join(os.path.dirname(__file__), "..", "icon-source.png")
    img.save(out, "PNG")
    print(f"Saved {out} ({SIZE}x{SIZE})")


if __name__ == "__main__":
    main()
