# Contributing

Thanks for helping. MacFanOptimizer runs as root and controls hardware, so the bar for changes is deliberately high. In return, the process is predictable.

## Review cadence

- **Pull requests and issues are triaged once a week.** Expect a first response within 7 days.
- Only PRs that are **green on CI** and meet the checklist below get reviewed. A red PR waits until the next weekly pass after it goes green.
- Small, focused PRs get merged fastest. For larger changes (new modes, protocol changes, new dependencies), open an issue first so we can agree on the approach before you write code.

## Merge requirements

Every PR must:

1. **Pass CI.** That means `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, the Swift protocol checks, and building the app bundle. Run `make ci` locally to check before you push.
2. **Include tests** for behaviour changes:
   - Controller or engine logic: a unit test in `crates/fan-core`, and a simulator scenario test in `sim.rs` if the change affects dynamics. "It felt fine on my Mac" is not enough. Show it in the simulator.
   - Protocol changes: update `fixtures/`, and both the Rust (`protocol.rs`) and Swift (`KitChecks`) sides.
   - Daemon socket behaviour: `crates/fand/tests/socket.rs`.
3. **Keep the safety invariants.** Fans must be released to macOS on exit and on sensor failure, critical temperature must override Fixed mode, and unsupported models must stay read-only. If a PR weakens any of these, it won't be merged.
4. **Not add dependencies lightly.** The daemon runs as root, so every dependency is attack surface. Justify each one in the PR description.
5. **Be explained.** Say what changed, why, and how you tested it (paste `fanctl` output for hardware-facing changes).

## Adding support for your Mac

This is the most valuable contribution. Use the [model support issue template](https://github.com/estevecastells/macfanoptimizer/issues/new?template=model_support.yml), or send a PR that:

1. Adds your model identifier to `SUPPORTED_MODELS` in `crates/fand/src/lib.rs`.
2. Includes the output of `fanctl probe` and `fanctl keys F` in the PR description.
3. Confirms the manual test from [docs/HARDWARE.md](docs/HARDWARE.md#validating-a-new-model): forcing a speed, releasing back to macOS, and smart mode under load.

## Development setup

```sh
rustup toolchain install stable --component clippy rustfmt
xcode-select --install        # Swift toolchain; full Xcode is not needed
make ci
```

You can develop without root using `fand --dry-run` (see the README). Never test fan writes while another fan-control app is running.

## Code style

- Match the surrounding code. Rust is formatted by `rustfmt.toml` (120 columns). Swift follows standard Swift API guidelines.
- Comments explain *why*, not *what*. Hardware facts (SMC key meanings, measured values) should cite how they were found out.
- Keep `fan-core` free of OS calls, so everything in it stays testable with the simulator.

## Reporting security issues

See [SECURITY.md](SECURITY.md). Please don't open public issues for vulnerabilities.
