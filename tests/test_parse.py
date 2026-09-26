from pathlib import Path

import pytest

from lamp.parse import parse_album_list, parse_album_page

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = Path(__file__).parent / "fixtures"


def test_parse_album_list():
    stubs = parse_album_list((FIXTURES / "album_list.html").read_text(encoding="utf-8"))
    assert len(stubs) == 3
    first = stubs[0]
    assert first.id == "98765432"
    assert first.title == "Item A weidian.com/item.html?itemID=111"
    assert first.thumb_url == "https://photo.yupoo.com/sellerx/abcd1234/small.jpg"
    assert first.photo_count == 23
    assert stubs[1].photo_count == 0  # count missing on second card
    third = stubs[2]
    # Live markup as of 2026-08 carries the cover URL as a plain absolute src
    # (class "album__absolute album__img autocover"), no data-src at all.
    assert third.id == "98765434"
    assert third.thumb_url == "https://photo.yupoo.com/sellerx/ijkl9012/medium.jpg"
    assert third.photo_count == 0


def test_parse_album_list_empty():
    html = (FIXTURES / "album_list_empty.html").read_text(encoding="utf-8")
    assert parse_album_list(html) == []


def test_parse_album_page():
    detail = parse_album_page((FIXTURES / "album_page.html").read_text(encoding="utf-8"))
    assert detail.title == "Item A"
    assert "item.taobao.com/item.htm?id=222" in detail.description
    assert detail.category == "hoodies"
    assert detail.photo_count == 23
    assert detail.date_published == "2025-11-24"
    assert detail.date_modified == "2026-08-16"


def test_parse_album_page_missing_everything():
    detail = parse_album_page("<html><body></body></html>")
    assert (detail.title, detail.description, detail.category, detail.photo_count) == (
        "",
        "",
        "",
        0,
    )
    assert (detail.date_published, detail.date_modified) == ("", "")


def test_parse_album_page_malformed_ld_json():
    html = (
        "<html><body>"
        '<script type="application/ld+json">not json {</script>'
        '<script type="application/ld+json">{"datePublished": "garbage"}</script>'
        '<script type="application/ld+json">{"dateModified": "2026-01-02"}</script>'
        "</body></html>"
    )
    detail = parse_album_page(html)
    assert detail.date_published == ""
    assert detail.date_modified == "2026-01-02"


def test_parse_price():
    from lamp.parse import parse_price

    assert parse_price("Nike Tech Fleece ¥180 Black") == 180
    assert parse_price("Stussy Tee ￥150 White") == 150
    assert parse_price("Bape Hoodie 220元 M-XXL") == 220
    assert parse_price("Balenciaga Shoes P350 40-45") == 350
    assert parse_price("Prada Jacket 420P New") == 420
    assert parse_price("Arcteryx 380 RMB") == 380
    assert parse_price("Plain Title No Price") == 0
    assert parse_price("Size 180cm height") == 0
    assert parse_price("Micro price 5") == 0  # < 10 ignored
    assert parse_price("Nike Tech Fleece ¥180.50 Black") == 181
    assert parse_price("180.50 yuan") == 181
    assert parse_price("Jacket ¥1,280") == 1280
    assert parse_price("¥180黑色") == 180
    assert parse_price("黑色180元新款") == 180
    assert parse_price("¥5 ¥180") == 180
    assert parse_price("¥8.8 ¥180") == 180
    assert parse_price("¥100000 then ¥180") == 180


def test_parse_album_page_ld_json_array_and_graph_forms():
    """JSON-LD may wrap the same data in an array or an @graph, not just a bare object."""
    array_form = (
        '<html><body><script type="application/ld+json">'
        '[{"@type": "WebPage"}, {"datePublished": "2025-03-04"}]'
        "</script></body></html>"
    )
    assert parse_album_page(array_form).date_published == "2025-03-04"

    graph_form = (
        '<html><body><script type="application/ld+json">'
        '{"@context": "x", "@graph": [{"@type": "ImageGallery", '
        '"datePublished": "2025-05-06", "dateModified": "2026-05-06"}]}'
        "</script></body></html>"
    )
    detail = parse_album_page(graph_form)
    assert (detail.date_published, detail.date_modified) == ("2025-05-06", "2026-05-06")


def test_parse_album_page_dates_collected_across_blocks():
    """A block carrying only dateModified must not hide a later datePublished."""
    html = (
        "<html><body>"
        '<script type="application/ld+json">{"dateModified": "2026-02-02"}</script>'
        '<script type="application/ld+json">{"datePublished": "2024-09-09"}</script>'
        "</body></html>"
    )
    detail = parse_album_page(html)
    assert (detail.date_published, detail.date_modified) == ("2024-09-09", "2026-02-02")


