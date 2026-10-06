//! `fanctl` — talk to the daemon, inspect the SMC, and run the simulator.

use clap::{Parser, Subcommand, ValueEnum};
use fan_core::protocol::{Request, Response, Status, DEFAULT_SOCKET_PATH};
use fan_core::sim::{self, Scenario};
use fan_core::{Config, Hardware, Mode, Profile};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "fanctl", version, about = "MacFanOptimizer control and diagnostics")]
struct Cli {
    /// Daemon socket path.
    #[arg(long, global = true, default_value = DEFAULT_SOCKET_PATH)]
    socket: PathBuf,
    /// Print raw JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show daemon status (fans, temperatures, decision).
    Status,
    /// Refresh status every interval until interrupted.
    Watch {
        #[arg(default_value_t = 2.0)]
        seconds: f64,
    },
    /// Set the control mode.
    Mode {
        #[arg(value_enum)]
        mode: ModeArg,
        /// RPM for `fixed`.
        rpm: Option<f64>,
    },
    /// Select the smart-mode curve profile.
    Profile {
        #[arg(value_enum)]
        profile: ProfileArg,
    },
    /// Print the daemon's current configuration (TOML).
    Config,
    /// List temperature sensors. Reads the SMC directly; no daemon needed.
    Sensors {
        /// Include non-control sensors (battery, SSD, ...).
        #[arg(long)]
        all: bool,
    },
    /// Read fans and the control temperature directly from the SMC (no daemon needed).
    Probe,
    /// Dump raw SMC keys, optionally filtered by prefix.
    Keys { prefix: Option<String> },
    /// Low-level fan writes for hardware bring-up. Requires root; stop the daemon first.
    Fan {
        #[command(subcommand)]
        action: FanAction,
    },
    /// Run the control loop against the thermal simulator.
    Simulate {
        #[arg(value_enum, default_value = "sustained")]
        scenario: ScenarioArg,
        #[arg(long, value_enum, default_value = "smart")]
        mode: ModeArg,
        #[arg(long, value_enum, default_value = "balanced")]
        profile: ProfileArg,
        #[arg(long, default_value_t = 600.0)]
        duration: f64,
        /// Print every sample as CSV instead of a summary table.
        #[arg(long)]
        csv: bool,
    },
    /// Compare all profiles and macOS's default across every scenario.
    Benchmark,
    /// Print the default configuration.
    DefaultConfig,
}

