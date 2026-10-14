//! Everything that differs per OS sits behind small traits here (BRIEF §5.7). The `macos` implementations are the
//! product; the `linux` ones exist only so the core can be built and tested in the cloud. Anything not compiled/run on
//! macOS is tracked in docs/MAC_HANDOFF.md.

pub mod audio_input;
pub mod file_source;
pub mod system;

#[cfg(feature = "cpal-input")]
pub mod cpal_input;

pub use audio_input::{AudioInput, DeviceInfo, DeviceSelector, InputCtl, InputEvent, OpenedStream};
pub use system::{
    AacEncoder, DiskSpace, FallbackDecoder, ManualMemoryPressure, ManualSleepWake, MemoryPressure,
    MemoryPressureLevel, MemoryRelief, NoopPowerGuard, PdfTextExtractor, PowerEvent, PowerGuard,
    SleepWakeEvents, SystemDisk,
};
