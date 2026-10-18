//! Running a downloader as a child process. **All** networking happens in these children — Parlo itself never opens a
//! socket (DECISIONS C-021). The child gets its own process group so cancel (and quit) can kill it *and* anything it
//! spawned (yt-dlp → ffmpeg, a JS runtime, …).

use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Process groups of children that are running right now (so quitting the app can kill them).
fn live_groups() -> &'static Mutex<HashSet<i32>> {
    static G: OnceLock<Mutex<HashSet<i32>>> = OnceLock::new();
    G.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Kill every running downloader child (called when the app exits).
pub fn kill_all_children() {
    let groups: Vec<i32> = live_groups().lock().unwrap().drain().collect();
    for g in groups {
        kill_group(g, libc::SIGKILL);
    }
}

fn kill_group(pgid: i32, sig: i32) {
    if pgid > 1 {
        // SAFETY: plain syscall on a process group id we created (the child is its own group leader)
        unsafe {
            libc::killpg(pgid, sig);
        }
    }
}

#[derive(Debug)]
pub enum RunError {
    Spawn(std::io::Error),
    Cancelled,
    TimedOut,
}

#[derive(Debug)]
pub struct RunResult {
    pub status: ExitStatus,
    /// last stderr lines (newest last)
    pub stderr_tail: Vec<String>,
}

impl RunResult {
    pub fn stderr_text(&self) -> String {
        self.stderr_tail.join("\n")
    }
}

const STDERR_KEEP: usize = 60;

enum Msg {
    Out(String),
    OutEof,
}

/// Split a byte stream into lines on `\n` **or** `\r` (curl's progress meter redraws with `\r`).
fn read_lines(mut r: impl Read, mut f: impl FnMut(String)) {
    let mut buf = [0u8; 8192];
    let mut cur: Vec<u8> = Vec::new();
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !cur.is_empty() {
                            f(String::from_utf8_lossy(&cur).into_owned());
                            cur.clear();
                        }
                    } else {
                        cur.push(b);
                    }
                }
            }
        }
    }
    if !cur.is_empty() {
        f(String::from_utf8_lossy(&cur).into_owned());
    }
}

/// Which stream carries the lines the caller wants to see live.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Live {
    Stdout,
    Stderr,
}

/// Run `cmd`, calling `on_line` for every line of the `live` stream as it arrives; the other stream is kept as a tail.
/// Blocks until the child exits, `cancel` is set (child group killed → `Cancelled`) or `timeout` passes (→ `TimedOut`).
pub fn run_streaming(
    mut cmd: Command,
    live: Live,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
    on_line: &mut dyn FnMut(&str),
) -> Result<RunResult, RunError> {
    use std::os::unix::process::CommandExt;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    #[cfg(target_os = "linux")]
    // SAFETY: only async-signal-safe prctl between fork and exec — the child dies with the thread that spawned it
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(RunError::Cancelled);
    }
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
    let pgid = child.id() as i32;
    live_groups().lock().unwrap().insert(pgid);

    let (tx, rx) = mpsc::channel::<Msg>();
    let tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
    let (live_stream, quiet_stream): (Box<dyn Read + Send>, Box<dyn Read + Send>) = {
        let o = child.stdout.take().expect("piped stdout");
        let e = child.stderr.take().expect("piped stderr");
        if live == Live::Stdout {
            (Box::new(o), Box::new(e))
        } else {
            (Box::new(e), Box::new(o))
        }
    };
    let tx2 = tx.clone();
    let t_live = std::thread::spawn(move || {
        read_lines(live_stream, |l| {
            let _ = tx2.send(Msg::Out(l));
        });
        let _ = tx2.send(Msg::OutEof);
    });
    let tail2 = tail.clone();
    let t_quiet = std::thread::spawn(move || {
        read_lines(quiet_stream, |l| {
            let mut t = tail2.lock().unwrap();
            if t.len() >= STDERR_KEEP {
                t.pop_front();
            }
            t.push_back(l);
        });
    });
    drop(tx);

    let started = Instant::now();
    let mut outcome: Option<RunError> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Msg::Out(l)) => {
                if live == Live::Stderr {
                    // keep the live stream's lines in the tail as well (errors are usually in there)
                    let mut t = tail.lock().unwrap();
                    if t.len() >= STDERR_KEEP {
                        t.pop_front();
                    }
                    t.push_back(l.clone());
                }
                on_line(&l)
            }
            Ok(Msg::OutEof) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if cancel.load(Ordering::SeqCst) {
            outcome = Some(RunError::Cancelled);
            break;
        }
        if timeout.is_some_and(|t| started.elapsed() > t) {
            outcome = Some(RunError::TimedOut);
            break;
        }
    }
    if outcome.is_some() {
        terminate(&mut child, pgid);
    }
    let status = child.wait();
    live_groups().lock().unwrap().remove(&pgid);
    // anything the child left behind in its group (grandchildren holding the pipes open) goes too
    kill_group(pgid, libc::SIGKILL);
    let _ = t_live.join();
    let _ = t_quiet.join();
    if let Some(e) = outcome {
        return Err(e);
    }
    let status = status.map_err(RunError::Spawn)?;
    let stderr_tail = tail.lock().unwrap().iter().cloned().collect();
    Ok(RunResult {
        status,
        stderr_tail,
    })
}

