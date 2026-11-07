//! Files written by the real slicers. See `tests/fixtures/README.md`.

use std::path::PathBuf;

use slicer_router_core::{detect, Rule, Slicer};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn real_slicer_output_is_attributed_to_the_right_slicer() {
    let cases = [
        (
            "real-bambu-studio-cli.3mf",
            Slicer::BambuStudio,
            Rule::ApplicationName,
        ),
        (
            "real-bambu-studio-p1s.3mf",
            Slicer::BambuStudio,
            Rule::PrinterProfile,
        ),
        (
            "real-orcaslicer-cli.3mf",
            Slicer::OrcaSlicer,
            Rule::ApplicationBambuFork,
        ),
        (
            "real-snapmaker-orca-cli.3mf",
            Slicer::SnapmakerOrca,
            Rule::ApplicationName,
        ),
        (
            "real-snapmaker-orca-u1.3mf",
            Slicer::SnapmakerOrca,
            Rule::PrinterProfile,
        ),
        (
            "real-prusaslicer-cli.3mf",
            Slicer::PrusaSlicer,
            Rule::ApplicationName,
        ),
    ];
    for (name, slicer, rule) in cases {
        let verdict = detect(&fixture(name)).unwrap();
        assert_eq!(
            (verdict.slicer, verdict.rule),
            (Some(slicer), rule),
            "{name}: {:?}",
            verdict.evidence
        );
        assert!(verdict.evidence.warnings.is_empty(), "{name}");
    }
}
