//! The set of OS integrations the app uses. `Platform::native()` picks the macOS implementations on macOS and the
//! Linux stubs elsewhere (Linux exists only so the core can be built and tested in the cloud, BRIEF §5.7).

use super::*;
use crate::error::{Error, Result};
use std::path::Path;
use std::sync::Arc;

pub struct UnavailableAac;
impl AacEncoder for UnavailableAac {
    fn is_available(&self) -> bool {
        false
    }
    fn encode_wav_to_m4a(&self, _wav: &Path, _dest: &Path, _kbps: u32) -> Result<()> {
        Err(Error::Encode(
            "M4A export is not available on this platform".into(),
        ))
    }
}

pub struct Platform {
    pub input: Arc<dyn AudioInput>,
    pub power: Arc<dyn PowerGuard>,
    pub sleep_wake: Arc<dyn SleepWakeEvents>,
    pub memory: Arc<dyn MemoryPressure>,
    pub disk: Arc<dyn DiskSpace>,
    pub pdf: Option<Arc<dyn PdfTextExtractor>>,
    pub aac: Arc<dyn AacEncoder>,
    pub fallback_decoder: Option<Arc<dyn FallbackDecoder>>,
}

impl Platform {
    pub fn native() -> Self {
        #[cfg(feature = "cpal-input")]
        let input: Arc<dyn AudioInput> = Arc::new(super::cpal_input::CpalInput::new());
        #[cfg(not(feature = "cpal-input"))]
        let input: Arc<dyn AudioInput> = Arc::new(super::file_source::FileSource::counter(
            48_000,
            super::file_source::Mode::Realtime,
        ));
        Self {
            input,
            power: Arc::new(NoopPowerGuard),
            sleep_wake: Arc::new(ManualSleepWake::new()),
            memory: Arc::new(ManualMemoryPressure::new()),
            disk: Arc::new(SystemDisk),
            pdf: None,
            aac: Arc::new(UnavailableAac),
            fallback_decoder: None,
        }
    }
}
