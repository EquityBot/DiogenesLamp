import json
from pathlib import Path

from lamp.http import BlockedError, FetchError
from lamp.ingest import Source
from lamp.scrape import SellerSummary, scrape_all, scrape_seller
from lamp.storage import load_shard

LIST_ITEM = """
<a class="album__main" href="/albums/{aid}?uid=1" title="{title}">
  <img class="album__img" data-src="//photo.yupoo.com/s/{aid}/small.jpg">
  <span class="album__count">3</span>
  <div class="album__title">{title}</div>
</a>
"""

# A card whose only image is Yupoo's own "no image" asset, which parse.py
# deliberately reports as no cover rather than as a downloadable thumb.
NO_COVER_LIST_ITEM = """
<a class="album__main" href="/albums/{aid}?uid=1" title="{title}">
  <img class="album__img" data-src="https://cdn.example.com/yupoo/album/im_photo_album.png">
  <span class="album__count">3</span>
  <div class="album__title">{title}</div>
</a>
"""

ALBUM_PAGE = """
<span class="showalbumheader__gallerytitle" data-name="{title}">{title}</span>
<div class="showalbumheader__gallerysubtitle">{desc}</div>
<span class="showalbumheader__gallerycount">3</span>
"""

EMPTY_PAGE = "<html><body></body></html>"


class FakeResponse:
    def __init__(self, text: str):
        self.text = text
        self.status_code = 200
        self.content = text.encode()

    def iter_content(self, chunk_size):
        yield self.content


class FakeClient:
    """Maps URL substrings to responses; downloads succeed unless a block
    pattern says otherwise (indexed by download call order)."""

    def __init__(self, pages: dict[str, str], blocked_substrings=(),
                 download_block_pattern=None, fetch_error_substrings=()):
        self.pages = pages
        self.blocked = tuple(blocked_substrings)
        self.fetch_errors = tuple(fetch_error_substrings)
        self.calls: list[str] = []
        self.download_calls: list[str] = []
        self._download_block_pattern = list(download_block_pattern or [])

    def get(self, url, referer=None):
        self.calls.append(url)
        for marker in self.blocked:
            if marker in url:
                raise BlockedError(429)
        for marker in self.fetch_errors:
            if marker in url:
                # Mirrors a real non-blocked hard failure (for example, the live-verified
                # bare-URL 404) rather than rate-limit/hotlink blocking.
                raise FetchError(f"fake fetch error for {url}")
        for marker, text in self.pages.items():
            if marker in url:
                return FakeResponse(text)
        return FakeResponse(EMPTY_PAGE)

    def download(self, url, dest, referer=None):
        idx = len(self.download_calls)
        self.download_calls.append(url)
        if idx < len(self._download_block_pattern) and self._download_block_pattern[idx]:
            raise BlockedError(403)
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(b"jpg")
        return True


SELLER = Source("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com", "sellerx")


def make_client():
    list_html = LIST_ITEM.format(aid="101", title="Item A") + LIST_ITEM.format(
        aid="102", title="Item B https://item.taobao.com/item.htm?id=7"
    )
    return FakeClient({
        "albums?page=1": list_html,
        "albums?page=2": EMPTY_PAGE,
        "/albums/101": ALBUM_PAGE.format(title="Item A", desc="weidian.com/item.html?itemID=5"),
        "/albums/102": ALBUM_PAGE.format(title="Item B", desc="plain text"),
    })


