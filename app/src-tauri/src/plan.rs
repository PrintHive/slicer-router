//! Turn opened files into launches, questions and notices.

use std::path::{Path, PathBuf};

use slicer_router_core::{detect, route, Config, InstalledSlicer, Route, Slicer, Verdict};

use crate::pending::PendingFile;

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Files and the slicer that should open each.
    pub launches: Vec<(PathBuf, InstalledSlicer)>,
    /// Files that need the person to pick a slicer.
    pub ask: Vec<PendingFile>,
    /// Problems to tell the person about.
    pub notices: Vec<String>,
}

/// Route every file. An unreadable file is routed like one with no metadata, so
/// the chosen slicer shows its own error instead of the router refusing.
pub fn plan(files: &[PathBuf], installed: &[InstalledSlicer], config: &Config) -> Plan {
    let slicers: Vec<Slicer> = installed.iter().map(|i| i.slicer).collect();
    let mut plan = Plan::default();
    for file in files {
        let verdict = detect(file).unwrap_or_else(|err| {
            eprintln!("slicer-router: {err}");
            Verdict::no_signal()
        });
        match route(&verdict, &slicers, config) {
            Route::Launch { slicer, .. } => {
                if let Some(target) = installed.iter().find(|i| i.slicer == slicer) {
                    plan.launches.push((file.clone(), target.clone()));
                }
            }
            Route::Ask { detected, .. } => plan.ask.push(PendingFile {
                path: file.clone(),
                detected,
            }),
            Route::NoneInstalled => plan.notices.push(format!(
                "No supported slicer is installed, so {} was not opened.",
                file.display()
            )),
            _ => plan.notices.push(format!(
                "This version cannot route {}; update Slicer Router.",
                file.display()
            )),
        }
    }
    plan
}

/// What a process started with files does before it creates any window.
#[derive(Debug, PartialEq)]
pub enum Startup {
    /// Every file was handed to its slicer. Exit without starting the app.
    Done,
    /// Files were handed over but some launches failed. Start the app only to show this.
    /// The app must not route the files again, or the ones that opened would open twice.
    Failed(String),
    /// A window is needed (no files, or a file needs a question or an error),
    /// and nothing was launched. The app routes the files itself.
    Window,
}

/// Route `files` and launch them directly when no window is needed.
///
/// On Windows each file in a multi-selection starts its own process, and the
/// single-instance plugin hands extra processes' files to the first one over a
/// message the sender does not wait for. A process that routes its own files
/// here never depends on that handoff for the common case.
pub fn startup(
    files: &[PathBuf],
    installed: &[InstalledSlicer],
    config: &Config,
    launch: impl FnMut(&InstalledSlicer, &Path) -> Result<(), String>,
) -> Startup {
    if files.is_empty() {
        return Startup::Window;
    }
    let plan = plan(files, installed, config);
    if !plan.ask.is_empty() || !plan.notices.is_empty() {
        return Startup::Window;
    }
    let failures = launch_all(&plan.launches, launch);
    if failures.is_empty() {
        Startup::Done
    } else {
        Startup::Failed(failures.join("\n"))
    }
}

