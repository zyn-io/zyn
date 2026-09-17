# Zyn Mole Atomic Canvas Pack

This pack contains 150 visual masters for the 2,222-Mole allocation in Zyn
Collection 01. Its size is exactly 2.5 times the 60-master Moth pack, matching
the 50% Mole versus 20% Moth collection allocation.

## Inventory

- 30 backgrounds
- 25 body directions
- 18 mouth directions
- 25 eye directions
- 22 fitted headgear directions
- 30 light-preview states, including neutral

Every master is authored as a complete, aligned Mole PFP. These images are
visual direction masters, not transparent overlays, and must not be recombined
to construct final NFTs. Final Mole recipes will be authored whole in canvas
cells and sliced with the same no-crop, no-stretch workflow used for the Owl
collection.

## Locked anatomy

- Right-facing three-quarter view at approximately 35 degrees
- Low broad rounded head with no visible external ears
- Small narrow-set eyes and one projecting spade-shaped muzzle
- Two broad asymmetric digging-paw shoulders with short pale claws
- Muzzle color always contrasts with the Body
- Mouth mark always differs from both Body and muzzle colors
- Star Mouth is protruding and six-lobed around a recessed dark center
- Coarse premium pixel clusters; no fur texture or micro-detail
- Never read as mouse, rat, bear, dog, monkey or pig

## Active files

- `atlases/`: nine approved image-generated source boards
- `canvas-slices/`: 150 atomic 192×192 Mole visual masters
- `canvas-previews/`: pure contact sheets and labeled review catalogs
- `references/`: approved Mole anatomy and style references
- `canvas-manifest.json`: category names, source boards and measured grids
- `validation.json`: count, uniqueness, dimensions and slicing policy
- `checksums.sha256`: SHA-256 digest for every exported master

The slicer detects the actual gold-frame interiors or continuous dark gutters.
It preserves every complete card, edge-pads rectangular cells to square and
performs one uniform nearest-neighbor resize. It never stretches one axis or
crops into the Mole subject.

Rebuild with:

```sh
python3 examples/zyn-pfp-concepts/tools/slice_mole_canvases.py
```
