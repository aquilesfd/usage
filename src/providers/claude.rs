use super::{Plan, SourceResult, Window, parse_ts, u64v, walk_files};
use crate::model::Record;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn collect(home: &Path) -> SourceResult {
    let root = home.join(".claude/projects");
    let mut records = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let files = walk_files(&root, "jsonl");
    for file in &files {
        let text = match fs::read_to_string(file) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                continue;
            }
            let msg = match v.get("message") {
                Some(m) => m,
                None => continue,
            };
            let usage = match msg.get("usage") {
                Some(u) => u,
                None => continue,
            };
            if let Some(id) = msg.get("id").and_then(|x| x.as_str()) {
                if !id.is_empty() && !seen.insert(id.to_string()) {
                    continue;
                }
            }
            let input = u64v(usage, "input_tokens");
            let output = u64v(usage, "output_tokens");
            let cache_read = u64v(usage, "cache_read_input_tokens");
            let cache_write = u64v(usage, "cache_creation_input_tokens");
            if input + output + cache_read + cache_write == 0 {
                continue;
            }
            let ts = v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .and_then(parse_ts)
                .unwrap_or(0);
            let model = msg
                .get("model")
                .and_then(|x| x.as_str())
                .unwrap_or("claude")
                .to_string();
            records.push(Record {
                ts,
                provider: "claude",
                model,
                input,
                output,
                cache_read,
                cache_write,
            });
        }
    }
    SourceResult {
        records,
        found: root.exists(),
        detail: format!("{} session files", files.len()),
    }
}

