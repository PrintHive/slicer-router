//! The slicers this crate can route to, and the static table describing each.
//!
//! Everything slicer-specific lives in the `*_SPEC` statics below. Detection,
//! discovery and launching read the table and never branch on a slicer name.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A slicer that can open `.3mf` projects.
///
/// Variants are in display order. The serialized form is kebab-case
/// (`"snapmaker-orca"`) and matches [`Slicer::id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Slicer {
    BambuStudio,
    OrcaSlicer,
    SnapmakerOrca,
    PrusaSlicer,
}

/// How to recognise and find one slicer.
#[derive(Debug)]
pub struct SlicerSpec {
    /// Stable machine name, used in config files and JSON output.
    pub id: &'static str,
    /// Name shown to people.
    pub display_name: &'static str,
    /// `CFBundleIdentifier` values of the macOS app, compared after trimming
    /// whitespace and trailing slashes and ignoring case.
    pub mac_bundle_ids: &'static [&'static str],
    /// Where to look on Windows.
    pub windows: WindowsHints,
    /// Printer names that mean this slicer is the one the project was made for.
    pub printers: PrinterPatterns,
}

/// Windows discovery hints for one slicer.
#[derive(Debug)]
pub struct WindowsHints {
    /// Executable file names, in preference order.
    pub exe_names: &'static [&'static str],
    /// Install folders relative to `Program Files`, `Program Files (x86)` and
    /// `%LOCALAPPDATA%\Programs`.
    pub install_dirs: &'static [&'static str],
    /// Prefixes of the `DisplayName` value under the registry `Uninstall` keys.
    pub uninstall_display_names: &'static [&'static str],
}

/// Patterns that attribute a printer profile to a slicer. Empty for slicers
/// that do not ship their own printers (OrcaSlicer).
#[derive(Debug)]
pub struct PrinterPatterns {
    /// Lowercase substrings matched against printer model, profile name and vendor.
    pub name_contains: &'static [&'static str],
    /// Lowercase whole `printer_model` values for printers whose model name
    /// does not carry the vendor, such as PrusaSlicer's `MK4S`.
    pub exact_models: &'static [&'static str],
}

static BAMBU_STUDIO_SPEC: SlicerSpec = SlicerSpec {
    id: "bambu-studio",
    display_name: "Bambu Studio",
    mac_bundle_ids: &["com.bambulab.bambu-studio"],
    windows: WindowsHints {
        exe_names: &["bambu-studio.exe"],
        install_dirs: &["Bambu Studio"],
        uninstall_display_names: &["Bambu Studio"],
    },
    printers: PrinterPatterns {
        name_contains: &["bambu"],
        exact_models: &[],
    },
};

static ORCA_SLICER_SPEC: SlicerSpec = SlicerSpec {
    id: "orca-slicer",
    display_name: "OrcaSlicer",
    mac_bundle_ids: &["com.softfever3d.orca-slicer"],
    windows: WindowsHints {
        exe_names: &["orca-slicer.exe"],
        install_dirs: &["OrcaSlicer"],
        uninstall_display_names: &["OrcaSlicer"],
    },
    printers: PrinterPatterns {
        name_contains: &[],
        exact_models: &[],
    },
};

static SNAPMAKER_ORCA_SPEC: SlicerSpec = SlicerSpec {
    id: "snapmaker-orca",
    display_name: "Snapmaker Orca",
    mac_bundle_ids: &["com.snapmaker.snapmaker-orca"],
    windows: WindowsHints {
        exe_names: &["snapmaker-orca.exe"],
        install_dirs: &["Snapmaker_Orca", "Snapmaker Orca"],
        uninstall_display_names: &["Snapmaker Orca", "Snapmaker_Orca"],
    },
    printers: PrinterPatterns {
        name_contains: &["snapmaker"],
        exact_models: &[],
    },
};

static PRUSA_SLICER_SPEC: SlicerSpec = SlicerSpec {
    id: "prusa-slicer",
    display_name: "PrusaSlicer",
    mac_bundle_ids: &["com.prusa3d.slic3r"],
    windows: WindowsHints {
        exe_names: &["prusa-slicer.exe"],
        install_dirs: &["Prusa3D\\PrusaSlicer", "PrusaSlicer"],
        uninstall_display_names: &["PrusaSlicer"],
    },
    printers: PrinterPatterns {
        name_contains: &["prusa"],
        exact_models: &[
            "mk2", "mk2s", "mk2.5", "mk2.5s", "mk3", "mk3s", "mk3.5", "mk3.9", "mk4", "mk4s",
            "mk4is", "mini", "xl", "core one", "coreone", "core one l",
        ],
    },
};

impl Slicer {
    /// Every supported slicer, in display order.
    pub const ALL: [Slicer; 4] = [
        Slicer::BambuStudio,
        Slicer::OrcaSlicer,
        Slicer::SnapmakerOrca,
        Slicer::PrusaSlicer,
    ];

    /// The registry entry for this slicer.
    pub fn spec(self) -> &'static SlicerSpec {
        match self {
            Slicer::BambuStudio => &BAMBU_STUDIO_SPEC,
            Slicer::OrcaSlicer => &ORCA_SLICER_SPEC,
            Slicer::SnapmakerOrca => &SNAPMAKER_ORCA_SPEC,
            Slicer::PrusaSlicer => &PRUSA_SLICER_SPEC,
        }
    }

    /// Stable machine name such as `"bambu-studio"`.
    pub fn id(self) -> &'static str {
        self.spec().id
    }

    /// Name shown to people such as `"Bambu Studio"`.
    pub fn display_name(self) -> &'static str {
        self.spec().display_name
    }
}

impl fmt::Display for Slicer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Error from parsing a slicer id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown slicer `{0}`; expected one of: {choices}", choices = Slicer::ALL.map(Slicer::id).join(", "))]
pub struct UnknownSlicer(pub String);

impl FromStr for Slicer {
    type Err = UnknownSlicer;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.trim().to_ascii_lowercase();
        Slicer::ALL
            .into_iter()
            .find(|slicer| slicer.id() == wanted)
            .ok_or_else(|| UnknownSlicer(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_name_matches_registry_id() {
        for slicer in Slicer::ALL {
            let json = serde_json::to_string(&slicer).unwrap();
            assert_eq!(json, format!("\"{}\"", slicer.id()));
            assert_eq!(slicer.id().parse::<Slicer>().unwrap(), slicer);
        }
    }

    #[test]
    fn all_lists_every_variant_once() {
        let mut ids: Vec<_> = Slicer::ALL.iter().map(|s| s.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn unknown_slicer_names_the_choices() {
        let err = "cura".parse::<Slicer>().unwrap_err();
        assert!(err.to_string().contains("snapmaker-orca"), "{err}");
    }
}
