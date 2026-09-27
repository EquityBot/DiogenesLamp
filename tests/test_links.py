import re
from pathlib import Path

import pytest

from lamp.links import (
    _AGENT_HOSTS,
    _SHORTLINK_HOSTS,
    ITEM_URLS,
    Link,
    _platform,
    extract_links,
    item_link,
)

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
    # Scheme-less URLs glued to CJK must still parse, and the CJK must not
    # become part of the hostname.
    (
        "购买weidian.com/item.html?itemID=42",
        [Link("weidian", "https://weidian.com/item.html?itemID=42")],
    ),
    (
        "链接item.taobao.com/item.htm?id=1马上买",
        [Link("taobao", "https://item.taobao.com/item.htm?id=1")],
    ),
    (
        "购买https://item.taobao.com/item.htm?id=9",
        [Link("taobao", "https://item.taobao.com/item.htm?id=9")],
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


def _viewer(name: str) -> str:
    return (Path(__file__).resolve().parents[1] / "viewer" / name).read_text(encoding="utf-8")


def test_shortlink_hosts_match_the_viewer_dedup_list():
    """A shortlink host the scraper stores but the viewer cannot key on would
    silently stop those albums from deduplicating."""
    block = re.search(r"SHORTLINK_HOSTS = \[([^\]]*)\]", _viewer("dedup.js")).group(1)
    assert set(re.findall(r'"([^"]+)"', block)) == set(_SHORTLINK_HOSTS)


def test_agent_hosts_match_the_viewer_agent_table():
    """An agent the scraper unwraps but the viewer cannot recognise would make
    a pasted link of that agent miss the album the scraper keyed from it."""
    assert set(re.findall(r'host: "([^"]+)"', _viewer("agents.js"))) == set(_AGENT_HOSTS)


def test_item_urls_match_the_viewer_canonical_forms():
    """Both sides rebuild a marketplace link from a bare id; they must agree."""
    block = re.search(r"const ITEM_URLS = \{(.*?)\};", _viewer("dedup.js"), re.S).group(1)
    prefixes = dict(re.findall(r'"?([a-z0-9]+)"?: "([^"]+)"', block))
    assert set(prefixes) == set(ITEM_URLS)
    for platform, template in ITEM_URLS.items():
        suffix = ".html" if platform == "1688" else ""
        assert template.format(id="123") == prefixes[platform] + "123" + suffix


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
            assert all(ord(ch) > 0x20 and ch not in "\x7f\ufeff\u200b\u200c\u200d\u2060"
                       for ch in link.url), link.url


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


@pytest.mark.parametrize("text", [
    "taobao.com.evil.example/item?id=5",
    "taobao.com.cn/item?id=5",
    "weidian.com.evil.example/item",
])
def test_a_suffix_lookalike_never_collapses_onto_the_real_host(text):
    # The lookahead rejected only alphanumerics and hyphens, so a following dot
    # ended the host: taobao.com.evil.example stored a link to taobao.com itself.
    assert extract_links(text) == []


@pytest.mark.parametrize("text,host", [
    ("taobao.com/item?id=5", "taobao.com"),
    ("item.taobao.com/x", "item.taobao.com"),
    ("buy from taobao.com. Then wait.", "taobao.com"),
])
def test_valid_hosts_still_match(text, host):
    links = extract_links(text)
    assert links and host in links[0].url


WD = Link("weidian", "https://weidian.com/item.html?itemID=7652831688")
TB = Link("taobao", "https://item.taobao.com/item.htm?id=560198996624")
ALI = Link("1688", "https://detail.1688.com/offer/691234567890.html")
WD_ENC = "https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D7652831688"

# Agent formats read from the public link converters on 27 September 2026.
AGENT_CASES = [
    (f"https://www.kakobuy.com/item/details?url={WD_ENC}&affcode=thelude", [WD]),
    ("https://hoobuy.com/product/1/560198996624", [TB]),
    ("https://hoobuy.com/product/0/691234567890", [ALI]),
    ("https://hoobuy.com/product/2/7652831688", [WD]),
    ("https://oopbuy.com/goods/details?id=7652831688&channel=weidian", [WD]),
    ("https://oopbuy.com/goods/details?id=691234567890&channel=1688", [ALI]),
    ("https://mulebuy.com/product/?shop_type=ali_1688&id=691234567890", [ALI]),
    ("https://joyagoo.com/product/?shop_type=taobao&id=560198996624", [TB]),
    # A retired agent's link still names the item.
    ("https://cnfans.com/product/?shop_type=weidian&id=7652831688", [WD]),
    ("https://www.cssbuy.com/item-micro-7652831688.html", [WD]),
    ("https://www.cssbuy.com/item-1688-691234567890.html", [ALI]),
    ("https://www.cssbuy.com/item-560198996624.html", [TB]),
    ("https://www.sugargoo.com/productDetail?productLink=https%3A%2F%2Fdetail.1688.com%2Foffer%2F691234567890.html", [ALI]),
    ("https://www.superbuy.com/en/page/buy/?url=https%3A%2F%2Fitem.taobao.com%2Fitem.htm%3Fid%3D560198996624", [TB]),
    (f"https://www.loongbuy.com/product-details?url={WD_ENC}", [WD]),
    ("https://www.pikobuy.com/product/detail?productUrl=https%3A%2F%2Fdetail.1688.com%2Foffer%2F691234567890.html", [ALI]),
    # One agent quoting another is one more unwrap; a third layer is not followed.
    ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fhoobuy.com%2Fproduct%2F2%2F7652831688", [WD]),
    ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fwww.kakobuy.com%2Fitem%2Fdetails%3Furl%3D"
     "https%253A%252F%252Fhoobuy.com%252Fproduct%252F2%252F7652831688", []),
    # 1688 direct forms keep the host they were given and drop tracking.
    ("https://detail.1688.com/offer/691234567890.html?spm=a2615.7691456", [ALI]),
    ("m.1688.com/offer/691234567890.html", [Link("1688", "https://m.1688.com/offer/691234567890.html")]),
    # Glued to prose like every other link.
    ("购买https://hoobuy.com/product/2/7652831688，尺码咨询", [WD]),
    # Refused: other sites wrapped by an agent, ambiguity, unknown channels, lookalikes, credentials.
    ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fevil.example%2Fitem.html%3FitemID%3D1", []),
    ("https://hoobuy.com/product/9/123", []),
    ("https://oopbuy.com/goods/details?id=5&channel=unknown", []),
    ("https://oopbuy.com/goods/details?id=5&id=6&channel=weidian", []),
    ("https://mulebuy.com/product/?shop_type=weidian&channel=taobao&id=5", []),
    ("https://oopbuy.com/goods/details?id=abc&channel=weidian", []),
    (f"https://notkakobuy.com/item/details?url={WD_ENC}", []),
    ("https://weidian.com@evil.example/item.html?itemID=123", []),
    ("https://user@weidian.com/item.html?itemID=123", []),
]


