//! Crash-safe streaming WAV writer (PCM16 mono/stereo) and header repair.
//!
//! Layout is the canonical 44-byte header followed by data. The RIFF/data sizes are *patched in place* on every
//! [`WavWriter::sync`] (flush + patch + fsync), so after a hard crash the file is playable and at most one sync
//! interval of audio is lost; [`repair_wav`] restores exact sizes from the file length.

use crate::error::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

pub const HEADER_LEN: u64 = 44;
/// Largest data chunk a classic RIFF/WAVE header can describe (BRIEF §9.4: >4 GB export guard).
pub const MAX_DATA_BYTES: u64 = u32::MAX as u64 - 36;

fn header(channels: u16, sample_rate: u32, data_len: u32) -> [u8; 44] {
    let block_align = channels * 2;
    let byte_rate = sample_rate * block_align as u32;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&data_len.saturating_add(36).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_len.to_le_bytes());
    h
}

pub struct WavWriter {
    out: BufWriter<File>,
    patch: File,
    path: PathBuf,
    channels: u16,
    sample_rate: u32,
    data_bytes: u64,
    synced_bytes: u64,
}

impl WavWriter {
    pub fn create(path: &Path, channels: u16, sample_rate: u32) -> Result<Self> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| Error::io(d, e))?;
        }
        let f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            .open(path)
            .map_err(|e| Error::io(path, e))?;
        let patch = f.try_clone().map_err(|e| Error::io(path, e))?;
        let mut out = BufWriter::with_capacity(256 * 1024, f);
        out.write_all(&header(channels, sample_rate, 0))
            .map_err(|e| Error::io(path, e))?;
        out.flush().map_err(|e| Error::io(path, e))?;
        Ok(Self {
            out,
            patch,
            path: path.to_path_buf(),
            channels,
            sample_rate,
            data_bytes: 0,
            synced_bytes: 0,
        })
    }

    /// Re-open an existing Parlo WAV for appending, truncating it to exactly `frames` frames first (resume after a
    /// crash: the manifest says how many frames were durably finished).
    pub fn open_append(path: &Path, frames: u64) -> Result<Self> {
        let info = repair_wav(path)?;
        if frames > info.frames {
            return Err(Error::State(format!(
                "{} has {} frames, checkpoint expects {frames}",
                path.display(),
                info.frames
            )));
        }
        let channels = info.channels;
        let data_bytes = frames * 2 * channels as u64;
        let mut f = OpenOptions::new()
            .write(true)
            .read(true)
            .open(path)
            .map_err(|e| Error::io(path, e))?;
        f.set_len(HEADER_LEN + data_bytes)
            .map_err(|e| Error::io(path, e))?;
        use std::io::{Seek, SeekFrom};
        f.seek(SeekFrom::End(0)).map_err(|e| Error::io(path, e))?;
        let patch = f.try_clone().map_err(|e| Error::io(path, e))?;
        let w = Self {
            out: BufWriter::with_capacity(256 * 1024, f),
            patch,
            path: path.to_path_buf(),
            channels,
            sample_rate: info.sample_rate,
            data_bytes,
            synced_bytes: data_bytes,
        };
        w.patch_header()?;
        Ok(w)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn frames(&self) -> u64 {
        self.data_bytes / (2 * self.channels as u64)
    }
    pub fn data_bytes(&self) -> u64 {
        self.data_bytes
    }
    pub fn remaining_capacity_bytes(&self) -> u64 {
        MAX_DATA_BYTES.saturating_sub(self.data_bytes)
    }

    /// Append interleaved PCM16 samples. Fails with `State` (nothing written) if the WAV 4 GB limit would be exceeded.
    pub fn write_i16(&mut self, samples: &[i16]) -> Result<()> {
        let bytes = samples.len() as u64 * 2;
        if self.data_bytes + bytes > MAX_DATA_BYTES {
            return Err(Error::State("WAV size limit (4 GB) reached".into()));
        }
        // little-endian on all supported targets (x86_64, aarch64)
        let raw =
            unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const u8, samples.len() * 2) };
        #[cfg(target_endian = "big")]
        compile_error!("big-endian targets are not supported");
        self.out
            .write_all(raw)
            .map_err(|e| Error::io(&self.path, e))?;
        self.data_bytes += bytes;
        Ok(())
    }

    /// Flush buffered data, patch the header sizes in place and fsync.
    pub fn sync(&mut self) -> Result<()> {
        self.out.flush().map_err(|e| Error::io(&self.path, e))?;
        self.patch_header()?;
        self.patch
            .sync_data()
            .map_err(|e| Error::io(&self.path, e))?;
        self.synced_bytes = self.data_bytes;
        Ok(())
    }

    fn patch_header(&self) -> Result<()> {
        let n = self.data_bytes as u32;
        self.patch
            .write_all_at(&(n.saturating_add(36)).to_le_bytes(), 4)
            .map_err(|e| Error::io(&self.path, e))?;
        self.patch
            .write_all_at(&n.to_le_bytes(), 40)
            .map_err(|e| Error::io(&self.path, e))?;
        Ok(())
    }

    /// Final sync; returns total frames written.
    pub fn finalize(mut self) -> Result<u64> {
        self.sync()?;
        Ok(self.frames())
    }
}

