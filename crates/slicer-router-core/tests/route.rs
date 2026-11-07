use slicer_router_core::{route, Config, Route, RouteReason, Rule, Slicer, Verdict};
use Slicer::{BambuStudio, OrcaSlicer, PrusaSlicer, SnapmakerOrca};

fn verdict(slicer: Option<Slicer>) -> Verdict {
    let mut verdict = Verdict::no_signal();
    if slicer.is_some() {
        verdict.slicer = slicer;
        verdict.rule = Rule::ApplicationName;
    }
    verdict
}

fn config_with_default(slicer: Slicer) -> Config {
    let mut config = Config::default();
    config.default_slicer = Some(slicer);
    config
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
    let config = config_with_default(SnapmakerOrca);
    let route = route(
        &verdict(Some(OrcaSlicer)),
        &[SnapmakerOrca, PrusaSlicer],
        &config,
    );
    assert_eq!(route, launch(SnapmakerOrca, RouteReason::Default));
}

#[test]
fn undetected_file_uses_the_default() {
    let config = config_with_default(PrusaSlicer);
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
    let config = config_with_default(OrcaSlicer);
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
    let config = config_with_default(PrusaSlicer);
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
fn routes_serialize_with_an_action_tag_and_kebab_case_names() {
    let launch = route(
        &verdict(Some(PrusaSlicer)),
        &[PrusaSlicer],
        &Config::default(),
    );
    assert_eq!(
        serde_json::to_value(&launch).unwrap(),
        serde_json::json!({"action": "launch", "slicer": "prusa-slicer", "reason": "detected"})
    );
    let ask = route(
        &verdict(None),
        &[BambuStudio, PrusaSlicer],
        &Config::default(),
    );
    assert_eq!(
        serde_json::to_value(&ask).unwrap(),
        serde_json::json!({"action": "ask", "choices": ["bambu-studio", "prusa-slicer"]})
    );
    let none = route(&verdict(None), &[], &Config::default());
    assert_eq!(
        serde_json::to_value(&none).unwrap(),
        serde_json::json!({"action": "none-installed"})
    );
    assert_eq!(
        serde_json::to_value(RouteReason::OnlyInstalled).unwrap(),
        "only-installed"
    );
}
