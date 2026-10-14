#![allow(dead_code)]
use std::path::{Path, PathBuf};

/// Absolute path to `fixtures/<rel>` at the repo root. Fails loudly (no silent skip) if the fixture is missing.
pub fn fixture(rel: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel);
    assert!(
        p.exists(),
        "fixture {} is missing. Generate fixtures first: scripts/gen-fixtures.sh",
        p.display()
    );
    p
}

/// Absolute path to `models/<rel>` at the repo root; fails loudly if missing (fetch with scripts/fetch-models.sh).
pub fn model(rel: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../models")
        .join(rel);
    assert!(
        p.exists(),
        "model {} is missing. Fetch it: scripts/fetch-models.sh",
        p.display()
    );
    p
}
