//! Crash-safe microphone recorder (BRIEF §8). Pipeline:
//!
//!   device callback ──(lock-free SPSC ring, copy only)──▶ writer thread ──▶ mono PCM16 WAV (fsync ~2 s)
//!                                                              ▲
//!   supervisor thread: commands, stream errors, stalls, default-device follow, sleep/wake, disk, warnings
//!
//! The recorder lives in the core: closing the window or reloading the webview never touches it.

pub mod recovery;
pub mod sidecar;

use crate::audio::resample::StreamResampler;
use crate::audio::wav::WavWriter;
use crate::error::{Error, Result};
use crate::platform::{
    AudioInput, DeviceSelector, DiskSpace, InputCtl, InputEvent, OpenedStream, PowerEvent,
    PowerGuard, SleepWakeEvents,
};
use crossbeam_channel::{bounded, select, unbounded, Receiver, Sender};
use sidecar::{DeviceSegment, Gap, GapReason, Pause, RecState, Sidecar};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct RecorderConfig {
    pub dir: PathBuf,
    pub device: DeviceSelector,
    pub id: Option<String>,
    pub sync_interval: Duration,
    pub ring_seconds: f32,
    pub stall_after: Duration,
    pub silent_after: Duration,
    pub silent_threshold_dbfs: f32,
    pub low_disk_warn_bytes: u64,
    pub low_disk_stop_bytes: u64,
    pub disk_check_interval: Duration,
    pub backoff: Vec<Duration>,
    pub default_poll_interval: Duration,
}

impl RecorderConfig {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            device: DeviceSelector::SystemDefault,
            id: None,
            sync_interval: Duration::from_secs(1),
            ring_seconds: 4.0,
            stall_after: Duration::from_secs(1),
            silent_after: Duration::from_secs(10),
            silent_threshold_dbfs: -60.0,
            low_disk_warn_bytes: 2_000_000_000,
            low_disk_stop_bytes: 300_000_000,
            disk_check_interval: Duration::from_secs(5),
            backoff: [100, 200, 400, 800, 1600, 3000]
                .iter()
                .map(|m| Duration::from_millis(*m))
                .collect(),
            default_poll_interval: Duration::from_millis(1000),
        }
    }
}

