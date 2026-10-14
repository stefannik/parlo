//! Small OS-integration traits (BRIEF §5.7) with Linux/no-op implementations and test doubles.

use crate::error::Result;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::path::Path;
use std::sync::Mutex;

/// Free disk space (injectable so tests can simulate "disk nearly full").
pub trait DiskSpace: Send + Sync {
    fn free_bytes(&self, path: &Path) -> std::io::Result<u64>;
}
pub struct SystemDisk;
impl DiskSpace for SystemDisk {
    fn free_bytes(&self, path: &Path) -> std::io::Result<u64> {
        crate::paths::free_disk_bytes(path)
    }
}

/// macOS: AVFoundation (`AVAssetReader`) fallback for files symphonia can't decode; Linux: none.
pub trait FallbackDecoder: Send + Sync {
    /// Decode `src` to a 16 kHz mono PCM16 WAV at `dest`. Return `Ok(false)` if the file is not handled.
    fn decode_to_wav16k(&self, src: &Path, dest: &Path) -> Result<bool>;
}

/// macOS: AudioToolbox `ExtAudioFile` M4A/AAC encoder; Linux: unavailable (tests cfg-gated).
pub trait AacEncoder: Send + Sync {
    fn is_available(&self) -> bool;
    /// Encode a mono PCM16 WAV to M4A at `bitrate_kbps`.
    fn encode_wav_to_m4a(&self, wav: &Path, dest: &Path, bitrate_kbps: u32) -> Result<()>;
}

/// macOS: PDFKit `PDFDocument.string` per page; Linux: none (tests use page-text fixtures).
pub trait PdfTextExtractor: Send + Sync {
    fn page_texts(&self, pdf: &Path) -> Result<Vec<String>>;
}

/// Prevent idle system sleep + App Nap while recording or while any job runs. Dropping the token ends the activity.
pub trait PowerGuard: Send + Sync {
    fn begin(&self, reason: &str) -> Box<dyn std::any::Any + Send>;
}
pub struct NoopPowerGuard;
impl PowerGuard for NoopPowerGuard {
    fn begin(&self, _reason: &str) -> Box<dyn std::any::Any + Send> {
        Box::new(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    WillSleep,
    DidWake,
}
pub trait SleepWakeEvents: Send + Sync {
    fn subscribe(&self) -> Receiver<PowerEvent>;
}
/// Linux stub / test double: events are injected with [`ManualSleepWake::send`].
pub struct ManualSleepWake {
    subs: Mutex<Vec<Sender<PowerEvent>>>,
}
impl ManualSleepWake {
    pub fn new() -> Self {
        Self {
            subs: Mutex::new(vec![]),
        }
    }
    pub fn send(&self, e: PowerEvent) {
        self.subs.lock().unwrap().retain(|s| s.send(e).is_ok());
    }
}
impl Default for ManualSleepWake {
    fn default() -> Self {
        Self::new()
    }
}
impl SleepWakeEvents for ManualSleepWake {
    fn subscribe(&self) -> Receiver<PowerEvent> {
        let (tx, rx) = unbounded();
        self.subs.lock().unwrap().push(tx);
        rx
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressureLevel {
    Normal,
    Warning,
    Critical,
}
pub trait MemoryPressure: Send + Sync {
    fn subscribe(&self) -> Receiver<MemoryPressureLevel>;
}
pub struct ManualMemoryPressure {
    subs: Mutex<Vec<Sender<MemoryPressureLevel>>>,
}
impl ManualMemoryPressure {
    pub fn new() -> Self {
        Self {
            subs: Mutex::new(vec![]),
        }
    }
    pub fn send(&self, l: MemoryPressureLevel) {
        self.subs.lock().unwrap().retain(|s| s.send(l).is_ok());
    }
}
impl Default for ManualMemoryPressure {
    fn default() -> Self {
        Self::new()
    }
}
impl MemoryPressure for ManualMemoryPressure {
    fn subscribe(&self) -> Receiver<MemoryPressureLevel> {
        let (tx, rx) = unbounded();
        self.subs.lock().unwrap().push(tx);
        rx
    }
}

/// Return freed heap memory to the OS (macOS: `malloc_zone_pressure_relief`; Linux: `malloc_trim`).
pub struct MemoryRelief;
impl MemoryRelief {
    pub fn relieve() {
        #[cfg(target_os = "linux")]
        unsafe {
            libc::malloc_trim(0);
        }
        #[cfg(target_os = "macos")]
        unsafe {
            extern "C" {
                fn malloc_zone_pressure_relief(
                    zone: *mut libc::c_void,
                    goal: libc::size_t,
                ) -> libc::size_t;
            }
            malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
        }
    }
}
