//! A small lumped thermal model of a MacBook Pro, used to test the whole
//! control loop (controller + engine + write policy) without real hardware.
//!
//! Calibrated against measurements on an M5 Pro MacBook Pro (Mac17,9): heavy
//! sustained load (~75 W) sits near 100 °C with both fans at 6000 rpm, idle
//! settles near 50 °C with fans off. Above 102 °C the simulated SoC throttles.

use crate::config::Config;
use crate::controller::{Decision, Reason};
use crate::engine::Engine;
use crate::hardware::{FanInfo, FanReading, Hardware};
use crate::sensors::SensorKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// Light background load.
    Idle,
    /// Sustained all-core load (compiling, rendering).
    Sustained,
    /// 10 s bursts of heavy load every 40 s.
    Bursty,
    /// Power ramps linearly from idle to 90 W over the run.
    Ramp,
    /// Heavy CPU+GPU load beyond what the cooling can fully handle.
    Extreme,
}

impl Scenario {
    pub const ALL: [Scenario; 5] =
        [Scenario::Idle, Scenario::Sustained, Scenario::Bursty, Scenario::Ramp, Scenario::Extreme];

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.name() == s)
    }

    pub fn name(self) -> &'static str {
        match self {
            Scenario::Idle => "idle",
            Scenario::Sustained => "sustained",
            Scenario::Bursty => "bursty",
            Scenario::Ramp => "ramp",
            Scenario::Extreme => "extreme",
        }
    }

    /// SoC power draw in watts at time `t` of a run lasting `total` seconds.
    pub fn power(self, t: f64, total: f64) -> f64 {
        match self {
            Scenario::Idle => 6.0,
            Scenario::Sustained => 75.0,
            Scenario::Bursty => {
                if t % 40.0 < 10.0 {
                    75.0
                } else {
                    8.0
                }
            }
            Scenario::Ramp => 6.0 + 84.0 * (t / total).clamp(0.0, 1.0),
            Scenario::Extreme => 110.0,
        }
    }
}

const AMBIENT_C: f64 = 25.0;
const HEAT_CAPACITY_J_PER_K: f64 = 40.0;
const PASSIVE_W_PER_K: f64 = 0.25;
const FAN_W_PER_K: f64 = 1.0;
const FAN_TAU_S: f64 = 1.5;
const THROTTLE_C: f64 = 102.0;

pub struct SimHardware {
    pub fans: Vec<FanInfo>,
    keys: Vec<SensorKey>,
    pub die_c: f64,
    pub power_w: f64,
    actual: Vec<f64>,
    target: Vec<f64>,
    forced: Vec<bool>,
    noise_state: u64,
    /// Sensor reads fail while `fail_reads` is true.
    pub fail_reads: bool,
    pub writes: u64,
    pub reads: u64,
}

impl Default for SimHardware {
    fn default() -> Self {
        Self::new()
    }
}

impl SimHardware {
    pub fn new() -> Self {
        let fan = |index| FanInfo { index, min_rpm: 2317.0, max_rpm: 7826.0 };
        SimHardware {
            fans: vec![fan(0), fan(1)],
            keys: ["Tp00", "Tm0g", "Tg08", "TB0T"].iter().map(|k| SensorKey::parse(k).unwrap()).collect(),
            die_c: 45.0,
            power_w: 6.0,
            actual: vec![0.0; 2],
            target: vec![0.0; 2],
            forced: vec![false; 2],
            noise_state: 0x9E3779B97F4A7C15,
            fail_reads: false,
            writes: 0,
            reads: 0,
        }
    }

    /// Emulates macOS's own fan policy on Apple Silicon: silent until the die
    /// is very hot, then ramping up late.
    fn system_target(&self, fan: &FanInfo) -> f64 {
        if self.die_c < 85.0 {
            0.0
        } else {
            fan.rpm_for_duty((self.die_c - 85.0) / 15.0 * 100.0)
        }
    }

