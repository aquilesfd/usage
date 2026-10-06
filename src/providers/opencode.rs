use super::sqlite_sessions;
use super::{Plan, SourceResult, Window, parse_ts};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn collect(home: &Path) -> SourceResult {
    let path = home.join(".local/share/opencode/opencode.db");
    sqlite_sessions::collect(&path, "opencode", &["session_v2", "session"])
}

/// Fetch the OpenCode Go subscription usage (rolling / weekly / monthly
/// percentages) from the official endpoint, read-only.
///
/// The API key is passed to `curl` via its config on stdin so it never appears
/// in the process list.
fn fetch_usage(key: &str) -> Option<Value> {
    let mut child = Command::new("curl")
        .args(["-s", "--max-time", "12", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        let config = format!(
            "url = \"https://opencode.ai/zen/go/v1/usage\"\n\
             header = \"Authorization: Bearer {key}\"\n\
             header = \"Accept: application/json\"\n"
        );
        stdin.write_all(config.as_bytes()).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    if v.get("usage").is_some() {
        Some(v)
    } else {
        None
    }
}

pub fn plan(home: &Path) -> Option<Plan> {
    let text = fs::read_to_string(home.join(".local/share/opencode/auth.json")).ok()?;
    let auth: Value = serde_json::from_str(&text).ok()?;
    let key = auth.get("opencode-go")?.get("key")?.as_str()?;
    let usage = fetch_usage(key)?;
    let usage = usage.get("usage")?;

    let mut windows = Vec::new();
    for (label, field) in [
        ("5h", "rolling"),
        ("weekly", "weekly"),
        ("monthly", "monthly"),
    ] {
        if let Some(w) = usage.get(field) {
            windows.push(Window {
                label: label.to_string(),
                used_pct: w.get("percent").and_then(|x| x.as_f64()).unwrap_or(0.0),
                resets_at: w
                    .get("resetsAt")
                    .and_then(|x| x.as_str())
                    .and_then(parse_ts),
            });
        }
    }
    if windows.is_empty() {
        return None;
    }
    Some(Plan {
        provider: "opencode",
        plan: "go".to_string(),
        captured: Some(chrono::Utc::now().timestamp()),
        windows,
        details: Vec::new(),
        note: None,
    })
}
