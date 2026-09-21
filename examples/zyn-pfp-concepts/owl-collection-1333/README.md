# Zyn Owl 1,333 — Collection 01

This directory contains the complete Owl allocation for the first 4,444-piece
Zyn PFP collection on Zcash.

- `canvases/`: 17 built-in image-generation source boards
- `images/`: 1,333 final 192×192 Owl PNGs
- `metadata/`: 1,333 matching JSON files
- `previews/owl-collection-1333-overview.png`: full release overview
- `previews/qa-canvas-boundaries.png`: first/last released portrait from each
  contributing source canvas
- `previews/qa-0021-0030-clean-slices.png`: full-size QA for the previously
  problematic #0021–#0030 range
- `source-map.json`: exact canvas cell used by every NFT
- `reserve-map.json`: clean unused portrait cells
- `rejected-source-cells.json`: the six compressed final rows and one empty cell
- `checksums.sha256`: final image hashes
- `validation.json`: count, uniqueness, dimensions and slicing audit
- `trait-schema.json`: store filter definitions, component vocabulary and viewer effect registry
- `component-catalog.json`: image-bound visual judgment for every released Owl
- `component-filter-counts.json`: verified per-category counts for store menus

## Production rule

Every Owl is authored as one complete portrait in a canvas cell. The collection
is not assembled from independently generated overlays. The exporter records the
grid that was actually rendered, measures real gutter positions and slices every
complete gutter-bounded cell exactly. It never crops inside a cell. A rectangular
cell is padded to square by extending only its outer background pixels, then the
whole square receives one uniform nearest-neighbor resize to 192×192. Horizontal
or vertical stretching is forbidden.

The visibly compressed final rows in source canvases 01, 04, 09, 10, 13 and 15
are explicitly forbidden release sources. All 59 cells remain only in the source
boards for provenance and are skipped before release numbering.

Canvas 04 row 8 column 1 contains only a background and is explicitly rejected.
Together these exclusions leave 1,532 complete Owl portraits: 1,333 are released
in canvas order and 199 remain in reserve. There are no repair substitutions.

No artificial rarity ranking is assigned. Metadata exposes `Species`, `Visual
Family`, `Interactive Light`, `Background`, `Body`, `Eyes`, `Beak` and `Headgear`
as store filters, plus a stable `zyn-light-v1` viewer contract. Component IDs
describe visually reviewed categories in the atomic artwork; they do not claim
composited-layer provenance. Confidence tiers remain available for later audit.

Rebuild with:

```sh
python3 examples/zyn-pfp-concepts/tools/build_owl_collection_1333.py
```