    pub fn fan_rpm(&self, i: usize) -> f64 {
        self.actual[i]
    }

    /// Simulate macOS resetting fans to automatic (e.g. after sleep/wake).
    pub fn system_reset(&mut self) {
        self.forced.iter_mut().for_each(|f| *f = false);
    }

    /// Simulate another app forcing a fan to `rpm`.
    pub fn external_write(&mut self, index: usize, rpm: f64) {
        self.forced[index] = true;
        self.target[index] = rpm;
    }

    /// Use `n` synthetic die sensors instead of the default set (scan-cost tests).
    pub fn with_sensor_count(mut self, n: usize) -> Self {
        self.keys = (0..n).map(|i| SensorKey::parse(&format!("Tp{i:02}")).unwrap()).collect();
        self
    }

    /// Advance physics by `dt` seconds with SoC power `power_w`.
    pub fn advance(&mut self, dt: f64, power_w: f64) {
        let throttle = if self.die_c > THROTTLE_C { (1.0 - (self.die_c - THROTTLE_C) / 10.0).max(0.3) } else { 1.0 };
        self.power_w = power_w * throttle;
        for i in 0..self.fans.len() {
            let goal = if self.forced[i] { self.target[i] } else { self.system_target(&self.fans[i]) };
            self.actual[i] += (goal - self.actual[i]) * (1.0 - (-dt / FAN_TAU_S).exp());
        }
        let n = self.fans.len() as f64;
        let airflow: f64 = self.fans.iter().zip(&self.actual).map(|(f, a)| a / f.max_rpm).sum::<f64>() / n;
        let conductance = PASSIVE_W_PER_K + FAN_W_PER_K * airflow;
        // Integrate in small steps for stability.
        let steps = (dt / 0.1).ceil().max(1.0) as usize;
        let h = dt / steps as f64;
        for _ in 0..steps {
            self.die_c += h * (self.power_w - conductance * (self.die_c - AMBIENT_C)) / HEAT_CAPACITY_J_PER_K;
        }
    }

    fn noise(&mut self) -> f64 {
        // xorshift64: deterministic, so tests are reproducible.
        let mut x = self.noise_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.noise_state = x;
        (x % 1000) as f64 / 1000.0 - 0.5
    }
}

impl Hardware for SimHardware {
    fn fans(&self) -> &[FanInfo] {
        &self.fans
    }

    fn temperature_keys(&self) -> &[SensorKey] {
        &self.keys
    }

    fn read_temperature(&mut self, key: SensorKey) -> Option<f64> {
        self.reads += 1;
        if self.fail_reads {
            return None;
        }
        let n = self.noise();
        let k = key.as_str();
        if k.len() == 4 && k.starts_with("Tp") && k != "Tp00" {
            // Synthetic sensors: spread slightly below the die hot spot.
            let i: f64 = k[2..].parse().unwrap_or(0.0);
            return Some(self.die_c - i * 0.1 + n);
        }
        Some(match k {
            "Tp00" => self.die_c + n,
            "Tm0g" => self.die_c - 2.0 + n,
            "Tg08" => self.die_c - 6.0 + n,
            _ => 30.0,
        })
    }

    fn read_fan(&mut self, index: u8) -> Result<FanReading, String> {
        let i = index as usize;
        Ok(FanReading { actual_rpm: self.actual[i], target_rpm: self.target[i], forced: self.forced[i] })
    }

    fn set_target(&mut self, index: u8, rpm: f64) -> Result<(), String> {
        let i = index as usize;
        self.forced[i] = true;
        self.target[i] = rpm;
        self.writes += 1;
        Ok(())
    }

