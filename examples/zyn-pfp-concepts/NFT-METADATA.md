# Zyn NFT metadata and viewer contract

Collection 01 uses `zyn-atomic-components/v1` metadata and the `zyn-light-v1`
interactive renderer.

## Store filters available now

- `Species`
- `Visual Family`
- `Interactive Light`

All three species have completed visual-category review and can also be filtered by:

- `Background`
- `Body`
- `Eyes`
- `Mouth` (Moths and Moles)
- `Beak` (Owls)
- `Headgear`

The human-readable light label and stable machine ID are both included in each
token's standard `attributes` array. `STORE-FILTERS.json` contains verified counts
for building filter menus without scanning all 4,444 files at runtime.

## Viewer contract

Every token includes:

```json
"viewer": {
  "renderer": "zyn-light-v1",
  "effectId": "afterglow",
  "effectIndex": 9,
  "interactive": true
}
```

The stable contract is `effectId`; `effectIndex` is included for simple clients
but must agree with the renderer registry. The web viewer accepts a metadata URL:

```text
/?metadata=/path/to/token-metadata.json
```

It loads `viewer.effectId`, selects that effect, and still allows the owner to
preview the other effects from the menu. The store or Nap Wallet should resolve
the token metadata URI, verify `viewer.renderer`, and pass the metadata URL or
effect ID to its viewer instance.

## Component fields

Each token also includes a structured `components` object. `light` is assigned
and usable now. All 4,444 Moths, Owls and Moles have visually categorized
Background, Body, Eyes, Mouth/Beak and Headgear IDs. These labels describe the
visible artwork; they do not claim that the atomic portraits were assembled from
those source layers.

Do not expose null fields as store filters. The complete allowed option lists are
in each collection's `trait-schema.json`. `STORE-FILTERS.json` indicates which
species/category combinations are ready.

Run the validator and rebuild the filter counts with:

```sh
python3 examples/zyn-pfp-concepts/tools/validate_store_metadata.py
```
