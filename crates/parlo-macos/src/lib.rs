//! macOS implementations of the `parlo-core::platform` traits (BRIEF §5.7, docs/MAC_HANDOFF.md section B).
//!
//! Status: DRAFTED IN THE CLOUD — type-checked from Linux with `cargo check --target aarch64-apple-darwin -p parlo-macos`
//! where that passes (see MAC_HANDOFF.md), UNCHECKED on a Mac otherwise.

#[cfg(target_os = "macos")]
pub mod aac;
#[cfg(target_os = "macos")]
pub mod decode;
#[cfg(target_os = "macos")]
pub mod memory;
#[cfg(target_os = "macos")]
pub mod mic;
#[cfg(target_os = "macos")]
pub mod pdf;
#[cfg(target_os = "macos")]
pub mod power;
#[cfg(target_os = "macos")]
pub mod sleepwake;

#[cfg(target_os = "macos")]
use parlo_core::platform::Platform;
#[cfg(target_os = "macos")]
use std::sync::Arc;

/// Replace the Linux stubs in `p` with the real macOS implementations.
#[cfg(target_os = "macos")]
pub fn install(p: &mut Platform) {
    p.power = Arc::new(power::MacPowerGuard);
    p.sleep_wake = Arc::new(sleepwake::MacSleepWake::new());
    p.memory = Arc::new(memory::MacMemoryPressure::new());
    p.pdf = Some(Arc::new(pdf::PdfKitExtractor));
    p.aac = Arc::new(aac::AudioToolboxAac);
    p.fallback_decoder = Some(Arc::new(decode::AvFoundationDecoder));
}
