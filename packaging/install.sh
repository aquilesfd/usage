#!/usr/bin/env bash
# Install the usage CLI, the COSMIC panel applet, and the 5-minute capture timer
# for the current user (no root required).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${HOME}/.local/bin"
app_dir="${HOME}/.local/share/applications"
icon_dir="${HOME}/.local/share/icons/hicolor/scalable/apps"
unit_dir="${HOME}/.config/systemd/user"

echo "==> building release binaries"
cargo build --release --manifest-path "${here}/Cargo.toml" --features applet

echo "==> installing binaries to ${bin_dir}"
install -Dm0755 "${here}/target/release/usage" "${bin_dir}/usage"
install -Dm0755 "${here}/target/release/usage-applet" "${bin_dir}/usage-applet"

echo "==> installing desktop entry and icon"
install -Dm0644 "${here}/res/io.github.aquilesfd.UsageApplet.desktop" \
    "${app_dir}/io.github.aquilesfd.UsageApplet.desktop"
install -Dm0644 "${here}/res/io.github.aquilesfd.UsageApplet-symbolic.svg" \
    "${icon_dir}/io.github.aquilesfd.UsageApplet-symbolic.svg"

echo "==> installing systemd user units"
install -Dm0644 "${here}/packaging/usage-capture.service" "${unit_dir}/usage-capture.service"
install -Dm0644 "${here}/packaging/usage-capture.timer" "${unit_dir}/usage-capture.timer"

echo "==> capturing an initial snapshot"
"${bin_dir}/usage" capture || true

echo "==> enabling the 5-minute capture timer"
systemctl --user daemon-reload
systemctl --user enable --now usage-capture.timer

echo "==> reloading the COSMIC panel"
# Drop applets from the previous panel instance before restarting it.
pkill -x usage-applet 2>/dev/null || true
pkill cosmic-panel 2>/dev/null || true

echo
echo "Installed. The Usage applet is now available to add from"
echo "Settings -> Desktop -> Panel, and snapshots are captured every 5 minutes."
