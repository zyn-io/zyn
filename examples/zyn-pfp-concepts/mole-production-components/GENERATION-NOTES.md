# Zyn Mole component generation notes

Mode: built-in image generation using `layered-moles-35-degree.png` as the
authoritative anatomy/style reference and `mole-variant-study.png` as the
supporting silhouette and pixel-density reference.

## Shared prompt lock

Every source board requested complete Mole PFP cards with the same right-facing
35-degree anatomy: low broad rounded head, no external ears, tiny narrow-set
eyes, one projecting spade-shaped contrasting muzzle, broad asymmetric digging
paws and short pale claws. The visual grammar is coarse premium pixel art based
on a 32×48 logical grid and the Zyn black/gold/cyan palette.

All prompts explicitly prohibited Owl or Moth anatomy, antennae, visible ears,
rodent teeth, mouse/rat/bear/dog/monkey/pig silhouettes, earrings, text, smooth
3D rendering, fur micro-detail, broken gutters, merged cells, incomplete final
rows and stretched cards.

## Prompt set

- Background A: 5×3, first 15 environments
- Background B: 5×3, second 15 environments
- Body: 5×5, 25 body/material directions; every muzzle contrasts with the body
- Mouth: 6×3, 18 mouth directions; marks differ from body and muzzle colors
- Eyes: 5×5, 25 tiny-eye and fitted-eyewear directions
- Headgear A: 6×2, first 12 fitted directions
- Headgear B: 5×2, final 10 fitted directions
- Light A: 5×3, neutral, canonical lights and first three subvariants
- Light B: 5×3, final 15 Mole-specific light subvariants

The Star Mouth prompt specifically requires six chunky ochre lobes around a
dark recessed center with a highlight and shadow following the 35-degree muzzle
plane. It must read as protruding anatomy rather than a flat star symbol.

## Export policy

Gold-framed boards are sliced from measured frame interiors. Unframed boards
are sliced between measured full-dark gutters. Rectangular cells are padded to
square by extending only their outer background pixels, then resized uniformly
to 192×192 with nearest-neighbor sampling. No Mole subject is internally cropped
or non-uniformly scaled.
