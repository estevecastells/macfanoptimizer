# Security policy

`fand` runs as root and accepts requests over a local Unix socket, so we take security reports seriously.

## Reporting

Please report vulnerabilities privately via [GitHub security advisories](https://github.com/estevecastells/macfanoptimizer/security/advisories/new), not public issues. Reports are reviewed with the weekly triage, or sooner for anything severe.

## Threat model (summary)

- Any local user can connect to `/var/run/macfanoptimizer.sock` and read status.
- Changing settings requires the connecting process's uid (checked with `getpeereid`) to be root or listed in `allowed_uids`. Only root can change `allowed_uids`.
- The daemon only writes the SMC fan keys `F<n>md`, `F<n>Tg` and, on older chips, `Ftst`. It never writes keys supplied by clients.
- Requests are size-limited JSON lines. Malformed input gets an error response and never crashes the daemon (see `crates/fand/tests/socket.rs`).
