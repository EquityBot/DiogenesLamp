//! Ported from tests/test_links.py.
mod common;

use std::collections::BTreeSet;

use lamp::links::{
    AGENT_HOSTS, ITEM_URLS, Link, SHORTLINK_HOSTS, extract_links, item_link, platform,
};

fn l(platform: &str, url: &str) -> Link {
    Link {
        platform: platform.into(),
        url: url.into(),
    }
}

fn check(cases: &[(&str, Vec<Link>)]) {
    for (text, expected) in cases {
        assert_eq!(&extract_links(text), expected, "{text}");
    }
}

#[test]
fn extract_links_cases() {
    check(&[
        (
            "buy here https://item.taobao.com/item.htm?id=123&spm=track.me",
            vec![l("taobao", "https://item.taobao.com/item.htm?id=123")],
        ),
        (
            "TMALL: https://detail.tmall.com/item.htm?id=456&scm=zzz",
            vec![l("tmall", "https://detail.tmall.com/item.htm?id=456")],
        ),
        (
            "wd https://weidian.com/item.html?itemID=789&spider_token=aa",
            vec![l("weidian", "https://weidian.com/item.html?itemID=789")],
        ),
        (
            "shop https://weidian.com/?userid=555&tabType=all",
            vec![l("weidian", "https://weidian.com/?userid=555")],
        ),
        (
            "shop https://shop123.v.weidian.com/?x=1",
            vec![l("weidian", "https://shop123.v.weidian.com/")],
        ),
        (
            "short https://m.tb.cn/h.ABCdef",
            vec![l("other", "https://m.tb.cn/h.ABCdef")],
        ),
        (
            "short https://k.youshop10.com/AbCd1234",
            vec![l("other", "https://k.youshop10.com/AbCd1234")],
        ),
        ("no links in this title at all", vec![]),
        (
            "bare host weidian.com/item.html?itemID=42",
            vec![l("weidian", "https://weidian.com/item.html?itemID=42")],
        ),
        ("nothing notataobao.com/item here", vec![]),
        ("brand taobao.company store", vec![]),
        (
            "buy https://item.taobao.com/item.htm?id=1.",
            vec![l("taobao", "https://item.taobao.com/item.htm?id=1")],
        ),
        (
            "(https://weidian.com/item.html?itemID=2)",
            vec![l("weidian", "https://weidian.com/item.html?itemID=2")],
        ),
        (
            "【https://item.taobao.com/item.htm?id=1001】",
            vec![l("taobao", "https://item.taobao.com/item.htm?id=1001")],
        ),
        (
            "（https://weidian.com/item.html?itemID=2002）",
            vec![l("weidian", "https://weidian.com/item.html?itemID=2002")],
        ),
        (
            "购买链接：https://item.taobao.com/item.htm?id=3003，尺码咨询客服",
            vec![l("taobao", "https://item.taobao.com/item.htm?id=3003")],
        ),
        (
            "淘宝：https://m.tb.cn/h.ABCdef快来看看",
            vec![l("other", "https://m.tb.cn/h.ABCdef")],
        ),
        (
            "微店https://k.youshop10.com/AbCd1234。",
            vec![l("other", "https://k.youshop10.com/AbCd1234")],
        ),
        (
            "购买weidian.com/item.html?itemID=42",
            vec![l("weidian", "https://weidian.com/item.html?itemID=42")],
        ),
        (
            "链接item.taobao.com/item.htm?id=1马上买",
            vec![l("taobao", "https://item.taobao.com/item.htm?id=1")],
        ),
        (
            "购买https://item.taobao.com/item.htm?id=9",
            vec![l("taobao", "https://item.taobao.com/item.htm?id=9")],
        ),
    ]);
}

#[test]
fn dedupes_and_preserves_order() {
    let text = "https://item.taobao.com/item.htm?id=1 then https://weidian.com/item.html?itemID=2 and again https://item.taobao.com/item.htm?id=1";
    assert_eq!(
        extract_links(text),
        vec![
            l("taobao", "https://item.taobao.com/item.htm?id=1"),
            l("weidian", "https://weidian.com/item.html?itemID=2")
        ]
    );
}

#[test]
fn fragments_are_stripped() {
    let with = extract_links("buy https://item.taobao.com/item.htm?id=695#detail now");
    assert_eq!(
        with,
        extract_links("buy https://item.taobao.com/item.htm?id=695 now")
    );
    assert_eq!(with[0].url, "https://item.taobao.com/item.htm?id=695");
    assert_eq!(
        extract_links("https://m.tb.cn/h.ABC#x")[0].url,
        "https://m.tb.cn/h.ABC"
    );
}

