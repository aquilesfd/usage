use super::{SourceResult, parse_ts, u64v, walk_files};
use crate::model::Record;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn collect(home: &Path) -> SourceResult {
    let root = home.join(".zcode/cli/rollout");
    let mut records = Vec::new();
    let files = walk_files(&root, "jsonl");
    for file in &files {
        let text = match fs::read_to_string(file) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || !line.contains("\"response\"") {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let response = match v.get("response") {
                Some(r) => r,
                None => continue,
            };
            let (input, output, cache_read, cache_write) = if let Some(u) = response.get("usage") {
                (
                    u64v(u, "inputTokens"),
                    u64v(u, "outputTokens"),
                    u64v(u, "cacheReadTokens"),
                    u64v(u, "cacheWriteTokens"),
                )
            } else if let Some(u) = response
                .get("providerMetadata")
                .and_then(|p| p.get("anthropic"))
                .and_then(|a| a.get("usage"))
            {
                (
                    u64v(u, "input_tokens"),
                    u64v(u, "output_tokens"),
                    u64v(u, "cache_read_input_tokens"),
                    u64v(u, "cache_creation_input_tokens"),
                )
            } else {
                continue;
            };
            if input + output + cache_read + cache_write == 0 {
                continue;
            }
            let ts = v
                .get("completedAt")
                .and_then(|x| x.as_str())
                .and_then(parse_ts)
                .unwrap_or(0);
            let model = v
                .get("model")
                .and_then(|m| m.get("modelId"))
                .and_then(|x| x.as_str())
                .unwrap_or("zcode")
                .to_string();
            records.push(Record {
                ts,
                provider: "zcode",
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
        detail: format!("{} rollout files", files.len()),
    }
}