def test_scrape_seller_full_pass(tmp_path: Path):
    summary = scrape_seller(make_client(), SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert summary == SellerSummary("sellerx", scraped=2, skipped=0, failed=0, error="")
    shard = load_shard(tmp_path, "sellerx")
    assert set(shard) == {"101", "102"}
    album = shard["101"]
    assert album["title"] == "Item A"
    assert album["list_title"] == "Item A"
    assert shard["102"]["title"] == "Item B"
    assert shard["102"]["list_title"] == "Item B https://item.taobao.com/item.htm?id=7"
    assert album["links"] == [
        {"platform": "weidian", "url": "https://weidian.com/item.html?itemID=5"}
    ]
    # The link only appears in the list-page title (102's album-page data-name
    # is clean "Item B") - extract_links must still find it via stub.title.
    assert shard["102"]["links"] == [
        {"platform": "taobao", "url": "https://item.taobao.com/item.htm?id=7"}
    ]
    assert album["thumb"] == "thumbs/sellerx/101.jpg"
    assert album["album_url"] == "https://sellerx.x.yupoo.com/albums/101?uid=1"
    assert album["first_seen"] == "2026-08-17T12:00:00+10:00"
    assert (tmp_path / "thumbs" / "sellerx" / "101.jpg").exists()


def test_scrape_seller_incremental_skips_unchanged(tmp_path: Path):
    scrape_seller(make_client(), SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    client = make_client()
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert summary.skipped == 2 and summary.scraped == 0
    assert not any("/albums/101" in url for url in client.calls)
    shard = load_shard(tmp_path, "sellerx")
    assert shard["101"]["first_seen"] == "2026-08-17T12:00:00+10:00"


def test_scrape_seller_retains_delisted_albums(tmp_path: Path):
    scrape_seller(make_client(), SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    only_102 = FakeClient({
        "albums?page=1": LIST_ITEM.format(
            aid="102", title="Item B https://item.taobao.com/item.htm?id=7"
        ),
        "albums?page=2": EMPTY_PAGE,
    })
    scrape_seller(only_102, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert set(load_shard(tmp_path, "sellerx")) == {"101", "102"}


def test_layout_change_guard_preserves_shard(tmp_path: Path):
    scrape_seller(make_client(), SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    empty = FakeClient({"albums?page=1": EMPTY_PAGE})
    summary = scrape_seller(empty, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert summary.error
    assert (tmp_path / "debug" / "sellerx-page1.html").exists()
    assert set(load_shard(tmp_path, "sellerx")) == {"101", "102"}


def test_pagination_stops_on_repeated_page(tmp_path: Path):
    # Defensive hardening: a real Yupoo past-the-end page comes back empty
    # (verified live), but a hypothetical seller could instead repeat the
    # last page's content forever. Page 2 here repeats page 1 verbatim; page
    # 3 would be empty but must never be requested.
    list_html = LIST_ITEM.format(aid="101", title="Item A") + LIST_ITEM.format(
        aid="102", title="Item B https://item.taobao.com/item.htm?id=7"
    )
    client = FakeClient({
        "albums?page=1": list_html,
        "albums?page=2": list_html,
        "albums?page=3": EMPTY_PAGE,
        "/albums/101": ALBUM_PAGE.format(title="Item A", desc="weidian.com/item.html?itemID=5"),
        "/albums/102": ALBUM_PAGE.format(title="Item B", desc="plain text"),
    })
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert summary.error == ""
    assert summary.scraped == 2
    assert not any("page=3" in url for url in client.calls)
    assert set(load_shard(tmp_path, "sellerx")) == {"101", "102"}


def test_blocked_streak_aborts_seller(tmp_path: Path):
    client = FakeClient({}, blocked_substrings=["sellerx"])
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert "blocked" in summary.error


def test_blocked_streak_on_album_pages_aborts_after_three(tmp_path: Path):
    from lamp.storage import write_shard

    # Pre-existing record for an album no longer listed - must survive an
    # aborted run untouched (same archive guarantee the delisted-album test
    # covers for a clean run).
    write_shard(tmp_path, "sellerx", {"999": {
        "id": "999", "seller": "sellerx", "title": "kept", "list_title": "kept",
        "thumb": "thumbs/sellerx/999.jpg",
        "album_url": "https://sellerx.x.yupoo.com/albums/999",
        "links": [], "category": "", "photo_count": 1,
        "first_seen": "2026-08-01T00:00:00+10:00",
        "updated_at": "2026-08-01T00:00:00+10:00",
    }})
    list_html = "".join(
        LIST_ITEM.format(aid=str(aid), title=f"Item {aid}") for aid in (101, 102, 103, 104)
    )
    client = FakeClient(
        {"albums?page=1": list_html, "albums?page=2": EMPTY_PAGE},
        blocked_substrings=["/albums/"],  # blocks album-page fetches only, not the list pages
    )
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert "blocked" in summary.error
    assert summary.failed == 3
    shard = load_shard(tmp_path, "sellerx")
    assert set(shard) == {"999"}
    assert shard["999"]["title"] == "kept"


def test_all_album_fetches_failed_reports_error(tmp_path: Path):
    # Live-verified defect: a seller whose list pages parse fine but whose
    # every album-page fetch fails a non-blocked way (real case: bare album
    # URLs 404) must not report a clean run - the CLI's exit code depends
    # entirely on summary.error, and 0 scraped + all failed must be loud.
    list_html = "".join(
        LIST_ITEM.format(aid=str(aid), title=f"Item {aid}") for aid in (101, 102, 103)
    )
    client = FakeClient(
        {"albums?page=1": list_html, "albums?page=2": EMPTY_PAGE},
        fetch_error_substrings=["/albums/"],
    )
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert summary.scraped == 0
    assert summary.failed == 3
    assert summary.error == "all 3 album fetches failed"
    assert "all" in summary.error and "failed" in summary.error


def test_partial_album_fetch_failures_no_error(tmp_path: Path):
    # Partial failure (some scraped, some failed) must stay non-error - only
    # a total wipeout should trip summary.error.
    list_html = "".join(
        LIST_ITEM.format(aid=str(aid), title=f"Item {aid}") for aid in (101, 102, 103)
    )
    client = FakeClient(
        {
            "albums?page=1": list_html,
            "albums?page=2": EMPTY_PAGE,
            "/albums/101": ALBUM_PAGE.format(title="Item 101", desc="plain text"),
            "/albums/102": ALBUM_PAGE.format(title="Item 102", desc="plain text"),
        },
        fetch_error_substrings=["/albums/103"],
    )
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert summary.scraped == 2
    assert summary.failed == 1
    assert summary.error == ""


def _seller_of_albums(*aids: int):
    list_html = "".join(LIST_ITEM.format(aid=str(aid), title=f"Item {aid}") for aid in aids)
    pages = {"albums?page=1": list_html, "albums?page=2": EMPTY_PAGE}
    for aid in aids:
        pages[f"/albums/{aid}"] = ALBUM_PAGE.format(title=f"Item {aid}", desc="plain text")
    return pages


def test_thumb_block_streak_disables_thumbs_for_rest_of_seller(tmp_path: Path):
    # Hotlink protection on the thumb CDN is a separate failure mode from
    # album-page blocking: album pages must keep fetching normally while
    # thumb downloads fail every single time.
    pages = _seller_of_albums(101, 102, 103, 104, 105)
    client = FakeClient(pages, download_block_pattern=[True] * 5)
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert summary.error == ""
    assert summary.scraped == 5
    assert summary.failed == 0
    assert summary.thumbs_blocked is True
    # Only 3 download attempts: thumb downloads stop for the rest of the
    # seller's run after the 3rd consecutive BlockedError.
    assert len(client.download_calls) == 3
    shard = load_shard(tmp_path, "sellerx")
    assert set(shard) == {"101", "102", "103", "104", "105"}
    # thumb field still set to the expected path even though the download
    # never happened - the viewer falls back to a placeholder.
    assert shard["105"]["thumb"] == "thumbs/sellerx/105.jpg"


def test_an_album_with_no_cover_records_no_thumb_path(tmp_path: Path):
    """Yupoo has no cover image for some albums at all.

    parse_album_list reports its placeholder asset as no thumb, and nothing
    is downloaded for those. Recording a path anyway pointed the viewer's
    <img> at a file that can never exist - a 404 per card, re-issued on
    every scroll-in because serve.py sends no-store on 404s - and the titled
    colour block the viewer draws instead never appeared.
    """
    client = FakeClient({
        "albums?page=1": NO_COVER_LIST_ITEM.format(aid="501", title="No cover"),
        "albums?page=2": EMPTY_PAGE,
        "/albums/501": ALBUM_PAGE.format(title="No cover", desc="plain text"),
    })
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert summary.scraped == 1
    assert client.download_calls == []
    assert load_shard(tmp_path, "sellerx")["501"]["thumb"] == ""
    assert not (tmp_path / "thumbs").exists()


def test_thumb_block_streak_resets_on_success(tmp_path: Path):
    # Intermittent failures (never 3 in a row) must never trip the guard,
    # and a successful download must reset the consecutive counter to 0.
    pages = _seller_of_albums(101, 102, 103, 104, 105)
    client = FakeClient(pages, download_block_pattern=[True, True, False, True, True])
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert summary.error == ""
    assert summary.scraped == 5
    assert summary.thumbs_blocked is False
    # thumbs never disabled, so every album attempted a download.
    assert len(client.download_calls) == 5


def test_scrape_all_writes_index(tmp_path: Path):
    sources = {
        "yupoo:sellerx": SELLER,
        "shop:weidian-7": Source("shop", "weidian-7", "weidian",
                                 "https://weidian.com/?userid=7", "weidian 7"),
    }
    summaries = scrape_all(sources, tmp_path, "2026-08-17T12:00:00+10:00",
                           client_factory=make_client)
    assert [s.seller for s in summaries] == ["sellerx"]
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert index["generated_at"] == "2026-08-17T12:00:00+10:00"
    assert index["sellers"][0]["id"] == "sellerx"
    assert index["sellers"][0]["album_count"] == 2
    assert index["sellers"][0]["shard"] == "catalog/sellerx.json"
    assert index["shops"][0]["id"] == "weidian-7"


def test_scoped_scrape_keeps_other_sellers_in_index(tmp_path: Path):
    from lamp.storage import write_shard

    other = Source("yupoo", "othersella", "yupoo",
                   "https://othersella.x.yupoo.com", "othersella")
    write_shard(tmp_path, "othersella", {"7": {
        "id": "7", "seller": "othersella", "title": "kept", "list_title": "kept",
        "thumb": "thumbs/othersella/7.jpg",
        "album_url": "https://othersella.x.yupoo.com/albums/7",
        "links": [], "category": "", "photo_count": 1,
        "first_seen": "2026-08-01T00:00:00+10:00",
        "updated_at": "2026-08-01T00:00:00+10:00",
    }})
    sources = {"yupoo:sellerx": SELLER, "yupoo:othersella": other}
    summaries = scrape_all(sources, tmp_path, "2026-08-17T12:00:00+10:00",
                           client_factory=make_client, only={"sellerx"})
    assert [s.seller for s in summaries] == ["sellerx"]  # only sellerx scraped
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert [s["id"] for s in index["sellers"]] == ["othersella", "sellerx"]
    assert index["sellers"][0]["album_count"] == 1  # untouched seller kept in manifest


def _aged_client():
    """One album with a fresh id, one with an ancient id (2017-era)."""
    list_html = (
        LIST_ITEM.format(aid="250700000", title="Fresh")
        + LIST_ITEM.format(aid="9000000", title="Ancient")
    )
    return FakeClient({
        "albums?page=1": list_html,
        "albums?page=2": EMPTY_PAGE,
        "/albums/250700000": ALBUM_PAGE.format(title="Fresh", desc=""),
        "/albums/9000000": ALBUM_PAGE.format(title="Ancient", desc=""),
    })


def test_max_age_days_skips_old_albums_without_fetching(tmp_path: Path):
    client = _aged_client()
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00",
                            max_age_days=180)
    assert (summary.scraped, summary.skipped_old) == (1, 1)
    # the whole point: no album page and no thumb for the stale listing
    assert not any("/albums/9000000" in url for url in client.calls)
    assert not any("9000000" in url for url in client.download_calls)
    assert set(load_shard(tmp_path, "sellerx")) == {"250700000"}


def test_max_age_days_zero_scrapes_everything(tmp_path: Path):
    summary = scrape_seller(_aged_client(), SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert (summary.scraped, summary.skipped_old) == (2, 0)


def test_max_age_days_leaves_existing_old_records_in_place(tmp_path: Path):
    """Skipping a stale album must not drop what an earlier scrape stored."""
    scrape_seller(_aged_client(), SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert set(load_shard(tmp_path, "sellerx")) == {"250700000", "9000000"}
    scrape_seller(_aged_client(), SELLER, tmp_path, "2026-08-18T12:00:00+10:00",
                  max_age_days=180, full=True)
    assert set(load_shard(tmp_path, "sellerx")) == {"250700000", "9000000"}


def test_max_age_days_trusts_a_stored_date_over_the_id_estimate(tmp_path: Path):
    """An ancient-looking id with a recent scraped date is not stale."""
    scrape_seller(_aged_client(), SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    shard_path = tmp_path / "catalog" / "sellerx.json"
    payload = json.loads(shard_path.read_text(encoding="utf-8"))
    for album in payload["albums"]:
        if album["id"] == "9000000":
            album["date_published"] = "2026-08-01"
    shard_path.write_text(json.dumps(payload), encoding="utf-8")

    client = _aged_client()
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00",
                            max_age_days=180, full=True)
    assert summary.skipped_old == 0
    assert any("/albums/9000000" in url for url in client.calls)


def test_unreadable_page_with_album_links_reports_a_layout_change(tmp_path: Path):
    """A page holding albums the parser cannot read is a parser problem."""
    cards = "".join(
        f'<a class="mystery__main" href="/albums/{aid}">x</a>' for aid in (901, 902, 903)
    )
    summary = scrape_seller(FakeClient({"albums?page=1": f"<html><body>{cards}</body></html>"}),
                            SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert "layout change" in summary.error
    assert (tmp_path / "debug" / "sellerx-page1.html").exists()


def test_a_lone_album_link_does_not_imply_a_layout_change(tmp_path: Path):
    """A promo or nav link on an otherwise empty page is not a parser bug."""
    html = '<html><body><a href="/albums/12345">featured</a></body></html>'
    summary = scrape_seller(FakeClient({"albums?page=1": html}), SELLER, tmp_path,
                            "2026-08-18T12:00:00+10:00")
    assert "empty or closed seller" in summary.error
    assert "layout change" not in summary.error


def test_page_without_album_links_reports_an_empty_seller(tmp_path: Path):
    """No album links at all means a closed or empty seller, not a parser bug.

    Conflating the 2 once hid every seller on Yupoo's second template
    behind an error that read like dead accounts.
    """
    summary = scrape_seller(FakeClient({"albums?page=1": EMPTY_PAGE}), SELLER, tmp_path,
                            "2026-08-18T12:00:00+10:00")
    assert "empty or closed seller" in summary.error
    assert "layout change" not in summary.error


def test_scrape_all_honours_skip_but_not_over_an_explicit_seller(tmp_path: Path):
    sources = {
        "yupoo:sellerx": SELLER,
        "yupoo:skipme": Source("yupoo", "skipme", "yupoo",
                               "https://skipme.x.yupoo.com", "skipme", skip=True),
    }
    summaries = scrape_all(sources, tmp_path, "2026-08-18T12:00:00+10:00",
                           client_factory=make_client)
    assert [s.seller for s in summaries] == ["sellerx"]

    # naming it explicitly still scrapes it
    summaries = scrape_all(sources, tmp_path, "2026-08-18T12:00:00+10:00",
                           client_factory=make_client, only={"skipme"})
    assert [s.seller for s in summaries] == ["skipme"]


def test_scrape_all_builds_a_rate_limited_client_by_default(tmp_path: Path, monkeypatch):
    """Every test passes a fake client, so the real wiring never runs.

    A broken limiter here would only show up as an unthrottled live scrape.
    """
    built = {}

    class SpyLimiter:
        def __init__(self, **kwargs):
            built.update(kwargs)

        def wait(self, host):
            pass

    monkeypatch.setattr("lamp.scrape.RateLimiter", SpyLimiter)
    monkeypatch.setattr("lamp.scrape.Client", lambda limiter: make_client())
    summaries = scrape_all({"yupoo:sellerx": SELLER}, tmp_path,
                           "2026-08-17T12:00:00+10:00", rps=12.0)
    assert summaries[0].scraped == 2
    assert built["global_rps"] == 12.0
    # the per-seller floor is a politeness limit and is not operator-tunable
    assert built["host_rps"] == 1.0
    # the shared thumb CDN would otherwise serialise the whole run at 1 req/s
    assert built["host_overrides"] == {"photo.yupoo.com": 12.0}


def test_scrape_all_scrapes_several_sellers_and_indexes_them_all(tmp_path: Path):
    sellers = {
        "yupoo:sellerx": SELLER,
        "yupoo:sellery": Source("yupoo", "sellery", "yupoo",
                                "https://sellery.x.yupoo.com", "sellery"),
    }
    summaries = scrape_all(sellers, tmp_path, "2026-08-17T12:00:00+10:00",
                           client_factory=make_client, workers=2)
    assert sorted(s.seller for s in summaries) == ["sellerx", "sellery"]
    assert all(s.scraped == 2 for s in summaries)
    assert set(load_shard(tmp_path, "sellerx")) == {"101", "102"}
    assert set(load_shard(tmp_path, "sellery")) == {"101", "102"}
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert [s["id"] for s in index["sellers"]] == ["sellerx", "sellery"]
    assert all(s["album_count"] == 2 for s in index["sellers"])


def test_index_omits_skipped_sellers_that_have_no_data(tmp_path: Path):
    """A skipped seller has no shard, so listing it would make the viewer
    report a deliberate exclusion as an unavailable seller."""
    sources = {
        "yupoo:sellerx": SELLER,
        "yupoo:skipme": Source("yupoo", "skipme", "yupoo",
                               "https://skipme.x.yupoo.com", "skipme", skip=True),
    }
    scrape_all(sources, tmp_path, "2026-08-18T12:00:00+10:00", client_factory=make_client)
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert [s["id"] for s in index["sellers"]] == ["sellerx"]


def test_corrupt_shard_errors_the_seller_not_the_run(tmp_path: Path):
    """One unreadable shard must not crash every other seller's scrape or
    the index build."""
    catalog = tmp_path / "catalog"
    catalog.mkdir()
    (catalog / "sellerx.json").write_text("{broken", encoding="utf-8")
    others = Source("yupoo", "sellery", "yupoo", "https://sellery.x.yupoo.com", "sellery")
    sources = {"yupoo:sellerx": SELLER, "yupoo:sellery": others}
    summaries = scrape_all(sources, tmp_path, "2026-08-18T12:00:00+10:00",
                           client_factory=make_client)
    by_seller = {s.seller: s for s in summaries}
    assert "corrupt shard" in by_seller["sellerx"].error
    assert by_seller["sellery"].scraped == 2  # untouched by its neighbour's mess
    # Still listed, with nothing counted: the viewer's fetch of it fails and
    # its load note names the seller, instead of the seller vanishing.
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert [(s["id"], s["album_count"]) for s in index["sellers"]] == [
        ("sellerx", 0), ("sellery", 2),
    ]


def test_index_keeps_a_skipped_seller_that_was_scraped_before(tmp_path: Path):
    """Skipping later must not hide albums already on disk."""
    scrape_all({"yupoo:sellerx": SELLER}, tmp_path, "2026-08-17T12:00:00+10:00",
               client_factory=make_client)
    skipped_now = Source("yupoo", "sellerx", "yupoo",
                         "https://sellerx.x.yupoo.com", "sellerx", skip=True)
    scrape_all({"yupoo:sellerx": skipped_now}, tmp_path, "2026-08-18T12:00:00+10:00",
               client_factory=make_client)
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert [s["id"] for s in index["sellers"]] == ["sellerx"]
    assert index["sellers"][0]["album_count"] == 2


def test_index_loads_each_shard_once(tmp_path: Path, monkeypatch):
    """The index build must not double-read shards: at tens of MB per shard
    the second parse per seller is the slowest part of a scrape's tail."""
    loads = []
    from lamp import scrape as scrape_module

    real_load_shard = scrape_module.load_shard

    def spy(data_dir, seller):
        loads.append(seller)
        return real_load_shard(data_dir, seller)

    monkeypatch.setattr(scrape_module, "load_shard", spy)
    scrape_all({"yupoo:sellerx": SELLER}, tmp_path, "2026-08-17T12:00:00+10:00",
               client_factory=make_client)
    # one load during scrape_seller + one during the index build, not 2
    assert loads.count("sellerx") == 2


def test_scrape_flushes_the_shard_periodically(tmp_path: Path, monkeypatch):
    """An interrupt mid-run must not cost every album scraped so far: the
    shard is rewritten after every 100 scraped albums, atomically."""
    from lamp import scrape as scrape_module

    pages = {"albums?page=1": "".join(
        LIST_ITEM.format(aid=str(aid), title=f"Item {aid}") for aid in range(101, 251)
    ), "albums?page=2": EMPTY_PAGE}
    for aid in range(101, 251):
        pages[f"/albums/{aid}"] = ALBUM_PAGE.format(title=f"Item {aid}", desc="")

    writes = 0
    real_write_shard = scrape_module.write_shard

    def spy(data_dir, seller, shard):
        nonlocal writes
        writes += 1
        real_write_shard(data_dir, seller, shard)

    monkeypatch.setattr(scrape_module, "write_shard", spy)
    summary = scrape_seller(FakeClient(pages), SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    assert summary.scraped == 150
    # 150 scraped = one mid-run flush at 100 plus the end-of-run write.
    assert writes == 2
    assert len(load_shard(tmp_path, "sellerx")) == 150


def test_scrape_dry_run_previews_without_album_fetches_or_disk_writes(tmp_path: Path):
    """--dry-run fetches list pages to count stubs, but touches no album pages,
    downloads no thumbs, and writes no shards or index files."""
    client = make_client()
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-17T12:00:00+10:00", dry_run=True)
    assert summary.scraped == 2
    assert summary.skipped == 0
    assert summary.failed == 0
    # List pages fetched, but NO album pages and NO downloads
    assert any("albums?page=1" in url for url in client.calls)
    assert not any("/albums/101" in url for url in client.calls)
    assert not any("/albums/102" in url for url in client.calls)
    assert client.download_calls == []
    # Nothing written to disk
    assert not (tmp_path / "catalog" / "sellerx.json").exists()
    assert not (tmp_path / "thumbs").exists()

    # scrape_all in dry_run mode also leaves index.json unwritten
    sources = {"yupoo:sellerx": SELLER}
    summaries = scrape_all(sources, tmp_path, "2026-08-17T12:00:00+10:00",
                           client_factory=make_client, dry_run=True)
    assert len(summaries) == 1
    assert summaries[0].scraped == 2
    assert not (tmp_path / "index.json").exists()


def test_scrape_dry_run_does_not_write_debug_html_for_an_empty_parse(tmp_path: Path):
    """An empty list page still reports the empty-seller error, but --dry-run
    must not write or overwrite data/debug/<seller>-page1.html."""
    summary = scrape_seller(
        FakeClient({"albums?page=1": EMPTY_PAGE}),
        SELLER,
        tmp_path,
        "2026-08-18T12:00:00+10:00",
        dry_run=True,
    )
    assert "empty or closed seller" in summary.error
    assert not (tmp_path / "debug").exists()


def test_incremental_scrape_backfills_price_on_pre_price_records(tmp_path: Path):
    """Catalogue records written before price extraction have no price key.
    An ordinary incremental scrape must still fetch those album pages once
    so a later price in the title or description is not left as 0 forever.
    """
    from lamp.storage import write_shard

    write_shard(tmp_path, "sellerx", {"101": {
        "id": "101", "seller": "sellerx", "title": "Item A", "list_title": "Item A",
        "thumb": "thumbs/sellerx/101.jpg",
        "album_url": "https://sellerx.x.yupoo.com/albums/101?uid=1",
        "links": [], "category": "", "photo_count": 3,
        "first_seen": "2026-08-01T00:00:00+10:00",
        "updated_at": "2026-08-01T00:00:00+10:00",
    }})
    client = FakeClient({
        "albums?page=1": LIST_ITEM.format(aid="101", title="Item A"),
        "albums?page=2": EMPTY_PAGE,
        "/albums/101": ALBUM_PAGE.format(
            title="Item A", desc="Nike Tech Fleece ¥180 Black",
        ),
    })
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert summary.skipped == 0
    assert summary.scraped == 1
    assert any("/albums/101" in url for url in client.calls)
    shard = load_shard(tmp_path, "sellerx")
    assert shard["101"]["price"] == 180
    assert shard["101"]["first_seen"] == "2026-08-01T00:00:00+10:00"


def test_incremental_scrape_still_skips_unchanged_priced_records(tmp_path: Path):
    """A record that already has a price key, even 0, is not re-fetched
    just because a later run could theoretically find a price.
    """
    from lamp.storage import write_shard

    write_shard(tmp_path, "sellerx", {"101": {
        "id": "101", "seller": "sellerx", "title": "Item A", "list_title": "Item A",
        "thumb": "thumbs/sellerx/101.jpg",
        "album_url": "https://sellerx.x.yupoo.com/albums/101?uid=1",
        "links": [], "category": "", "photo_count": 3, "price": 0,
        "first_seen": "2026-08-01T00:00:00+10:00",
        "updated_at": "2026-08-01T00:00:00+10:00",
    }})
    client = FakeClient({
        "albums?page=1": LIST_ITEM.format(aid="101", title="Item A"),
        "albums?page=2": EMPTY_PAGE,
        "/albums/101": ALBUM_PAGE.format(title="Item A", desc="¥180"),
    })
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert summary.skipped == 1
    assert summary.scraped == 0
    assert not any("/albums/101" in url for url in client.calls)
    assert load_shard(tmp_path, "sellerx")["101"]["price"] == 0


def _pre_price_record(**extra):
    return {
        "id": "101", "seller": "sellerx", "title": "Item A", "list_title": "Item A",
        "thumb": "thumbs/sellerx/101.jpg",
        "album_url": "https://sellerx.x.yupoo.com/albums/101?uid=1",
        "links": [], "category": "", "photo_count": 3,
        "first_seen": "2026-08-01T00:00:00+10:00",
        "updated_at": "2026-08-01T00:00:00+10:00", **extra,
    }


def _one_album_client():
    return FakeClient({
        "albums?page=1": LIST_ITEM.format(aid="101", title="Item A"),
        "albums?page=2": EMPTY_PAGE,
        "/albums/101": ALBUM_PAGE.format(title="Item A", desc="¥180"),
    })


def _write_thumb(data_dir: Path) -> None:
    thumb = data_dir / "thumbs" / "sellerx" / "101.jpg"
    thumb.parent.mkdir(parents=True)
    thumb.write_bytes(b"old cover")


def test_refetched_album_keeps_a_cover_already_on_disk(tmp_path: Path):
    """A price backfill re-fetches every album page; it must not download
    every cover again as well. A record from before thumb_url was stored
    counts as the same cover."""
    from lamp.storage import write_shard

    write_shard(tmp_path, "sellerx", {"101": _pre_price_record()})
    _write_thumb(tmp_path)
    client = _one_album_client()
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert summary.scraped == 1
    assert client.download_calls == []
    assert load_shard(tmp_path, "sellerx")["101"]["thumb_url"] == (
        "https://photo.yupoo.com/s/101/small.jpg")


def test_changed_cover_url_or_full_run_downloads_the_cover_again(tmp_path: Path):
    from lamp.storage import write_shard

    _write_thumb(tmp_path)
    write_shard(tmp_path, "sellerx", {"101": _pre_price_record(
        thumb_url="https://photo.yupoo.com/s/101/older.jpg")})
    client = _one_album_client()
    scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert len(client.download_calls) == 1

    client = _one_album_client()
    scrape_seller(client, SELLER, tmp_path, "2026-08-19T12:00:00+10:00", full=True)
    assert len(client.download_calls) == 1


def test_unchanged_album_with_a_missing_cover_fetches_only_the_cover(tmp_path: Path):
    from lamp.storage import write_shard

    write_shard(tmp_path, "sellerx", {"101": _pre_price_record(price=0, thumb="")})
    client = _one_album_client()
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert (summary.skipped, summary.scraped, summary.thumbs_refetched) == (1, 0, 1)
    assert not any("/albums/101" in url for url in client.calls)
    assert len(client.download_calls) == 1
    assert load_shard(tmp_path, "sellerx")["101"]["thumb"] == "thumbs/sellerx/101.jpg"

    client = _one_album_client()  # the cover is on disk now: nothing to do
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-19T12:00:00+10:00")
    assert summary.thumbs_refetched == 0 and client.download_calls == []


def test_unchanged_album_with_a_changed_cover_fetches_the_new_cover(tmp_path: Path):
    from lamp.storage import write_shard

    _write_thumb(tmp_path)
    write_shard(tmp_path, "sellerx", {"101": _pre_price_record(
        price=0, thumb_url="https://photo.yupoo.com/s/101/older.jpg")})
    client = _one_album_client()
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert (summary.skipped, summary.thumbs_refetched) == (1, 1)
    assert not any("/albums/101" in url for url in client.calls)
    assert load_shard(tmp_path, "sellerx")["101"]["thumb_url"] == (
        "https://photo.yupoo.com/s/101/small.jpg")


def test_a_failed_cover_download_keeps_the_old_url_so_it_is_retried(tmp_path: Path):
    """The old file survives a failed download, so recording the new URL
    would claim a cover that never arrived and stop every later retry."""
    from lamp.storage import write_shard

    _write_thumb(tmp_path)
    older = "https://photo.yupoo.com/s/101/older.jpg"
    write_shard(tmp_path, "sellerx", {"101": _pre_price_record(thumb_url=older)})
    client = _one_album_client()
    client._download_block_pattern = [True]
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    assert summary.scraped == 1
    assert load_shard(tmp_path, "sellerx")["101"]["thumb_url"] == older

    client = _one_album_client()  # the next run tries the new cover again
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-19T12:00:00+10:00")
    assert summary.thumbs_refetched == 1
    assert load_shard(tmp_path, "sellerx")["101"]["thumb_url"] != older


def test_listing_stamps_last_seen_and_marks_delisted_albums_gone(tmp_path: Path):
    scrape_seller(make_client(), SELLER, tmp_path, "2026-08-17T12:00:00+10:00")
    only_102 = FakeClient({
        "albums?page=1": LIST_ITEM.format(
            aid="102", title="Item B https://item.taobao.com/item.htm?id=7"),
        "albums?page=2": EMPTY_PAGE,
    })
    scrape_seller(only_102, SELLER, tmp_path, "2026-08-18T12:00:00+10:00")
    shard = load_shard(tmp_path, "sellerx")
    assert shard["101"]["gone_since"] == "2026-08-18T12:00:00+10:00"
    assert shard["101"]["last_seen"] == "2026-08-17T12:00:00+10:00"
    assert shard["102"]["last_seen"] == "2026-08-18T12:00:00+10:00"
    assert "gone_since" not in shard["102"]

    scrape_all({"yupoo:sellerx": SELLER}, tmp_path, "2026-08-19T12:00:00+10:00",
               client_factory=lambda: only_102)
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert index["sellers"][0]["album_count"] == 1  # the gone album is not counted

    scrape_seller(make_client(), SELLER, tmp_path, "2026-08-20T12:00:00+10:00")
    assert "gone_since" not in load_shard(tmp_path, "sellerx")["101"]  # relisted


def test_a_stopped_seller_saves_nothing_it_has_not_listed(tmp_path: Path):
    import threading

    stop = threading.Event()
    stop.set()
    client = make_client()
    summary = scrape_seller(client, SELLER, tmp_path, "2026-08-18T12:00:00+10:00", stop=stop)
    assert summary.error == "interrupted before the listing finished"
    assert client.calls == []
    assert not (tmp_path / "catalog" / "sellerx.json").exists()


def test_scrape_all_reports_each_seller_as_it_finishes(tmp_path: Path):
    sellers = {
        "yupoo:sellerx": SELLER,
        "yupoo:sellery": Source("yupoo", "sellery", "yupoo",
                                "https://sellery.x.yupoo.com", "sellery"),
    }
    seen: list[str] = []
    summaries = scrape_all(sellers, tmp_path, "2026-08-17T12:00:00+10:00",
                           client_factory=make_client, workers=2,
                           on_done=lambda s: seen.append(s.seller))
    assert sorted(seen) == ["sellerx", "sellery"]
    assert [s.seller for s in summaries] == ["sellerx", "sellery"]


def test_ctrl_c_writes_the_index_then_raises(tmp_path: Path):
    """The first seller's report stands in for Ctrl+C arriving mid-run: the
    queued seller is dropped or stopped, and the index is still written."""
    import pytest

    sellers = {
        "yupoo:sellerx": SELLER,
        "yupoo:sellery": Source("yupoo", "sellery", "yupoo",
                                "https://sellery.x.yupoo.com", "sellery"),
    }

    pressed: list[bool] = []

    def interrupt(_summary):
        if not pressed:  # once: a second press abandons the run
            pressed.append(True)
            raise KeyboardInterrupt

    with pytest.raises(KeyboardInterrupt):
        scrape_all(sellers, tmp_path, "2026-08-17T12:00:00+10:00",
                   client_factory=make_client, workers=1, on_done=interrupt)
    index = json.loads((tmp_path / "index.json").read_text(encoding="utf-8"))
    assert "sellerx" in [s["id"] for s in index["sellers"]]


def test_a_second_ctrl_c_abandons_running_sellers_but_still_writes_the_index(tmp_path: Path):
    """sellerx's report is the first press. sellery is stuck in a request, so
    only a second press (a timer here) can end the wait, and the call must
    return then without it. Its thread is a daemon, so it cannot hold the
    process open at exit either."""
    import _thread
    import threading
    import time

    import pytest

    release = threading.Event()

    class StuckClient(FakeClient):
        def get(self, url, referer=None):
            if "sellery" in url:
                release.wait(10)
            return super().get(url, referer)

    sellery = Source("yupoo", "sellery", "yupoo", "https://sellery.x.yupoo.com", "sellery")
    pressed: list[bool] = []

    def first_press(_summary):
        if not pressed:
            pressed.append(True)
            threading.Timer(0.3, _thread.interrupt_main).start()  # the second press
            raise KeyboardInterrupt

    started = time.monotonic()
    try:
        with pytest.raises(KeyboardInterrupt):
            scrape_all({"yupoo:sellerx": SELLER, "yupoo:sellery": sellery}, tmp_path,
                       "2026-08-17T12:00:00+10:00",
                       client_factory=lambda: StuckClient(make_client().pages),
                       workers=2, on_done=first_press)
        assert time.monotonic() - started < 5
        assert (tmp_path / "index.json").exists()
        still_running = [t for t in threading.enumerate()
                         if t is not threading.current_thread() and t.is_alive()
                         and not isinstance(t, threading.Timer)]
        assert still_running and all(t.daemon for t in still_running)
    finally:
        release.set()
