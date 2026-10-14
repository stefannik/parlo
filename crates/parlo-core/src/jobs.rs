//! Job manager (BRIEF §10): one job per engine at a time, FIFO queue with visible state, cancel, pause on critical
//! memory pressure. Jobs are closures that acquire their engine from a model slot themselves.

use crate::error::{Error, Result};
use crate::models::EngineKind;
use crate::platform::PowerGuard;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

pub type JobId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: JobId,
    pub engine: EngineKind,
    pub label: String,
    pub state: JobState,
}

pub struct JobCtx {
    pub id: JobId,
    pub cancel: Arc<AtomicBool>,
}

type JobFn = Box<dyn FnOnce(&JobCtx) -> Result<()> + Send>;

struct Queued {
    id: JobId,
    f: JobFn,
    cancel: Arc<AtomicBool>,
}

struct Shared {
    queues: Mutex<HashMap<EngineKind, VecDeque<Queued>>>,
    infos: Mutex<HashMap<JobId, (JobInfo, Arc<AtomicBool>)>>,
    cv: Condvar,
    paused: AtomicBool,
    shutdown: AtomicBool,
    running: AtomicU64,
    on_change: Arc<dyn Fn(&JobInfo) + Send + Sync>,
    power: Arc<dyn PowerGuard>,
}

pub struct JobManager {
    sh: Arc<Shared>,
    next: AtomicU64,
    _workers: Vec<std::thread::JoinHandle<()>>,
}

impl JobManager {
    pub fn new(power: Arc<dyn PowerGuard>, on_change: Arc<dyn Fn(&JobInfo) + Send + Sync>) -> Self {
        let sh = Arc::new(Shared {
            queues: Mutex::new(HashMap::new()),
            infos: Mutex::new(HashMap::new()),
            cv: Condvar::new(),
            paused: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            running: AtomicU64::new(0),
            on_change,
            power,
        });
        let mut workers = Vec::new();
        for kind in [EngineKind::Stt, EngineKind::Tts] {
            let sh2 = sh.clone();
            workers.push(
                std::thread::Builder::new()
                    .name(format!("parlo-jobs-{kind:?}"))
                    .spawn(move || worker(sh2, kind))
                    .expect("spawn job worker"),
            );
        }
        Self {
            sh,
            next: AtomicU64::new(1),
            _workers: workers,
        }
    }

    pub fn submit(
        &self,
        engine: EngineKind,
        label: &str,
        f: impl FnOnce(&JobCtx) -> Result<()> + Send + 'static,
    ) -> JobId {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let cancel = Arc::new(AtomicBool::new(false));
        let info = JobInfo {
            id,
            engine,
            label: label.to_string(),
            state: JobState::Queued,
        };
        self.sh
            .infos
            .lock()
            .unwrap()
            .insert(id, (info.clone(), cancel.clone()));
        self.sh
            .queues
            .lock()
            .unwrap()
            .entry(engine)
            .or_default()
            .push_back(Queued {
                id,
                f: Box::new(f),
                cancel,
            });
        (self.sh.on_change)(&info);
        self.sh.cv.notify_all();
        id
    }

    /// Cancel a queued or running job (the running job must observe `ctx.cancel`).
    pub fn cancel(&self, id: JobId) {
        let flag = self.sh.infos.lock().unwrap().get(&id).map(|x| x.1.clone());
        if let Some(f) = flag {
            f.store(true, Ordering::SeqCst);
        }
        // remove from the queue if it hasn't started
        let mut q = self.sh.queues.lock().unwrap();
        for dq in q.values_mut() {
            if let Some(pos) = dq.iter().position(|j| j.id == id) {
                dq.remove(pos);
                drop(q);
                set_state(&self.sh, id, JobState::Cancelled);
                return;
            }
        }
    }

    pub fn info(&self, id: JobId) -> Option<JobInfo> {
        self.sh.infos.lock().unwrap().get(&id).map(|x| x.0.clone())
    }

    pub fn list(&self) -> Vec<JobInfo> {
        let mut v: Vec<JobInfo> = self
            .sh
            .infos
            .lock()
            .unwrap()
            .values()
            .map(|x| x.0.clone())
            .collect();
        v.sort_by_key(|j| j.id);
        v
    }

    /// Critical memory pressure: don't start queued jobs until resumed.
    pub fn set_paused(&self, p: bool) {
        self.sh.paused.store(p, Ordering::SeqCst);
        self.sh.cv.notify_all();
    }

    pub fn any_running(&self) -> bool {
        self.sh.running.load(Ordering::SeqCst) > 0
    }

    /// Block until the job reaches a final state (tests, CLI).
    pub fn wait(&self, id: JobId) -> JobState {
        loop {
            match self.info(id).map(|i| i.state) {
                Some(JobState::Queued) | Some(JobState::Running) | None => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                Some(s) => return s,
            }
        }
    }
}

impl Drop for JobManager {
    fn drop(&mut self) {
        self.sh.shutdown.store(true, Ordering::SeqCst);
        for (_, flag) in self.sh.infos.lock().unwrap().values() {
            flag.store(true, Ordering::SeqCst);
        }
        self.sh.cv.notify_all();
    }
}

fn set_state(sh: &Shared, id: JobId, st: JobState) {
    let info = {
        let mut m = sh.infos.lock().unwrap();
        m.get_mut(&id).map(|x| {
            x.0.state = st;
            x.0.clone()
        })
    };
    if let Some(i) = info {
        (sh.on_change)(&i);
    }
}

fn worker(sh: Arc<Shared>, kind: EngineKind) {
    loop {
        let job = {
            let mut q = sh.queues.lock().unwrap();
            loop {
                if sh.shutdown.load(Ordering::SeqCst) {
                    return;
                }
                if !sh.paused.load(Ordering::SeqCst) {
                    if let Some(j) = q.get_mut(&kind).and_then(|d| d.pop_front()) {
                        break j;
                    }
                }
                q = sh.cv.wait(q).unwrap();
            }
        };
        set_state(&sh, job.id, JobState::Running);
        sh.running.fetch_add(1, Ordering::SeqCst);
        let _power = sh.power.begin("Parlo is working on a job");
        let ctx = JobCtx {
            id: job.id,
            cancel: job.cancel,
        };
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (job.f)(&ctx)));
        sh.running.fetch_sub(1, Ordering::SeqCst);
        let st = match res {
            Ok(Ok(())) => JobState::Done,
            Ok(Err(Error::Cancelled)) => JobState::Cancelled,
            Ok(Err(e)) => JobState::Failed(e.to_string()),
            Err(_) => JobState::Failed("The job crashed unexpectedly.".into()),
        };
        set_state(&sh, job.id, st);
    }
}
