use super::SourceResult;
use crate::model::Record;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

fn model_id(raw: Option<String>) -> String {
    match raw {
        Some(s) => serde_json::from_str::<Value>(&s)
            .ok()
            .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
        None => "unknown".to_string(),
    }
}

/// Collect token usage from an opencode-compatible SQLite store.
///
/// Both opencode and kilo use a `session`/`session_v2` table carrying
/// `tokens_input`, `tokens_output`, `tokens_reasoning`, `tokens_cache_read`,
/// `tokens_cache_write` and `time_created`.
pub fn collect(path: &Path, provider: &'static str, tables: &[&str]) -> SourceResult {
    let mut records = Vec::new();
    if !path.exists() {
        return SourceResult {
            records,
            found: false,
            detail: "not found".to_string(),
        };
    }
    let conn = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(_) => {
            return SourceResult {
                records,
                found: true,
                detail: "unreadable".to_string(),
            };
        }
    };
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    let mut map: HashMap<String, Record> = HashMap::new();
    let sql = "select id, model, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write, time_created from ";
    for table in tables {
        let query = format!("{sql}{table}");
        let mut stmt = match conn.prepare(&query) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let model: Option<String> = row.get(1).ok();
            let input: i64 = row.get(2).unwrap_or(0);
            let output: i64 = row.get(3).unwrap_or(0);
            let reasoning: i64 = row.get(4).unwrap_or(0);
            let cache_read: i64 = row.get(5).unwrap_or(0);
            let cache_write: i64 = row.get(6).unwrap_or(0);
            let created: i64 = row.get(7).unwrap_or(0);
            Ok((
                id,
                model,
                input,
                output,
                reasoning,
                cache_read,
                cache_write,
                created,
            ))
        });
        let rows = match rows {
            Ok(r) => r,
            Err(_) => continue,
        };
        for row in rows.flatten() {
            let (id, model, input, output, reasoning, cache_read, cache_write, created) = row;
            if map.contains_key(&id) {
                continue;
            }
            let input = input.max(0) as u64;
            let output = (output.max(0) + reasoning.max(0)) as u64;
            let cache_read = cache_read.max(0) as u64;
            let cache_write = cache_write.max(0) as u64;
            if input + output + cache_read + cache_write == 0 {
                continue;
            }
            map.insert(
                id,
                Record {
                    ts: created / 1000,
                    provider,
                    model: model_id(model),
                    input,
                    output,
                    cache_read,
                    cache_write,
                },
            );
        }
    }
    let n = map.len();
    records.extend(map.into_values());
    SourceResult {
        records,
        found: true,
        detail: format!("{n} sessions"),
    }
}
