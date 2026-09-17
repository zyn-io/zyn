# Nap Mole Early-User Collection — 333

This collection is built from complete atomic portraits rather than independently composited overlays.

## Output

- `canvases-v2/`: sixteen accepted strict 5 × 5 source canvases
- `canvases/`: preserved rejected v1 source canvases; never used by the current builder
- `candidates/`: the deterministic 400-image candidate pool
- `images/`: the final 333-image collection
- `metadata/`: token metadata with exact source provenance and image binding
- `previews/candidate-pool-400.png`: all 400 candidates
- `previews/nap-mole-collection-333.png`: final collection overview
- `previews/removed-67.png`: the 67 candidates removed from the pool
- `previews/source-canvas-01-qa.png` … `source-canvas-16-qa.png`: labeled 5 × 5 source-cell audits
- `candidate-map.json`: source mapping for the 400 candidates
- `source-map.json`: source and candidate mapping for the final 333
- `removed-candidates.json`: the exact 67 removals
- `source-reserve-map.json`: empty in v2 because all 400 cells enter the candidate pool
- `checksums.sha256`: checksums for final images
- `validation.json`: machine-readable build audit

## Selection method

The sixteen v2 source canvases contain exactly 400 complete portraits. The old five-canvas atlas is preserved for traceability but rejected because its generated 9 × 9 panels had unequal geometry and several baked-in expression defects.

All 25 cells from each v2 canvas enter the 400-candidate pool. The reproducible seed `nap-mole-early-users-333-v1` keeps 21 portraits from each of canvases 1–13 and 20 from each of canvases 14–16, for exactly 333. Manually rejected expression or mouth defects are always excluded before the remaining random removals.

## Image geometry

Each final NFT is 128 × 192 pixels. The exporter locates the continuous black gutter nearest each expected 5 × 5 boundary, slices the complete panel between those gutters, then center-crops it to 2:3 and reduces once with nearest-neighbor sampling. This trims the slightly wider fourth columns without stretching any character or copying edge pixels.

All 400 v2 source cells are represented in labeled QA sheets. Cells with open-looking eyes or malformed mouths are listed in `manual-rejected-source-cells.json` and cannot enter the final 333.

## Trait status

All 25 requested skin/body families are visibly represented. Every metadata record is bound to its PNG by filename, dimensions, and SHA-256 digest, and records exact canvas/row/column provenance. Headwear and mouth were manually classified against the final token images for store filtering. Other appearance fields remain intentionally broad instead of assigning guessed component names.

Rebuild with:

```sh
python3 apps/nap/branding/nap-mole-collection-333/build_collection.py
```
