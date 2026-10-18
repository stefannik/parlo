//! Where downloaded audio lives (`<data>/downloads/<id>/audio.<ext>` + `meta.json`), and the cleanup rules:
//! it is kept while it is the current transcription (player, re-transcribe and resume need it) and deleted when that
//! transcription is replaced or cleared; an orphan sweep at launch removes everything else (partial downloads included).

use super::Downloaded;
use crate::paths::AppPaths;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DownloadMeta {
    pub url: String,
    pub title: String,
    pub duration_s: Option<f64>,
    pub tool: String,
    pub bytes: u64,
    pub downloaded_at: String,
}

pub fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325u64, |h, x| {
        (h ^ *x as u64).wrapping_mul(0x100000001b3)
    })
}

/// Stable id of a download directory for a URL (`dl-<hash>`).
pub fn id_for_url(url: &str) -> String {
    format!("dl-{:016x}", fnv(url.as_bytes()))
}

/// Ids come from the UI: only our own shape is accepted, so they can never walk out of the downloads directory.
pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("dl-")
        .is_some_and(|h| h.len() == 16 && h.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn job_dir(paths: &AppPaths, id: &str) -> PathBuf {
    paths.downloads().join(id)
}

pub fn write_meta(job_dir: &Path, d: &Downloaded) {
    let meta = DownloadMeta {
        url: d.url.clone(),
        title: d.title.clone(),
        duration_s: d.duration_s,
        tool: d.tool.id().into(),
        bytes: d.bytes,
        downloaded_at: chrono::Local::now().to_rfc3339(),
    };
    let _ = crate::store::atomic_write_json(&job_dir.join("meta.json"), &meta);
}

pub fn read_meta(job_dir: &Path) -> Option<DownloadMeta> {
    serde_json::from_slice(&std::fs::read(job_dir.join("meta.json")).ok()?).ok()
}

/// The finished audio of a job directory.
pub fn find_media(job_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(job_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("audio.") && !n.ends_with(".part"))
        })
}

/// If `path` is the audio of a download job (`<downloads>/<id>/audio.*`), the job id.
pub fn id_of_media_path(paths: &AppPaths, path: &Path) -> Option<String> {
    let dir = path.parent()?;
    if dir.parent()? != paths.downloads() {
        return None;
    }
    let id = dir.file_name()?.to_str()?;
    valid_id(id).then(|| id.to_string())
}

pub fn remove_job(paths: &AppPaths, id: &str) {
    if valid_id(id) {
        let _ = std::fs::remove_dir_all(job_dir(paths, id));
    }
}

/// Delete every download directory not in `keep` (and any stray file), and the staging leftovers of the kept ones.
/// Returns the ids removed. Safe at launch: nothing is downloading yet.
pub fn sweep(paths: &AppPaths, keep: &HashSet<String>) -> Vec<String> {
    let mut removed = Vec::new();
    let Ok(rd) = std::fs::read_dir(paths.downloads()) else {
        return removed;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let kept = keep.contains(&name) && p.is_dir() && find_media(&p).is_some();
        if kept {
            for junk in [".dl", ".tmp"] {
                let _ = std::fs::remove_dir_all(p.join(junk));
            }
        } else {
            let _ = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
            removed.push(name);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(paths: &AppPaths, id: &str, with_media: bool) -> PathBuf {
        let d = job_dir(paths, id);
        std::fs::create_dir_all(d.join(".dl")).unwrap();
        std::fs::write(d.join(".dl/media.m4a.part"), b"half").unwrap();
        if with_media {
            std::fs::write(d.join("audio.m4a"), b"done").unwrap();
        }
        d
    }

    #[test]
    fn ids_are_stable_and_safe() {
        let a = id_for_url("https://a.com/x");
        assert_eq!(a, id_for_url("https://a.com/x"));
        assert_ne!(a, id_for_url("https://a.com/y"));
        assert!(valid_id(&a));
        for bad in [
            "",
            "dl-",
            "dl-../etc",
            "../dl-0123456789abcdef",
            "dl-0123456789abcdeg",
            "stt-0123456789abcdef",
            "dl-0123456789abcdef/x",
        ] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn sweep_keeps_only_referenced_finished_downloads() {
        let t = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(t.path());
        let (keep_id, partial_id, orphan_id) = (
            "dl-0000000000000001",
            "dl-0000000000000002",
            "dl-0000000000000003",
        );
        let kept = mk(&paths, keep_id, true);
        let partial = mk(&paths, partial_id, false);
        let orphan = mk(&paths, orphan_id, true);
        std::fs::write(paths.downloads().join("stray.txt"), b"x").unwrap();
        let keep: HashSet<String> = [keep_id, partial_id]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut removed = sweep(&paths, &keep);
        removed.sort();
        assert_eq!(
            removed,
            vec!["dl-0000000000000002", "dl-0000000000000003", "stray.txt"]
        );
        assert!(kept.join("audio.m4a").exists());
        assert!(
            !kept.join(".dl").exists(),
            "staging leftovers of a kept download go"
        );
        assert!(
            !partial.exists(),
            "a referenced download that never finished is thrown away"
        );
        assert!(!orphan.exists());
    }

    #[test]
    fn media_path_to_id() {
        let t = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(t.path());
        let id = id_for_url("https://a.com/x");
        let d = mk(&paths, &id, true);
        assert_eq!(
            id_of_media_path(&paths, &d.join("audio.m4a")),
            Some(id.clone())
        );
        assert_eq!(
            id_of_media_path(&paths, &t.path().join("elsewhere/audio.m4a")),
            None
        );
        assert_eq!(find_media(&d), Some(d.join("audio.m4a")));
        remove_job(&paths, &id);
        assert!(!d.exists());
        remove_job(&paths, "../..");
    }
}
