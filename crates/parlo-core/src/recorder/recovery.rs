//! Crash recovery (BRIEF §4.4, §8.1): find recordings whose sidecar still says `recording`/`paused`, repair the WAV
//! header from the file length, and mark them `recovered` so the UI can offer Transcribe / Save audio… / Discard.

use super::sidecar::{RecState, Sidecar};
use crate::audio::wav::repair_wav;
use crate::error::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Recoverable {
    pub id: String,
    pub wav: PathBuf,
    pub sidecar: Sidecar,
}

/// Scan `dir` for interrupted recordings and repair them. Returns every recording in state `Recovered` that is
/// not yet transcribed (including ones repaired on a previous launch and still pending).
pub fn recover_all(dir: &Path) -> Result<Vec<Recoverable>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(crate::error::Error::io(dir, e)),
    };
    let mut jsons: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    jsons.sort();
    for j in jsons {
        let Ok(mut sc) = Sidecar::read(&j) else {
            continue;
        };
        let wav = dir.join(&sc.file);
        match sc.state {
            RecState::Recording | RecState::Paused => {
                if !wav.exists() {
                    continue;
                }
                match repair_wav(&wav) {
                    Ok(info) => {
                        sc.state = RecState::Recovered;
                        sc.frames_written = info.frames;
                        sc.duration_ms = info.frames * 1000 / info.sample_rate as u64;
                        sc.sample_rate = info.sample_rate;
                        if let Some(p) = sc.pauses.last_mut() {
                            if p.end_frame.is_none() {
                                p.end_frame = Some(info.frames);
                            }
                        }
                        sc.stop_reason =
                            Some("interrupted (app crash or power loss); recovered".into());
                        sc.write(dir)?;
                        out.push(Recoverable {
                            id: sc.id.clone(),
                            wav,
                            sidecar: sc,
                        });
                    }
                    Err(e) => tracing::warn!("could not repair {}: {e}", wav.display()),
                }
            }
            RecState::Recovered if !sc.transcribed && wav.exists() => out.push(Recoverable {
                id: sc.id.clone(),
                wav,
                sidecar: sc,
            }),
            _ => {}
        }
    }
    Ok(out)
}
