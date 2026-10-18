//! Ported from tests/test_classify.py and tests/test_ingest.py.
mod common;

use lamp::ingest::{
    Source, Sources, classify_url, load_sources, merge_sources, parse_dump, write_sources,
};

fn shape(src: &Source) -> (String, String, String, String) {
    (
        src.kind.clone(),
        src.id.clone(),
        src.platform.py_str(),
        src.url_str(),
    )
}

#[test]
fn classify_url_cases() {
    let y = |id: &str, url: &str| {
        (
            "yupoo".to_string(),
            id.to_string(),
            "yupoo".to_string(),
            url.to_string(),
        )
    };
    let s = |id: &str, p: &str, url: &str| {
        (
            "shop".to_string(),
            id.to_string(),
            p.to_string(),
            url.to_string(),
        )
    };
    let x = "https://sellerx.x.yupoo.com";
    let cases = vec![
        ("https://sellerx.x.yupoo.com/", y("sellerx", x)),
        ("http://sellerx.x.yupoo.com/albums", y("sellerx", x)),
        ("https://sellerx.x.yupoo.com/album", y("sellerx", x)),
        ("https://sellerx.x.yupoo.com/categories", y("sellerx", x)),
        (
            "https://sellerx.x.yupoo.com/categories/4383841",
            y("sellerx", x),
        ),
        (
            "https://sellerx.x.yupoo.com/collections/4730261",
            y("sellerx", x),
        ),
        (
            "https://sellerx.x.yupoo.com/albums?tab=gallery",
            y("sellerx", x),
        ),
        ("https://sellerx.x.yupoo.com/?lang=en-us", y("sellerx", x)),
        ("sellerx.x.yupoo.com", y("sellerx", x)),
        (
            "https://www.sellerx.x.yupoo.com/",
            y("www-sellerx", "https://www.sellerx.x.yupoo.com"),
        ),
        (
            "https://shop00000001.taobao.com/",
            s(
                "taobao-shop00000001",
                "taobao",
                "https://shop00000001.taobao.com/",
            ),
        ),
        (
            "https://someclub.world.taobao.com/",
            s(
                "taobao-someclub-world",
                "taobao",
                "https://someclub.world.taobao.com/",
            ),
        ),
        (
            "https://weidian.com/?userid=1000000001",
            s(
                "weidian-1000000001",
                "weidian",
                "https://weidian.com/?userid=1000000001",
            ),
        ),
        (
            "https://weidian.com/?userid=1000000002&spider_token=cd8f&tabType=all",
            s(
                "weidian-1000000002",
                "weidian",
                "https://weidian.com/?userid=1000000002",
            ),
        ),
        (
            "https://weidian.com/?userid=1.2",
            s("weidian-com", "web", "https://weidian.com/?userid=1.2"),
        ),
        (
            "https://k.youshop10.com/AbCd1234",
            s(
                "youshop-abcd1234",
                "weidian-shortlink",
                "https://k.youshop10.com/AbCd1234",
            ),
        ),
        (
            "https://vlink.cc/some_shop",
            s(
                "vlink-some-shop",
                "aggregator",
                "https://vlink.cc/some_shop",
            ),
        ),
        (
            "https://linktr.ee/someshop",
            s(
                "linktr-someshop",
                "aggregator",
                "https://linktr.ee/someshop",
            ),
        ),
        (
            "http://www.example-store.ru/",
            s("example-store-ru", "web", "http://www.example-store.ru/"),
        ),
        (
            "https://shop.example/coupon/?discount=X",
            s(
                "shop-example",
                "web",
                "https://shop.example/coupon/?discount=X",
            ),
        ),
        ("\"https://sellerx.x.yupoo.com/\"", y("sellerx", x)),
        ("'https://sellerx.x.yupoo.com/'", y("sellerx", x)),
    ];
    for (raw, expected) in cases {
        assert_eq!(shape(&classify_url(raw).expect(raw)), expected, "{raw}");
    }
}

#[test]
fn classify_rejects_junk_malformed_and_unsafe() {
    for raw in [
        "N/A",
        "",
        "not a url at all",
        "https://",
        "https://[seller.x.yupoo.com/",
        "https://[not-an-ipv6]/albums",
        "https://seller.x.yupoo.com\u{ff0f}albums",
        "ftp://example.test/path",
        "javascript://example.test/path",
        "https://user:password@example.test/path",
        "https://user@example.test/path",
        "\u{0}\u{0}",
    ] {
        assert!(classify_url(raw).is_none(), "{raw:?}");
    }
    assert_eq!(
        classify_url("https://sellerx.x.yupoo.com/")
            .unwrap()
            .name
            .py_str(),
        "sellerx"
    );
}

