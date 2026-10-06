//! CPU time vs wall time of SMC reads.
use fan_core::{sensors, Config, Hardware};
fn cpu() -> f64 {
    let mut r: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut r) };
    (r.ru_utime.tv_sec + r.ru_stime.tv_sec) as f64 + (r.ru_utime.tv_usec + r.ru_stime.tv_usec) as f64 * 1e-6
}
fn main() {
    let mut hw = fand::SmcHardware::open(false).unwrap();
    let keys = sensors::select_control_keys(hw.temperature_keys(), &Config::default().sensors);
    for k in &keys {
        hw.read_temperature(*k);
    }
    let (c0, t0) = (cpu(), std::time::Instant::now());
    for _ in 0..50 {
        for k in &keys {
            hw.read_temperature(*k);
        }
    }
    let n = (50 * keys.len()) as f64;
    println!("per read: wall {:.1} us, cpu {:.1} us", t0.elapsed().as_secs_f64() * 1e6 / n, (cpu() - c0) * 1e6 / n);
}
