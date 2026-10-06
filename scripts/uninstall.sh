#!/bin/bash
# Remove the MacFanOptimizer daemon. Fans are returned to macOS control.
#
#   sudo scripts/uninstall.sh [--purge]     (--purge also deletes config and logs)
set -euo pipefail

LABEL="io.github.estevecastells.macfanoptimizer"

if [[ $EUID -ne 0 ]]; then
  echo "uninstall.sh must run as root: sudo $0 $*" >&2
  exit 1
fi

# SIGTERM makes fand hand the fans back to macOS before exiting.
launchctl bootout "system/${LABEL}" 2>/dev/null || true
rm -f "/Library/LaunchDaemons/${LABEL}.plist" \
      "/Library/PrivilegedHelperTools/${LABEL}.fand" \
      /usr/local/bin/fanctl \
      /var/run/macfanoptimizer.sock \
      /etc/newsyslog.d/macfanoptimizer.conf

if [[ "${1:-}" == "--purge" ]]; then
  rm -rf "/Library/Application Support/MacFanOptimizer"
  rm -f /var/log/macfanoptimizer.log*
fi

echo "MacFanOptimizer daemon removed; fans are under macOS control."
