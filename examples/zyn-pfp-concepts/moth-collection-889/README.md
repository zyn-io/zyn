# Zyn Moths — 889-item collection

This directory contains the complete Moth allocation for Zyn Collection 01.

- `canvases/`: eleven authored production canvases with verified 9×9, 10×9 or 10×10 grids
- `images/`: 889 atomic 192×192 PNGs
- `metadata/`: 889 matching JSON files
- `source-map.json`: exact source canvas, row and column for every image
- `reserve-map.json`: 49 clean unused portraits available for later batches
- `rejected-source-cells.json`: nine malformed source cells that must never be reused
- `checksums.sha256`: deterministic image checksums
- `validation.json`: machine-readable supply, uniqueness, size and source-grid audit
- `trait-schema.json`: store filter definitions, component vocabulary and viewer effect registry
- `component-catalog.json`: image-bound visual judgment for every released Moth
- `component-filter-counts.json`: verified per-category counts for store menus
- `GENERATION-NOTES.md`: canvas prompt families and production constraints
- `previews/moth-collection-889-overview.png`: the complete collection overview

The source canvases contain 947 authored cells. The release uses 889 complete
portraits, including nine canvas-11 replacements for canvas 04's compressed final
column. The remainder is 49 clean reserve portraits plus nine rejected malformed
cells.

Each PFP was authored as a complete image inside its canvas cell and then sliced.
No independently generated visual layers were recombined.

The metadata exposes `Species`, `Visual Family`, `Interactive Light`, `Background`,
`Body`, `Eyes`, `Mouth` and `Headgear` as store filters, plus a stable
`zyn-light-v1` viewer contract. Component IDs describe visually reviewed
categories in the finished atomic artwork; they do not claim composited-layer
provenance. Confidence tiers remain in `component-catalog.json` for later audit.

The cropper detects the actual dark gutters in every generated canvas before it
slices. This accommodates slightly compressed final rows or columns and prevents
neighboring portraits from bleeding into edge items. When a source canvas has a
compressed final row, the exporter preserves the portrait scale and extends only
the low-detail bottom edge to the normal row height instead of stretching the
entire Moth. Canvas 04's intrinsically compressed final-column portraits are not
geometrically repaired; those nine release positions use unused, fully
proportioned portraits from the reserve canvas.

Rebuild the slices with:

```sh
python3 examples/zyn-pfp-concepts/tools/build_moth_collection_889.py
```
