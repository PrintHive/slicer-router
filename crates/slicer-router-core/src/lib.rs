//! Detect which slicer wrote a `.3mf` project and route it to the right
//! installed slicer.

mod config;
mod detect;
mod discover;
mod evidence;
mod launch;
mod route;
mod slicer;
mod snapshot;

pub use config::{Config, ConfigError, CONFIG_PATH_ENV};
pub use detect::{classify, detect, Rule, Verdict};
pub use discover::{discover_installed, InstalledSlicer};
pub use evidence::{read_evidence, read_thumbnail, DetectError, Evidence, PrinterProfile};
pub use launch::{launch, LaunchError};
pub use route::{route, Route, RouteReason};
pub use slicer::{PrinterPatterns, Slicer, SlicerSpec, UnknownSlicer, WindowsHints};
pub use snapshot::{Snapshot, APP_GROUP};
