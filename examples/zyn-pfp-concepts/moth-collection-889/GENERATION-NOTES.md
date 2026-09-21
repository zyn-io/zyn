# Zyn Moth 889 — generation notes

The eleven source canvases were authored with the built-in image generation
system. Each cell was generated as one complete Moth portrait; no independent
body, eye, mouth, headgear or background image was later recombined.

## Locked visual reference

- Large rounded Moth silhouette
- Right-facing three-quarter view at approximately 35 degrees
- Large near eye and smaller far eye
- Two gold clubbed antennae
- Broad overlapping wing shoulders
- Black, graphite, cyan, teal, cream, violet and Zcash-gold identity
- Deliberate pixel clusters and hard pixel edges
- Static light is part of the authored scene; wallet motion effects are additive

## Canvas prompt families

1. Balanced Genesis
2. Vault and Tunnel
3. Aurora and Prism
4. Moonlight and Starlight
5. Bioluminescence and Caustics
6. Afterglow and Eclipse
7. Rare Signals
8. Shielded Stealth
9. Genesis Finale
10. Vault Reserve
11. Color Reserve

Every prompt requested a dense uniform contact sheet of distinct, complete PFPs
with no captions, labels, empty cells, layer diagrams or isolated components.
The approved Moth reference boards were supplied to preserve anatomy, framing,
material finish and accessory fit across canvas families.

## Export

Image-generation grids are visually regular but their last row or column can be
slightly compressed. `tools/build_moth_collection_889.py` therefore detects the
real dark gutters before cropping, applies a five-pixel inset, and resizes each
atomic portrait to 192×192 with nearest-neighbor sampling. For compressed final
rows, it restores the median normal-row height by extending only the bottom edge;
the face, antennae and body retain the same scale as the preceding rows.
Canvas 04's final column is intrinsically compressed. Its nine released positions
are replaced with unused, fully proportioned portraits from canvas 11's reserve
region; geometric repair is intentionally avoided.

The source canvases contain 947 cells. The release uses 889 complete portraits,
including nine canvas-11 replacements for canvas 04's compressed final column.
The remaining source inventory is therefore 49 clean reserve portraits and nine
rejected malformed cells, recorded separately in `reserve-map.json` and
`rejected-source-cells.json`.
