"""Build transparent edge masks from the approved unrevealed card renders."""

from collections import deque
from pathlib import Path

import numpy as np
from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "assets"


def largest_component(binary: np.ndarray) -> np.ndarray:
    height, width = binary.shape
    visited = np.zeros_like(binary)
    largest: list[tuple[int, int]] = []

    for y in range(height):
        for x in range(width):
            if not binary[y, x] or visited[y, x]:
                continue
            queue = deque([(y, x)])
            visited[y, x] = True
            component: list[tuple[int, int]] = []
            while queue:
                py, px = queue.popleft()
                component.append((py, px))
                for ny, nx in ((py - 1, px), (py + 1, px), (py, px - 1), (py, px + 1)):
                    if 0 <= ny < height and 0 <= nx < width and binary[ny, nx] and not visited[ny, nx]:
                        visited[ny, nx] = True
                        queue.append((ny, nx))
            if len(component) > len(largest):
                largest = component

    mask = np.zeros_like(binary)
    if largest:
        points = np.asarray(largest)
        mask[points[:, 0], points[:, 1]] = True
    return mask


def make_edge_mask(animal: str) -> None:
    image = Image.open(ASSETS / f"{animal}-shadow.png").convert("RGBA")
    pixels = np.asarray(image)
    rgb = pixels[:, :, :3]

    # The approved renders use true black for the featureless subject. Selecting
    # only the largest connected near-black field excludes dark cave details.
    subject = largest_component(rgb.max(axis=2) < 13)
    if animal == "owl":
        # The moon scene has a one-pixel black horizon touching the left wing.
        # It belongs to the background and must not receive the contour glow.
        subject[:, :64] = False
    output = np.zeros_like(pixels)
    output[:, :, 3] = subject.astype(np.uint8) * 255
    Image.fromarray(output, "RGBA").save(ASSETS / f"{animal}-edge.png", optimize=True)


for species in ("mole", "owl", "moth"):
    make_edge_mask(species)
