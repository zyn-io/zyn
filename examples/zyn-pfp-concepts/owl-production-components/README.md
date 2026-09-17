# Zyn Owl Atomic Canvas Pack

This pack contains the 90 visual masters for the 1,333-Owl allocation in Zyn
Collection 01. The component volume is exactly 1.5 times the 60-master Moth pack,
matching the 30% Owl versus 20% Moth collection allocation.

## Inventory

- 18 backgrounds
- 15 body directions
- 11 mouth and beak directions
- 15 eye directions
- 13 fitted headgear directions
- 18 light-preview states, including neutral

`atlases/` contains the six authored boards plus one targeted Light replacement
board. `canvas-slices/` contains the 90
atomic 192×192 masters. `canvas-previews/` contains labeled review sheets.
`canvas-manifest.json` records the exact file inventory, while `validation.json`
checks count, uniqueness and dimensions. `checksums.sha256` records every master.
Pure review grids preserve their authored column counts: 6 columns for Background
and Light, 5 for Body, Eyes and Headgear, and 4 for Mouth. Each grid card is
exactly 192×192 with no title or label space mixed into the grid dimensions.
Labeled review catalogs are exported separately as `*-catalog.png`.

The Light atlas is intentionally sliced from measured dark gutter bands rather
than equal arithmetic sixths. Its generated source columns are uneven, and equal
division leaks neighboring backgrounds into columns 2 and 5. Each detected Light
cell is normalized independently to exactly 192×192. The source board's compressed
sixth column is not normalized or stretched: Aurora, Eclipse and Constellation
Sweep come from three native-square 512×512 cells in
`atlases/light-last-column-replacements.png`.

Every character component was authored as a complete, aligned PFP. These files
are visual direction masters, not transparent overlays, and must not be recombined
to construct final NFTs. A final Owl recipe is authored together in one canvas
cell and then sliced, following the working Moth production method.

Rebuild with:

```sh
python3 examples/zyn-pfp-concepts/tools/slice_owl_canvases.py
```

## Full-collection guardrails

The future 1,333-item Owl batch must not assume a requested grid was actually
rendered. Each source canvas must be audited for its real row and column count,
actual gutter coordinates and compressed edge cells before any NFT IDs are
assigned. Cells with malformed proportions are rejected and replaced from a
reserve pool; they are never stretched into compliance. The final build must
prove 1,333 images, 1,333 metadata files, 1,333 unique hashes, 192×192 dimensions
and byte-for-byte stability for unaffected items after any targeted repair.
