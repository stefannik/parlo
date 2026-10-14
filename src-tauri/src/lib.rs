//! Parlo app shell (Tauri v2): commands, events, window lifecycle, menus, single instance.
//! Closing the window destroys the webview (frees WebKit memory); recording and jobs live in the core and continue.

mod commands;
mod ipc;
mod native;
mod state;

use ipc::*;
use state::AppState;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_specta::{collect_commands, collect_events, Builder, Event};

pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::app_info,
            commands::settings_get,
            commands::settings_update,
            commands::mic_permission_status,
            commands::mic_permission_request,
            commands::open_privacy_settings,
            commands::input_devices_list,
            commands::recording_start,
            commands::recording_pause,
            commands::recording_resume,
            commands::recording_stop,
            commands::recording_discard,
            commands::recoverables_list,
            commands::recoverable_discard,
            commands::transcribe_start,
            commands::stt_resume,
            commands::job_cancel,
            commands::tts_start,
            commands::tts_resume,
            commands::voices_list,
            commands::text_import,
            commands::text_import_full,
            commands::export_tts,
            commands::export_transcript,
            commands::export_recording,
            commands::workspace_get,
            commands::workspace_update,
            commands::storage_usage,
            commands::storage_clear,
            commands::reveal_in_finder,
            commands::ui_visibility,
            commands::diagnostics_text,
            commands::quit_confirmed,
        ])
        .events(collect_events![RecorderStateEvent, RecorderWarningEvent, ModelsStateEvent, JobChangedEvent, MenuActionEvent, QuitRequestedEvent])
}

pub const BINDINGS_PATH: &str = "../ui/src/bindings.ts";

/// (Re)create the single main window if it doesn't exist; focus it otherwise.
pub fn ensure_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    #[allow(unused_mut)]
    let mut b = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into())).title("Parlo").inner_size(960.0, 700.0).min_inner_size(720.0, 560.0).resizable(true).center();
    #[cfg(target_os = "macos")]
    {
        b = b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    }
    match b.build() {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => tracing::error!("could not create the main window: {e}"),
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let item = |id: &str, label: &str, accel: Option<&str>| MenuItem::with_id(app, id, label, true, accel);
    let app_menu = Submenu::with_items(
        app,
        "Parlo",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Parlo"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &item("settings", "Settings…", Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Quit Parlo", Some("CmdOrCtrl+Q"))?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &item("open", "Open…", Some("CmdOrCtrl+O"))?,
            &PredefinedMenuItem::separator(app)?,
            &item("record", "Record / Stop", Some("CmdOrCtrl+R"))?,
            &item("pause", "Pause / Resume Recording", Some("CmdOrCtrl+P"))?,
            &PredefinedMenuItem::separator(app)?,
            &item("export", "Export…", Some("CmdOrCtrl+E"))?,
            &item("copy_transcript", "Copy Transcript", Some("CmdOrCtrl+Shift+C"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[&item("tab_transcribe", "Transcribe", Some("CmdOrCtrl+1"))?, &item("tab_speak", "Speak", Some("CmdOrCtrl+2"))?, &PredefinedMenuItem::separator(app)?, &PredefinedMenuItem::fullscreen(app, None)?],
    )?;
    let window = Submenu::with_items(app, "Window", true, &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::maximize(app, None)?])?;
    let help = Submenu::with_items(app, "Help", true, &[&item("generate", "Generate Speech", Some("CmdOrCtrl+Enter"))?])?;
    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window, &help])
}

