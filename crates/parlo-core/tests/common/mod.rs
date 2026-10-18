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
/// Length of `synth/en_all.wav`, the source of the `url/` fixtures. It depends on the TTS that generated it (espeak-ng on
/// Linux, `say` on macOS), so tests compare against it instead of a fixed number.
pub fn url_source_secs() -> f64 {
    let r = hound::WavReader::open(fixture("generated/synth/en_all.wav")).expect("readable WAV");
    r.duration() as f64 / r.spec().sample_rate as f64
}

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

pub fn norm_words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .replace(['’', '\''], "")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Word error rate (Levenshtein over words) of `hyp` against `reference`.
pub fn wer(reference: &str, hyp: &str) -> f64 {
    let (r, h) = (norm_words(reference), norm_words(hyp));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut cur = vec![i; h.len() + 1];
        for j in 1..=h.len() {
            let sub = prev[j - 1] + usize::from(r[i - 1] != h[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len().max(1) as f64
}

// ---------------------------------------------------------------------------------------------- URL download helpers
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Copy `scripts/fake-yt-dlp.py` into `dir/<name>/yt-dlp` (so its flag files sit next to it) and return the script path.
pub fn fake_ytdlp(dir: &Path, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let d = dir.join(name);
    std::fs::create_dir_all(&d).unwrap();
    let dst = d.join("yt-dlp");
    std::fs::copy(repo_root().join("scripts/fake-yt-dlp.py"), &dst).unwrap();
    std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        d.join("audio"),
        fixture("generated/url/episode.m4a")
            .to_string_lossy()
            .as_bytes(),
    )
    .unwrap();
    dst
}

/// Every argv the fake was called with (one Vec per invocation).
pub fn fake_calls(script: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(script.with_file_name("argv.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// `scripts/fixture-server.py` serving `dir` on a free localhost port; killed on drop.
pub struct FixtureServer {
    child: std::process::Child,
    pub port: u16,
}

impl FixtureServer {
    pub fn start(dir: &Path, extra: &[&str]) -> Self {
        use std::io::{BufRead, BufReader};
        let mut child = std::process::Command::new("python3")
            .arg("-I")
            .arg(repo_root().join("scripts/fixture-server.py"))
            .arg(dir)
            .args(extra)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("python3 (needed for scripts/fixture-server.py)");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let port = line
            .trim()
            .strip_prefix("PORT ")
            .unwrap_or_else(|| panic!("fixture server said {line:?}"))
            .parse()
            .unwrap();
        Self { child, port }
    }
    pub fn url(&self, path: &str) -> String {
        format!(
            "http://127.0.0.1:{}/{}",
            self.port,
            path.trim_start_matches('/')
        )
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
