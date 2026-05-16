//! Golden-snapshot regression test.
//!
//! For each `fixtures/*.ndjson`, asserts the Snapshot produced by applying
//! every event to a fresh RenderModel matches `fixtures/golden/<name>.snapshot.json`.
//!
//! To regenerate goldens after an intentional renderer change:
//!     CAMOUFLAGE_UPDATE_GOLDENS=1 cargo test -p camouflage-renderer golden
//!
//! This is intentionally a *separate* gate from the fixture-validates
//! check in camouflage-headless: that only confirms NDJSON parses;
//! this confirms the rendered output is byte-identical to the expected
//! UI state.

use camouflage_renderer::golden::{snapshot_of_fixture_path, snapshot_to_golden_json};
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

fn list_fixture_paths() -> Vec<PathBuf> {
    let dir = fixtures_dir();
    let mut out: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixtures dir readable")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ndjson"))
        .collect();
    out.sort();
    out
}

fn golden_path_for(fixture: &Path) -> PathBuf {
    let stem = fixture.file_stem().unwrap();
    fixtures_dir()
        .join("golden")
        .join(format!("{}.snapshot.json", stem.to_string_lossy()))
}

#[test]
fn golden_snapshots_match() {
    let update = std::env::var("CAMOUFLAGE_UPDATE_GOLDENS").is_ok();
    let fixtures = list_fixture_paths();
    assert!(!fixtures.is_empty(), "no fixtures found");

    let mut mismatches: Vec<String> = Vec::new();

    for fixture in &fixtures {
        let snap = snapshot_of_fixture_path(fixture).expect("read fixture");
        let actual = snapshot_to_golden_json(&snap);
        let golden = golden_path_for(fixture);

        if update {
            std::fs::create_dir_all(golden.parent().unwrap()).expect("mkdir golden/");
            std::fs::write(&golden, &actual).expect("write golden");
            eprintln!("updated {}", golden.display());
            continue;
        }

        if !golden.exists() {
            mismatches.push(format!(
                "missing golden for {}: re-run with CAMOUFLAGE_UPDATE_GOLDENS=1 to create",
                fixture.display()
            ));
            continue;
        }
        let expected = std::fs::read_to_string(&golden).expect("read golden");
        if actual.trim() != expected.trim() {
            // Quick diff hint: first differing line.
            let first_diff = actual
                .lines()
                .zip(expected.lines())
                .enumerate()
                .find(|(_, (a, e))| a != e)
                .map(|(i, (a, e))| format!("line {}: actual={:?} expected={:?}", i + 1, a, e))
                .unwrap_or_else(|| "differ at trailing length".into());
            mismatches.push(format!(
                "golden mismatch for {}: {}",
                fixture.file_name().unwrap().to_string_lossy(),
                first_diff
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "golden snapshot regression ({} mismatch(es)). To accept, re-run with CAMOUFLAGE_UPDATE_GOLDENS=1.\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}
