//! Ported from tests/test_parse.py.
mod common;

use common::fixture;
use lamp::parse::{parse_album_list, parse_album_page, parse_price, price_evidence};
use lamp::pyjson::Value;
use std::time::{Duration, Instant};

#[test]
fn parse_album_list_first_template() {
    let stubs = parse_album_list(&fixture("album_list.html"));
    assert_eq!(stubs.len(), 3);
    assert_eq!(stubs[0].id, "98765432");
    assert_eq!(stubs[0].title, "Item A weidian.com/item.html?itemID=111");
    assert_eq!(
        stubs[0].thumb_url,
        "https://photo.yupoo.com/sellerx/abcd1234/small.jpg"
    );
    assert_eq!(stubs[0].photo_count, 23);
    assert_eq!(stubs[1].photo_count, 0);
    assert_eq!(stubs[2].id, "98765434");
    assert_eq!(
        stubs[2].thumb_url,
        "https://photo.yupoo.com/sellerx/ijkl9012/medium.jpg"
    );
    assert_eq!(stubs[2].photo_count, 0);
}

#[test]
fn parse_album_list_empty() {
    assert!(parse_album_list(&fixture("album_list_empty.html")).is_empty());
}

#[test]
fn parse_album_page_fixture() {
    let detail = parse_album_page(&fixture("album_page.html"));
    assert_eq!(detail.title, "Item A");
    assert!(
        detail
            .description
            .contains("item.taobao.com/item.htm?id=222")
    );
    assert_eq!(detail.category, "hoodies");
    assert_eq!(detail.photo_count, 23);
    assert_eq!(detail.date_published, "2025-11-24");
    assert_eq!(detail.date_modified, "2026-08-16");
}

#[test]
fn parse_album_page_missing_everything() {
    let detail = parse_album_page("<html><body></body></html>");
    assert_eq!(
        (
            detail.title.as_str(),
            detail.description.as_str(),
            detail.category.as_str(),
            detail.photo_count
        ),
        ("", "", "", 0)
    );
    assert_eq!(
        (
            detail.date_published.as_str(),
            detail.date_modified.as_str()
        ),
        ("", "")
    );
}

/// Scraped pages reach 8 MiB, so nesting depth is attacker-chosen: tree walks
/// must not recurse once per level.
const DEEP: usize = 100_000;

#[test]
fn deeply_nested_album_list_does_not_overflow_the_stack() {
    let html = format!(
        "{}<a class=\"album__main\" href=\"/albums/42\"><span class=\"album__title\">{}Deep{}</span></a>{}",
        "<div>".repeat(DEEP),
        "<b>".repeat(DEEP),
        "</b>".repeat(DEEP),
        "</div>".repeat(DEEP),
    );
    let stubs = parse_album_list(&html);
    assert_eq!(stubs.len(), 1);
    assert_eq!(
        (stubs[0].id.as_str(), stubs[0].title.as_str()),
        ("42", "Deep")
    );
}

#[test]
fn deeply_nested_album_page_does_not_overflow_the_stack() {
    let html = format!(
        "{}<h1 class=\"showalbumheader__gallerytitle\">{}Deep title{}</h1>",
        "<div>".repeat(DEEP),
        "<span>".repeat(DEEP),
        "</span>".repeat(DEEP),
    );
    assert_eq!(parse_album_page(&html).title, "Deep title");
}

/// An unmatched closing tag must not rescan every open tag: on an 8 MiB page
/// that is about 10^12 comparisons. The bound is generous; the quadratic
/// version took over 400 times as long as the matched page here.
#[test]
fn unmatched_closing_tags_at_depth_take_linear_time() {
    const N: usize = 40_000;
    let title = "<h1 class=\"showalbumheader__gallerytitle\">Deep title</h1>";
    let timed = |closers: &str| {
        let html = format!("{}{}{title}", "<div>".repeat(N), closers.repeat(N));
        let started = Instant::now();
        let detail = parse_album_page(&html);
        (started.elapsed(), detail.title)
    };
    let (matched, matched_title) = timed("</div>");
    let (unmatched, unmatched_title) = timed("</span>");
    assert_eq!(
        (matched_title.as_str(), unmatched_title.as_str()),
        ("Deep title", "Deep title")
    );
    assert!(
        unmatched < matched * 10 + Duration::from_secs(1),
        "unmatched {unmatched:?}, matched {matched:?}"
    );
}

