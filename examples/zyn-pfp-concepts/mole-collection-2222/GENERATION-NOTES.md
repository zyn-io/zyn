# Zyn Mole 2,222 — generation notes

Mode: built-in image generation using the approved layered Mole study and the
approved Mole variant study as visual references.

## Locked visual rules

- Right-facing three-quarter view at approximately 35 degrees
- Low, broad rounded head with absolutely no external ears
- Tiny narrow-set eyes, much smaller than Owl eyes
- Projecting spade-shaped muzzle visibly contrasting with the body
- Mouth mark distinct from body and muzzle
- Broad asymmetric digging-paw shoulders with short pale claws
- Premium chunky hard-edged pixels with Punk/Monke-level simplification
- Approximately 16 character colors; static light may add smooth colors
- Star Mouth is protruding six-lobed ochre anatomy around a recessed dark center
- One complete portrait per source cell; no independent overlay assembly

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
13. Archivists and Geologists
14. Miners and Prospectors
15. Royal Vault
16. Rare Signals
17. Burrow Cross-Sections
18. Crystal Geodes
19. Ancient Rail
20. Root Network
21. Subterranean Phosphor
22. Quantum Tunnel
23. Gold Vein
24. Genesis Core
25. Full-Spectrum Reserve A
26. Material Reserve
27. Face Variation Reserve
28. Headgear Reserve
29. Final Quality Reserve
30. Neutral QA Reserve
31. Clean Geometry Reserve A
32. Clean Geometry Reserve B
33. Clean Geometry Reserve C

## Source-board correction

The first Vault/Tunnel attempt was discarded because row 3 contained a
background-only cell and its final row contained no complete Moles. A second
repair attempt repeated the bad edge row, so the accepted board was regenerated
as an explicit complete 10×9 source grid. Neither failed render is part of this
directory or release.

The generator did not always honor requested row counts. The exporter therefore
measures the actual rendered gutters rather than assuming a prompt grid. Across
33 accepted boards it found 2,537 authored gutter-bounded cells. A cell is now
rejected if its row or column is below 90% of that board's median span, or if its
height is below 95% of its width. The second rule specifically prevents vertical
padding from repeating a prematurely cut bottom scanline into a visible horizontal
band. The combined audit rejects 247 source cells. Of the 2,290 remaining unique
cells, 2,222 form the release and 68 are clean reserves.

The original release numbers affected by the short-edge-row defect were
`0161–0170`, `0331–0340`, `0921–0930`, `1061–1070`, `1281–1300`, `1501–1510`,
`1641–1660`, `1871–1880` and `2111–2120`. Numbering was rebuilt after exclusion,
so these ranges describe the pre-fix release only.

## Clean slicing policy

Only a portrait proven between two full-board dark gutters is eligible. The
complete cell is sliced without an internal crop. Rectangular cells are made
square by extending their existing edge pixels only after the 95% aspect check,
then resized with one uniform nearest-neighbor scale factor. No portrait is
horizontally stretched, vertically squeezed or reconstructed from component
layers.