#[test]
fn classify_strips_control_and_invisible_characters() {
    let src = classify_url("seller.x.yupoo.com\u{0}.evil.com").unwrap();
    assert!(!src.url_str().contains('\0') && !src.name.py_str().contains('\0'));
    assert!(
        !classify_url("https://sell\u{200b}er.x.yupoo.com/")
            .unwrap()
            .url_str()
            .contains('\u{200b}')
    );
}

#[test]
fn source_names_roundtrip_control_characters() {
    let dir = common::tmp();
    let name = "quote\" slash\\ tab\t newline\n carriage\r backspace\u{8} formfeed\u{c} nul\u{0} del\u{7f}";
    let source = Source::new("shop", "example", "web", "https://example.test/", name);
    let path = dir.path().join("sources.toml");
    let sources: Sources = [("shop:example".to_string(), source.clone())]
        .into_iter()
        .collect();
    write_sources(&path, &sources).unwrap();
    assert_eq!(load_sources(&path).unwrap(), sources);
}

const DUMP: &str = "1\thttps://sellerx.x.yupoo.com/\n2\thttps://sellerx.x.yupoo.com/albums\n3\thttps://shop00000001.taobao.com/\n4\tN/A\n\n5\thttps://weidian.com/?userid=99&spider_token=zz\nhttps://sellery.x.yupoo.com/categories\n7,https://sellerz.x.yupoo.com/\n";

#[test]
fn parse_dump_strips_indexes_blanks_and_quotes() {
    assert_eq!(
        parse_dump(DUMP),
        [
            "https://sellerx.x.yupoo.com/",
            "https://sellerx.x.yupoo.com/albums",
            "https://shop00000001.taobao.com/",
            "N/A",
            "https://weidian.com/?userid=99&spider_token=zz",
            "https://sellery.x.yupoo.com/categories",
            "https://sellerz.x.yupoo.com/",
        ]
    );
    for (input, expected) in [
        (
            "https://weidian.com/item.html?itemID=1,2\n",
            "https://weidian.com/item.html?itemID=1,2",
        ),
        (
            "123,https://seller.x.yupoo.com/\n",
            "https://seller.x.yupoo.com/",
        ),
        ("12, https://spaced.example/\n", "https://spaced.example/"),
        (
            "1,2,https://seller.x.yupoo.com/\n",
            "https://seller.x.yupoo.com/",
        ),
        ("12\t34\thttps://s.x.yupoo.com/\n", "https://s.x.yupoo.com/"),
        (
            "1,https://weidian.com/i.html?itemID=1,2\n",
            "https://weidian.com/i.html?itemID=1,2",
        ),
        (
            "\"1\",\"https://seller.x.yupoo.com/\"\n",
            "https://seller.x.yupoo.com/",
        ),
        (
            "1,\"https://seller.x.yupoo.com/\"\n",
            "https://seller.x.yupoo.com/",
        ),
        (
            "\"https://seller.x.yupoo.com/\"\n",
            "https://seller.x.yupoo.com/",
        ),
        (
            "\"1\"\t\"https://seller.x.yupoo.com/\"\n",
            "https://seller.x.yupoo.com/",
        ),
        (
            "'https://seller.x.yupoo.com/'\n",
            "https://seller.x.yupoo.com/",
        ),
    ] {
        assert_eq!(parse_dump(input), [expected], "{input:?}");
    }
}

