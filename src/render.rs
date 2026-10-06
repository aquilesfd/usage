use crate::Opts;
use crate::model::{Record, Totals};
use crate::pricing;
use crate::providers::{Plan, Source};
use chrono::{Duration, Local, NaiveDate, TimeZone, Timelike};
use std::collections::BTreeMap;

struct Style {
    on: bool,
}

impl Style {
    fn c(&self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

fn human(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn money(x: f64) -> String {
    if x >= 1000.0 {
        format!("${x:.0}")
    } else {
        format!("${x:.2}")
    }
}

fn bar(frac: f64, width: usize) -> String {
    let blocks = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let total = (frac.clamp(0.0, 1.0) * width as f64 * 8.0).round() as usize;
    let full = total / 8;
    let rem = total % 8;
    let mut s = String::new();
    for _ in 0..full {
        s.push('█');
    }
    if full < width && rem > 0 {
        s.push(blocks[rem]);
    }
    s
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn today_start() -> i64 {
    let now = Local::now();
    now.timestamp() - now.time().num_seconds_from_midnight() as i64
}

fn period_start(period: &str, today: i64) -> Option<i64> {
    match period {
        "today" => Some(today),
        "week" | "7d" => Some(today - 6 * 86400),
        "month" | "30d" => Some(today - 29 * 86400),
        _ => None,
    }
}

fn totals(records: &[&Record], start: Option<i64>) -> Totals {
    let mut t = Totals::default();
    for r in records {
        if let Some(s) = start {
            if r.ts < s {
                continue;
            }
        }
        let p = pricing::lookup(&r.model).0;
        t.add(r, &p);
    }
    t
}

fn daily(records: &[&Record], days: i64) -> Vec<(NaiveDate, f64, f64)> {
    let today = Local::now().date_naive();
    let mut map: BTreeMap<NaiveDate, (f64, f64)> = BTreeMap::new();
    for i in (0..days).rev() {
        map.insert(today - Duration::days(i), (0.0, 0.0));
    }
    for r in records {
        let d = Local
            .timestamp_opt(r.ts, 0)
            .single()
            .map(|x| x.date_naive());
        if let Some(d) = d {
            if let Some(e) = map.get_mut(&d) {
                let p = pricing::lookup(&r.model).0;
                let (w, wo) = pricing::cost(r, &p);
                e.0 += w;
                e.1 += wo;
            }
        }
    }
    map.into_iter().map(|(d, (w, wo))| (d, w, wo)).collect()
}

fn totals_json(t: &Totals) -> serde_json::Value {
    serde_json::json!({
        "input": t.input,
        "output": t.output,
        "cache_read": t.cache_read,
        "cache_write": t.cache_write,
        "total_tokens": t.total_tokens(),
        "cost_with_cache": t.cost_with,
        "cost_without_cache": t.cost_without,
        "cache_saved": t.saved(),
        "calls": t.calls,
    })
}

fn filter<'a>(records: &'a [Record], opts: &Opts) -> Vec<&'a Record> {
    records
        .iter()
        .filter(|r| {
            opts.providers
                .as_ref()
                .map_or(true, |p| p.iter().any(|x| x == r.provider))
        })
        .collect()
}

fn age(now: i64, ts: i64) -> String {
    let d = now - ts;
    if d <= 0 {
        "just now".to_string()
    } else if d >= 86400 {
        format!("{}d ago", d / 86400)
    } else if d >= 3600 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}m ago", (d / 60).max(1))
    }
}

fn reset(now: i64, ts: i64) -> String {
    let d = ts - now;
    if d <= 0 {
        "expired".to_string()
    } else if d >= 86400 {
        format!("in {}d {}h", d / 86400, (d % 86400) / 3600)
    } else {
        format!("in {}h {}m", d / 3600, (d % 3600) / 60)
    }
}

pub fn sources(sources: &[Source], opts: &Opts) {
    let style = Style { on: opts.color };
    println!("{}", style.c("1", "usage sources"));
    println!(
        "  {:<12} {:<4} {:>7}  {:<40} {}",
        "NAME", "ST", "RECORDS", "PATH", "DETAIL"
    );
    for s in sources {
        let status = if s.supported && s.found && s.records > 0 {
            style.c("32", "ok")
        } else if s.found {
            style.c("33", "n/a")
        } else {
            style.c("90", "--")
        };
        println!(
            "  {:<12} {:<4} {:>7}  {:<40} {}",
            s.name, status, s.records, s.path, s.detail
        );
    }
}

