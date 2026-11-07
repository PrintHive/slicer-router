//! Turn opened files into launches, questions and notices.

use std::path::PathBuf;

use slicer_router_core::{detect, route, Config, InstalledSlicer, Route, Slicer, Verdict};

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Files and the slicer that should open each.
    pub launches: Vec<(PathBuf, InstalledSlicer)>,
    /// Files that need the person to pick a slicer.
    pub ask: Vec<PathBuf>,
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
            Route::Ask { .. } => plan.ask.push(file.clone()),
            Route::NoneInstalled => plan.notices.push(format!(
                "No supported slicer is installed, so {} was not opened.",
                file.display()
            )),
        }
    }
    plan
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
            .map(|&slicer| InstalledSlicer {
                slicer,
                path: PathBuf::from(format!("/fake/{}", slicer.id())),
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
        assert_eq!(plan.ask, [unreadable]);
        assert!(plan.launches.is_empty());
    }

    #[test]
    fn unknown_file_uses_the_default_when_set() {
        let dir = tempfile_dir();
        let unreadable = dir.join("not-a-zip-2.3mf");
        std::fs::write(&unreadable, b"junk").unwrap();
        let config = Config {
            default_slicer: Some(PrusaSlicer),
            ..Config::default()
        };
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

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("slicer-router-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
