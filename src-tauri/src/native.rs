//! OS integration glue. macOS implementations live in `crates/parlo-macos` (objc2-based, type-checked from Linux with
//! `cargo check --target aarch64-apple-darwin -p parlo-macos`); Linux has stubs so the shell builds and runs in CI.

use crate::ipc::MicPermission;
use crate::state::AppState;
use parlo_core::platform::Platform;
use parlo_core::store::{Performance, Settings};

/// The platform bundle for this OS.
pub fn platform() -> Platform {
    #[allow(unused_mut)]
    let mut p = Platform::native();
    #[cfg(target_os = "macos")]
    parlo_macos::install(&mut p);
    p
}

#[cfg(target_os = "macos")]
pub fn mic_permission_status(_s: &AppState) -> MicPermission {
    match parlo_macos::mic::authorization_status() {
        parlo_macos::mic::Status::NotDetermined => MicPermission::NotDetermined,
        parlo_macos::mic::Status::Granted => MicPermission::Granted,
        parlo_macos::mic::Status::Denied => MicPermission::Denied,
        parlo_macos::mic::Status::Restricted => MicPermission::Restricted,
    }
}
#[cfg(not(target_os = "macos"))]
pub fn mic_permission_status(s: &AppState) -> MicPermission {
    // Linux (dev/CI only): there is no TCC; "granted" iff an input device exists
    match parlo_core::platform::AudioInput::devices(&*s.platform.input) {
        Ok(d) if !d.is_empty() => MicPermission::Granted,
        _ => MicPermission::Unknown,
    }
}

#[cfg(target_os = "macos")]
pub fn mic_permission_request(_s: &AppState) -> MicPermission {
    if parlo_macos::mic::request_access_blocking() {
        MicPermission::Granted
    } else {
        MicPermission::Denied
    }
}
#[cfg(not(target_os = "macos"))]
pub fn mic_permission_request(s: &AppState) -> MicPermission {
    mic_permission_status(s)
}

pub fn open_privacy_settings(kind: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let pane = match kind {
            "microphone" => "Privacy_Microphone",
            _ => "Privacy",
        };
        std::process::Command::new("open").arg(format!("x-apple.systempreferences:com.apple.preference.security?{pane}")).status().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Ok(())
    }
}

/// CPU threads for whisper (BRIEF §10 "Good-citizen compute"): Balanced ≈ half the cores, Maximum = all.
pub fn stt_threads(s: &Settings) -> usize {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    match s.performance {
        Performance::Maximum => n,
        Performance::Balanced => (n / 2).max(2),
    }
}
