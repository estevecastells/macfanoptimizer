//! The control algorithm: temperature in, fan decision out.
//!
//! Design (fast up, slow down):
//! - Temperature is smoothed with an asymmetric EMA: rises are followed within
//!   a couple of seconds, falls are followed slowly so brief dips are ignored.
//! - Increases in duty are applied almost immediately (rate-limited only so
//!   the fans don't jump audibly from min to max in one step).
//! - Decreases only start after the curve has asked for less for
//!   `down_hold_s`, use the curve shifted by `hysteresis_c`, and are ramped.
//! - When the curve settles back to 0 %, control is handed back to macOS so
//!   the fans can spin down completely at idle.
//! - Critical temperature forces max; repeated sensor failure hands back to macOS.

use crate::config::{Config, Mode};
use crate::curve::Curve;
use serde::{Deserialize, Serialize};

/// Width of the proportional safety band below `critical_c` in fixed mode.
pub const SAFETY_BAND_C: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    /// Let macOS control the fans.
    System,
    /// Drive fans at this % of their [min, max] RPM range.
    Duty { pct: f64 },
    /// Drive fans at an absolute RPM (clamped to each fan's range), but never
    /// below `floor_pct` of their range (the proportional safety floor).
    Rpm { rpm: f64, floor_pct: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Following the selected mode.
    Normal,
    /// Smart mode, temperature below the curve: macOS is in charge.
    Idle,
    /// Temperature at or above the critical threshold.
    Critical,
    /// Fixed mode, temperature approaching critical: fans raised above the fixed speed.
    Protecting,
    /// Sensor reads failed repeatedly; macOS is in charge.
    SensorFailure,
    /// A sensor read failed; holding the previous decision.
    SensorGlitch,
    /// Fans didn't respond to control on this machine; macOS is in charge.
    ControlDisabled,
}

#[derive(Clone, Debug, Default)]
pub struct Controller {
    smoothed: Option<f64>,
    last_time: Option<f64>,
    duty: f64,
    active: bool,
    below_since: Option<f64>,
    failures: u32,
    critical: bool,
    last: Option<(Decision, Reason)>,
}

impl Controller {
    pub fn new() -> Self {
        Self::default()
    }

    /// Smoothed control temperature, if any reading has been seen.
    pub fn smoothed(&self) -> Option<f64> {
        self.smoothed
    }

    /// Current smart-mode duty (0–100).
    pub fn duty(&self) -> f64 {
        self.duty
    }

    /// Forget all history (e.g. after a mode change or wake from sleep).
    pub fn reset(&mut self) {
        *self = Controller::default();
    }

    /// Advance the controller. `now` is a monotonic time in seconds.
    pub fn update(&mut self, cfg: &Config, raw_temp: Option<f64>, now: f64) -> (Decision, Reason) {
        let out = self.step(cfg, raw_temp, now);
        self.last = Some(out);
        out
    }

