use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use std::path::Path;

/// One quota window for a provider plan at a point in time.
#[derive(Clone, Debug)]
pub struct PlanPoint {
    pub captured_at: i64,
    pub provider: String,
    pub plan: String,
    pub window: String,
    pub used_pct: f64,
    pub resets_at: Option<i64>,
    pub details: String,
    pub note: Option<String>,
}

/// Cumulative provider token usage at a point in time.
#[derive(Clone, Debug, Default)]
pub struct ProviderPoint {
    pub captured_at: i64,
    pub provider: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub calls: u64,
    pub cost_with: f64,
    pub cost_without: f64,
}

impl ProviderPoint {
    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// Usage within a rolling window (5h / weekly / monthly) for one provider.
#[derive(Clone, Debug)]
pub struct WindowPoint {
    pub provider: String,
    pub window: String,
    pub tokens: u64,
    pub cost: f64,
    pub calls: u64,
}

/// Open (creating the parent directory if needed) the history database.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(path)?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    init(&conn)?;
    Ok(conn)
}

/// Open an existing history database read-only (used by the panel applet).
pub fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(2));
    Ok(conn)
}

fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        CREATE TABLE IF NOT EXISTS captures (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at INTEGER NOT NULL,
            records     INTEGER NOT NULL,
            providers   INTEGER NOT NULL,
            plans       INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS plan_snapshots (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at INTEGER NOT NULL,
            provider    TEXT NOT NULL,
            plan        TEXT NOT NULL,
            window      TEXT NOT NULL,
            used_pct    REAL NOT NULL,
            resets_at   INTEGER,
            details     TEXT,
            note        TEXT
        );
        CREATE INDEX IF NOT EXISTS plan_snapshots_lookup
            ON plan_snapshots (provider, window, captured_at);
        CREATE TABLE IF NOT EXISTS provider_snapshots (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at   INTEGER NOT NULL,
            provider      TEXT NOT NULL,
            input         INTEGER NOT NULL,
            output        INTEGER NOT NULL,
            cache_read    INTEGER NOT NULL,
            cache_write   INTEGER NOT NULL,
            calls         INTEGER NOT NULL,
            cost_with     REAL NOT NULL,
            cost_without REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS provider_snapshots_lookup
            ON provider_snapshots (provider, captured_at);
        CREATE TABLE IF NOT EXISTS usage_windows (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at INTEGER NOT NULL,
            provider    TEXT NOT NULL,
            window      TEXT NOT NULL,
            tokens      INTEGER NOT NULL,
            cost        REAL NOT NULL,
            calls       INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS usage_windows_lookup
            ON usage_windows (provider, window, captured_at);
        ",
    )
}

/// Persist one capture run. Returns the new capture id.
pub fn record(
    conn: &mut Connection,
    captured_at: i64,
    records: usize,
    plans: &[PlanPoint],
    providers: &[ProviderPoint],
    windows: &[WindowPoint],
) -> rusqlite::Result<i64> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO captures (captured_at, records, providers, plans) VALUES (?1, ?2, ?3, ?4)",
        params![
            captured_at,
            records as i64,
            providers.len() as i64,
            plans.len() as i64
        ],
    )?;
    let id = tx.last_insert_rowid();
    {
        let mut stmt = tx.prepare(
            "INSERT INTO plan_snapshots
                (captured_at, provider, plan, window, used_pct, resets_at, details, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for p in plans {
            stmt.execute(params![
                captured_at,
                p.provider,
                p.plan,
                p.window,
                p.used_pct,
                p.resets_at,
                p.details,
                p.note,
            ])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO provider_snapshots
                (captured_at, provider, input, output, cache_read, cache_write, calls, cost_with, cost_without)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for p in providers {
            stmt.execute(params![
                captured_at,
                p.provider,
                p.input as i64,
                p.output as i64,
                p.cache_read as i64,
                p.cache_write as i64,
                p.calls as i64,
                p.cost_with,
                p.cost_without,
            ])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO usage_windows (captured_at, provider, window, tokens, cost, calls)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for w in windows {
            stmt.execute(params![
                captured_at,
                w.provider,
                w.window,
                w.tokens as i64,
                w.cost,
                w.calls as i64,
            ])?;
        }
    }
    tx.commit()?;
    Ok(id)
}

/// Rolling usage windows from the latest capture.
pub fn latest_windows(conn: &Connection) -> Vec<WindowPoint> {
    let Some(ts) = latest_capture_at(conn) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT provider, window, tokens, cost, calls FROM usage_windows
         WHERE captured_at = ?1 ORDER BY provider",
    ) else {
        return out;
    };
    let rows = stmt.query_map(params![ts], |r| {
        Ok(WindowPoint {
            provider: r.get(0)?,
            window: r.get(1)?,
            tokens: r.get::<_, i64>(2)? as u64,
            cost: r.get(3)?,
            calls: r.get::<_, i64>(4)? as u64,
        })
    });
    if let Ok(rows) = rows {
        out.extend(rows.flatten());
    }
    out
}

/// Timestamp of the most recent capture, if any.
pub fn latest_capture_at(conn: &Connection) -> Option<i64> {
    conn.query_row("SELECT max(captured_at) FROM captures", [], |r| r.get(0))
        .ok()
        .flatten()
}

/// Timestamp of the capture immediately before the latest one.
pub fn previous_capture_at(conn: &Connection) -> Option<i64> {
    let latest = latest_capture_at(conn)?;
    conn.query_row(
        "SELECT max(captured_at) FROM captures WHERE captured_at < ?1",
        params![latest],
        |r| r.get(0),
    )
    .ok()
    .flatten()
}

/// Most recent snapshot for every (provider, window) pair.
pub fn current_plans(conn: &Connection) -> Vec<PlanPoint> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT p.captured_at, p.provider, p.plan, p.window, p.used_pct, p.resets_at, p.details, p.note
         FROM plan_snapshots p
         WHERE p.captured_at = (
             SELECT max(q.captured_at) FROM plan_snapshots q
             WHERE q.provider = p.provider AND q.window = p.window
         )
         GROUP BY p.provider, p.window
         ORDER BY p.used_pct DESC",
    ) else {
        return out;
    };
    let rows = stmt.query_map([], |r| {
        Ok(PlanPoint {
            captured_at: r.get(0)?,
            provider: r.get(1)?,
            plan: r.get(2)?,
            window: r.get(3)?,
            used_pct: r.get(4)?,
            resets_at: r.get(5)?,
            details: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            note: r.get(7)?,
        })
    });
    if let Ok(rows) = rows {
        out.extend(rows.flatten());
    }
    out
}

