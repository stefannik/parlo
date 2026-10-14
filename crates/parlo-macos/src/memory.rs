//! `MemoryPressure` from a `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE` dispatch source. DRAFTED, UNCHECKED on a Mac.
//! (Plain FFI: the `dispatch2` bindings don't expose the memory-pressure source type.)

use block2::RcBlock;
use crossbeam_channel::{unbounded, Receiver, Sender};
use parlo_core::platform::{MemoryPressure, MemoryPressureLevel};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

const DISPATCH_MEMORYPRESSURE_NORMAL: usize = 0x01;
const DISPATCH_MEMORYPRESSURE_WARN: usize = 0x02;
const DISPATCH_MEMORYPRESSURE_CRITICAL: usize = 0x04;

extern "C" {
    static _dispatch_source_type_memorypressure: c_void;
    fn dispatch_source_create(kind: *const c_void, handle: usize, mask: usize, queue: *mut c_void) -> *mut c_void;
    fn dispatch_source_set_event_handler(source: *mut c_void, handler: &block2::Block<dyn Fn()>);
    fn dispatch_source_get_data(source: *mut c_void) -> usize;
    fn dispatch_resume(object: *mut c_void);
    fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
}

pub struct MacMemoryPressure {
    subs: Arc<Mutex<Vec<Sender<MemoryPressureLevel>>>>,
}

impl MacMemoryPressure {
    pub fn new() -> Self {
        let subs: Arc<Mutex<Vec<Sender<MemoryPressureLevel>>>> = Arc::new(Mutex::new(Vec::new()));
        unsafe {
            let queue = dispatch_get_global_queue(0, 0);
            let src = dispatch_source_create(&_dispatch_source_type_memorypressure as *const c_void, 0, DISPATCH_MEMORYPRESSURE_NORMAL | DISPATCH_MEMORYPRESSURE_WARN | DISPATCH_MEMORYPRESSURE_CRITICAL, queue);
            if src.is_null() {
                tracing::warn!("could not create the memory-pressure dispatch source");
                return Self { subs };
            }
            let s2 = subs.clone();
            let src_addr = src as usize;
            let handler = RcBlock::new(move || {
                let data = dispatch_source_get_data(src_addr as *mut c_void);
                let level = if data & DISPATCH_MEMORYPRESSURE_CRITICAL != 0 {
                    MemoryPressureLevel::Critical
                } else if data & DISPATCH_MEMORYPRESSURE_WARN != 0 {
                    MemoryPressureLevel::Warning
                } else {
                    MemoryPressureLevel::Normal
                };
                s2.lock().unwrap().retain(|tx| tx.send(level).is_ok());
            });
            dispatch_source_set_event_handler(src, &handler);
            std::mem::forget(handler); // process-lifetime source
            dispatch_resume(src);
        }
        Self { subs }
    }
}

impl Default for MacMemoryPressure {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryPressure for MacMemoryPressure {
    fn subscribe(&self) -> Receiver<MemoryPressureLevel> {
        let (tx, rx) = unbounded();
        self.subs.lock().unwrap().push(tx);
        rx
    }
}