    fn step(&mut self, cfg: &Config, raw_temp: Option<f64>, now: f64) -> (Decision, Reason) {
        let poll_s = cfg.poll_interval_ms as f64 / 1000.0;
        let dt = match self.last_time {
            Some(t) if now >= t => now - t,
            _ => 0.0,
        };
        self.last_time = Some(now);

        // A long gap means we were asleep or stalled: old smoothing state is meaningless.
        if dt > (poll_s * 5.0).max(10.0) {
            self.smoothed = None;
            self.below_since = None;
        }

        let Some(raw) = raw_temp else {
            self.failures += 1;
            if self.failures >= cfg.safety.max_sensor_failures {
                self.active = false;
                self.duty = 0.0;
                return (Decision::System, Reason::SensorFailure);
            }
            return match self.last {
                Some((d, _)) => (d, Reason::SensorGlitch),
                None => (Decision::System, Reason::SensorGlitch),
            };
        };
        self.failures = 0;

        let t = match self.smoothed {
            None => raw,
            Some(s) => {
                let tau = if raw > s { cfg.tuning.rise_tau_s } else { cfg.tuning.fall_tau_s };
                let alpha = if tau <= 0.0 { 1.0 } else { 1.0 - (-dt / tau).exp() };
                s + (raw - s) * alpha
            }
        };
        self.smoothed = Some(t);

        if matches!(cfg.mode, Mode::System) {
            self.active = false;
            self.duty = 0.0;
            self.below_since = None;
            self.critical = false;
            return (Decision::System, Reason::Normal);
        }

        // Safety: react to the raw value too, so a spike isn't hidden by smoothing.
        // Exit only well below the threshold, otherwise fixed mode flaps around it.
        let safety = &cfg.safety;
        self.critical = if self.critical {
            t >= safety.critical_c - safety.critical_hysteresis_c
        } else {
            t >= safety.critical_c || raw >= safety.critical_c + 3.0
        };
        if self.critical {
            self.active = true;
            self.duty = 100.0;
            self.below_since = None;
            return (Decision::Duty { pct: 100.0 }, Reason::Critical);
        }

        match cfg.mode {
            Mode::System => unreachable!(),
            Mode::Max => {
                self.active = true;
                self.duty = 100.0;
                (Decision::Duty { pct: 100.0 }, Reason::Normal)
            }
            Mode::Fixed { rpm } => {
                // Safety floor: a curve from 0 % at `critical - SAFETY_BAND_C` to 100 % at
                // critical, tracked with the same hold/ramp/hysteresis as smart mode, so a
                // fixed speed that can't hold the load settles instead of oscillating.
                let c = cfg.safety.critical_c;
                let curve = Curve::new(vec![(c - SAFETY_BAND_C, 0.0), (c, 100.0)]).expect("valid safety curve");
                self.track(cfg, &curve, t, dt, now);
                let floor_pct = if self.active { self.duty } else { 0.0 };
                let reason = if floor_pct > 0.0 { Reason::Protecting } else { Reason::Normal };
                (Decision::Rpm { rpm, floor_pct }, reason)
            }
            Mode::Smart => {
                self.track(cfg, &cfg.curve(), t, dt, now);
                if self.active {
                    (Decision::Duty { pct: self.duty }, Reason::Normal)
                } else {
                    (Decision::System, Reason::Idle)
                }
            }
        }
    }

