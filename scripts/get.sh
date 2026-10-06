#!/bin/bash
# Install or upgrade MacFanOptimizer from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/estevecastells/macfanoptimizer/main/scripts/get.sh | bash
#
# Environment options:
#   MFO_VERSION=v0.1.0    install a specific release (default: latest)
#   MFO_NO_APP=1          install only the fan service and CLI, not the menu bar app
#   MFO_DOWNLOAD_ONLY=1   download and verify into a temp dir, then stop
#
# Downloads with curl don't get macOS's quarantine flag, so the unsigned app
# opens without a Gatekeeper prompt. Checksums are verified before anything runs.
set -euo pipefail

REPO="estevecastells/macfanoptimizer"
APP_ZIP="MacFanOptimizer-macos-arm64.zip"
CLI_TGZ="macfanoptimizer-macos-arm64.tar.gz"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" ]] || die "MacFanOptimizer only runs on macOS"
[[ "$(uname -m)" == "arm64" ]] || die "MacFanOptimizer requires an Apple Silicon Mac"
major="$(sw_vers -productVersion | cut -d. -f1)"
(( major >= 14 )) || die "macOS 14 or later is required"

version="${MFO_VERSION:-latest}"
if [[ "$version" == "latest" ]]; then
  base="https://github.com/${REPO}/releases/latest/download"
else
  base="https://github.com/${REPO}/releases/download/${version}"
fi

tmp="$(mktemp -d)"
if [[ -z "${MFO_DOWNLOAD_ONLY:-}" ]]; then
  trap 'rm -rf "$tmp"' EXIT
fi

say "Downloading MacFanOptimizer (${version})"
for f in SHA256SUMS "$CLI_TGZ" "$APP_ZIP"; do
  curl -fSL --progress-bar -o "${tmp}/${f}" "${base}/${f}" || die "download failed: ${base}/${f}"
done

say "Verifying checksums"
(cd "$tmp" && shasum -a 256 -c SHA256SUMS) || die "checksum mismatch; aborting"

tar -xzf "${tmp}/${CLI_TGZ}" -C "$tmp"

if [[ -n "${MFO_DOWNLOAD_ONLY:-}" ]]; then
  say "Downloaded and verified in ${tmp}"
  exit 0
fi

say "Installing the fan service (requires your password)"
sudo /bin/bash "${tmp}/macfanoptimizer/install.sh" --bin-dir "${tmp}/macfanoptimizer" --uid "$(id -u)"

if [[ -z "${MFO_NO_APP:-}" ]]; then
  dest="/Applications"
  [[ -w "$dest" ]] || dest="${HOME}/Applications"
  mkdir -p "$dest"
  say "Installing the menu bar app to ${dest}"
  pkill -x MacFanOptimizer 2>/dev/null || true
  rm -rf "${dest}/MacFanOptimizer.app"
  ditto -x -k "${tmp}/${APP_ZIP}" "$dest"
  open "${dest}/MacFanOptimizer.app"
fi

say "Done. Try: fanctl status"
