#!/bin/bash
# Build MacFanOptimizer.app (menu bar UI + bundled daemon and installer).
# Works with Command Line Tools only; no Xcode project needed.
#
#   scripts/build-app.sh            → build/MacFanOptimizer.app
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
app="build/MacFanOptimizer.app"

echo "==> Building Rust daemon and CLI (release)"
cargo build --release --locked -p fand -p fanctl

echo "==> Building Swift menu bar app (release)"
(cd app && swift build -c release --product MacFanOptimizer)
swift_bin="$(cd app && swift build -c release --show-bin-path)/MacFanOptimizer"

echo "==> Assembling ${app}"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$swift_bin" "$app/Contents/MacOS/MacFanOptimizer"
cp target/release/fand target/release/fanctl "$app/Contents/Resources/"
cp scripts/install.sh scripts/uninstall.sh "$app/Contents/Resources/"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>io.github.estevecastells.macfanoptimizer.app</string>
  <key>CFBundleName</key><string>MacFanOptimizer</string>
  <key>CFBundleDisplayName</key><string>MacFanOptimizer</string>
  <key>CFBundleExecutable</key><string>MacFanOptimizer</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHumanReadableCopyright</key><string>MIT License</string>
</dict>
</plist>
EOF

# Ad-hoc signature so Gatekeeper/TCC treat the bundle consistently on this machine.
codesign --force --deep --sign - "$app" >/dev/null
echo "==> Done: $root/$app"
