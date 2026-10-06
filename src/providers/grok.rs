use super::{Plan, SourceResult, Window, parse_ts, u64v};
use crate::model::Record;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn collect(home: &Path) -> SourceResult {
    let path = home.join(".grok/logs/unified.jsonl");
    let mut records = Vec::new();
    if !path.exists() {
        return SourceResult {
            records,
            found: false,
            detail: "not found".to_string(),
        };
    }
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            return SourceResult {
                records,
                found: true,
                detail: "unreadable".to_string(),
            };
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || !line.contains("inference_done") {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("msg").and_then(|m| m.as_str()) != Some("shell.turn.inference_done") {
            continue;
        }
        let ctx = match v.get("ctx") {
            Some(c) => c,
            None => continue,
        };
        let prompt = u64v(ctx, "prompt_tokens");
        let cached = u64v(ctx, "cached_prompt_tokens");
        let output = u64v(ctx, "completion_tokens");
        let input = prompt.saturating_sub(cached);
        if input + output + cached == 0 {
            continue;
        }
        let ts = v
            .get("ts")
            .and_then(|x| x.as_str())
            .and_then(parse_ts)
            .unwrap_or(0);
        records.push(Record {
            ts,
            provider: "grok",
            model: "grok".to_string(),
            input,
            output,
            cache_read: cached,
            cache_write: 0,
        });
    }
    let n = records.len();
    SourceResult {
        records,
        found: true,
        detail: format!("{n} inferences"),
    }
}

fn line_ts(line: &str) -> i64 {
    line.find("\"ts\":\"")
        .and_then(|i| {
            let rest = &line[i + 6..];
            let end = rest.find('"')?;
            parse_ts(&rest[..end])
        })
        .unwrap_or(0)
}

pub fn plan(home: &Path) -> Option<Plan> {
    let text = fs::read_to_string(home.join(".grok/logs/unified.jsonl")).ok()?;
    let mut tier: Option<(String, i64)> = None;
    let mut ratio: Option<(f64, i64)> = None;
    for line in text.lines() {
        if let Some(idx) = line.find("\"subscriptionTier\":\"") {
            let rest = &line[idx + 20..];
            if let Some(end) = rest.find('"') {
                tier = Some((rest[..end].to_string(), line_ts(line)));
            }
        }
        if let Some(idx) = line.find("tokens (actual/limit): ") {
            let rest = &line[idx + 23..];
            let tok: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '/')
                .collect();
            let mut it = tok.split('/');
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                if let (Ok(a), Ok(b)) = (a.parse::<f64>(), b.parse::<f64>()) {
                    if b > 0.0 {
                        let ts = line_ts(line);
                        if ratio.map_or(true, |(_, t)| ts >= t) {
                            ratio = Some((a / b * 100.0, ts));
                        }
                    }
                }
            }
        }
    }
    let (name, ts) = tier?;
    let mut windows = Vec::new();
    let mut note = Some("rolling 24h free token cap; no live quota recorded locally".to_string());
    if let Some((pct, rts)) = ratio {
        let now = chrono::Utc::now().timestamp();
        if now - rts <= 86400 {
            windows.push(Window {
                label: "24h".to_string(),
                used_pct: pct.clamp(0.0, 100.0),
                resets_at: Some(rts + 86400),
            });
            note = None;
        }
    }
    Some(Plan {
        provider: "grok",
        plan: name,
        captured: Some(ts),
        windows,
        details: Vec::new(),
        note,
    })
}