@pytest.mark.parametrize("text,expected", AGENT_CASES)
def test_agent_links_unwrap_to_the_marketplace_item(text, expected):
    assert extract_links(text) == expected


def test_an_agent_link_and_the_direct_link_are_one_entry():
    text = f"https://hoobuy.com/product/2/7652831688 or {WD.url}"
    assert extract_links(text) == [WD]


def test_invisible_characters_end_a_url_instead_of_splicing_it():
    """"id=12<NUL>34" must not become id 1234, an id nobody wrote."""
    links = extract_links("weidian.com/item.html?itemID=12\x0034")
    assert links == [Link("weidian", "https://weidian.com/item.html?itemID=12")]


@pytest.mark.parametrize("platform,item_id,expected", [
    ("taobao", "1", TB.url.replace("560198996624", "1")),
    ("1688", "691234567890", ALI.url),
    ("weidian", "x", None),
    ("nowhere", "1", None),
])
def test_item_link_builds_only_well_formed_items(platform, item_id, expected):
    link = item_link(platform, item_id)
    assert (link.url if link else None) == expected


@pytest.mark.parametrize("text,expected", [
    # The wrapped value is judged as one URL by its own host: a marketplace
    # URL riding inside another site's query string names nothing.
    ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fevil.example%2F%3Fnext%3Dhttps%3A%2F%2F"
     "weidian.com%2Fitem.html%3FitemID%3D123", []),
    ("https://www.kakobuy.com/item/details?url=weidian.com%2Fitem.html%3FitemID%3D123", [
        Link("weidian", "https://weidian.com/item.html?itemID=123")]),
    ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fitem.taobao.com%2Fitem.htm%3Fid%3D5%23detail", [
        Link("taobao", "https://item.taobao.com/item.htm?id=5")]),
    ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fweidian.com%40evil.example%2Fitem.html%3FitemID%3D1", []),
])
def test_wrapped_urls_are_judged_whole(text, expected):
    assert extract_links(text) == expected


@pytest.mark.parametrize("text,expected", [
    # The decoded wrapped URL gets the same cleanup as prose candidates: an
    # invisible character ends it and trailing punctuation falls away.
    ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D12%0034", [
        Link("weidian", "https://weidian.com/item.html?itemID=12")]),
    ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fitem.taobao.com%2Fitem.htm%3Fid%3D5%29%E3%80%82", [
        Link("taobao", "https://item.taobao.com/item.htm?id=5")]),
    ("https://www.sugargoo.com/productDetail?productLink=https%3A%2F%2Fdetail.1688.com%2Foffer%2F7.html%E2%80%8B", [
        Link("1688", "https://detail.1688.com/offer/7.html")]),
])
def test_wrapped_urls_get_the_same_cleanup_as_prose(text, expected):
    assert extract_links(text) == expected
