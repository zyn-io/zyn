# Zyn Underground — Collection 01

## Release

- Supply: 4,444 NFTs
- Format: square 192×192 PNG
- Character safe area: centered 128×192
- Base artwork: one shared 16-color indexed palette
- Lighting: smooth colors rendered after the base artwork
- Species: Mole, Owl and Moth
- No fourth or mysterious species in Collection 01

## Species supply

| Species | Supply | Share |
|---|---:|---:|
| Mole | 2,222 | 50% |
| Owl | 1,333 | 30% |
| Moth | 889 | 20% |
| Total | 4,444 | 100% |

## Layer order

1. Background
2. Rear anatomy bundled with Body when required
3. Body/head, including native ears, tufts or antennae
4. Mouth, beak or mouthparts
5. Eyes
6. Headgear back
7. Headgear front
8. Optional foreground object bundled with a Body trait
9. Material mask
10. Runtime lighting and foreground glints

Earrings are not included in Collection 01. Mouths, eyes and headgear are fitted to each species. A hat may contain rear, front and body-occlusion images while remaining one metadata trait. Owl headgear may use its occlusion image to hide one or both native ear tufts.

## Trait inventory

| Trait | Selectable choices | Artwork files |
|---|---:|---:|
| Background | 12 core, plus 6 Owl-exclusive | 18 |
| Body | 10 Mole, 15 Owl, 10 Moth | 35 |
| Mouth | 8 Mole, 11 Owl, 7 Moth | 26 |
| Eyes | 11 Mole, 15 Owl, 10 Moth | 36 |
| Hat | 9 core fittings, plus 4 Owl-exclusive | 31 |
| Light | 11 canonical shader configurations; Owl has 6 extra visual subvariants | 0 composable PNGs |
| Total visible PNG choices | | 146 |

### Backgrounds

Deep Black, Tunnel Grid, Shielded Pool, Node Chamber, Gold Vault, Cyan Cavern, Moon Blocks, Aurora Garden, Eclipse Gate, Prism Archive, Genesis Chamber and Zero Field.

Owls add Midnight Observatory, Shielded Forest, Lunar Ruins, Star Map Vault,
Solar Archive and Nebula Tunnel, giving that species 18 background directions.

### Mole bodies

Charcoal Hoodie, Earth Miner, Slate Utility, Brown Tunnel Scout, Silver Node Technician, Albino Archivist, Cyan Circuit, Gold-Dust Keeper, Obsidian Armor and Genesis Robe.

### Owl bodies

Charcoal Sentinel, Brown Scholar, Ash Courier, Silver Oracle, Snow Watcher, Cyan
Circuit, Gold-Flecked, Obsidian Armor, Moon Robe, Genesis Keeper, Teal Navigator,
Bronze Cartographer, Dusk Plum, Eclipse Feather and Shielded White.

### Moth bodies

Charcoal Velvet, Ashwing, Brown Dustwing, Silver Silk, Moonwhite, Cyan Circuit, Aurora Wing, Gold Pattern, Obsidian Mantle and Genesis Wing.

### Mouths

- Mole: Neutral, Smile, Smirk, Open, Star Mouth, Gold Tooth, Respirator and Pipe.
- Owl: Calm Diamond Beak, Short Beak, Open Hooting Beak, Sharp Hooked Beak, Gold
  Beak, Respirator, Signal Whistle, Crystal Beak, Crescent Moon Beak, Split Rune
  Beak and Cipher Beak.
- Moth: Small Mouthparts, Curled Proboscis, Long Proboscis, Hidden Mouth, Glowing Mouth, Respirator and Crystal Feeder.

For every Mole Body, the base artwork owns a clearly contrasting muzzle patch. The Mouth layer adds only the expression mark and must use a palette index distinct from both the muzzle and surrounding body. Star Mouth is a compact, protruding radial mouth: six chunky ochre lobes surround a dark recessed center, with one-pixel light and shadow edges following the 35-degree facial plane. It must read as puckered anatomy, never as a flat star symbol. This rule preserves eight selectable Mole mouth traits without multiplying artwork files by body color.

### Eyes

- Mole: Hidden, Sleepy, Alert, Suspicious, Cyan, Gold, Scarred, Goggles, Visor, Pixel Shades and Laser.
- Owl: Round Cyan, Narrowed Cyan, Sleepy, Moonlit, Gold, Constellation, Monocle,
  Dark Visor, Eclipse, Prism, Aurora, Void, Star Map, Shielded Scan and Luminous White.
- Moth: Round, Faceted, Sleepy, Cyan, Mint, Violet, Aurora, Prism, Void and Luminous.

### Headgear

The nine core concepts are None, Black Hood, Miner Helmet, Beanie, Zyn Cap, Node
Headset, Vault Crown, Moon Halo and Genesis Helm. Owls add Archivist Circlet,
Solar Visor Helmet, Cipher Cowl and Night-Watch Crest. Owl fittings declare
`earCoverage`: None, Zyn Cap, Node Headset, Vault Crown, Moon Halo, Archivist
Circlet and Night-Watch Crest keep tufts visible; Beanie may partially hide them;
Black Hood, Miner Helmet, Genesis Helm, Solar Visor Helmet and Cipher Cowl hide both tufts.

