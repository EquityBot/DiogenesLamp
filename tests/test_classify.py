import pytest

from lamp.ingest import Source, classify_url

CASES = [
    # Yupoo shapes observed in real dumps: every path variant collapses to the seller root.
    ("https://sellerx.x.yupoo.com/", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("http://sellerx.x.yupoo.com/albums", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("https://sellerx.x.yupoo.com/album", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("https://sellerx.x.yupoo.com/categories", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("https://sellerx.x.yupoo.com/categories/4383841", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("https://sellerx.x.yupoo.com/collections/4730261", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("https://sellerx.x.yupoo.com/albums?tab=gallery", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("https://sellerx.x.yupoo.com/?lang=en-us", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("sellerx.x.yupoo.com", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    # Multi-label subdomain: id flattened to one slug label (dots would nest TOML tables).
    ("https://www.sellerx.x.yupoo.com/", ("yupoo", "www-sellerx", "yupoo", "https://www.sellerx.x.yupoo.com")),
    # Taobao shops.
    ("https://shop00000001.taobao.com/", ("shop", "taobao-shop00000001", "taobao", "https://shop00000001.taobao.com/")),
    ("https://someclub.world.taobao.com/", ("shop", "taobao-someclub-world", "taobao", "https://someclub.world.taobao.com/")),
    # Weidian shops: keep only userid, strip tracking params.
    ("https://weidian.com/?userid=1000000001", ("shop", "weidian-1000000001", "weidian", "https://weidian.com/?userid=1000000001")),
    (
        "https://weidian.com/?userid=1000000002&spider_token=cd8f&tabType=all",
        ("shop", "weidian-1000000002", "weidian", "https://weidian.com/?userid=1000000002"),
    ),
    # Weidian userid must be numeric: a non-numeric value falls through to the web fallback
    # rather than producing a TOML-unsafe id.
    ("https://weidian.com/?userid=1.2", ("shop", "weidian-com", "web", "https://weidian.com/?userid=1.2")),
    # Weidian short links: stored as-is.
    ("https://k.youshop10.com/AbCd1234", ("shop", "youshop-abcd1234", "weidian-shortlink", "https://k.youshop10.com/AbCd1234")),
    # Aggregators.
    ("https://vlink.cc/some_shop", ("shop", "vlink-some-shop", "aggregator", "https://vlink.cc/some_shop")),
    ("https://linktr.ee/someshop", ("shop", "linktr-someshop", "aggregator", "https://linktr.ee/someshop")),
    # Anything else: generic web shop, original URL kept.
    ("http://www.example-store.ru/", ("shop", "example-store-ru", "web", "http://www.example-store.ru/")),
    ("https://shop.example/coupon/?discount=X", ("shop", "shop-example", "web", "https://shop.example/coupon/?discount=X")),
    # Quoted URLs from CSV dumps.
    ('"https://sellerx.x.yupoo.com/"', ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
    ("'https://sellerx.x.yupoo.com/'", ("yupoo", "sellerx", "yupoo", "https://sellerx.x.yupoo.com")),
]


@pytest.mark.parametrize("raw,expected", CASES)
def test_classify_url(raw, expected):
    src = classify_url(raw)
    assert src is not None
    assert (src.kind, src.id, src.platform, src.url) == expected


@pytest.mark.parametrize("raw", ["N/A", "", "not a url at all", "https://"])
def test_classify_rejects_junk(raw):
    assert classify_url(raw) is None


# urlparse and its hostname accessor raise ValueError on these three, which used
# to escape classify_url and stop the whole import instead of counting one
# invalid row.
@pytest.mark.parametrize("raw", [
    "https://[seller.x.yupoo.com/",
    "https://[not-an-ipv6]/albums",
    "https://seller.x.yupoo.com／albums",
])
def test_classify_rejects_malformed_authority_without_raising(raw):
    assert classify_url(raw) is None


def test_name_defaults_to_id_for_yupoo():
    assert classify_url("https://sellerx.x.yupoo.com/").name == "sellerx"
