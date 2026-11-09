//! Files written by the real slicers. See `tests/fixtures/README.md`.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use slicer_router_core::{detect, read_thumbnail, Rule, Slicer};

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

#[test]
fn real_previews_are_read_and_missing_ones_are_none() {
    let mut expected = Vec::new();
    zip::ZipArchive::new(File::open(fixture("real-bambu-studio-cli.3mf")).unwrap())
        .unwrap()
        .by_name("Metadata/plate_1.png")
        .unwrap()
        .read_to_end(&mut expected)
        .unwrap();
    assert_eq!(
        read_thumbnail(&fixture("real-bambu-studio-cli.3mf")).unwrap(),
        Some(expected)
    );

    // These name Metadata/plate_1.png or Metadata/thumbnail.png in _rels/.rels
    // but their CLIs never rendered it.
    for name in ["real-orcaslicer-cli.3mf", "real-prusaslicer-cli.3mf"] {
        assert_eq!(read_thumbnail(&fixture(name)).unwrap(), None, "{name}");
    }
}