@pytest.mark.manual
def test_live_album_list_parses():
    """Canary against real markup: pytest -m manual tests/test_parse.py -v

    The sample has to be saved by hand, and nothing in the repo does it for
    you: a scrape writes data/debug/<seller>-page1.html only for a page it
    could *not* parse, which is the one page this canary must never be
    pointed at. Save a page that works, from any seller in sources.toml,
    sending the project's own User-Agent (lamp.http.USER_AGENT):

        mkdir -p data/debug
        curl -A "$(python -c 'from lamp.http import USER_AGENT as u; print(u)')" \\
          "https://<seller>.x.yupoo.com/albums?page=1" \\
          -o data/debug/sample-list.html

    Resolved from the repo root rather than the working directory, so it
    finds the same file whichever directory pytest was started in.
    """
    sample = ROOT / "data" / "debug" / "sample-list.html"
    assert sample.exists(), f"save a live list page to {sample} first - see this docstring"
    stubs = parse_album_list(sample.read_text(encoding="utf-8", errors="replace"))
    assert stubs, "selectors no longer match live Yupoo markup"
    assert all(s.id and s.thumb_url for s in stubs)


def test_parse_album_list_second_template():
    """Yupoo's album3__ template: most sellers in a real collection used it (2026-08)."""
    stubs = parse_album_list((FIXTURES / "album_list_v2.html").read_text(encoding="utf-8"))
    assert [s.id for s in stubs] == ["246541366", "246541300"]
    first = stubs[0]
    assert first.title == "Item One https://weidian.com/item.html?itemID=11"
    # the real cover lives in data-origin-src; src holds a base64 placeholder
    assert first.thumb_url == "https://photo.yupoo.com/sellerv2/902e9b3f5c/medium.jpeg"
    assert first.photo_count == 7
    # protocol-relative cover URLs are still absolutised
    assert stubs[1].thumb_url == "https://photo.yupoo.com/sellerv2/abc123/medium.jpeg"
    assert stubs[1].photo_count == 12


def test_parse_album_list_never_returns_a_data_uri_thumb():
    """A base64 placeholder as thumb_url would make every download fail."""
    html = (
        '<a class="album3__main" href="/albums/5" title="x">'
        '<img class="album__img" src="data:image/png;base64,AAAA">'
        "</a>"
    )
    assert parse_album_list(html)[0].thumb_url == ""


def test_yupoo_no_cover_placeholder_is_not_a_thumb():
    """Albums without a cover point at Yupoo's own placeholder asset.

    Downloading it gives every such album an identical grey card, so it is
    reported as no cover and the viewer hides the image.
    """
    html = (
        '<a class="album3__main" href="/albums/7" title="No cover">'
        '<img class="album__img" data-origin-src='
        '"https://alfred-internal-1.cos.example.com/yupoo/album/im_photo_album.png">'
        "</a>"
    )
    assert parse_album_list(html)[0].thumb_url == ""

    # a real cover in the same position is still taken
    real = html.replace("im_photo_album.png", "medium.jpeg")
    assert parse_album_list(real)[0].thumb_url.endswith("medium.jpeg")


def test_placeholder_cover_does_not_hide_a_real_one_later_in_the_card():
    """A card holds several images; giving up on the first placeholder would
    discard a genuine cover sitting behind it."""
    html = (
        '<a class="album3__main" href="/albums/8" title="x">'
        '<img class="album__img" data-origin-src='
        '"https://cdn.example.com/yupoo/album/im_photo_album.png">'
        '<img class="album3__img" data-src="https://photo.example.com/s/real.jpg">'
        "</a>"
    )
    assert parse_album_list(html)[0].thumb_url == "https://photo.example.com/s/real.jpg"


def test_cover_image_wins_over_an_unrelated_image_rendered_first():
    """Only the cover classes are trusted first; a badge must not become the thumb."""
    html = (
        '<a class="album3__main" href="/albums/9" title="x">'
        '<img class="seller-badge" src="https://cdn.example.com/badge.png">'
        '<img class="album__img" data-origin-src="https://photo.example.com/s/cover.jpg">'
        "</a>"
    )
    assert parse_album_list(html)[0].thumb_url == "https://photo.example.com/s/cover.jpg"


def test_placeholder_match_is_exact_not_a_suffix():
    """A real cover merely ending in the placeholder name is still a cover."""
    html = (
        '<a class="album__main" href="/albums/10" title="x">'
        '<img class="album__img" src="https://photo.example.com/s/my_im_photo_album.png">'
        "</a>"
    )
    assert parse_album_list(html)[0].thumb_url.endswith("my_im_photo_album.png")


def test_price_evidence_preserves_selected_rule_and_ambiguous_candidates():
    from lamp.parse import price_evidence

    evidence = price_evidence("Offer ¥180, previous ¥220")
    assert evidence["amount"] == 180
    assert evidence["token"] == "¥180"
    assert evidence["rule"] == "currency"
    assert evidence["candidates"] == [180, 220]
    assert price_evidence("No price supplied") == {}