fn credentials(home: &Path) -> Option<Value> {
    let text = fs::read_to_string(home.join(".claude/.credentials.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("claudeAiOauth").cloned()
}

fn subscription(home: &Path) -> Option<String> {
    credentials(home)?
        .get("subscriptionType")?
        .as_str()
        .map(str::to_string)
}

fn token_expired(home: &Path) -> bool {
    let Some(oauth) = credentials(home) else {
        return true;
    };
    let expires = oauth.get("expiresAt").and_then(|x| x.as_i64()).unwrap_or(0);
    expires <= chrono::Utc::now().timestamp_millis() + 60_000
}

/// Build a plan from an object carrying `five_hour` / `seven_day` windows.
fn build_plan(u: &Value, plan_name: String, captured: Option<i64>) -> Plan {
    let mut windows = Vec::new();
    if let Some(fh) = u.get("five_hour") {
        windows.push(Window {
            label: "5h".to_string(),
            used_pct: fh
                .get("utilization")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            resets_at: fh
                .get("resets_at")
                .and_then(|x| x.as_str())
                .and_then(parse_ts),
        });
    }
    if let Some(sd) = u.get("seven_day") {
        windows.push(Window {
            label: "weekly".to_string(),
            used_pct: sd
                .get("utilization")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            resets_at: sd
                .get("resets_at")
                .and_then(|x| x.as_str())
                .and_then(parse_ts),
        });
    }
    let mut details = Vec::new();
    if let Some(rows) = u
        .get("seven_day_breakdown")
        .and_then(|b| b.get("rows"))
        .and_then(|r| r.as_array())
    {
        let parts: Vec<String> = rows
            .iter()
            .filter_map(|r| {
                let name = r.get("display_name").and_then(|x| x.as_str())?;
                let pct = r.get("percent").and_then(|x| x.as_f64()).unwrap_or(0.0);
                if pct <= 0.0 {
                    return None;
                }
                Some(format!("{name} {pct:.0}%"))
            })
            .collect();
        if !parts.is_empty() {
            details.push(format!("weekly split: {}", parts.join(" / ")));
        }
    }
    if let Some(eu) = u.get("extra_usage") {
        let enabled = eu
            .get("is_enabled")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let used = eu
            .get("used_credits")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let limit = eu
            .get("monthly_limit")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let cur = eu.get("currency").and_then(|x| x.as_str()).unwrap_or("");
        if used > 0.0 || enabled {
            details.push(format!(
                "extra usage: {} ({used:.0}/{limit:.0} {cur})",
                if enabled { "on" } else { "off" }
            ));
        }
    }
    Plan {
        provider: "claude",
        plan: plan_name,
        captured,
        windows,
        details,
        note: None,
    }
}

/// Read Claude Code's cached utilization written to `~/.claude.json`.
fn cached_plan(home: &Path) -> Option<Plan> {
    let text = fs::read_to_string(home.join(".claude.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let cu = v.get("cachedUsageUtilization")?;
    let u = cu.get("utilization")?;
    let captured = cu
        .get("fetchedAtMs")
        .and_then(|x| x.as_i64())
        .map(|ms| ms / 1000);
    let mut plan = build_plan(
        u,
        subscription(home).unwrap_or_else(|| "claude".to_string()),
        captured,
    );
    if token_expired(home) {
        plan.note = Some("cached; OAuth token expired — run Claude Code to refresh".to_string());
    }
    Some(plan)
}

/// Fetch fresh utilization from the (undocumented) OAuth usage endpoint.
///
/// Read-only: the OAuth token is passed to `curl` via its config on stdin so it
/// never appears in the process list, and credentials are never written back.
/// Returns `None` on any failure so callers can fall back to cached data.
fn fetch_usage(token: &str) -> Option<Value> {
    let mut child = Command::new("curl")
        .args(["-s", "--max-time", "8", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        let config = format!(
            "url = \"https://api.anthropic.com/api/oauth/usage\"\n\
             header = \"Authorization: Bearer {token}\"\n\
             header = \"anthropic-beta: oauth-2025-04-20\"\n\
             header = \"Accept: application/json\"\n\
             header = \"User-Agent: claude-code/usage\"\n"
        );
        stdin.write_all(config.as_bytes()).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    if v.get("five_hour").is_some() || v.get("seven_day").is_some() {
        Some(v)
    } else {
        None
    }
}

fn fresh_plan(home: &Path) -> Option<Plan> {
    let oauth = credentials(home)?;
    let mut token = oauth
        .get("accessToken")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let expires = oauth.get("expiresAt").and_then(|x| x.as_i64()).unwrap_or(0);
    let now_ms = chrono::Utc::now().timestamp_millis();
    if token.is_empty() || expires <= now_ms + 60_000 {
        token = claude_refresh(home)?;
    }
    if token.is_empty() {
        return None;
    }
    let usage = fetch_usage(&token)?;
    Some(build_plan(
        &usage,
        subscription(home).unwrap_or_else(|| "claude".to_string()),
        Some(now_ms / 1000),
    ))
}

/// Claude Code's public OAuth client id.
const OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const OAUTH_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";

fn claude_running() -> bool {
    // Don't race a live Claude Code session for the rotating refresh token.
    Command::new("pgrep")
        .args(["-x", "claude"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Exchange the stored refresh token for a new access token.
fn refresh_token(refresh: &str) -> Option<Value> {
    let mut child = Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "10",
            "-X",
            "POST",
            OAUTH_TOKEN_URL,
            "-H",
            "Content-Type: application/json",
            "-H",
            "Accept: application/json",
            "--data-binary",
            "@-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh,
        "client_id": OAUTH_CLIENT_ID,
    })
    .to_string();
    child.stdin.as_mut()?.write_all(body.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("access_token").and_then(|x| x.as_str())?;
    Some(v)
}

/// Atomically replace the credentials file with `value`, mode 0600.
fn write_credentials(path: &Path, value: &Value) -> bool {
    use std::os::unix::fs::OpenOptionsExt;
    let data = match serde_json::to_vec_pretty(value) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let tmp = path.with_extension("json.tmp");
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
    {
        Ok(f) => f,
        Err(_) => return false,
    };
    if file.write_all(&data).is_err() || file.sync_all().is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    drop(file);
    std::fs::rename(&tmp, path).is_ok()
}

/// Back off after a failed refresh so we don't hammer the token endpoint.
const REFRESH_COOLDOWN_SECS: i64 = 900;

fn refresh_state_path() -> std::path::PathBuf {
    crate::paths::usage_dir().join("claude-refresh.state")
}

fn refresh_cooldown_active() -> bool {
    let Ok(text) = fs::read_to_string(refresh_state_path()) else {
        return false;
    };
    let Ok(last) = text.trim().parse::<i64>() else {
        return false;
    };
    chrono::Utc::now().timestamp() - last < REFRESH_COOLDOWN_SECS
}

fn mark_refresh_attempt() {
    let path = refresh_state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = fs::write(path, chrono::Utc::now().timestamp().to_string());
}

/// Refresh the OAuth token if needed. Serialized with a file lock, re-reads
/// credentials under the lock, skips while Claude Code is running, and writes
/// back atomically. Set `USAGE_CLAUDE_REFRESH=0` to disable.
fn claude_refresh(home: &Path) -> Option<String> {
    use std::os::unix::io::AsRawFd;
    if std::env::var("USAGE_CLAUDE_REFRESH").ok().as_deref() == Some("0") {
        return None;
    }
    if claude_running() || refresh_cooldown_active() {
        return None;
    }
    let lock_path = crate::paths::usage_dir().join("claude-refresh.lock");
    if let Some(parent) = lock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)
        .ok()?;
    let fd = lock.as_raw_fd();
    if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
        return None;
    }

    let result = (|| {
        // Re-read under the lock: another process may have refreshed already.
        let oauth = credentials(home)?;
        let existing = oauth
            .get("accessToken")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let expires = oauth.get("expiresAt").and_then(|x| x.as_i64()).unwrap_or(0);
        let now = chrono::Utc::now().timestamp_millis();
        if !existing.is_empty() && expires > now + 60_000 {
            return Some(existing);
        }
        let refresh = oauth
            .get("refreshToken")
            .and_then(|x| x.as_str())?
            .to_string();
        if refresh.is_empty() {
            return None;
        }
        mark_refresh_attempt();
        let response = refresh_token(&refresh)?;
        let new_access = response.get("access_token")?.as_str()?.to_string();
        let new_refresh = response
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        let expires_in = response
            .get("expires_in")
            .and_then(|x| x.as_i64())
            .unwrap_or(3600);

        let path = home.join(".claude/.credentials.json");
        let text = fs::read_to_string(&path).ok()?;
        let mut full: Value = serde_json::from_str(&text).ok()?;
        {
            let obj = full.get_mut("claudeAiOauth")?.as_object_mut()?;
            obj.insert("accessToken".into(), Value::String(new_access.clone()));
            if let Some(r) = new_refresh {
                obj.insert("refreshToken".into(), Value::String(r));
            }
            obj.insert(
                "expiresAt".into(),
                Value::Number((now + expires_in * 1000).into()),
            );
        }
        if !write_credentials(&path, &full) {
            return None;
        }
        Some(new_access)
    })();

    unsafe { libc::flock(fd, libc::LOCK_UN) };
    result
}

pub fn plan(home: &Path) -> Option<Plan> {
    fresh_plan(home).or_else(|| cached_plan(home))
}
