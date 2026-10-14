//! `FileSource`: a scriptable fake input that feeds the *real* recorder pipeline (BRIEF §5.6/§14.1).
//! Real-time mode paces by the wall clock (and drops on overrun like a device would); accelerated mode applies
//! back-pressure instead, so multi-hour soaks run in seconds without dropping samples.

use super::audio_input::*;
use crate::error::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A mono sample generator.
pub trait Signal: Send {
    fn fill(&mut self, out: &mut [f32], first_frame: u64);
}

/// sample[i] encodes `i % 30000` exactly after f32→i16 conversion (`k / 32767`): lets tests prove "no drops, in order".
pub struct CounterSignal;
pub const COUNTER_MOD: u64 = 30_000;
impl Signal for CounterSignal {
    fn fill(&mut self, out: &mut [f32], first_frame: u64) {
        for (i, o) in out.iter_mut().enumerate() {
            *o = ((first_frame + i as u64) % COUNTER_MOD + 1) as f32 / 32767.0;
        }
    }
}

pub struct SineSignal {
    pub freq: f32,
    pub amp: f32,
    pub rate: f32,
}
impl Signal for SineSignal {
    fn fill(&mut self, out: &mut [f32], first_frame: u64) {
        for (i, o) in out.iter_mut().enumerate() {
            let t = (first_frame + i as u64) as f32 / self.rate;
            *o = self.amp * (2.0 * std::f32::consts::PI * self.freq * t).sin();
        }
    }
}

pub struct ConstSignal(pub f32);
impl Signal for ConstSignal {
    fn fill(&mut self, out: &mut [f32], _first: u64) {
        out.fill(self.0);
    }
}

