# Nap Mole Collection — Component Review Manifest

Status: visual review only. These are atomic full-portrait canvases, not independent transparent overlays.

## Collection math

- Supply target: 333
- Fixed character: 1 sleeping Mole face with closed eyes and three `z`, `z`, `Z` sleep glyphs
- Backgrounds: 2
- Light effects: 2
- Bodies: 12
- Headwear: 14 (10 eye masks, 3 pajama hats, 1 none)
- Mouths: 6
- Available production recipes: `1 × 2 × 2 × 12 × 14 × 6 = 4,032`
- Recipes unused after selecting 333 unique NFTs: 3,699, before compatibility exclusions

The two reserve eye masks on canvas 07 are not counted in the 14 production headwear options.

## Slicing contract

Canvases 01–08 are 1024 × 1536 with a 2 × 2 layout. Split at the exact horizontal and vertical midpoint to obtain four 512 × 768 cells.

Canvas 09 is 1536 × 1536 with a 3 × 2 layout. Split it into six 512 × 768 cells. Production-ready individual mouth slices are also saved under `slices/mouth/`.

Reduce 512 × 768 cells with nearest-neighbor sampling to 128 × 192.

Cell order is always:

1. top-left
2. top-right
3. bottom-left
4. bottom-right

## Canvases

### 01-background-light-2x2.png

1. `campfire--moonlight`
2. `campfire--starlight`
3. `wall-torch--moonlight`
4. `wall-torch--starlight`

Production dimensions represented by this canvas:

- Background: `campfire`, `wall-torch`
- Light: `moonlight`, `starlight`

### 02-body-set-a-2x2.png

1. `pajama-charcoal-gold-piping`
2. `pajama-ivory-charcoal-piping`
3. `pajama-teal-ivory-piping`
4. `pajama-moon-print`

### 03-body-set-b-2x2.png

1. `pajama-star-print`
2. `pajama-shield-print`
3. `pajama-encrypted-grid`
4. `sleeping-bag-charcoal-quilt`

### 04-body-set-c-2x2.png

1. `sleeping-bag-teal-cocoon`
2. `sleeping-bag-ivory-cloud`
3. `sleeping-bag-gold-lined-vault`
4. `sleeping-bag-midnight-stars`

### 05-eye-mask-set-a-2x2.png

1. `eye-mask-plain-ivory`
2. `eye-mask-teal-shield`
3. `eye-mask-small-z`
4. `eye-mask-moon-phases`

### 06-eye-mask-set-b-2x2.png

1. `eye-mask-encrypted-grid`
2. `eye-mask-quiet-dots`
3. `eye-mask-gold-stripe`
4. `eye-mask-teal-checker`

### 07-eye-mask-set-c-reserve-2x2.png

1. `eye-mask-gold-crescent` — production
2. `eye-mask-stars` — production
3. `eye-mask-cloud` — reserve, excluded from batch-one math
4. `eye-mask-lock-shield` — reserve, excluded from batch-one math

### 08-headwear-hats-none-2x2.png

1. `none`
2. `pajama-hat-charcoal-gold`
3. `pajama-hat-ivory-teal`
4. `pajama-hat-teal-ivory`

### 09-mouth-production-3x2.png

Row-major:

1. `neutral-dash`
2. `friendly-smile`
3. `star-mouth`
4. `gold-tooth`
5. `clenched-line`
6. `signal-grille`

Rejected from production: `side-smirk`, `small-open-o`, `crescent-mouth`, and `surprised-ring`. The six-mouth production set contains only one smile.

## Review checks

- Fixed closed sleeping eyes in every cell
- Six distinct production mouths with only one smile
- Exactly three gold sleep glyphs beside the head
- No cat-like ears or fur texture
- Mole scale, crop, snout, paws, and lower ledge remain consistent
- Only two background IDs and two light-effect IDs enter production
- Eye-mask patterns remain legible at 128 × 192
- Sleeping bags read as padded silhouettes rather than oversized pajama tops
