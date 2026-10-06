//! Persistent daemon configuration (TOML on disk, JSON over the socket).

use crate::curve::Curve;
use serde::{Deserialize, Serialize};

/// What the daemon does with the fans.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mode {
    /// Hands-off: macOS (thermalmonitord) controls the fans.
    System,
    /// Temperature-driven control using the active profile's curve.
    Smart,
    /// Constant speed for all fans. Safety overrides still apply.
    Fixed { rpm: f64 },
    /// All fans at maximum.
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Quiet,
    Balanced,
    Performance,
    Custom,
}

impl Profile {
    /// Built-in curves, tuned for an M5 Pro MacBook Pro (Mac17,9).
    ///
    /// Measured on that machine: idle die ≈ 45–55 °C, all-core load ≈ 65–70 °C
    /// with fans at 6000 rpm. Apple Silicon throttles around 100–105 °C.
    pub fn builtin_curve(self) -> Option<Curve> {
        let pts = match self {
            Profile::Quiet => vec![(66.0, 0.0), (74.0, 15.0), (82.0, 45.0), (88.0, 80.0), (92.0, 100.0)],
            Profile::Balanced => vec![(58.0, 0.0), (65.0, 15.0), (72.0, 45.0), (78.0, 75.0), (83.0, 100.0)],
            Profile::Performance => vec![(50.0, 0.0), (56.0, 25.0), (63.0, 50.0), (70.0, 80.0), (76.0, 100.0)],
            Profile::Custom => return None,
        };
        Some(Curve::new(pts).expect("built-in curves are valid"))
    }
}

/// Controller dynamics. Defaults favour reacting fast to heat and backing off slowly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// Smoothing time constant while temperature rises (seconds). Small = react fast.
    pub rise_tau_s: f64,
    /// Smoothing time constant while temperature falls (seconds). Large = ignore brief dips.
    pub fall_tau_s: f64,
    /// Maximum duty increase per second.
    pub ramp_up_pct_per_s: f64,
    /// Maximum duty decrease per second once the hold period is over.
    pub ramp_down_pct_per_s: f64,
    /// How long the curve must ask for less before fans start slowing down.
    pub down_hold_s: f64,
    /// When slowing down, the curve is evaluated at `temp + hysteresis_c`.
    pub hysteresis_c: f64,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            rise_tau_s: 2.0,
            fall_tau_s: 12.0,
            ramp_up_pct_per_s: 25.0,
            ramp_down_pct_per_s: 2.0,
            down_hold_s: 30.0,
            hysteresis_c: 3.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Safety {
    /// At or above this smoothed temperature, fans go to max regardless of mode.
    pub critical_c: f64,
    /// Once critical, stay at max until the temperature is this far below `critical_c`.
    pub critical_hysteresis_c: f64,
    /// After this many consecutive failed sensor reads, control returns to macOS.
    pub max_sensor_failures: u32,
}

impl Default for Safety {
    fn default() -> Self {
        Safety { critical_c: 95.0, critical_hysteresis_c: 6.0, max_sensor_failures: 5 }
    }
}

/// Which SMC temperature keys drive the controller.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SensorConfig {
    /// Key prefixes considered part of the SoC die (CPU/GPU/fabric clusters).
    pub families: Vec<String>,
    /// Extra keys always included.
    pub include: Vec<String>,
    /// Keys never used (e.g. a sensor that reports garbage).
    pub exclude: Vec<String>,
    /// Readings outside this range are treated as invalid.
    pub min_valid_c: f64,
    pub max_valid_c: f64,
    /// Control temperature = mean of the hottest N sensors. 1 = pure max.
    /// A small N ignores a single noisy sensor while still tracking hot spots.
    pub aggregate_top_n: usize,
    /// Each SMC read costs ~135 µs on Apple Silicon (firmware round-trip), so
    /// between full scans only the hottest `hot_set_size` sensors are re-read.
    pub hot_set_size: usize,
    /// Read every control sensor once per this many ticks.
    pub full_scan_every_ticks: u32,
}

