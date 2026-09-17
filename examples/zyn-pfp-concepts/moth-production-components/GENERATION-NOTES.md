# Canvas generation notes

Source mode: built-in image generation for complete authored canvases, followed by
deterministic cropping with `tools/slice_moth_canvases.py`.

Reference roles:

- `references/approved-layered-moths.png`: strict finished-PFP quality, framing,
  material, and character reference.
- `references/approved-moth-layer-master.png`: strict anatomy, 35-degree view,
  accessory fit, and background-first light reference.

Prompt set used for the approved canvases:

1. Ten complete body-direction PFPs using one large Moth silhouette.
2. Ten complete eye-direction PFPs preserving the near/far eye hierarchy.
3. Seven complete mouth-direction PFPs plus one empty reference cell.
4. Nine complete fitted-headgear PFPs.
5. Twelve background-only canvases with calm portrait centers.
6. Neutral plus eleven complete static-light PFPs with readable faces.

Global constraints: match the supplied Moth; large rounded head; 35-degree
right-facing view; clubbed gold antennae; broad overlapping wing shoulders;
polished pixel clusters; black/graphite/cyan/teal/cream/violet/Zcash-gold palette;
no tiny sprite, insect realism, painterly blur, or independent overlay assembly.

