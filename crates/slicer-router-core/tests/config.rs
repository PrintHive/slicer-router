use std::path::PathBuf;

use slicer_router_core::{Config, ConfigError, Slicer};
use tempfile::TempDir;

fn config_file(contents: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, contents).unwrap();
    (dir, path)
}

fn saved_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn config_round_trips_through_json_on_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested/config.json");
    assert_eq!(Config::load(&path).unwrap(), Config::default());

    let mut config = Config::default();
    config.default_slicer = Some(Slicer::SnapmakerOrca);
    config
        .overrides
        .insert(Slicer::BambuStudio, Slicer::OrcaSlicer);
    config.save(&path).unwrap();

    let json = saved_json(&path);
    assert_eq!(json["default_slicer"], "snapmaker-orca");
    assert_eq!(json["overrides"]["bambu-studio"], "orca-slicer");
    assert_eq!(Config::load(&path).unwrap(), config);
}

#[test]
fn a_partial_file_fills_in_the_rest() {
    let (_dir, path) = config_file(r#"{"default_slicer":"prusa-slicer"}"#);
    let config = Config::load(&path).unwrap();
    assert_eq!(config.default_slicer, Some(Slicer::PrusaSlicer));
    assert!(config.overrides.is_empty() && config.warnings.is_empty());
}

#[test]
fn an_unknown_default_slicer_is_skipped_with_a_warning() {
    let (_dir, path) =
        config_file(r#"{"default_slicer":"cura","overrides":{"bambu-studio":"orca-slicer"}}"#);
    let config = Config::load(&path).unwrap();
    assert_eq!(config.default_slicer, None);
    assert_eq!(
        config.overrides.get(&Slicer::BambuStudio),
        Some(&Slicer::OrcaSlicer)
    );
    assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
    assert!(config.warnings[0].contains("cura"), "{:?}", config.warnings);
}

#[test]
fn unknown_override_keys_and_values_are_dropped_with_warnings() {
    let (_dir, path) = config_file(
        r#"{"overrides":{"cura":"orca-slicer","bambu-studio":"kiri-moto","prusa-slicer":"orca-slicer"}}"#,
    );
    let config = Config::load(&path).unwrap();
    let kept: Vec<_> = config.overrides.iter().collect();
    assert_eq!(kept, [(&Slicer::PrusaSlicer, &Slicer::OrcaSlicer)]);
    assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
}

#[test]
fn values_of_the_wrong_type_are_skipped_not_fatal() {
    let (_dir, path) = config_file(r#"{"default_slicer":5,"overrides":["x"]}"#);
    let config = Config::load(&path).unwrap();
    assert_eq!(config.default_slicer, None);
    assert!(config.overrides.is_empty());
    assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
}

#[test]
fn unknown_top_level_keys_survive_load_and_save() {
    let (_dir, path) =
        config_file(r#"{"default_slicer":"orca-slicer","theme":"dark","future":{"a":[1,2]}}"#);
    let mut config = Config::load(&path).unwrap();
    config.default_slicer = Some(Slicer::PrusaSlicer);
    config.save(&path).unwrap();

    let json = saved_json(&path);
    assert_eq!(json["default_slicer"], "prusa-slicer");
    assert_eq!(json["theme"], "dark");
    assert_eq!(json["future"]["a"][1], 2);
}

#[test]
fn warnings_are_not_written_back() {
    let (_dir, path) = config_file(r#"{"default_slicer":"cura"}"#);
    let config = Config::load(&path).unwrap();
    config.save(&path).unwrap();
    let json = saved_json(&path);
    assert!(json.get("warnings").is_none(), "{json}");
    assert!(Config::load(&path).unwrap().warnings.is_empty());
}

#[test]
fn a_file_that_is_not_a_json_object_is_a_parse_error() {
    for contents in ["{ not json", "[]", "\"orca-slicer\""] {
        let (_dir, path) = config_file(contents);
        assert!(
            matches!(Config::load(&path), Err(ConfigError::Parse { .. })),
            "{contents}"
        );
    }
}

#[test]
fn load_for_update_replaces_a_damaged_file_but_keeps_a_good_one() {
    let (_dir, path) = config_file("{ not json");
    let mut config = Config::load_for_update(&path).unwrap();
    assert_eq!(config, Config::default());
    config.default_slicer = Some(Slicer::OrcaSlicer);
    config.save(&path).unwrap();
    assert_eq!(Config::load(&path).unwrap(), config);

    std::fs::write(&path, r#"{"default_slicer":"prusa-slicer","theme":"dark"}"#).unwrap();
    let kept = Config::load_for_update(&path).unwrap();
    assert_eq!(kept.default_slicer, Some(Slicer::PrusaSlicer));
}

#[test]
fn load_for_update_still_fails_on_an_unreadable_path() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        Config::load_for_update(dir.path()),
        Err(ConfigError::Io { .. })
    ));
}
