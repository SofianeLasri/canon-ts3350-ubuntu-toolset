//! Manual duplex: the TS3350 only prints one side, so the fronts are printed,
//! the user flips the stack, then the backs are printed.

mod passes;
mod plan;
mod testpage;

pub use passes::SourceDocument;
pub use plan::{Binding, DuplexProfile, PassPlan, Slot, plan};
pub use testpage::calibration_pdf;
