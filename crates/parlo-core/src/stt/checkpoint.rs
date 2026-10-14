//! Job checkpoints (BRIEF §7.2 step 7): `jobs/<id>/job.json` + append-only `segments.jsonl`, one line per finished chunk.

use super::assemble::Seg;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobMeta {
    pub version: u32,
    /// identity of the input (path + size + mtime, or recording id); a different identity invalidates the checkpoint
    pub input_key: String,
    pub model: String,
    pub language_setting: String,
    /// language locked after detection (so a resumed job doesn't re-detect differently)
    pub locked_language: Option<String>,
    pub language_prob: Option<f32>,
    pub total_chunks: usize,
    pub duration_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub index: usize,
    pub start_s: f64,
    pub end_s: f64,
    /// true if the chunk was skipped by the energy gate (kept so resume knows it is done)
    pub skipped: bool,
    pub segments: Vec<Seg>,
}

pub struct Checkpoint {
    dir: PathBuf,
}

impl Checkpoint {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    fn meta_path(&self) -> PathBuf {
        self.dir.join("job.json")
    }
    fn seg_path(&self) -> PathBuf {
        self.dir.join("segments.jsonl")
    }

    pub fn load_meta(&self) -> Option<JobMeta> {
        serde_json::from_slice(&std::fs::read(self.meta_path()).ok()?).ok()
    }

    pub fn write_meta(&self, m: &JobMeta) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| Error::io(&self.dir, e))?;
        let tmp = self.dir.join("job.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(m)?).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, self.meta_path()).map_err(|e| Error::io(self.meta_path(), e))
    }

    /// Load completed chunks (a torn last line from a crash is ignored).
    pub fn load_chunks(&self) -> Vec<ChunkRecord> {
        let Ok(text) = std::fs::read_to_string(self.seg_path()) else {
            return vec![];
        };
        text.lines()
            .filter_map(|l| serde_json::from_str::<ChunkRecord>(l).ok())
            .collect()
    }

    pub fn append(&self, rec: &ChunkRecord) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| Error::io(&self.dir, e))?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.seg_path())
            .map_err(|e| Error::io(self.seg_path(), e))?;
        let mut line = serde_json::to_vec(rec)?;
        line.push(b'\n');
        f.write_all(&line)
            .map_err(|e| Error::io(self.seg_path(), e))?;
        f.sync_data().map_err(|e| Error::io(self.seg_path(), e))
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
