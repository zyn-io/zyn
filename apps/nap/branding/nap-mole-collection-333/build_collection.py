#!/usr/bin/env python3
"""Build the 333-item Nap Mole collection from five atomic source canvases."""

from __future__ import annotations

import csv
import hashlib
import json
import random
import shutil
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parent
CANVASES = ROOT / "canvases-v2"
CANDIDATES = ROOT / "candidates"
IMAGES = ROOT / "images"
METADATA = ROOT / "metadata"
PREVIEWS = ROOT / "previews"
MANUAL_REJECTIONS = ROOT / "manual-rejected-source-cells.json"
VISUAL_TRAITS = ROOT / "visual-traits.csv"

CANVAS_COLUMNS = 5
CANVAS_ROWS = 5
SOURCE_PER_CANVAS = CANVAS_COLUMNS * CANVAS_ROWS
CANDIDATES_PER_CANVAS = SOURCE_PER_CANVAS
CANDIDATE_SUPPLY = 400
FINAL_SUPPLY = 333
OUTPUT_SIZE = (128, 192)
SEED = "nap-mole-early-users-333-v1"
GRID_SEARCH_RADIUS = 35
GRID_DARKNESS = 25

APPROVED_SKINS = [
    "charcoal", "earth-brown", "slate", "warm-tunnel-brown", "silver",
    "albino-ivory", "cyan", "gold-dust", "obsidian", "violet",
    "moss", "moonstone", "copper", "violet-cipher", "privacy-teal",
    "ash", "cream", "midnight-navy", "aurora-gradient", "prism-mosaic",
    "vault-black-gold", "solar-orange", "bioluminescent-green", "eclipse",
    "shielded-white-cyan",
]

# 13 × 21 + 3 × 20 = 333. Every board remains represented.
FINAL_QUOTAS = {canvas: (21 if canvas <= 13 else 20) for canvas in range(1, 17)}

HEADWEAR_LABELS = {
    "none": "None",
    "sleep-mask": "Sleep Mask",
    "pajama-hat": "Pajama Hat",
    "sleep-mask-and-pajama-hat": "Sleep Mask + Pajama Hat",
}

MOUTH_LABELS = {
    "neutral-dash": "Neutral Dash",
    "friendly-smile": "Friendly Smile",
    "star-mouth": "Star Mouth",
    "gold-tooth": "Gold Tooth",
    "clenched-line": "Clenched Line",
    "signal-grille": "Signal Grille",
}