/// SIGTERM the whole group, give it a moment, then SIGKILL.
fn terminate(child: &mut std::process::Child, pgid: i32) {
    kill_group(pgid, libc::SIGTERM);
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            kill_group(pgid, libc::SIGKILL); // stragglers
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    kill_group(pgid, libc::SIGKILL);
}

/// Run to completion, collecting stdout into a string (probes, `--version`). Not for long transfers.
pub fn run_capture(
    cmd: Command,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<(RunResult, String), RunError> {
    let mut out = String::new();
    let r = run_streaming(cmd, Live::Stdout, cancel, Some(timeout), &mut |l| {
        out.push_str(l);
        out.push('\n');
    })?;
    Ok((r, out))
}

/// Build a command with a clean, predictable environment for a downloader: nothing of the user's yt-dlp config is read
/// (callers pass `--no-config`), temp files go under Parlo's cache, locale is UTF-8.
pub fn command(program: &Path, args: &[OsString], tmp_dir: Option<&Path>) -> Command {
    let mut c = Command::new(program);
    c.args(args);
    c.env("LC_ALL", "C.UTF-8")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .env_remove("YTDLP_NO_UPDATE");
    if let Some(t) = tmp_dir {
        let _ = std::fs::create_dir_all(t);
        c.env("TMPDIR", t);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    #[test]
    fn streams_lines_and_keeps_stderr_tail() {
        let cancel = AtomicBool::new(false);
        let mut seen = vec![];
        let r = run_streaming(
            sh("echo one; echo two >&2; printf 'a\\rb\\n'; exit 3"),
            Live::Stdout,
            &cancel,
            None,
            &mut |l| seen.push(l.to_string()),
        )
        .unwrap();
        assert_eq!(seen, vec!["one", "a", "b"]);
        assert_eq!(r.status.code(), Some(3));
        assert_eq!(r.stderr_tail, vec!["two"]);
    }

    #[test]
    fn cancel_kills_the_whole_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        // a shell that starts a grandchild (sleep) and then waits: killing only the shell would leave `sleep` running
        let script = format!(
            "sleep 60 & echo $! > {}; echo started; wait",
            pidfile.display()
        );
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            c2.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let r = run_streaming(sh(&script), Live::Stdout, &cancel, None, &mut |_| {});
        t.join().unwrap();
        assert!(matches!(r, Err(RunError::Cancelled)));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancel must be quick"
        );
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        // SAFETY: signal 0 only probes for existence
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        // a zombie would still answer kill(0); reap check via /proc on Linux
        let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| s.contains(") Z"))
            .unwrap_or(false);
        assert!(!alive || zombie, "grandchild {pid} survived the cancel");
    }

    #[test]
    fn timeout_kills_the_child() {
        let cancel = AtomicBool::new(false);
        let r = run_streaming(
            sh("sleep 30"),
            Live::Stdout,
            &cancel,
            Some(Duration::from_millis(300)),
            &mut |_| {},
        );
        assert!(matches!(r, Err(RunError::TimedOut)));
    }

    #[test]
    fn spawn_failure_is_reported() {
        let cancel = AtomicBool::new(false);
        let r = run_streaming(
            Command::new("/definitely/not/here"),
            Live::Stdout,
            &cancel,
            None,
            &mut |_| {},
        );
        assert!(matches!(r, Err(RunError::Spawn(_))));
    }

    #[test]
    fn already_cancelled_never_spawns() {
        let cancel = AtomicBool::new(true);
        let r = run_streaming(sh("echo hi"), Live::Stdout, &cancel, None, &mut |_| {});
        assert!(matches!(r, Err(RunError::Cancelled)));
    }
}
