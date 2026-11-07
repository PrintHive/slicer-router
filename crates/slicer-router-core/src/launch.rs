//! Start a slicer on a file.

use std::io;
use std::path::Path;
use std::process::Command;

use crate::discover::InstalledSlicer;

/// Why a slicer could not be started.
#[derive(Debug, thiserror::Error)]
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

/// Open `file` in `slicer` and return once the slicer has been handed the file.
///
/// macOS runs `open -a <bundle> <file>`, naming the slicer's own bundle so the
/// system never picks the router as the handler. Windows starts the slicer
/// executable with the file as its argument.
///
/// Refuses, with [`LaunchError::WouldLaunchSelf`], any target that contains the
/// running executable, so a misconfigured entry cannot make the router relaunch itself.
pub fn launch(slicer: &InstalledSlicer, file: &Path) -> Result<(), LaunchError> {
    refuse_self(&slicer.path)?;
    let spawn_err = |source| LaunchError::Spawn {
        slicer: slicer.clone(),
        source,
    };
    if cfg!(target_os = "macos") {
        let status = Command::new("open")
            .arg("-a")
            .arg(&slicer.path)
            .arg(file)
            .status()
            .map_err(spawn_err)?;
        if !status.success() {
            return Err(LaunchError::Failed {
                slicer: slicer.clone(),
                status,
            });
        }
    } else {
        Command::new(&slicer.path)
            .arg(file)
            .spawn()
            .map_err(spawn_err)?;
    }
    Ok(())
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
