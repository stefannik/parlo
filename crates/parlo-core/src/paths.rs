//! The single place that knows where things live (BRIEF §5.5). macOS: ~/Library/...; Linux (dev/test only): XDG.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    /// settings.json, workspace.json, recordings/, outputs/, jobs/
    pub data: PathBuf,
    /// temporary decoded PCM etc. — safe to delete
    pub cache: PathBuf,
    /// rotating logs
    pub logs: PathBuf,
}

impl AppPaths {
    /// Platform default locations. `PARLO_HOME` (if set) redirects everything under one directory (tests, dev).
    pub fn discover() -> Self {
        if let Some(home) = std::env::var_os("PARLO_HOME") {
            return Self::under(home);
        }
        if cfg!(target_os = "macos") {
            let h = directories::BaseDirs::new()
                .map(|b| b.home_dir().to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            return Self {
                data: h.join("Library/Application Support/Parlo"),
                cache: h.join("Library/Caches/Parlo"),
                logs: h.join("Library/Logs/Parlo"),
            };
        }
        match directories::ProjectDirs::from("com", "parlo", "Parlo") {
            Some(d) => Self {
                data: d.data_dir().to_path_buf(),
                cache: d.cache_dir().to_path_buf(),
                logs: d.state_dir().unwrap_or_else(|| d.data_dir()).join("logs"),
            },
            None => Self::under(PathBuf::from(".parlo")),
        }
    }

    pub fn under(root: impl AsRef<Path>) -> Self {
        let r = root.as_ref();
        Self {
            data: r.join("data"),
            cache: r.join("cache"),
            logs: r.join("logs"),
        }
    }

    pub fn recordings(&self) -> PathBuf {
        self.data.join("recordings")
    }
    pub fn tts_outputs(&self) -> PathBuf {
        self.data.join("outputs").join("tts")
    }
    pub fn jobs(&self) -> PathBuf {
        self.data.join("jobs")
    }
    pub fn settings_file(&self) -> PathBuf {
        self.data.join("settings.json")
    }
    pub fn workspace_file(&self) -> PathBuf {
        self.data.join("workspace.json")
    }
    pub fn decode_cache(&self) -> PathBuf {
        self.cache.join("decoded")
    }
}

/// Free bytes on the filesystem containing `path` (nearest existing ancestor).
pub fn free_disk_bytes(path: &Path) -> std::io::Result<u64> {
    let mut p = path;
    while !p.exists() {
        match p.parent() {
            Some(x) => p = x,
            None => break,
        }
    }
    let c = std::ffi::CString::new(p.as_os_str().to_string_lossy().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
}
