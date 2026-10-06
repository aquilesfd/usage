use crate::model::{Record, Totals};
use crate::paths;
use crate::pricing;
use crate::providers::{self, Plan};
use crate::store::{self, PlanPoint, ProviderPoint, WindowPoint};
use std::collections::BTreeMap;
use std::path::Path;

/// Rolling usage windows reported for every provider.
pub const WINDOWS: &[(&str, i64)] = &[
    ("5h", 5 * 3600),
    ("weekly", 7 * 86400),
    ("monthly", 30 * 86400),
];

pub struct Summary {
    pub captured_at: i64,
    pub records: usize,
    pub providers: usize,
    pub plans: usize,
    pub db: std::path::PathBuf,
}

/// Turn provider plans into snapshot rows (one row per quota window).
pub fn plan_points(plans: &[Plan], captured_at: i64) -> Vec<PlanPoint> {
    let mut out = Vec::new();
    for p in plans {
        let details = p.details.join(" · ");
        if p.windows.is_empty() {
            out.push(PlanPoint {
                captured_at,
                provider: p.provider.to_string(),
                plan: p.plan.clone(),
                window: "plan".to_string(),
                used_pct: 0.0,
                resets_at: None,
                details,
                note: p.note.clone(),
            });
        } else {
            for w in &p.windows {
                out.push(PlanPoint {
                    captured_at,
                    provider: p.provider.to_string(),
                    plan: p.plan.clone(),
                    window: w.label.clone(),
                    used_pct: w.used_pct,
                    resets_at: w.resets_at,
                    details: details.clone(),
                    note: p.note.clone(),
                });
            }
        }
    }
    out
}

/// Sum records into cumulative per-provider snapshot rows.
pub fn provider_points(records: &[Record], captured_at: i64) -> Vec<ProviderPoint> {
    let mut map: BTreeMap<&'static str, Totals> = BTreeMap::new();
    for r in records {
        let p = pricing::lookup(&r.model).0;
        map.entry(r.provider).or_default().add(r, &p);
    }
    map.into_iter()
        .map(|(provider, t)| ProviderPoint {
            captured_at,
            provider: provider.to_string(),
            input: t.input,
            output: t.output,
            cache_read: t.cache_read,
            cache_write: t.cache_write,
            calls: t.calls,
            cost_with: t.cost_with,
            cost_without: t.cost_without,
        })
        .collect()
}

/// Sum records into rolling 5h / weekly / monthly windows per provider.
pub fn window_points(records: &[Record], now: i64) -> Vec<WindowPoint> {
    let mut providers: BTreeMap<&'static str, ()> = BTreeMap::new();
    for r in records {
        providers.insert(r.provider, ());
    }
    let mut out = Vec::new();
    for (provider, _) in providers {
        for (label, secs) in WINDOWS {
            let mut totals = Totals::default();
            for r in records
                .iter()
                .filter(|r| r.provider == provider && r.ts >= now - secs)
            {
                let p = pricing::lookup(&r.model).0;
                totals.add(r, &p);
            }
            if totals.calls == 0 {
                continue;
            }
            out.push(WindowPoint {
                provider: provider.to_string(),
                window: label.to_string(),
                tokens: totals.total_tokens(),
                cost: totals.cost_with,
                calls: totals.calls,
            });
        }
    }
    out
}

/// Collect current usage and persist one snapshot to the history database.
pub fn run_once(home: &Path) -> Result<Summary, String> {
    let collected = providers::collect_all(home);
    let captured_at = chrono::Utc::now().timestamp();
    let plans = plan_points(&collected.plans, captured_at);
    let providers = provider_points(&collected.records, captured_at);
    let windows = window_points(&collected.records, captured_at);

    let db = paths::db_path();
    let mut conn = store::open(&db).map_err(|e| format!("open {}: {e}", db.display()))?;
    store::record(
        &mut conn,
        captured_at,
        collected.records.len(),
        &plans,
        &providers,
        &windows,
    )
    .map_err(|e| format!("write snapshot: {e}"))?;

    let summary = Summary {
        captured_at,
        records: collected.records.len(),
        providers: providers.len(),
        plans: collected.plans.len(),
        db: db.clone(),
    };

    write_latest_cache(&collected.plans, &providers, captured_at);

    Ok(summary)
}

/// Best-effort JSON cache of the latest capture, for external consumers.
fn write_latest_cache(plans: &[Plan], providers: &[ProviderPoint], captured_at: i64) {
    let plans_json: Vec<serde_json::Value> = plans
        .iter()
        .map(|p| {
            serde_json::json!({
                "provider": p.provider,
                "plan": p.plan,
                "windows": p.windows.iter().map(|w| serde_json::json!({
                    "label": w.label,
                    "used_pct": w.used_pct,
                    "resets_at": w.resets_at,
                })).collect::<Vec<_>>(),
                "details": p.details,
                "note": p.note,
            })
        })
        .collect();
    let providers_json: Vec<serde_json::Value> = providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "provider": p.provider,
                "total_tokens": p.total_tokens(),
                "cost_with_cache": p.cost_with,
                "calls": p.calls,
            })
        })
        .collect();
    let out = serde_json::json!({
        "captured_at": captured_at,
        "plans": plans_json,
        "providers": providers_json,
    });
    let path = paths::latest_json_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(&out).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Record;

    #[test]
    fn provider_points_sum_by_provider() {
        let records = vec![
            Record {
                ts: 0,
                provider: "claude",
                model: "claude-sonnet-5".to_string(),
                input: 100,
                output: 10,
                cache_read: 0,
                cache_write: 0,
            },
            Record {
                ts: 0,
                provider: "claude",
                model: "claude-sonnet-5".to_string(),
                input: 200,
                output: 20,
                cache_read: 0,
                cache_write: 0,
            },
        ];
        let points = provider_points(&records, 7);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].provider, "claude");
        assert_eq!(points[0].input, 300);
        assert_eq!(points[0].output, 30);
        assert_eq!(points[0].calls, 2);
        assert_eq!(points[0].captured_at, 7);
    }

    #[test]
    fn plan_points_expand_windows() {
        let plans = vec![Plan {
            provider: "codex",
            plan: "plus".to_string(),
            captured: Some(1),
            windows: vec![
                crate::providers::Window {
                    label: "5h".to_string(),
                    used_pct: 50.0,
                    resets_at: None,
                },
                crate::providers::Window {
                    label: "weekly".to_string(),
                    used_pct: 5.0,
                    resets_at: None,
                },
            ],
            details: vec!["x".to_string()],
            note: None,
        }];
        let points = plan_points(&plans, 9);
        assert_eq!(points.len(), 2);
        assert!(points.iter().all(|p| p.provider == "codex"));
        assert!(points.iter().all(|p| p.captured_at == 9));
    }
}
