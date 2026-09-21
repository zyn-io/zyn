#!/usr/bin/env python3
"""Build `zyn.manifest.v1` manifests and content hashes for the 333.

Mirrors `zyn_vm::manifest::Manifest::content` exactly. The Rust side is the
normative one; this exists so the collection can be prepared without a build,
and `cargo test -p zyn-vm manifest` plus `--check` below keep the two honest.

Writes, under `out/`:

    manifests/NNNN.bin   the committed preimage, byte for byte
    content.json         serial -> content hash, the `content` field to mint
    blobs/<sha256>       every media file, named by its own hash

`blobs/` is exactly what a gateway serves: objects keyed by their own hash, so
identical bytes deduplicate, anyone may mirror it, and moving providers touches
no token. Nothing here contains a URL — where the bytes live is a hint held
outside consensus (DECISIONS §67).
"""

import hashlib
import json
import pathlib
import shutil
import struct
import sys

HERE = pathlib.Path(__file__).parent
DOMAIN = b"zyn.manifest.v1"
FULL, PREVIEW = 0, 1

# `Encoder::leaf()` is `hash_leaf`, which domain-separates a leaf from an
# internal node. Read from the Rust so the two cannot drift apart silently.
LEAF_PREFIX = b"\x00"


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


MIME = {".png": "image/png", ".gif": "image/gif", ".mp4": "video/mp4", ".webp": "image/webp"}


def build():
    meta_dir, img_dir = HERE / "metadata", HERE / "images"
    out = HERE / "out"
    (out / "manifests").mkdir(parents=True, exist_ok=True)
    (out / "blobs").mkdir(parents=True, exist_ok=True)

    index = {}
    for meta_path in sorted(meta_dir.glob("*.json")):
        serial = meta_path.stem
        meta = json.loads(meta_path.read_text())

        img_path = img_dir / meta["image"]
        raw = img_path.read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        if digest != meta["image_sha256"]:
            sys.exit(f"{serial}: image on disk does not match its recorded sha256")

        dims = meta.get("image_dimensions", {})
        media = [{
            "role": FULL,
            "sha256": digest,
            "mime": MIME.get(img_path.suffix.lower(), "application/octet-stream"),
            "bytes": len(raw),
            "width": dims.get("width", 0),
            "height": dims.get("height", 0),
            "duration_ms": 0,
        }]

        pre = preimage(meta["name"], meta["description"], media, meta.get("attributes", []))
        (out / "manifests" / f"{serial}.bin").write_bytes(pre)
        shutil.copyfile(img_path, out / "blobs" / digest)
        # The manifest is a blob too, and the wallet asks for it by the *content*
        # hash the item committed — leaf-prefixed, not the plain digest of these
        # bytes. Without this the gateway serves art nobody can find.
        (out / "blobs" / content_hash(pre)).write_bytes(pre)

        index[serial] = {
            "content": content_hash(pre),
            "name": meta["name"],
            "media": [{"role": "full", "sha256": digest, "bytes": len(raw)}],
        }

    (out / "content.json").write_text(json.dumps(index, indent=2, sort_keys=True) + "\n")
    blobs = list((out / "blobs").iterdir())
    print(f"built {len(index)} manifests")
    print(f"  blobs:   {len(blobs)} objects, {sum(b.stat().st_size for b in blobs) / 1e6:.1f} MB")
    print(f"  content: {out / 'content.json'}")
    first = sorted(index)[0]
    print(f"  {index[first]['name']}  content {index[first]['content']}")
    return index


if __name__ == "__main__":
    build()
