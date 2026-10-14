//! `AudioInput`: the capture abstraction. The real-time callback may ONLY copy samples into the lock-free ring buffer
//! (no allocation, no locks, no I/O) — see [`InputCtl::push`].

use crate::error::Result;
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSelector {
    /// Follow the system default input; switch when it changes (AirPods in/out).
    SystemDefault,
    /// A specific device; if it vanishes the recorder falls back to the system default and notifies.
    Named(String),
}

/// Out-of-band events from a stream (never sent from the real-time path except `StreamError`, which is rare).
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    StreamError(String),
}

/// Handed to the input implementation on `open`: where to put samples and how to report things.
pub struct InputCtl {
    pub producer: rtrb::Producer<f32>,
    /// total interleaved samples *delivered by the device* (including dropped ones) — heartbeat for stall detection
    pub delivered: Arc<AtomicU64>,
    /// interleaved samples dropped because the ring was full (overruns)
    pub dropped: Arc<AtomicU64>,
    pub events: Sender<InputEvent>,
}

impl InputCtl {
    /// Real-time safe: copies `samples` into the ring (drops the tail if full), updates counters.
    #[inline]
    pub fn push(&mut self, samples: &[f32]) {
        self.delivered
            .fetch_add(samples.len() as u64, Ordering::Relaxed);
        let free = self.producer.slots();
        let n = free.min(samples.len());
        if n > 0 {
            if let Ok(mut chunk) = self.producer.write_chunk_uninit(n) {
                let (a, b) = chunk.as_mut_slices();
                let (s1, s2) = samples[..n].split_at(a.len());
                for (d, s) in a.iter_mut().zip(s1) {
                    d.write(*s);
                }
                for (d, s) in b.iter_mut().zip(s2) {
                    d.write(*s);
                }
                unsafe { chunk.commit_all() };
            }
        }
        if n < samples.len() {
            self.dropped
                .fetch_add((samples.len() - n) as u64, Ordering::Relaxed);
        }
    }
}

/// A running stream. Dropping `guard` stops capture.
pub struct OpenedStream {
    pub sample_rate: u32,
    pub channels: u16,
    pub device_name: String,
    pub guard: Box<dyn std::any::Any + Send>,
}

pub trait AudioInput: Send + Sync {
    fn devices(&self) -> Result<Vec<DeviceInfo>>;
    /// Id of the current system default input device (polled while recording to follow device switches).
    fn default_device_id(&self) -> Option<String>;
    /// Open a stream on `sel`. Errors: `Error::MicPermission`, `Error::Device`.
    fn open(&self, sel: &DeviceSelector, ctl: InputCtl) -> Result<OpenedStream>;
}
