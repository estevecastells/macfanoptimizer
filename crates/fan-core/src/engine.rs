//! The control loop body: read sensors → decide → apply to fans, with
//! minimal SMC writes and recovery from external interference.

use crate::config::{Config, Mode};
use crate::controller::{Controller, Decision, Reason};
use crate::hardware::Hardware;
use crate::protocol::{FanStatus, SensorValue, Status};
use crate::sensors::{self, SensorKey};
use std::time::Instant;

/// What we last told a fan to do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Applied {
    /// Unknown state (startup, or after a failed write): next tick re-applies.
    Unknown,
    Released,
    Target(f64),
}

pub struct Engine<H: Hardware> {
    hw: H,
    cfg: Config,
    ctrl: Controller,
    control_keys: Vec<SensorKey>,
    /// Hottest sensors from the last full scan; read every tick.
    hot_keys: Vec<SensorKey>,
    applied: Vec<Applied>,
    readings: Vec<(SensorKey, f64)>,
    status: Status,
    started: Option<f64>,
}

impl<H: Hardware> Engine<H> {
    pub fn new(hw: H, cfg: Config) -> Result<Self, String> {
        cfg.validate()?;
        let control_keys = sensors::select_control_keys(hw.temperature_keys(), &cfg.sensors);
        let fans = hw.fans().len();
        let status = Status {
            mode: cfg.mode.clone(),
            profile: cfg.profile,
            decision: Decision::System,
            reason: Reason::Normal,
            hotspot_key: None,
            hotspot_c: None,
            control_c: None,
            duty_pct: 0.0,
            fans: hw.fans().iter().map(|&info| FanStatus { info, reading: None, commanded_rpm: None }).collect(),
            groups: vec![],
            control_sensor_count: control_keys.len(),
            tick_us: 0,
            ticks: 0,
            smc_writes: 0,
            last_error: None,
            external_override: false,
            conflicts: vec![],
            uptime_s: 0.0,
            model: String::new(),
            writes_enabled: true,
        };
        Ok(Engine {
            hw,
            cfg,
            ctrl: Controller::new(),
            control_keys,
            hot_keys: Vec::new(),
            applied: vec![Applied::Unknown; fans],
            readings: Vec::new(),
            status,
            started: None,
        })
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn set_machine(&mut self, model: String, writes_enabled: bool) {
        self.status.model = model;
        self.status.writes_enabled = writes_enabled;
    }

    pub fn set_conflicts(&mut self, conflicts: Vec<String>) {
        self.status.conflicts = conflicts;
    }

    pub fn hardware(&self) -> &H {
        &self.hw
    }

    pub fn hardware_mut(&mut self) -> &mut H {
        &mut self.hw
    }

    pub fn control_keys(&self) -> &[SensorKey] {
        &self.control_keys
    }

    /// Replace the configuration. Takes effect on the next tick.
    pub fn set_config(&mut self, cfg: Config) -> Result<(), String> {
        cfg.validate()?;
        if cfg.sensors != self.cfg.sensors {
            self.control_keys = sensors::select_control_keys(self.hw.temperature_keys(), &cfg.sensors);
            self.status.control_sensor_count = self.control_keys.len();
            self.hot_keys.clear();
        }
        self.status.mode = cfg.mode.clone();
        self.status.profile = cfg.profile;
        self.cfg = cfg;
        Ok(())
    }

    /// One control iteration. `now` is monotonic seconds.
    pub fn tick(&mut self, now: f64) -> &Status {
        let t0 = Instant::now();
        let started = *self.started.get_or_insert(now);

        // 1. Sensors: full scan periodically, otherwise only the hot set.
        let sc = &self.cfg.sensors;
        let full = self.hot_keys.is_empty() || self.status.ticks.is_multiple_of(sc.full_scan_every_ticks as u64);
        let keys = std::mem::take(if full { &mut self.control_keys } else { &mut self.hot_keys });
        self.readings.clear();
        for &key in &keys {
            if let Some(c) = self.hw.read_temperature(key) {
                if sensors::is_valid(&self.cfg.sensors, c) {
                    self.readings.push((key, c));
                }
            }
        }
        *(if full { &mut self.control_keys } else { &mut self.hot_keys }) = keys;
        self.readings.sort_by(|a, b| b.1.total_cmp(&a.1));
        if full {
            self.hot_keys = self.readings.iter().take(self.cfg.sensors.hot_set_size).map(|r| r.0).collect();
            self.status.groups = sensors::summarize(&self.readings);
        }
        let hottest = self.readings.first().copied();
        let top = &self.readings[..self.readings.len().min(self.cfg.sensors.aggregate_top_n)];
        let control = (!top.is_empty()).then(|| top.iter().map(|r| r.1).sum::<f64>() / top.len() as f64);

        // 2. Decide.
        let (decision, reason) = self.ctrl.update(&self.cfg, control, now);

        // 3. Apply.
        let mut last_error = None;
        let mut external = false;
        for i in 0..self.applied.len() {
            let info = self.hw.fans()[i];
            let reading = self.hw.read_fan(info.index);
            let reading = match reading {
                Ok(r) => Some(r),
                Err(e) => {
                    last_error = Some(e);
                    None
                }
            };

            let wanted = match decision {
                Decision::System => None,
                Decision::Duty { pct } => Some(info.rpm_for_duty(pct)),
                Decision::Rpm { rpm, floor_pct } => {
                    Some(info.clamp(rpm).max(info.rpm_for_duty(floor_pct) * (floor_pct > 0.0) as u8 as f64))
                }
            };

            let result = match (wanted, self.applied[i]) {
                (None, Applied::Released) => Ok(()),
                (None, _) => {
                    let r = self.hw.release(info.index);
                    if r.is_ok() {
                        self.applied[i] = Applied::Released;
                        self.status.smc_writes += 1;
                    }
                    r
                }
                (Some(rpm), applied) => {
                    let need = match applied {
                        Applied::Target(last) => {
                            let moved = (last - rpm).abs() >= self.cfg.write_deadband_rpm
                                // Always land exactly on the extremes.
                                || (rpm != last && (rpm >= info.max_rpm || rpm <= info.min_rpm));
                            // macOS resets fans to auto on wake; another app may have rewritten the target.
                            let lost = reading.is_some_and(|r| !r.forced);
                            let overridden = reading
                                .is_some_and(|r| r.forced && (r.target_rpm - last).abs() > self.cfg.write_deadband_rpm);
                            external |= overridden;
                            moved || lost || overridden
                        }
                        _ => true,
                    };
                    if need {
                        match self.hw.set_target(info.index, rpm) {
                            Ok(()) => {
                                self.applied[i] = Applied::Target(rpm);
                                self.status.smc_writes += 1;
                                Ok(())
                            }
                            Err(e) => {
                                self.applied[i] = Applied::Unknown;
                                Err(e)
                            }
                        }
                    } else {
                        Ok(())
                    }
                }
            };
            if let Err(e) = result {
                last_error = Some(e);
            }

            let fs = &mut self.status.fans[i];
            fs.reading = reading;
            fs.commanded_rpm = match self.applied[i] {
                Applied::Target(r) => Some(r),
                _ => None,
            };
        }

        // 4. Report.
        let s = &mut self.status;
        s.decision = decision;
        s.reason = reason;
        s.hotspot_key = hottest.map(|h| h.0);
        s.hotspot_c = hottest.map(|h| h.1);
        s.control_c = self.ctrl.smoothed();
        s.duty_pct = match decision {
            Decision::Duty { pct } => pct,
            _ => 0.0,
        };
        s.ticks += 1;
        s.last_error = last_error;
        s.external_override = external;
        s.uptime_s = now - started;
        s.tick_us = t0.elapsed().as_micros() as u64;
        &self.status
    }

    /// Hand every fan back to macOS. Called on shutdown and on mode=system.
    pub fn release_all(&mut self) -> Result<(), String> {
        let mut err = None;
        for i in 0..self.applied.len() {
            let idx = self.hw.fans()[i].index;
            match self.hw.release(idx) {
                Ok(()) => self.applied[i] = Applied::Released,
                Err(e) => err = Some(e),
            }
        }
        err.map_or(Ok(()), Err)
    }

    /// Every temperature sensor (not only control ones), for diagnostics.
    pub fn all_sensors(&mut self) -> Vec<SensorValue> {
        let keys: Vec<SensorKey> = self.hw.temperature_keys().to_vec();
        keys.into_iter()
            .map(|key| SensorValue {
                key,
                group: sensors::group_of(key).to_string(),
                celsius: self.hw.read_temperature(key),
                control: self.control_keys.contains(&key),
            })
            .collect()
    }

    pub fn is_controlling(&self) -> bool {
        !matches!(self.cfg.mode, Mode::System) && self.applied.iter().any(|a| matches!(a, Applied::Target(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::SimHardware;

    fn hot(mut hw: SimHardware) -> SimHardware {
        hw.die_c = 80.0;
        hw
    }

    #[test]
    fn adaptive_scan_reads_hot_set_between_full_scans() {
        let cfg = Config::default();
        let mut e = Engine::new(SimHardware::new().with_sensor_count(100), cfg.clone()).unwrap();
        for t in 0..10 {
            e.tick(t as f64 * 2.0);
        }
        let every = cfg.sensors.full_scan_every_ticks as u64;
        let full_scans = 10u64.div_ceil(every);
        let expected = full_scans * 100 + (10 - full_scans) * cfg.sensors.hot_set_size as u64;
        assert_eq!(e.hardware().reads, expected);
    }

    #[test]
    fn small_changes_are_not_written() {
        let mut cfg = Config::default();
        cfg.tuning.ramp_up_pct_per_s = 1000.0;
        let mut e = Engine::new(hot(SimHardware::new()), cfg).unwrap();
        e.tick(0.0);
        let writes = e.status().smc_writes;
        assert_eq!(writes, 2, "one write per fan to start");
        // Same temperature (plus sensor noise) → no new writes.
        for t in 1..20 {
            e.tick(t as f64);
        }
        assert_eq!(e.status().smc_writes, writes);
    }

    #[test]
    fn detects_and_corrects_external_override() {
        let mut e = Engine::new(hot(SimHardware::new()), Config::default()).unwrap();
        e.tick(0.0);
        let mine = e.status().fans[0].commanded_rpm.unwrap();
        e.hardware_mut().external_write(0, 3000.0);
        let s = e.tick(1.0);
        assert!(s.external_override);
        let r = e.hardware_mut().read_fan(0).unwrap();
        assert!((r.target_rpm - mine).abs() < 100.0, "must restore our target, got {}", r.target_rpm);
    }

    #[test]
    fn release_all_hands_back_every_fan() {
        let mut e = Engine::new(hot(SimHardware::new()), Config::default()).unwrap();
        e.tick(0.0);
        assert!(e.is_controlling());
        e.release_all().unwrap();
        for i in 0..2 {
            assert!(!e.hardware_mut().read_fan(i).unwrap().forced);
        }
    }

    #[test]
    fn switching_to_system_mode_releases_once() {
        let mut e = Engine::new(hot(SimHardware::new()), Config::default()).unwrap();
        e.tick(0.0);
        e.set_config(Config { mode: Mode::System, ..Config::default() }).unwrap();
        e.tick(1.0);
        let writes = e.status().smc_writes;
        assert!(!e.hardware_mut().read_fan(0).unwrap().forced);
        e.tick(2.0);
        assert_eq!(e.status().smc_writes, writes, "no repeated releases");
    }
}
