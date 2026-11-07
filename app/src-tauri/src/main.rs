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

mod failure;
mod icons;
mod overrides;
mod pending;
mod plan;

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use failure::{launch_reason, Failure};
use pending::PendingFile;
use plan::{launch_all, plan, startup, Plan, Startup};
use serde::Serialize;
use slicer_router_core::{discover_installed, launch, Config, InstalledSlicer, Slicer, Snapshot};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};

const SETTINGS_WINDOW: &str = "settings";

/// Sent to the window whenever the pending files or the failures change, so an
/// open window refreshes instead of showing what it loaded at startup.
const STATE_CHANGED_EVENT: &str = "state-changed";

#[derive(Default)]
struct Shared {
    /// Files waiting for the person to choose a slicer.
    pending: Mutex<Vec<PendingFile>>,
    /// Files that did not open, shown until the person dismisses them.
    failures: Mutex<Vec<Failure>>,
    /// Slicer icons read from their apps, by app path. `None` is an app without a
    /// readable icon, remembered so it is not read again.
    icons: Mutex<HashMap<PathBuf, Option<String>>>,
    /// A file arrived since launch.
    got_files: AtomicBool,
    /// The person asked for the window (direct launch or reopen), so it stays
    /// open after pending files are handled.
    window_requested: AtomicBool,
}