fn providers_at(conn: &Connection, captured_at: Option<i64>) -> Vec<ProviderPoint> {
    let Some(ts) = captured_at else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT captured_at, provider, input, output, cache_read, cache_write, calls, cost_with, cost_without
         FROM provider_snapshots WHERE captured_at = ?1 ORDER BY provider",
    ) else {
        return out;
    };
    let rows = stmt.query_map(params![ts], |r| {
        Ok(ProviderPoint {
            captured_at: r.get(0)?,
            provider: r.get(1)?,
            input: r.get::<_, i64>(2)? as u64,
            output: r.get::<_, i64>(3)? as u64,
            cache_read: r.get::<_, i64>(4)? as u64,
            cache_write: r.get::<_, i64>(5)? as u64,
            calls: r.get::<_, i64>(6)? as u64,
            cost_with: r.get(7)?,
            cost_without: r.get(8)?,
        })
    });
    if let Ok(rows) = rows {
        out.extend(rows.flatten());
    }
    out
}

/// Cumulative provider totals from the latest capture.
pub fn latest_providers(conn: &Connection) -> Vec<ProviderPoint> {
    providers_at(conn, latest_capture_at(conn))
}

/// Cumulative provider totals from the capture before the latest one.
pub fn previous_providers(conn: &Connection) -> Vec<ProviderPoint> {
    providers_at(conn, previous_capture_at(conn))
}

/// Per-provider change since a timestamp: cost, calls, and total tokens.
#[derive(Clone, Copy, Debug, Default)]
pub struct Delta {
    pub cost: f64,
    pub calls: u64,
    pub tokens: u64,
}

/// Cumulative cost/calls/tokens per provider at the latest capture, minus the
/// value at the last capture before `start`, so callers can compute a period
/// delta.
pub fn usage_since(conn: &Connection, start: i64) -> BTreeMap<String, Delta> {
    let latest = latest_capture_at(conn);
    let mut out = BTreeMap::new();
    let Some(latest) = latest else {
        return out;
    };
    let now_at = providers_at(conn, Some(latest));
    for p in now_at {
        let baseline: (f64, i64, i64) = conn
            .query_row(
                "SELECT cost_with, calls, input + output + cache_read + cache_write
                 FROM provider_snapshots
                 WHERE provider = ?1 AND captured_at < ?2
                 ORDER BY captured_at DESC LIMIT 1",
                params![p.provider, start],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap_or((0.0, 0, 0));
        out.insert(
            p.provider.clone(),
            Delta {
                cost: (p.cost_with - baseline.0).max(0.0),
                calls: p.calls.saturating_sub(baseline.1 as u64),
                tokens: p.total_tokens().saturating_sub(baseline.2 as u64),
            },
        );
    }
    out
}

/// Recent (captured_at, used_pct) history for one provider window, oldest first.
pub fn plan_history(
    conn: &Connection,
    provider: &str,
    window: &str,
    limit: usize,
) -> Vec<(i64, f64)> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT captured_at, used_pct FROM plan_snapshots
         WHERE provider = ?1 AND window = ?2
         ORDER BY captured_at DESC LIMIT ?3",
    ) else {
        return out;
    };
    let rows = stmt.query_map(params![provider, window, limit as i64], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
    });
    if let Ok(rows) = rows {
        out.extend(rows.flatten());
    }
    out.reverse();
    out
}

