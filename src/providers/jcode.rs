use super::{SourceResult, parse_ts, u64v, walk_files};
use crate::model::Record;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn collect(home: &Path) -> SourceResult {
    let root = home.join(".jcode/sessions");
    let mut records = Vec::new();
    let files = walk_files(&root, "jsonl");
    for file in &files {
        let text = match fs::read_to_string(file) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || !line.contains("token_usage") {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let meta = v.get("meta");
            let model = meta
                .and_then(|m| m.get("model"))
                .and_then(|x| x.as_str())
                .unwrap_or("jcode")
                .to_string();
            let meta_ts = meta
                .and_then(|m| {
                    m.get("updated_at")
                        .or_else(|| m.get("last_active_at"))
                        .and_then(|x| x.as_str())
                })
                .and_then(parse_ts)
                .unwrap_or(0);
            let messages = match v.get("append_messages").and_then(|a| a.as_array()) {
                Some(a) => a,
                None => continue,
            };
            for msg in messages {
                let usage = match msg.get("token_usage") {
                    Some(u) => u,
                    None => continue,
                };
                let input = u64v(usage, "input_tokens");
                let output = u64v(usage, "output_tokens");
                let cache_read = u64v(usage, "cache_read_input_tokens");
                let cache_write = u64v(usage, "cache_creation_input_tokens");
                if input + output + cache_read + cache_write == 0 {
                    continue;
                }
                let ts = msg
                    .get("timestamp")
                    .and_then(|x| x.as_str())
                    .and_then(parse_ts)
                    .unwrap_or(meta_ts);
                records.push(Record {
                    ts,
                    provider: "jcode",
                    model: model.clone(),
                    input,
                    output,
                    cache_read,
                    cache_write,
                });
            }
        }
    }
    SourceResult {
        records,
        found: root.exists(),
        detail: format!("{} session journals", files.len()),
    }
}