## Canonical light supply

| Light | Supply |
|---|---:|
| Sunlight | 622 |
| Moonlight | 622 |
| Starlight | 578 |
| Bioluminescence | 533 |
| Aurora | 444 |
| Afterglow | 400 |
| Caustics | 356 |
| Prism | 311 |
| Laser | 267 |
| Veiled Light | 178 |
| Eclipse | 133 |
| Total | 4,444 |

The canonical light appears in the static NFT preview. Nap Wallet uses the same light configuration for the interactive drag response. The effect selector remains a development and preview tool rather than part of the final collectible view.

## Material mask

Each artwork trait may have a same-size material mask:

- Red: reflective or foil response
- Green: emissive response
- Blue: iridescent response
- Alpha: veiled-light reveal response

For deterministic hand-authored layers, the renderer may composite selected visible layers and masks. The image-generated Collection 01 artwork is an explicit exception: the entire trait recipe is authored together in one canvas cell and sliced to 192×192, so independent generated overlays are never recombined. Nap Wallet applies the runtime response over that atomic image.

## Combination space

```text
Mole: 12 × 10 × 8 × 11 × 9 × 11 = 1,045,440
Owl:  18 × 15 × 11 × 15 × 13 × 11 = 6,370,650
Moth: 12 × 10 × 7 × 10 × 9 × 11 =   831,600

Total: 8,247,690 valid combinations before compatibility exclusions
```

The generator selects 4,444 unique recipes while enforcing the fixed species and
light supplies, balanced visual-family counts and compatibility exclusions. The
collection does not assign artificial rarity tiers or rarity scores.

## Moth atomic-canvas implementation status

The active Moth art pack is available in `moth-production-components/`. Its source atlases contain 12 background directions, 10 body directions, 10 eye directions, 7 mouth directions, 9 headgear directions and neutral plus 11 static-light directions. `canvas-slices/` contains atomic 192×192 PFPs cropped directly from those approved canvases. They are visual trait masters, not transparent overlays.

The previous prototype batch has been removed from the active collection paths. The complete 889-item Moth allocation now lives in `moth-collection-889/`: eleven full-PFP canvases provide 947 authored cells, of which 889 are released and 58 remain as reserve alternatives. The gutter-aware exporter slices the completed portraits directly to 192×192 PNGs and never reconstructs final artwork from independently generated body, eye, mouth, headgear or light files. Every released image has a matching metadata file, unique SHA-256 hash and source-canvas record.

## Owl atomic-canvas implementation status

The complete 1,333-item Owl allocation lives in `owl-collection-1333/`. Seventeen
full-PFP canvases provide 1,592 authored cells. Source review rejects 59 portraits
from six visibly compressed final rows and one empty-background cell. The other
1,532 cells are valid: 1,333 are released and 199 remain in reserve. The exporter
records the actually rendered 10×10, 10×9 or 9×9 grid for each board, measures its
gutters, slices complete cells without an internal crop, edge-pads rectangular
cells to square and then applies one uniform nearest-neighbor resize. Every released image has
matching metadata, a unique SHA-256 hash and an exact source-cell record.

Detailed visual trait classification and global token IDs remain pending until
the Mole allocation is assembled into the final 4,444-item reveal order.

## Mole atomic-canvas implementation status

The 150-master Mole direction pack lives in `mole-production-components/`. Its
volume follows the Collection 01 supply ratio: `2,222 / 889 ≈ 2.5`, exactly 2.5
times the 60-master Moth pack. It contains 30 backgrounds, 25 bodies, 18 mouths,
25 eyes, 22 fitted headgear directions and 30 light previews including neutral.

Every master is a complete 35-degree Mole PFP and is used as a visual direction,
not a composable overlay. The complete Mole allocation now lives in
`mole-collection-2222/`. Thirty-three whole-PFP source boards provide 2,537
measured gutter-bounded cells. The exporter rejects 247 cells under the combined
90% row/column span and 95% height-to-width safety checks; 2,290 unique clean
cells remain, of which 2,222 are released and 68 stay in reserve. Each released
image has matching metadata, a unique SHA-256 hash and an exact source-canvas
record.

The exporter slices complete cells without an internal crop, rejects landscape
cells that could repeat a horizontal cutoff into their padding, extends only the
existing boundary pixels when square padding is needed, and applies one uniform
nearest-neighbor resize to 192×192. The Mole, Owl and Moth allocations now total
the locked 4,444 supply; detailed trait classification and the global reveal
ordering remain separate cataloging steps.

## Batch 01 reuse ledger

`BATCH-01-COMPONENT-COVERAGE.md` records the cross-species component and reserve
audit. `BATCH-01-ASSET-COVERAGE.json` is its machine-readable companion and lists
all 300 prepared masters, exact released source-family counts and the 316 clean
whole-portrait reserves available for later batches. Because Collection 01 used
atomic authored portraits, exact per-token component IDs remain untracked and
must not be inferred as fact from visual similarity alone.
