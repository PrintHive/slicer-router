//! Choose a slicer for a verdict. Pure: no filesystem, no processes.

use serde::Serialize;

use crate::config::Config;
use crate::detect::Verdict;
use crate::slicer::Slicer;

/// Why a slicer was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum RouteReason {
    /// The config maps the detected slicer to this one.
    Override,
    /// The file belongs to this slicer and it is installed.
    Detected,
    /// Nothing usable was detected, so the configured default was used.
    Default,
    /// No default is set, but this is the only slicer installed.
    OnlyInstalled,
}

/// What to do with a file. Serializes with an `action` tag: `launch`, `ask` or
/// `none-installed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Route {
    /// Open the file in this slicer.
    Launch { slicer: Slicer, reason: RouteReason },
    /// Several slicers are installed and none was chosen. Ask the person which
    /// one to use, then store the answer with [`Config::remember_choice`].
    ///
    /// `detected` is the slicer the file belongs to when that slicer is not
    /// installed, so the question can say so. It is `None` for a file with no signal.
    Ask {
        choices: Vec<Slicer>,
        detected: Option<Slicer>,
    },
    /// No supported slicer is installed.
    NoneInstalled,
}

/// Decide where `verdict`'s file should open.
///
/// Candidates in order: the config override for the detected slicer, the
/// detected slicer, the configured default. The first one in `installed` wins.
/// With none available, a lone installed slicer is used, several are offered
/// as a choice, and zero is [`Route::NoneInstalled`].
pub fn route(verdict: &Verdict, installed: &[Slicer], config: &Config) -> Route {
    let overridden = verdict
        .slicer
        .and_then(|detected| config.overrides.get(&detected).copied());
    let candidates = [
        (overridden, RouteReason::Override),
        (verdict.slicer, RouteReason::Detected),
        (config.default_slicer, RouteReason::Default),
    ];
    let chosen = candidates
        .into_iter()
        .find_map(|(slicer, reason)| Some((slicer.filter(|s| installed.contains(s))?, reason)));
    match (chosen, installed) {
        (Some((slicer, reason)), _) => Route::Launch { slicer, reason },
        (None, []) => Route::NoneInstalled,
        (None, [only]) => Route::Launch {
            slicer: *only,
            reason: RouteReason::OnlyInstalled,
        },
        (None, choices) => Route::Ask {
            choices: choices.to_vec(),
            detected: verdict.slicer,
        },
    }
}
