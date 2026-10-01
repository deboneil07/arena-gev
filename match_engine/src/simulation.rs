//! Backwards-compatible names for the master engine.
//!
//! The orchestrator now lives in [`crate::engine`] as
//! [`crate::engine::SimulationEngine`] (it gained the 3D ball). These aliases
//! keep earlier call sites working.

pub use crate::engine::{SimulationConfig, SimulationEngine as MatchSimulation};
