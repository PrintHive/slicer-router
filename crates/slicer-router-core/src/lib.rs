//! Detect which slicer wrote a `.3mf` project and route it to the right
//! installed slicer.

mod slicer;

pub use slicer::{PrinterPatterns, Slicer, SlicerSpec, UnknownSlicer, WindowsHints};
