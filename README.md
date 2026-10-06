# MacFanOptimizer

[![CI](https://github.com/estevecastells/macfanoptimizer/actions/workflows/ci.yml/badge.svg)](https://github.com/estevecastells/macfanoptimizer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**An open-source, automatic alternative to Macs Fan Control for Apple Silicon MacBooks.**

MacFanOptimizer watches your chip's temperature and drives the fans for you. It keeps them silent when the Mac is cool, ramps them to full speed when it's working hard, and slows them down calmly afterwards. You don't have to pick a fixed speed or buy a license to get temperature-based control.

| | macOS default | Macs Fan Control (free) | MacFanOptimizer |
|---|---|---|---|
| Automatic, temperature-driven | ✅ late: lets the chip run hot | ❌ constant speeds only (sensor-based is Pro) | ✅ |
| Reaches max fan speed under sustained load | only near throttling | only if you set it manually | ✅ |
| Silent at idle | ✅ | ❌ while a constant speed is set | ✅ hands control back to macOS |
| Open source | ❌ | ❌ | ✅ MIT |
| Resource use | n/a | Qt app | ~1 MB Rust daemon, ~6 MB RAM, ~3 ms of SMC I/O every 2 s |

> Not affiliated with CrystalIDEA (makers of Macs Fan Control) or Apple. Mentioned only for comparison.

## How it works

```
 menu bar app (Swift, runs as you) ──JSON over Unix socket──▶ fand (Rust, root, launchd)
                                                                  │
 fanctl (CLI) ───────────────────────────────────────────────────┘  reads ~118 die sensors,
                                                                     writes fan targets via the SMC
```

Every 2 seconds the daemon:

1. Reads the SoC die temperature sensors. It does a full scan every 10 s and re-reads only the hottest 16 sensors in between, because each SMC read costs about 135 µs on Apple Silicon.
2. Computes the control temperature: the mean of the 4 hottest sensors, smoothed. Rises are followed within about 2 s, falls over about 12 s.
3. Maps it through a fan curve: **fast up, slow down**. Increases apply right away. Decreases wait for 30 s of lower temperatures, then ramp down gently with 3 °C of hysteresis, so bursty workloads don't make the fans hunt.
4. Writes the SMC only when the target moves by at least 50 rpm. If macOS (after sleep) or another app takes the fans back, it re-asserts its target.
5. When the curve reaches 0 %, it **hands the fans back to macOS**, so they can stop completely at idle.

Safety: a hot chip always wins over a fixed speed, sensor failures hand control back to macOS, and the fans are released on exit, crash or uninstall.

### Modes and profiles

| Mode | Behaviour |
|---|---|
| **Smart** (default) | Temperature curve, using the selected profile |
| **Fixed** | Constant RPM, raised automatically if the chip approaches 95 °C |
| **Max** | All fans at full speed |
| **macOS** | Hands off |

| Profile | Fans start | Full speed at |
|---|---|---|
| Quiet | 66 °C | 92 °C |
| Balanced | 58 °C | 83 °C |
| Performance | 50 °C | 76 °C |
| Custom | `custom_curve` in the config file | |

### Simulated comparison

From `make benchmark`: 20 minutes in the built-in thermal model, which is calibrated against an M5 Pro MacBook Pro. "macOS default" is an *emulation* of Apple's late-ramping policy, not a measurement.

| Sustained heavy load | max °C | time above 90 °C | avg rpm |
|---|---|---|---|
| macOS default (emulated) | 95.9 | 1172 s | 6147 |
| Fixed 6000 rpm | 95.0 | 1130 s | 6396 |
| **Smart / Balanced** | **85.4** | **0 s** | 7627 |

## Supported hardware

| Model | Chip | Status |
|---|---|---|
| `Mac17,9` | M5 Pro MacBook Pro | ✅ reference development machine |

On any other model the daemon runs **read-only**: it shows temperatures and what it *would* do, but never writes to the fans. Run `fanctl probe` to see your model. If you want your Mac supported, open a [model support issue](https://github.com/estevecastells/macfanoptimizer/issues/new?template=model_support.yml). Adding a model is usually a one-line change plus a test report. See [docs/HARDWARE.md](docs/HARDWARE.md).

Requires macOS 14 or later.

## Install

Requires an Apple Silicon Mac on macOS 14 or later. **Quit Macs Fan Control first**, and remove it from your login items. Two fan controllers will fight over the fans; MacFanOptimizer detects that and warns you.

### One-line install (recommended)

```sh
curl -fsSL https://raw.githubusercontent.com/estevecastells/macfanoptimizer/main/scripts/get.sh | bash
```

This downloads the [latest release](https://github.com/estevecastells/macfanoptimizer/releases/latest), verifies its SHA-256 checksums, installs the fan service (asking for your password once), and puts **MacFanOptimizer.app** in Applications. You can [read the script](scripts/get.sh) first.

### Download the app

1. Download `MacFanOptimizer-macos-arm64.zip` from the [latest release](https://github.com/estevecastells/macfanoptimizer/releases/latest), unzip it, and move **MacFanOptimizer.app** to Applications.
2. Open it. The app isn't notarized by Apple yet, so the first time macOS says it can't verify it. Go to **System Settings → Privacy & Security** and click **Open Anyway**.
3. Click the fan icon in the menu bar, then **Install Fan Service…**.

### Command line only

Download `macfanoptimizer-macos-arm64.tar.gz` from the [latest release](https://github.com/estevecastells/macfanoptimizer/releases/latest), then:

```sh
tar -xzf macfanoptimizer-macos-arm64.tar.gz
sudo ./macfanoptimizer/install.sh
```

### From source

You need [Rust](https://rustup.rs) and the Xcode Command Line Tools (`xcode-select --install`). Full Xcode is not required.

```sh
git clone https://github.com/estevecastells/macfanoptimizer
cd macfanoptimizer
make install        # builds, installs the daemon (asks for your password), prints status
make run-app        # builds and opens the menu bar app
```

### Uninstall

```sh
sudo "/Library/Application Support/MacFanOptimizer/uninstall.sh"          # fans go back to macOS
sudo "/Library/Application Support/MacFanOptimizer/uninstall.sh" --purge  # also delete config and logs
```

Then delete MacFanOptimizer.app.

## Command line

```sh
fanctl status                 # temperatures, fans, current decision
fanctl watch                  # live view
fanctl mode smart             # smart | fixed 3500 | max | system
fanctl profile quiet          # quiet | balanced | performance | custom
fanctl config                 # print the active config
fanctl sensors [--all]        # every temperature sensor (no daemon needed)
fanctl probe                  # what the controller sees on this Mac (no daemon needed)
fanctl simulate bursty        # run the controller against the thermal model
fanctl benchmark              # compare policies across scenarios
```

Configuration lives in `/Library/Application Support/MacFanOptimizer/config.toml`. Every tuning knob is documented in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Logs go to `/var/log/macfanoptimizer.log`.

## Development

```sh
make test        # Rust unit, integration and simulator tests + Swift protocol checks
make lint        # rustfmt + clippy -D warnings
make ci          # everything CI runs
```

You can work on everything without root and without touching your fans:

```sh
cargo run -p fand -- --dry-run --socket /tmp/mfo.sock --config /tmp/mfo.toml
MACFANOPTIMIZER_SOCKET=/tmp/mfo.sock build/MacFanOptimizer.app/Contents/MacOS/MacFanOptimizer
```

Repository layout:

| Path | What |
|---|---|
| `crates/smc` | Dependency-free Apple SMC access (IOKit FFI, type codecs) |
| `crates/fan-core` | Pure logic: curves, controller, engine, protocol, thermal simulator |
| `crates/fand` | Root daemon: SMC backend, socket server, launchd integration |
| `crates/fanctl` | CLI |
| `app/` | SwiftUI menu bar app (SwiftPM, no Xcode project) |
| `fixtures/` | Wire-protocol samples shared by the Rust and Swift tests |
| `scripts/` | Install, uninstall, app bundling |

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Pull requests are reviewed once a week.

## License

[MIT](LICENSE)
