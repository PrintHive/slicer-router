use slicer_router_core::{route, Config, Evidence, Route, RouteReason, Rule, Slicer, Verdict};
use Slicer::{BambuStudio, OrcaSlicer, PrusaSlicer, SnapmakerOrca};

fn verdict(slicer: Option<Slicer>) -> Verdict {
    Verdict {
        slicer,
        rule: if slicer.is_some() {
            Rule::ApplicationName
        } else {
            Rule::NoSignal
        },
        evidence: Evidence::default(),
    }
}

fn launch(slicer: Slicer, reason: RouteReason) -> Route {
    Route::Launch { slicer, reason }
}

#[test]
fn detected_slicer_opens_when_installed() {
    let route = route(
        &verdict(Some(SnapmakerOrca)),
        &[BambuStudio, SnapmakerOrca],
        &Config::default(),
    );
    assert_eq!(route, launch(SnapmakerOrca, RouteReason::Detected));
}

#[test]
fn missing_detected_slicer_falls_back_to_the_default() {
    let config = Config {
        default_slicer: Some(SnapmakerOrca),
        ..Config::default()
    };
    let route = route(
        &verdict(Some(OrcaSlicer)),
        &[SnapmakerOrca, PrusaSlicer],
        &config,
    );
    assert_eq!(route, launch(SnapmakerOrca, RouteReason::Default));
}

#[test]
fn undetected_file_uses_the_default() {
    let config = Config {
        default_slicer: Some(PrusaSlicer),
        ..Config::default()
    };
    let route = route(&verdict(None), &[BambuStudio, PrusaSlicer], &config);
    assert_eq!(route, launch(PrusaSlicer, RouteReason::Default));
}

#[test]
fn undetected_file_with_no_default_asks() {
    let route = route(
        &verdict(None),
        &[BambuStudio, PrusaSlicer],
        &Config::default(),
    );
    assert_eq!(
        route,
        Route::Ask {
            choices: vec![BambuStudio, PrusaSlicer]
        }
    );
}

#[test]
fn default_that_is_not_installed_asks() {
    let config = Config {
        default_slicer: Some(OrcaSlicer),
        ..Config::default()
    };
    let route = route(&verdict(None), &[BambuStudio, PrusaSlicer], &config);
    assert_eq!(
        route,
        Route::Ask {
            choices: vec![BambuStudio, PrusaSlicer]
        }
    );
}

#[test]
fn a_lone_installed_slicer_is_used_without_asking() {
    let route = route(&verdict(None), &[PrusaSlicer], &Config::default());
    assert_eq!(route, launch(PrusaSlicer, RouteReason::OnlyInstalled));
}

#[test]
fn nothing_installed_is_reported() {
    let config = Config {
        default_slicer: Some(PrusaSlicer),
        ..Config::default()
    };
    assert_eq!(
        route(&verdict(Some(BambuStudio)), &[], &config),
        Route::NoneInstalled
    );
}

#[test]
fn override_redirects_the_detected_slicer() {
    let mut config = Config::default();
    config.overrides.insert(BambuStudio, OrcaSlicer);
    let route = route(
        &verdict(Some(BambuStudio)),
        &[BambuStudio, OrcaSlicer],
        &config,
    );
    assert_eq!(route, launch(OrcaSlicer, RouteReason::Override));
}

#[test]
fn override_to_a_missing_slicer_keeps_the_detected_one() {
    let mut config = Config::default();
    config.overrides.insert(BambuStudio, OrcaSlicer);
    let route = route(&verdict(Some(BambuStudio)), &[BambuStudio], &config);
    assert_eq!(route, launch(BambuStudio, RouteReason::Detected));
}

#[test]
fn override_only_applies_to_its_own_slicer() {
    let mut config = Config::default();
    config.overrides.insert(BambuStudio, OrcaSlicer);
    let route = route(
        &verdict(Some(PrusaSlicer)),
        &[PrusaSlicer, OrcaSlicer],
        &config,
    );
    assert_eq!(route, launch(PrusaSlicer, RouteReason::Detected));
}

#[test]
fn config_round_trips_through_json_on_disk() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("nested/config.json");
    assert_eq!(Config::load(&path).unwrap(), Config::default());

    let mut config = Config {
        default_slicer: Some(SnapmakerOrca),
        ..Config::default()
    };
    config.overrides.insert(BambuStudio, OrcaSlicer);
    config.save(&path).unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(json["default_slicer"], "snapmaker-orca");
    assert_eq!(json["overrides"]["bambu-studio"], "orca-slicer");
    assert_eq!(Config::load(&path).unwrap(), config);
}

#[test]
fn partial_and_invalid_config_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, r#"{"default_slicer":"prusa-slicer"}"#).unwrap();
    assert_eq!(
        Config::load(&path).unwrap().default_slicer,
        Some(PrusaSlicer)
    );
    std::fs::write(&path, r#"{"default_slicer":"cura"}"#).unwrap();
    assert!(Config::load(&path).is_err());
}
