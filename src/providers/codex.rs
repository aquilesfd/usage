use super::{Plan, SourceResult, Window, parse_ts, u64v, walk_files};
use crate::model::Record;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Limits {
    pub plan: String,
    pub captured: i64,
    pub primary_pct: f64,
    pub primary_resets: i64,
    pub secondary_pct: f64,
    pub secondary_window: i64,
    pub secondary_resets: i64,
}

fn f64v(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn i64v(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(0)
}

pub fn collect(home: &Path) -> (SourceResult, Option<Limits>) {
    let root = home.join(".codex/sessions");
    let mut records = Vec::new();
    let mut limits: Option<Limits> = None;
    let mut limits_ts = 0i64;
    let files = walk_files(&root, "jsonl");
    for file in &files {
        let text = match fs::read_to_string(file) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut model = String::from("gpt-5-codex");
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("type").and_then(|t| t.as_str()) == Some("turn_context") {
                if let Some(m) = v
                    .get("payload")
                    .and_then(|p| p.get("model"))
                    .and_then(|x| x.as_str())
                {
                    model = m.to_string();
                }
                continue;
            }
            let payload = match v.get("payload") {
                Some(p) => p,
                None => continue,
            };
            if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                continue;
            }
            let ts = v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .and_then(parse_ts)
                .unwrap_or(0);
            if let Some(rl) = payload.get("rate_limits") {
                if ts >= limits_ts && rl.get("primary").is_some() {
                    limits_ts = ts;
                    limits = Some(Limits {
                        plan: rl
                            .get("plan_type")
                            .and_then(|x| x.as_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        captured: ts,
                        primary_pct: f64v(rl.get("primary").unwrap(), "used_percent"),
                        primary_resets: i64v(rl.get("primary").unwrap(), "resets_at"),
                        secondary_pct: rl
                            .get("secondary")
                            .map(|s| f64v(s, "used_percent"))
                            .unwrap_or(0.0),
                        secondary_window: rl
                            .get("secondary")
                            .map(|s| i64v(s, "window_minutes"))
                            .unwrap_or(0),
                        secondary_resets: rl
                            .get("secondary")
                            .map(|s| i64v(s, "resets_at"))
                            .unwrap_or(0),
                    });
                }
            }
            let info = match payload.get("info") {
                Some(i) => i,
                None => continue,
            };
            let usage = match info
                .get("last_token_usage")
                .or_else(|| info.get("total_token_usage"))
            {
                Some(u) => u,
                None => continue,
            };
            let input_total = u64v(usage, "input_tokens");
            let cached = u64v(usage, "cached_input_tokens");
            let cache_write = u64v(usage, "cache_write_input_tokens");
            let output = u64v(usage, "output_tokens");
            let input = input_total.saturating_sub(cached);
            if input + output + cached + cache_write == 0 {
                continue;
            }
            records.push(Record {
                ts,
                provider: "codex",
                model: model.clone(),
                input,
                output,
                cache_read: cached,
                cache_write,
            });
        }
    }
    (
        SourceResult {
            records,
            found: root.exists(),
            detail: format!("{} rollout files", files.len()),
        },
        limits,
    )
}

pub fn plan_from_limits(l: &Limits) -> Plan {
    let mut windows = vec![Window {
        label: "5h".to_string(),
        used_pct: l.primary_pct,
        resets_at: if l.primary_resets > 0 {
            Some(l.primary_resets)
        } else {
            None
        },
    }];
    if l.secondary_window > 0 {
        windows.push(Window {
            label: "weekly".to_string(),
            used_pct: l.secondary_pct,
            resets_at: if l.secondary_resets > 0 {
                Some(l.secondary_resets)
            } else {
                None
            },
        });
    }
    Plan {
        provider: "codex",
        plan: l.plan.clone(),
        captured: Some(l.captured),
        windows,
        details: Vec::new(),
        note: None,
    }
}

/// Fetch live rate limits from ChatGPT's Codex usage endpoint (read-only).
///
/// The access token is passed to `curl` via its config on stdin so it never
/// appears in the process list. Returns `None` on any failure so callers can
/// fall back to the rollout-derived limits.
fn fetch_usage(token: &str, account_id: &str) -> Option<Value> {
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
            "url = \"https://chatgpt.com/backend-api/wham/usage\"\n\
             header = \"Authorization: Bearer {token}\"\n\
             header = \"ChatGPT-Account-Id: {account_id}\"\n\
             header = \"OpenAI-Beta: codex-1\"\n\
             header = \"originator: Codex Desktop\"\n\
             header = \"Accept: application/json\"\n"
        );
        stdin.write_all(config.as_bytes()).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    if v.get("rate_limit").is_some() {
        Some(v)
    } else {
        None
    }
}

fn window(entry: &Value, label: &str) -> Window {
    Window {
        label: label.to_string(),
        used_pct: entry
            .get("used_percent")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0),
        resets_at: entry.get("reset_at").and_then(|x| x.as_i64()),
    }
}

pub fn fresh_plan(home: &Path) -> Option<Plan> {
    let text = fs::read_to_string(home.join(".codex/auth.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let tokens = v.get("tokens")?;
    let token = tokens.get("access_token").and_then(|x| x.as_str())?;
    let account_id = tokens
        .get("account_id")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let usage = fetch_usage(token, account_id)?;

    let plan_type = usage
        .get("plan_type")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let rate_limit = usage.get("rate_limit")?;
    let mut windows = Vec::new();
    if let Some(p) = rate_limit.get("primary_window") {
        windows.push(window(p, "5h"));
    }
    if let Some(s) = rate_limit.get("secondary_window") {
        windows.push(window(s, "weekly"));
    }
    let mut details = Vec::new();
    if let Some(credits) = usage.get("credits") {
        let has = credits
            .get("has_credits")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let unlimited = credits
            .get("unlimited")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        if has || unlimited {
            let balance = credits
                .get("balance")
                .and_then(|x| x.as_str())
                .unwrap_or("0");
            details.push(format!("credits: {balance}"));
        }
    }
    Some(Plan {
        provider: "codex",
        plan: plan_type,
        captured: Some(chrono::Utc::now().timestamp()),
        windows,
        details,
        note: None,
    })
}
