//! Local rotating log files (BRIEF §5.5, §11.4): size-capped (default 5 × 2 MB), never uploaded anywhere.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct RotatingFile {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    dir: PathBuf,
    base: String,
    max_bytes: u64,
    keep: usize,
    file: Option<File>,
    written: u64,
}

impl Inner {
    fn path(&self, n: usize) -> PathBuf {
        if n == 0 {
            self.dir.join(format!("{}.log", self.base))
        } else {
            self.dir.join(format!("{}.{n}.log", self.base))
        }
    }
    fn open(&mut self) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let p = self.path(0);
        self.written = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        self.file = Some(OpenOptions::new().create(true).append(true).open(p)?);
        Ok(())
    }
    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let _ = std::fs::remove_file(self.path(self.keep - 1));
        for n in (0..self.keep - 1).rev() {
            let _ = std::fs::rename(self.path(n), self.path(n + 1));
        }
        self.open()
    }
}

impl RotatingFile {
    pub fn new(dir: impl Into<PathBuf>, base: &str, max_bytes: u64, keep: usize) -> Self {
        Self { inner: Arc::new(Mutex::new(Inner { dir: dir.into(), base: base.into(), max_bytes, keep: keep.max(2), file: None, written: 0 })) }
    }
    pub fn standard(dir: impl Into<PathBuf>) -> Self {
        Self::new(dir, "parlo", 2 * 1024 * 1024, 5)
    }
    pub fn handle(&self) -> RotatingHandle {
        RotatingHandle { inner: self.inner.clone() }
    }
}

#[derive(Clone)]
pub struct RotatingHandle {
    inner: Arc<Mutex<Inner>>,
}

impl Write for RotatingHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut g = self.inner.lock().unwrap();
        if g.file.is_none() {
            g.open()?;
        }
        if g.written + buf.len() as u64 > g.max_bytes {
            g.rotate()?;
        }
        let n = g.file.as_mut().unwrap().write(buf)?;
        g.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        match self.inner.lock().unwrap().file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

/// Last `n` lines of the current log (for "Copy diagnostics").
pub fn tail_lines(dir: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(dir.join("parlo.log")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}
