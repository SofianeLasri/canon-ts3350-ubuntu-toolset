//! eSCL protocol (AirScan / Mopria) spoken by the TS3350 scanner.

pub mod caps;
pub mod client;
pub mod discovery;
pub mod settings;
pub mod xml;

pub use caps::{Capabilities, ColorMode, InputCaps, Range, UNITS_PER_INCH, content_type_label, intent_label};
pub use client::{Client, ScannerState};
pub use discovery::{DiscoveredScanner, discover};
pub use settings::{Paper, Region, ScanSettings};
