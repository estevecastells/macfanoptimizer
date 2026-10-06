## What and why

<!-- What does this change, and what problem does it solve? Link the issue. -->

## How it was tested

<!-- Tests added/updated. For hardware-facing changes, paste `fanctl status` / `fanctl probe` output and your Mac model. -->

## Checklist

- [ ] `make ci` passes locally (fmt, clippy -D warnings, tests, Swift checks, app build)
- [ ] Behaviour changes have unit and/or simulator tests
- [ ] Protocol changes update `fixtures/` and both the Rust and Swift sides
- [ ] Safety invariants kept (release on exit/sensor failure, critical overrides fixed, unsupported models read-only)
- [ ] No new dependencies, or each one is justified above

PRs are reviewed once a week; only green PRs are reviewed.
