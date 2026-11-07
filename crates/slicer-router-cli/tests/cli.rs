use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Run the binary with a throwaway config so the developer's own is never touched.
fn run(config_dir: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_slicer-router"))
        .args(args)
        .env(
            "SLICER_ROUTER_CONFIG",
            config_dir.path().join("config.json"),
        )
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn detect_json_reports_slicer_rule_and_evidence() {
    let dir = TempDir::new().unwrap();
    let file = fixture("real-snapmaker-orca-u1.3mf");
    let output = run(&dir, &["detect", file.to_str().unwrap(), "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["slicer"], "snapmaker-orca");
    assert_eq!(json["rule"], "printer-profile");
    assert_eq!(json["evidence"]["application"], "BambuStudio-2.2.4");
    assert_eq!(json["evidence"]["printer"]["model"], "Snapmaker U1");
}

#[test]
fn detect_prints_a_readable_verdict() {
    let dir = TempDir::new().unwrap();
    let file = fixture("real-orcaslicer-cli.3mf");
    let output = run(&dir, &["detect", file.to_str().unwrap()]);
    let text = stdout(&output);
    assert!(text.contains("orca-slicer"), "{text}");
    assert!(text.contains("application-bambu-fork"), "{text}");
    assert!(text.contains("BambuStudio-2.3.1"), "{text}");
}

#[test]
fn detect_fails_cleanly_on_a_corrupt_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("broken.3mf");
    std::fs::write(&file, b"not a zip").unwrap();
    let output = run(&dir, &["detect", file.to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a valid .3mf"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn slicers_json_is_a_list_of_known_slicers() {
    let dir = TempDir::new().unwrap();
    let output = run(&dir, &["slicers", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    for item in json.as_array().unwrap() {
        assert!(
            item["slicer"].is_string() && item["path"].is_string(),
            "{item}"
        );
    }
}

#[test]
fn default_is_stored_in_the_config_file() {
    let dir = TempDir::new().unwrap();
    assert!(stdout(&run(&dir, &["default"])).contains("No default"));
    assert!(run(&dir, &["default", "prusa-slicer"]).status.success());
    assert!(stdout(&run(&dir, &["default"])).contains("PrusaSlicer"));
    let saved = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
    assert!(saved.contains("prusa-slicer"), "{saved}");
    assert!(!run(&dir, &["default", "cura"]).status.success());
}

#[test]
fn open_dry_run_never_starts_a_slicer_and_names_its_choice() {
    let dir = TempDir::new().unwrap();
    let file = fixture("real-snapmaker-orca-u1.3mf");
    let output = run(&dir, &["open", file.to_str().unwrap(), "--dry-run"]);
    let text = stdout(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        assert!(text.contains("real-snapmaker-orca-u1.3mf ->"), "{text}");
    } else {
        // A machine without that slicer and with several others needs a default first.
        assert!(
            stderr.contains("no default slicer") || stderr.contains("no supported slicer"),
            "{stderr}"
        );
    }
}

#[test]
fn an_unknown_slicer_in_the_config_is_a_warning_not_an_error() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("config.json"),
        r#"{"default_slicer":"cura","theme":"dark"}"#,
    )
    .unwrap();
    let output = run(&dir, &["default"]);
    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("No default"), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cura"), "{stderr}");

    assert!(run(&dir, &["default", "orca-slicer"]).status.success());
    let saved = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
    assert!(
        saved.contains("orca-slicer") && saved.contains("theme"),
        "{saved}"
    );
}
