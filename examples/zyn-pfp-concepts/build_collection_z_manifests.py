#!/usr/bin/env python3
"""Build `zyn.manifest.v1` manifests and content hashes for Collection Z (CAVEZ).

The 333's `apps/nap/branding/nap-mole-collection-333/build_manifests.py` is the
original; this is the same preimage, adapted for a collection that arrives as
three separately generated species rather than one directory.

Three adaptations, and they are the whole difference:

  * **One global index across three species.** Mole takes 0..=2221, Owl
    2222..=3554, Moth 3555..=4443 — a fixed order, because the index is
    identity: `item_address(ADDRESS_SCOPE_V1, CAVEZ, index)` is derived from it
    and the manifest root commits to it. Reordering the species after
    publication would rename every item. The order is arbitrary but it is not
    revisable.

  * **The hash lives in `checksums.sha256`**, not inline in each metadata file,
    so it is read from there and still verified against the bytes on disk.

  * **Attribute values are stringified.** `Canvas Row: 1` is an integer in the
    source metadata and a string in the preimage, which encodes every value as
    UTF-8 bytes. Left as an int it would raise rather than silently differ.

`name` keeps the species-local number it was generated with ("Zyn Mole #0001"),
which is display text; the global index is identity. The two differ on purpose
and only the index is committed.

Writes, under `out/`:

    manifests/NNNN.bin   the committed preimage, byte for byte, by global index
    content.json         global index -> content hash and provenance
    blobs/<sha256>       every image and every manifest, named by its own hash

Run `cargo run -p zynzapd --bin zyn-cave-manifest` afterwards to derive the item
ids and the manifest root from `content.json` using the normative Rust.
"""

import hashlib
import json
import pathlib
import struct
import sys

HERE = pathlib.Path(__file__).parent
DOMAIN = b"zyn.manifest.v1"
FULL = 0
LEAF_PREFIX = b"\x00"

# Fixed, and fixed forever once published: the global index is identity.
SPECIES = [
    ("mole-collection-2222", 2222),
    ("owl-collection-1333", 1333),
    ("moth-collection-889", 889),
]
SUPPLY = 4444


def _u8(v):
    return struct.pack(">B", v)


def _u32(v):
    return struct.pack(">I", v)


def _u64(v):
    return struct.pack(">Q", v)


def _s(text):
    b = text.encode("utf-8")
    return _u32(len(b)) + b


def preimage(name, description, media, attributes):
    """The exact bytes `Manifest::content` hashes."""
    media = sorted(media, key=lambda m: (m["role"], m["sha256"]))
    attributes = sorted(attributes, key=lambda a: (a["trait_type"], a["value"]))

    out = bytearray(DOMAIN)
    out += _s(name)
    out += _s(description)
    out += _u32(len(media))
    for m in media:
        out += _u8(m["role"])
        out += bytes.fromhex(m["sha256"])
        out += _s(m["mime"])
        out += _u64(m["bytes"])
        out += _u32(m["width"])
        out += _u32(m["height"])
        out += _u32(m["duration_ms"])
    out += _u32(len(attributes))
    for a in attributes:
        out += _s(a["trait_type"])
        out += _s(a["value"])
    return bytes(out)


def content_hash(pre):
    return hashlib.sha256(LEAF_PREFIX + pre).hexdigest()


def png_size(raw):
    """Width and height from the IHDR chunk, which is always first."""
    if raw[:8] != b"\x89PNG\r\n\x1a\n" or raw[12:16] != b"IHDR":
        sys.exit("not a PNG, or IHDR is not the first chunk")
    return struct.unpack(">II", raw[16:24])


def read_checksums(path):
    """`<sha256>  images/NNNN.png` -> {"NNNN.png": sha}."""
    sums = {}
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        digest, _, name = line.partition("  ")
        sums[pathlib.PurePosixPath(name.strip()).name] = digest.strip()
    return sums


def stringify(attributes):
    """Every attribute value is UTF-8 in the preimage, so an int must become
    its decimal text here rather than raise inside `_s`."""
    out = []
    for a in attributes:
        v = a["value"]
        out.append({
            "trait_type": a["trait_type"],
            "value": v if isinstance(v, str) else json.dumps(v) if isinstance(v, (dict, list)) else str(v),
        })
    return out


def build():
    out = HERE / "out"
    (out / "manifests").mkdir(parents=True, exist_ok=True)
    (out / "blobs").mkdir(parents=True, exist_ok=True)

    index = {}
    global_index = 0
    for dirname, expected in SPECIES:
        src = HERE / dirname
        if not src.is_dir():
            sys.exit(f"missing species directory {src}")
        sums = read_checksums(src / "checksums.sha256")
        metas = sorted((src / "metadata").glob("*.json"))
        if len(metas) != expected:
            sys.exit(f"{dirname}: {len(metas)} metadata files, expected {expected}")

        for meta_path in metas:
            meta = json.loads(meta_path.read_text())
            img_name = meta["image"]
            img_path = src / "images" / img_name
            raw = img_path.read_bytes()
            digest = hashlib.sha256(raw).hexdigest()
            recorded = sums.get(img_name)
            if recorded is None:
                sys.exit(f"{dirname}/{img_name}: absent from checksums.sha256")
            if digest != recorded:
                sys.exit(f"{dirname}/{img_name}: bytes on disk do not match checksums.sha256")

            width, height = png_size(raw)
            media = [{
                "role": FULL,
                "sha256": digest,
                "mime": "image/png",
                "bytes": len(raw),
                "width": width,
                "height": height,
                "duration_ms": 0,
            }]

            pre = preimage(
                meta["name"],
                meta["description"],
                media,
                stringify(meta.get("attributes", [])),
            )
            content = content_hash(pre)

            (out / "manifests" / f"{global_index:04d}.bin").write_bytes(pre)
            (out / "blobs" / digest).write_bytes(raw)
            # The manifest is a blob too, and the wallet asks for it by the
            # *content* hash the item committed — leaf-prefixed, not the plain
            # digest of these bytes. Without this the gateway serves art nobody
            # can find.
            (out / "blobs" / content).write_bytes(pre)

            index[str(global_index)] = {
                "content": content,
                "name": meta["name"],
                "species": dirname.split("-")[0],
                "source": f"{dirname}/{img_name}",
                "media": [{"role": "full", "sha256": digest, "bytes": len(raw)}],
            }
            global_index += 1

    if global_index != SUPPLY:
        sys.exit(f"built {global_index} manifests, expected {SUPPLY}")

    (out / "content.json").write_text(json.dumps(index, indent=2, sort_keys=True) + "\n")
    blobs = list((out / "blobs").iterdir())
    print(f"built {len(index)} manifests, indices 0..={global_index - 1}")
    print(f"  blobs:   {len(blobs)} objects, {sum(b.stat().st_size for b in blobs) / 1e6:.1f} MB")
    print(f"  content: {out / 'content.json'}")
    for name, base in [("mole", 0), ("owl", 2222), ("moth", 3555)]:
        e = index[str(base)]
        print(f"  {base:>4}  {e['name']:<20} content {e['content'][:16]}…")
    return index


if __name__ == "__main__":
    build()
