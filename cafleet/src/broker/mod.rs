//! Broker — the synchronous data-access layer (SPEC §6.2). The contract each
//! submodule must satisfy is pinned by its colocated `#[cfg(test)]` suite; the
//! expected public API is catalogued in [`test_support`].

pub mod asset_installs;
pub mod fleets;
pub mod members;
pub mod messaging;
pub mod monitor;
pub mod queries;
pub mod records;
#[cfg(test)]
pub mod test_support;

pub use asset_installs::*;
pub use fleets::*;
pub use members::*;
pub use messaging::*;
pub use monitor::*;
pub use queries::*;