fn viewer(name: &str) -> String {
    common::read(&common::root().join("viewer").join(name))
}

fn quoted(block: &str) -> BTreeSet<String> {
    block
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn shortlink_hosts_match_the_viewer_dedup_list() {
    let source = viewer("dedup.js");
    let start = source.find("SHORTLINK_HOSTS = [").unwrap() + "SHORTLINK_HOSTS = [".len();
    let block = &source[start..start + source[start..].find(']').unwrap()];
    assert_eq!(
        quoted(block),
        SHORTLINK_HOSTS.iter().map(|s| s.to_string()).collect()
    );
}

#[test]
fn agent_hosts_match_the_viewer_agent_table() {
    let source = viewer("agents.js");
    let hosts: BTreeSet<String> = source
        .match_indices("host: \"")
        .map(|(i, m)| {
            let rest = &source[i + m.len()..];
            rest[..rest.find('"').unwrap()].to_string()
        })
        .collect();
    assert_eq!(hosts, AGENT_HOSTS.iter().map(|s| s.to_string()).collect());
}

#[test]
fn item_urls_match_the_viewer_canonical_forms() {
    let source = viewer("dedup.js");
    let start = source.find("const ITEM_URLS = {").unwrap();
    let block = &source[start..start + source[start..].find("};").unwrap()];
    let prefixes: std::collections::BTreeMap<String, String> = block
        .lines()
        .filter_map(|line| line.trim().split_once(": \""))
        .map(|(key, value)| {
            (
                key.trim_matches('"').to_string(),
                value.trim_end_matches(['"', ',']).to_string(),
            )
        })
        .collect();
    assert_eq!(
        prefixes.keys().cloned().collect::<BTreeSet<_>>(),
        ITEM_URLS.iter().map(|(p, _)| p.to_string()).collect()
    );
    for (platform, template) in ITEM_URLS {
        let suffix = if platform == "1688" { ".html" } else { "" };
        assert_eq!(
            template.replace("{id}", "123"),
            format!("{}123{suffix}", prefixes[platform])
        );
    }
}

#[test]
fn control_and_invisible_characters_never_reach_a_url() {
    for text in [
        "taobao.com/item.htm?id=123\u{0}evil",
        "taobao.com/item.htm?id=123\u{200b}\u{200d}",
        "\u{feff}taobao.com/x.htm?id=5",
        "k.youshop10.com/abc\u{0}def",
        "m.tb.cn/h.Xyz\u{200b}12",
    ] {
        for link in extract_links(text) {
            assert_eq!(link.url, link.url.trim());
            assert!(
                link.url.chars().all(|c| c as u32 > 0x20
                    && !"\u{7f}\u{feff}\u{200b}\u{200c}\u{200d}\u{2060}".contains(c)),
                "{}",
                link.url
            );
        }
    }
}

#[test]
fn platform_uses_a_label_boundary() {
    for (host, expected) in [
        ("tmall.com", "tmall"),
        ("detail.tmall.com", "tmall"),
        ("taobao.com", "taobao"),
        ("item.taobao.com", "taobao"),
        ("weidian.com", "weidian"),
        ("shop123.v.weidian.com", "weidian"),
        ("m.tb.cn", "other"),
        ("k.youshop10.com", "other"),
        ("evil-tmall.com", "other"),
        ("not-taobao.com", "other"),
        ("prefix-weidian.com", "other"),
    ] {
        assert_eq!(platform(host), expected, "{host}");
    }
}

#[test]
fn suffix_lookalikes_never_collapse_onto_the_real_host() {
    for text in [
        "taobao.com.evil.example/item?id=5",
        "taobao.com.cn/item?id=5",
        "weidian.com.evil.example/item",
    ] {
        assert!(extract_links(text).is_empty(), "{text}");
    }
}

#[test]
fn valid_hosts_still_match() {
    for (text, host) in [
        ("taobao.com/item?id=5", "taobao.com"),
        ("item.taobao.com/x", "item.taobao.com"),
        ("buy from taobao.com. Then wait.", "taobao.com"),
    ] {
        let links = extract_links(text);
        assert!(!links.is_empty() && links[0].url.contains(host), "{text}");
    }
}

const WD: &str = "https://weidian.com/item.html?itemID=7652831688";
const TB: &str = "https://item.taobao.com/item.htm?id=560198996624";
const ALI: &str = "https://detail.1688.com/offer/691234567890.html";
const WD_ENC: &str = "https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D7652831688";

#[test]
fn agent_links_unwrap_to_the_marketplace_item() {
    let wd = || vec![l("weidian", WD)];
    let tb = || vec![l("taobao", TB)];
    let ali = || vec![l("1688", ALI)];
    let none = Vec::new;
    let cases: Vec<(String, Vec<Link>)> = vec![
        (format!("https://www.kakobuy.com/item/details?url={WD_ENC}&affcode=thelude"), wd()),
        ("https://hoobuy.com/product/1/560198996624".into(), tb()),
        ("https://hoobuy.com/product/0/691234567890".into(), ali()),
        ("https://hoobuy.com/product/2/7652831688".into(), wd()),
        ("https://oopbuy.com/goods/details?id=7652831688&channel=weidian".into(), wd()),
        ("https://oopbuy.com/goods/details?id=691234567890&channel=1688".into(), ali()),
        ("https://mulebuy.com/product/?shop_type=ali_1688&id=691234567890".into(), ali()),
        ("https://joyagoo.com/product/?shop_type=taobao&id=560198996624".into(), tb()),
        ("https://cnfans.com/product/?shop_type=weidian&id=7652831688".into(), wd()),
        ("https://www.cssbuy.com/item-micro-7652831688.html".into(), wd()),
        ("https://www.cssbuy.com/item-1688-691234567890.html".into(), ali()),
        ("https://www.cssbuy.com/item-560198996624.html".into(), tb()),
        ("https://www.sugargoo.com/productDetail?productLink=https%3A%2F%2Fdetail.1688.com%2Foffer%2F691234567890.html".into(), ali()),
        ("https://www.superbuy.com/en/page/buy/?url=https%3A%2F%2Fitem.taobao.com%2Fitem.htm%3Fid%3D560198996624".into(), tb()),
        (format!("https://www.loongbuy.com/product-details?url={WD_ENC}"), wd()),
        ("https://www.pikobuy.com/product/detail?productUrl=https%3A%2F%2Fdetail.1688.com%2Foffer%2F691234567890.html".into(), ali()),
        (format!("https://www.itaobuy.com/product-detail?url={WD_ENC}"), wd()),
        ("https://bbdbuy.com/index/item/index1688.html?tp=taobao&tid=&searchlang=en&url=https%3A%2F%2Fdetail.1688.com%2Foffer%2F691234567890.html".into(), ali()),
        ("https://www.ootdbuy.com/goods/details?id=560198996624&channel=TAOBAO".into(), tb()),
        ("https://www.lovegobuy.com/product?id=691234567890&shop_type=ali_1688".into(), ali()),
        ("https://cnshopper.com/goods/detail?keyword=7652831688&platform=weidian".into(), wd()),
        ("https://alleurobuy.com/index/detail-public?id=560198996624&type=tao".into(), tb()),
        ("https://alleurobuy.com/index/detail-public?id=691234567890&type=ali".into(), ali()),
        ("https://rizzitgo.com/detail-page?goodsId=7652831688&source=3".into(), wd()),
        ("https://rizzitgo.com/detail-page?goodsId=691234567890&source=2".into(), ali()),
        ("https://vigorbuy.com/product/1/560198996624".into(), tb()),
        ("https://usfans.com/product/1/691234567890".into(), ali()),
        ("https://usfans.com/product/3/7652831688".into(), wd()),
        ("https://www.litbuy.com/product/weidian/7652831688".into(), wd()),
        ("https://hipobuy.com/product/0/691234567890.html".into(), ali()),
        ("https://www.gtbuy.com/product/1/560198996624".into(), tb()),
        ("https://hoobuy.com/product/3/7652831688".into(), none()),
        ("https://usfans.com/product/0/691234567890".into(), none()),
        ("https://rizzitgo.com/detail-page?goodsId=5&source=weidian".into(), none()),
        ("https://www.litbuy.com/product/tmall/5".into(), none()),
        ("https://usfans.com/product/0/691234567890?keyword=691234567890&platform=1688".into(), none()),
        ("https://hoobuy.com/product/2/7652831688?id=5&channel=taobao".into(), wd()),
        ("https://hoobuy.com/product/2/7652831688?id=5&keyword=6".into(), wd()),
        ("https://www.cssbuy.com/item-micro-7652831688.html?id=1&id=2".into(), wd()),
        ("https://oopbuy.com/goods/details?id=7652831688&keyword=6&channel=weidian".into(), none()),
        (format!("https://www.kakobuy.com/item/details?url={WD_ENC}&id=1&id=2"), wd()),
        ("https://oopbuy.com/goods/details?keyword=7652831688&platform=weidian".into(), none()),
        ("https://cnshopper.com/goods/detail?id=7652831688&channel=weidian".into(), none()),
        ("https://www.kakobuy.com/item/details?id=7652831688&channel=weidian".into(), none()),
        (format!("https://oopbuy.com/goods/details?url={WD_ENC}"), none()),
        ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fhoobuy.com%2Fproduct%2F2%2F7652831688".into(), wd()),
        ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fwww.kakobuy.com%2Fitem%2Fdetails%3Furl%3Dhttps%253A%252F%252Fhoobuy.com%252Fproduct%252F2%252F7652831688".into(), none()),
        ("https://detail.1688.com/offer/691234567890.html?spm=a2615.7691456".into(), ali()),
        ("m.1688.com/offer/691234567890.html".into(), vec![l("1688", "https://m.1688.com/offer/691234567890.html")]),
        ("购买https://hoobuy.com/product/2/7652831688，尺码咨询".into(), wd()),
        ("https://www.kakobuy.com/item/details?url=https%3A%2F%2Fevil.example%2Fitem.html%3FitemID%3D1".into(), none()),
        ("https://hoobuy.com/product/9/123".into(), none()),
        ("https://oopbuy.com/goods/details?id=5&channel=unknown".into(), none()),
        ("https://oopbuy.com/goods/details?id=5&id=6&channel=weidian".into(), none()),
        ("https://mulebuy.com/product/?shop_type=weidian&channel=taobao&id=5".into(), none()),
        ("https://oopbuy.com/goods/details?id=abc&channel=weidian".into(), none()),
        (format!("https://notkakobuy.com/item/details?url={WD_ENC}"), none()),
        ("https://weidian.com@evil.example/item.html?itemID=123".into(), none()),
        ("https://user@weidian.com/item.html?itemID=123".into(), none()),
    ];
    for (text, expected) in cases {
        assert_eq!(extract_links(&text), expected, "{text}");
    }
}

#[test]
fn an_agent_link_and_the_direct_link_are_one_entry() {
    assert_eq!(
        extract_links(&format!("https://hoobuy.com/product/2/7652831688 or {WD}")),
        vec![l("weidian", WD)]
    );
}

#[test]
fn invisible_characters_end_a_url() {
    assert_eq!(
        extract_links("weidian.com/item.html?itemID=12\u{0}34"),
        vec![l("weidian", "https://weidian.com/item.html?itemID=12")]
    );
}

#[test]
fn item_link_builds_only_well_formed_items() {
    assert_eq!(
        item_link("taobao", "1").map(|l| l.url),
        Some(TB.replace("560198996624", "1"))
    );
    assert_eq!(
        item_link("1688", "691234567890").map(|l| l.url),
        Some(ALI.to_string())
    );
    assert_eq!(item_link("weidian", "x"), None);
    assert_eq!(item_link("nowhere", "1"), None);
}

#[test]
fn wrapped_urls_are_judged_whole_and_cleaned() {
    check(&[
        (
            "https://www.kakobuy.com/item/details?url=https%3A%2F%2Fevil.example%2F%3Fnext%3Dhttps%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D123",
            vec![],
        ),
        (
            "https://www.kakobuy.com/item/details?url=weidian.com%2Fitem.html%3FitemID%3D123",
            vec![l("weidian", "https://weidian.com/item.html?itemID=123")],
        ),
        (
            "https://www.kakobuy.com/item/details?url=https%3A%2F%2Fitem.taobao.com%2Fitem.htm%3Fid%3D5%23detail",
            vec![l("taobao", "https://item.taobao.com/item.htm?id=5")],
        ),
        (
            "https://www.kakobuy.com/item/details?url=https%3A%2F%2Fweidian.com%40evil.example%2Fitem.html%3FitemID%3D1",
            vec![],
        ),
        (
            "https://www.kakobuy.com/item/details?url=https%3A%2F%2Fweidian.com%2Fitem.html%3FitemID%3D12%0034",
            vec![l("weidian", "https://weidian.com/item.html?itemID=12")],
        ),
        (
            "https://www.kakobuy.com/item/details?url=https%3A%2F%2Fitem.taobao.com%2Fitem.htm%3Fid%3D5%29%E3%80%82",
            vec![l("taobao", "https://item.taobao.com/item.htm?id=5")],
        ),
        (
            "https://www.sugargoo.com/productDetail?productLink=https%3A%2F%2Fdetail.1688.com%2Foffer%2F7.html%E2%80%8B",
            vec![l("1688", "https://detail.1688.com/offer/7.html")],
        ),
    ]);
}
