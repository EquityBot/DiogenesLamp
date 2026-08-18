"""Generate synthetic catalog data for viewer development.

Usage: python tests/make_demo_data.py [data_dir] [sellers] [albums_per_seller]
Then:  lamp --data <data_dir> serve --no-browser
"""

import json
import random
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

WORDS = ("alpha", "bravo", "canvas", "denim", "echo", "fleece", "grid", "hood",
         "indigo", "jacket", "knit", "lunar", "mesh", "nylon", "orbit", "polo")


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
            age_days = rng.randint(0, 1095)
            albums.append({
                "id": aid, "seller": seller, "title": title, "list_title": title,
                "thumb": f"thumbs/{seller}/{aid}.jpg",
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
    print(f"wrote {n_sellers} sellers x {n_albums} albums to {data_dir}/")


if __name__ == "__main__":
    main()
