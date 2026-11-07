//! Decide which slicer a `.3mf` belongs to, from the evidence in the file.
//!
//! Rules run in the order of [`RULES`]. The first one that names a slicer wins.
//! "Printer profile wins" is that ordering: the printer a project was prepared
//! for outranks which program happened to save it.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::evidence::{read_evidence, DetectError, Evidence};
use crate::slicer::Slicer;

/// Which rule produced a [`Verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Rule {
    /// The printer model or profile name belongs to one slicer's vendor.
    PrinterProfile,
    /// The `Application` metadata names a slicer unambiguously.
    ApplicationName,
    /// The `Application` metadata is `BambuStudio-<version>` with a version
    /// that is not Bambu Studio's four-part release number. OrcaSlicer and its
    /// forks write that tag for compatibility, so the file is treated as OrcaSlicer's.
    ApplicationBambuFork,
    /// The archive has PrusaSlicer's `Slic3r_PE.config` and nothing else decided.
    PrusaConfigFile,
    /// No signal. The user's default slicer should open the file.
    NoSignal,
}

/// The outcome of [`detect`] or [`classify`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Verdict {
    /// The slicer the file belongs to, or `None` when nothing identified it.
    pub slicer: Option<Slicer>,
    /// The rule that decided, for debugging and `--json` output.
    pub rule: Rule,
    /// What was read from the file.
    pub evidence: Evidence,
}

type RuleFn = fn(&Evidence) -> Option<(Slicer, Rule)>;

/// Detection rules in priority order.
const RULES: [RuleFn; 3] = [by_printer_profile, by_application, by_prusa_config_file];

/// Detect the slicer for the `.3mf` at `path`.
///
/// Fails only when the file is not a readable zip archive; see [`read_evidence`].
pub fn detect(path: &Path) -> Result<Verdict, DetectError> {
    Ok(classify(read_evidence(path)?))
}

/// Apply the detection rules to already-read evidence.
pub fn classify(evidence: Evidence) -> Verdict {
    let fired = RULES.iter().find_map(|rule| rule(&evidence));
    match fired {
        Some((slicer, rule)) => Verdict {
            slicer: Some(slicer),
            rule,
            evidence,
        },
        None => Verdict {
            slicer: None,
            rule: Rule::NoSignal,
            evidence,
        },
    }
}

impl Verdict {
    /// A verdict with no signal, for callers that choose to route a file they
    /// could not read the same way as a file with no metadata.
    pub fn no_signal() -> Self {
        classify(Evidence::default())
    }
}

/// Printer fields are tried most specific first: the model, then the preset name,
/// then the vendor. A preset name is free text that people extend with other
/// brands ("MK4S for Bambu filament"), so it must not outvote the model.
fn by_printer_profile(evidence: &Evidence) -> Option<(Slicer, Rule)> {
    let printer = &evidence.printer;
    let lower = |field: &Option<String>| field.as_deref().map(str::to_ascii_lowercase);
    let tiers = [
        (lower(&printer.model), true),
        (lower(&printer.settings_id), false),
        (lower(&printer.vendor), false),
    ];
    tiers
        .iter()
        .filter_map(|(name, is_model)| Some((name.as_deref()?, *is_model)))
        .find_map(|(name, is_model)| {
            Slicer::ALL.iter().copied().find(|slicer| {
                let patterns = &slicer.spec().printers;
                patterns
                    .name_contains
                    .iter()
                    .any(|needle| name.contains(needle))
                    || (is_model && patterns.exact_models.contains(&name))
            })
        })
        .map(|slicer| (slicer, Rule::PrinterProfile))
}

/// How the version after an `Application` prefix must look for a row to match.
#[derive(Clone, Copy)]
enum Release {
    Any,
    /// Bambu Studio's four-part release, for example `02.08.02.61` or `02.10.00.100`.
    BambuFourPart,
    /// Any version that is not [`Release::BambuFourPart`].
    NotBambuFourPart,
}

impl Release {
    fn matches(self, version: &str) -> bool {
        let four_part = {
            let parts: Vec<&str> = version.split('.').collect();
            parts.len() == 4
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        };
        match self {
            Release::Any => true,
            Release::BambuFourPart => four_part,
            Release::NotBambuFourPart => !four_part,
        }
    }
}

/// `Application` metadata values, in match order.
///
/// OrcaSlicer 2.3 and Snapmaker Orca 2.2 write `BambuStudio-<their own version>`,
/// a three-part number, which is why the Bambu prefix has two rows. Bambu
/// Studio's own releases have four numeric parts, and a part may outgrow two
/// digits (`02.10.00.100`).
const APPLICATION_SIGNATURES: &[(&str, Release, Slicer, Rule)] = &[
    (
        "BambuStudio-",
        Release::BambuFourPart,
        Slicer::BambuStudio,
        Rule::ApplicationName,
    ),
    (
        "BambuStudio-",
        Release::NotBambuFourPart,
        Slicer::OrcaSlicer,
        Rule::ApplicationBambuFork,
    ),
    (
        "OrcaSlicer-",
        Release::Any,
        Slicer::OrcaSlicer,
        Rule::ApplicationName,
    ),
    (
        "Snapmaker_Orca-",
        Release::Any,
        Slicer::SnapmakerOrca,
        Rule::ApplicationName,
    ),
    (
        "PrusaSlicer-",
        Release::Any,
        Slicer::PrusaSlicer,
        Rule::ApplicationName,
    ),
];

fn by_application(evidence: &Evidence) -> Option<(Slicer, Rule)> {
    let application = evidence.application.as_deref()?;
    APPLICATION_SIGNATURES
        .iter()
        .find_map(|&(prefix, release, slicer, rule)| {
            let head = application.get(..prefix.len())?;
            let version = application.get(prefix.len()..)?;
            (head.eq_ignore_ascii_case(prefix) && release.matches(version))
                .then_some((slicer, rule))
        })
}

fn by_prusa_config_file(evidence: &Evidence) -> Option<(Slicer, Rule)> {
    evidence
        .prusa_config_present
        .then_some((Slicer::PrusaSlicer, Rule::PrusaConfigFile))
}