/// Quit while recording asks first (BRIEF §8.2); otherwise quit right away.
fn request_quit(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let recording = state.recorder.lock().unwrap().as_ref().is_some_and(|h| h.snapshot().state != parlo_core::recorder::RecorderState::Stopped);
    let job_running = state.jobs.any_running();
    if !recording {
        state.quitting.store(true, Ordering::SeqCst);
        app.exit(0);
        return;
    }
    if app.get_webview_window("main").is_some() {
        let _ = QuitRequestedEvent { recording, job_running }.emit(app);
        return;
    }
    // no window: native confirmation
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let app2 = app.clone();
    app.dialog()
        .message("A recording is in progress. Stop and save it before quitting?")
        .title("Parlo")
        .buttons(MessageDialogButtons::OkCancelCustom("Stop and save".into(), "Cancel".into()))
        .show(move |stop| {
            if stop {
                let state = app2.state::<Arc<AppState>>();
                if let Some(h) = state.recorder.lock().unwrap().take() {
                    let _ = h.stop();
                }
                state.quitting.store(true, Ordering::SeqCst);
                app2.exit(0);
            }
        });
}

/// One thread, parked on a condvar: ticks at 20 Hz only while recording AND the UI is visible (BRIEF §5.4, §11.1).
fn spawn_level_ticker(app: AppHandle, state: Arc<AppState>) {
    std::thread::Builder::new()
        .name("parlo-level-ticker".into())
        .spawn(move || {
            let (m, cv) = &*state.ticker;
            loop {
                let mut g = m.lock().unwrap();
                while !(g.recording && g.visible) {
                    g = cv.wait(g).unwrap();
                }
                drop(g);
                commands::emit_recorder_state(&app, &state);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        })
        .expect("spawn ticker");
}

pub fn run() {
    let builder = specta_builder();
    #[cfg(debug_assertions)]
    if let Err(e) = builder.export(specta_typescript::Typescript::default(), BINDINGS_PATH) {
        eprintln!("could not export TypeScript bindings: {e}");
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // a second launch just brings the existing instance forward (recreating the window if needed)
            ensure_main_window(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let handle = app.handle().clone();
            let paths = parlo_core::paths::AppPaths::discover();
            let models = parlo_core::paths::ModelPaths::discover();
            // local rotating logs (5 × 2 MB), never uploaded
            let log = parlo_core::logging::RotatingFile::standard(&paths.logs);
            let _ = tracing_subscriber::fmt().with_writer(move || log.handle()).with_ansi(false).with_max_level(tracing::Level::INFO).try_init();
            let state = AppState::new(&handle, paths.clone(), models);
            // retention / cache policy at launch (never touches untranscribed recordings or the current output)
            let (settings, ws) = (state.settings.lock().unwrap().clone(), state.workspace.lock().unwrap().clone());
            if let Err(e) = parlo_core::store::cleanup(&paths, &settings, &ws, std::time::SystemTime::now()) {
                tracing::warn!("cleanup failed: {e}");
            }
            app.manage(state.clone());
            spawn_level_ticker(handle.clone(), state);
            let menu = build_menu(&handle)?;
            app.set_menu(menu)?;
            ensure_main_window(&handle);
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            if id == "quit" {
                request_quit(app);
            } else {
                let _ = MenuActionEvent { action: id }.emit(app);
            }
        })
        .on_window_event(|window, event| {
            // closing the window destroys the webview; the app (and any recording/job) stays alive in the Dock
            if let WindowEvent::Destroyed = event {
                if window.label() == "main" {
                    let state = window.state::<Arc<AppState>>();
                    let (m, cv) = &*state.ticker;
                    m.lock().unwrap().visible = false;
                    cv.notify_all();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Parlo");

    app.run(|app, event| match event {
        RunEvent::ExitRequested { api, code, .. } => {
            // keep running when the last window closes; an explicit exit(…) (code set) goes through
            let state = app.state::<Arc<AppState>>();
            if code.is_none() && !state.quitting.load(Ordering::SeqCst) {
                api.prevent_exit();
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { has_visible_windows, .. } => {
            if !has_visible_windows {
                ensure_main_window(app);
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    /// Regenerates `ui/src/bindings.ts` from the Rust definitions (run: `cargo test -p parlo-app export_bindings`).
    #[test]
    fn export_bindings() {
        super::specta_builder()
            .export(specta_typescript::Typescript::default(), super::BINDINGS_PATH)
            .expect("export bindings");
    }
}
