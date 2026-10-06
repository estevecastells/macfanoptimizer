//! Hardware abstraction implemented by the real SMC backend (in `fand`) and
//! by the thermal simulator (in [`crate::sim`]).

use crate::sensors::SensorKey;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanInfo {
    pub index: u8,
    pub min_rpm: f64,
    pub max_rpm: f64,
}

impl FanInfo {
    /// Map a duty percentage onto this fan's RPM range.
    pub fn rpm_for_duty(&self, pct: f64) -> f64 {
        self.min_rpm + (self.max_rpm - self.min_rpm) * pct.clamp(0.0, 100.0) / 100.0
    }

    pub fn clamp(&self, rpm: f64) -> f64 {
        rpm.clamp(self.min_rpm, self.max_rpm)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanReading {
    pub actual_rpm: f64,
    pub target_rpm: f64,
    /// True when the fan is in forced/manual mode (someone is overriding macOS).
    pub forced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub key: SensorKey,
    pub celsius: f64,
}

pub trait Hardware {
    fn fans(&self) -> &[FanInfo];

    /// All temperature sensor keys this machine exposes (discovered once).
    fn temperature_keys(&self) -> &[SensorKey];

    /// Read one temperature sensor. `None` on read failure.
    fn read_temperature(&mut self, key: SensorKey) -> Option<f64>;

    fn read_fan(&mut self, index: u8) -> Result<FanReading, String>;

    /// Put the fan in forced mode at `rpm`.
    fn set_target(&mut self, index: u8, rpm: f64) -> Result<(), String>;

    /// Return the fan to macOS control.
    fn release(&mut self, index: u8) -> Result<(), String>;
}
