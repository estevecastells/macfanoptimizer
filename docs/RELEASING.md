# Releasing

Releases are built and published by `.github/workflows/release.yml` when a version tag is pushed.

1. Bump `version` in the root `Cargo.toml` (for example `0.2.0`), then run `cargo build` to refresh `Cargo.lock`.
2. If the UI or the controller changed, refresh the README images:
   ```sh
   fanctl status --json > /tmp/status.json
   (cd app && swift run RenderScreenshots /tmp/status.json ../docs/images)   # menu bar panel, light + dark
   python3 docs/images/make_chart.py                                          # simulator chart, light + dark
   ```
3. Check that `make ci` passes, then commit: `Release v0.2.0`.
4. Tag and push:
   ```sh
   git tag v0.2.0 && git push origin main v0.2.0
   ```
5. The workflow checks that the tag matches the Cargo version, runs the tests, builds the app and CLI, smoke-tests the CLI, and publishes a GitHub release with:
   - `MacFanOptimizer-macos-arm64.zip`: menu bar app, bundling the daemon and installer
   - `macfanoptimizer-macos-arm64.tar.gz`: `fand`, `fanctl`, `install.sh`, `uninstall.sh`
   - `SHA256SUMS`
6. Verify the release: `MFO_DOWNLOAD_ONLY=1 bash scripts/get.sh` downloads the latest release and checks its checksums without installing anything.

Asset names carry no version, so `releases/latest/download/<asset>` always resolves to the newest release. `scripts/get.sh` depends on this.

To build the same artifacts locally, run `scripts/package-release.sh` (output goes to `dist/`).

## Signing

Binaries are ad-hoc signed. Browser downloads of the app therefore need a one-time "Open Anyway" in System Settings; `get.sh` avoids this because `curl` doesn't set the quarantine flag. Signing with a Developer ID and notarizing would remove that step. It needs an Apple Developer Program membership, and the certificate and notary credentials would be stored as repository secrets.
