use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::exit;

use usage::providers;
use usage::{Opts, capture, paths, render, store};

fn print_help() {
    println!("usage - local AI usage dashboard by aquilesfd");
    println!();
    println!("USAGE:");
    println!("  usage [options]                 show the report");
    println!("  usage capture [--json]          snapshot usage into the history db");
    println!("  usage history [options]         show captured plan snapshots");
    println!("  usage sources                   list detected data sources and exit");
    println!();
    println!("OPTIONS:");
    println!("  -p, --provider <list>   comma-separated providers to include");
    println!("  -d, --days <n>          days in the daily chart (default 14)");
    println!("  -t, --top <n>           top models shown per provider (default 5)");
    println!("      --period <p>        provider breakdown period: today|week|month|all");
    println!("      --json              emit JSON instead of tables");
    println!("      --sources           list detected data sources and exit");
    println!("      --no-color          disable ANSI color");
    println!("  -h, --help              show this help");
    println!();
    println!("PROVIDERS:");
    println!("  claude, codex, opencode, mimocode, grok, jcode, zcode");
    println!("  detected only: cursor, muse, kilo, kiro, devin, antigravity, cline");
}

fn parse_opts(args: &[String]) -> Opts {
    let mut opts = Opts {
        providers: None,
        days: 14,
        top: 5,
        period: "all".to_string(),
        json: false,
        sources: false,
        color: std::io::stdout().is_terminal(),
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                exit(0);
            }
            "--json" => opts.json = true,
            "--sources" => opts.sources = true,
            "--no-color" => opts.color = false,
            "-p" | "--provider" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    let list: Vec<String> = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !list.is_empty() {
                        opts.providers = Some(list);
                    }
                }
            }
            "-d" | "--days" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    opts.days = v.parse().unwrap_or(14).max(1);
                }
            }
            "-t" | "--top" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    opts.top = v.parse().unwrap_or(5).max(1);
                }
            }
            "--period" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    opts.period = v.clone();
                }
            }
            other => {
                eprintln!("unknown option: {other}");
                eprintln!("try `usage --help`");
                exit(2);
            }
        }
        i += 1;
    }

    if opts.json {
        opts.color = false;
    }
    opts
}

fn home() -> PathBuf {
    match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => {
            eprintln!("HOME is not set");
            exit(1);
        }
    }
}

fn cmd_capture(json: bool) -> ! {
    match capture::run_once(&home()) {
        Ok(s) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "captured_at": s.captured_at,
                        "records": s.records,
                        "providers": s.providers,
                        "plans": s.plans,
                        "db": s.db.display().to_string(),
                    })
                );
            } else {
                println!(
                    "captured {} records, {} provider snapshots, {} plan rows at {} -> {}",
                    s.records,
                    s.providers,
                    s.plans,
                    chrono::DateTime::from_timestamp(s.captured_at, 0)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| s.captured_at.to_string()),
                    s.db.display()
                );
            }
            exit(0);
        }
        Err(e) => {
            eprintln!("capture failed: {e}");
            exit(1);
        }
    }
}

fn cmd_history(provider: Option<String>, window: Option<String>, limit: usize) -> ! {
    let db = paths::db_path();
    let conn = match store::open(&db) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot open {}: {e}", db.display());
            exit(1);
        }
    };
    let rows = store::recent_plan_snapshots(&conn, limit);
    println!(
        "captured {} snapshots in {}",
        store::capture_count(&conn),
        db.display()
    );
    println!(
        "  {:<19} {:<12} {:<16} {:<8} {:>7}",
        "CAPTURED", "PROVIDER", "PLAN", "WINDOW", "USED"
    );
    let mut shown = 0usize;
    for r in rows {
        if provider.as_ref().is_some_and(|p| p != &r.provider) {
            continue;
        }
        if window.as_ref().is_some_and(|w| w != &r.window) {
            continue;
        }
        let ts = chrono::DateTime::from_timestamp(r.captured_at, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| r.captured_at.to_string());
        println!(
            "  {:<19} {:<12} {:<16} {:<8} {:>6.1}%",
            ts, r.provider, r.plan, r.window, r.used_pct
        );
        shown += 1;
    }
    if shown == 0 {
        println!("  (no matching snapshots)");
    }
    exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Subcommands share a small option set with the default report.
    if let Some(cmd) = args.get(1).map(String::as_str) {
        let rest: Vec<String> = args[2..].to_vec();
        match cmd {
            "capture" => {
                let json = rest.iter().any(|a| a == "--json");
                cmd_capture(json);
            }
            "history" => {
                let mut provider = None;
                let mut window = None;
                let mut limit = 50usize;
                let mut i = 0;
                while i < rest.len() {
                    match rest[i].as_str() {
                        "-p" | "--provider" => {
                            i += 1;
                            provider = rest.get(i).cloned();
                        }
                        "-w" | "--window" => {
                            i += 1;
                            window = rest.get(i).cloned();
                        }
                        "-n" | "--limit" => {
                            i += 1;
                            limit = rest.get(i).and_then(|v| v.parse().ok()).unwrap_or(50);
                        }
                        "-h" | "--help" => {
                            print_help();
                            exit(0);
                        }
                        _ => {}
                    }
                    i += 1;
                }
                cmd_history(provider, window, limit);
            }
            _ => {}
        }
    }

    let opts = parse_opts(&args[1..]);
    let home = home();
    let collected = providers::collect_all(&home);

    if opts.sources {
        render::sources(&collected.sources, &opts);
        return;
    }

    render::report(
        &collected.records,
        &collected.plans,
        &collected.sources,
        &opts,
    );
}
