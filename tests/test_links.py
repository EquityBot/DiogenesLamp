import re
from pathlib import Path

import pytest

from lamp.links import _SHORTLINK_HOSTS, Link, _platform, extract_links

CASES = [
    (
        "buy here https://item.taobao.com/item.htm?id=123&spm=track.me",
        [Link("taobao", "https://item.taobao.com/item.htm?id=123")],
    ),
    (
        "TMALL: https://detail.tmall.com/item.htm?id=456&scm=zzz",
        [Link("tmall", "https://detail.tmall.com/item.htm?id=456")],
    ),
    (
        "wd https://weidian.com/item.html?itemID=789&spider_token=aa",
        [Link("weidian", "https://weidian.com/item.html?itemID=789")],
    ),
    (
        "shop https://weidian.com/?userid=555&tabType=all",
        [Link("weidian", "https://weidian.com/?userid=555")],
    ),
    (
        "shop https://shop123.v.weidian.com/?x=1",
        [Link("weidian", "https://shop123.v.weidian.com/")],
    ),
    ("short https://m.tb.cn/h.ABCdef", [Link("other", "https://m.tb.cn/h.ABCdef")]),
    ("short https://k.youshop10.com/AbCd1234", [Link("other", "https://k.youshop10.com/AbCd1234")]),
    ("no links in this title at all", []),
    ("bare host weidian.com/item.html?itemID=42", [Link("weidian", "https://weidian.com/item.html?itemID=42")]),
    # Garbage / near-miss URLs must not match.
    ("nothing notataobao.com/item here", []),
    ("brand taobao.company store", []),
    # Trailing prose punctuation must not be absorbed into the URL.
    (
        "buy https://item.taobao.com/item.htm?id=1.",
        [Link("taobao", "https://item.taobao.com/item.htm?id=1")],
    ),
    (
        "(https://weidian.com/item.html?itemID=2)",
        [Link("weidian", "https://weidian.com/item.html?itemID=2")],
    ),
    # CJK brackets and punctuation surrounding or following URLs without spaces.
    (
        "【https://item.taobao.com/item.htm?id=1001】",
        [Link("taobao", "https://item.taobao.com/item.htm?id=1001")],
    ),
    (
        "（https://weidian.com/item.html?itemID=2002）",
        [Link("weidian", "https://weidian.com/item.html?itemID=2002")],
    ),
    (
        "购买链接：https://item.taobao.com/item.htm?id=3003，尺码咨询客服",
        [Link("taobao", "https://item.taobao.com/item.htm?id=3003")],
    ),
    (
        "淘宝：https://m.tb.cn/h.ABCdef快来看看",
        [Link("other", "https://m.tb.cn/h.ABCdef")],
    ),
    (
        "微店https://k.youshop10.com/AbCd1234。",
        [Link("other", "https://k.youshop10.com/AbCd1234")],
    ),
]


@pytest.mark.parametrize("text,expected", CASES)
def test_extract_links(text, expected):
    assert extract_links(text) == expected


def test_dedupes_and_preserves_order():
    text = (
        "https://item.taobao.com/item.htm?id=1 then "
        "https://weidian.com/item.html?itemID=2 and again "
        "https://item.taobao.com/item.htm?id=1"
    )
    assert extract_links(text) == [
        Link("taobao", "https://item.taobao.com/item.htm?id=1"),
        Link("weidian", "https://weidian.com/item.html?itemID=2"),
    ]


def test_fragments_are_stripped_so_the_same_item_matches_itself():
    """A '#detail' swallowed from surrounding prose must not enter the item id."""
    with_fragment = extract_links("buy https://item.taobao.com/item.htm?id=695#detail now")
    clean = extract_links("buy https://item.taobao.com/item.htm?id=695 now")
    assert with_fragment == clean
    assert with_fragment[0].url == "https://item.taobao.com/item.htm?id=695"
    # shortlinks take the same treatment
    assert extract_links("https://m.tb.cn/h.ABC#x")[0].url == "https://m.tb.cn/h.ABC"


def test_shortlink_hosts_match_the_viewer_dedup_list():
    """A shortlink host the scraper stores but the viewer cannot key on would
    silently stop those albums from deduplicating."""
    dedup_js = (Path(__file__).resolve().parents[1] / "viewer" / "dedup.js").read_text(
        encoding="utf-8"
    )
    viewer_hosts = set(re.findall(r'url\.hostname === "([^"]+)"', dedup_js))
    assert viewer_hosts == set(_SHORTLINK_HOSTS)


def test_control_and_invisible_characters_never_reach_a_url():
    """Album text is remote markup: NULs, zero-width spaces and BOMs must be
    scrubbed, not stored. A NUL in a link poisons dedup keys and any
    downstream consumer that validates URLs."""
    for text in (
        "taobao.com/item.htm?id=123\x00evil",
        "taobao.com/item.htm?id=123\u200b\u200d",
        "\ufefftaobao.com/x.htm?id=5",
        "k.youshop10.com/abc\x00def",
        "m.tb.cn/h.Xyz\u200b12",
    ):
        for link in extract_links(text):
            assert link.url == link.url.strip()
            assert all(ord(ch) > 0x7E or 0x20 < ord(ch) for ch in link.url), link.url


@pytest.mark.parametrize(
    "host,platform",
    [
        ("tmall.com", "tmall"),
        ("detail.tmall.com", "tmall"),
        ("taobao.com", "taobao"),
        ("item.taobao.com", "taobao"),
        ("weidian.com", "weidian"),
        ("shop123.v.weidian.com", "weidian"),
        ("m.tb.cn", "other"),
        ("k.youshop10.com", "other"),
        # A hyphenated lookalike is a different registrable domain. The old
        # endswith("tmall.com") check treated these as marketplace hosts.
        ("evil-tmall.com", "other"),
        ("not-taobao.com", "other"),
        ("prefix-weidian.com", "other"),
    ],
)
def test_platform_uses_a_hostname_label_boundary(host, platform):
    assert _platform(host) == platform
