# Zyn Moth Atomic Canvas Pack

This is the active Moth artwork workflow.

Each NFT is authored as one complete, aligned PFP inside a large canvas cell. The
cell is then cropped and exported as one atomic 192 × 192 PNG. Independent body,
eye, mouth, headgear, and light images are never recombined afterward, eliminating
position drift between generated assets.

## Active files

- `atlases/` — approved full-canvas source boards
- `canvas-slices/` — atomic 192 × 192 exports sliced directly from the boards
- `canvas-previews/` — contact sheets rebuilt only from those exports
- `canvas-manifest.json` — filenames, dimensions, and atomic-canvas policy
- `references/` — the two approved Moth direction images

Current canvas library:

- 12 background canvases
- 10 body-direction canvases
- 10 eye-direction canvases
- 7 mouth-direction canvases
- 9 headgear canvases
- 12 light canvases, including neutral

These are complete PFP canvases and visual trait masters, not transparent overlays.

## Collection production rule

For every collection item:

1. Select the recipe.
2. Author the entire recipe together in one canvas cell, using the approved Moth
   references and the trait-master canvases.
3. Approve the full image as a whole.
4. Slice the cell to 192 × 192.
5. Store the recipe in metadata, but do not recreate the image from independently
   generated visual layers.
6. Apply interactive wallet/web lighting as a runtime viewer effect over the atomic
   NFT image.

## Rebuild the slices

```sh
python3 examples/zyn-pfp-concepts/tools/slice_moth_canvases.py
```

The exporter uses the measured interiors of the actual gold frames. It performs no
redrawing and no cross-canvas compositing.

The earlier procedural kit and the attempted separated-overlay extraction are kept
only under `../archive/` for recovery and are not active sources.