/// Launch every planned file. Returns one message per failure.
pub fn launch_all(
    launches: &[(PathBuf, InstalledSlicer)],
    mut launch: impl FnMut(&InstalledSlicer, &Path) -> Result<(), String>,
) -> Vec<String> {
    launches
        .iter()
        .filter_map(|(file, slicer)| launch(slicer, file).err())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Slicer::{BambuStudio, OrcaSlicer, PrusaSlicer, SnapmakerOrca};

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    fn installed(slicers: &[Slicer]) -> Vec<InstalledSlicer> {
        slicers
            .iter()
            .map(|&slicer| {
                InstalledSlicer::new(slicer, PathBuf::from(format!("/fake/{}", slicer.id())))
            })
            .collect()
    }

    #[test]
    fn real_files_go_to_the_slicer_that_made_them() {
        let installed = installed(&[BambuStudio, OrcaSlicer, SnapmakerOrca, PrusaSlicer]);
        let files = [
            fixture("real-snapmaker-orca-u1.3mf"),
            fixture("real-bambu-studio-p1s.3mf"),
            fixture("real-orcaslicer-cli.3mf"),
            fixture("real-prusaslicer-cli.3mf"),
        ];
        let plan = plan(&files, &installed, &Config::default());
        let routed: Vec<Slicer> = plan.launches.iter().map(|(_, s)| s.slicer).collect();
        assert_eq!(
            routed,
            [SnapmakerOrca, BambuStudio, OrcaSlicer, PrusaSlicer]
        );
        assert!(plan.ask.is_empty() && plan.notices.is_empty());
    }

    #[test]
    fn unknown_file_with_no_default_is_held_for_the_picker() {
        let dir = tempfile_dir();
        let unreadable = dir.join("not-a-zip.3mf");
        std::fs::write(&unreadable, b"junk").unwrap();
        let plan = plan(
            std::slice::from_ref(&unreadable),
            &installed(&[BambuStudio, PrusaSlicer]),
            &Config::default(),
        );
        assert_eq!(
            plan.ask,
            [PendingFile {
                path: unreadable,
                detected: None
            }]
        );
        assert!(plan.launches.is_empty());
    }

    #[test]
    fn a_file_for_a_missing_slicer_is_held_with_that_slicer_named() {
        let snapmaker_file = fixture("real-snapmaker-orca-u1.3mf");
        let plan = plan(
            std::slice::from_ref(&snapmaker_file),
            &installed(&[BambuStudio, PrusaSlicer]),
            &Config::default(),
        );
        assert_eq!(
            plan.ask,
            [PendingFile {
                path: snapmaker_file,
                detected: Some(SnapmakerOrca)
            }]
        );
        assert!(plan.launches.is_empty());
    }

    #[test]
    fn unknown_file_uses_the_default_when_set() {
        let dir = tempfile_dir();
        let unreadable = dir.join("not-a-zip-2.3mf");
        std::fs::write(&unreadable, b"junk").unwrap();
        let mut config = Config::default();
        config.default_slicer = Some(PrusaSlicer);
        let plan = plan(
            std::slice::from_ref(&unreadable),
            &installed(&[BambuStudio, PrusaSlicer]),
            &config,
        );
        assert_eq!(plan.launches[0].1.slicer, PrusaSlicer);
    }

    #[test]
    fn no_installed_slicer_is_reported() {
        let plan = plan(
            &[fixture("real-bambu-studio-p1s.3mf")],
            &[],
            &Config::default(),
        );
        assert_eq!(plan.notices.len(), 1);
    }

    fn junk_file(name: &str) -> PathBuf {
        let file = tempfile_dir().join(name);
        std::fs::write(&file, b"junk").unwrap();
        file
    }

    fn all_four() -> Vec<InstalledSlicer> {
        installed(&[BambuStudio, OrcaSlicer, SnapmakerOrca, PrusaSlicer])
    }

    #[test]
    fn every_file_of_a_multi_selection_launches_before_any_window_exists() {
        let files = [
            fixture("real-snapmaker-orca-u1.3mf"),
            fixture("real-bambu-studio-p1s.3mf"),
            fixture("real-prusaslicer-cli.3mf"),
        ];
        let mut launched = Vec::new();
        let result = startup(&files, &all_four(), &Config::default(), |slicer, file| {
            launched.push((file.to_owned(), slicer.slicer));
            Ok(())
        });
        assert_eq!(result, Startup::Done);
        assert_eq!(
            launched,
            [
                (files[0].clone(), SnapmakerOrca),
                (files[1].clone(), BambuStudio),
                (files[2].clone(), PrusaSlicer),
            ]
        );
    }

    #[test]
    fn no_files_means_the_settings_window() {
        let result = startup(&[], &all_four(), &Config::default(), |_, _| {
            panic!("nothing to launch")
        });
        assert_eq!(result, Startup::Window);
    }

    #[test]
    fn a_question_defers_every_file_so_none_opens_twice() {
        let ask = junk_file("startup-ask.3mf");
        let files = [fixture("real-bambu-studio-p1s.3mf"), ask];
        let installed = installed(&[BambuStudio, PrusaSlicer]);
        let result = startup(&files, &installed, &Config::default(), |_, _| {
            panic!("the window path routes the files itself")
        });
        assert_eq!(result, Startup::Window);
    }

    #[test]
    fn no_installed_slicer_needs_the_window_to_say_so() {
        let result = startup(
            &[fixture("real-bambu-studio-p1s.3mf")],
            &[],
            &Config::default(),
            |_, _| panic!("nothing to launch with"),
        );
        assert_eq!(result, Startup::Window);
    }

    #[test]
    fn launch_failures_are_reported_for_the_window_to_show() {
        let files = [
            fixture("real-bambu-studio-p1s.3mf"),
            fixture("real-prusaslicer-cli.3mf"),
        ];
        let result = startup(&files, &all_four(), &Config::default(), |slicer, _| {
            if slicer.slicer == BambuStudio {
                Err("cannot start Bambu Studio".to_owned())
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Startup::Failed("cannot start Bambu Studio".into()));
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("slicer-router-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
