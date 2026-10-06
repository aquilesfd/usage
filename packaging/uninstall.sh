#!/usr/bin/env bash
# Remove the user-level installation created by packaging/install.sh.
set -euo pipefail

bin_dir="${HOME}/.local/bin"
app_dir="${HOME}/.local/share/applications"
icon_dir="${HOME}/.local/share/icons/hicolor/scalable/apps"
unit_dir="${HOME}/.config/systemd/user"

systemctl --user disable --now usage-capture.timer 2>/dev/null || true

rm -f "${bin_dir}/usage" "${bin_dir}/usage-applet"
rm -f "${app_dir}/io.github.aquilesfd.UsageApplet.desktop"
rm -f "${icon_dir}/io.github.aquilesfd.UsageApplet-symbolic.svg"
rm -f "${unit_dir}/usage-capture.service" "${unit_dir}/usage-capture.timer"

systemctl --user daemon-reload
pkill cosmic-panel 2>/dev/null || true

echo "Removed. Capture history in ${HOME}/.local/share/usage was left in place."
