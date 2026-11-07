//! Start a slicer on a file.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::discover::InstalledSlicer;

/// Why a slicer could not be started.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LaunchError {
    #[error("cannot start {}: {source}", slicer.slicer)]
    Spawn {
        slicer: InstalledSlicer,
        source: io::Error,
    },
    #[error("starting {} failed: {status}", slicer.slicer)]
    Failed {
        slicer: InstalledSlicer,
        status: std::process::ExitStatus,
    },
    #[error("refusing to launch {}: it is the running router itself", .0.display())]
    WouldLaunchSelf(std::path::PathBuf),
}

/// Open `file` in `slicer`.
///
/// `file` is made absolute first, so a relative path, or a name that starts
/// with `-`, cannot be read as an option or resolved against another folder.
///
/// What a successful return means differs by platform:
///
/// - macOS runs `open -a <bundle> <file>` and waits for `open` to exit. That
///   happens once the system has found the app and handed it the file, so a
///   missing or broken bundle is reported as [`LaunchError::Failed`]. The slicer
///   keeps running afterwards. Naming the slicer's own bundle keeps the system
///   from picking the router as the handler.
/// - Windows starts the slicer executable with the file as its argument and
///   returns as soon as the process exists. Whether the slicer then opens the
///   file is not observed, and a failure after that point is not reported.
///
/// Refuses, with [`LaunchError::WouldLaunchSelf`], any target that contains the
/// running executable, so a misconfigured entry cannot make the router relaunch itself.
pub fn launch(slicer: &InstalledSlicer, file: &Path) -> Result<(), LaunchError> {
    refuse_self(&slicer.path)?;
    let spawn_err = |source| LaunchError::Spawn {
        slicer: slicer.clone(),
        source,
    };
    let mut command = launch_command(&slicer.path, file);
    if cfg!(target_os = "macos") {
        let status = command.status().map_err(spawn_err)?;
        if !status.success() {
            return Err(LaunchError::Failed {
                slicer: slicer.clone(),
                status,
            });
        }
    } else {
        command.spawn().map_err(spawn_err)?;
    }
    Ok(())
}

/// The process that opens `file` in the slicer at `slicer_path`.
fn launch_command(slicer_path: &Path, file: &Path) -> Command {
    let file: PathBuf = std::path::absolute(file).unwrap_or_else(|_| file.to_owned());
    let mut command = if cfg!(target_os = "macos") {
        let mut open = Command::new("open");
        open.arg("-a").arg(slicer_path);
        open
    } else {
        Command::new(slicer_path)
    };
    command.arg(file);
    command
}

fn refuse_self(target: &Path) -> Result<(), LaunchError> {
    let (Ok(me), Ok(target)) = (std::env::current_exe(), target.canonicalize()) else {
        return Ok(());
    };
    match me.canonicalize() {
        Ok(me) if me.starts_with(&target) => Err(LaunchError::WouldLaunchSelf(target)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::Slicer;

    #[test]
    fn the_file_argument_is_absolute() {
        for name in ["model.3mf", "-model.3mf"] {
            let command = launch_command(Path::new("/slicer"), Path::new(name));
            let file = Path::new(command.get_args().last().unwrap());
            assert!(file.is_absolute(), "{name} became {file:?}");
            assert!(file.ends_with(name), "{name} became {file:?}");
        }
    }

    #[test]
    fn macos_names_the_slicer_bundle_and_windows_runs_the_executable() {
        let file = std::env::temp_dir().join("a.3mf");
        let command = launch_command(Path::new("/Applications/X.app"), &file);
        let args: Vec<_> = command.get_args().collect();
        if cfg!(target_os = "macos") {
            assert_eq!(command.get_program(), "open");
            assert_eq!(
                args,
                [
                    "-a".as_ref(),
                    "/Applications/X.app".as_ref(),
                    file.as_os_str()
                ]
            );
        } else {
            assert_eq!(command.get_program(), "/Applications/X.app");
            assert_eq!(args, [file.as_os_str()]);
        }
    }

    #[test]
    fn refuses_to_launch_the_running_executable() {
        let me = std::env::current_exe().unwrap();
        let installed = InstalledSlicer {
            slicer: Slicer::OrcaSlicer,
            path: me.clone(),
        };
        let err = launch(&installed, Path::new("/nonexistent.3mf")).unwrap_err();
        assert!(matches!(err, LaunchError::WouldLaunchSelf(_)), "{err}");
    }

    #[test]
    fn refuses_a_folder_that_contains_the_running_executable() {
        let me = std::env::current_exe().unwrap();
        let installed = InstalledSlicer {
            slicer: Slicer::OrcaSlicer,
            path: me.parent().unwrap().to_owned(),
        };
        let err = launch(&installed, Path::new("/nonexistent.3mf")).unwrap_err();
        assert!(matches!(err, LaunchError::WouldLaunchSelf(_)), "{err}");
    }
}