#[test]
fn parse_album_page_malformed_ld_json() {
    let html = "<html><body><script type=\"application/ld+json\">not json {</script>\
        <script type=\"application/ld+json\">{\"datePublished\": \"garbage\"}</script>\
        <script type=\"application/ld+json\">{\"dateModified\": \"2026-01-02\"}</script></body></html>";
    let detail = parse_album_page(html);
    assert_eq!(detail.date_published, "");
    assert_eq!(detail.date_modified, "2026-01-02");
}

#[test]
fn prices() {
    for (text, amount) in [
        ("Nike Tech Fleece ¥180 Black", 180),
        ("Stussy Tee ￥150 White", 150),
        ("Bape Hoodie 220元 M-XXL", 220),
        ("Balenciaga Shoes P350 40-45", 350),
        ("Prada Jacket 420P New", 420),
        ("Arcteryx 380 RMB", 380),
        ("Plain Title No Price", 0),
        ("Size 180cm height", 0),
        ("Micro price 5", 0),
        ("Nike Tech Fleece ¥180.50 Black", 181),
        ("180.50 yuan", 181),
        ("Jacket ¥1,280", 1280),
        ("¥180黑色", 180),
        ("黑色180元新款", 180),
        ("¥5 ¥180", 180),
        ("¥8.8 ¥180", 180),
        ("¥100000 then ¥180", 180),
    ] {
        assert_eq!(parse_price(text), amount, "{text}");
    }
}

#[test]
fn ld_json_array_and_graph_forms() {
    let array = "<html><body><script type=\"application/ld+json\">[{\"@type\": \"WebPage\"}, {\"datePublished\": \"2025-03-04\"}]</script></body></html>";
    assert_eq!(parse_album_page(array).date_published, "2025-03-04");
    let graph = "<html><body><script type=\"application/ld+json\">{\"@context\": \"x\", \"@graph\": [{\"@type\": \"ImageGallery\", \"datePublished\": \"2025-05-06\", \"dateModified\": \"2026-05-06\"}]}</script></body></html>";
    let detail = parse_album_page(graph);
    assert_eq!(
        (
            detail.date_published.as_str(),
            detail.date_modified.as_str()
        ),
        ("2025-05-06", "2026-05-06")
    );
}

#[test]
fn dates_collected_across_blocks() {
    let html = "<html><body><script type=\"application/ld+json\">{\"dateModified\": \"2026-02-02\"}</script>\
        <script type=\"application/ld+json\">{\"datePublished\": \"2024-09-09\"}</script></body></html>";
    let detail = parse_album_page(html);
    assert_eq!(
        (
            detail.date_published.as_str(),
            detail.date_modified.as_str()
        ),
        ("2024-09-09", "2026-02-02")
    );
}

#[test]
fn second_template() {
    let stubs = parse_album_list(&fixture("album_list_v2.html"));
    assert_eq!(
        stubs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["246541366", "246541300"]
    );
    assert_eq!(
        stubs[0].title,
        "Item One https://weidian.com/item.html?itemID=11"
    );
    assert_eq!(
        stubs[0].thumb_url,
        "https://photo.yupoo.com/sellerv2/902e9b3f5c/medium.jpeg"
    );
    assert_eq!(stubs[0].photo_count, 7);
    assert_eq!(
        stubs[1].thumb_url,
        "https://photo.yupoo.com/sellerv2/abc123/medium.jpeg"
    );
    assert_eq!(stubs[1].photo_count, 12);
}

#[test]
fn never_returns_a_data_uri_thumb() {
    let html = "<a class=\"album3__main\" href=\"/albums/5\" title=\"x\"><img class=\"album__img\" src=\"data:image/png;base64,AAAA\"></a>";
    assert_eq!(parse_album_list(html)[0].thumb_url, "");
}

