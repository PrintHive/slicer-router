//! Slicer Router desktop shell.
//!
//! Opened with files, it routes each one, launches the slicer and exits without
//! showing a window. Opened on its own, it shows the settings window.
//!
//! A file named on the command line is routed in `main` before Tauri exists, so
//! a Windows multi-selection (one process per file) launches every file without
//! relying on the single-instance handoff. The app starts only when a window is
//! needed: no files, a question to ask, or an error to show.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod pending;
mod plan;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use pending::PendingFile;
use plan::{launch_all, plan, startup, Plan, Startup};
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
    pending: Mutex<Vec<PendingFile>>,
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

/// Pending files that share a question.
#[derive(Serialize)]
struct PendingGroupView {
    /// The slicer these files belong to when it is not installed, or `null` for
    /// files with no signal.
    detected: Option<Slicer>,
    detected_name: Option<&'static str>,
    files: Vec<String>,
}

#[derive(Serialize)]
struct StateView {
    platform: &'static str,
    installed: Vec<SlicerView>,
    default_slicer: Option<Slicer>,
    pending: Vec<PendingGroupView>,
    notice: Option<String>,
    config_path: Option<PathBuf>,
}

fn pending_view(pending: &[PendingFile]) -> Vec<PendingGroupView> {
    pending::groups(pending)
        .into_iter()
        .map(|(detected, files)| PendingGroupView {
            detected,
            detected_name: detected.map(Slicer::display_name),
            files: files.iter().map(|f| f.display().to_string()).collect(),
        })
        .collect()
}

fn main() {
    let cwd = std::env::current_dir().unwrap_or_default();
    let files = files_from_args(std::env::args_os(), &cwd);
    let started = startup(
        &files,
        &discover_installed(),
        &load_config_or_default(),
        launch_file,
    );
    match started {
        Startup::Done => {}
        Startup::Failed(notice) => run_app(Some(notice)),
        Startup::Window => run_app(None),
    }
}

fn launch_file(slicer: &InstalledSlicer, file: &Path) -> Result<(), String> {
    launch(slicer, file).map_err(|err| err.to_string())
}

/// Run the Tauri app. `failure` is set when the files already launched and only
/// the error needs showing: that process skips the single-instance handoff,
/// which would send its command line to the running instance and open those
/// files a second time.
fn run_app(failure: Option<String>) {
    let mut builder = tauri::Builder::default();
    if failure.is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let files = files_from_args(argv.into_iter().map(OsString::from), Path::new(&cwd));
            if files.is_empty() {
                show_settings(app, true);
            } else {
                handle_files(app, files);
            }
        }));
    }
    let shared = Shared {
        notice: Mutex::new(failure),
        ..Shared::default()
    };
    builder
        .manage(shared)
        .invoke_handler(tauri::generate_handler![get_state, choose])
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
    if app.state::<Shared>().notice.lock().unwrap().is_some() {
        show_settings(app, false);
        return;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let files = files_from_args(std::env::args_os(), &cwd);
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

/// Existing files named on the command line, which may hold non-Unicode names.
/// Relative paths resolve against `cwd`.
fn files_from_args(argv: impl IntoIterator<Item = OsString>, cwd: &Path) -> Vec<PathBuf> {
    argv.into_iter()
        .skip(1)
        .filter(|arg| arg.as_encoded_bytes().first() != Some(&b'-'))
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
    notices.extend(launch_all(&launches, launch_file));

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
        pending: pending_view(&shared.pending.lock().unwrap()),
        notice: shared.notice.lock().unwrap().take(),
        config_path,
    }
}

/// Answer a question and open the files that were waiting on it.
///
/// `detected` is the group's slicer from [`PendingGroupView`], or `None` for
/// files with no signal and for picking the default slicer with nothing waiting.
/// The answer is saved by [`pending::resolve`]: an override for a detected slicer,
/// the default slicer otherwise.
#[tauri::command]
fn choose(
    app: AppHandle,
    shared: State<'_, Shared>,
    detected: Option<Slicer>,
    slicer: Slicer,
) -> Result<(), String> {
    let installed = discover_installed();
    let target = installed
        .iter()
        .find(|i| i.slicer == slicer)
        .ok_or_else(|| format!("{slicer} is not installed"))?;
    let path = Config::default_path().ok_or("cannot determine the config folder")?;

    let (released, nothing_left) = {
        let mut pending = shared.pending.lock().unwrap();
        let released = pending::resolve(&path, &mut pending, detected, slicer)?;
        (released, pending.is_empty())
    };
    let failures: Vec<String> = released
        .iter()
        .filter_map(|file| launch_file(target, file).err())
        .collect();
    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    if !released.is_empty() && nothing_left && !shared.window_requested.load(Ordering::SeqCst) {
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
        let argv = [
            "slicer-router",
            "--flag",
            "slicer-router-args-test.3mf",
            "missing.3mf",
        ]
        .map(OsString::from);
        assert_eq!(files_from_args(argv, &dir), vec![file.clone()]);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn the_window_gets_one_question_per_group_with_the_missing_slicer_named() {
        let pending = [
            PendingFile {
                path: "a.3mf".into(),
                detected: None,
            },
            PendingFile {
                path: "b.3mf".into(),
                detected: Some(Slicer::SnapmakerOrca),
            },
        ];
        assert_eq!(
            serde_json::to_value(pending_view(&pending)).unwrap(),
            serde_json::json!([
                {"detected": null, "detected_name": null, "files": ["a.3mf"]},
                {"detected": "snapmaker-orca", "detected_name": "Snapmaker Orca", "files": ["b.3mf"]},
            ])
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_non_unicode_argument_is_skipped_not_a_panic() {
        use std::os::unix::ffi::OsStringExt;
        let argv = vec![
            OsString::from("slicer-router"),
            OsString::from_vec(vec![b'f', 0xff, b'.', b'3', b'm', b'f']),
        ];
        assert!(files_from_args(argv, &std::env::temp_dir()).is_empty());
    }
}