    fn release(&mut self, index: u8) -> Result<(), String> {
        self.forced[index as usize] = false;
        self.writes += 1;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub t: f64,
    pub power_w: f64,
    pub die_c: f64,
    pub control_c: Option<f64>,
    pub fan_rpm: f64,
    pub decision: Decision,
    pub reason: Reason,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub max_die_c: f64,
    pub final_die_c: f64,
    pub avg_fan_rpm: f64,
    pub max_fan_rpm: f64,
    pub seconds_above_90c: f64,
    pub smc_writes: u64,
    /// Times the commanded fan speed changed direction (up↔down). Low = no hunting.
    pub direction_changes: u32,
}

/// Run `scenario` for `duration_s` seconds of simulated time.
pub fn run(cfg: &Config, scenario: Scenario, duration_s: f64) -> (Vec<Sample>, Summary) {
    run_with(cfg, scenario, duration_s, |_, _| {})
}

/// Like [`run`], with a hook called before every tick (for fault injection).
pub fn run_with(
    cfg: &Config,
    scenario: Scenario,
    duration_s: f64,
    mut hook: impl FnMut(f64, &mut SimHardware),
) -> (Vec<Sample>, Summary) {
    let dt = cfg.poll_interval_ms as f64 / 1000.0;
    let mut engine = Engine::new(SimHardware::new(), cfg.clone()).expect("valid config");
    let mut samples = Vec::new();
    let mut sum = Summary::default();
    let mut last_cmd: Option<f64> = None;
    let mut last_dir = 0i8;
    let mut t = 0.0;
    while t <= duration_s {
        hook(t, engine.hardware_mut());
        let status = engine.tick(t).clone();
        let hw = engine.hardware();
        let cmd = status.fans[0].commanded_rpm.unwrap_or(0.0);
        if let Some(prev) = last_cmd {
            let dir = if cmd > prev + 1.0 {
                1
            } else if cmd < prev - 1.0 {
                -1
            } else {
                0
            };
            if dir != 0 {
                if last_dir != 0 && dir != last_dir {
                    sum.direction_changes += 1;
                }
                last_dir = dir;
            }
        }
        last_cmd = Some(cmd);
        samples.push(Sample {
            t,
            power_w: hw.power_w,
            die_c: hw.die_c,
            control_c: status.control_c,
            fan_rpm: hw.fan_rpm(0),
            decision: status.decision,
            reason: status.reason,
        });
        sum.max_die_c = sum.max_die_c.max(hw.die_c);
        sum.max_fan_rpm = sum.max_fan_rpm.max(hw.fan_rpm(0));
        sum.avg_fan_rpm += hw.fan_rpm(0);
        if hw.die_c > 90.0 {
            sum.seconds_above_90c += dt;
        }
        let p = scenario.power(t, duration_s);
        engine.hardware_mut().advance(dt, p);
        t += dt;
    }
    sum.avg_fan_rpm /= samples.len() as f64;
    sum.final_die_c = engine.hardware().die_c;
    sum.smc_writes = engine.status().smc_writes;
    (samples, sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Mode, Profile};

    fn smart(profile: Profile) -> Config {
        Config { mode: Mode::Smart, profile, ..Config::default() }
    }

    fn system() -> Config {
        Config { mode: Mode::System, ..Config::default() }
    }

    #[test]
    fn model_calibration_matches_measurements() {
        let mut hw = SimHardware::new();
        hw.set_target(0, 6000.0).unwrap();
        hw.set_target(1, 6000.0).unwrap();
        for _ in 0..600 {
            hw.advance(1.0, 75.0);
        }
        assert!((hw.die_c - 99.0).abs() < 3.0, "75 W @ 6000 rpm should settle ≈99 °C, got {}", hw.die_c);
        let mut idle = SimHardware::new();
        for _ in 0..600 {
            idle.advance(1.0, 6.0);
        }
        assert!((idle.die_c - 49.0).abs() < 3.0, "idle with fans off should settle ≈49 °C, got {}", idle.die_c);
    }

    #[test]
    fn idle_stays_released_and_quiet() {
        let (samples, sum) = run(&smart(Profile::Balanced), Scenario::Idle, 600.0);
        assert!(samples.iter().all(|s| s.decision == Decision::System));
        assert_eq!(sum.max_fan_rpm, 0.0);
    }

    #[test]
    fn extreme_load_reaches_max_fan_speed() {
        let (_, sum) = run(&smart(Profile::Balanced), Scenario::Extreme, 600.0);
        assert!(sum.max_fan_rpm > 7826.0 * 0.98, "fans must reach the top, got {}", sum.max_fan_rpm);
    }

    #[test]
    fn smart_beats_macos_default_under_sustained_load() {
        let (_, ours) = run(&smart(Profile::Balanced), Scenario::Sustained, 900.0);
        let (_, apple) = run(&system(), Scenario::Sustained, 900.0);
        assert!(ours.max_die_c + 5.0 < apple.max_die_c, "ours {} vs macOS {}", ours.max_die_c, apple.max_die_c);
        assert!(ours.max_fan_rpm > 7826.0 * 0.98, "sustained heavy load must reach max fans");
    }

    #[test]
    fn bursty_load_does_not_hunt() {
        let (_, sum) = run(&smart(Profile::Balanced), Scenario::Bursty, 1200.0);
        // 30 bursts in 20 minutes; fan speed must not chase each one.
        assert!(sum.direction_changes <= 6, "too many direction changes: {}", sum.direction_changes);
        // Hysteresis and deadband keep SMC traffic low: < 1 write per 5 s per fan on average.
        assert!(sum.smc_writes < 1200 / 5 * 2, "too many writes: {}", sum.smc_writes);
    }

    #[test]
    fn profiles_are_ordered_by_aggressiveness() {
        let (_, q) = run(&smart(Profile::Quiet), Scenario::Ramp, 900.0);
        let (_, b) = run(&smart(Profile::Balanced), Scenario::Ramp, 900.0);
        let (_, p) = run(&smart(Profile::Performance), Scenario::Ramp, 900.0);
        assert!(q.max_die_c > b.max_die_c && b.max_die_c > p.max_die_c);
        assert!(q.avg_fan_rpm < b.avg_fan_rpm && b.avg_fan_rpm < p.avg_fan_rpm);
    }

    #[test]
    fn recovers_after_system_reset_on_wake() {
        let cfg = smart(Profile::Balanced);
        let (samples, _) = run_with(&cfg, Scenario::Sustained, 300.0, |t, hw| {
            if t == 200.0 {
                hw.system_reset();
            }
        });
        let late = samples.iter().find(|s| s.t == 210.0).unwrap();
        assert!(late.fan_rpm > 2317.0, "fans must be re-forced after a reset");
    }

    #[test]
    fn sensor_outage_returns_control_to_macos() {
        let cfg = smart(Profile::Balanced);
        let (samples, _) = run_with(&cfg, Scenario::Sustained, 120.0, |t, hw| hw.fail_reads = t >= 60.0);
        let last = samples.last().unwrap();
        assert_eq!((last.decision, last.reason), (Decision::System, Reason::SensorFailure));
    }

    #[test]
    fn fixed_mode_does_not_flap_at_critical() {
        let cfg = Config { mode: Mode::Fixed { rpm: 6000.0 }, ..Config::default() };
        let (_, sum) = run(&cfg, Scenario::Sustained, 1200.0);
        assert!(sum.direction_changes <= 10, "critical override flapping: {} flips", sum.direction_changes);
    }

    #[test]
    fn fixed_mode_holds_speed_but_respects_critical() {
        let cfg = Config { mode: Mode::Fixed { rpm: 2500.0 }, ..Config::default() };
        let (samples, sum) = run(&cfg, Scenario::Extreme, 600.0);
        assert!(samples.iter().any(|s| s.reason == Reason::Critical));
        // The safety override must cool as well as an explicit Max mode would.
        let (_, max) = run(&Config { mode: Mode::Max, ..Config::default() }, Scenario::Extreme, 600.0);
        assert!(
            sum.final_die_c <= max.final_die_c + 1.0,
            "fixed+critical {} vs max {}",
            sum.final_die_c,
            max.final_die_c
        );
    }
}
