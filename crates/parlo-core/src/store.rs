//! Persistent state (BRIEF §5.5): `settings.json`, `workspace.json` (atomic writes) and the retention/cleanup policy.

use crate::error::{Error, Result};
use crate::paths::AppPaths;
use crate::platform::DeviceSelector;
use crate::recorder::sidecar::{RecState, Sidecar};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Performance {
    Balanced,
    Maximum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    Day1,
    Days7,
    Days30,
    Forever,
}

impl Retention {
    pub fn duration(self) -> Option<Duration> {
        match self {
            Retention::Day1 => Some(Duration::from_secs(86_400)),
            Retention::Days7 => Some(Duration::from_secs(7 * 86_400)),
            Retention::Days30 => Some(Duration::from_secs(30 * 86_400)),
            Retention::Forever => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleUnload {
    Immediately,
    Secs60,
    Mins5,
    Mins15,
}

impl IdleUnload {
    pub fn to_timeout(self) -> crate::models::IdleTimeout {
        use crate::models::IdleTimeout::*;
        match self {
            IdleUnload::Immediately => Immediately,
            IdleUnload::Secs60 => After(Duration::from_secs(60)),
            IdleUnload::Mins5 => After(Duration::from_secs(300)),
            IdleUnload::Mins15 => After(Duration::from_secs(900)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub appearance: Appearance,
    pub model_idle_timeout: IdleUnload,
    pub performance: Performance,
    /// `None` = follow the system default input
    pub input_device: Option<String>,
    pub recording_retention: Retention,
    /// "auto" or an ISO-639-1 code
    pub default_language: String,
    pub default_voice: String,
    pub default_speed: f32,
    /// "standard" | "fast"
    pub tts_quality: String,
    // --- Downloads ("Transcribe from URL", DECISIONS C-021). The only network use in Parlo, and only when the user starts it.
    /// master switch: off hides the URL card and makes the download commands refuse
    pub allow_downloads: bool,
    /// override for the system yt-dlp (file or directory); `None` = search /opt/homebrew/bin, /usr/local/bin, ~/.local/bin
    pub downloader_path: Option<String>,
    /// let yt-dlp read the signed-in cookies of `cookies_browser` (private / age-restricted videos); read locally, sent
    /// only to the site being downloaded from
    pub use_browser_cookies: bool,
    /// yt-dlp browser name (chrome, firefox, safari, …)
    pub cookies_browser: Option<String>,
    /// `--impersonate chrome` when the downloader supports it
    pub impersonate: bool,
    /// `--sleep-interval 5 --max-sleep-interval 15` (be polite to the site)
    pub rate_limit_sleep: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            appearance: Appearance::System,
            model_idle_timeout: IdleUnload::Secs60,
            performance: Performance::Balanced,
            input_device: None,
            recording_retention: Retention::Days7,
            default_language: "auto".into(),
            default_voice: "george".into(),
            default_speed: 1.0,
            tts_quality: "standard".into(),
            allow_downloads: true,
            downloader_path: None,
            use_browser_cookies: false,
            cookies_browser: None,
            impersonate: true,
            rate_limit_sleep: true,
        }
    }
}

impl Settings {
    pub fn device_selector(&self) -> DeviceSelector {
        match &self.input_device {
            Some(d) => DeviceSelector::Named(d.clone()),
            None => DeviceSelector::SystemDefault,
        }
    }

    /// The download knobs for [`crate::download`]. Cookies are only passed when switched on **and** a browser is chosen.
    pub fn download_options(&self) -> crate::download::DownloadOptions {
        crate::download::DownloadOptions {
            impersonate: self.impersonate,
            cookies_browser: self
                .use_browser_cookies
                .then(|| self.cookies_browser.clone())
                .flatten(),
            rate_limit_sleep: self.rate_limit_sleep,
        }
    }

    pub fn load(paths: &AppPaths) -> Self {
        std::fs::read(paths.settings_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        atomic_write_json(&paths.settings_file(), self)
    }

    /// Merge a JSON patch (`{"default_voice":"alba"}`) into the settings; unknown keys and invalid values are rejected.
    pub fn apply_patch(&self, patch: &serde_json::Value) -> Result<Self> {
        let mut cur = serde_json::to_value(self)?;
        let (Some(c), Some(p)) = (cur.as_object_mut(), patch.as_object()) else {
            return Err(Error::Other("settings patch must be an object".into()));
        };
        for (k, v) in p {
            if !c.contains_key(k) {
                return Err(Error::Other(format!("unknown setting '{k}'")));
            }
            c.insert(k.clone(), v.clone());
        }
        let next: Settings = serde_json::from_value(cur)
            .map_err(|e| Error::Other(format!("invalid setting: {e}")))?;
        if !(0.5..=2.0).contains(&next.default_speed) {
            return Err(Error::Other(
                "default_speed must be between 0.5 and 2.0".into(),
            ));
        }
        if let Some(b) = &next.cookies_browser {
            if !crate::download::discover::is_known_browser(b) {
                return Err(Error::Other(format!("unknown browser '{b}'")));
            }
        }
        Ok(next)
    }
}

/// The current transcript / TTS output / editor text, restored on relaunch (not a history library).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub transcript: Option<WorkspaceTranscript>,
    pub tts: Option<WorkspaceTts>,
    /// a URL whose download was started and not finished (quit/crash/failure): the URL field is refilled on launch so
    /// retrying is one click. Cleared when the download completes or is cancelled.
    pub pending_url: Option<String>,
    /// set by a normal quit (⌘Q, Dock → Quit; not a crash): the next launch opens on the empty Transcribe screen and
    /// offers the transcript behind a "Last transcript" link instead of showing it. Cleared when the transcript is
    /// shown again, replaced or cleared (DECISIONS C-026).
    pub transcript_parked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceTranscript {
    pub text: String,
    pub source_name: String,
    pub language: Option<String>,
    pub duration_s: f64,
    pub words: usize,
    pub edited: bool,
    pub exported_or_copied: bool,
    pub recording_id: Option<String>,
    /// transcribed from this URL (the audio is kept in `downloads/<download_id>/` while this is the current transcript)
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub download_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceTts {
    pub text: String,
    pub voice: String,
    pub output_id: Option<String>,
    pub source_name: Option<String>,
}

impl Workspace {
    pub fn load(paths: &AppPaths) -> Self {
        std::fs::read(paths.workspace_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        atomic_write_json(&paths.workspace_file(), self)
    }
    /// Normal quit: park the current transcript (C-026). Returns whether anything changed.
    pub fn park_transcript(&mut self) -> bool {
        let park = self.transcript.is_some() && !self.transcript_parked;
        self.transcript_parked |= park;
        park
    }
}

pub fn atomic_write_json(path: &Path, v: &impl Serialize) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::io(d, e))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub recordings_deleted: Vec<String>,
    pub outputs_deleted: Vec<String>,
    pub cache_files_deleted: usize,
}

fn age(p: &Path, now: SystemTime) -> Option<Duration> {
    now.duration_since(std::fs::metadata(p).ok()?.modified().ok()?)
        .ok()
}

/// Retention policy (BRIEF §8.5, §9.4). **Never** deletes a recording that hasn't been transcribed (or explicitly
/// discarded), nor the current workspace output. Decoded-PCM cache older than one day is always safe to delete.
pub fn cleanup(
    paths: &AppPaths,
    settings: &Settings,
    ws: &Workspace,
    now: SystemTime,
) -> Result<CleanupReport> {
    let mut rep = CleanupReport::default();
    if let Some(keep) = settings.recording_retention.duration() {
        if let Ok(rd) = std::fs::read_dir(paths.recordings()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_none_or(|x| x != "json") {
                    continue;
                }
                let Ok(sc) = Sidecar::read(&p) else { continue };
                let wav = paths.recordings().join(&sc.file);
                let deletable = sc.transcribed
                    && matches!(
                        sc.state,
                        RecState::Finalized | RecState::Recovered | RecState::StoppedByError
                    );
                if deletable && age(&wav, now).is_some_and(|a| a > keep) {
                    let _ = std::fs::remove_file(&wav);
                    let _ = std::fs::remove_file(&p);
                    rep.recordings_deleted.push(sc.id);
                }
            }
        }
    }
    let current = ws.tts.as_ref().and_then(|t| t.output_id.clone());
    if let Ok(rd) = std::fs::read_dir(paths.tts_outputs()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if Some(&name) == current.as_ref() {
                continue;
            }
            if age(&e.path(), now).is_some_and(|a| a > Duration::from_secs(7 * 86_400)) {
                let _ = std::fs::remove_dir_all(e.path());
                rep.outputs_deleted.push(name);
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(paths.decode_cache()) {
        for e in rd.flatten() {
            if age(&e.path(), now).is_some_and(|a| a > Duration::from_secs(86_400))
                && std::fs::remove_file(e.path()).is_ok()
            {
                rep.cache_files_deleted += 1;
            }
        }
    }
    Ok(rep)
}
