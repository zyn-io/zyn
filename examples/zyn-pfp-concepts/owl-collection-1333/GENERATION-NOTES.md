# Zyn Owl 1,333 — generation notes

Mode: built-in image generation using the approved Zyn Owl anatomy and component
boards as visual references.

## Locked visual rules

- Right-facing three-quarter view at approximately 35 degrees
- Broad rounded head; large near eye disc and smaller far eye disc
- Tiny species-specific beak and folded wing shoulders
- Two native feather tufts unless fitted headgear intentionally hides them
- Premium chunky hard-edged pixel clusters with Punk/Monke-level simplicity
- Controlled 16-color character palette; static light may add smooth colors
- One complete portrait per source cell; never independent overlay assembly

## Prompt families

1. Balanced Genesis
2. Vault and Tunnel
3. Moonlight and Starlight
4. Aurora and Bioluminescence
5. Sunlight and Dawn
6. Afterglow and Eclipse
7. Prism and Laser
8. Caustics and Veiled Light
9. Nebula and Comet
10. Lantern Gold and Voidlight
11. Shielded Stealth
12. Node and Cipher
13. Archivists and Moon Scribes
14. Miners and Navigators
15. Royal Vault
16. Rare Signals
17. Full-Spectrum Reserve

Each prompt requested a strict 10×10 square contact sheet with identical cells,
continuous dark gutters, consistent Owl scale, no captions and no compressed edge
rows or columns. The generated result was then audited rather than trusted: the
actual boards include 10×10, 10×9 and 9×9 grids.

## Clean slicing policy

The exporter measures the actual full-dark gutter bands, excludes unused black
edge margins and takes each complete gutter-bounded cell without an internal crop.
If a cell is rectangular, its existing boundary background pixels are extended
outward to make a square. The full square is then uniformly resized to 192×192
with nearest-neighbor sampling. This preserves the authored portrait and avoids
both subject cropping and non-uniform scaling.

Visual review identified compressed final rows in canvases 01, 04, 09, 10, 13
and 15. Those 59 source cells are hard-coded as forbidden and none appear in the
1,333-item release.

Canvas 04 row 8 column 1 is also rejected during subject-presence review because
it contains only the bioluminescent background. No substitution or geometric
repair is performed. The 1,532 remaining cells are ordered directly by canvas,
row and column: the first 1,333 form the release and 199 remain in reserve.
