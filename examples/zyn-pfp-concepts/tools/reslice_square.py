#!/usr/bin/env python3
"""Re-cut the Mole and Owl releases from their source canvases, square by
cropping instead of padding.

The original exporters (build_mole_collection_2222.py, build_owl_collection_1333.py)
made a non-square cell square by repeating its edge column or row, which reads
as streaks, and kept any sliver of dark board gutter left inside the measured
cell, which reads as a black line at an edge. This cuts the same cells, found
by the same gutter measurement, and:

  * trims up to 4 source px of flat near-black gutter from each edge;
  * makes the cell square by cropping — a tall Mole cell loses half the extra
    height from the top and half from the bottom; a tall Owl cell only 15%
    from the top, because owl ear tufts sit high and the rest comes off the
    body at the bottom (both chosen by comparing crops side by side); a wide cell loses its
    sides evenly; per-image offsets in OFFSETS override the rule where review
    found the head or headgear clipped;
  * scales the square to 192 with one nearest-neighbour step, as before.

Every item keeps its cell, so its art, traits and component review are the
same portrait; only the framing changes. Moths were cut with a five-pixel
inset and never padded, so they are left alone.

    python3 tools/reslice_square.py            # write images, checksums, catalog hashes
    python3 tools/reslice_square.py --verify   # prove the cell mapping by rebuilding the old images
"""

import hashlib
import json
import pathlib
import sys

import numpy as np
from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import build_mole_collection_2222 as mole_builder  # noqa: E402
import build_owl_collection_1333 as owl_builder  # noqa: E402

SPECIES = {
    "mole-collection-2222": mole_builder,
    "owl-collection-1333": owl_builder,
}
SIZE = 192
TOP_SHARE = {"mole-collection-2222": 0.5, "owl-collection-1333": 0.15}
GUTTER_MAX = 4

# Per-item vertical offsets (source px cropped from the top), set by eye where
# the rule clipped a head or headgear. Keyed by folder and item number.
OFFSETS: dict[tuple[str, str], int] = {}


def lum(a):
    return (a[..., :3] * [0.299, 0.587, 0.114]).sum(-1)


def trim_gutter(a):
    """Drop flat near-black lines at each edge that are darker than the art inside."""
    def dark(line):
        l = lum(line.astype(float))
        return l.mean() < 22 and l.std() < 14

    def edge(get, n):
        k = 0
        while k < GUTTER_MAX and k + 1 < n and dark(get(k)):
            k += 1
        if k and lum(get(k).astype(float)).mean() - lum(get(k - 1).astype(float)).mean() > 25:
            return k
        return 0

    H, W = a.shape[:2]
    t = edge(lambda i: a[i], H)
    b = edge(lambda i: a[H - 1 - i], H)
    l = edge(lambda i: a[:, i], W)
    r = edge(lambda i: a[:, W - 1 - i], W)
    return a[t:H - b, l:W - r], (t, b, l, r)


def square(a, share, offset=None):
    H, W = a.shape[:2]
    if H > W:
        d = H - W
        t = min(d, max(0, round(d * share) if offset is None else offset))
        return a[t:t + W], t
    if W > H:
        d = W - H
        l = d // 2
        return a[:, l:l + H], l
    return a, 0


def cells(folder):
    builder = SPECIES[folder]
    source_map = json.loads((ROOT / folder / "source-map.json").read_text())
    ranges = {}
    for i, rec in enumerate(source_map, start=1):
        c = rec["canvas"]
        if c not in ranges:
            canvas = Image.open(ROOT / folder / "canvases" / f"canvas-{c:02d}.png").convert("RGB")
            if hasattr(builder, "GRID_SPECS"):   # the owl exporter is told each board's grid
                cols, rows = builder.GRID_SPECS[c]
                xr_, yr_ = builder.measured_cell_ranges(canvas, cols, "x"), builder.measured_cell_ranges(canvas, rows, "y")
            else:                                # the mole exporter measures it
                xr_, yr_ = builder.measured_cell_ranges(canvas, "x"), builder.measured_cell_ranges(canvas, "y")
            ranges[c] = (canvas, xr_, yr_)
        canvas, xr, yr = ranges[c]
        x0, x1 = xr[rec["column"] - 1]
        y0, y1 = yr[rec["row"] - 1]
        assert (x1 - x0, y1 - y0) == (rec["sourceWidth"], rec["sourceHeight"]), (folder, i, rec)
        yield f"{i:04d}", rec, canvas, (x0, x1), (y0, y1)


def main():
    verify = "--verify" in sys.argv
    report = []
    for folder, builder in SPECIES.items():
        base = ROOT / folder
        sums = {}
        for line in (base / "checksums.sha256").read_text().splitlines():
            h, name = line.split(None, 1)
            sums[name.strip()] = h
        catalog = json.loads((base / "component-catalog.json").read_text())
        for number, rec, canvas, xr, yr in cells(folder):
            if verify:
                old = builder.slice_pad_and_resize(canvas, xr, yr)
                yield_path = pathlib.Path(sys.argv[sys.argv.index("--verify") + 1]) / folder / f"{number}.png"
                same = np.array_equal(np.asarray(old), np.asarray(Image.open(yield_path).convert("RGB")))
                if not same:
                    sys.exit(f"{folder} {number}: the measured cell does not rebuild the original image")
                continue
            cell = np.asarray(canvas.crop((xr[0], yr[0], xr[1], yr[1])).convert("RGB"))
            cell, trimmed = trim_gutter(cell)
            sq, off = square(cell, TOP_SHARE[folder], OFFSETS.get((folder, number)))
            img = Image.fromarray(np.ascontiguousarray(sq)).resize((SIZE, SIZE), Image.Resampling.NEAREST)
            path = base / "images" / f"{number}.png"
            img.save(path, optimize=True)
            new = hashlib.sha256(path.read_bytes()).hexdigest()
            sums[f"images/{number}.png"] = new
            catalog["items"][number]["imageSha256"] = new
            report.append({"image": f"{folder}/images/{number}.png", "cell": [rec["sourceWidth"], rec["sourceHeight"]],
                           "gutterTrim": trimmed, "cropOffset": off, "sha256": new})
        if not verify:
            (base / "checksums.sha256").write_text("".join(f"{h}  {n}\n" for n, h in sorted(sums.items())))
            (base / "component-catalog.json").write_text(json.dumps(catalog, indent=2) + "\n")
    if verify:
        print("verified: every measured cell rebuilds its original image exactly")
    else:
        (ROOT / "RESLICE.json").write_text(json.dumps({"tool": "tools/reslice_square.py", "topShare": TOP_SHARE,
                                                       "offsets": {f"{f}/{n}": o for (f, n), o in OFFSETS.items()},
                                                       "images": report}, indent=1) + "\n")
        print(f"re-cut {len(report)} images")


if __name__ == "__main__":
    main()
