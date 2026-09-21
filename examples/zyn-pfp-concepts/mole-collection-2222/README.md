# Zyn Mole Collection 01 — 2,222

This directory contains the complete Mole allocation for the first 4,444-item
Zyn collection on Zcash.

## Contents

- `images/0001.png` through `images/2222.png`: final 192×192 Mole artwork
- `metadata/0001.json` through `metadata/2222.json`: matching metadata
- `canvases/`: original whole-PFP production boards
- `source-map.json`: exact canvas, row and column for every released token
- `reserve-map.json`: clean unused source cells
- `rejected-source-cells.json`: cells excluded before numbering
- `checksums.sha256`: SHA-256 checksum for every release PNG
- `validation.json`: machine-readable build and geometry audit
- `trait-schema.json`: store filter definitions, component vocabulary and viewer effect registry
- `component-catalog.json`: image-bound visual judgment for every released Mole
- `component-review-overrides.json`: reproducible direct-review corrections for inspected edge cases
- `component-filter-counts.json`: verified per-category counts for store menus
- `previews/`: collection overview and boundary QA sheets

## Production rule

Each NFT was authored as a complete portrait inside a canvas cell. The Mole
component masters were visual direction only; the final release was not assembled
from independent body, mouth, eye, headgear or light overlays.

The exporter detects the actual rendered gutter grid, uses only complete cells
bounded by two gutters, rejects undersized rows or columns, and also rejects any
source cell whose height is below 95% of its width. This prevents a short source
cell from being completed by repeating its bottom scanline. Eligible rectangular
cells are edge-padded to square and receive one uniform nearest-neighbor resize to
192×192. The exporter never crops into the portrait and never stretches one axis
independently.

No rarity tiers or rarity scores are assigned. Metadata exposes `Species`,
`Visual Family`, `Interactive Light`, `Background`, `Body`, `Eyes`, `Mouth` and
`Headgear` as store filters, plus a stable `zyn-light-v1` viewer contract.
Component IDs describe visually reviewed categories in the atomic artwork; they
do not claim composited-layer provenance. Confidence tiers and runner-up matches
remain in the catalog for later audit.

Rebuild with:

```sh
python3 examples/zyn-pfp-concepts/tools/build_mole_collection_2222.py
```
