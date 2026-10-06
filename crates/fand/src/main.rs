//! `fand` — the MacFanOptimizer daemon.
//!
//! Runs as root under launchd. Every `poll_interval_ms` it reads the SoC
//! temperature sensors, runs the controller, and writes fan targets only when
//! they meaningfully change. On exit (SIGTERM, SIGINT, panic) it hands the
//! fans back to macOS.

use fan_core::controller::{Decision, Reason};
use fan_core::protocol::DEFAULT_SOCKET_PATH;
use fan_core::{Config, Engine, Hardware};
use fand::{conflicts, log, server, SmcHardware, DEFAULT_CONFIG_PATH};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "\
usage: fand [--config PATH] [--socket PATH] [--dry-run] [--once]

  --config PATH   config file (default: /Library/Application Support/MacFanOptimizer/config.toml)
  --socket PATH   control socket (default: /var/run/macfanoptimizer.sock)
  --dry-run       never write to the SMC; log intended writes (works without root)
  --once          run a single control tick, print status as JSON, and exit
  --print-default-config
";

struct Args {
    config: PathBuf,
    socket: PathBuf,
    dry_run: bool,
    once: bool,
}

fn parse_args() -> Args {
    let mut args =
        Args { config: DEFAULT_CONFIG_PATH.into(), socket: DEFAULT_SOCKET_PATH.into(), dry_run: false, once: false };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" => args.config = it.next().unwrap_or_else(|| die("--config needs a path")).into(),
            "--socket" => args.socket = it.next().unwrap_or_else(|| die("--socket needs a path")).into(),
            "--dry-run" => args.dry_run = true,
            "--once" => args.once = true,
            "--print-default-config" => {
                print!("{}", Config::default().to_toml());
                std::process::exit(0);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => die(&format!("unknown argument {other}\n{USAGE}")),
        }
    }
    args
}

fn die(msg: &str) -> ! {
    eprintln!("fand: {msg}");
    std::process::exit(2);
}

/// Monotonic seconds *including* time asleep (Rust's `Instant` stops during
/// sleep on macOS, which would hide sleep/wake gaps from the controller).
fn now_s() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9
}

fn load_config(path: &PathBuf) -> Config {
    match std::fs::read_to_string(path) {
        Ok(s) => Config::from_toml(&s).unwrap_or_else(|e| {
            log!("invalid config {}: {e}; using defaults", path.display());
            Config::default()
        }),
        Err(_) => {
            log!("no config at {}; using defaults", path.display());
            Config::default()
        }
    }
}

/// Hands fans back to macOS when dropped, including during a panic unwind.
struct ReleaseGuard<H: Hardware>(Arc<server::Shared<H>>);

impl<H: Hardware> Drop for ReleaseGuard<H> {
    fn drop(&mut self) {
        match self.0.engine().release_all() {
            Ok(()) => log!("fans returned to macOS control"),
            Err(e) => log!("error returning fans to macOS: {e}"),
        }
    }
}

fn main() {
    let args = parse_args();
    let is_root = unsafe { libc::geteuid() } == 0;
    if !is_root && !args.dry_run {
        die("must run as root to control fans (use --dry-run to test without root)");
    }

    let cfg = load_config(&args.config);
    let model = fand::hardware_model();
    let supported = fand::is_supported_model(&model);
    if !supported && !cfg.allow_unsupported_model {
        log!(
            "{model} is not a validated model (supported: {:?}); running READ-ONLY. \
             Set allow_unsupported_model = true in the config to control fans anyway.",
            fand::SUPPORTED_MODELS
        );
    }
    let writes_enabled = !args.dry_run && (supported || cfg.allow_unsupported_model);
    let hw = SmcHardware::open(writes_enabled).unwrap_or_else(|e| die(&format!("SMC: {e}")));
    log!(
        "fand {} starting on {model}: {} fan(s) {:?}, {} temperature sensors, unlock key: {}, writes: {}",
        env!("CARGO_PKG_VERSION"),
        hw.fans().len(),
        hw.fans().iter().map(|f| (f.min_rpm, f.max_rpm)).collect::<Vec<_>>(),
        hw.temperature_keys().len(),
        hw.has_unlock_key(),
        writes_enabled
    );

    let mut engine = Engine::new(hw, cfg).unwrap_or_else(|e| die(&format!("config: {e}")));
    engine.set_machine(model, writes_enabled);
    log!("control sensors ({}): {:?}", engine.control_keys().len(), engine.control_keys());

    if args.once {
        let status = engine.tick(now_s()).clone();
        println!("{}", serde_json::to_string_pretty(&status).unwrap());
        if !args.dry_run {
            let _ = engine.release_all();
        }
        return;
    }

    let config_path = (is_root || args.config.parent().is_some_and(|p| p.exists())).then(|| args.config.clone());
    let shared = server::Shared::new(engine, config_path);
    let _guard = ReleaseGuard(shared.clone());

    let term = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT, signal_hook::consts::SIGHUP] {
        signal_hook::flag::register(sig, term.clone()).expect("register signal handler");
    }

    if let Err(e) = server::spawn(shared.clone(), &args.socket) {
        die(&format!("cannot listen on {}: {e}", args.socket.display()));
    }
    log!("listening on {}", args.socket.display());

    let mut last_logged: Option<(Decision, Reason)> = None;
    let mut ticks: u64 = 0;
    while !term.load(Ordering::Relaxed) {
        let poll = {
            let mut engine = shared.engine();
            if ticks.is_multiple_of(15) {
                engine.set_conflicts(conflicts::running_fan_apps());
            }
            let status = engine.tick(now_s());
            let now = (status.decision, status.reason);
            if should_log(last_logged, now) {
                let fans: Vec<String> =
                    status.fans.iter().filter_map(|f| f.reading.map(|r| format!("{:.0}", r.actual_rpm))).collect();
                log!(
                    "{:?} ({:?}) control={:.1}°C hotspot={:?}@{:.1}°C fans=[{}] rpm{}",
                    now.0,
                    now.1,
                    status.control_c.unwrap_or(f64::NAN),
                    status.hotspot_key,
                    status.hotspot_c.unwrap_or(f64::NAN),
                    fans.join(", "),
                    status.last_error.as_ref().map(|e| format!(" error: {e}")).unwrap_or_default(),
                );
                last_logged = Some(now);
            }
            if !status.conflicts.is_empty() && ticks.is_multiple_of(150) {
                log!("warning: {:?} running; it will fight fand over the fans", status.conflicts);
            }
            Duration::from_millis(engine.config().poll_interval_ms)
        };
        ticks += 1;
        // Sleep in short slices so SIGTERM is handled promptly.
        let deadline = now_s() + poll.as_secs_f64();
        while !term.load(Ordering::Relaxed) {
            let left = deadline - now_s();
            if left <= 0.0 {
                break;
            }
            if shared.wait(Duration::from_secs_f64(left.min(0.5))) {
                break; // config changed: apply it now
            }
        }
    }
    log!("shutting down");
    let _ = std::fs::remove_file(&args.socket);
}

/// Log on decision/reason kind changes or duty moves of ≥10 points.
fn should_log(last: Option<(Decision, Reason)>, now: (Decision, Reason)) -> bool {
    let Some(last) = last else { return true };
    if last.1 != now.1 {
        return true;
    }
    match (last.0, now.0) {
        (Decision::Duty { pct: a }, Decision::Duty { pct: b }) => (a - b).abs() >= 10.0,
        (a, b) => std::mem::discriminant(&a) != std::mem::discriminant(&b) || a != b,
    }
}
