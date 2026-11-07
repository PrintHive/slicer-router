//! Files that wait for a person to choose a slicer.

use std::path::{Path, PathBuf};

use slicer_router_core::{Config, Slicer};

/// A file the router could not place on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingFile {
    pub path: PathBuf,
    /// The slicer the file belongs to when that slicer is not installed, or
    /// `None` for a file with no signal. The answer is saved against this.
    pub detected: Option<Slicer>,
}

/// Pending files grouped by what they were detected as, in first-seen order.
/// Each group is asked about once.
pub fn groups(pending: &[PendingFile]) -> Vec<(Option<Slicer>, Vec<&Path>)> {
    let mut groups: Vec<(Option<Slicer>, Vec<&Path>)> = Vec::new();
    for file in pending {
        match groups
            .iter_mut()
            .find(|(detected, _)| *detected == file.detected)
        {
            Some((_, files)) => files.push(&file.path),
            None => groups.push((file.detected, vec![&file.path])),
        }
    }
    groups
}

/// Save the answer for one group and take that group's files out of `pending`.
///
/// A file with a detected slicer saves an override for it and a file with no
/// signal sets the default slicer (see [`Config::remember_choice`]). A damaged
/// config file is replaced. If saving fails, `pending` is left as it was so
/// nothing is lost. Returns the files to open in `slicer`.
pub fn resolve(
    config_path: &Path,
    pending: &mut Vec<PendingFile>,
    detected: Option<Slicer>,
    slicer: Slicer,
) -> Result<Vec<PathBuf>, String> {
    let mut config = Config::load_for_update(config_path).map_err(|e| e.to_string())?;
    config.remember_choice(detected, slicer);
    config.save(config_path).map_err(|e| e.to_string())?;

    let (released, kept): (Vec<_>, Vec<_>) = std::mem::take(pending)
        .into_iter()
        .partition(|file| file.detected == detected);
    *pending = kept;
    Ok(released.into_iter().map(|file| file.path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use Slicer::{BambuStudio, PrusaSlicer, SnapmakerOrca};

    fn file(name: &str, detected: Option<Slicer>) -> PendingFile {
        PendingFile {
            path: PathBuf::from(name),
            detected,
        }
    }

    /// A fresh folder for one test, so tests never share a config file.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "slicer-router-pending-{}-{test}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_are_grouped_by_what_they_were_detected_as() {
        let pending = [
            file("a.3mf", None),
            file("b.3mf", Some(SnapmakerOrca)),
            file("c.3mf", None),
        ];
        let grouped = groups(&pending);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].0, None);
        assert_eq!(grouped[0].1, [Path::new("a.3mf"), Path::new("c.3mf")]);
        assert_eq!(grouped[1].0, Some(SnapmakerOrca));
    }

    #[test]
    fn answering_a_missing_slicers_files_saves_an_override_and_keeps_the_rest_waiting() {
        let path = scratch("override").join("config.json");
        let mut pending = vec![
            file("snap.3mf", Some(SnapmakerOrca)),
            file("junk.3mf", None),
        ];
        let released = resolve(&path, &mut pending, Some(SnapmakerOrca), PrusaSlicer).unwrap();

        assert_eq!(released, [PathBuf::from("snap.3mf")]);
        assert_eq!(pending, [file("junk.3mf", None)]);
        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.default_slicer, None);
        assert_eq!(saved.overrides.get(&SnapmakerOrca), Some(&PrusaSlicer));
    }

    #[test]
    fn answering_for_files_with_no_signal_sets_the_default() {
        let path = scratch("default").join("config.json");
        let mut pending = vec![
            file("junk.3mf", None),
            file("snap.3mf", Some(SnapmakerOrca)),
        ];
        let released = resolve(&path, &mut pending, None, BambuStudio).unwrap();

        assert_eq!(released, [PathBuf::from("junk.3mf")]);
        assert_eq!(pending, [file("snap.3mf", Some(SnapmakerOrca))]);
        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.default_slicer, Some(BambuStudio));
        assert!(saved.overrides.is_empty());
    }

    #[test]
    fn a_damaged_config_is_replaced_by_the_answer() {
        let path = scratch("damaged").join("config.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let mut pending = vec![file("junk.3mf", None)];
        resolve(&path, &mut pending, None, PrusaSlicer).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().default_slicer,
            Some(PrusaSlicer)
        );
    }

    #[test]
    fn a_config_that_cannot_be_updated_leaves_the_files_waiting() {
        // A folder where the config file belongs can be neither read nor replaced.
        let path = scratch("blocked").join("config.json");
        std::fs::create_dir_all(&path).unwrap();
        let mut pending = vec![file("junk.3mf", None)];
        assert!(resolve(&path, &mut pending, None, PrusaSlicer).is_err());
        assert_eq!(pending, [file("junk.3mf", None)]);
    }
}
