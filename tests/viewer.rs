//! Ported from tests/test_viewer_dedup.py (the node suite runner) and
//! tests/test_viewer_source.py (source-level viewer contracts and file hygiene).
mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use regex::Regex;

/// The viewer's logic modules have node tests; they must run, never skip.
#[test]
fn viewer_js_unit_tests_pass_under_node() {
    let node = common::node().expect("node is required: the viewer suite must run, not skip");
    let dir = common::root().join("tests").join("viewer");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".test.cjs"))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no viewer JS test files found - glob broken?"
    );
    let output = Command::new(node)
        .arg("--test")
        .args(&files)
        .current_dir(common::root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const TEXT_SUFFIXES: [&str; 13] = [
    ".rs", ".js", ".cjs", ".json", ".toml", ".md", ".css", ".html", ".yml", ".yaml", ".lock",
    ".ts", ".py",
];

/// Hand-edited text files, as git tracks them.
fn text_sources() -> Vec<PathBuf> {
    let root = common::root();
    let listed = Command::new("git")
        .args(["-C", &root.to_string_lossy(), "ls-files", "-z"])
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    String::from_utf8_lossy(&listed.stdout)
        .split('\0')
        .filter(|n| !n.is_empty() && TEXT_SUFFIXES.iter().any(|s| n.ends_with(s)))
        .map(|n| root.join(n))
        .filter(|p| p.is_file())
        .collect()
}

#[test]
fn source_files_carry_no_nul_bytes() {
    let sources = text_sources();
    assert!(
        !sources.is_empty(),
        "found no source files to check - has the layout moved?"
    );
    for path in sources {
        assert!(
            !std::fs::read(&path).unwrap().contains(&0),
            "{} holds a NUL byte",
            path.display()
        );
    }
}

fn app_js() -> String {
    common::read(&common::root().join("viewer/app.js"))
}

#[test]
fn app_js_reaches_storage_only_through_the_guarded_accessors() {
    let source = app_js();
    let raw: Vec<&str> = Regex::new(r"localStorage\.\w+")
        .unwrap()
        .find_iter(&source)
        .map(|m| m.as_str())
        .collect();
    assert!(
        raw.is_empty(),
        "unguarded storage access in app.js: {raw:?}"
    );
    let calls: Vec<(String, String)> =
        Regex::new(r"\b(readStored|writeStored|readFilters|writeFilters)\(([^,)]*)")
            .unwrap()
            .captures_iter(&source)
            .map(|c| (c[1].to_string(), c[2].to_string()))
            .collect();
    assert!(
        !calls.is_empty(),
        "app.js no longer routes storage through the guarded accessors"
    );
    for (name, first) in calls {
        assert_eq!(
            first.trim(),
            "window",
            "{name}() is handed {first:?}; pass the host instead"
        );
    }
}

#[test]
fn every_focusable_control_on_a_card_carries_the_roving_tabindex() {
    let source = app_js();
    let selector = Regex::new(r#"CARD_STOPS = "([^"]+)""#)
        .unwrap()
        .captures(&source)
        .expect("app.js no longer declares CARD_STOPS");
    let stops: BTreeSet<String> = selector[1]
        .split(',')
        .map(|p| p.trim().to_string())
        .collect();
    let built: BTreeSet<String> = Regex::new(r#"<(?:a|button) class="([a-z0-9-]+)""#)
        .unwrap()
        .captures_iter(&source)
        .map(|c| c[1].to_string())
        .collect();
    assert!(
        !built.is_empty(),
        "found no statically classed card controls - has makeCard changed?"
    );
    for class in built {
        assert!(
            stops.contains(&format!(".{class}")),
            ".{class} is focusable but not a roving stop"
        );
    }
    assert!(stops.contains(".card-links a"));
}

#[test]
fn the_roving_tab_stop_follows_the_cursor_wherever_it_moves() {
    let source = app_js();
    let setter = Regex::new(r"(?s)\nfunction setCursor\(next\) \{\n(.*?)\n\}\n")
        .unwrap()
        .captures(&source)
        .expect("app.js no longer declares setCursor");
    assert!(
        setter[1].contains("setCardTabStop("),
        "setCursor moves the cursor without moving the tab stop"
    );
    let focusin = Regex::new(r#"(?s)addEventListener\("focusin".*?\n\}\);"#)
        .unwrap()
        .find(&source)
        .expect("app.js no longer binds focusin on the grid");
    assert!(
        focusin.as_str().contains("setCursor("),
        "focusin moves the cursor without moving the tab stop"
    );
    assert!(
        source.contains("cursor: state.view.cursor"),
        "the render key omits the cursor"
    );
}

#[test]
fn the_render_loop_refreshes_the_title_of_every_card_it_reuses() {
    let source = app_js();
    let render = Regex::new(r"(?s)\nfunction renderGrid\(force = false\) \{\n(.*?)\n\}\n")
        .unwrap()
        .captures(&source)
        .expect("app.js no longer declares renderGrid");
    let reuse = Regex::new(r"(?s)let card = cardCache\.get\(key\);\n(.*?)\n    if \(!card\) \{")
        .unwrap()
        .captures(&render[1])
        .expect("renderGrid no longer has a card reuse branch");
    assert!(
        reuse[1].contains("refreshCardTitle("),
        "renderGrid reuses a card without refreshing its title"
    );
    let maker = Regex::new(r"(?s)\nfunction makeCard\(album\) \{\n(.*?)\n\}\n")
        .unwrap()
        .captures(&source)
        .expect("app.js no longer declares makeCard");
    assert!(
        maker[1].contains("refreshCardTitle("),
        "makeCard fills the title without stamping the query it used"
    );
}
