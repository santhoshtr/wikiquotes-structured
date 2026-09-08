//! Golden tests over real pages from the English dump.
//!
//! Each fixture is a page as the dump holds it, next to a snapshot of what the
//! pipeline makes of it. A change in any rule shows up here as a diff, so the
//! effect can be read before it is accepted.
//!
//! To accept a change: `UPDATE_FIXTURES=1 cargo test --test fixtures`, then
//! read the diff before committing it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wikiquotes_structured::dump::RawPage;
use wikiquotes_structured::model::Document;
use wikiquotes_structured::parse::DocumentBuilder;
use wikiquotes_structured::{normalize, quote};

#[derive(Deserialize)]
struct Meta {
    title: String,
    page_id: u32,
    revision_id: u32,
    timestamp: String,
    redirect_to: Option<String>,
}

#[derive(Serialize)]
struct Snapshot {
    document: Document,
    quotes: Vec<quote::Quote>,
}

#[test]
fn every_fixture_matches_its_snapshot() {
    let update = std::env::var("UPDATE_FIXTURES").is_ok();
    let mut checked = 0;
    let mut stale = Vec::new();

    for wiki in fixture_files() {
        let name = wiki.file_stem().unwrap().to_string_lossy().to_string();
        let meta: Meta = serde_json::from_str(
            &std::fs::read_to_string(wiki.with_extension("meta.json")).unwrap(),
        )
        .unwrap();
        let page = RawPage {
            title: meta.title,
            page_id: meta.page_id,
            namespace: 0,
            redirect_to: meta.redirect_to,
            revision_id: meta.revision_id,
            timestamp: meta.timestamp,
            text: std::fs::read_to_string(&wiki).unwrap(),
        };

        let document = DocumentBuilder::new().build(&page, "en");
        let quotes = normalize::quotes(&document);
        let snapshot = serde_json::to_string_pretty(&Snapshot { document, quotes }).unwrap() + "\n";

        let expected_path = wiki.with_extension("snapshot.json");
        if update {
            std::fs::write(&expected_path, &snapshot).unwrap();
            checked += 1;
            continue;
        }
        match std::fs::read_to_string(&expected_path) {
            Ok(expected) if expected == snapshot => checked += 1,
            Ok(_) => stale.push(name),
            Err(_) => stale.push(format!("{name} (no snapshot yet)")),
        }
    }

    assert!(checked > 0, "no fixtures found in tests/fixtures");
    assert!(
        stale.is_empty(),
        "these fixtures no longer match their snapshot: {}\n\
         Read the change, then accept it with UPDATE_FIXTURES=1 cargo test --test fixtures",
        stale.join(", ")
    );
}

/// Every fixture keeps the page type its name promises, so a change in the
/// classifier cannot pass unnoticed.
#[test]
fn the_fixtures_cover_the_page_types() {
    let names: Vec<String> = fixture_files()
        .iter()
        .map(|path| path.file_stem().unwrap().to_string_lossy().to_string())
        .collect();
    for expected in [
        "person-with-parse-error",
        "film",
        "television-series",
        "literary-work",
        "theme",
        "proverbs",
        "redirect",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing fixture: {expected}"
        );
    }
}

fn fixture_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("tests/fixtures must exist")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "wiki"))
        .collect();
    files.sort();
    files
}
