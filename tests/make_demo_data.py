"""Generate synthetic catalog data for viewer development.

Usage: python tests/make_demo_data.py [data_dir] [sellers] [albums_per_seller]
Then:  lamp --data <data_dir> serve --no-browser

Two albums in every three get a cover, spread across each seller rather than
sitting on its first albums. The rest are left cover-less deliberately: Yupoo
has no cover image for some albums, and the viewer draws its titled colour
block for those, so a demo where every card carried an image would never
exercise that path. Spread rather than prefixed because the viewer orders by
posting date, which is unrelated to position, so a prefix of covers left every
screen of the demo showing nothing but the fallback.
"""

import json
import random
import struct
import sys
import zlib
from datetime import UTC, datetime, timedelta
from pathlib import Path

WORDS = ("alpha", "bravo", "canvas", "denim", "echo", "fleece", "grid", "hood",
         "indigo", "jacket", "knit", "lunar", "mesh", "nylon", "orbit", "polo")

UNCOVERED_EVERY = 3
COVER_PX = 240


def has_cover(index: int) -> bool:
    return index % UNCOVERED_EVERY != 0


def _png(width: int, height: int, rows: list[bytes]) -> bytes:
    """Minimal 8-bit RGB PNG encoder.

    Written by hand rather than with Pillow: this is a development helper,
    and the package deliberately carries no image dependency.
    """
    raw = b"".join(b"\x00" + row for row in rows)

    def chunk(tag: bytes, data: bytes) -> bytes:
        body = tag + data
        return (struct.pack(">I", len(data)) + body
                + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6))
            + chunk(b"IEND", b""))


def _hsv_to_rgb(hue: float, sat: float, val: float) -> tuple[int, int, int]:
    sector = int(hue * 6) % 6
    offset = hue * 6 - int(hue * 6)
    p, q, t = val * (1 - sat), val * (1 - sat * offset), val * (1 - sat * (1 - offset))
    r, g, b = ((val, t, p), (q, val, p), (p, val, t),
               (p, q, val), (t, p, val), (val, p, q))[sector]
    return int(r * 255), int(g * 255), int(b * 255)


def cover_bytes(seed: int) -> bytes:
    """A diagonal two-tone gradient, hue keyed on the album id.

    Stands in for a product photo well enough to lay out and eyeball the
    grid, while staying obviously synthetic to anyone reading a screenshot.
    """
    hue = (seed * 0.61803398875) % 1.0
    top = _hsv_to_rgb(hue, 0.45, 0.85)
    bottom = _hsv_to_rgb((hue + 0.08) % 1.0, 0.60, 0.45)
    rows = []
    for y in range(COVER_PX):
        row = bytearray()
        for x in range(COVER_PX):
            ratio = (y + x * 0.35) / (COVER_PX * 1.35)
            row += bytes(int(a + (b - a) * ratio) for a, b in zip(top, bottom))
        rows.append(bytes(row))
    return _png(COVER_PX, COVER_PX, rows)


def main() -> None:
    data_dir = Path(sys.argv[1] if len(sys.argv) > 1 else "demo-data")
    n_sellers = int(sys.argv[2]) if len(sys.argv) > 2 else 5
    n_albums = int(sys.argv[3]) if len(sys.argv) > 3 else 2000
    rng = random.Random(42)
    run_ts = "2026-08-17T12:00:00+10:00"
    # Spread demo postings across the last three years so the age filter has
    # something to cut: with every album the same age, every threshold would
    # return either the whole corpus or nothing.
    today = datetime.now(UTC).date()
    sellers = []
    (data_dir / "catalog").mkdir(parents=True, exist_ok=True)
    for i in range(n_sellers):
        seller = f"demoseller{i}"
        thumb_dir = data_dir / "thumbs" / seller
        thumb_dir.mkdir(parents=True, exist_ok=True)
        albums = []
        for j in range(n_albums):
            # Offset per seller: favorites key on album id, so ids must be
            # globally unique like real Yupoo ids or the manual favorites
            # check stars one card per seller at once.
            aid = str((i + 1) * 10_000_000 + j)
            title = " ".join(rng.sample(WORDS, 3)) + f" {aid}"
            links = []
            if j % 3 == 0:
                links.append({"platform": "weidian",
                              "url": f"https://weidian.com/item.html?itemID={j}"})
            if j % 4 == 0:
                links.append({"platform": "taobao",
                              "url": f"https://item.taobao.com/item.htm?id={j}"})
            if has_cover(j):
                (thumb_dir / f"{aid}.png").write_bytes(cover_bytes(int(aid)))
            age_days = rng.randint(0, 1095)
            albums.append({
                "id": aid, "seller": seller, "title": title, "list_title": title,
                "thumb": (f"thumbs/{seller}/{aid}.png"
                          if has_cover(j) else ""),
                "album_url": f"https://{seller}.x.yupoo.com/albums/{aid}?uid=1",
                "links": links,
                "category": rng.choice(("hoodies", "tees", "shoes", "")),
                "photo_count": rng.randint(3, 40),
                "date_published": (today - timedelta(days=age_days)).isoformat(),
                "date_modified": (today - timedelta(days=age_days // 2)).isoformat(),
                "first_seen": run_ts if j % 10 == 0 else "2026-08-01T00:00:00+10:00",
                "updated_at": run_ts,
            })
        (data_dir / "catalog" / f"{seller}.json").write_text(
            json.dumps({"seller": seller, "albums": albums}), encoding="utf-8"
        )
        sellers.append({"id": seller, "name": seller,
                        "url": f"https://{seller}.x.yupoo.com",
                        "album_count": n_albums, "shard": f"catalog/{seller}.json"})
    shops = [{"id": "weidian-1", "platform": "weidian", "name": "demo shop",
              "url": "https://weidian.com/?userid=1"}]
    (data_dir / "index.json").write_text(
        json.dumps({"generated_at": run_ts, "sellers": sellers, "shops": shops}),
        encoding="utf-8",
    )
    covers = n_sellers * sum(1 for j in range(n_albums) if has_cover(j))
    print(f"wrote {n_sellers} sellers x {n_albums} albums "
          f"({covers} covers) to {data_dir}/")


if __name__ == "__main__":
    main()