/// Information recovered from a (possibly crash-truncated) WAV written by [`WavWriter`].
#[derive(Debug, Clone, PartialEq)]
pub struct WavInfo {
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: u64,
}

/// Repair a crash-truncated WAV: sizes are rewritten from the actual file length (a trailing partial frame is
/// truncated). Idempotent. Returns the recovered info. Errors if the file isn't a recognisable PCM16 WAV.
pub fn repair_wav(path: &Path) -> Result<WavInfo> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| Error::io(path, e))?;
    let len = f.metadata().map_err(|e| Error::io(path, e))?.len();
    if len < HEADER_LEN {
        return Err(Error::State(format!(
            "{} is too short to be a WAV file ({len} bytes)",
            path.display()
        )));
    }
    let mut h = [0u8; 44];
    f.read_exact_at(&mut h, 0).map_err(|e| Error::io(path, e))?;
    if &h[0..4] != b"RIFF" || &h[8..12] != b"WAVE" || &h[12..16] != b"fmt " || &h[36..40] != b"data"
    {
        return Err(Error::State(format!(
            "{} is not a Parlo WAV (unexpected header)",
            path.display()
        )));
    }
    let channels = u16::from_le_bytes([h[22], h[23]]);
    let sample_rate = u32::from_le_bytes([h[24], h[25], h[26], h[27]]);
    if channels == 0 || sample_rate == 0 || u16::from_le_bytes([h[34], h[35]]) != 16 {
        return Err(Error::State(format!(
            "{} has an unsupported WAV format",
            path.display()
        )));
    }
    let frame = 2 * channels as u64;
    let data = ((len - HEADER_LEN) / frame) * frame;
    if data < len - HEADER_LEN {
        f.set_len(HEADER_LEN + data)
            .map_err(|e| Error::io(path, e))?;
    }
    let n = data.min(MAX_DATA_BYTES) as u32;
    f.write_all_at(&n.saturating_add(36).to_le_bytes(), 4)
        .map_err(|e| Error::io(path, e))?;
    f.write_all_at(&n.to_le_bytes(), 40)
        .map_err(|e| Error::io(path, e))?;
    f.sync_all().map_err(|e| Error::io(path, e))?;
    Ok(WavInfo {
        channels,
        sample_rate,
        frames: data / frame,
    })
}

/// Random-access reader over a PCM16 WAV produced by Parlo (bounded memory: reads only the requested range).
pub struct PcmFile {
    f: File,
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: u64,
}

impl PcmFile {
    pub fn open(path: &Path) -> Result<Self> {
        let f = File::open(path).map_err(|e| Error::io(path, e))?;
        let len = f.metadata().map_err(|e| Error::io(path, e))?.len();
        let mut h = [0u8; 44];
        f.read_exact_at(&mut h, 0).map_err(|e| Error::io(path, e))?;
        if &h[0..4] != b"RIFF" || &h[36..40] != b"data" {
            return Err(Error::State(format!(
                "{} is not a Parlo PCM WAV",
                path.display()
            )));
        }
        let channels = u16::from_le_bytes([h[22], h[23]]);
        let sample_rate = u32::from_le_bytes([h[24], h[25], h[26], h[27]]);
        let frames = (len.saturating_sub(HEADER_LEN)) / (2 * channels as u64);
        Ok(Self {
            f,
            channels,
            sample_rate,
            frames,
        })
    }

    /// Read `n` frames starting at `start` as mono f32 (first channel if multi-channel), clamped to the file end.
    pub fn read_f32(&self, start: u64, n: usize, out: &mut Vec<f32>) -> Result<()> {
        out.clear();
        if start >= self.frames {
            return Ok(());
        }
        let n = n.min((self.frames - start) as usize);
        let ch = self.channels as usize;
        let mut raw = vec![0u8; n * ch * 2];
        self.f
            .read_exact_at(&mut raw, HEADER_LEN + start * 2 * ch as u64)
            .map_err(Error::IoBare)?;
        out.reserve(n);
        for i in 0..n {
            let o = i * ch * 2;
            out.push(i16::from_le_bytes([raw[o], raw[o + 1]]) as f32 / 32768.0);
        }
        Ok(())
    }
}
