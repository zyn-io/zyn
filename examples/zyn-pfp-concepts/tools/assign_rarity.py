#!/usr/bin/env python3
"""Rarity classes for Collection Z, from the traits alone.

Each item scores the information in its traits: for every slot, how surprising
its value is across all 4,444 (-log2 of the share of items that have it),
summed over seven slots — Species, Background, Body, Eyes, Mouth (Beak for
owls: one slot, since every item has exactly one), Headgear, Interactive
Light. A trait carried by 1 item in 4,444 adds ~12 bits; one carried by half
the collection adds 1.

Items are ranked by score, highest first; equal scores rank by collection
index (Mole 0..2221, Owl 2222..3554, Moth 3555..4443, the manifest order).
Classes are cut by rank, not by score, so their sizes are fixed:

    Mythic      top 1%          44
    Legendary   next 4%        178
    Epic        next 10%       445
    Rare        next 20%       888
    Uncommon    next 30%     1,334
    Common      the rest     1,555

Visual Family is left out: it names the production batch an item came from
("Clean Geometry Reserve C"), not something a holder sees. Canvas position and
the tagging marker are bookkeeping.

Writes RARITY.json (kept in git; the metadata is not) and adds to each metadata file two attributes — `Rarity` and `Rarity Rank` — which
enter the manifest preimage, and a `rarity` block with the score. Rerun after
any component re-review: rarity is derived from the tags, and the store
validator refuses metadata whose rarity no longer matches them.

    python3 tools/assign_rarity.py          # write
    python3 tools/assign_rarity.py --check  # verify only
"""

import collections
import json
import math
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
# Manifest order: the index is identity (build_collection_z_manifests.py).
SPECIES = [("Mole", "mole-collection-2222"), ("Owl", "owl-collection-1333"), ("Moth", "moth-collection-889")]
SLOTS = ["Species", "Background", "Body", "Eyes", "Mouth", "Headgear", "Interactive Light"]
CLASSES = [("Mythic", 0.01), ("Legendary", 0.05), ("Epic", 0.15), ("Rare", 0.35), ("Uncommon", 0.65), ("Common", 1.0)]
SCHEMA = "zyn-cavez-rarity/v1"
METHOD = (
    "information content: sum of -log2(share of the 4,444 with that value) over species, background, "
    "body, eyes, mouth/beak, headgear and interactive light; ranked highest first, ties by collection index; "
    "classes by rank: 1% Mythic, 4% Legendary, 10% Epic, 20% Rare, 30% Uncommon, 35% Common"
)
RARITY_TRAITS = {"Rarity", "Rarity Rank"}


def load():
    items = []
    for species, folder in SPECIES:
        for path in sorted((ROOT / folder / "metadata").glob("*.json")):
            meta = json.loads(path.read_text())
            traits = {a["trait_type"]: a["value"] for a in meta["attributes"]}
            if "Beak" in traits:
                traits["Mouth"] = traits.pop("Beak")
            assert traits["Species"] == species, path
            items.append({"path": path, "meta": meta, "traits": traits, "index": len(items)})
    return items


def compute(items):
    """Global index -> (class, rank, score)."""
    n = len(items)
    freq = {s: collections.Counter(i["traits"][s] for i in items) for s in SLOTS}
    scores = [sum(-math.log2(freq[s][i["traits"][s]] / n) for s in SLOTS) for i in items]
    # Round before ranking so float noise cannot reorder equal scores.
    order = sorted(range(n), key=lambda k: (-round(scores[k], 9), k))
    cuts = [(name, round(share * n)) for name, share in CLASSES]
    out = {}
    for rank, k in enumerate(order, 1):
        name = next(c for c, upto in cuts if rank <= upto)
        out[k] = (name, rank, round(scores[k], 4))
    return out


def with_rarity(meta, klass, rank, score, of):
    attrs = [a for a in meta["attributes"] if a["trait_type"] not in RARITY_TRAITS]
    attrs += [{"trait_type": "Rarity", "value": klass}, {"trait_type": "Rarity Rank", "value": rank}]
    meta = {**meta, "attributes": attrs}
    meta["rarity"] = {"schema": SCHEMA, "class": klass, "rank": rank, "of": of, "score": score, "method": METHOD}
    return meta


def main():
    check = "--check" in sys.argv
    items = load()
    rarity = compute(items)
    stale = []
    for item in items:
        klass, rank, score = rarity[item["index"]]
        want = with_rarity(item["meta"], klass, rank, score, len(items))
        if want != item["meta"]:
            stale.append(item["path"])
            if not check:
                item["path"].write_text(json.dumps(want, indent=2) + "\n")
    counts = collections.Counter((rarity[i["index"]][0], i["traits"]["Species"]) for i in items)
    for name, _ in CLASSES:
        row = {sp: counts[(name, sp)] for sp, _ in SPECIES}
        print(f"{name:10s} {sum(row.values()):5d}  {row}")
    if check:
        if stale:
            sys.exit(f"{len(stale)} metadata files do not carry their current rarity; run tools/assign_rarity.py")
        print("rarity: every item matches its traits")
    else:
        print(f"rarity: {len(stale)} files updated")
    if not check:
        # The record kept in git (metadata files are not): global index ->
        # class, rank, score, in manifest order.
        record = {
            "schema": SCHEMA, "method": METHOD, "supply": len(items),
            "classes": {name: sum(counts[(name, sp)] for sp, _ in SPECIES) for name, _ in CLASSES},
            "items": [
                {"index": i["index"], "name": i["meta"]["name"], "class": rarity[i["index"]][0],
                 "rank": rarity[i["index"]][1], "score": rarity[i["index"]][2]}
                for i in items
            ],
        }
        head = json.dumps({k: v for k, v in record.items() if k != "items"}, indent=1)[:-2]
        rows = ",\n  ".join(json.dumps(r, separators=(",", ":")) for r in record["items"])
        (ROOT / "RARITY.json").write_text(f'{head},\n "items": [\n  {rows}\n ]\n}}\n')
        for species, folder in SPECIES:
            path = ROOT / folder / "collection.json"
            coll = json.loads(path.read_text())
            coll["rarity"] = {"schema": SCHEMA, "method": METHOD, "classes": {
                name: counts[(name, species)] for name, _ in CLASSES}}
            if "Rarity" not in coll["storeFilters"]:
                coll["storeFilters"] = [*coll["storeFilters"], "Rarity"]
            path.write_text(json.dumps(coll, indent=2) + "\n")


if __name__ == "__main__":
    main()
