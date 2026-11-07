//! Files that could not be opened, and what the person can do about them.

use std::path::{Path, PathBuf};

use slicer_router_core::{InstalledSlicer, LaunchError, Slicer};

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

    /// `slicer` failed to open `file` for `reason`. An app that is no longer at
    /// its path cannot be retried, so that is the reason whatever the error said.
    pub fn launch(file: &Path, slicer: &InstalledSlicer, reason: String) -> Self {
        if slicer.path.exists() {
            Failure {
                file: file.to_owned(),
                slicer: Some(slicer.slicer),
                reason,
                retryable: true,
            }
        } else {
            Self::gone(file, slicer.slicer)
        }
    }

    fn gone(file: &Path, slicer: Slicer) -> Self {
        Failure {
            file: file.to_owned(),
            slicer: Some(slicer),
            reason: format!("{slicer} was moved or deleted"),
            retryable: false,
        }
    }
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

    fn failed(file: &str, slicer: &InstalledSlicer) -> Failure {
        Failure::launch(Path::new(file), slicer, "didn't start".into())
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
        let failure = failed("a.3mf", &present(PrusaSlicer));
        assert_eq!(failure.slicer, Some(PrusaSlicer));
        assert_eq!(failure.reason, "didn't start");
        assert!(failure.retryable);
    }

    #[test]
    fn a_slicer_that_was_moved_or_deleted_cannot_be_tried_again() {
        let failure = failed("a.3mf", &gone(BambuStudio));
        assert_eq!(failure.slicer, Some(BambuStudio));
        assert_eq!(failure.reason, "Bambu Studio was moved or deleted");
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
}
