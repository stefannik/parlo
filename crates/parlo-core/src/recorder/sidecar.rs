//! Recording sidecar JSON (`rec-YYYYMMDD-HHMMSS.json`): state, devices, pauses, gaps — written atomically.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecState {
    Recording,
    Paused,
    /// stopped normally by the user
    Finalized,
    /// stopped by the core to protect the data (disk nearly full, WAV size limit, permission revoked, I/O error)
    StoppedByError,
    /// found in `recording`/`paused` state after a crash and repaired
    Recovered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapReason {
    DeviceChanged,
    DeviceLost,
    StreamError,
    Stall,
    ResumedAfterSleep,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceSegment {
    pub name: String,
    pub since_frame: u64,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pause {
    pub start_frame: u64,
    pub end_frame: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Gap {
    /// position in the file (frames) where audio continues after the gap
    pub at_frame: u64,
    pub reason: GapReason,
    /// wall-clock length of the interruption (audio missing from the timeline)
    pub gap_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sidecar {
    pub version: u32,
    pub id: String,
    pub file: String,
    pub state: RecState,
    pub sample_rate: u32,
    pub started_at: String,
    pub devices: Vec<DeviceSegment>,
    pub pauses: Vec<Pause>,
    pub gaps: Vec<Gap>,
    pub frames_written: u64,
    pub duration_ms: u64,
    pub overruns: u64,
    pub stop_reason: Option<String>,
    /// set once a transcript was produced; recordings are never auto-deleted before that (BRIEF §8.5)
    pub transcribed: bool,
}

impl Sidecar {
    pub fn path_for(wav: &Path) -> PathBuf {
        wav.with_extension("json")
    }

    /// Atomic write (tmp + rename).
    pub fn write(&self, dir: &Path) -> Result<()> {
        let path = dir.join(format!("{}.json", self.id));
        let tmp = dir.join(format!("{}.json.tmp", self.id));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| Error::io(&path, e))?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<Self> {
        let b = std::fs::read(path).map_err(|e| Error::io(path, e))?;
        Ok(serde_json::from_slice(&b)?)
    }
}
