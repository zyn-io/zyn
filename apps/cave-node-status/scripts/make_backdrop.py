#!/usr/bin/env python3
"""Derive the page backdrop from the camera original.

The chamber photograph is nearly black — mean luminance 18/255, median 7 —
which reads as a flat dark rectangle once the page's scrims are over it. This
lifts it so the cave is actually visible behind the telemetry, and it is a
script rather than a hand-edited file so re-exporting the original does not
quietly lose the correction.

    python3 scripts/make_backdrop.py

Source `assets/cave-node-chamber.png` is the untouched original and is never
shipped; `assets/cave-node-chamber.webp` is what the build copies into dist.
"""

import pathlib

from PIL import Image, ImageEnhance

# Gamma well under 1 to lift shadows — this image lives almost entirely in
# them. Contrast and saturation come back up afterwards because lifting
# shadows alone leaves the rock looking grey and washed.
GAMMA = 0.45
CONTRAST = 1.15
SATURATION = 1.12
QUALITY = 82

root = pathlib.Path(__file__).resolve().parent.parent
source = root / "assets" / "cave-node-chamber.png"
target = root / "assets" / "cave-node-chamber.webp"

image = Image.open(source).convert("RGB")
curve = [min(255, round(255 * ((value / 255) ** GAMMA))) for value in range(256)]
image = image.point(curve * 3)
image = ImageEnhance.Contrast(image).enhance(CONTRAST)
image = ImageEnhance.Color(image).enhance(SATURATION)
image.save(target, "WEBP", quality=QUALITY, method=6)

levels = sorted(image.convert("L").getdata())
mean = sum(levels) / len(levels)
print(
    f"{target.relative_to(root)}: {target.stat().st_size // 1024} KB, "
    f"mean luminance {mean:.0f}/255 (was 18)"
)
