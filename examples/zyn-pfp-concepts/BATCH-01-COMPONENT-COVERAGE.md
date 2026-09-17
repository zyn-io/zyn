# Zyn Collection 01 — component and reserve audit

This audit covers the released 889 Moths, 1,333 Owls and 2,222 Moles, together
with their prepared visual-master packs.

## Important provenance limitation

The finished NFTs were authored as complete portraits in canvas cells. They were
not assembled from independent component PNGs. Release metadata records the
source canvas, row, column and broad Visual Family, but it does not record exact
Background, Body, Mouth, Eyes, Headgear or Light master IDs.

Therefore this audit does **not** invent exact component usage counts. It records:

- exact released and unused whole-portrait sources;
- exact source-family counts;
- light directions explicitly named by released source families; and
- every prepared component as individually untracked where the saved provenance
  cannot prove its use.

`BATCH-01-ASSET-COVERAGE.json` is the complete machine-readable ledger for all
300 prepared visual masters.

## Verified inventories

| Creature | Released NFTs | Prepared masters | Clean unused portraits | Rejected source cells |
|---|---:|---:|---:|---:|
| Moth | 889 | 60 | 49 | 9 |
| Owl | 1,333 | 90 | 199 | 60 |
| Mole | 2,222 | 150 | 68 | 247 |
| **Total** | **4,444** | **300** | **316** | **316** |

All 4,444 released images are unique 192×192 PNGs with matching metadata. All
300 prepared masters are also unique 192×192 PNGs.

## Definitely unused whole portraits

These are safe, exact later-batch assets because they were not assigned a Batch
1 token:

- **Moth:** 49 cells in the Color Reserve board. Exact coordinates are in
  `moth-collection-889/reserve-map.json`.
- **Owl:** 199 cells: nine Royal Vault, 100 Rare Signals and 90 Full-Spectrum
  Reserve. Exact coordinates are in `owl-collection-1333/reserve-map.json`.
- **Mole:** 68 cells in Clean Geometry Reserve C. Exact coordinates are in
  `mole-collection-2222/reserve-map.json`.

Rejected cells are not reserves and must never be reused.

## Explicit light-direction coverage

These statuses mean the named direction appears in a released source-family
prompt. They do not mean every master PNG was copied exactly.

### Moth

Theme-confirmed: Aurora, Prism, Moonlight, Starlight, Bioluminescence, Caustics,
Afterglow and Eclipse.

Priority later-batch candidates: **Sunlight, Laser and Veiled Light**. Neutral is
visually common but was not separately tagged.

### Owl

Theme-confirmed: Moonlight, Starlight, Aurora, Bioluminescence, Sunlight, Dawn
Beam, Afterglow, Eclipse, Prism, Laser, Caustics, Veiled Light, Nebula Glow,
Comet Trail, Lantern Gold and Voidlight.

Priority later-batch candidate: **Constellation Sweep**. Its closest dedicated
source direction is in the unreleased Full-Spectrum reserve. Neutral is visually
common but was not separately tagged.

### Mole

All 30 prepared light directions have a corresponding released source-family
theme, including Neutral QA and the geological/cipher subvariants. They should
be treated as Batch 1 visual territory unless deliberately redesigned.

## Other component categories

The overview sheets visibly demonstrate broad use of the prepared palette and
silhouette language: dark, brown, silver, cream, cyan and violet bodies; crowns,
hoods, helmets, caps, halos, headsets and visors; and round, sleepy, prism, void,
goggle and luminous eye treatments all appear across the three releases.

Exact named master usage is still unproven for Body, Mouth/Beak, Eyes, Headgear
and Background. Subtle mouth/beak variants are especially impossible to audit
reliably from the current overview sheets. Every one of those master IDs is
listed as `not-individually-tracked` in the JSON ledger rather than incorrectly
calling it used or unused.

## Rule for Collection 02 and later

1. Start with the 316 exact unused whole portraits above.
2. Give first thematic priority to Moth Sunlight/Laser/Veiled Light and Owl
   Constellation Sweep.
3. Before reusing another master direction, manually tag Batch 1 at the component
   level or make the later-batch version visibly different.
4. For every new portrait, save a recipe record at authoring time with exact
   Background, Body, Mouth/Beak, Eyes, Headgear and Light IDs. The recipe is
   provenance metadata only; the artwork should still be authored and sliced as
   one atomic portrait.
