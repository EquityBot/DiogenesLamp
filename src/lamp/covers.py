"""Bounded local cover preparation; no network or model dependency."""

from __future__ import annotations

import hashlib
import os
import tempfile
from pathlib import Path
from typing import cast

from PIL import Image, ImageOps

COVER_EDGE = 640


def resize_cover(path: Path) -> bool:
    """Orient and shrink a verified image, keeping its format and transparency."""
    try:
        opened = Image.open(path)
    except Image.DecompressionBombError as exc:
        raise ValueError("cover dimensions exceed limits") from exc
    with opened as source:
        if source.format not in {"JPEG", "PNG", "WEBP"}:
            raise ValueError("unsupported cover format")
        if max(source.size) > 8192 or source.width * source.height > 20_000_000:
            raise ValueError("cover dimensions exceed limits")
        if max(source.size) <= COVER_EDGE and source.getexif().get(274, 1) == 1:
            return False
        image = ImageOps.exif_transpose(source)
        image.thumbnail((COVER_EDGE, COVER_EDGE), Image.Resampling.LANCZOS)
        fmt = source.format
        if fmt == "JPEG" and image.mode not in {"RGB", "L"}:
            image = image.convert("RGB")
        with tempfile.NamedTemporaryFile(dir=path.parent, suffix=".tmp", delete=False) as file:
            temp = Path(file.name)
        try:
            image.save(temp, format=fmt, quality=85, optimize=True)
            os.replace(temp, path)
        finally:
            temp.unlink(missing_ok=True)
    return True


def cover_facts(path: Path) -> dict[str, str]:
    if not path.is_file() or path.stat().st_size > 8 * 1024 * 1024:
        return {}
    version = hashlib.sha256(path.read_bytes()).hexdigest()[:24]
    result = {"thumb_version": version, "cover_hash": ""}
    try:
        with Image.open(path) as source:
            if max(source.size) > 8192 or source.width * source.height > 20_000_000:
                return result
            pixels = cast(list[int], list(ImageOps.exif_transpose(source).convert("L")
                          .resize((9, 8), Image.Resampling.LANCZOS).get_flattened_data()))
        bits = 0
        for row in range(8):
            for col in range(8):
                bits = (bits << 1) | (pixels[row * 9 + col] > pixels[row * 9 + col + 1])
        result["cover_hash"] = f"{bits:016x}"
    except (OSError, ValueError, Image.DecompressionBombError):
        pass
    return result