/// Streams samples from a WAV file (looping at the end).
pub struct WavSignal {
    reader: hound::WavReader<std::io::BufReader<std::fs::File>>,
    path: std::path::PathBuf,
}
impl WavSignal {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        Ok(Self {
            reader: hound::WavReader::open(path)?,
            path: path.to_path_buf(),
        })
    }
}
impl Signal for WavSignal {
    fn fill(&mut self, out: &mut [f32], _first: u64) {
        let spec = self.reader.spec();
        let ch = spec.channels as usize;
        for o in out.iter_mut() {
            let mut acc = 0f32;
            for _ in 0..ch {
                let s = match spec.sample_format {
                    hound::SampleFormat::Int => self
                        .reader
                        .samples::<i16>()
                        .next()
                        .map(|r| r.map(|v| v as f32 / 32768.0).unwrap_or(0.0)),
                    hound::SampleFormat::Float => self
                        .reader
                        .samples::<f32>()
                        .next()
                        .map(|r| r.unwrap_or(0.0)),
                };
                match s {
                    Some(v) => acc += v,
                    None => {
                        self.reader = hound::WavReader::open(&self.path).expect("reopen");
                    }
                }
            }
            *o = acc / ch as f32;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Realtime,
    Accelerated,
}

#[derive(Debug, Clone)]
pub enum Fault {
    /// The stream dies (error callback) when the signal reaches `at_frame`.
    StreamError {
        at_frame: u64,
        skip_frames_on_reopen: u64,
    },
    /// The device stops delivering samples for `dur` without any error (stall).
    Stall { at_frame: u64, dur: Duration },
    /// The system default device changes (e.g. AirPods connect): `default_device_id()` returns `new_id` and the next
    /// `open` uses `new_rate`. The old stream keeps running until the recorder switches.
    DefaultDeviceChange {
        at_frame: u64,
        new_id: String,
        new_rate: u32,
    },
    /// The default device is removed: stream error + default changes to `new_id`/`new_rate`.
    DeviceRemoved {
        at_frame: u64,
        new_id: String,
        new_rate: u32,
    },
}
impl Fault {
    fn at(&self) -> u64 {
        match self {
            Fault::StreamError { at_frame, .. }
            | Fault::Stall { at_frame, .. }
            | Fault::DefaultDeviceChange { at_frame, .. }
            | Fault::DeviceRemoved { at_frame, .. } => *at_frame,
        }
    }
}

struct Shared {
    signal: Box<dyn Signal>,
    pos: u64,
    faults: Vec<Fault>,
    default_id: String,
    rate: u32,
    devices: Vec<DeviceInfo>,
    opens: u32,
    deny_permission: bool,
    skip_on_open: u64,
    total_frames: Option<u64>,
}

pub struct FileSource {
    shared: Arc<Mutex<Shared>>,
    channels: u16,
    mode: Mode,
    block_ms: u32,
}

impl FileSource {
    pub fn new(signal: Box<dyn Signal>, rate: u32, channels: u16, mode: Mode) -> Self {
        let default_id = "file-default".to_string();
        Self {
            shared: Arc::new(Mutex::new(Shared {
                signal,
                pos: 0,
                faults: vec![],
                default_id: default_id.clone(),
                rate,
                devices: vec![DeviceInfo {
                    id: default_id.clone(),
                    name: "File source".into(),
                    is_default: true,
                }],
                opens: 0,
                deny_permission: false,
                skip_on_open: 0,
                total_frames: None,
            })),
            channels,
            mode,
            block_ms: 10,
        }
    }
    pub fn counter(rate: u32, mode: Mode) -> Self {
        Self::new(Box::new(CounterSignal), rate, 1, mode)
    }
    pub fn with_faults(self, mut faults: Vec<Fault>) -> Self {
        faults.sort_by_key(|f| f.at());
        self.shared.lock().unwrap().faults = faults;
        self
    }
    pub fn with_total_frames(self, n: u64) -> Self {
        self.shared.lock().unwrap().total_frames = Some(n);
        self
    }
    pub fn with_devices(self, devices: Vec<DeviceInfo>) -> Self {
        {
            let mut s = self.shared.lock().unwrap();
            if let Some(d) = devices.iter().find(|d| d.is_default) {
                s.default_id = d.id.clone();
            }
            s.devices = devices;
        }
        self
    }
    pub fn deny_permission(self) -> Self {
        self.shared.lock().unwrap().deny_permission = true;
        self
    }
    /// frames of signal consumed so far
    pub fn position(&self) -> u64 {
        self.shared.lock().unwrap().pos
    }
    pub fn opens(&self) -> u32 {
        self.shared.lock().unwrap().opens
    }
    pub fn set_default_device(&self, id: &str, rate: u32) {
        let mut s = self.shared.lock().unwrap();
        s.default_id = id.to_string();
        s.rate = rate;
        if !s.devices.iter().any(|d| d.id == id) {
            s.devices.push(DeviceInfo {
                id: id.into(),
                name: id.into(),
                is_default: false,
            });
        }
        for d in s.devices.iter_mut() {
            d.is_default = d.id == id;
        }
    }
    pub fn remove_device(&self, id: &str) {
        self.shared.lock().unwrap().devices.retain(|d| d.id != id);
    }
}

struct Guard {
    stop: Arc<AtomicBool>,
    th: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.th.take() {
            let _ = t.join();
        }
    }
}

impl AudioInput for FileSource {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        Ok(self.shared.lock().unwrap().devices.clone())
    }
    fn default_device_id(&self) -> Option<String> {
        Some(self.shared.lock().unwrap().default_id.clone())
    }
    fn open(&self, sel: &DeviceSelector, mut ctl: InputCtl) -> Result<OpenedStream> {
        let (rate, name) = {
            let mut s = self.shared.lock().unwrap();
            if s.deny_permission {
                return Err(Error::MicPermission);
            }
            let name = match sel {
                DeviceSelector::SystemDefault => s.default_id.clone(),
                DeviceSelector::Named(n) => {
                    if !s.devices.iter().any(|d| &d.id == n) {
                        return Err(Error::Device(format!("device '{n}' is not available")));
                    }
                    n.clone()
                }
            };
            s.opens += 1;
            let skip = std::mem::take(&mut s.skip_on_open);
            s.pos += skip;
            (s.rate, name)
        };
        let channels = self.channels as usize;
        let mode = self.mode;
        let block = (rate as usize * self.block_ms as usize / 1000).max(1);
        let shared = self.shared.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let th = std::thread::Builder::new()
            .name("file-source".into())
            .spawn(move || {
                let start = Instant::now();
                let mut produced_here: u64 = 0;
                let mut mono = vec![0f32; block];
                let mut inter = vec![0f32; block * channels];
                while !stop2.load(Ordering::Relaxed) {
                    // faults + end-of-signal
                    let (first, n, fault) = {
                        let mut s = shared.lock().unwrap();
                        let first = s.pos;
                        let mut n = block as u64;
                        if let Some(t) = s.total_frames {
                            if first >= t {
                                drop(s);
                                std::thread::sleep(Duration::from_millis(2));
                                continue;
                            }
                            n = n.min(t - first);
                        }
                        let mut fault = None;
                        if let Some(f) = s.faults.first() {
                            if f.at() <= first {
                                fault = Some(s.faults.remove(0));
                            } else {
                                n = n.min(f.at() - first);
                            }
                        }
                        (first, n as usize, fault)
                    };
                    if let Some(f) = fault {
                        match f {
                            Fault::StreamError {
                                skip_frames_on_reopen,
                                ..
                            } => {
                                shared.lock().unwrap().skip_on_open = skip_frames_on_reopen;
                                let _ = ctl
                                    .events
                                    .send(InputEvent::StreamError("simulated stream error".into()));
                                return;
                            }
                            Fault::DeviceRemoved {
                                new_id, new_rate, ..
                            } => {
                                {
                                    let mut s = shared.lock().unwrap();
                                    s.default_id = new_id.clone();
                                    s.rate = new_rate;
                                    s.devices.retain(|d| d.id != "file-default");
                                    s.devices.push(DeviceInfo {
                                        id: new_id.clone(),
                                        name: new_id,
                                        is_default: true,
                                    });
                                }
                                let _ = ctl
                                    .events
                                    .send(InputEvent::StreamError("device removed".into()));
                                return;
                            }
                            Fault::DefaultDeviceChange {
                                new_id, new_rate, ..
                            } => {
                                let mut s = shared.lock().unwrap();
                                s.default_id = new_id.clone();
                                s.rate = new_rate;
                                if !s.devices.iter().any(|d| d.id == new_id) {
                                    s.devices.push(DeviceInfo {
                                        id: new_id.clone(),
                                        name: new_id.clone(),
                                        is_default: false,
                                    });
                                }
                                for d in s.devices.iter_mut() {
                                    d.is_default = d.id == new_id;
                                }
                            }
                            Fault::Stall { dur, .. } => {
                                let until = Instant::now() + dur;
                                while Instant::now() < until && !stop2.load(Ordering::Relaxed) {
                                    std::thread::sleep(Duration::from_millis(5));
                                }
                                continue;
                            }
                        }
                        continue;
                    }
                    {
                        let mut s = shared.lock().unwrap();
                        s.signal.fill(&mut mono[..n], first);
                        s.pos += n as u64;
                    }
                    for (i, v) in mono[..n].iter().enumerate() {
                        for c in 0..channels {
                            inter[i * channels + c] = *v;
                        }
                    }
                    let samples = &inter[..n * channels];
                    match mode {
                        Mode::Realtime => {
                            produced_here += n as u64;
                            let due =
                                start + Duration::from_secs_f64(produced_here as f64 / rate as f64);
                            let now = Instant::now();
                            if due > now {
                                std::thread::sleep(due - now);
                            }
                            ctl.push(samples);
                        }
                        Mode::Accelerated => {
                            // back-pressure: never drop in accelerated mode
                            while ctl.producer.slots() < samples.len()
                                && !stop2.load(Ordering::Relaxed)
                            {
                                std::thread::sleep(Duration::from_micros(200));
                            }
                            ctl.push(samples);
                        }
                    }
                }
            })
            .map_err(|e| Error::Device(e.to_string()))?;
        Ok(OpenedStream {
            sample_rate: rate,
            channels: self.channels,
            device_name: name,
            guard: Box::new(Guard { stop, th: Some(th) }),
        })
    }
}
