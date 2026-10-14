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

/// Peak resident set size of this process in MB (portable: `getrusage`; Linux reports KB, macOS bytes).
pub fn peak_rss_mb() -> f64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let v = ru.ru_maxrss as f64;
    if cfg!(target_os = "macos") {
        v / (1024.0 * 1024.0)
    } else {
        v / 1024.0
    }
}

/// Current resident set size in MB (portable via `ps`).
pub fn current_rss_mb() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .unwrap_or(0.0)
        / 1024.0
}