#[test]
fn yupoo_no_cover_placeholder_is_not_a_thumb() {
    let html = "<a class=\"album3__main\" href=\"/albums/7\" title=\"No cover\"><img class=\"album__img\" data-origin-src=\"https://alfred-internal-1.cos.example.com/yupoo/album/im_photo_album.png\"></a>";
    assert_eq!(parse_album_list(html)[0].thumb_url, "");
    let real = html.replace("im_photo_album.png", "medium.jpeg");
    assert!(
        parse_album_list(&real)[0]
            .thumb_url
            .ends_with("medium.jpeg")
    );
}

#[test]
fn placeholder_does_not_hide_a_real_cover_later() {
    let html = "<a class=\"album3__main\" href=\"/albums/8\" title=\"x\"><img class=\"album__img\" data-origin-src=\"https://cdn.example.com/yupoo/album/im_photo_album.png\"><img class=\"album3__img\" data-src=\"https://photo.example.com/s/real.jpg\"></a>";
    assert_eq!(
        parse_album_list(html)[0].thumb_url,
        "https://photo.example.com/s/real.jpg"
    );
}

#[test]
fn cover_image_wins_over_an_unrelated_image() {
    let html = "<a class=\"album3__main\" href=\"/albums/9\" title=\"x\"><img class=\"seller-badge\" src=\"https://cdn.example.com/badge.png\"><img class=\"album__img\" data-origin-src=\"https://photo.example.com/s/cover.jpg\"></a>";
    assert_eq!(
        parse_album_list(html)[0].thumb_url,
        "https://photo.example.com/s/cover.jpg"
    );
}

#[test]
fn placeholder_match_is_exact() {
    let html = "<a class=\"album__main\" href=\"/albums/10\" title=\"x\"><img class=\"album__img\" src=\"https://photo.example.com/s/my_im_photo_album.png\"></a>";
    assert!(
        parse_album_list(html)[0]
            .thumb_url
            .ends_with("my_im_photo_album.png")
    );
}

#[test]
fn price_evidence_keeps_rule_and_candidates() {
    let evidence = price_evidence("Offer ¥180, previous ¥220");
    assert!(matches!(evidence.get("amount"), Some(Value::Int(180))));
    assert_eq!(evidence.get("token").and_then(Value::as_str), Some("¥180"));
    assert_eq!(
        evidence.get("rule").and_then(Value::as_str),
        Some("currency")
    );
    let candidates: Vec<String> = evidence["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::py_str)
        .collect();
    assert_eq!(candidates, ["180", "220"]);
    assert!(price_evidence("No price supplied").is_empty());
}

/// Canary against real markup: `cargo test -- --ignored`.
///
/// The sample has to be saved by hand, and nothing in the repo does it for
/// you: a scrape writes data/debug/<seller>-page1.html only for a page it
/// could not parse, which is the one page this canary must never be pointed
/// at. Save a page that works, sending the project's own User-Agent:
///
///     mkdir -p data/debug
///     curl -A "DiogenesLamp/0.1 (+https://github.com/EquityBot/DiogenesLamp)" \
///       "https://<seller>.x.yupoo.com/albums?page=1" -o data/debug/sample-list.html
#[test]
#[ignore = "manual canary against a saved live sample"]
fn live_album_list_parses() {
    let sample = common::root()
        .join("data")
        .join("debug")
        .join("sample-list.html");
    assert!(
        sample.exists(),
        "save a live list page to {} first - see this test's comment",
        sample.display()
    );
    let bytes = std::fs::read(&sample).unwrap();
    let stubs = parse_album_list(&String::from_utf8_lossy(&bytes));
    assert!(
        !stubs.is_empty(),
        "selectors no longer match live Yupoo markup"
    );
    assert!(
        stubs
            .iter()
            .all(|s| !s.id.is_empty() && !s.thumb_url.is_empty())
    );
}
