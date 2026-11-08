//! Files that could not be opened, and what the person can do about them.

use std::path::{Path, PathBuf};

use slicer_router_core::{InstalledSlicer, LaunchError, Slicer};

use crate::plan::launch_all;

/// One file that did not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub file: PathBuf,
    /// The slicer that was meant to open it, or `None` when none could be chosen.
    pub slicer: Option<Slicer>,
    /// Short and for people: no path, no error chain.
    pub reason: String,
    /// Whether launching the same slicer again can succeed.
    pub retryable: bool,
}

impl Failure {
    pub fn no_slicer_installed(file: &Path) -> Self {
        Self::unplaced(file, "No supported slicer is installed")
    }

    pub fn unroutable(file: &Path) -> Self {
        Self::unplaced(
            file,
            "This version of Slicer Router cannot route it. Update Slicer Router",
        )
    }

    fn unplaced(file: &Path, reason: &str) -> Self {
        Failure {
            file: file.to_owned(),
            slicer: None,
            reason: reason.to_owned(),
            retryable: false,
        }
    }

    /// `slicer` failed to open `file` for `reason`. A file that is no longer
    /// there, or an app that is no longer at its path, cannot be retried, so that
    /// is the reason whatever the error said.
    pub fn launch(file: &Path, slicer: &InstalledSlicer, reason: String) -> Self {
        if !file.exists() {
            Self::file_gone(file, Some(slicer.slicer))
        } else if !slicer.path.exists() {
            Self::slicer_gone(file, slicer.slicer)
        } else {
            Failure {
                file: file.to_owned(),
                slicer: Some(slicer.slicer),
                reason,
                retryable: true,
            }
        }
    }

    fn slicer_gone(file: &Path, slicer: Slicer) -> Self {
        Self::gone(file, Some(slicer), format!("{slicer} was moved or deleted"))
    }

