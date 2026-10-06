//! Hardware-independent core of MacFanOptimizer.
//!
//! Everything here is pure logic driven by an injected clock and a
//! [`hardware::Hardware`] implementation, so the full control loop can be
//! exercised against the thermal simulator in [`sim`] without touching fans.

pub mod config;
pub mod controller;
pub mod curve;
pub mod engine;
pub mod hardware;
pub mod protocol;
pub mod sensors;
pub mod sim;

pub use config::{Config, Mode, Profile};
pub use controller::{Controller, Decision};
pub use curve::Curve;
pub use engine::Engine;
pub use hardware::{FanInfo, FanReading, Hardware, Reading};
