use std::path::PathBuf;

use slicer_router_core::{Config, InstalledSlicer, Rule, Slicer, Snapshot, Verdict};
use tempfile::TempDir;
use Slicer::{BambuStudio, OrcaSlicer, PrusaSlicer, SnapmakerOrca};

fn verdict(slicer: Option<Slicer>) -> Verdict {
    let mut verdict = Verdict::no_signal();
    if slicer.is_some() {
        verdict.slicer = slicer;
        verdict.rule = Rule::PrinterProfile;
    }
    verdict
}

fn installed(slicers: &[Slicer]) -> Vec<InstalledSlicer> {
    slicers
        .iter()
        .map(|&slicer| {
            InstalledSlicer::new(slicer, PathBuf::from(format!("/Apps/{}.app", slicer.id())))
        })
        .collect()
}

fn opener(snapshot: &Snapshot, detected: Option<Slicer>) -> Option<Slicer> {
    snapshot.opener(&verdict(detected)).map(|i| i.slicer)
}

#[test]
fn the_opener_is_the_slicer_the_router_launches() {
    let mut config = Config::default();
    config.overrides.insert(BambuStudio, OrcaSlicer);
    let snapshot = Snapshot::new(installed(&[BambuStudio, OrcaSlicer, PrusaSlicer]), &config);

    assert_eq!(opener(&snapshot, Some(PrusaSlicer)), Some(PrusaSlicer));
    assert_eq!(opener(&snapshot, Some(BambuStudio)), Some(OrcaSlicer));
    assert_eq!(
        snapshot.opener(&verdict(Some(PrusaSlicer))).unwrap().path,
        PathBuf::from("/Apps/prusa-slicer.app")
    );
}

#[test]
fn a_file_the_router_would_ask_about_has_no_opener() {
    let none_set = Snapshot::new(installed(&[BambuStudio, PrusaSlicer]), &Config::default());
    assert_eq!(opener(&none_set, None), None, "no signal and no default");
    assert_eq!(
        opener(&none_set, Some(SnapmakerOrca)),
        None,
        "detected slicer missing"
    );

    let nothing_installed = Snapshot::new(Vec::new(), &Config::default());
    assert_eq!(opener(&nothing_installed, Some(BambuStudio)), None);
}

#[test]
fn the_default_or_a_lone_slicer_opens_files_with_no_signal() {
    let mut config = Config::default();
    config.default_slicer = Some(PrusaSlicer);
    let with_default = Snapshot::new(installed(&[BambuStudio, PrusaSlicer]), &config);
    assert_eq!(opener(&with_default, None), Some(PrusaSlicer));

    let lone = Snapshot::new(installed(&[OrcaSlicer]), &Config::default());
    assert_eq!(opener(&lone, None), Some(OrcaSlicer));
}

#[test]
fn a_saved_snapshot_loads_back_and_routes_the_same() {
    let dir = TempDir::new().unwrap();
    let mut config = Config::default();
    config.default_slicer = Some(SnapmakerOrca);
    let snapshot = Snapshot::new(installed(&[BambuStudio, SnapmakerOrca]), &config);

    assert!(snapshot.save(&dir.path().join("group")).unwrap());
    let loaded = Snapshot::load(&dir.path().join("group")).unwrap();
    assert_eq!(loaded, snapshot);
    assert_eq!(opener(&loaded, None), Some(SnapmakerOrca));
}

#[test]
fn saving_reports_whether_the_routing_changed() {
    let dir = TempDir::new().unwrap();
    let mut config = Config::default();
    let slicers = installed(&[BambuStudio, PrusaSlicer]);

    assert!(Snapshot::new(slicers.clone(), &config)
        .save(dir.path())
        .unwrap());
    assert!(!Snapshot::new(slicers.clone(), &config)
        .save(dir.path())
        .unwrap());

    config.default_slicer = Some(BambuStudio);
    assert!(Snapshot::new(slicers.clone(), &config)
        .save(dir.path())
        .unwrap());
    assert!(Snapshot::new(installed(&[BambuStudio]), &config)
        .save(dir.path())
        .unwrap());
}

#[test]
fn settings_that_do_not_route_stay_out_of_the_snapshot() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        r#"{"default_slicer":"orca-slicer","theme":"dark"}"#,
    )
    .unwrap();
    let config = Config::load(&config_path).unwrap();
    let snapshot_dir = dir.path().join("group");
    Snapshot::new(installed(&[OrcaSlicer]), &config)
        .save(&snapshot_dir)
        .unwrap();

    let saved = std::fs::read_to_string(snapshot_dir.join("routing-snapshot.json")).unwrap();
    assert!(!saved.contains("theme"), "{saved}");
    assert!(saved.contains("orca-slicer"), "{saved}");
}

#[test]
fn a_missing_damaged_or_other_version_snapshot_is_none() {
    let dir = TempDir::new().unwrap();
    assert_eq!(Snapshot::load(dir.path()), None, "missing");

    let file = dir.path().join("routing-snapshot.json");
    std::fs::write(&file, b"{ not json").unwrap();
    assert_eq!(Snapshot::load(dir.path()), None, "damaged");

    std::fs::write(&file, br#"{"version":2,"installed":[],"config":{}}"#).unwrap();
    assert_eq!(Snapshot::load(dir.path()), None, "another version");

    std::fs::write(&file, br#"{"version":1,"installed":[],"config":{}}"#).unwrap();
    assert!(Snapshot::load(dir.path()).is_some(), "this version");
}

#[test]
fn the_snapshot_lives_in_the_app_group_container_on_macos() {
    let dir = Snapshot::default_dir();
    if cfg!(target_os = "macos") {
        let dir = dir.expect("HOME is set");
        assert!(
            dir.ends_with("Library/Group Containers/L5C8455NJH.com.printhive.slicer-router"),
            "{}",
            dir.display()
        );
    } else {
        assert_eq!(dir, None);
    }
}
