//! `SleepWakeEvents` from `NSWorkspace` willSleep / didWake notifications. DRAFTED, UNCHECKED on a Mac.

use block2::RcBlock;
use crossbeam_channel::{unbounded, Receiver, Sender};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::NSNotification;
use parlo_core::platform::{PowerEvent, SleepWakeEvents};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

pub struct MacSleepWake {
    subs: Arc<Mutex<Vec<Sender<PowerEvent>>>>,
}

impl MacSleepWake {
    /// Registers the observers once; they live for the lifetime of the process.
    pub fn new() -> Self {
        let subs: Arc<Mutex<Vec<Sender<PowerEvent>>>> = Arc::new(Mutex::new(Vec::new()));
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        for (name, ev) in unsafe {
            [
                (NSWorkspaceWillSleepNotification, PowerEvent::WillSleep),
                (NSWorkspaceDidWakeNotification, PowerEvent::DidWake),
            ]
        } {
            let s = subs.clone();
            let block = RcBlock::new(move |_n: NonNull<NSNotification>| {
                s.lock().unwrap().retain(|tx| tx.send(ev).is_ok());
            });
            // The returned observer token must stay alive; we intentionally leak it (process-lifetime observer).
            let token = unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
            };
            std::mem::forget(token);
        }
        Self { subs }
    }
}

impl Default for MacSleepWake {
    fn default() -> Self {
        Self::new()
    }
}

impl SleepWakeEvents for MacSleepWake {
    fn subscribe(&self) -> Receiver<PowerEvent> {
        let (tx, rx) = unbounded();
        self.subs.lock().unwrap().push(tx);
        rx
    }
}