pub struct RecorderDeps {
    pub input: Arc<dyn AudioInput>,
    pub power: Arc<dyn PowerGuard>,
    pub sleep_wake: Arc<dyn SleepWakeEvents>,
    pub disk: Arc<dyn DiskSpace>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderState {
    Recording,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecorderWarning {
    DeviceChanged {
        from: String,
        to: String,
    },
    /// the pinned device disappeared; fell back to the system default
    PinnedDeviceLost {
        pinned: String,
        fallback: String,
    },
    StreamRestarted {
        reason: String,
    },
    ResumedAfterSleep {
        gap_ms: u64,
    },
    LowDisk {
        free_mb: u64,
    },
    SilentInput,
    Clipping,
    /// the core stopped the recording to protect it (file is kept)
    Stopped {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Level {
    pub rms: f32,
    pub peak: f32,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub state: RecorderState,
    pub id: String,
    pub frames: u64,
    pub elapsed_ms: u64,
    pub device: String,
    pub level: Level,
    pub overruns: u64,
    pub gaps: usize,
}

#[derive(Debug, Clone)]
pub struct RecordingInfo {
    pub id: String,
    pub wav: PathBuf,
    pub sidecar: Sidecar,
}

// --------------------------------------------------------------------------------------------- writer thread
struct WriterShared {
    frames: AtomicU64,
    rms_bits: AtomicU32,
    peak_bits: AtomicU32,
    /// ms since recording start of the last block above the silence threshold
    last_loud_ms: AtomicU64,
    clipped: AtomicBool,
    failed: Mutex<Option<String>>,
    start: Instant,
}

enum WMsg {
    Session {
        consumer: rtrb::Consumer<f32>,
        in_rate: u32,
        channels: u16,
    },
    EndSession(Sender<()>),
    Finish(Sender<Result<u64>>),
}

struct Sess {
    c: rtrb::Consumer<f32>,
    ch: usize,
    rs: Option<StreamResampler>,
}

/// Owns the WAV, level meter and scratch buffers; drains the ring of the current session.
struct Writer {
    wav: WavWriter,
    sh: Arc<WriterShared>,
    file_rate: u32,
    cur: Option<Sess>,
    mono: Vec<f32>,
    rsout: Vec<f32>,
    pcm: Vec<i16>,
    thr: f32,
    win_sq: f64,
    win_n: u64,
    win_peak: f32,
    failed: bool,
    dirty: bool,
}

impl Writer {
    fn write_pcm(&mut self) {
        if self.failed {
            return;
        }
        match self.wav.write_i16(&self.pcm) {
            Ok(()) => {
                self.sh.frames.store(self.wav.frames(), Ordering::Relaxed);
                self.dirty = true;
            }
            Err(e) => {
                self.failed = true;
                *self.sh.failed.lock().unwrap() = Some(e.to_string());
            }
        }
    }

    /// Returns true if any data was processed.
    fn drain(&mut self) -> bool {
        let Some(s) = self.cur.as_mut() else {
            return false;
        };
        let take = s.c.slots().min(16384 * s.ch);
        let take = take - take % s.ch;
        if take == 0 {
            return false;
        }
        let Ok(chunk) = s.c.read_chunk(take) else {
            return false;
        };
        let (a, b) = chunk.as_slices();
        self.mono.clear();
        let (mut acc, mut k) = (0f32, 0usize);
        for v in a.iter().chain(b.iter()) {
            acc += *v;
            k += 1;
            if k == s.ch {
                self.mono.push(acc / s.ch as f32);
                acc = 0.0;
                k = 0;
            }
        }
        chunk.commit_all();
        let (mut sq, mut pk) = (0f64, 0f32);
        for v in &self.mono {
            sq += (*v as f64) * (*v as f64);
            pk = pk.max(v.abs());
        }
        self.win_sq += sq;
        self.win_n += self.mono.len() as u64;
        self.win_peak = self.win_peak.max(pk);
        if self.win_n >= (self.file_rate as u64 / 20).max(1) {
            let rms = (self.win_sq / self.win_n as f64).sqrt() as f32;
            self.sh.rms_bits.store(rms.to_bits(), Ordering::Relaxed);
            self.sh
                .peak_bits
                .store(self.win_peak.to_bits(), Ordering::Relaxed);
            if rms > self.thr {
                self.sh.last_loud_ms.store(
                    self.sh.start.elapsed().as_millis() as u64,
                    Ordering::Relaxed,
                );
            }
            if self.win_peak >= 0.999 {
                self.sh.clipped.store(true, Ordering::Relaxed);
            }
            self.win_sq = 0.0;
            self.win_n = 0;
            self.win_peak = 0.0;
        }
        if self.failed {
            return true; // keep draining so the device side never blocks; nothing more is written
        }
        self.pcm.clear();
        if let Some(rs) = self.cur.as_mut().and_then(|s| s.rs.as_mut()) {
            self.rsout.clear();
            if rs.process(&self.mono, &mut self.rsout).is_err() {
                return true;
            }
            self.pcm.extend(
                self.rsout
                    .iter()
                    .map(|x| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16),
            );
        } else {
            self.pcm.extend(
                self.mono
                    .iter()
                    .map(|x| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16),
            );
        }
        self.write_pcm();
        true
    }

    fn end_session(&mut self) {
        while self.drain() {}
        if let Some(mut old) = self.cur.take() {
            if let Some(rs) = old.rs.as_mut() {
                self.rsout.clear();
                if rs.finish(&mut self.rsout).is_ok() {
                    self.pcm.clear();
                    self.pcm.extend(
                        self.rsout
                            .iter()
                            .map(|x| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16),
                    );
                    self.write_pcm();
                }
            }
        }
    }

    fn sync(&mut self) {
        let _ = self.wav.sync();
        self.dirty = false;
    }
}

fn writer_thread(
    wav: WavWriter,
    rx: Receiver<WMsg>,
    sh: Arc<WriterShared>,
    sync_every: Duration,
    silent_thr: f32,
) {
    let file_rate = wav.sample_rate();
    let mut w = Writer {
        wav,
        sh: sh.clone(),
        file_rate,
        cur: None,
        mono: Vec::with_capacity(16384),
        rsout: Vec::new(),
        pcm: Vec::with_capacity(16384),
        thr: 10f32.powf(silent_thr / 20.0),
        win_sq: 0.0,
        win_n: 0,
        win_peak: 0.0,
        failed: false,
        dirty: false,
    };
    let mut last_sync = Instant::now();
    loop {
        let mut msg = rx.try_recv().ok();
        if msg.is_none() && w.cur.is_none() {
            msg = rx.recv_timeout(Duration::from_millis(20)).ok();
        }
        if let Some(m) = msg {
            match m {
                WMsg::Session {
                    consumer,
                    in_rate,
                    channels,
                } => {
                    w.end_session();
                    let rs = if in_rate != file_rate {
                        StreamResampler::new(in_rate, file_rate).ok()
                    } else {
                        None
                    };
                    w.cur = Some(Sess {
                        c: consumer,
                        ch: channels.max(1) as usize,
                        rs,
                    });
                }
                WMsg::EndSession(ack) => {
                    w.end_session();
                    sh.frames.store(w.wav.frames(), Ordering::Relaxed);
                    w.sync();
                    last_sync = Instant::now();
                    let _ = ack.send(());
                }
                WMsg::Finish(reply) => {
                    w.end_session();
                    let r = w.wav.finalize();
                    let _ = reply.send(r);
                    return;
                }
            }
            continue;
        }
        if !w.drain() {
            std::thread::sleep(Duration::from_millis(3));
        }
        if w.dirty && last_sync.elapsed() >= sync_every {
            w.sync();
            last_sync = Instant::now();
        }
    }
}

// --------------------------------------------------------------------------------------------- supervisor
enum Cmd {
    Pause(Sender<Result<()>>),
    Resume(Sender<Result<()>>),
    Stop {
        discard: bool,
        reply: Sender<Result<Option<RecordingInfo>>>,
    },
}

struct Shared {
    state: Mutex<RecorderState>,
    device: Mutex<String>,
    gaps: AtomicU64,
}

pub struct RecorderHandle {
    cmd: Sender<Cmd>,
    sh: Arc<WriterShared>,
    shared: Arc<Shared>,
    dropped: Arc<AtomicU64>,
    id: String,
    dir: PathBuf,
    file_rate: u32,
    th: Option<std::thread::JoinHandle<()>>,
}

impl RecorderHandle {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn snapshot(&self) -> Snapshot {
        let frames = self.sh.frames.load(Ordering::Relaxed);
        Snapshot {
            state: *self.shared.state.lock().unwrap(),
            id: self.id.clone(),
            frames,
            elapsed_ms: frames * 1000 / self.file_rate as u64,
            device: self.shared.device.lock().unwrap().clone(),
            level: Level {
                rms: f32::from_bits(self.sh.rms_bits.load(Ordering::Relaxed)),
                peak: f32::from_bits(self.sh.peak_bits.load(Ordering::Relaxed)),
            },
            overruns: self.dropped.load(Ordering::Relaxed),
            gaps: self.shared.gaps.load(Ordering::Relaxed) as usize,
        }
    }
    fn call<T>(&self, mk: impl FnOnce(Sender<T>) -> Cmd) -> Result<T> {
        let (tx, rx) = bounded(1);
        self.cmd
            .send(mk(tx))
            .map_err(|_| Error::State("recorder already stopped".into()))?;
        rx.recv()
            .map_err(|_| Error::State("recorder thread ended".into()))
    }
    pub fn pause(&self) -> Result<()> {
        self.call(Cmd::Pause)?
    }
    pub fn resume(&self) -> Result<()> {
        self.call(Cmd::Resume)?
    }
    /// Stop, finalize the file and return its info.
    pub fn stop(mut self) -> Result<RecordingInfo> {
        let r = self.call(|reply| Cmd::Stop {
            discard: false,
            reply,
        });
        if let Some(t) = self.th.take() {
            let _ = t.join();
        }
        match r {
            Ok(inner) => inner?.ok_or_else(|| Error::State("no recording info".into())),
            // the core already stopped the recording to protect it (low disk, permission revoked…): return what is on disk
            Err(_) => {
                let json = self.dir.join(format!("{}.json", self.id));
                let sidecar = Sidecar::read(&json)?;
                Ok(RecordingInfo {
                    id: self.id.clone(),
                    wav: self.dir.join(&sidecar.file),
                    sidecar,
                })
            }
        }
    }
    /// Stop and delete the recording (the caller must have confirmed with the user).
    pub fn discard(mut self) -> Result<()> {
        self.call(|reply| Cmd::Stop {
            discard: true,
            reply,
        })??;
        if let Some(t) = self.th.take() {
            let _ = t.join();
        }
        Ok(())
    }
}

fn local_stamp() -> (String, String) {
    let now = chrono::Local::now();
    (
        format!("rec-{}", now.format("%Y%m%d-%H%M%S")),
        now.to_rfc3339(),
    )
}

struct Sup {
    cfg: RecorderConfig,
    deps: RecorderDeps,
    events: Sender<RecorderWarning>,
    sh: Arc<WriterShared>,
    shared: Arc<Shared>,
    wtx: Sender<WMsg>,
    delivered: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    in_events_tx: Sender<InputEvent>,
    stream: Option<OpenedStream>,
    current_dev_id: String,
    side: Sidecar,
    side_dir: PathBuf,
    file_rate: u32,
    pause_wall: Option<Instant>,
}

impl Sup {
    fn warn(&self, w: RecorderWarning) {
        let _ = self.events.send(w);
    }

    fn frames(&self) -> u64 {
        self.sh.frames.load(Ordering::Relaxed)
    }

    fn write_sidecar(&mut self) {
        self.side.frames_written = self.frames();
        self.side.duration_ms = self.side.frames_written * 1000 / self.file_rate as u64;
        self.side.overruns = self.dropped.load(Ordering::Relaxed);
        if let Err(e) = self.side.write(&self.side_dir) {
            tracing::warn!("sidecar write failed: {e}");
        }
    }

    fn open_stream(&mut self, sel: &DeviceSelector) -> Result<()> {
        let secs = self.cfg.ring_seconds.max(0.5);
        // ring capacity sized for the device's native rate: unknown until open, so allocate for 192 kHz x 2 ch worst case
        let cap = (secs * 192_000.0 * 2.0) as usize;
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(cap);
        let ctl = InputCtl {
            producer,
            delivered: self.delivered.clone(),
            dropped: self.dropped.clone(),
            events: self.in_events_tx.clone(),
        };
        let st = self.deps.input.open(sel, ctl)?;
        let _ = self.wtx.send(WMsg::Session {
            consumer,
            in_rate: st.sample_rate,
            channels: st.channels,
        });
        *self.shared.device.lock().unwrap() = st.device_name.clone();
        self.current_dev_id = match sel {
            DeviceSelector::Named(n) => n.clone(),
            DeviceSelector::SystemDefault => self
                .deps
                .input
                .default_device_id()
                .unwrap_or_else(|| st.device_name.clone()),
        };
        if self.side.devices.last().map(|d| (&d.name, d.sample_rate))
            != Some((&st.device_name, st.sample_rate))
        {
            self.side.devices.push(DeviceSegment {
                name: st.device_name.clone(),
                since_frame: self.frames(),
                sample_rate: st.sample_rate,
            });
        }
        self.stream = Some(st);
        Ok(())
    }

    fn close_stream(&mut self) {
        // dropping the guard stops capture; then the writer drains and ends the session
        self.stream = None;
        let (ack_tx, ack_rx) = bounded(1);
        if self.wtx.send(WMsg::EndSession(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(Duration::from_secs(10));
        }
    }

    /// Reopen after an interruption, with backoff. Returns false if a Stop arrived (or fatal error) meanwhile.
    fn reopen(
        &mut self,
        reason: GapReason,
        cmds: &Receiver<Cmd>,
        since: Instant,
        label: &str,
    ) -> ReopenResult {
        let mut sel = self.cfg.device.clone();
        let mut attempt = 0usize;
        loop {
            match self.open_stream(&sel) {
                Ok(()) => {
                    let gap_ms = since.elapsed().as_millis() as u64;
                    let at = self.frames();
                    self.side.gaps.push(Gap {
                        at_frame: at,
                        reason,
                        gap_ms,
                    });
                    self.shared.gaps.fetch_add(1, Ordering::Relaxed);
                    self.delivered_reset();
                    match reason {
                        GapReason::ResumedAfterSleep => {
                            self.warn(RecorderWarning::ResumedAfterSleep { gap_ms })
                        }
                        GapReason::DeviceChanged => {}
                        _ => self.warn(RecorderWarning::StreamRestarted {
                            reason: label.to_string(),
                        }),
                    }
                    self.write_sidecar();
                    return ReopenResult::Ok;
                }
                Err(Error::MicPermission) => {
                    return ReopenResult::Fatal("Microphone access was revoked".into())
                }
                Err(e) => {
                    if let DeviceSelector::Named(n) = &sel {
                        // pinned device vanished: fall back to the system default (BRIEF §8.2)
                        let fb = self
                            .deps
                            .input
                            .default_device_id()
                            .unwrap_or_else(|| "default".into());
                        self.warn(RecorderWarning::PinnedDeviceLost {
                            pinned: n.clone(),
                            fallback: fb,
                        });
                        sel = DeviceSelector::SystemDefault;
                        self.cfg.device = DeviceSelector::SystemDefault;
                        continue;
                    }
                    tracing::warn!("reopen attempt {attempt} failed: {e}");
                    let d = self
                        .cfg
                        .backoff
                        .get(attempt)
                        .or(self.cfg.backoff.last())
                        .copied()
                        .unwrap_or(Duration::from_secs(1));
                    attempt += 1;
                    // sleep, but react to Stop
                    match cmds.recv_timeout(d) {
                        Ok(Cmd::Stop { discard, reply }) => {
                            return ReopenResult::Stop(discard, reply)
                        }
                        Ok(Cmd::Pause(r)) | Ok(Cmd::Resume(r)) => {
                            let _ = r
                                .send(Err(Error::State("recorder is restarting its input".into())));
                        }
                        Err(_) => {}
                    }
                }
            }
        }
    }

    fn delivered_reset(&self) {
        // heartbeat baseline is tracked by the loop via last_delivered; nothing to reset here
    }

    fn finish(&mut self, state: RecState, reason: Option<String>) -> Result<RecordingInfo> {
        self.close_stream();
        let (tx, rx) = bounded(1);
        let _ = self.wtx.send(WMsg::Finish(tx));
        let frames = rx
            .recv()
            .map_err(|_| Error::State("writer ended".into()))??;
        self.side.frames_written = frames;
        self.side.state = state;
        self.side.stop_reason = reason;
        if let Some(p) = self.side.pauses.last_mut() {
            if p.end_frame.is_none() {
                p.end_frame = Some(frames);
            }
        }
        self.write_sidecar();
        *self.shared.state.lock().unwrap() = RecorderState::Stopped;
        Ok(RecordingInfo {
            id: self.side.id.clone(),
            wav: self.side_dir.join(&self.side.file),
            sidecar: self.side.clone(),
        })
    }
}

enum ReopenResult {
    Ok,
    Fatal(String),
    Stop(bool, Sender<Result<Option<RecordingInfo>>>),
}

/// Start recording. Fails fast (before creating any file) on permission/device/disk problems.
pub fn start(
    cfg: RecorderConfig,
    deps: RecorderDeps,
    events: Sender<RecorderWarning>,
) -> Result<RecorderHandle> {
    std::fs::create_dir_all(&cfg.dir).map_err(|e| Error::io(&cfg.dir, e))?;
    // pre-flight: disk
    let free = deps
        .disk
        .free_bytes(&cfg.dir)
        .map_err(|e| Error::io(&cfg.dir, e))?;
    if free < cfg.low_disk_stop_bytes {
        return Err(Error::DiskSpace {
            free_mb: free / 1_000_000,
            needed_mb: cfg.low_disk_stop_bytes / 1_000_000,
        });
    }
    if free < cfg.low_disk_warn_bytes {
        let _ = events.send(RecorderWarning::LowDisk {
            free_mb: free / 1_000_000,
        });
    }

    let power_token = deps.power.begin("Parlo is recording audio");
    let (in_tx, in_rx) = unbounded::<InputEvent>();
    let delivered = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicU64::new(0));
    let (id, started_at) = match &cfg.id {
        Some(i) => (i.clone(), chrono::Local::now().to_rfc3339()),
        None => local_stamp(),
    };
    // open the first stream *before* creating files so a denied permission leaves nothing behind
    let secs = cfg.ring_seconds.max(0.5);
    let (producer, consumer) = rtrb::RingBuffer::<f32>::new((secs * 192_000.0 * 2.0) as usize);
    let ctl = InputCtl {
        producer,
        delivered: delivered.clone(),
        dropped: dropped.clone(),
        events: in_tx.clone(),
    };
    let first = deps.input.open(&cfg.device, ctl)?;
    let file_rate = first.sample_rate;
    let wav_name = format!("{id}.wav");
    let wav_path = cfg.dir.join(&wav_name);
    let wav = WavWriter::create(&wav_path, 1, file_rate)?;
    let sh = Arc::new(WriterShared {
        frames: AtomicU64::new(0),
        rms_bits: AtomicU32::new(0),
        peak_bits: AtomicU32::new(0),
        last_loud_ms: AtomicU64::new(0),
        clipped: AtomicBool::new(false),
        failed: Mutex::new(None),
        start: Instant::now(),
    });
    let (wtx, wrx) = unbounded::<WMsg>();
    let wsh = sh.clone();
    let (sync_every, silent_thr) = (cfg.sync_interval, cfg.silent_threshold_dbfs);
    let wth = std::thread::Builder::new()
        .name("parlo-rec-writer".into())
        .spawn(move || writer_thread(wav, wrx, wsh, sync_every, silent_thr))
        .map_err(|e| Error::Other(e.to_string()))?;
    let _ = wtx.send(WMsg::Session {
        consumer,
        in_rate: first.sample_rate,
        channels: first.channels,
    });

    let shared = Arc::new(Shared {
        state: Mutex::new(RecorderState::Recording),
        device: Mutex::new(first.device_name.clone()),
        gaps: AtomicU64::new(0),
    });
    let side = Sidecar {
        version: 1,
        id: id.clone(),
        file: wav_name,
        state: RecState::Recording,
        sample_rate: file_rate,
        started_at,
        devices: vec![DeviceSegment {
            name: first.device_name.clone(),
            since_frame: 0,
            sample_rate: first.sample_rate,
        }],
        pauses: vec![],
        gaps: vec![],
        frames_written: 0,
        duration_ms: 0,
        overruns: 0,
        stop_reason: None,
        transcribed: false,
    };
    side.write(&cfg.dir)?;
    let current_dev_id = match &cfg.device {
        DeviceSelector::Named(n) => n.clone(),
        DeviceSelector::SystemDefault => deps
            .input
            .default_device_id()
            .unwrap_or_else(|| first.device_name.clone()),
    };
    let (ctx, crx) = unbounded::<Cmd>();
    let sw_rx = deps.sleep_wake.subscribe();
    let mut sup = Sup {
        cfg: cfg.clone(),
        deps,
        events,
        sh: sh.clone(),
        shared: shared.clone(),
        wtx,
        delivered: delivered.clone(),
        dropped: dropped.clone(),
        in_events_tx: in_tx,
        stream: Some(first),
        current_dev_id,
        side,
        side_dir: cfg.dir.clone(),
        file_rate,
        pause_wall: None,
    };
    let handle_id = id.clone();
    let th = std::thread::Builder::new()
        .name("parlo-rec-supervisor".into())
        .spawn(move || {
            let _power = power_token; // keeps the system awake while recording
            supervise(&mut sup, crx, in_rx, sw_rx);
            let _ = wth.join();
        })
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(RecorderHandle {
        cmd: ctx,
        sh,
        shared,
        dropped,
        id: handle_id,
        dir: cfg.dir.clone(),
        file_rate,
        th: Some(th),
    })
}

fn supervise(
    sup: &mut Sup,
    cmds: Receiver<Cmd>,
    input_events: Receiver<InputEvent>,
    sleep_wake: Receiver<PowerEvent>,
) {
    let tick = Duration::from_millis(100);
    let mut last_delivered = sup.delivered.load(Ordering::Relaxed);
    let mut last_progress = Instant::now();
    let mut last_disk = Instant::now();
    let mut last_default_poll = Instant::now();
    let mut last_sidecar = Instant::now();
    let mut warned_low_disk = false;
    let mut warned_silent = false;
    let mut last_clip_warn: Option<Instant> = None;
    let mut slept_at: Option<Instant> = None;
    let mut paused = false;

    macro_rules! stop_with {
        ($state:expr, $reason:expr) => {{
            let r = sup.finish($state, $reason);
            return_stop(sup, r);
            return;
        }};
    }

    loop {
        select! {
            recv(cmds) -> m => {
                match m {
                    Err(_) => { // handle dropped without stop: finalize safely
                        let _ = sup.finish(RecState::Finalized, Some("recorder handle dropped".into()));
                        return;
                    }
                    Ok(Cmd::Pause(reply)) => {
                        if paused { let _ = reply.send(Ok(())); continue; }
                        sup.close_stream();
                        let f = sup.frames();
                        sup.side.pauses.push(Pause { start_frame: f, end_frame: None });
                        sup.side.state = RecState::Paused;
                        sup.write_sidecar();
                        *sup.shared.state.lock().unwrap() = RecorderState::Paused;
                        sup.pause_wall = Some(Instant::now());
                        paused = true;
                        let _ = reply.send(Ok(()));
                    }
                    Ok(Cmd::Resume(reply)) => {
                        if !paused { let _ = reply.send(Ok(())); continue; }
                        let sel = sup.cfg.device.clone();
                        match sup.open_stream(&sel) {
                            Ok(()) => {
                                let f = sup.frames();
                                if let Some(p) = sup.side.pauses.last_mut() { p.end_frame = Some(f); }
                                sup.side.state = RecState::Recording;
                                sup.write_sidecar();
                                *sup.shared.state.lock().unwrap() = RecorderState::Recording;
                                paused = false;
                                last_delivered = sup.delivered.load(Ordering::Relaxed);
                                last_progress = Instant::now();
                                let _ = reply.send(Ok(()));
                            }
                            Err(e) => { let _ = reply.send(Err(e)); }
                        }
                    }
                    Ok(Cmd::Stop { discard, reply }) => {
                        let r = sup.finish(RecState::Finalized, None);
                        if discard {
                            if let Ok(info) = &r {
                                let _ = std::fs::remove_file(&info.wav);
                                let _ = std::fs::remove_file(sup.side_dir.join(format!("{}.json", info.id)));
                            }
                            let _ = reply.send(r.map(|_| None));
                        } else {
                            let _ = reply.send(r.map(Some));
                        }
                        return;
                    }
                }
            }
            recv(input_events) -> e => {
                if paused || sup.stream.is_none() { continue; }
                if let Ok(InputEvent::StreamError(msg)) = e {
                    tracing::warn!("input stream error: {msg}");
                    let since = Instant::now();
                    sup.close_stream();
                    let reason = if msg.contains("removed") { GapReason::DeviceLost } else { GapReason::StreamError };
                    match sup.reopen(reason, &cmds, since, &msg) {
                        ReopenResult::Ok => { last_delivered = sup.delivered.load(Ordering::Relaxed); last_progress = Instant::now(); }
                        ReopenResult::Fatal(r) => { sup.warn(RecorderWarning::Stopped { reason: r.clone() }); stop_with!(RecState::StoppedByError, Some(r)); }
                        ReopenResult::Stop(discard, reply) => { reply_stop(sup, discard, reply); return; }
                    }
                }
            }
            recv(sleep_wake) -> e => {
                match e {
                    Ok(PowerEvent::WillSleep) => {
                        // flush + fsync before the machine sleeps; stop the stream (CoreAudio will stop anyway)
                        slept_at = Some(Instant::now());
                        if !paused { sup.close_stream(); }
                        sup.write_sidecar();
                    }
                    Ok(PowerEvent::DidWake) => {
                        if let (Some(t), false) = (slept_at.take(), paused) {
                            match sup.reopen(GapReason::ResumedAfterSleep, &cmds, t, "resumed after sleep") {
                                ReopenResult::Ok => { last_delivered = sup.delivered.load(Ordering::Relaxed); last_progress = Instant::now(); }
                                ReopenResult::Fatal(r) => { sup.warn(RecorderWarning::Stopped { reason: r.clone() }); stop_with!(RecState::StoppedByError, Some(r)); }
                                ReopenResult::Stop(discard, reply) => { reply_stop(sup, discard, reply); return; }
                            }
                        }
                    }
                    Err(_) => {}
                }
            }
            default(tick) => {}
        }
        if paused || sup.stream.is_none() {
            continue;
        }
        let now = Instant::now();
        // writer failure (disk full / WAV size limit): stop gracefully, keep the file
        let failure = sup.sh.failed.lock().unwrap().clone();
        if let Some(msg) = failure {
            sup.warn(RecorderWarning::Stopped {
                reason: msg.clone(),
            });
            stop_with!(RecState::StoppedByError, Some(msg));
        }
        // stall detection: no samples delivered for stall_after
        let d = sup.delivered.load(Ordering::Relaxed);
        if d != last_delivered {
            last_delivered = d;
            last_progress = now;
        } else if now.duration_since(last_progress) >= sup.cfg.stall_after {
            tracing::warn!("input stalled: rebuilding stream");
            let since = last_progress;
            sup.close_stream();
            match sup.reopen(GapReason::Stall, &cmds, since, "input stalled") {
                ReopenResult::Ok => {
                    last_delivered = sup.delivered.load(Ordering::Relaxed);
                    last_progress = Instant::now();
                }
                ReopenResult::Fatal(r) => {
                    sup.warn(RecorderWarning::Stopped { reason: r.clone() });
                    stop_with!(RecState::StoppedByError, Some(r));
                }
                ReopenResult::Stop(discard, reply) => {
                    reply_stop(sup, discard, reply);
                    return;
                }
            }
        }
        // follow the system default input (AirPods in/out)
        if sup.cfg.device == DeviceSelector::SystemDefault
            && now.duration_since(last_default_poll) >= sup.cfg.default_poll_interval
        {
            last_default_poll = now;
            if let Some(cur) = sup.deps.input.default_device_id() {
                if cur != sup.current_dev_id {
                    let from = sup.current_dev_id.clone();
                    let since = Instant::now();
                    sup.close_stream();
                    match sup.reopen(
                        GapReason::DeviceChanged,
                        &cmds,
                        since,
                        "default input changed",
                    ) {
                        ReopenResult::Ok => {
                            sup.warn(RecorderWarning::DeviceChanged { from, to: cur });
                            last_delivered = sup.delivered.load(Ordering::Relaxed);
                            last_progress = Instant::now();
                        }
                        ReopenResult::Fatal(r) => {
                            sup.warn(RecorderWarning::Stopped { reason: r.clone() });
                            stop_with!(RecState::StoppedByError, Some(r));
                        }
                        ReopenResult::Stop(discard, reply) => {
                            reply_stop(sup, discard, reply);
                            return;
                        }
                    }
                }
            }
        }
        // disk space
        if now.duration_since(last_disk) >= sup.cfg.disk_check_interval {
            last_disk = now;
            if let Ok(free) = sup.deps.disk.free_bytes(&sup.side_dir) {
                if free < sup.cfg.low_disk_stop_bytes {
                    let r = format!("Disk nearly full ({} MB free)", free / 1_000_000);
                    sup.warn(RecorderWarning::Stopped { reason: r.clone() });
                    stop_with!(RecState::StoppedByError, Some(r));
                } else if free < sup.cfg.low_disk_warn_bytes && !warned_low_disk {
                    warned_low_disk = true;
                    sup.warn(RecorderWarning::LowDisk {
                        free_mb: free / 1_000_000,
                    });
                }
            }
        }
        // silent input / clipping
        let since_loud = sup.sh.start.elapsed().as_millis() as u64
            - sup
                .sh
                .last_loud_ms
                .load(Ordering::Relaxed)
                .min(sup.sh.start.elapsed().as_millis() as u64);
        if since_loud as u128 >= sup.cfg.silent_after.as_millis() && !warned_silent {
            warned_silent = true;
            sup.warn(RecorderWarning::SilentInput);
        } else if (since_loud as u128) < 2000 {
            warned_silent = false;
        }
        if sup.sh.clipped.swap(false, Ordering::Relaxed)
            && last_clip_warn.is_none_or(|t| now.duration_since(t) > Duration::from_secs(5))
        {
            last_clip_warn = Some(now);
            sup.warn(RecorderWarning::Clipping);
        }
        if now.duration_since(last_sidecar) >= Duration::from_secs(5) {
            last_sidecar = now;
            sup.write_sidecar();
        }
        let _ = slept_at.is_some();
    }
}

fn reply_stop(sup: &mut Sup, discard: bool, reply: Sender<Result<Option<RecordingInfo>>>) {
    let r = sup.finish(RecState::Finalized, None);
    if discard {
        if let Ok(info) = &r {
            let _ = std::fs::remove_file(&info.wav);
            let _ = std::fs::remove_file(sup.side_dir.join(format!("{}.json", info.id)));
        }
        let _ = reply.send(r.map(|_| None));
    } else {
        let _ = reply.send(r.map(Some));
    }
}

/// After a core-initiated stop the supervisor exits; the finalized info stays in the sidecar on disk and the shared
/// state flips to `Stopped` so `RecorderHandle::snapshot()` (and `stop()`, which then fails with a clear message) reflect it.
fn return_stop(_sup: &mut Sup, r: Result<RecordingInfo>) {
    if let Err(e) = r {
        tracing::error!("finalize after protective stop failed: {e}");
    }
}
