//! The control loop body: read sensors → decide → apply to fans, with
//! minimal SMC writes and recovery from external interference.

use crate::config::{Config, Mode};
use crate::controller::{Controller, Decision, Reason};
use crate::hardware::Hardware;
use crate::protocol::{FanStatus, MachineInfo, SensorValue, Status};
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

/// How long a fan may take to obey a new target before we conclude that
/// control doesn't work on this machine. Fans spin up in a few seconds; the
/// margin covers M1–M4 `Ftst` unlocks, which can need several retries.
pub const VERIFY_TIMEOUT_S: f64 = 30.0;

/// Runtime proof that writes actually move the fans. This is what makes it
/// safe to enable control on models nobody has validated.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Verify {
    Untested,
    Pending { since: f64, target: f64 },
    Passed,
}

fn rpm_tolerance(target: f64) -> f64 {
    (target * 0.10).max(250.0)
}

pub struct Engine<H: Hardware> {
    hw: H,
    cfg: Config,
    ctrl: Controller,
    control_keys: Vec<SensorKey>,
    /// Hottest sensors from the last full scan; read every tick.
    hot_keys: Vec<SensorKey>,
    applied: Vec<Applied>,
    verify: Vec<Verify>,
    /// Set when verification failed: fans stay with macOS until restart.
    disabled: Option<String>,
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
            fans: hw
                .fans()
                .iter()
                .map(|&info| FanStatus { info, reading: None, commanded_rpm: None, verified: None })
                .collect(),
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
            chip: String::new(),
            support: Default::default(),
            support_note: String::new(),
            control_disabled: None,
        };
        Ok(Engine {
            hw,
            cfg,
            ctrl: Controller::new(),
            control_keys,
            hot_keys: Vec::new(),
            applied: vec![Applied::Unknown; fans],
            verify: vec![Verify::Untested; fans],
            disabled: None,
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

    /// Describe the machine. Fan-response verification only runs when writes
    /// really reach the hardware (not in dry-run or monitor-only mode).
    pub fn set_machine(&mut self, machine: MachineInfo, writes_enabled: bool) {
        self.status.model = machine.model;
        self.status.chip = machine.chip;
        self.status.support = machine.support;
        self.status.support_note = machine.note;
        self.status.writes_enabled = writes_enabled;
    }

    /// Why control was disabled at runtime, if it was.
    pub fn control_disabled(&self) -> Option<&str> {
        self.disabled.as_deref()
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

    pub fn into_hardware(self) -> H {
        self.hw
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
        let (mut decision, mut reason) = self.ctrl.update(&self.cfg, control, now);
        if self.disabled.is_some() {
            (decision, reason) = (Decision::System, Reason::ControlDisabled);
        }

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

            if let (Verify::Pending { since, target }, Some(r)) = (self.verify[i], reading) {
                if self.status.writes_enabled {
                    let target_held = (r.target_rpm - target).abs() <= self.cfg.write_deadband_rpm;
                    if r.forced && target_held && (r.actual_rpm - target).abs() <= rpm_tolerance(target) {
                        self.verify[i] = Verify::Passed;
                    } else if now - since > VERIFY_TIMEOUT_S {
                        self.disabled = Some(format!(
                            "fan {} did not respond to control: asked for {:.0} rpm, fan at {:.0} rpm \
                             (SMC target {:.0}, forced {}). Fans returned to macOS.",
                            info.index, target, r.actual_rpm, r.target_rpm, r.forced
                        ));
                    }
                }
            }

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
                                self.verify[i] = match self.verify[i] {
                                    Verify::Untested => Verify::Pending { since: now, target: rpm },
                                    Verify::Pending { since, .. } => Verify::Pending { since, target: rpm },
                                    Verify::Passed => Verify::Passed,
                                };
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
            fs.verified = match self.verify[i] {
                Verify::Passed => Some(true),
                _ if self.disabled.is_some() => Some(false),
                _ => None,
            };
        }

        // A failed verification takes effect immediately, not on the next tick.
        if self.disabled.is_some() && self.applied.iter().any(|a| !matches!(a, Applied::Released)) {
            if let Err(e) = self.release_all() {
                last_error = Some(e);
            }
            decision = Decision::System;
            reason = Reason::ControlDisabled;
            self.status.fans.iter_mut().for_each(|f| f.commanded_rpm = None);
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
        s.control_disabled = self.disabled.clone();
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

    fn run(e: &mut Engine<SimHardware>, from: u32, to: u32) {
        for t in from..to {
            e.hardware_mut().advance(1.0, 75.0);
            e.tick(t as f64);
        }
    }

    #[test]
    fn verification_passes_when_fans_obey() {
        let mut e = Engine::new(hot(SimHardware::new()), Config::default()).unwrap();
        run(&mut e, 0, 15);
        assert!(e.status().fans.iter().all(|f| f.verified == Some(true)));
        assert_eq!(e.control_disabled(), None);
    }

    #[test]
    fn fans_that_ignore_writes_disable_control_and_return_to_macos() {
        let mut e = Engine::new(hot(SimHardware::new()), Config::default()).unwrap();
        e.hardware_mut().ignore_writes = true;
        run(&mut e, 0, (VERIFY_TIMEOUT_S as u32) + 5);
        let s = e.status();
        assert_eq!(s.reason, Reason::ControlDisabled);
        assert!(s.control_disabled.as_deref().unwrap().contains("did not respond"));
        for i in 0..2 {
            assert!(!e.hardware_mut().read_fan(i).unwrap().forced, "fan {i} must be back with macOS");
        }
        // And it stays off: no further writes.
        let writes = e.status().smc_writes;
        run(&mut e, 40, 60);
        assert_eq!(e.status().smc_writes, writes);
    }

    #[test]
    fn os_that_keeps_reclaiming_fans_disables_control() {
        // Like thermalmonitord undoing forced mode on M1–M4 without a working Ftst unlock.
        let mut e = Engine::new(hot(SimHardware::new()), Config::default()).unwrap();
        e.hardware_mut().reclaim_forced = true;
        run(&mut e, 0, (VERIFY_TIMEOUT_S as u32) + 5);
        assert_eq!(e.status().reason, Reason::ControlDisabled);
    }

    #[test]
    fn slow_unlock_within_timeout_still_verifies() {
        // Forced mode only sticks after ~10 s of retries (an Ftst unlock taking effect).
        let mut e = Engine::new(hot(SimHardware::new()), Config::default()).unwrap();
        e.hardware_mut().reclaim_forced = true;
        run(&mut e, 0, 10);
        e.hardware_mut().reclaim_forced = false;
        run(&mut e, 10, 25);
        assert_eq!(e.control_disabled(), None);
        assert!(e.status().fans.iter().all(|f| f.verified == Some(true)));
    }

    #[test]
    fn dry_run_never_disables_control() {
        let mut e = Engine::new(hot(SimHardware::new()), Config::default()).unwrap();
        e.set_machine(MachineInfo::default(), false);
        e.hardware_mut().ignore_writes = true;
        run(&mut e, 0, 60);
        assert_eq!(e.control_disabled(), None);
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