#[derive(Subcommand)]
enum FanAction {
    /// Force a fan (or all fans) to an RPM.
    Set {
        /// Fan index, or `all`.
        fan: String,
        rpm: f64,
    },
    /// Return all fans to macOS control.
    Release,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    System,
    Smart,
    Fixed,
    Max,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProfileArg {
    Quiet,
    Balanced,
    Performance,
    Custom,
}

impl From<ProfileArg> for Profile {
    fn from(p: ProfileArg) -> Self {
        match p {
            ProfileArg::Quiet => Profile::Quiet,
            ProfileArg::Balanced => Profile::Balanced,
            ProfileArg::Performance => Profile::Performance,
            ProfileArg::Custom => Profile::Custom,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ScenarioArg {
    Idle,
    Sustained,
    Bursty,
    Ramp,
    Extreme,
}

impl From<ScenarioArg> for Scenario {
    fn from(s: ScenarioArg) -> Self {
        match s {
            ScenarioArg::Idle => Scenario::Idle,
            ScenarioArg::Sustained => Scenario::Sustained,
            ScenarioArg::Bursty => Scenario::Bursty,
            ScenarioArg::Ramp => Scenario::Ramp,
            ScenarioArg::Extreme => Scenario::Extreme,
        }
    }
}

fn mode_from(m: ModeArg, rpm: Option<f64>) -> Result<Mode, String> {
    Ok(match m {
        ModeArg::System => Mode::System,
        ModeArg::Smart => Mode::Smart,
        ModeArg::Max => Mode::Max,
        ModeArg::Fixed => Mode::Fixed { rpm: rpm.ok_or("fixed mode needs an RPM, e.g. `fanctl mode fixed 3500`")? },
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fanctl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn call(cli: &Cli, req: Request) -> Result<Response, String> {
    match fand::client::request(&cli.socket, &req)? {
        Response::Error { message } => Err(message),
        r => Ok(r),
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    match &cli.cmd {
        Cmd::Status => {
            let Response::Status { status } = call(cli, Request::Status)? else {
                return Err("unexpected response".into());
            };
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&status).unwrap());
            } else {
                print_status(&status);
            }
        }
        Cmd::Watch { seconds } => loop {
            let Response::Status { status } = call(cli, Request::Status)? else {
                return Err("unexpected response".into());
            };
            print!("\x1b[2J\x1b[H");
            print_status(&status);
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds.max(0.25)));
        },
        Cmd::Mode { mode, rpm } => {
            call(cli, Request::SetMode { mode: mode_from(*mode, *rpm)? })?;
            println!("ok");
        }
        Cmd::Profile { profile } => {
            call(cli, Request::SetProfile { profile: (*profile).into() })?;
            println!("ok");
        }
        Cmd::Config => {
            let Response::Config { config } = call(cli, Request::GetConfig)? else {
                return Err("unexpected response".into());
            };
            print!("{}", config.to_toml());
        }
        Cmd::Sensors { all } => sensors(cli, *all)?,
        Cmd::Probe => probe()?,
        Cmd::Keys { prefix } => keys(prefix.as_deref())?,
        Cmd::Fan { action } => fan(action)?,
        Cmd::Simulate { scenario, mode, profile, duration, csv } => {
            let cfg = Config { mode: mode_from(*mode, Some(3500.0))?, profile: (*profile).into(), ..Config::default() };
            let (samples, sum) = sim::run(&cfg, (*scenario).into(), *duration);
            if *csv {
                println!("t,power_w,die_c,control_c,fan_rpm,decision,reason");
                for s in samples {
                    println!(
                        "{:.1},{:.1},{:.2},{:.2},{:.0},{:?},{:?}",
                        s.t,
                        s.power_w,
                        s.die_c,
                        s.control_c.unwrap_or(f64::NAN),
                        s.fan_rpm,
                        s.decision,
                        s.reason
                    );
                }
            } else {
                for s in samples.iter().step_by((30_000 / cfg.poll_interval_ms).max(1) as usize) {
                    println!(
                        "t={:>5.0}s  power={:>5.1}W  die={:>5.1}°C  fan={:>5.0} rpm  {:?}",
                        s.t, s.power_w, s.die_c, s.fan_rpm, s.reason
                    );
                }
                println!("\n{sum:#?}");
            }
        }
        Cmd::Benchmark => benchmark(),
        Cmd::DefaultConfig => print!("{}", Config::default().to_toml()),
    }
    Ok(())
}

fn print_status(s: &Status) {
    let mode = match &s.mode {
        Mode::Fixed { rpm } => format!("fixed {rpm:.0} rpm"),
        m => format!("{m:?}").to_lowercase(),
    };
    println!("mode      {mode}  (profile {:?})", s.profile);
    println!("decision  {:?}  reason {:?}", s.decision, s.reason);
    println!(
        "control   {:.1} °C   hotspot {:.1} °C ({})",
        s.control_c.unwrap_or(f64::NAN),
        s.hotspot_c.unwrap_or(f64::NAN),
        s.hotspot_key.map(|k| k.to_string()).unwrap_or_default()
    );
    for f in &s.fans {
        let r = f.reading;
        println!(
            "fan {}     {:>5.0} rpm  (target {:>5.0}, range {:.0}–{:.0}, {})",
            f.info.index,
            r.map_or(f64::NAN, |r| r.actual_rpm),
            r.map_or(f64::NAN, |r| r.target_rpm),
            f.info.min_rpm,
            f.info.max_rpm,
            if r.is_some_and(|r| r.forced) { "forced" } else { "macOS" }
        );
    }
    for g in s.groups.iter().take(6) {
        println!("  {:<22} max {:>5.1} °C  avg {:>5.1} °C  ({} sensors)", g.group, g.max_c, g.avg_c, g.count);
    }
    println!(
        "model {}  writes {}  tick {} µs  smc writes {}  uptime {:.0}s",
        s.model,
        if s.writes_enabled { "on" } else { "OFF (read-only)" },
        s.tick_us,
        s.smc_writes,
        s.uptime_s
    );
    if let Some(e) = &s.last_error {
        println!("error: {e}");
    }
    if !s.conflicts.is_empty() {
        println!("warning: {} is running and will fight over the fans. Quit it.", s.conflicts.join(", "));
    }
    if s.external_override {
        println!("warning: another process changed the fan targets");
    }
}

fn open_hw(writes: bool) -> Result<fand::SmcHardware, String> {
    fand::SmcHardware::open(writes)
}

fn sensors(cli: &Cli, all: bool) -> Result<(), String> {
    let mut hw = open_hw(false)?;
    let cfg = Config::default();
    let control = fan_core::sensors::select_control_keys(hw.temperature_keys(), &cfg.sensors);
    let keys: Vec<_> = hw.temperature_keys().to_vec();
    let mut rows: Vec<_> = keys
        .into_iter()
        .filter(|k| all || control.contains(k))
        .map(|k| (fan_core::sensors::group_of(k), k, hw.read_temperature(k)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)));
    if cli.json {
        let v: Vec<_> = rows
            .iter()
            .map(|(g, k, c)| serde_json::json!({"key": k, "group": g, "celsius": c, "control": control.contains(k)}))
            .collect();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return Ok(());
    }
    let mut last = "";
    for (g, k, c) in rows {
        if g != last {
            println!("\n{g}");
            last = g;
        }
        let mark = if control.contains(&k) { "*" } else { " " };
        println!("  {mark}{k}  {:>6.1} °C", c.unwrap_or(f64::NAN));
    }
    println!("\n* = drives the controller");
    Ok(())
}

fn probe() -> Result<(), String> {
    let model = fand::hardware_model();
    let hw = open_hw(false)?;
    println!(
        "model {model} ({}), {} fans, {} temperature sensors, Ftst unlock key: {}",
        if fand::is_supported_model(&model) { "supported" } else { "NOT validated" },
        hw.fans().len(),
        hw.temperature_keys().len(),
        hw.has_unlock_key()
    );
    let mut engine =
        fan_core::Engine::new(hw, Config { mode: Mode::System, ..Config::default() }).map_err(|e| e.to_string())?;
    // System mode with writes disabled: a pure read of what the controller sees.
    engine.set_machine(model, false);
    let status = engine.tick(0.0).clone();
    print_status(&status);
    Ok(())
}

fn keys(prefix: Option<&str>) -> Result<(), String> {
    let mut smc = smc::Smc::open().map_err(|e| e.to_string())?;
    for k in smc.all_keys().map_err(|e| e.to_string())? {
        let name = k.to_string();
        if prefix.is_some_and(|p| !name.starts_with(p)) {
            continue;
        }
        match smc.read_raw(k) {
            Ok((info, bytes)) => {
                println!("{name}  {}  {:>2}  {}", info.data_type, info.size, smc::decode(info.data_type, &bytes))
            }
            Err(e) => println!("{name}  error: {e}"),
        }
    }
    Ok(())
}

fn fan(action: &FanAction) -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("fan writes require root: sudo fanctl fan ...".into());
    }
    let model = fand::hardware_model();
    if !fand::is_supported_model(&model) {
        return Err(format!("{model} is not a validated model; refusing to write"));
    }
    let mut hw = open_hw(true)?;
    let fans: Vec<_> = hw.fans().to_vec();
    match action {
        FanAction::Set { fan, rpm } => {
            let targets: Vec<_> = if fan == "all" {
                fans.clone()
            } else {
                let i: u8 = fan.parse().map_err(|_| format!("bad fan index {fan}"))?;
                fans.iter().filter(|f| f.index == i).copied().collect()
            };
            if targets.is_empty() {
                return Err(format!("no fan {fan}"));
            }
            for f in targets {
                let r = f.clamp(*rpm);
                hw.set_target(f.index, r)?;
                println!("fan {} → {r:.0} rpm (forced)", f.index);
            }
        }
        FanAction::Release => {
            for f in &fans {
                hw.release(f.index)?;
            }
            println!("all fans returned to macOS control");
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    for f in &fans {
        let r = hw.read_fan(f.index)?;
        println!("fan {}: actual {:.0} rpm, target {:.0}, forced {}", f.index, r.actual_rpm, r.target_rpm, r.forced);
    }
    Ok(())
}

fn benchmark() {
    let configs: Vec<(&str, Config)> = vec![
        ("macOS default", Config { mode: Mode::System, ..Config::default() }),
        ("quiet", Config { profile: Profile::Quiet, ..Config::default() }),
        ("balanced", Config { profile: Profile::Balanced, ..Config::default() }),
        ("performance", Config { profile: Profile::Performance, ..Config::default() }),
        ("fixed 6000", Config { mode: Mode::Fixed { rpm: 6000.0 }, ..Config::default() }),
    ];
    println!(
        "{:<10} {:<14} {:>8} {:>9} {:>9} {:>8} {:>7} {:>6}",
        "scenario", "policy", "max °C", "final °C", "avg rpm", ">90°C s", "writes", "flips"
    );
    for scenario in Scenario::ALL {
        for (name, cfg) in &configs {
            let (_, s) = sim::run(cfg, scenario, 1200.0);
            println!(
                "{:<10} {:<14} {:>8.1} {:>9.1} {:>9.0} {:>8.0} {:>7} {:>6}",
                scenario.name(),
                name,
                s.max_die_c,
                s.final_die_c,
                s.avg_fan_rpm,
                s.seconds_above_90c,
                s.smc_writes,
                s.direction_changes
            );
        }
        println!();
    }
}
