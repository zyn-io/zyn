# Zyn Moth Variant Study

This concept sheet explores six Moth directions for the first 4,444-piece Zyn PFP collection. It follows the collection rule that the character artwork uses a shared 16-color base palette, while the canonical light effect may add smooth colors in the wallet viewer.

The sheet is the approved visual direction for atomic 192×192 Moth canvases, with the character constrained to the centered 128×192 safe area.

The active authored canvas pack is in `moth-production-components/`. It contains complete PFP canvases for the body, eye, mouth, headgear, background and static-light directions, built directly from the two approved Moth references. Each final combination must be authored together in one canvas and sliced as one image; independently generated overlays are not recombined. The discarded procedural and layer-extraction kits are no longer active sources.

## Simplicity rules

- Use one identical base Moth silhouette: two clubbed antennae, round face, tiny mouthparts, and folded wing shoulders.
- Use 6–9 flat colors per character from the shared collection palette.
- Use one dominant accessory at most.
- Avoid fur strands, engraving, circuitry, dithering, gradients, and micro-detail in the base artwork.
- Keep the static light treatment to one simple background motif and a 1–2 logical-pixel reflection on the character.
- Check every completed trait combination at 32 px and 64 px.

## Variants

1. **Vault Scout** — Charcoal Velvet body, Cyan eyes, Black Hood, Node Chamber background, Bioluminescence.
2. **Moon Archivist** — Moonwhite body, Luminous eyes, Moon Halo, Moon Blocks background, Moonlight.
3. **Aurora Wing** — Aurora Wing body, Aurora eyes, no headgear, Aurora Garden background, Aurora.
4. **Prism Regent** — Gold Pattern body, Prism eyes, Vault Crown, Prism Archive background, Prism.
5. **Zero Warden** — Obsidian Mantle body, Void eyes, Genesis Helm, Eclipse Gate background, Eclipse.
6. **Genesis Signal** — Genesis Wing body, Luminous eyes, Node Headset, Zero Field background, Afterglow.

## Generation brief

The first detailed Moth sheet was used as the edit target. The revision deliberately removes its ornate fur, wing engraving, jewelry, scenery, and feather-like anatomy. The revised Moth is defined by clubbed antennae, a round face, tiny mouthparts, folded wing shoulders, and square-PFP proportions.

The revised output is a 3×2 sheet of consistent head-and-shoulders portraits. Each portrait uses the same camera, scale, anatomy, coarse pixel density, and border. The canonical light effect appears primarily as simple background geometry and produces only a small matching reflection on the character.

Constraints included: preserve Moth anatomy and antennae; no ape, owl, or mole features; no earrings; no card frames; no photorealism or 3D rendering; no third-party NFT logos; and sufficient silhouette and facial clarity to remain readable as a 64 px wallet avatar.

## Files

- `moth-variant-study.png` — current simplified concept sheet
- `moth-variant-study-detailed.png` — archived high-detail direction
- `COLLECTION-SPEC.md` — authoritative first-release trait, canvas and supply specification
- `mole-owl-moth-study.png` — original three-species direction sheet
