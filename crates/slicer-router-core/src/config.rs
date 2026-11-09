//! The router's small JSON settings file.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
///
/// Loading is lenient so that a newer or hand-edited file never locks the
/// router out: a slicer id this version does not know is skipped with an entry
/// in [`Config::warnings`], and top-level keys it does not know are kept and
/// written back by [`Config::save`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawConfig")]
#[non_exhaustive]
pub struct Config {
    /// Opens files that no rule could attribute to a slicer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_slicer: Option<Slicer>,
    /// Detected slicer to the slicer that should open its files instead.
    pub overrides: BTreeMap<Slicer, Slicer>,
    /// What [`Config::load`] skipped, in plain words. Never saved.
    #[serde(skip)]
    pub warnings: Vec<String>,
    /// Top-level keys this version does not use, kept so saving does not erase them.
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// The file as written, before unknown slicer ids are filtered out.
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    default_slicer: Option<Value>,
    overrides: Option<Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl From<RawConfig> for Config {
    fn from(raw: RawConfig) -> Self {
        let mut warnings = Vec::new();
        let default_slicer = raw
            .default_slicer
            .and_then(|value| known_slicer(&value, "default_slicer", &mut warnings));
        let mut overrides = BTreeMap::new();
        match raw.overrides {
            None | Some(Value::Null) => {}
            Some(Value::Object(entries)) => {
                for (detected, replacement) in entries {
                    let detected_slicer = known_slicer(
                        &Value::String(detected.clone()),
                        "overrides key",
                        &mut warnings,
                    );
                    let replacement_slicer = known_slicer(
                        &replacement,
                        &format!("overrides[{detected}]"),
                        &mut warnings,
                    );
                    if let (Some(detected), Some(replacement)) =
                        (detected_slicer, replacement_slicer)
                    {
                        overrides.insert(detected, replacement);
                    }
                }
            }
            Some(other) => warnings.push(format!(
                "overrides: expected an object of slicer ids, got {other}; ignored"
            )),
        }
        Config {
            default_slicer,
            overrides,
            warnings,
            extra: raw.extra,
        }
    }
}

/// `value` as a slicer, or `None` with a warning naming `what` it was for.
fn known_slicer(value: &Value, what: &str, warnings: &mut Vec<String>) -> Option<Slicer> {
    let parsed = match value {
        Value::Null => return None,
        Value::String(id) => id.parse::<Slicer>().map_err(|err| err.to_string()),
        other => Err(format!("{other} is not a slicer id")),
    };
    parsed
        .map_err(|reason| warnings.push(format!("{what}: {reason}; ignored")))
        .ok()
}

/// Why the config file could not be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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

    /// Record the answer to a [`Route::Ask`](crate::Route::Ask).
    ///
    /// A file that belongs to a slicer that is not installed gets an override for
    /// that slicer, so only its files change slicer. A file with no signal sets
    /// the default slicer.
    pub fn remember_choice(&mut self, detected: Option<Slicer>, chosen: Slicer) {
        match detected {
            Some(detected) => {
                self.overrides.insert(detected, chosen);
            }
            None => self.default_slicer = Some(chosen),
        }
    }

    /// Read the config at `path`. A missing file is the default config.
    ///
    /// Fails only when the file cannot be read or is not a JSON object. Slicer
    /// ids this version does not know are reported in [`Config::warnings`].
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

    /// Like [`Config::load`], but a file that is not valid JSON counts as empty.
    ///
    /// Use it before changing a setting and saving, so choosing a slicer repairs
    /// a damaged file instead of failing on it. A file that cannot be read still
    /// fails, because saving over it would risk losing it.
    pub fn load_for_update(path: &Path) -> Result<Config, ConfigError> {
        match Config::load(path) {
            Err(ConfigError::Parse { .. }) => Ok(Config::default()),
            loaded => loaded,
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

/// The macOS folder for the config, and the thumbnail snapshot beside it, of the
/// user whose home is `home`.
pub(crate) fn mac_folder(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Slicer Router")
}

fn config_path_from(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = env(CONFIG_PATH_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let dir = if cfg!(target_os = "macos") {
        mac_folder(Path::new(&env("HOME")?))
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
