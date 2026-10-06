use std::path::PathBuf;

/// User's home directory, if `$HOME` is set.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn xdg(env: &str, fallback: PathBuf) -> PathBuf {
    match std::env::var_os(env) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => fallback,
    }
}

/// `$XDG_DATA_HOME` (defaults to `~/.local/share`).
pub fn data_dir() -> PathBuf {
    let base = home().unwrap_or_else(|| PathBuf::from("."));
    xdg("XDG_DATA_HOME", base.join(".local/share"))
}

/// `$XDG_STATE_HOME` (defaults to `~/.local/state`).
pub fn state_dir() -> PathBuf {
    let base = home().unwrap_or_else(|| PathBuf::from("."));
    xdg("XDG_STATE_HOME", base.join(".local/state"))
}

/// Directory holding the capture history database and logs.
pub fn usage_dir() -> PathBuf {
    data_dir().join("usage")
}

/// SQLite database holding captured plan + provider snapshots.
pub fn db_path() -> PathBuf {
    usage_dir().join("usage.db")
}

/// JSON cache of the most recent capture, read by the panel applet.
pub fn latest_json_path() -> PathBuf {
    usage_dir().join("latest.json")
}
