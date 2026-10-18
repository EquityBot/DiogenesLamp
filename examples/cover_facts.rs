//! Print `cover_facts` for each path argument as one JSON line (parity checks).
fn main() {
    for path in std::env::args().skip(1) {
        let facts = lamp::covers::cover_facts(std::path::Path::new(&path));
        println!(
            "{}",
            lamp::pyjson::dumps(&facts.into(), lamp::pyjson::Style::Compact)
        );
    }
}
