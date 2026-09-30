#!/usr/bin/env python3
"""Repair two slicing defects in the released images, in place.

The exporter made each non-square source cell square by repeating its edge
pixels, and some cells kept a sliver of the dark board gutter. Two defects
follow, both only at an image's edges:

  * frame lines — up to 6 near-black, flat lines at an edge, darker than the
    art just inside: replaced by the first real line, extended outward;
  * streak bands — 8 px or more of one repeated column (or row) at an edge,
    where that line has detail along it (mean step >= 6), so the repeat reads
    as horizontal streaks: the copies are inpainted (OpenCV Telea) from the
    real pixels around them. A band over plain background is invisible and is
    left alone. Mirroring was tried and rejected: where the character reaches
    the edge it produced mirrored half-characters.

Nothing inside the art is touched, so traits and component reviews still
hold; checksums.sha256 and each component-catalog imageSha256 are updated to
the new bytes, and IMAGE-REPAIRS.json records every change with the old and
new hashes. Bands 16 px or wider are improved, not perfect ("wide": true).

Needs numpy, pillow and opencv-python-headless.

    python3 tools/repair_slicing.py            # repair
    python3 tools/repair_slicing.py --dry-run  # list only
"""

import hashlib
import json
import pathlib
import sys

import cv2
import numpy as np
from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parents[1]
FOLDERS = ["mole-collection-2222", "owl-collection-1333", "moth-collection-889"]
BAND_MIN, DETAIL_MIN, WIDE, GUTTER_MAX = 8, 6.0, 16, 6


def lum(a):
    return (a * [0.299, 0.587, 0.114]).sum(-1)


def lines(a, side):
    """The image as lines from the given edge inward (views, so writes land)."""
    t = np.transpose(a, (1, 0, 2))
    return {"L": t, "R": t[::-1], "T": a, "B": a[::-1]}[side]


def gutter_width(a, side):
    L, k = lines(a, side), 0
    while k < GUTTER_MAX:
        l = lum(L[k].astype(float))
        if l.mean() < 22 and l.std() < 14:
            k += 1
        else:
            break
    if k and lum(L[k].astype(float)).mean() - lum(L[k - 1].astype(float)).mean() > 25:
        return k
    return 0


def band(a, side):
    L, n = lines(a, side), 0
    while n + 1 < len(L) // 2 and np.array_equal(L[n], L[n + 1]):
        n += 1
    w = n + 1
    if w < BAND_MIN:
        return 0
    detail = float(np.abs(np.diff(L[0].astype(float), axis=0)).sum(1).mean())
    return w if detail >= DETAIL_MIN else 0


def repair(a):
    a = a.copy()
    gutters = {s: gutter_width(a, s) for s in "TBLR"}
    for s, g in gutters.items():
        L = lines(a, s)
        for i in range(g):
            L[i] = L[g]
    bands = {s: band(a, s) for s in "LRTB"}
    mask = np.zeros(a.shape[:2], np.uint8)
    for s, w in bands.items():
        if w:
            k = w - 1  # the innermost line of a band is real; its copies are not
            {"L": lambda: mask.__setitem__((slice(None), slice(0, k)), 255),
             "R": lambda: mask.__setitem__((slice(None), slice(-k, None)), 255),
             "T": lambda: mask.__setitem__((slice(0, k),), 255),
             "B": lambda: mask.__setitem__((slice(-k, None),), 255)}[s]()
    if mask.any():
        a = cv2.inpaint(np.ascontiguousarray(a), mask, 5, cv2.INPAINT_TELEA)
    found = {"gutter": {s: g for s, g in gutters.items() if g}, "band": {s: w for s, w in bands.items() if w}}
    return a, found


def main():
    dry = "--dry-run" in sys.argv
    record = []
    for folder in FOLDERS:
        base = ROOT / folder
        sums = {}
        for line in (base / "checksums.sha256").read_text().splitlines():
            h, name = line.split(None, 1)
            sums[name.strip()] = h
        catalog = json.loads((base / "component-catalog.json").read_text())
        for path in sorted((base / "images").glob("*.png")):
            raw = path.read_bytes()
            a = np.asarray(Image.open(path).convert("RGB"))
            b, found = repair(a)
            if not (found["gutter"] or found["band"]):
                continue
            old = hashlib.sha256(raw).hexdigest()
            entry = {"image": f"{folder}/images/{path.name}", **found,
                     "wide": any(w >= WIDE for w in found["band"].values()), "old": old}
            if not dry:
                Image.fromarray(b).save(path, optimize=True)
                new = hashlib.sha256(path.read_bytes()).hexdigest()
                sums[f"images/{path.name}"] = new
                catalog["items"][path.stem]["imageSha256"] = new
                entry["new"] = new
            record.append(entry)
        if not dry:
            (base / "checksums.sha256").write_text("".join(f"{h}  {n}\n" for n, h in sorted(sums.items())))
            (base / "component-catalog.json").write_text(json.dumps(catalog, indent=2) + "\n")
    wide = [r["image"] for r in record if r["wide"]]
    print(f"repaired {len(record)} images ({sum(bool(r['gutter']) for r in record)} frame lines, "
          f"{sum(bool(r['band']) for r in record)} streak bands, {len(wide)} of them wide)")
    if not dry:
        (ROOT / "IMAGE-REPAIRS.json").write_text(json.dumps({"tool": "tools/repair_slicing.py", "images": record}, indent=1) + "\n")


if __name__ == "__main__":
    main()
