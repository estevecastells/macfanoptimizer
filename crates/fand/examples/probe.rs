//! Measure per-tick SMC cost and sample control-signal candidates.
//! `cargo run --release -p fand --example probe -- <seconds>`
use fan_core::{sensors, Config, Hardware};
use std::time::Instant;
fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let mut hw = fand::SmcHardware::open(false).unwrap();
    let keys = sensors::select_control_keys(hw.temperature_keys(), &Config::default().sensors);
    for k in &keys {
        hw.read_temperature(*k);
    } // warm key-info cache
    let t = Instant::now();
    for _ in 0..20 {
        for k in &keys {
            hw.read_temperature(*k);
        }
    }
    println!(
        "steady tick: {:.2} ms for {} sensors ({:.1} us/read)",
        t.elapsed().as_secs_f64() * 50.0,
        keys.len(),
        t.elapsed().as_secs_f64() * 1e6 / (20 * keys.len()) as f64
    );
    for _ in 0..secs {
        let mut v: Vec<f64> =
            keys.iter().filter_map(|k| hw.read_temperature(*k)).filter(|c| *c > 10.0 && *c < 125.0).collect();
        v.sort_by(|a, b| b.total_cmp(a));
        let top = |n: usize| v.iter().take(n).sum::<f64>() / n.min(v.len()) as f64;
        let rpm = hw.read_fan(0).unwrap().actual_rpm;
        println!(
            "max {:5.1}  top4 {:5.1}  top8 {:5.1}  top16 {:5.1}  mean {:5.1}  fan {:4.0}",
            v[0],
            top(4),
            top(8),
            top(16),
            v.iter().sum::<f64>() / v.len() as f64,
            rpm
        );
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