pub fn report(records: &[Record], plans: &[Plan], sources: &[Source], opts: &Opts) {
    let style = Style { on: opts.color };
    let filtered = filter(records, opts);
    let today = today_start();
    let now = chrono::Utc::now().timestamp();

    let rows = [
        ("Today", Some(today)),
        ("7 days", Some(today - 6 * 86400)),
        ("30 days", Some(today - 29 * 86400)),
        ("All time", None),
    ];
    let period_totals: Vec<(&str, Totals)> = rows
        .iter()
        .map(|(label, start)| (*label, totals(&filtered, *start)))
        .collect();

    let pstart = period_start(&opts.period, today);
    let period_label = if pstart.is_none() {
        "all time".to_string()
    } else {
        opts.period.clone()
    };

    let mut by_provider: BTreeMap<&str, Totals> = BTreeMap::new();
    let mut by_model: BTreeMap<&str, BTreeMap<String, Totals>> = BTreeMap::new();
    for r in &filtered {
        if let Some(s) = pstart {
            if r.ts < s {
                continue;
            }
        }
        let p = pricing::lookup(&r.model).0;
        by_provider.entry(r.provider).or_default().add(r, &p);
        by_model
            .entry(r.provider)
            .or_default()
            .entry(r.model.clone())
            .or_default()
            .add(r, &p);
    }

    let chart = daily(&filtered, opts.days.max(1));
    let plan_list: Vec<&Plan> = plans
        .iter()
        .filter(|p| {
            opts.providers
                .as_ref()
                .map_or(true, |f| f.iter().any(|x| x == p.provider))
        })
        .collect();

    if opts.json {
        let periods: serde_json::Map<String, serde_json::Value> = period_totals
            .iter()
            .map(|(l, t)| (l.to_string(), totals_json(t)))
            .collect();
        let providers: serde_json::Map<String, serde_json::Value> = by_provider
            .iter()
            .map(|(k, t)| (k.to_string(), totals_json(t)))
            .collect();
        let models: serde_json::Map<String, serde_json::Value> = by_model
            .iter()
            .map(|(k, m)| {
                let mut v: Vec<(&String, &Totals)> = m.iter().collect();
                v.sort_by(|a, b| {
                    b.1.cost_with
                        .partial_cmp(&a.1.cost_with)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let arr: Vec<serde_json::Value> = v
                    .into_iter()
                    .take(opts.top)
                    .map(|(name, t)| {
                        let mut val = totals_json(t);
                        val["model"] = serde_json::Value::String(name.clone());
                        val
                    })
                    .collect();
                (k.to_string(), serde_json::Value::Array(arr))
            })
            .collect();
        let daily_json: Vec<serde_json::Value> = chart
            .iter()
            .map(|(d, w, wo)| {
                serde_json::json!({
                    "date": d.to_string(),
                    "cost_with_cache": w,
                    "cost_without_cache": wo,
                })
            })
            .collect();
        let plans_json: Vec<serde_json::Value> = plan_list
            .iter()
            .map(|p| {
                let windows: Vec<serde_json::Value> = p
                    .windows
                    .iter()
                    .map(|w| {
                        serde_json::json!({
                            "label": w.label,
                            "used_pct": w.used_pct,
                            "left_pct": (100.0 - w.used_pct).max(0.0),
                            "resets_at": w.resets_at,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "provider": p.provider,
                    "plan": p.plan,
                    "captured_at": p.captured,
                    "windows": windows,
                    "details": p.details,
                    "note": p.note,
                })
            })
            .collect();
        let out = serde_json::json!({
            "generated_at": Local::now().to_rfc3339(),
            "periods": periods,
            "providers": providers,
            "top_models": models,
            "daily": daily_json,
            "plans": plans_json,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return;
    }

    println!(
        "{} {}",
        style.c("1", "usage"),
        style.c("90", "by aquilesfd")
    );
    let active: Vec<&str> = by_provider.keys().copied().collect();
    println!(
        "{}",
        style.c(
            "90",
            &format!(
                "{} records across {} providers{}",
                filtered.len(),
                active.len(),
                if active.is_empty() {
                    String::new()
                } else {
                    format!(": {}", active.join(", "))
                }
            )
        )
    );
    println!();

    println!("{}", style.c("1", "BY TIME PERIOD"));
    println!(
        "  {:<10} {:>10} {:>10} {:>10} {:>10} {:>12} {:>12} {:>12}",
        "", "INPUT", "OUTPUT", "CACHE-R", "CACHE-W", "WITH CACHE", "NO CACHE", "CACHE SAVED"
    );
    for (label, t) in &period_totals {
        println!(
            "  {:<10} {:>10} {:>10} {:>10} {:>10} {:>12} {:>12} {:>12}",
            label,
            human(t.input),
            human(t.output),
            human(t.cache_read),
            human(t.cache_write),
            style.c("36", &money(t.cost_with)),
            money(t.cost_without),
            style.c("32", &money(t.saved())),
        );
    }
    println!();

    println!("{}", style.c("1", &format!("BY PROVIDER ({period_label})")));
    println!(
        "  {:<12} {:>8} {:>10} {:>10} {:>10} {:>10} {:>12} {:>12}",
        "PROVIDER", "CALLS", "INPUT", "OUTPUT", "CACHE-R", "CACHE-W", "WITH CACHE", "NO CACHE"
    );
    let mut grand = Totals::default();
    for (name, t) in &by_provider {
        grand.merge(t);
        println!(
            "  {:<12} {:>8} {:>10} {:>10} {:>10} {:>10} {:>12} {:>12}",
            name,
            t.calls,
            human(t.input),
            human(t.output),
            human(t.cache_read),
            human(t.cache_write),
            style.c("36", &money(t.cost_with)),
            money(t.cost_without),
        );
    }
    if by_provider.len() > 1 {
        println!(
            "  {:<12} {:>8} {:>10} {:>10} {:>10} {:>10} {:>12} {:>12}",
            style.c("1", "TOTAL"),
            grand.calls,
            human(grand.input),
            human(grand.output),
            human(grand.cache_read),
            human(grand.cache_write),
            style.c("36", &money(grand.cost_with)),
            money(grand.cost_without),
        );
    }
    println!();

    println!(
        "{}",
        style.c(
            "1",
            &format!(
                "TOP MODELS BY PROVIDER ({period_label}, top {} each)",
                opts.top
            )
        )
    );
    for (provider, models) in &by_model {
        let mut v: Vec<(&String, &Totals)> = models.iter().collect();
        v.sort_by(|a, b| {
            b.1.cost_with
                .partial_cmp(&a.1.cost_with)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        println!("  {}", style.c("35", provider));
        for (model, t) in v.into_iter().take(opts.top) {
            println!(
                "    {:<26} {:>7} {:>9} {:>9} {:>9} {:>11} {:>11}",
                truncate(model, 26),
                t.calls,
                human(t.input),
                human(t.output),
                human(t.cache_read),
                style.c("36", &money(t.cost_with)),
                money(t.cost_without),
            );
        }
    }
    println!();

    let max = chart.iter().map(|(_, w, _)| *w).fold(0.0f64, f64::max);
    println!(
        "{}",
        style.c(
            "1",
            &format!("DAILY COST (with cache, last {} days)", opts.days)
        )
    );
    for (date, with, _) in &chart {
        let frac = if max > 0.0 { with / max } else { 0.0 };
        println!(
            "  {}  {:<24} {:>9}",
            date.format("%Y-%m-%d"),
            style.c("36", &bar(frac, 24)),
            money(*with)
        );
    }

    if !plan_list.is_empty() {
        println!();
        println!("{}", style.c("1", "PLANS & LIMITS"));
        for p in &plan_list {
            let cap = p
                .captured
                .map(|ts| {
                    format!(
                        "  {}",
                        style.c("90", &format!("(captured {})", age(now, ts)))
                    )
                })
                .unwrap_or_default();
            println!("  {:<9} {}{}", p.provider, style.c("35", &p.plan), cap);
            for w in &p.windows {
                let used = w.used_pct.clamp(0.0, 100.0);
                let left = (100.0 - used).max(0.0);
                let color = if used >= 85.0 {
                    "31"
                } else if used >= 60.0 {
                    "33"
                } else {
                    "32"
                };
                let r = w
                    .resets_at
                    .map(|ts| format!("  {}", reset(now, ts)))
                    .unwrap_or_default();
                println!(
                    "    {:<8} {} {:>5.1}% used  {:>5.1}% left{}",
                    w.label,
                    style.c(color, &format!("{:<20}", bar(used / 100.0, 20))),
                    used,
                    left,
                    r
                );
            }
            for d in &p.details {
                println!("    {}", style.c("90", d));
            }
            if let Some(n) = &p.note {
                println!("    {}", style.c("90", &format!("note: {n}")));
            }
        }
    }

    let empty: Vec<&str> = sources
        .iter()
        .filter(|s| s.supported && s.found && s.records == 0)
        .map(|s| s.name)
        .collect();
    let unsupported: Vec<&str> = sources
        .iter()
        .filter(|s| !s.supported && s.found)
        .map(|s| s.name)
        .collect();
    if !empty.is_empty() {
        println!();
        println!(
            "{}",
            style.c(
                "90",
                &format!(
                    "note: {} found locally but recorded no token usage",
                    empty.join(", ")
                )
            )
        );
    }
    if !unsupported.is_empty() {
        println!(
            "{}",
            style.c(
                "90",
                &format!(
                    "note: no local token counts for {} (run `usage --sources`)",
                    unsupported.join(", ")
                )
            )
        );
    }
}
