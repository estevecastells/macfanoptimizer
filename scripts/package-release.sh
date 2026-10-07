#!/bin/bash
# Build release artifacts into dist/:
#   MacFanOptimizer-macos-arm64.zip     menu bar app (bundles the daemon + installer)
#   macfanoptimizer-macos-arm64.tar.gz  fand, fanctl, install/uninstall scripts
#   SHA256SUMS
#   SHA256SUMS.sig                      Ed25519 signature the in-app updater checks (needs $UPDATE_SIGNING_KEY)
#   RELEASE_NOTES.md                    install instructions prepended to the release notes
#
# Asset names carry no version so that .../releases/latest/download/<name> always works.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

export MACOSX_DEPLOYMENT_TARGET=14.0
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
repo="estevecastells/macfanoptimizer"

scripts/build-app.sh

rm -rf dist
stage="dist/stage/macfanoptimizer"
mkdir -p "$stage"
cp target/release/fand target/release/fanctl scripts/install.sh scripts/uninstall.sh LICENSE "$stage/"
codesign --force --sign - "$stage/fand" "$stage/fanctl" >/dev/null
tar -C dist/stage -czf dist/macfanoptimizer-macos-arm64.tar.gz macfanoptimizer
ditto -c -k --sequesterRsrc --keepParent build/MacFanOptimizer.app dist/MacFanOptimizer-macos-arm64.zip
rm -rf dist/stage

(cd dist && shasum -a 256 MacFanOptimizer-macos-arm64.zip macfanoptimizer-macos-arm64.tar.gz > SHA256SUMS)
if [[ -n "${UPDATE_SIGNING_KEY:-}" ]]; then
  swift scripts/sign-update.swift dist/SHA256SUMS > dist/SHA256SUMS.sig
else
  echo "warning: UPDATE_SIGNING_KEY not set; release won't be installable by the in-app updater" >&2
fi

cat > dist/RELEASE_NOTES.md <<EOF
## Install

**Terminal (recommended).** This installs the fan service and the menu bar app and verifies checksums:

\`\`\`sh
curl -fsSL https://raw.githubusercontent.com/${repo}/main/scripts/get.sh | bash
\`\`\`

**App download.** Download \`MacFanOptimizer-macos-arm64.zip\`, unzip it, and move **MacFanOptimizer.app** to Applications. Open it, then click **Install Fan Service…** in the menu bar. The app isn't notarized yet, so the first time macOS will say it can't verify it. Go to **System Settings → Privacy & Security** and click **Open Anyway**.

**Command line only.** Download \`macfanoptimizer-macos-arm64.tar.gz\`, extract it, and run \`sudo ./macfanoptimizer/install.sh\`.

Requires an Apple Silicon Mac on macOS 14 or later. Validated on \`Mac17,9\` (M5 Pro); other Apple Silicon Macs with fans are supported with runtime verification, and fanless Macs run in monitoring mode. See [Supported hardware](https://github.com/${repo}#supported-hardware). Quit Macs Fan Control or any other fan app first. Tried it on a new model? Run \`fanctl report\` and [open a model support issue](https://github.com/${repo}/issues/new?template=model_support.yml).

EOF

echo "==> Release artifacts (v${version}):"
ls -la dist
cat dist/SHA256SUMS
