# Architecture

```
┌──────────────────────────┐   newline-delimited JSON    ┌────────────────────────────────────────┐
│ MacFanOptimizer.app      │ ─────────────────────────▶  │ fand  (root, launchd KeepAlive)        │
│ SwiftUI MenuBarExtra     │   /var/run/                 │                                        │
│ FanOptimizerKit (client) │   macfanoptimizer.sock      │  server ─▶ Engine ─▶ Controller        │
└──────────────────────────┘                             │              │                         │
┌──────────────────────────┐                             │              ▼                         │
│ fanctl (CLI)             │ ─────────────────────────▶  │         SmcHardware ─▶ smc ─▶ IOKit    │
└──────────────────────────┘                             └────────────────────────────────────────┘
```

## Crates

- **`smc`** handles IOKit FFI to the `AppleSMC` user client. It does key info (cached), reads, writes and enumeration, and decodes `flt`, `fpXY`, `spXY`, `ui*`, `si*`, `flag`, `ioft` and `ch8*`. It has no dependencies.
- **`fan-core`** is all the logic, with no OS calls:
  - `curve`: piecewise-linear temperature → duty (% of each fan's min..max range).
  - `controller`: a state machine, `(config, temperature, time) → Decision`.
  - `engine`: one tick of sensors → decide → apply. It owns the write policy (deadband, re-assert after wake or external override) and adaptive sensor scanning.
  - `hardware`: the `Hardware` trait, implemented by `SmcHardware` and `SimHardware`.
  - `sim`: a lumped thermal model with emulated macOS fan policy, plus scenarios and summaries.
  - `protocol`: the wire types. Samples live in `/fixtures` and are tested from both Rust and Swift.
- **`fand`** is the daemon binary, plus a library with `SmcHardware`, the socket server/client and conflict detection.
- **`fanctl`** is the CLI.

## Control algorithm

Each tick (default every 2 s):

1. **Sense.** Every `full_scan_every_ticks` ticks (default 5) all control sensors are read, and the hottest `hot_set_size` (default 16) become the hot set. Other ticks read only the hot set. The control temperature is the mean of the `aggregate_top_n` (default 4) hottest valid readings.
2. **Smooth.** An asymmetric EMA: `rise_tau_s` = 2 s, `fall_tau_s` = 12 s. A gap of more than 5× the poll interval (sleep) resets it. The daemon's clock includes sleep time.
3. **Safety.**
   - In any mode other than System, a smoothed temperature ≥ `critical_c` (95 °C), or a raw reading ≥ `critical_c` + 3, forces 100 %. It stays there until the temperature drops `critical_hysteresis_c` (6 °C) below the threshold.
   - In Fixed mode, a safety floor tracks a curve from 0 % at `critical_c` − 10 °C to 100 % at `critical_c`, so a fixed speed that can't hold the load settles at an equilibrium instead of oscillating.
   - `max_sensor_failures` consecutive failed reads hand control back to macOS.
4. **Track the curve (Smart).**
   - Up: duty moves toward `curve(t)` at up to `ramp_up_pct_per_s` (25 %/s). The first sample jumps directly.
   - Down: once `curve(t + hysteresis_c)` has been below the current duty for `down_hold_s` (30 s), duty ramps down at `ramp_down_pct_per_s` (2 %/s).
   - When duty reaches 0, the fans are released to macOS. They are forced again as soon as the curve asks for more than 0 %.
5. **Apply.** RPM = min + duty × (max − min). The SMC is written only when the target moves by at least `write_deadband_rpm` (50), when it reaches an extreme, when macOS took the fan back (`md == 0`, e.g. after wake), or when another process changed the target.

6. **Verify.** After the first write, each fan must actually follow (forced mode held, target held, RPM within tolerance) within 30 s. If not, the fans go back to macOS and control is disabled until restart. This is what makes it safe to enable control on Macs nobody has validated (see [HARDWARE.md](HARDWARE.md#support-levels)).

All of these knobs are in `config.toml` (see `fanctl default-config`).

## Why these choices

- **Sample every 2 s, decide slowly.** A heat spike on Apple Silicon takes only a few seconds, so checking once a minute would react too late. Slowing down is what needs patience, and the hold-and-ramp handles that. The result is calm fans that still catch spikes.
- **Mean of the hottest 4, not the max.** A single core boosting briefly reads 100 °C, and fans barely affect a local hotspot like that. A small top-N mean still tracks real hot spots and ignores single-sensor noise.
- **Hand back to macOS at idle.** Forced mode can't go below `F<n>Mn` (2317 rpm), but macOS can stop the fans completely.
- **Root daemon plus unprivileged UI.** SMC writes need root. Keeping the root part small, written in Rust, and free of UI shrinks the attack surface. The socket checks the peer uid on every mutating request.

## Testing strategy

| Layer | Where | What |
|---|---|---|
| Codecs | `smc/src/value.rs` | SMC type encode/decode |
| Curve, config, controller | `fan-core` unit tests | Interpolation, validation, hold, hysteresis, critical, failures |
| Engine | `fan-core/src/engine.rs` | Adaptive scan counts, write deadband, override recovery, release, runtime verification (fans ignoring writes, OS reclaiming control, slow unlock) |
| Closed loop | `fan-core/src/sim.rs` | Calibration, reaching max, no hunting, wake recovery, profile ordering |
| Daemon IPC | `fand/tests/socket.rs` | Permissions, persistence, malformed input |
| Wire compatibility | `fixtures/` + `KitChecks` | The Rust and Swift sides agree |
| Hardware | manual, see `HARDWARE.md` | Real fans on real Macs |