fn rows(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn merge_counts_malformed_rows_as_invalid() {
    let (merged, report) = merge_sources(
        &Sources::new(),
        &rows(&[
            "https://sellerx.x.yupoo.com/",
            "https://[not-an-ipv6]/albums",
            "https://sellery.x.yupoo.com/",
        ]),
    );
    assert_eq!(
        merged
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["yupoo:sellerx", "yupoo:sellery"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    assert_eq!((report.rows, report.invalid, report.yupoo_new), (3, 1, 2));
}

#[test]
fn merge_dedupes_counts_and_is_idempotent() {
    let (first, report) = merge_sources(&Sources::new(), &parse_dump(DUMP));
    let mut keys: Vec<&String> = first.keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "shop:taobao-shop00000001",
            "shop:weidian-99",
            "yupoo:sellerx",
            "yupoo:sellery",
            "yupoo:sellerz"
        ]
    );
    assert_eq!(
        (
            report.rows,
            report.invalid,
            report.yupoo_new,
            report.shops_new
        ),
        (7, 1, 3, 2)
    );
    let mut platforms = report.shops_by_platform.clone();
    platforms.sort();
    assert_eq!(
        platforms,
        [("taobao".to_string(), 1), ("weidian".to_string(), 1)]
    );
    let mut edited = first.clone();
    edited.get_mut("yupoo:sellerx").unwrap().name = "My Renamed Seller".into();
    let (second, report) = merge_sources(&edited, &parse_dump(DUMP));
    assert_eq!(second, edited);
    assert_eq!(
        (
            report.yupoo_new,
            report.shops_new,
            report.yupoo_known,
            report.shops_known
        ),
        (0, 0, 3, 2)
    );
}

#[test]
fn toml_roundtrip_missing_file_and_unsafe_id() {
    let dir = common::tmp();
    let (merged, _) = merge_sources(&Sources::new(), &parse_dump(DUMP));
    let path = dir.path().join("sources.toml");
    write_sources(&path, &merged).unwrap();
    assert_eq!(load_sources(&path).unwrap(), merged);
    assert!(
        load_sources(&dir.path().join("nope.toml"))
            .unwrap()
            .is_empty()
    );
    let bad = Source::new(
        "shop",
        "weidian-1.2",
        "web",
        "https://weidian.com/?userid=1.2",
        "weidian 1.2",
    );
    let sources: Sources = [("shop:weidian-1.2".to_string(), bad)]
        .into_iter()
        .collect();
    assert!(write_sources(&dir.path().join("s.toml"), &sources).is_err());
}

#[test]
fn skip_flag_round_trips_and_survives_reimport() {
    let dir = common::tmp();
    let path = dir.path().join("sources.toml");
    let mut huge = Source::new("yupoo", "huge", "yupoo", "https://huge.x.yupoo.com", "huge");
    huge.skip = true;
    let keep = Source::new("yupoo", "keep", "yupoo", "https://keep.x.yupoo.com", "keep");
    let sources: Sources = [
        ("yupoo:huge".to_string(), huge),
        ("yupoo:keep".to_string(), keep),
    ]
    .into_iter()
    .collect();
    write_sources(&path, &sources).unwrap();
    let loaded = load_sources(&path).unwrap();
    assert!(loaded["yupoo:huge"].skip && !loaded["yupoo:keep"].skip);
    let (merged, _) = merge_sources(
        &loaded,
        &rows(&["https://huge.x.yupoo.com", "https://new.x.yupoo.com"]),
    );
    assert!(merged["yupoo:huge"].skip);
    write_sources(&path, &merged).unwrap();
    assert!(load_sources(&path).unwrap()["yupoo:huge"].skip);
    assert!(
        !common::read(&path)
            .split("[yupoo.new]")
            .nth(1)
            .unwrap()
            .contains("skip")
    );
}

#[test]
fn load_sources_strips_a_trailing_slash() {
    let dir = common::tmp();
    let path = dir.path().join("sources.toml");
    common::write(
        &path,
        "[yupoo.s]\nurl = \"https://s.x.yupoo.com/\"\nname = \"s\"\n",
    );
    assert_eq!(
        load_sources(&path).unwrap()["yupoo:s"].url_str(),
        "https://s.x.yupoo.com"
    );
}

#[test]
fn colliding_slugs_are_reported() {
    let (merged, report) = merge_sources(
        &Sources::new(),
        &rows(&["https://foo.bar.x.yupoo.com", "https://foo-bar.x.yupoo.com"]),
    );
    assert_eq!(report.yupoo_new, 1);
    assert_eq!(
        report.collisions,
        [(
            "yupoo:foo-bar".to_string(),
            "https://foo.bar.x.yupoo.com".to_string(),
            "https://foo-bar.x.yupoo.com".to_string()
        )]
    );
    assert_eq!(
        merged["yupoo:foo-bar"].url_str(),
        "https://foo.bar.x.yupoo.com"
    );
    let (_, report) = merge_sources(
        &Sources::new(),
        &rows(&["https://seller.x.yupoo.com", "https://seller.x.yupoo.com"]),
    );
    assert_eq!(report.yupoo_new, 1);
    assert!(report.collisions.is_empty());
}