impl Default for SensorConfig {
    fn default() -> Self {
        SensorConfig {
            families: ["Tp", "Tm", "Ts", "Tg", "Te"].map(String::from).to_vec(),
            include: vec![],
            exclude: vec![],
            min_valid_c: 10.0,
            max_valid_c: 125.0,
            aggregate_top_n: 4,
            hot_set_size: 16,
            full_scan_every_ticks: 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mode: Mode,
    pub profile: Profile,
    /// Used when `profile = "custom"`.
    pub custom_curve: Curve,
    /// Sensor sampling period. Sampling is cheap (~1 ms); slow-down decisions
    /// are governed by `tuning.down_hold_s`, not by this.
    pub poll_interval_ms: u64,
    /// Fan target changes smaller than this are not written to the SMC.
    pub write_deadband_rpm: f64,
    pub tuning: Tuning,
    pub safety: Safety,
    pub sensors: SensorConfig,
    /// Users (besides root) allowed to change settings over the socket.
    pub allowed_uids: Vec<u32>,
    /// Control fans on Mac models that haven't been validated (see `SUPPORTED_MODELS` in fand).
    pub allow_unsupported_model: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: Mode::Smart,
            profile: Profile::Balanced,
            custom_curve: Profile::Balanced.builtin_curve().unwrap(),
            poll_interval_ms: 2000,
            write_deadband_rpm: 50.0,
            tuning: Tuning::default(),
            safety: Safety::default(),
            sensors: SensorConfig::default(),
            allowed_uids: vec![],
            allow_unsupported_model: false,
        }
    }
}

impl Config {
    pub fn curve(&self) -> Curve {
        self.profile.builtin_curve().unwrap_or_else(|| self.custom_curve.clone())
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(250..=60_000).contains(&self.poll_interval_ms) {
            return Err("poll_interval_ms must be between 250 and 60000".into());
        }
        let t = &self.tuning;
        for (name, v) in [
            ("rise_tau_s", t.rise_tau_s),
            ("fall_tau_s", t.fall_tau_s),
            ("down_hold_s", t.down_hold_s),
            ("hysteresis_c", t.hysteresis_c),
        ] {
            if !(v >= 0.0 && v.is_finite()) {
                return Err(format!("tuning.{name} must be >= 0"));
            }
        }
        if !(t.ramp_up_pct_per_s > 0.0 && t.ramp_down_pct_per_s > 0.0) {
            return Err("ramp rates must be > 0".into());
        }
        let s = &self.sensors;
        if s.aggregate_top_n == 0 || s.hot_set_size < s.aggregate_top_n || s.full_scan_every_ticks == 0 {
            return Err(
                "sensors: need aggregate_top_n >= 1, hot_set_size >= aggregate_top_n, full_scan_every_ticks >= 1"
                    .into(),
            );
        }
        if !(40.0..=110.0).contains(&self.safety.critical_c) {
            return Err("safety.critical_c must be between 40 and 110".into());
        }
        if let Mode::Fixed { rpm } = self.mode {
            if !(rpm.is_finite() && rpm >= 0.0) {
                return Err("fixed rpm must be >= 0".into());
            }
        }
        Ok(())
    }

    pub fn from_toml(s: &str) -> Result<Self, String> {
        let c: Config = toml::from_str(s).map_err(|e| e.to_string())?;
        c.validate()?;
        Ok(c)
    }

    pub fn to_toml(&self) -> String {
        let body = toml::to_string(self).expect("config always serializes");
        format!("{TOML_HEADER}{body}")
    }
}

const TOML_HEADER: &str = "\
# MacFanOptimizer daemon configuration.
# Changes made from the menu bar app or `fanctl` are written back here.
# After editing by hand: sudo launchctl kickstart -k system/io.github.estevecastells.macfanoptimizer
#
# mode.kind: system | smart | fixed (needs rpm) | max
# profile:   quiet | balanced | performance | custom (uses custom_curve)
# custom_curve: [[temperature_c, duty_pct], ...]; duty 0 = min rpm, 100 = max rpm
\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_validates_and_roundtrips_through_toml() {
        let c = Config::default();
        c.validate().unwrap();
        assert_eq!(Config::from_toml(&c.to_toml()).unwrap(), c);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let c = Config::from_toml("profile = \"quiet\"\n[mode]\nkind = \"fixed\"\nrpm = 3000\n").unwrap();
        assert_eq!(c.mode, Mode::Fixed { rpm: 3000.0 });
        assert_eq!(c.profile, Profile::Quiet);
        assert_eq!(c.tuning, Tuning::default());
    }

    #[test]
    fn custom_profile_uses_custom_curve() {
        let c = Config {
            profile: Profile::Custom,
            custom_curve: Curve::new(vec![(40.0, 0.0), (50.0, 100.0)]).unwrap(),
            ..Config::default()
        };
        assert_eq!(c.curve().duty(45.0), 50.0);
    }

    #[test]
    fn rejects_invalid() {
        let c = Config { poll_interval_ms: 10, ..Config::default() };
        assert!(c.validate().is_err());
    }
}
