//! The router's small JSON settings file.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::slicer::Slicer;

/// Environment variable that replaces the default config file location.
pub const CONFIG_PATH_ENV: &str = "SLICER_ROUTER_CONFIG";

/// User choices that [`route`](crate::route) consults.
///
/// Stored as JSON, for example:
///
/// ```json
/// { "default_slicer": "orca-slicer", "overrides": { "bambu-studio": "orca-slicer" } }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Opens files that no rule could attribute to a slicer.
    pub default_slicer: Option<Slicer>,
    /// Detected slicer to the slicer that should open its files instead.
    pub overrides: BTreeMap<Slicer, Slicer>,
}

/// Why the config file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot access config {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("config {} is not valid: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl Config {
    /// Where the config lives: `$SLICER_ROUTER_CONFIG` if set, otherwise
    /// `~/Library/Application Support/Slicer Router/config.json` on macOS,
    /// `%APPDATA%\Slicer Router\config.json` on Windows and
    /// `$XDG_CONFIG_HOME/slicer-router/config.json` elsewhere.
    ///
    /// `None` when the home directory cannot be determined.
    pub fn default_path() -> Option<PathBuf> {
        config_path_from(|name| std::env::var_os(name))
    }

    /// Read the config at `path`. A missing file is the default config.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| ConfigError::Parse {
                path: path.to_owned(),
                source,
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(source) => Err(ConfigError::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Write the config to `path`, creating parent folders. The file is replaced
    /// in one rename, so a crash never leaves half a config behind.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let io_err = |source| ConfigError::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        let mut json = serde_json::to_string_pretty(self).expect("config serializes");
        json.push('\n');
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, json).map_err(io_err)?;
        fs::rename(&tmp, path).map_err(io_err)
    }
}

fn config_path_from(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = env(CONFIG_PATH_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let dir = if cfg!(target_os = "macos") {
        PathBuf::from(env("HOME")?)
            .join("Library/Application Support")
            .join("Slicer Router")
    } else if cfg!(windows) {
        PathBuf::from(env("APPDATA")?).join("Slicer Router")
    } else {
        let base = env("XDG_CONFIG_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        base.join("slicer-router")
    };
    Some(dir.join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        let path = config_path_from(|name| (name == CONFIG_PATH_ENV).then(|| "/tmp/x.json".into()));
        assert_eq!(path, Some(PathBuf::from("/tmp/x.json")));
    }

    #[test]
    fn default_path_ends_in_config_json() {
        let path = config_path_from(|name| match name {
            "HOME" => Some("/home/u".into()),
            "APPDATA" => Some("C:\\Users\\u\\AppData\\Roaming".into()),
            _ => None,
        })
        .unwrap();
        assert!(path.ends_with("config.json"), "{}", path.display());
        assert!(path.to_string_lossy().to_lowercase().contains("slicer"));
    }

    #[test]
    fn no_home_means_no_path() {
        assert_eq!(config_path_from(|_| None), None);
    }
}
