// Prevents an extra console window on Windows in release; harmless elsewhere.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// macOS: libmalloc keeps freed large blocks (model weights) resident in its large-block cache and
/// `malloc_zone_pressure_relief` does not release them — measured +520 MB after a TTS job, which breaks the BRIEF §10 /
/// §12.1 "back to baseline +30 MB after the idle unload" rule. `MallocLargeCache=0` (read by libmalloc at its first
/// allocation) disables that cache (DECISIONS C-017). LaunchServices launches get it from Info.plist `LSEnvironment`;
/// any other launch (terminal, tests) re-executes itself once with the variable set (same PID).
#[cfg(target_os = "macos")]
fn ensure_malloc_env() {
    use std::os::unix::process::CommandExt;
    if std::env::var_os("MallocLargeCache").is_some() {
        return;
    }
    std::env::set_var("MallocLargeCache", "0");
    if let Ok(exe) = std::env::current_exe() {
        let err = std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .exec();
        eprintln!("could not re-exec with MallocLargeCache=0: {err}");
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    ensure_malloc_env();
    parlo_app_lib::run()
}
