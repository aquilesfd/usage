# usage

Local multi-provider AI usage dashboard for Pop!_OS / COSMIC, by aquilesfd.

`usage` reads the on-disk session data that AI coding tools already write, then
reports token usage, cost, and plan quotas. It also ships a COSMIC panel applet
and a systemd user timer that captures per-plan usage every 5 minutes.

## Components

| Path | What it is |
| --- | --- |
| `usage` | CLI report (`usage [options]`), snapshot capture (`usage capture`), history (`usage history`) |
| `usage-applet` | COSMIC panel applet (built with `--features applet`) |
| `usage-capture.timer` | systemd user timer that runs `usage capture` every 5 minutes |
| `usage-capture.service` | oneshot unit invoked by the timer |

## Providers

Token usage is collected locally for:

- **claude** — `~/.claude/projects/**/*.jsonl`
- **codex** — `~/.codex/sessions/**/*.jsonl`
- **opencode** — `~/.local/share/opencode/opencode.db`
- **mimocode** — `~/.local/share/mimocode/mimocode.db` (per-step `part` rows)
- **grok** — `~/.grok/logs/unified.jsonl`
- **jcode** — `~/.jcode/sessions`
- **zcode** — `~/.zcode/cli/rollout`
- **kilo** — `~/.local/share/kilo/kilo.db` (schema-compatible with opencode)

Plan quotas / limits are captured for **claude**, **codex**, and **grok**.
**cursor**, **muse**, **kiro**, **devin**, **antigravity**, and **cline** are
detected and shown, but keep their usage server-side; `usage --sources` explains
what is and is not available for each.

## CLI

```sh
usage                          # full report
usage --period today -d 7      # provider breakdown for today, 7-day chart
usage --json                   # machine-readable report
usage --sources                # what was detected and where
usage capture                  # write one snapshot to the history db
usage history -n 20            # recent plan snapshots
usage history -p claude -w 5h  # one provider/window over time
```

## Capture history

Snapshots are stored in `${XDG_DATA_HOME:-~/.local/share}/usage/usage.db`
(SQLite). Each capture writes:

- one `plan_snapshots` row per provider + plan + quota window
  (`used_pct`, `resets_at`, details, note), and
- one `provider_snapshots` row per provider with cumulative tokens, calls, and
  cost.

A JSON cache of the latest capture is written to
`~/.local/share/usage/latest.json`.

## Install

```sh
./packaging/install.sh
```

This builds both binaries, installs them to `~/.local/bin`, installs the desktop
entry and symbolic icon, enables `usage-capture.timer` (every 5 minutes), runs an
initial capture, and restarts `cosmic-panel` so the applet appears.

After installing, add **Usage** from *Settings → Desktop → Panel*.

To remove: `./packaging/uninstall.sh`.

## Panel applet

The panel shows the highest plan utilization across providers, colored by
severity. Click for a popup with:

- every plan and quota window as a progress bar, with reset countdown,
- per-provider token totals, today's cost, and the change since the last
  5-minute capture.

The applet refreshes from the database once a minute; the systemd timer keeps the
database current.

## Development

```sh
cargo build                                  # CLI only (fast)
cargo build --release --features applet      # CLI + applet (compiles libcosmic)
cargo run --features applet --bin usage-applet
```

Requires `libxkbcommon-dev` and `libwayland-dev` to build the applet.
