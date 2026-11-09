//! Routing overrides edited from the settings window.

use std::path::Path;

use slicer_router_core::{Config, Slicer};

/// Send files detected as `detected` to `slicer`, and save the config.
///
/// `None`, or `detected` itself, removes the override, so the file goes where
/// its own metadata says. A damaged config file is replaced.
pub fn set(config_path: &Path, detected: Slicer, slicer: Option<Slicer>) -> Result<(), String> {
    let mut config = Config::load_for_update(config_path).map_err(|e| e.to_string())?;
    apply(&mut config, detected, slicer);
    config.save(config_path).map_err(|e| e.to_string())
}

fn apply(config: &mut Config, detected: Slicer, slicer: Option<Slicer>) {
    match slicer.filter(|slicer| *slicer != detected) {
        Some(slicer) => {
            config.overrides.insert(detected, slicer);
        }
        None => {
            config.overrides.remove(&detected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Slicer::{BambuStudio, OrcaSlicer, PrusaSlicer};

    fn scratch(test: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "slicer-router-overrides-{}-{test}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn choosing_another_slicer_saves_an_override_and_keeps_the_rest_of_the_config() {
        let path = scratch("set");
        std::fs::write(
            &path,
            r#"{"default_slicer": "prusa-slicer", "theme": "dark"}"#,
        )
        .unwrap();
        set(&path, BambuStudio, Some(OrcaSlicer)).unwrap();

        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.overrides.get(&BambuStudio), Some(&OrcaSlicer));
        assert_eq!(saved.default_slicer, Some(PrusaSlicer));
        assert!(std::fs::read_to_string(&path).unwrap().contains("theme"));
    }

    #[test]
    fn choosing_the_detected_slicer_removes_the_override() {
        let path = scratch("same");
        set(&path, BambuStudio, Some(OrcaSlicer)).unwrap();
        set(&path, PrusaSlicer, Some(OrcaSlicer)).unwrap();
        set(&path, BambuStudio, Some(BambuStudio)).unwrap();

        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.overrides.get(&BambuStudio), None);
        assert_eq!(saved.overrides.get(&PrusaSlicer), Some(&OrcaSlicer));
    }

    #[test]
    fn choosing_nothing_removes_the_override() {
        let path = scratch("none");
        set(&path, BambuStudio, Some(OrcaSlicer)).unwrap();
        set(&path, BambuStudio, None).unwrap();
        assert!(Config::load(&path).unwrap().overrides.is_empty());
    }

    #[test]
    fn a_damaged_config_is_replaced() {
        let path = scratch("damaged");
        std::fs::write(&path, b"{ not json").unwrap();
        set(&path, BambuStudio, Some(OrcaSlicer)).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().overrides.get(&BambuStudio),
            Some(&OrcaSlicer)
        );
    }
}
