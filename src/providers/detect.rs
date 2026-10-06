use super::{Plan, Source};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

/// Providers we can see on disk but cannot read token/quota numbers from.
pub struct Detected {
    pub sources: Vec<Source>,
    pub plans: Vec<Plan>,
}

fn source(name: &'static str, path: &Path, found: bool, detail: &str) -> Source {
    Source {
        name,
        path: path.display().to_string(),
        supported: false,
        found,
        records: 0,
        detail: if found {
            detail.to_string()
        } else {
            "not found".to_string()
        },
    }
}

fn note_plan(provider: &'static str, plan: &str, note: &str, details: Vec<String>) -> Plan {
    Plan {
        provider,
        plan: plan.to_string(),
        captured: None,
        windows: Vec::new(),
        details,
        note: Some(note.to_string()),
    }
}

/// Read the Cursor plan from its VS Code global storage, if present.
fn cursor_plan(home: &Path) -> Option<Plan> {
    let db = home.join(".config/Cursor/User/globalStorage/state.vscdb");
    if !db.exists() {
        return None;
    }
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let get = |key: &str| -> Option<String> {
        conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    };
    let membership =
        get("cursorAuth/stripeMembershipType").unwrap_or_else(|| "unknown".to_string());
    let mut details = Vec::new();
    if let Some(email) = get("cursorAuth/cachedEmail") {
        details.push(format!("account: {email}"));
    }
    if let Some(signup) = get("cursorAuth/cachedSignUpType") {
        details.push(format!("signup: {signup}"));
    }
    Some(note_plan(
        "cursor",
        &membership,
        "usage is server-side; no local quota recorded",
        details,
    ))
}

pub fn collect(home: &Path) -> Detected {
    let mut sources = Vec::new();
    let mut plans = Vec::new();

    // Cursor (IDE + CLI agent).
    let cursor = home.join(".config/Cursor");
    sources.push(source(
        "cursor",
        &cursor,
        cursor.exists(),
        "no local token counts (usage is server-side)",
    ));
    if let Some(p) = cursor_plan(home) {
        plans.push(p);
    }

    // Muse Code.
    let muse = home.join(".local/share/muse");
    sources.push(source(
        "muse",
        &muse,
        muse.exists(),
        "session traces carry no token usage",
    ));
    if muse.exists() {
        plans.push(note_plan(
            "muse",
            "Muse Code",
            "no local token counts or quota",
            Vec::new(),
        ));
    }

    // Kiro CLI.
    let kiro = home.join(".local/share/kiro-cli");
    let kiro_found = kiro.exists();
    sources.push(source(
        "kiro",
        &kiro,
        kiro_found,
        "only shell history + auth state stored locally",
    ));
    if kiro_found {
        plans.push(note_plan(
            "kiro",
            "detected",
            "no local token counts",
            Vec::new(),
        ));
    }

    // Devin CLI.
    let devin = home.join(".local/share/devin");
    let devin_found = devin.exists();
    sources.push(source(
        "devin",
        &devin,
        devin_found,
        "session store has no token counts",
    ));
    if devin_found {
        plans.push(note_plan(
            "devin",
            "detected",
            "no local token counts",
            Vec::new(),
        ));
    }

    // Antigravity.
    let antigravity = home.join(".config/Antigravity");
    let antigravity_found = antigravity.exists();
    sources.push(source(
        "antigravity",
        &antigravity,
        antigravity_found,
        "local store is an opaque protobuf blob",
    ));
    if antigravity_found {
        plans.push(note_plan(
            "antigravity",
            "detected",
            "no local token counts",
            Vec::new(),
        ));
    }

    // Cline.
    let cline = home.join(".cline");
    let cline_found = cline.exists();
    sources.push(source(
        "cline",
        &cline,
        cline_found,
        "no per-message token counts in local data",
    ));
    if cline_found {
        plans.push(note_plan(
            "cline",
            "detected",
            "no local token counts",
            Vec::new(),
        ));
    }

    // Kilo is collected like opencode; its local rows are usually empty, so
    // surface it as a detected plan as well.
    if home.join(".local/share/kilo/kilo.db").exists() {
        plans.push(note_plan(
            "kilo",
            "detected",
            "local session rows carry no token counts",
            Vec::new(),
        ));
    }

    Detected { sources, plans }
}
