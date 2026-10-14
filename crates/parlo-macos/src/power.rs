//! `PowerGuard`: `NSProcessInfo beginActivity` (user-initiated + idle-system-sleep-disabled) while recording or any
//! job runs; this also disables App Nap. DRAFTED, UNCHECKED on a Mac.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};
use parlo_core::platform::PowerGuard;
use std::any::Any;

pub struct MacPowerGuard;

struct ActivityToken(Retained<ProtocolObject<dyn NSObjectProtocol>>);
// The token is an opaque NSObject handle; ending the activity from another thread is allowed.
unsafe impl Send for ActivityToken {}

impl Drop for ActivityToken {
    fn drop(&mut self) {
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}

impl PowerGuard for MacPowerGuard {
    fn begin(&self, reason: &str) -> Box<dyn Any + Send> {
        let opts = NSActivityOptions::UserInitiated | NSActivityOptions::IdleSystemSleepDisabled;
        let token = NSProcessInfo::processInfo()
            .beginActivityWithOptions_reason(opts, &NSString::from_str(reason));
        Box::new(ActivityToken(token))
    }
}