    fn file_gone(file: &Path, slicer: Option<Slicer>) -> Self {
        let name = file.file_name().map_or_else(
            || file.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        Self::gone(file, slicer, format!("{name} was moved or deleted"))
    }

    fn gone(file: &Path, slicer: Option<Slicer>, reason: String) -> Self {
        Failure {
            file: file.to_owned(),
            slicer,
            reason,
            retryable: false,
        }
    }
}

/// Remove from `failures`, and return, the ones for the files the person was
/// shown (as the window displays them). A failure that arrived since stays.
pub fn take_shown(failures: &mut Vec<Failure>, shown: &[String]) -> Vec<Failure> {
    let (taken, kept) = std::mem::take(failures)
        .into_iter()
        .partition(|failure| shown.contains(&failure.file.display().to_string()));
    *failures = kept;
    taken
}

/// Why `err` stopped a slicer from opening a file, in a few words.
pub fn launch_reason(err: &LaunchError) -> String {
    match err {
        LaunchError::Failed { slicer, status } => match status.code() {
            Some(code) => format!("{} didn't start (exit status {code})", slicer.slicer),
            None => format!("{} didn't start", slicer.slicer),
        },
        LaunchError::Spawn { slicer, source } => {
            format!("{} couldn't be started: {source}", slicer.slicer)
        }
        other => other.to_string(),
    }
}

/// "Try again": launch the failures that can work again in the slicer that
/// failed them. Each is checked again first, since the file or the slicer may
/// have gone since it failed. Returns what is still failing, which is the
/// failures that cannot be retried, the ones that failed again, and the ones
/// whose file or slicer is no longer there.
pub fn retry(
    failures: Vec<Failure>,
    installed: &[InstalledSlicer],
    launch: impl FnMut(&InstalledSlicer, &Path) -> Result<(), String>,
) -> Vec<Failure> {
    let (again, mut remaining): (Vec<_>, Vec<_>) = failures.into_iter().partition(|f| f.retryable);
    let mut launches = Vec::new();
    for failure in again {
        let Some(slicer) = failure.slicer else {
            remaining.push(failure);
            continue;
        };
        if !failure.file.exists() {
            remaining.push(Failure::file_gone(&failure.file, Some(slicer)));
            continue;
        }
        match installed.iter().find(|i| i.slicer == slicer) {
            Some(target) => launches.push((failure.file, target.clone())),
            None => remaining.push(Failure::slicer_gone(&failure.file, slicer)),
        }
    }
    remaining.extend(launch_all(&launches, launch));
    remaining
}

/// "Open in another slicer": launch every failed file in `target`, once, without
/// saving anything. Returns the files that failed there.
pub fn open_in(
    failures: Vec<Failure>,
    target: &InstalledSlicer,
    launch: impl FnMut(&InstalledSlicer, &Path) -> Result<(), String>,
) -> Vec<Failure> {
    let launches: Vec<_> = failures
        .into_iter()
        .map(|failure| (failure.file, target.clone()))
        .collect();
    launch_all(&launches, launch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Slicer::{BambuStudio, OrcaSlicer, PrusaSlicer};

    /// A slicer whose app exists, so a failed launch can be retried.
    fn present(slicer: Slicer) -> InstalledSlicer {
        InstalledSlicer::new(slicer, std::env::temp_dir())
    }

    /// A slicer whose app has been deleted.
    fn gone(slicer: Slicer) -> InstalledSlicer {
        InstalledSlicer::new(slicer, "/slicer-router-test/deleted.app".into())
    }

    /// An empty file named `name` in a folder of its own, so no test sees another's.
    fn real_file(test: &str, name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "slicer-router-failure-{}-{test}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(name);
        std::fs::write(&file, b"").unwrap();
        file
    }

    fn failed(file: &Path, slicer: &InstalledSlicer) -> Failure {
        Failure::launch(file, slicer, "didn't start".into())
    }

    fn summary(failures: &[Failure]) -> Vec<(String, bool)> {
        failures
            .iter()
            .map(|f| {
                let name = f.file.file_name().unwrap().to_string_lossy().into_owned();
                (name, f.retryable)
            })
            .collect()
    }

    #[test]
    fn a_file_with_no_slicer_installed_names_no_slicer_and_cannot_be_retried() {
        let failure = Failure::no_slicer_installed(Path::new("a.3mf"));
        assert_eq!(failure.slicer, None);
        assert_eq!(failure.reason, "No supported slicer is installed");
        assert!(!failure.retryable);
    }

    #[test]
    fn an_unroutable_file_names_no_slicer_and_cannot_be_retried() {
        let failure = Failure::unroutable(Path::new("a.3mf"));
        assert_eq!(failure.slicer, None);
        assert!(!failure.retryable);
    }

    #[test]
    fn a_slicer_that_failed_to_start_can_be_tried_again() {
        let file = real_file("start", "a.3mf");
        let failure = failed(&file, &present(PrusaSlicer));
        assert_eq!(failure.slicer, Some(PrusaSlicer));
        assert_eq!(failure.reason, "didn't start");
        assert!(failure.retryable);
    }

    #[test]
    fn a_slicer_that_was_moved_or_deleted_cannot_be_tried_again() {
        let file = real_file("slicer-gone", "a.3mf");
        let failure = failed(&file, &gone(BambuStudio));
        assert_eq!(failure.slicer, Some(BambuStudio));
        assert_eq!(failure.reason, "Bambu Studio was moved or deleted");
        assert!(!failure.retryable);
    }

    #[test]
    fn a_file_that_is_not_there_cannot_be_tried_again_and_the_reason_names_it() {
        let failure = failed(
            Path::new("/slicer-router-test/benchy.3mf"),
            &present(OrcaSlicer),
        );
        assert_eq!(failure.slicer, Some(OrcaSlicer));
        assert_eq!(failure.reason, "benchy.3mf was moved or deleted");
        assert!(!failure.retryable);
    }

    #[test]
    fn spawn_errors_and_exit_codes_read_as_one_short_sentence() {
        let installed = present(OrcaSlicer);
        let spawn = LaunchError::Spawn {
            slicer: installed.clone(),
            source: std::io::Error::other("no such file"),
        };
        assert_eq!(
            launch_reason(&spawn),
            "OrcaSlicer couldn't be started: no such file"
        );
        assert_eq!(
            launch_reason(&LaunchError::WouldLaunchSelf("/x".into())),
            "refusing to launch /x: it is the running router itself"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_nonzero_exit_names_the_status() {
        use std::os::unix::process::ExitStatusExt;
        let err = LaunchError::Failed {
            slicer: present(PrusaSlicer),
            status: std::process::ExitStatus::from_raw(1 << 8),
        };
        assert_eq!(
            launch_reason(&err),
            "PrusaSlicer didn't start (exit status 1)"
        );
    }

    #[test]
    fn retry_relaunches_only_what_can_work_again_and_keeps_the_rest() {
        let prusa = present(PrusaSlicer);
        let (a, b, c, d) = (
            real_file("retry", "a.3mf"),
            real_file("retry", "b.3mf"),
            real_file("retry", "c.3mf"),
            real_file("retry", "d.3mf"),
        );
        let failures = vec![
            failed(&a, &prusa),
            failed(&b, &gone(BambuStudio)),
            Failure::no_slicer_installed(&c),
            failed(&d, &prusa),
        ];
        let mut launched = Vec::new();
        let remaining = retry(failures, std::slice::from_ref(&prusa), |slicer, file| {
            launched.push((slicer.slicer, file.to_owned()));
            if file == d {
                Err("still broken".into())
            } else {
                Ok(())
            }
        });

        assert_eq!(launched, [(PrusaSlicer, a), (PrusaSlicer, d)]);
        assert_eq!(
            summary(&remaining),
            [
                ("b.3mf".to_owned(), false),
                ("c.3mf".to_owned(), false),
                ("d.3mf".to_owned(), true)
            ]
        );
    }

    #[test]
    fn retry_gives_up_on_a_slicer_that_disappeared_since_the_failure() {
        let file = real_file("retry-slicer", "a.3mf");
        let failures = vec![failed(&file, &present(PrusaSlicer))];
        let remaining = retry(failures, &[], |_, _| panic!("nothing to launch with"));
        assert_eq!(
            remaining,
            [Failure {
                file,
                slicer: Some(PrusaSlicer),
                reason: "PrusaSlicer was moved or deleted".into(),
                retryable: false,
            }]
        );
    }

    #[test]
    fn retry_checks_the_file_again_so_one_deleted_since_is_not_launched() {
        let prusa = present(PrusaSlicer);
        let file = real_file("retry-file", "a.3mf");
        let failures = vec![failed(&file, &prusa)];
        assert!(failures[0].retryable);
        std::fs::remove_file(&file).unwrap();

        let remaining = retry(failures, std::slice::from_ref(&prusa), |_, _| {
            panic!("the file is gone")
        });
        assert_eq!(
            remaining,
            [Failure {
                file,
                slicer: Some(PrusaSlicer),
                reason: "a.3mf was moved or deleted".into(),
                retryable: false,
            }]
        );
    }

    #[test]
    fn opening_elsewhere_launches_every_failed_file_and_reports_new_failures() {
        let (a, b) = (
            real_file("elsewhere", "a.3mf"),
            real_file("elsewhere", "b.3mf"),
        );
        let failures = vec![
            failed(&a, &gone(BambuStudio)),
            Failure::no_slicer_installed(&b),
        ];
        let orca = present(OrcaSlicer);
        let remaining = open_in(failures, &orca, |slicer, file| {
            assert_eq!(slicer.slicer, OrcaSlicer);
            if file == b {
                Err("didn't start".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(
            remaining,
            [Failure {
                file: b,
                slicer: Some(OrcaSlicer),
                reason: "didn't start".into(),
                retryable: true,
            }]
        );
    }

    #[test]
    fn only_the_failures_the_person_was_shown_are_taken() {
        let prusa = present(PrusaSlicer);
        let (a, b, c) = (
            real_file("shown", "a.3mf"),
            real_file("shown", "b.3mf"),
            real_file("shown", "c.3mf"),
        );
        let mut failures = vec![failed(&a, &prusa), failed(&b, &prusa), failed(&c, &prusa)];
        let shown = [a.display().to_string(), c.display().to_string()];

        let taken = take_shown(&mut failures, &shown);

        assert_eq!(
            summary(&taken),
            [("a.3mf".to_owned(), true), ("c.3mf".to_owned(), true)]
        );
        assert_eq!(summary(&failures), [("b.3mf".to_owned(), true)]);
    }
}
