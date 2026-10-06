pub mod claude;
pub mod codex;
pub mod detect;
pub mod grok;
pub mod jcode;
pub mod kilo;
pub mod mimocode;
pub mod opencode;
pub mod sqlite_sessions;
pub mod zcode;

use crate::model::Record;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub struct SourceResult {
    pub records: Vec<Record>,
    pub found: bool,
    pub detail: String,
}

pub struct Source {
    pub name: &'static str,
    pub path: String,
    pub supported: bool,
    pub found: bool,
    pub records: usize,
    pub detail: String,
}

pub struct Window {
    pub label: String,
    pub used_pct: f64,
    pub resets_at: Option<i64>,
}

pub struct Plan {
    pub provider: &'static str,
    pub plan: String,
    pub captured: Option<i64>,
    pub windows: Vec<Window>,
    pub details: Vec<String>,
    pub note: Option<String>,
}

pub struct Collected {
    pub records: Vec<Record>,
    pub sources: Vec<Source>,
    pub plans: Vec<Plan>,
}

pub fn collect_all(home: &Path) -> Collected {
    let mut records = Vec::new();
    let mut sources = Vec::new();
    let mut plans = Vec::new();

    let r = claude::collect(home);
    push(
        &mut records,
        &mut sources,
        "claude",
        home.join(".claude/projects"),
        true,
        r,
    );

    let (r, limits) = codex::collect(home);
    push(
        &mut records,
        &mut sources,
        "codex",
        home.join(".codex/sessions"),
        true,
        r,
    );
    if let Some(p) = codex::fresh_plan(home) {
        plans.push(p);
    } else if let Some(l) = &limits {
        plans.push(codex::plan_from_limits(l));
    }

    let r = opencode::collect(home);
    push(
        &mut records,
        &mut sources,
        "opencode",
        home.join(".local/share/opencode/opencode.db"),
        true,
        r,
    );

    let r = kilo::collect(home);
    push(
        &mut records,
        &mut sources,
        "kilo",
        home.join(".local/share/kilo/kilo.db"),
        true,
        r,
    );

    let r = mimocode::collect(home);
    push(
        &mut records,
        &mut sources,
        "mimocode",
        home.join(".local/share/mimocode/mimocode.db"),
        true,
        r,
    );

    let r = grok::collect(home);
    push(
        &mut records,
        &mut sources,
        "grok",
        home.join(".grok/logs/unified.jsonl"),
        true,
        r,
    );

    let r = jcode::collect(home);
    push(
        &mut records,
        &mut sources,
        "jcode",
        home.join(".jcode/sessions"),
        true,
        r,
    );

    let r = zcode::collect(home);
    push(
        &mut records,
        &mut sources,
        "zcode",
        home.join(".zcode/cli/rollout"),
        true,
        r,
    );

    if let Some(p) = claude::plan(home) {
        plans.push(p);
    }
    if let Some(p) = grok::plan(home) {
        plans.push(p);
    }
    if let Some(p) = opencode::plan(home) {
        plans.push(p);
    }

    let detected = detect::collect(home);
    sources.extend(detected.sources);
    plans.extend(detected.plans);

    Collected {
        records,
        sources,
        plans,
    }
}

fn push(
    records: &mut Vec<Record>,
    sources: &mut Vec<Source>,
    name: &'static str,
    path: PathBuf,
    supported: bool,
    r: SourceResult,
) {
    let count = r.records.len();
    records.extend(r.records);
    sources.push(Source {
        name,
        path: path.display().to_string(),
        supported,
        found: r.found,
        records: count,
        detail: r.detail,
    });
}

pub fn walk_files(root: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some(ext) {
                out.push(p);
            }
        }
    }
    out
}

pub fn u64v(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(x) => x
            .as_u64()
            .unwrap_or_else(|| x.as_f64().unwrap_or(0.0) as u64),
        None => 0,
    }
}

pub fn parse_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp())
}
