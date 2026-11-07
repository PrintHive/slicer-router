//! Slicer Router desktop shell.
//!
//! Opened with files, it routes each one, launches the slicer and exits without
//! showing a window. Opened on its own, it shows the settings window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod plan;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use plan::{plan, Plan};
use serde::Serialize;
use slicer_router_core::{discover_installed, launch, Config, InstalledSlicer, Slicer};
use tauri::{AppHandle, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};

const SETTINGS_WINDOW: &str = "settings";

/// How long a macOS launch waits for the system to deliver a file-open event
/// before concluding that the person started the app directly.
const OPEN_EVENT_GRACE: Duration = Duration::from_millis(700);

#[derive(Default)]
struct Shared {
    /// Files waiting for the person to choose a slicer.
    pending: Mutex<Vec<PathBuf>>,
    /// Latest problem to show in the settings window.
    notice: Mutex<Option<String>>,
    /// A file arrived since launch.
    got_files: AtomicBool,
    /// The person asked for the window (direct launch or reopen), so it stays
    /// open after pending files are handled.
    window_requested: AtomicBool,
}

#[derive(Serialize)]
struct SlicerView {
    id: Slicer,
    name: &'static str,
    path: PathBuf,
}

#[derive(Serialize)]
struct StateView {
    platform: &'static str,
    installed: Vec<SlicerView>,
    default_slicer: Option<Slicer>,
    pending: Vec<String>,
    notice: Option<String>,
    config_path: Option<PathBuf>,
}

fn main() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let files = files_from_args(&argv, Path::new(&cwd));
            if files.is_empty() {
                show_settings(app, true);
            } else {
                handle_files(app, files);
            }
        }))
        .manage(Shared::default())
        .invoke_handler(tauri::generate_handler![get_state, set_default]);

    builder
        .build(tauri::generate_context!())
        .expect("failed to build the Slicer Router app")
        .run(|app, event| match event {
            RunEvent::Ready => on_ready(app),
            #[cfg(target_os = "macos")]
            RunEvent::Opened { urls } => {
                let files = urls
                    .iter()
                    .filter_map(|url| url.to_file_path().ok())
                    .collect();
                handle_files(app, files);
            }
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => show_settings(app, true),
            _ => {}
        });
}

fn on_ready(app: &AppHandle) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let argv: Vec<String> = std::env::args().collect();
    let files = files_from_args(&argv, &cwd);
    if !files.is_empty() {
        handle_files(app, files);
        return;
    }
    let grace = if cfg!(target_os = "macos") {
        OPEN_EVENT_GRACE
    } else {
        Duration::ZERO
    };
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(grace);
        let main_handle = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            let shared = main_handle.state::<Shared>();
            if !shared.got_files.load(Ordering::SeqCst) {
                show_settings(&main_handle, true);
            }
        });
    });
}

/// Existing files named on the command line. Relative paths resolve against `cwd`.
fn files_from_args(argv: &[String], cwd: &Path) -> Vec<PathBuf> {
    argv.iter()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .map(|arg| cwd.join(arg))
        .filter(|path| path.is_file())
        .collect()
}

fn load_config_or_default() -> Config {
    let loaded = Config::default_path().map(|path| Config::load(&path));
    match loaded {
        Some(Ok(config)) => {
            for warning in &config.warnings {
                eprintln!("slicer-router: config: {warning}");
            }
            config
        }
        Some(Err(err)) => {
            eprintln!("slicer-router: {err}; using defaults");
            Config::default()
        }
        None => Config::default(),
    }
}

/// Route `files`, launch what can be launched, and exit unless a window is
/// needed to finish the job.
fn handle_files(app: &AppHandle, files: Vec<PathBuf>) {
    let shared = app.state::<Shared>();
    shared.got_files.store(true, Ordering::SeqCst);

    let installed = discover_installed();
    let Plan {
        launches,
        ask,
        notices,
    } = plan(&files, &installed, &load_config_or_default());

    let mut notices = notices;
    for (file, slicer) in launches {
        if let Err(err) = launch(&slicer, &file) {
            notices.push(err.to_string());
        }
    }

    let needs_window = !ask.is_empty() || !notices.is_empty();
    shared.pending.lock().unwrap().extend(ask);
    if !notices.is_empty() {
        *shared.notice.lock().unwrap() = Some(notices.join("\n"));
    }

    if needs_window {
        show_settings(app, false);
    } else if app.get_webview_window(SETTINGS_WINDOW).is_none() {
        app.exit(0);
    }
}

fn show_settings(app: &AppHandle, requested: bool) {
    let shared = app.state::<Shared>();
    if requested {
        shared.window_requested.store(true, Ordering::SeqCst);
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built =
        WebviewWindowBuilder::new(app, SETTINGS_WINDOW, WebviewUrl::App("index.html".into()))
            .title("Slicer Router")
            .inner_size(480.0, 600.0)
            .resizable(false)
            .focused(true)
            .build();
    if let Err(err) = built {
        eprintln!("slicer-router: cannot open the settings window: {err}");
        app.exit(1);
    }
}

#[tauri::command]
fn get_state(shared: State<'_, Shared>) -> StateView {
    let installed = discover_installed()
        .into_iter()
        .map(|InstalledSlicer { slicer, path, .. }| SlicerView {
            id: slicer,
            name: slicer.display_name(),
            path,
        })
        .collect();
    let config_path = Config::default_path();
    let default_slicer = load_config_or_default().default_slicer;
    StateView {
        platform: std::env::consts::OS,
        installed,
        default_slicer,
        pending: shared
            .pending
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        notice: shared.notice.lock().unwrap().take(),
        config_path,
    }
}

/// Remember `slicer` for files with no detectable origin, then open any files
/// that were waiting for that choice. A damaged config file is replaced.
#[tauri::command]
fn set_default(app: AppHandle, shared: State<'_, Shared>, slicer: Slicer) -> Result<(), String> {
    let path = Config::default_path().ok_or("cannot determine the config folder")?;
    let mut config = Config::load_for_update(&path).map_err(|e| e.to_string())?;
    config.default_slicer = Some(slicer);
    config.save(&path).map_err(|e| e.to_string())?;

    let pending = std::mem::take(&mut *shared.pending.lock().unwrap());
    if pending.is_empty() {
        return Ok(());
    }
    let installed = discover_installed();
    let target = installed
        .iter()
        .find(|i| i.slicer == slicer)
        .ok_or_else(|| format!("{slicer} is not installed"))?;
    let mut failures = Vec::new();
    for file in &pending {
        if let Err(err) = launch(target, file) {
            failures.push(err.to_string());
        }
    }
    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    if !shared.window_requested.load(Ordering::SeqCst) {
        app.exit(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_existing_non_flag_arguments_are_files() {
        let dir = std::env::temp_dir();
        let file = dir.join("slicer-router-args-test.3mf");
        std::fs::write(&file, b"").unwrap();
        let argv = vec![
            "slicer-router".to_owned(),
            "--flag".to_owned(),
            "slicer-router-args-test.3mf".to_owned(),
            "missing.3mf".to_owned(),
        ];
        assert_eq!(files_from_args(&argv, &dir), vec![file.clone()]);
        std::fs::remove_file(file).unwrap();
    }
}
