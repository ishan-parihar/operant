pub mod builtin;
mod dynamic;
pub mod harness_seam;
mod runner;
mod traits;

pub use dynamic::DynamicHooks;
pub use harness_seam::HooksSeam;
pub use runner::HookRunner;
// HookHandler and HookResult are part of the crate's public hook API surface.
// They may appear unused internally but are intentionally re-exported for
// external integrations and future plugin authors.
#[allow(unused_imports)]
pub use traits::{HookHandler, HookResult};
