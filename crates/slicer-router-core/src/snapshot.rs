//! The routing state that the macOS Finder thumbnail extension reads.
//!
//! The extension runs in a sandbox, so it can neither scan for installed slicers
//! nor read the config file. The app writes both into a snapshot beside the
//! config file, in the one folder the extension is entitled to read, and the
//! extension routes each file with the same [`route`] the app uses.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{mac_folder, Config};
use crate::detect::Verdict;
use crate::discover::InstalledSlicer;
use crate::route::{route, Route};
use crate::slicer::Slicer;

/// Changes when the file's shape does. A reader ignores any other version.
const VERSION: u32 = 1;
const FILE_NAME: &str = "routing-snapshot.json";

/// The installed slicers and the routing settings, as the app last saw them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Snapshot {
    version: u32,
    pub installed: Vec<InstalledSlicer>,
    /// Only the default slicer and the overrides.
    pub config: Config,
}

impl Snapshot {
    /// Capture `installed` and the routing settings of `config`.
    pub fn new(installed: Vec<InstalledSlicer>, config: &Config) -> Self {
        let mut routing = Config::default();
        routing.default_slicer = config.default_slicer;
        routing.overrides = config.overrides.clone();
        Snapshot {
            version: VERSION,
            installed,
            config: routing,
        }
    }

    /// The installed slicer that will open `verdict`'s file, or `None` when the
    /// router would ask which slicer to use or no slicer is installed.
    pub fn opener(&self, verdict: &Verdict) -> Option<&InstalledSlicer> {
        let slicers: Vec<Slicer> = self.installed.iter().map(|i| i.slicer).collect();
        match route(verdict, &slicers, &self.config) {
            Route::Launch { slicer, .. } => self.installed.iter().find(|i| i.slicer == slicer),
            _ => None,
        }
    }

    /// The snapshot saved in `dir`, or `None` when there is none, it cannot be
    /// read, or another version of the app wrote it.
    pub fn load(dir: &Path) -> Option<Snapshot> {
        let bytes = fs::read(dir.join(FILE_NAME)).ok()?;
        serde_json::from_slice::<Snapshot>(&bytes)
            .ok()
            .filter(|snapshot| snapshot.version == VERSION)
    }

    /// Save the snapshot in `dir`, creating the folder, unless `dir` already
    /// holds the same one. Returns whether the file changed. The file is
    /// replaced in one rename, so a reader never sees half of it.
    pub fn save(&self, dir: &Path) -> io::Result<bool> {
        let mut json = serde_json::to_string_pretty(self).expect("snapshot serializes");
        json.push('\n');
        let path = dir.join(FILE_NAME);
        if fs::read(&path).is_ok_and(|saved| saved == json.as_bytes()) {
            return Ok(false);
        }
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &path)?;
        Ok(true)
    }

    /// Where the app saves the snapshot on macOS: the folder that holds the
    /// config file, so it follows [`CONFIG_PATH_ENV`](crate::CONFIG_PATH_ENV) and a
    /// test config never replaces the real snapshot. `None` on other platforms,
    /// which have no thumbnail extension.
    pub fn default_dir() -> Option<PathBuf> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        Some(Config::default_path()?.parent()?.to_owned())
    }

    /// Where the extension reads the snapshot of the user whose home is `home`:
    /// `~/Library/Application Support/Slicer Router`. Its entitlements grant read
    /// access to that folder alone.
    pub fn dir_for_home(home: &Path) -> PathBuf {
        mac_folder(home)
    }
}
