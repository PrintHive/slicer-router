//! Detect which slicer wrote a `.3mf` project and route it to the right
//! installed slicer.

mod config;
mod detect;
mod evidence;
mod route;
mod slicer;

pub use config::{Config, ConfigError, CONFIG_PATH_ENV};
pub use detect::{classify, detect, Rule, Verdict};
pub use evidence::{read_evidence, DetectError, Evidence, PrinterProfile};
pub use route::{route, Route, RouteReason};
pub use slicer::{PrinterPatterns, Slicer, SlicerSpec, UnknownSlicer, WindowsHints};