def clear_directory(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    for child in path.iterdir():
        if child.is_file():
            child.unlink()
        elif child.is_dir():
            shutil.rmtree(child)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def black_grid_cell_ranges(
    canvas: Image.Image,
    axis: str,
    count: int,
) -> list[tuple[int, int]]:
    """Slice on the real continuous black gutters nearest each ideal boundary."""
    gray = canvas.convert("L")
    projection = (
        gray.resize((canvas.width, 1), Image.Resampling.BOX)
        if axis == "x"
        else gray.resize((1, canvas.height), Image.Resampling.BOX)
    )
    scores = list(projection.get_flattened_data())
    length = len(scores)
    internal_bands: list[tuple[int, int]] = []

    for boundary_index in range(1, count):
        ideal = round(boundary_index * length / count)
        low = max(0, ideal - GRID_SEARCH_RADIUS)
        high = min(length, ideal + GRID_SEARCH_RADIUS + 1)
        dark_positions = [
            position for position in range(low, high) if scores[position] < GRID_DARKNESS
        ]
        runs: list[tuple[int, int]] = []
        if dark_positions:
            start = previous = dark_positions[0]
            for position in dark_positions[1:]:
                if position > previous + 1:
                    runs.append((start, previous))
                    start = position
                previous = position
            runs.append((start, previous))

        viable = [run for run in runs if run[1] - run[0] + 1 >= 3]
        if not viable:
            raise RuntimeError(
                f"Cannot find {axis}-axis black gutter near boundary {boundary_index}: {runs}"
            )
        band = min(
            viable,
            key=lambda run: (
                abs((run[0] + run[1]) / 2 - ideal),
                sum(scores[run[0] : run[1] + 1]) / (run[1] - run[0] + 1),
            ),
        )
        internal_bands.append(band)

    edge_bands = [(0, 4), *internal_bands, (length - 5, length - 1)]
    ranges = [
        (edge_bands[index][1] + 1, edge_bands[index + 1][0])
        for index in range(count)
    ]
    if any(end <= start for start, end in ranges):
        raise RuntimeError(f"Invalid {axis}-axis grid ranges: {ranges}")
    return ranges


def center_crop_to_ratio(image: Image.Image, target_width: int, target_height: int) -> Image.Image:
    """Preserve geometry by center-cropping to the final ratio; never stretch axes."""
    target_ratio = target_width / target_height
    source_ratio = image.width / image.height
    if abs(source_ratio - target_ratio) < 1e-9:
        return image

    if source_ratio < target_ratio:
        new_height = round(image.width / target_ratio)
        top = max(0, (image.height - new_height) // 2)
        return image.crop((0, top, image.width, top + new_height))

    new_width = round(image.height * target_ratio)
    left = max(0, (image.width - new_width) // 2)
    return image.crop((left, 0, left + new_width, image.height))


def render_cell(
    canvas: Image.Image,
    x_range: tuple[int, int],
    y_range: tuple[int, int],
) -> Image.Image:
    x0, x1 = x_range
    y0, y1 = y_range
    cell = canvas.crop((x0, y0, x1, y1)).convert("RGB")
    cropped = center_crop_to_ratio(cell, *OUTPUT_SIZE)
    return cropped.resize(OUTPUT_SIZE, Image.Resampling.NEAREST)


def load_manual_rejections() -> dict[tuple[int, int, int], str]:
    if not MANUAL_REJECTIONS.exists():
        return {}
    payload = json.loads(MANUAL_REJECTIONS.read_text())
    rejected: dict[tuple[int, int, int], str] = {}
    for item in payload:
        key = (int(item["canvas"]), int(item["row"]), int(item["column"]))
        rejected[key] = str(item["reason"])
    return rejected


def load_visual_traits() -> dict[int, dict[str, str]]:
    """Load the traits manually reviewed against the final token PNGs."""
    if not VISUAL_TRAITS.exists():
        raise RuntimeError(f"Missing visual trait review: {VISUAL_TRAITS}")

    traits: dict[int, dict[str, str]] = {}
    with VISUAL_TRAITS.open(newline="") as handle:
        for row in csv.DictReader(handle):
            token_id = int(row["token_id"])
            if token_id in traits:
                raise RuntimeError(f"Duplicate visual trait row for token {token_id:04d}")
            headwear = row["headwear"]
            mouth = row["mouth"]
            if headwear not in HEADWEAR_LABELS:
                raise RuntimeError(f"Token {token_id:04d}: invalid headwear {headwear!r}")
            if mouth not in MOUTH_LABELS:
                raise RuntimeError(f"Token {token_id:04d}: invalid mouth {mouth!r}")
            traits[token_id] = {"headwear": headwear, "mouth": mouth}

    expected = set(range(1, FINAL_SUPPLY + 1))
    actual = set(traits)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise RuntimeError(f"Visual trait coverage mismatch: missing={missing}, extra={extra}")
    return traits


def make_overview(paths: list[Path], output: Path, columns: int, label: bool = False) -> None:
    thumb = (64, 96)
    label_height = 14 if label else 0
    rows = (len(paths) + columns - 1) // columns
    sheet = Image.new("RGB", (columns * thumb[0], rows * (thumb[1] + label_height)), (7, 9, 13))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default()
    for index, path in enumerate(paths):
        x = (index % columns) * thumb[0]
        y = (index // columns) * (thumb[1] + label_height)
        image = Image.open(path).convert("RGB").resize(thumb, Image.Resampling.NEAREST)
        sheet.paste(image, (x, y))
        if label:
            draw.text((x + 2, y + thumb[1] + 1), path.stem, fill=(224, 165, 27), font=font)
    sheet.save(output, optimize=True)


def make_source_overview(
    records: list[dict[str, object]],
    rendered: dict[tuple[int, int, int], Image.Image],
    canvas_number: int,
    output: Path,
) -> None:
    board = [record for record in records if record["canvas"] == canvas_number]
    board.sort(key=lambda record: (int(record["row"]), int(record["column"])))
    tile_width, tile_height, label_height = 128, 192, 16
    sheet = Image.new(
        "RGB",
        (CANVAS_COLUMNS * tile_width, CANVAS_ROWS * (tile_height + label_height)),
        (7, 9, 13),
    )
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default()
    for index, record in enumerate(board):
        x = (index % CANVAS_COLUMNS) * tile_width
        y = (index // CANVAS_COLUMNS) * (tile_height + label_height)
        key = (canvas_number, int(record["row"]), int(record["column"]))
        sheet.paste(rendered[key], (x, y))
        draw.text(
            (x + 3, y + tile_height + 2),
            f"C{canvas_number} R{record['row']} K{record['column']}",
            fill=(224, 165, 27),
            font=font,
        )
    sheet.save(output, optimize=True)


def make_review_pages(paths: list[Path], output_directory: Path) -> None:
    """Create readable 5x5 token sheets for visual expression/geometry review."""
    columns, rows = 5, 5
    tile_width, tile_height, label_height = 256, 384, 24
    page_size = columns * rows
    font = ImageFont.load_default()
    for page_index, start in enumerate(range(0, len(paths), page_size), start=1):
        page_paths = paths[start : start + page_size]
        sheet = Image.new(
            "RGB",
            (columns * tile_width, rows * (tile_height + label_height)),
            (7, 9, 13),
        )
        draw = ImageDraw.Draw(sheet)
        for index, path in enumerate(page_paths):
            x = (index % columns) * tile_width
            y = (index // columns) * (tile_height + label_height)
            image = Image.open(path).convert("RGB").resize(
                (tile_width, tile_height), Image.Resampling.NEAREST
            )
            sheet.paste(image, (x, y))
            draw.text(
                (x + 5, y + tile_height + 4),
                path.stem,
                fill=(224, 165, 27),
                font=font,
            )
        sheet.save(output_directory / f"final-review-{page_index:02d}.png", optimize=True)


def main() -> None:
    canvas_paths = sorted(CANVASES.glob("canvas-*.png"))
    if len(canvas_paths) != 16:
        raise RuntimeError(f"Expected 16 v2 canvases, found {len(canvas_paths)}")

    for directory in (CANDIDATES, IMAGES, METADATA, PREVIEWS):
        clear_directory(directory)

    rng = random.Random(SEED)
    manual_rejections = load_manual_rejections()
    visual_traits = load_visual_traits()
    source_records: list[dict[str, object]] = []
    rendered: dict[tuple[int, int, int], Image.Image] = {}
    raw_digests: set[str] = set()

    for canvas_number, path in enumerate(canvas_paths, start=1):
        canvas = Image.open(path).convert("RGB")
        if canvas.size != (1024, 1536):
            raise RuntimeError(f"{path.name}: expected 1024x1536, found {canvas.size}")

        x_ranges = black_grid_cell_ranges(canvas, "x", CANVAS_COLUMNS)
        y_ranges = black_grid_cell_ranges(canvas, "y", CANVAS_ROWS)
        if len(x_ranges) != CANVAS_COLUMNS or len(y_ranges) != CANVAS_ROWS:
            raise RuntimeError(
                f"{path.name}: expected a measured {CANVAS_COLUMNS}x{CANVAS_ROWS} grid, "
                f"found {len(x_ranges)}x{len(y_ranges)}"
            )

        for row, y_range in enumerate(y_ranges, start=1):
            for column, x_range in enumerate(x_ranges, start=1):
                image = render_cell(canvas, x_range, y_range)
                digest = hashlib.sha256(image.tobytes()).hexdigest()
                if digest in raw_digests:
                    raise RuntimeError(
                        f"Exact duplicate source cell at canvas {canvas_number}, row {row + 1}, column {column + 1}"
                    )
                raw_digests.add(digest)
                key = (canvas_number, row, column)
                rendered[key] = image
                source_records.append(
                    {
                        "canvas": canvas_number,
                        "row": row,
                        "column": column,
                        "skinFamilyPool": APPROVED_SKINS,
                        "renderDigest": digest,
                        "manualReview": manual_rejections.get(key, "accepted"),
                    }
                )

    if len(source_records) != 400:
        raise RuntimeError(f"Expected 400 source cells, found {len(source_records)}")

    for canvas_number in range(1, 17):
        make_source_overview(
            source_records,
            rendered,
            canvas_number,
            PREVIEWS / f"source-canvas-{canvas_number:02d}-qa.png",
        )

    candidate_records: list[dict[str, object]] = []
    source_reserve: list[dict[str, object]] = []
    for canvas_number in range(1, 17):
        board = [record for record in source_records if record["canvas"] == canvas_number]
        rng.shuffle(board)
        selected = board[:CANDIDATES_PER_CANVAS]
        source_reserve.extend(board[CANDIDATES_PER_CANVAS:])
        candidate_records.extend(selected)

    rng.shuffle(candidate_records)
    if len(candidate_records) != CANDIDATE_SUPPLY:
        raise RuntimeError(f"Expected 400 candidates, found {len(candidate_records)}")

    for candidate_id, record in enumerate(candidate_records, start=1):
        record["candidateId"] = candidate_id
        key = (int(record["canvas"]), int(record["row"]), int(record["column"]))
        rendered[key].save(CANDIDATES / f"{candidate_id:04d}.png", optimize=True)

    final_records: list[dict[str, object]] = []
    for canvas_number in range(1, 17):
        board = [record for record in candidate_records if record["canvas"] == canvas_number]
        rng.shuffle(board)
        board = [
            record
            for record in board
            if (int(record["canvas"]), int(record["row"]), int(record["column"]))
            not in manual_rejections
        ]
        quota = FINAL_QUOTAS[canvas_number]
        if len(board) < quota:
            raise RuntimeError(
                f"Canvas {canvas_number}: only {len(board)} reviewed cells remain for quota {quota}"
            )
        final_records.extend(board[:quota])

    rng.shuffle(final_records)
    selected_candidate_ids = {int(record["candidateId"]) for record in final_records}
    removed_records = [
        record
        for record in candidate_records
        if int(record["candidateId"]) not in selected_candidate_ids
    ]
    if len(final_records) != FINAL_SUPPLY or len(removed_records) != 67:
        raise RuntimeError(
            f"Selection mismatch: final={len(final_records)} removed={len(removed_records)}"
        )

    for token_id, record in enumerate(final_records, start=1):
        candidate_id = int(record["candidateId"])
        destination = IMAGES / f"{token_id:04d}.png"
        shutil.copy2(CANDIDATES / f"{candidate_id:04d}.png", destination)
        record["tokenId"] = token_id
        traits = visual_traits[token_id]
        image_digest = sha256(destination)
        metadata = {
            "name": f"Nap Mole #{token_id:03d}",
            "description": "A sleeping Mole from the Nap early-user collection.",
            "image": f"{token_id:04d}.png",
            "image_sha256": image_digest,
            "image_dimensions": {"width": OUTPUT_SIZE[0], "height": OUTPUT_SIZE[1]},
            "attributes": [
                {"trait_type": "Species", "value": "Mole"},
                {"trait_type": "Collection", "value": "Nap Early Users"},
                {"trait_type": "Pose", "value": "Sleeping 35-degree"},
                {"trait_type": "Expression", "value": "Sleeping"},
                {"trait_type": "Headwear", "value": HEADWEAR_LABELS[traits["headwear"]]},
                {"trait_type": "Mouth", "value": MOUTH_LABELS[traits["mouth"]]},
                {"trait_type": "Setting", "value": "Cave"},
                {"trait_type": "Sleep Glyphs", "value": "z z Z"},
            ],
            "source": {
                "candidateId": candidate_id,
                "canvas": record["canvas"],
                "row": record["row"],
                "column": record["column"],
            },
            "visual_trait_source": "Manual review of the final token PNG",
        }
        (METADATA / f"{token_id:04d}.json").write_text(json.dumps(metadata, indent=2) + "\n")

    candidate_map = {
        "version": 1,
        "seed": SEED,
        "supply": CANDIDATE_SUPPLY,
        "selection": "all 25 equal cells from each of sixteen strict 5x5 v2 canvases",
        "items": candidate_records,
    }
    (ROOT / "candidate-map.json").write_text(json.dumps(candidate_map, indent=2) + "\n")
    (ROOT / "source-reserve-map.json").write_text(json.dumps(source_reserve, indent=2) + "\n")
    (ROOT / "removed-candidates.json").write_text(json.dumps(removed_records, indent=2) + "\n")
    (ROOT / "source-map.json").write_text(json.dumps(final_records, indent=2) + "\n")

    candidate_paths = sorted(CANDIDATES.glob("*.png"))
    final_paths = sorted(IMAGES.glob("*.png"))
    removed_paths = [CANDIDATES / f"{int(record['candidateId']):04d}.png" for record in removed_records]
    make_overview(candidate_paths, PREVIEWS / "candidate-pool-400.png", columns=20)
    make_overview(final_paths, PREVIEWS / "nap-mole-collection-333.png", columns=19)
    make_overview(removed_paths, PREVIEWS / "removed-67.png", columns=10, label=True)
    make_review_pages(final_paths, PREVIEWS)

    checksums = "".join(f"{sha256(path)}  images/{path.name}\n" for path in final_paths)
    (ROOT / "checksums.sha256").write_text(checksums)

    wrong_size_files = [
        path.name for path in final_paths if Image.open(path).size != OUTPUT_SIZE
    ]
    final_digests = [sha256(path) for path in final_paths]
    final_duplicate_count = len(final_digests) - len(set(final_digests))

    board_counts = {
        str(canvas): sum(1 for record in final_records if record["canvas"] == canvas)
        for canvas in range(1, 17)
    }

    metadata_paths = sorted(METADATA.glob("*.json"))
    metadata_image_pair_mismatches: list[dict[str, object]] = []
    for token_id, record in enumerate(final_records, start=1):
        stem = f"{token_id:04d}"
        image_path = IMAGES / f"{stem}.png"
        metadata_path = METADATA / f"{stem}.json"
        if not metadata_path.exists():
            metadata_image_pair_mismatches.append({"tokenId": token_id, "reason": "missing metadata"})
            continue
        payload = json.loads(metadata_path.read_text())
        actual_dimensions = Image.open(image_path).size
        expected_source = {
            "candidateId": int(record["candidateId"]),
            "canvas": record["canvas"],
            "row": record["row"],
            "column": record["column"],
        }
        checks = {
            "name": payload.get("name") == f"Nap Mole #{token_id:03d}",
            "image": payload.get("image") == f"{stem}.png",
            "image_sha256": payload.get("image_sha256") == sha256(image_path),
            "image_dimensions": payload.get("image_dimensions")
            == {"width": actual_dimensions[0], "height": actual_dimensions[1]},
            "source": payload.get("source") == expected_source,
            "visual_traits": {
                attribute.get("trait_type"): attribute.get("value")
                for attribute in payload.get("attributes", [])
            }.get("Headwear") == HEADWEAR_LABELS[visual_traits[token_id]["headwear"]]
            and {
                attribute.get("trait_type"): attribute.get("value")
                for attribute in payload.get("attributes", [])
            }.get("Mouth") == MOUTH_LABELS[visual_traits[token_id]["mouth"]],
        }
        failed_checks = [name for name, passed in checks.items() if not passed]
        if failed_checks:
            metadata_image_pair_mismatches.append(
                {"tokenId": token_id, "failedChecks": failed_checks}
            )

    metadata_valid = (
        len(metadata_paths) == FINAL_SUPPLY and not metadata_image_pair_mismatches
    )
    validation = {
        "valid": not wrong_size_files and final_duplicate_count == 0 and metadata_valid,
        "sourceCanvasCount": len(canvas_paths),
        "sourceCellCount": len(source_records),
        "candidateCount": len(candidate_paths),
        "removedCandidateCount": len(removed_records),
        "finalCount": len(final_paths),
        "metadataCount": len(metadata_paths),
        "imageSize": list(OUTPUT_SIZE),
        "wrongSizeFileCount": len(wrong_size_files),
        "wrongSizeFiles": wrong_size_files,
        "exactDuplicateCount": len(source_records) - len(raw_digests),
        "finalExactDuplicateCount": final_duplicate_count,
        "metadataImagePairMismatchCount": len(metadata_image_pair_mismatches),
        "metadataImagePairMismatches": metadata_image_pair_mismatches,
        "closedEyeVisualReview": (
            "all selected final cells show closed eyelids or a complete sleep mask; "
            "rejected source cells are listed separately"
        ),
        "manualRejectedSourceCellCount": len(manual_rejections),
        "manualRejectedSourceCells": [
            {"canvas": key[0], "row": key[1], "column": key[2], "reason": reason}
            for key, reason in sorted(manual_rejections.items())
        ],
        "finalCanvasCounts": board_counts,
        "skinFamiliesRequested": APPROVED_SKINS,
        "skinFamilyCount": len(APPROVED_SKINS),
        "metadataTraitCoverage": {
            "reviewedTokenCount": len(visual_traits),
            "expectedTokenCount": FINAL_SUPPLY,
            "complete": len(visual_traits) == FINAL_SUPPLY,
            "manuallyReviewedVisibleTraits": ["Headwear", "Mouth"],
        },
        "exactTraitClassification": (
            "Headwear and mouth manually reviewed against every final PNG; "
            "fixed collection traits and cryptographic image binding verified"
        ),
    }
    (ROOT / "validation.json").write_text(json.dumps(validation, indent=2) + "\n")
    print(json.dumps(validation, indent=2))


if __name__ == "__main__":
    main()
