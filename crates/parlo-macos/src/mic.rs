//! Microphone permission (TCC) via AVFoundation. DRAFTED, UNCHECKED on a Mac.

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NotDetermined,
    Granted,
    Denied,
    Restricted,
}

pub fn authorization_status() -> Status {
    let media = unsafe { AVMediaTypeAudio }.expect("AVMediaTypeAudio");
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(media) } {
        AVAuthorizationStatus::NotDetermined => Status::NotDetermined,
        AVAuthorizationStatus::Authorized => Status::Granted,
        AVAuthorizationStatus::Restricted => Status::Restricted,
        _ => Status::Denied,
    }
}

/// Ask for microphone access (shows the system prompt the first time) and wait for the answer.
pub fn request_access_blocking() -> bool {
    if authorization_status() == Status::Granted {
        return true;
    }
    let media = unsafe { AVMediaTypeAudio }.expect("AVMediaTypeAudio");
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    let block = RcBlock::new(move |granted: Bool| {
        let _ = tx.send(granted.as_bool());
    });
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &block) };
    rx.recv_timeout(Duration::from_secs(300)).unwrap_or(false)
}
