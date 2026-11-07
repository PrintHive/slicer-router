//! Detect which slicer wrote a `.3mf` project and route it to the right
//! installed slicer.

mod evidence;
mod slicer;

pub use evidence::{read_evidence, DetectError, Evidence, PrinterProfile};
pub use slicer::{PrinterPatterns, Slicer, SlicerSpec, UnknownSlicer, WindowsHints};