    /// Move `self.duty` toward `curve(t)`: fast up, held and ramped down.
    fn track(&mut self, cfg: &Config, curve: &Curve, t: f64, dt: f64, now: f64) {
        let tune = &cfg.tuning;
        let up_target = curve.duty(t);
        let down_target = curve.duty(t + tune.hysteresis_c);

        if up_target > self.duty {
            self.duty = if dt == 0.0 { up_target } else { up_target.min(self.duty + tune.ramp_up_pct_per_s * dt) };
            self.below_since = None;
            self.active = true;
        } else if down_target < self.duty {
            let since = *self.below_since.get_or_insert(now);
            if now - since >= tune.down_hold_s {
                self.duty = down_target.max(self.duty - tune.ramp_down_pct_per_s * dt);
            }
        } else {
            self.below_since = None;
        }

        if self.active && self.duty <= 0.0 {
            self.active = false;
            self.below_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Profile;

    fn cfg() -> Config {
        Config {
            profile: Profile::Custom,
            custom_curve: Curve::new(vec![(60.0, 0.0), (70.0, 50.0), (80.0, 100.0)]).unwrap(),
            poll_interval_ms: 1000,
            ..Config::default()
        }
    }

    fn duty(d: Decision) -> f64 {
        match d {
            Decision::Duty { pct } => pct,
            Decision::System => 0.0,
            Decision::Rpm { .. } => panic!("unexpected rpm"),
        }
    }

    #[test]
    fn idle_hands_control_to_system() {
        let mut c = Controller::new();
        assert_eq!(c.update(&cfg(), Some(45.0), 0.0), (Decision::System, Reason::Idle));
    }

    #[test]
    fn first_sample_jumps_straight_to_curve() {
        let mut c = Controller::new();
        let (d, _) = c.update(&cfg(), Some(70.0), 0.0);
        assert_eq!(duty(d), 50.0);
    }

    #[test]
    fn ramps_up_quickly_and_reaches_max() {
        let cfg = cfg();
        let mut c = Controller::new();
        c.update(&cfg, Some(50.0), 0.0);
        let mut d = 0.0;
        for i in 1..=10 {
            d = duty(c.update(&cfg, Some(90.0), i as f64).0);
        }
        assert_eq!(d, 100.0, "sustained heat must reach max within 10 s");
    }

    #[test]
    fn holds_before_slowing_down_then_ramps_gradually() {
        let cfg = cfg();
        let mut c = Controller::new();
        c.update(&cfg, Some(80.0), 0.0);
        assert_eq!(c.duty(), 100.0);
        // Temperature drops to 65 °C (curve wants 25 %).
        let mut first_drop = None;
        let mut prev = c.duty();
        for t in 1..=120 {
            let d = duty(c.update(&cfg, Some(65.0), t as f64).0);
            assert!(prev - d <= cfg.tuning.ramp_down_pct_per_s + 1e-9, "dropped too fast at t={t}");
            if d < 100.0 && first_drop.is_none() {
                first_drop = Some(t as f64);
            }
            prev = d;
        }
        let first_drop = first_drop.expect("fans must eventually slow down");
        assert!(first_drop > cfg.tuning.down_hold_s, "slowed down after {first_drop}s, before the hold expired");
        assert!(prev < 50.0, "should have ramped most of the way down, at {prev}");
    }

    #[test]
    fn short_dip_does_not_slow_fans() {
        let cfg = cfg();
        let mut c = Controller::new();
        c.update(&cfg, Some(75.0), 0.0);
        let start = c.duty();
        for t in 1..=10 {
            c.update(&cfg, Some(55.0), t as f64);
        }
        for t in 11..=20 {
            c.update(&cfg, Some(75.0), t as f64);
        }
        assert!(c.duty() >= start - 1e-9);
    }

    #[test]
    fn eventually_releases_when_cool() {
        let cfg = cfg();
        let mut c = Controller::new();
        c.update(&cfg, Some(75.0), 0.0);
        let mut last = (Decision::Duty { pct: 0.0 }, Reason::Normal);
        for t in 1..=300 {
            last = c.update(&cfg, Some(40.0), t as f64);
        }
        assert_eq!(last, (Decision::System, Reason::Idle));
    }

    #[test]
    fn critical_forces_max_in_fixed_mode() {
        let mut cfg = cfg();
        cfg.mode = Mode::Fixed { rpm: 2000.0 };
        let mut c = Controller::new();
        assert_eq!(c.update(&cfg, Some(50.0), 0.0).0, Decision::Rpm { rpm: 2000.0, floor_pct: 0.0 });
        assert_eq!(c.update(&cfg, Some(110.0), 1.0), (Decision::Duty { pct: 100.0 }, Reason::Critical));
    }

    #[test]
    fn critical_has_hysteresis() {
        let mut cfg = cfg();
        cfg.mode = Mode::Fixed { rpm: 2000.0 };
        cfg.tuning.fall_tau_s = 0.0;
        let mut c = Controller::new();
        assert_eq!(c.update(&cfg, Some(96.0), 0.0).1, Reason::Critical);
        assert_eq!(c.update(&cfg, Some(93.0), 1.0).1, Reason::Critical, "must not leave critical just below threshold");
        // Below the critical band: the floor is held at max, then ramps away.
        let (d, r) = c.update(&cfg, Some(87.0), 2.0);
        assert_eq!((d, r), (Decision::Rpm { rpm: 2000.0, floor_pct: 100.0 }, Reason::Protecting));
        let mut last = d;
        for t in 3..=200 {
            last = c.update(&cfg, Some(70.0), t as f64).0;
        }
        assert_eq!(last, Decision::Rpm { rpm: 2000.0, floor_pct: 0.0 });
    }

    #[test]
    fn system_mode_is_never_overridden() {
        let mut cfg = cfg();
        cfg.mode = Mode::System;
        let mut c = Controller::new();
        assert_eq!(c.update(&cfg, Some(110.0), 0.0).0, Decision::System);
    }

    #[test]
    fn sensor_glitch_holds_then_failure_releases() {
        let cfg = cfg();
        let mut c = Controller::new();
        let (d, _) = c.update(&cfg, Some(75.0), 0.0);
        for t in 1..cfg.safety.max_sensor_failures {
            assert_eq!(c.update(&cfg, None, t as f64), (d, Reason::SensorGlitch));
        }
        let t = cfg.safety.max_sensor_failures as f64;
        assert_eq!(c.update(&cfg, None, t), (Decision::System, Reason::SensorFailure));
    }

    #[test]
    fn long_gap_resets_smoothing() {
        let cfg = cfg();
        let mut c = Controller::new();
        c.update(&cfg, Some(40.0), 0.0);
        c.update(&cfg, Some(80.0), 3600.0);
        assert_eq!(c.smoothed(), Some(80.0));
    }
}
