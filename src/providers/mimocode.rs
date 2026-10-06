use super::SourceResult;
use crate::model::Record;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

fn json_u64(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

/// mimocode records per-step token usage inside the `part` table as JSON
/// (`{"type":"step-finish","tokens":{...}}`), with the model on the parent
/// message row.
pub fn collect(home: &Path) -> SourceResult {
    let path = home.join(".local/share/mimocode/mimocode.db");
    let mut records = Vec::new();
    if !path.exists() {
        return SourceResult {
            records,
            found: false,
            detail: "not found".to_string(),
        };
    }
    let conn = match Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
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

    let sql = "select p.id, p.time_created, p.data, m.data
               from part p join message m on p.message_id = m.id
               where p.data like '%\"step-finish\"%'";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => {
            return SourceResult {
                records,
                found: true,
                detail: "schema mismatch".to_string(),
            };
        }
    };
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let ts: i64 = row.get(1).unwrap_or(0);
        let pdata: String = row.get(2)?;
        let mdata: Option<String> = row.get(3).ok();
        Ok((id, ts, pdata, mdata))
    });
    let rows = match rows {
        Ok(r) => r,
        Err(_) => {
            return SourceResult {
                records,
                found: true,
                detail: "query failed".to_string(),
            };
        }
    };

    let mut seen: HashSet<String> = HashSet::new();
    for row in rows.flatten() {
        let (id, ts, pdata, mdata) = row;
        if !seen.insert(id) {
            continue;
        }
        let part: Value = match serde_json::from_str(&pdata) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if part.get("type").and_then(|x| x.as_str()) != Some("step-finish") {
            continue;
        }
        let tokens = match part.get("tokens") {
            Some(t) => t,
            None => continue,
        };
        let input = json_u64(tokens, "input");
        let output = json_u64(tokens, "output") + json_u64(tokens, "reasoning");
        let cache_read = tokens
            .get("cache")
            .map(|c| json_u64(c, "read"))
            .unwrap_or(0);
        let cache_write = tokens
            .get("cache")
            .map(|c| json_u64(c, "write"))
            .unwrap_or(0);
        if input + output + cache_read + cache_write == 0 {
            continue;
        }
        let model = mdata
            .as_deref()
            .and_then(|d| serde_json::from_str::<Value>(d).ok())
            .and_then(|m| {
                m.get("modelID")
                    .and_then(|x| x.as_str())
                    .or_else(|| m.get("model").and_then(|x| x.as_str()))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "mimocode".to_string());
        records.push(Record {
            ts: ts / 1000,
            provider: "mimocode",
            model,
            input,
            output,
            cache_read,
            cache_write,
        });
    }
    let n = records.len();
    SourceResult {
        records,
        found: true,
        detail: format!("{n} steps"),
    }
}