impl Shared {
    /// Keep the outcome of routing some files: queue the questions and the
    /// failures.
    fn record(&self, ask: Vec<PendingFile>, failures: Vec<Failure>) {
        self.pending.lock().unwrap().extend(ask);
        let mut kept = self.failures.lock().unwrap();
        kept.retain(|old| !failures.iter().any(|new| new.file == old.file));
        kept.extend(failures);
    }
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
struct FailureView {
    file: String,
    slicer: Option<Slicer>,
    slicer_name: Option<&'static str>,
    reason: String,
    retryable: bool,
}

#[derive(Serialize)]
struct StateView {
    platform: &'static str,
    installed: Vec<SlicerView>,
    default_slicer: Option<Slicer>,
    /// Detected slicer to the slicer that opens its files instead.
    overrides: BTreeMap<Slicer, Slicer>,
    pending: Vec<PendingGroupView>,
    failures: Vec<FailureView>,
    config_path: Option<PathBuf>,
}

fn failures_view(failures: &[Failure]) -> Vec<FailureView> {
    failures
        .iter()
        .map(|failure| FailureView {
            file: failure.file.display().to_string(),
            slicer: failure.slicer,
            slicer_name: failure.slicer.map(Slicer::display_name),
            reason: failure.reason.clone(),
            retryable: failure.retryable,
        })
        .collect()
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
    let installed = discover_installed();
    let config = load_config_or_default();
    let started = startup(&files, &installed, &config, launch_file);
    refresh_thumbnails(installed, &config);
    match started {
        Startup::Done => {}
        Startup::Failed(failures) => run_app(Some(failures)),
        Startup::Window => run_app(None),
    }
}

fn launch_file(slicer: &InstalledSlicer, file: &Path) -> Result<(), String> {
    launch(slicer, file).map_err(|err| launch_reason(&err))
}

/// Save what the Finder thumbnail extension needs to badge each `.3mf` with the
/// slicer that will open it. Finder keeps showing cached thumbnails after the
/// routing changes, so a change also resets the Quick Look thumbnail cache.
fn refresh_thumbnails(installed: Vec<InstalledSlicer>, config: &Config) {
    let Some(dir) = Snapshot::default_dir() else {
        return;
    };
    match Snapshot::new(installed, config).save(&dir) {
        Ok(false) => {}
        Ok(true) => {
            let reset = Command::new("/usr/bin/qlmanage")
                .args(["-r", "cache"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match reset {
                Ok(mut child) => {
                    std::thread::spawn(move || child.wait());
                }
                Err(err) => eprintln!("slicer-router: cannot reset the Quick Look cache: {err}"),
            }
        }
        Err(err) => eprintln!(
            "slicer-router: cannot save the thumbnail snapshot in {}: {err}",
            dir.display()
        ),
    }
}

/// Run the Tauri app. `failures` is set when the files already launched and only
/// the errors need showing: that process skips the single-instance handoff,
/// which would send its command line to the running instance and open those
/// files a second time.
fn run_app(failures: Option<Vec<Failure>>) {
    let mut builder = tauri::Builder::default();
    if failures.is_none() {
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
        failures: Mutex::new(failures.unwrap_or_default()),
        ..Shared::default()
    };
    builder
        .manage(shared)
        .invoke_handler(tauri::generate_handler![
            get_state,
            slicer_icons,
            choose,
            set_override,
            retry_failures,
            open_failures_in,
            dismiss_failures
        ])
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

/// What the app does when it is ready to run, before anyone has interacted with it.
#[derive(Debug, PartialEq)]
enum Ready {
    /// Files already launched and only their error needs showing.
    ShowFailure,
    /// Files came on the command line (Windows).
    RouteArgs(Vec<PathBuf>),
    /// Started directly: show the settings window.
    ShowSettings,
    /// Files already arrived and were handled. Do nothing more.
    Idle,
}

/// macOS delivers the open-documents event (`RunEvent::Opened`) before
/// `applicationDidFinishLaunching`, which is when `RunEvent::Ready` fires, so by
/// `Ready` the app knows whether it was opened with files. That replaces a timer
/// that waited for the event and could show a window that then had to be taken
/// back. Observed on macOS 26: `Opened` arrives about 140 ms before `Ready`.
fn ready_action(has_failure: bool, args: Vec<PathBuf>, got_files: bool) -> Ready {
    if has_failure {
        Ready::ShowFailure
    } else if !args.is_empty() {
        Ready::RouteArgs(args)
    } else if got_files {
        Ready::Idle
    } else {
        Ready::ShowSettings
    }
}

fn on_ready(app: &AppHandle) {
    let shared = app.state::<Shared>();
    let cwd = std::env::current_dir().unwrap_or_default();
    let action = ready_action(
        !shared.failures.lock().unwrap().is_empty(),
        files_from_args(std::env::args_os(), &cwd),
        shared.got_files.load(Ordering::SeqCst),
    );
    match action {
        Ready::ShowFailure => show_settings(app, false),
        Ready::RouteArgs(files) => handle_files(app, files),
        Ready::ShowSettings => show_settings(app, true),
        Ready::Idle => {}
    }
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
        mut failures,
    } = plan(&files, &installed, &load_config_or_default());

    failures.extend(launch_all(&launches, launch_file));

    let needs_window = !ask.is_empty() || !failures.is_empty();
    shared.record(ask, failures);
    let _ = app.emit(STATE_CHANGED_EVENT, ());

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
    state_view(
        discover_installed(),
        &load_config_or_default(),
        &shared.pending.lock().unwrap(),
        &shared.failures.lock().unwrap(),
        Config::default_path(),
    )
}

fn state_view(
    installed: Vec<InstalledSlicer>,
    config: &Config,
    pending: &[PendingFile],
    failures: &[Failure],
    config_path: Option<PathBuf>,
) -> StateView {
    StateView {
        platform: std::env::consts::OS,
        installed: installed
            .into_iter()
            .map(|InstalledSlicer { slicer, path, .. }| SlicerView {
                id: slicer,
                name: slicer.display_name(),
                path,
            })
            .collect(),
        default_slicer: config.default_slicer,
        overrides: config.overrides.clone(),
        pending: pending_view(pending),
        failures: failures_view(failures),
        config_path,
    }
}

/// The icon of each installed slicer as a PNG data URL, or `null` when its app
/// has none the window can read. Reading is slow enough to keep off the main
/// thread, so [`get_state`] does not carry it.
#[tauri::command]
async fn slicer_icons(
    shared: State<'_, Shared>,
) -> Result<BTreeMap<Slicer, Option<String>>, String> {
    let installed = discover_installed();
    let unread: Vec<PathBuf> = {
        let cache = shared.icons.lock().unwrap();
        installed
            .iter()
            .map(|i| i.path.clone())
            .filter(|path| !cache.contains_key(path))
            .collect()
    };
    if !unread.is_empty() {
        let read = tauri::async_runtime::spawn_blocking(move || {
            unread
                .into_iter()
                .map(|path| {
                    let icon = icons::png_data_url(&path);
                    (path, icon)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|err| err.to_string())?;
        shared.icons.lock().unwrap().extend(read);
    }
    let cache = shared.icons.lock().unwrap();
    Ok(installed
        .into_iter()
        .map(|i| (i.slicer, cache.get(&i.path).cloned().flatten()))
        .collect())
}

/// Send files detected as `detected` to `slicer` instead, or back to where their
/// own metadata says when `slicer` is `None` or `detected` itself.
#[tauri::command]
fn set_override(detected: Slicer, slicer: Option<Slicer>) -> Result<(), String> {
    let installed = discover_installed();
    if let Some(slicer) = slicer {
        if !installed.iter().any(|i| i.slicer == slicer) {
            return Err(format!("{slicer} is not installed"));
        }
    }
    let path = Config::default_path().ok_or("cannot determine the config folder")?;
    overrides::set(&path, detected, slicer)?;
    refresh_thumbnails(installed, &load_config_or_default());
    Ok(())
}

#[tauri::command]
fn dismiss_failures(shared: State<'_, Shared>) {
    shared.failures.lock().unwrap().clear();
}

/// Launch the failures that can work again in the slicer that failed them. What
/// still fails comes back as failures.
#[tauri::command]
fn retry_failures(app: AppHandle, shared: State<'_, Shared>) {
    let failures = std::mem::take(&mut *shared.failures.lock().unwrap());
    let remaining = failure::retry(failures, &discover_installed(), launch_file);
    finish_recovery(&app, &shared, remaining);
}

/// Open every failed file in `slicer`, this once. The config is not changed.
#[tauri::command]
fn open_failures_in(
    app: AppHandle,
    shared: State<'_, Shared>,
    slicer: Slicer,
) -> Result<(), String> {
    let target = discover_installed()
        .into_iter()
        .find(|i| i.slicer == slicer)
        .ok_or_else(|| format!("{slicer} is not installed"))?;
    let failures = std::mem::take(&mut *shared.failures.lock().unwrap());
    let remaining = failure::open_in(failures, &target, launch_file);
    finish_recovery(&app, &shared, remaining);
    Ok(())
}

fn finish_recovery(app: &AppHandle, shared: &Shared, remaining: Vec<Failure>) {
    shared.record(Vec::new(), remaining);
    let _ = app.emit(STATE_CHANGED_EVENT, ());
    exit_if_idle(app, shared);
}

/// Exit once nothing is waiting or failing, unless the person asked for the window.
fn exit_if_idle(app: &AppHandle, shared: &Shared) {
    if shared.pending.lock().unwrap().is_empty()
        && shared.failures.lock().unwrap().is_empty()
        && !shared.window_requested.load(Ordering::SeqCst)
    {
        app.exit(0);
    }
}

/// Answer a question and open the files that were waiting on it.
///
/// `detected` is the group's slicer from [`PendingGroupView`], or `None` for
/// files with no signal and for picking the default slicer with nothing waiting.
/// The answer is saved by [`pending::resolve`]: an override for a detected slicer,
/// the default slicer otherwise. Files that do not open become failures for the
/// window to show, not an error from this command.
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

    let released = pending::resolve(&path, &mut shared.pending.lock().unwrap(), detected, slicer)?;
    let launches: Vec<_> = released
        .iter()
        .map(|file| (file.clone(), target.clone()))
        .collect();
    let failures = launch_all(&launches, launch_file);
    refresh_thumbnails(installed.clone(), &load_config_or_default());
    shared.record(Vec::new(), failures);
    let _ = app.emit(STATE_CHANGED_EVENT, ());
    if !released.is_empty() {
        exit_if_idle(&app, &shared);
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
    fn a_launch_that_already_received_files_never_shows_a_window_on_its_own() {
        let file = vec![PathBuf::from("a.3mf")];
        assert_eq!(ready_action(false, vec![], true), Ready::Idle);
        assert_eq!(ready_action(false, vec![], false), Ready::ShowSettings);
        assert_eq!(
            ready_action(false, file.clone(), false),
            Ready::RouteArgs(file.clone())
        );
        assert_eq!(ready_action(true, file, true), Ready::ShowFailure);
    }

    fn failure(file: &str, reason: &str) -> Failure {
        Failure {
            file: file.into(),
            slicer: Some(Slicer::PrusaSlicer),
            reason: reason.into(),
            retryable: true,
        }
    }

    #[test]
    fn routing_more_files_adds_questions_and_keeps_failures_until_dismissed() {
        let shared = Shared::default();
        let file = |name: &str| PendingFile {
            path: name.into(),
            detected: None,
        };
        shared.record(vec![file("a.3mf")], Vec::new());
        shared.record(vec![file("b.3mf")], vec![failure("x.3mf", "first")]);
        shared.record(Vec::new(), vec![failure("y.3mf", "second")]);
        shared.record(Vec::new(), Vec::new());

        assert_eq!(
            *shared.pending.lock().unwrap(),
            [file("a.3mf"), file("b.3mf")]
        );
        assert_eq!(
            *shared.failures.lock().unwrap(),
            [failure("x.3mf", "first"), failure("y.3mf", "second")],
            "routing with no problems keeps earlier failures until they are dismissed"
        );
    }

    #[test]
    fn a_file_that_fails_again_replaces_its_earlier_failure() {
        let shared = Shared::default();
        shared.record(Vec::new(), vec![failure("x.3mf", "first")]);
        shared.record(Vec::new(), vec![failure("x.3mf", "second")]);
        assert_eq!(
            *shared.failures.lock().unwrap(),
            [failure("x.3mf", "second")]
        );
    }

    #[test]
    fn the_whole_state_has_the_shape_the_window_reads() {
        let mut config = Config::default();
        config.default_slicer = Some(Slicer::BambuStudio);
        config
            .overrides
            .insert(Slicer::PrusaSlicer, Slicer::OrcaSlicer);
        let pending = [PendingFile {
            path: "a.3mf".into(),
            detected: None,
        }];
        let view = state_view(
            vec![InstalledSlicer::new(
                Slicer::BambuStudio,
                "/Applications/BambuStudio.app".into(),
            )],
            &config,
            &pending,
            &[failure("b.3mf", "PrusaSlicer didn't start")],
            Some("/config.json".into()),
        );
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            serde_json::json!({
                "platform": std::env::consts::OS,
                "installed": [
                    {"id": "bambu-studio", "name": "Bambu Studio", "path": "/Applications/BambuStudio.app"},
                ],
                "default_slicer": "bambu-studio",
                "overrides": {"prusa-slicer": "orca-slicer"},
                "pending": [{"detected": null, "detected_name": null, "files": ["a.3mf"]}],
                "failures": [{
                    "file": "b.3mf",
                    "slicer": "prusa-slicer",
                    "slicer_name": "PrusaSlicer",
                    "reason": "PrusaSlicer didn't start",
                    "retryable": true,
                }],
                "config_path": "/config.json",
            })
        );
    }

    #[test]
    fn the_window_gets_failures_with_the_slicer_named_and_whether_to_retry() {
        let failures = [
            failure(
                "/Downloads/a.3mf",
                "PrusaSlicer didn't start (exit status 1)",
            ),
            Failure::no_slicer_installed(Path::new("/Downloads/b.3mf")),
        ];
        assert_eq!(
            serde_json::to_value(failures_view(&failures)).unwrap(),
            serde_json::json!([
                {
                    "file": "/Downloads/a.3mf",
                    "slicer": "prusa-slicer",
                    "slicer_name": "PrusaSlicer",
                    "reason": "PrusaSlicer didn't start (exit status 1)",
                    "retryable": true,
                },
                {
                    "file": "/Downloads/b.3mf",
                    "slicer": null,
                    "slicer_name": null,
                    "reason": "No supported slicer is installed",
                    "retryable": false,
                },
            ])
        );
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