/// Most recent plan snapshots across all providers, newest first.
pub fn recent_plan_snapshots(conn: &Connection, limit: usize) -> Vec<PlanPoint> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT captured_at, provider, plan, window, used_pct, resets_at, details, note
         FROM plan_snapshots ORDER BY captured_at DESC, provider LIMIT ?1",
    ) else {
        return out;
    };
    let rows = stmt.query_map(params![limit as i64], |r| {
        Ok(PlanPoint {
            captured_at: r.get(0)?,
            provider: r.get(1)?,
            plan: r.get(2)?,
            window: r.get(3)?,
            used_pct: r.get(4)?,
            resets_at: r.get(5)?,
            details: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            note: r.get(7)?,
        })
    });
    if let Ok(rows) = rows {
        out.extend(rows.flatten());
    }
    out
}

/// Number of stored captures.
pub fn capture_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM captures", [], |r| r.get(0))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn
    }

    fn plan(ts: i64, pct: f64) -> PlanPoint {
        PlanPoint {
            captured_at: ts,
            provider: "claude".to_string(),
            plan: "pro".to_string(),
            window: "5h".to_string(),
            used_pct: pct,
            resets_at: Some(ts + 3600),
            details: "d".to_string(),
            note: None,
        }
    }

    fn provider(ts: i64) -> ProviderPoint {
        ProviderPoint {
            captured_at: ts,
            provider: "claude".to_string(),
            input: 10,
            output: 5,
            cache_read: 1,
            cache_write: 2,
            calls: 3,
            cost_with: 1.5,
            cost_without: 2.5,
        }
    }

    #[test]
    fn records_and_reads_back() {
        let mut conn = mem();
        let windows = vec![WindowPoint {
            provider: "claude".to_string(),
            window: "5h".to_string(),
            tokens: 18,
            cost: 1.5,
            calls: 3,
        }];
        let id = record(
            &mut conn,
            100,
            4,
            &[plan(100, 42.0)],
            &[provider(100)],
            &windows,
        )
        .unwrap();
        assert_eq!(id, 1);
        assert_eq!(latest_capture_at(&conn), Some(100));
        assert_eq!(capture_count(&conn), 1);
        assert!(previous_capture_at(&conn).is_none());

        let current = current_plans(&conn);
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].used_pct, 42.0);

        let latest = latest_providers(&conn);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].total_tokens(), 18);

        let wins = latest_windows(&conn);
        assert_eq!(wins.len(), 1);
        assert_eq!(wins[0].window, "5h");
        assert_eq!(wins[0].tokens, 18);
    }

    #[test]
    fn history_is_oldest_first_and_limited() {
        let mut conn = mem();
        for (ts, pct) in [(100, 10.0), (200, 20.0), (300, 30.0)] {
            record(&mut conn, ts, 0, &[plan(ts, pct)], &[], &[]).unwrap();
        }
        assert_eq!(
            plan_history(&conn, "claude", "5h", 10),
            vec![(100, 10.0), (200, 20.0), (300, 30.0)]
        );
        assert_eq!(
            plan_history(&conn, "claude", "5h", 2),
            vec![(200, 20.0), (300, 30.0)]
        );
        assert_eq!(previous_capture_at(&conn), Some(200));
    }

    #[test]
    fn usage_since_subtracts_baseline() {
        let mut conn = mem();
        let mut first = provider(100);
        first.cost_with = 1.0;
        first.calls = 1;
        let mut second = provider(200);
        second.cost_with = 4.0;
        second.calls = 5;
        second.input = 20;
        record(&mut conn, 100, 0, &[], &[first], &[]).unwrap();
        record(&mut conn, 200, 0, &[], &[second], &[]).unwrap();

        let since = usage_since(&conn, 150);
        let delta = since.get("claude").copied().unwrap();
        assert!((delta.cost - 3.0).abs() < f64::EPSILON);
        assert_eq!(delta.calls, 4);
        // first provider has 18 tokens, second has 28.
        assert_eq!(delta.tokens, 10);
    }
}
