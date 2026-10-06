#!/bin/bash
# Install (or upgrade) the MacFanOptimizer daemon as a launchd system service.
#
#   sudo scripts/install.sh [--bin-dir DIR] [--uid UID]
#
# --bin-dir  directory containing the `fand` and `fanctl` binaries
#            (default: this script's directory if they are there, as in a
#            release download; otherwise target/release of the checkout)
# --uid      user allowed to change settings without sudo (default: $SUDO_UID)
set -euo pipefail

LABEL="io.github.estevecastells.macfanoptimizer"
HELPER="/Library/PrivilegedHelperTools/${LABEL}.fand"
PLIST="/Library/LaunchDaemons/${LABEL}.plist"
CONFIG_DIR="/Library/Application Support/MacFanOptimizer"
CONFIG="${CONFIG_DIR}/config.toml"
LOG="/var/log/macfanoptimizer.log"
CLI="/usr/local/bin/fanctl"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -x "${here}/fand" ]]; then
  bin_dir="$here"
else
  bin_dir="${here}/../target/release"
fi
allowed_uid="${SUDO_UID:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin-dir) bin_dir="$2"; shift 2 ;;
    --uid) allowed_uid="$2"; shift 2 ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ $EUID -ne 0 ]]; then
  echo "install.sh must run as root: sudo $0 $*" >&2
  exit 1
fi
for b in fand fanctl; do
  [[ -x "${bin_dir}/${b}" ]] || { echo "missing ${bin_dir}/${b}; run 'make build' first" >&2; exit 1; }
done

echo "==> Installing daemon to ${HELPER}"
mkdir -p "$(dirname "$HELPER")" "$(dirname "$CLI")"
# Stop the running instance first; it returns fans to macOS on SIGTERM.
launchctl bootout "system/${LABEL}" 2>/dev/null || true
install -m 755 -o root -g wheel "${bin_dir}/fand" "$HELPER"
install -m 755 -o root -g wheel "${bin_dir}/fanctl" "$CLI"
# Binaries from a browser download carry a quarantine flag; launchd must not trip over it.
xattr -d com.apple.quarantine "$HELPER" "$CLI" 2>/dev/null || true

mkdir -p "$CONFIG_DIR"
if [[ ! -f "$CONFIG" ]]; then
  echo "==> Writing default config to ${CONFIG}"
  "$HELPER" --print-default-config > "$CONFIG"
  if [[ -n "$allowed_uid" && "$allowed_uid" != "0" ]]; then
    sed -i '' "s/^allowed_uids = \[\]/allowed_uids = [${allowed_uid}]/" "$CONFIG"
  fi
else
  echo "==> Keeping existing config ${CONFIG}"
fi
# Keep an uninstaller on the system, so removal doesn't depend on the download/checkout.
install -m 755 -o root -g wheel "${here}/uninstall.sh" "${CONFIG_DIR}/uninstall.sh"
chown -R root:wheel "$CONFIG_DIR"
chmod 755 "$CONFIG_DIR"
chmod 644 "$CONFIG"

# Rotate the log at 1 MB, keep 3 compressed copies.
echo "${LOG} 644 3 1024 * J" > /etc/newsyslog.d/macfanoptimizer.conf

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>${LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${HELPER}</string>
    <string>--config</string><string>${CONFIG}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>ExitTimeOut</key><integer>10</integer>
  <key>StandardErrorPath</key><string>${LOG}</string>
  <key>StandardOutPath</key><string>${LOG}</string>
</dict>
</plist>
EOF
chown root:wheel "$PLIST"
chmod 644 "$PLIST"

echo "==> Starting ${LABEL}"
launchctl bootstrap system "$PLIST"
launchctl enable "system/${LABEL}"

for _ in 1 2 3 4 5 6 7 8 9 10; do
  [[ -S /var/run/macfanoptimizer.sock ]] && break
  sleep 0.5
done

if pgrep -qx "Macs Fan Control"; then
  echo
  echo "WARNING: Macs Fan Control is running. Two fan controllers will fight over the fans."
  echo "         Quit it (and disable its login item) before relying on MacFanOptimizer."
fi

echo
"$CLI" status || { echo "daemon did not start; see ${LOG}" >&2; exit 1; }
echo
echo "Installed. Logs: ${LOG}"
echo "Uninstall: sudo '${CONFIG_DIR}/uninstall.sh'"
