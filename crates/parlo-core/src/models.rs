//! Model lifecycle (BRIEF §10): nothing loads at startup; engines load on first use and unload after
//! `model_idle_timeout` via ONE re-armable timer thread (no polling), or immediately on memory pressure.
//! After an unload the heap is returned to the OS (`MemoryRelief`).

use crate::error::Result;
use crate::platform::{MemoryPressure, MemoryPressureLevel, MemoryRelief};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineKind {
    Stt,
    Tts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelState {
    Unloaded,
    Loading,
    Ready,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelsEvent {
    pub engine: EngineKind,
    pub state: ModelState,
    /// milliseconds until the idle unload fires (only while Ready and idle)
    pub unload_in_ms: Option<u64>,
}

pub type EventFn = Arc<dyn Fn(ModelsEvent) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleTimeout {
    Immediately,
    After(Duration),
}

impl IdleTimeout {
    pub const DEFAULT: IdleTimeout = IdleTimeout::After(Duration::from_secs(60));
}

// ------------------------------------------------------------------------------------------------ the timer
struct TimerState {
    deadlines: HashMap<EngineKind, Instant>,
    shutdown: bool,
}

/// One thread, one condvar: sleeps until the earliest deadline (or forever if none), so an idle app has zero wakeups.
struct Reaper {
    st: Arc<(Mutex<TimerState>, Condvar)>,
}

impl Reaper {
    fn spawn(on_expire: Arc<dyn Fn(EngineKind) + Send + Sync>) -> Self {
        let st = Arc::new((
            Mutex::new(TimerState {
                deadlines: HashMap::new(),
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let st2 = st.clone();
        std::thread::Builder::new()
            .name("parlo-model-reaper".into())
            .spawn(move || {
                let (lock, cv) = &*st2;
                let mut g = lock.lock().unwrap();
                loop {
                    if g.shutdown {
                        return;
                    }
                    let now = Instant::now();
                    let due: Vec<EngineKind> = g
                        .deadlines
                        .iter()
                        .filter(|(_, d)| **d <= now)
                        .map(|(k, _)| *k)
                        .collect();
                    for k in &due {
                        g.deadlines.remove(k);
                    }
                    if !due.is_empty() {
                        drop(g);
                        for k in due {
                            on_expire(k);
                        }
                        g = lock.lock().unwrap();
                        continue;
                    }
                    match g.deadlines.values().min().copied() {
                        Some(next) => {
                            g = cv
                                .wait_timeout(g, next.saturating_duration_since(now))
                                .unwrap()
                                .0;
                        }
                        None => g = cv.wait(g).unwrap(),
                    }
                }
            })
            .expect("spawn reaper");
        Self { st }
    }
    fn arm(&self, k: EngineKind, at: Instant) {
        let (l, cv) = &*self.st;
        l.lock().unwrap().deadlines.insert(k, at);
        cv.notify_all();
    }
    fn disarm(&self, k: EngineKind) {
        let (l, cv) = &*self.st;
        l.lock().unwrap().deadlines.remove(&k);
        cv.notify_all();
    }
    fn remaining(&self, k: EngineKind) -> Option<Duration> {
        self.st
            .0
            .lock()
            .unwrap()
            .deadlines
            .get(&k)
            .map(|d| d.saturating_duration_since(Instant::now()))
    }
}

impl Drop for Reaper {
    fn drop(&mut self) {
        let (l, cv) = &*self.st;
        l.lock().unwrap().shutdown = true;
        cv.notify_all();
    }
}

// ------------------------------------------------------------------------------------------------ slots
type Loader<E> = Box<dyn Fn() -> Result<E> + Send + Sync>;

struct SlotInner<E> {
    engine: Option<E>,
    /// number of active users (jobs); idle timer only runs at 0
    users: usize,
}

pub struct Slot<E: Send + 'static> {
    kind: EngineKind,
    loader: Loader<E>,
    inner: Mutex<SlotInner<E>>,
    /// serialises *use* of the engine (one job per engine at a time)
    use_lock: Mutex<()>,
    host: Arc<HostShared>,
}

struct HostShared {
    reaper: Mutex<Option<Reaper>>,
    timeout: Mutex<IdleTimeout>,
    events: EventFn,
    unloaders: Mutex<HashMap<EngineKind, Arc<dyn Fn() + Send + Sync>>>,
}

impl HostShared {
    fn emit(&self, engine: EngineKind, state: ModelState) {
        let unload = if state == ModelState::Ready {
            self.reaper
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|r| r.remaining(engine))
                .map(|d| d.as_millis() as u64)
        } else {
            None
        };
        (self.events)(ModelsEvent {
            engine,
            state,
            unload_in_ms: unload,
        });
    }
}

/// A loaded engine checked out for exclusive use. Dropping it arms the idle timer.
pub struct EngineGuard<'a, E: Send + 'static> {
    slot: &'a Slot<E>,
    _use: MutexGuard<'a, ()>,
    engine: Option<E>,
}

impl<E: Send + 'static> std::ops::Deref for EngineGuard<'_, E> {
    type Target = E;
    fn deref(&self) -> &E {
        self.engine.as_ref().unwrap()
    }
}
impl<E: Send + 'static> std::ops::DerefMut for EngineGuard<'_, E> {
    fn deref_mut(&mut self) -> &mut E {
        self.engine.as_mut().unwrap()
    }
}
impl<E: Send + 'static> Drop for EngineGuard<'_, E> {
    fn drop(&mut self) {
        let mut g = self.slot.inner.lock().unwrap();
        g.engine = self.engine.take();
        g.users = g.users.saturating_sub(1);
        let idle = g.users == 0;
        drop(g);
        if idle {
            self.slot.on_idle();
        }
    }
}

impl<E: Send + 'static> Slot<E> {
    pub fn is_loaded(&self) -> bool {
        self.inner.lock().unwrap().engine.is_some()
    }

    /// Check the engine out for a job, loading it first if necessary (state events: Loading → Ready).
    pub fn acquire(&self) -> Result<EngineGuard<'_, E>> {
        let use_guard = self.use_lock.lock().unwrap();
        if let Some(r) = self.host.reaper.lock().unwrap().as_ref() {
            r.disarm(self.kind);
        }
        let mut g = self.inner.lock().unwrap();
        g.users += 1;
        let engine = match g.engine.take() {
            Some(e) => e,
            None => {
                drop(g);
                self.host.emit(self.kind, ModelState::Loading);
                let t = Instant::now();
                let loaded = (self.loader)();
                match loaded {
                    Ok(e) => {
                        tracing::info!(
                            "{:?} model loaded in {:.2}s",
                            self.kind,
                            t.elapsed().as_secs_f64()
                        );
                        self.host.emit(self.kind, ModelState::Ready);
                        self.inner.lock().unwrap().users = 1;
                        e
                    }
                    Err(err) => {
                        self.inner.lock().unwrap().users = 0;
                        self.host.emit(self.kind, ModelState::Unloaded);
                        return Err(err);
                    }
                }
            }
        };
        Ok(EngineGuard {
            slot: self,
            _use: use_guard,
            engine: Some(engine),
        })
    }

    fn on_idle(&self) {
        let timeout = *self.host.timeout.lock().unwrap();
        match timeout {
            IdleTimeout::Immediately => {
                self.unload();
            }
            IdleTimeout::After(d) => {
                if let Some(r) = self.host.reaper.lock().unwrap().as_ref() {
                    r.arm(self.kind, Instant::now() + d);
                }
                self.host.emit(self.kind, ModelState::Ready);
            }
        }
    }

    /// Unload now if nobody is using it. Returns true if memory was released.
    pub fn unload(&self) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.users > 0 || g.engine.is_none() {
            return false;
        }
        let e = g.engine.take();
        drop(g);
        drop(e);
        MemoryRelief::relieve();
        self.host.emit(self.kind, ModelState::Unloaded);
        true
    }
}

/// Owns the timer and (optionally) the memory-pressure subscription for all slots.
pub struct ModelHost {
    shared: Arc<HostShared>,
    stop: Arc<AtomicBool>,
}

impl ModelHost {
    pub fn new(timeout: IdleTimeout, events: EventFn) -> Arc<Self> {
        let shared = Arc::new(HostShared {
            reaper: Mutex::new(None),
            timeout: Mutex::new(timeout),
            events,
            unloaders: Mutex::new(HashMap::new()),
        });
        let weak = Arc::downgrade(&shared);
        let reaper = Reaper::spawn(Arc::new(move |k| {
            if let Some(sh) = weak.upgrade() {
                let f = sh.unloaders.lock().unwrap().get(&k).cloned();
                if let Some(f) = f {
                    f();
                }
            }
        }));
        *shared.reaper.lock().unwrap() = Some(reaper);
        Arc::new(Self {
            shared,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn slot<E: Send + 'static>(
        &self,
        kind: EngineKind,
        loader: impl Fn() -> Result<E> + Send + Sync + 'static,
    ) -> Arc<Slot<E>> {
        let slot = Arc::new(Slot {
            kind,
            loader: Box::new(loader),
            inner: Mutex::new(SlotInner {
                engine: None,
                users: 0,
            }),
            use_lock: Mutex::new(()),
            host: self.shared.clone(),
        });
        let weak = Arc::downgrade(&slot);
        self.shared.unloaders.lock().unwrap().insert(
            kind,
            Arc::new(move || {
                if let Some(s) = weak.upgrade() {
                    s.unload();
                }
            }),
        );
        slot
    }

    pub fn set_idle_timeout(&self, t: IdleTimeout) {
        *self.shared.timeout.lock().unwrap() = t;
    }

    /// Unload every idle engine right now (memory warning, manual "free memory").
    pub fn unload_all_idle(&self) {
        let fs: Vec<_> = self
            .shared
            .unloaders
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        for f in fs {
            f();
        }
    }

    pub fn idle_unload_in(&self, kind: EngineKind) -> Option<Duration> {
        self.shared
            .reaper
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|r| r.remaining(kind))
    }

    /// Subscribe to memory pressure: Warning → unload idle engines at once; Critical → same, and `on_critical(true)` so
    /// the caller can pause queued jobs (`on_critical(false)` when back to Normal).
    pub fn watch_memory_pressure(
        self: &Arc<Self>,
        mp: &dyn MemoryPressure,
        on_critical: impl Fn(bool) + Send + 'static,
    ) {
        let rx = mp.subscribe();
        let me = Arc::downgrade(self);
        let stop = self.stop.clone();
        std::thread::Builder::new()
            .name("parlo-mem-pressure".into())
            .spawn(move || {
                while let Ok(level) = rx.recv() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Some(host) = me.upgrade() else { return };
                    match level {
                        MemoryPressureLevel::Normal => on_critical(false),
                        MemoryPressureLevel::Warning => host.unload_all_idle(),
                        MemoryPressureLevel::Critical => {
                            host.unload_all_idle();
                            on_critical(true);
                        }
                    }
                }
            })
            .expect("spawn pressure watcher");
    }
}

impl Drop for ModelHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
