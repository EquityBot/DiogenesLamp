//! Arbitrary bytes, decoded lossily as UTF-8, to the HTML parser and the
//! selector and text walks the catalogue parsers run on its tree.
//!
//! On Linux with a nightly toolchain and cargo-fuzz:
//! `cargo +nightly fuzz run html_parse -- -max_total_time=300`
#![no_main]

const SELECTORS: [&str; 5] = [
    "a.album__main, a.album3__main",
    ".album__title",
    "img.album__img",
    "script[type=\"application/ld+json\"]",
    ".showalbumheader__gallerytitle",
];

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let doc = lamp::html::parse(&html);
    let root = doc.root();
    for selector in SELECTORS {
        let found = doc.select(root, selector);
        // Node ids follow creation order, which is document order.
        assert!(
            found.windows(2).all(|pair| pair[0] < pair[1]),
            "select left document order"
        );
        for id in found {
            doc.get_text(id, "\n", true);
        }
    }
    doc.get_text(root, "", false);
});
