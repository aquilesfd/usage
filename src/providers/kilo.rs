use super::SourceResult;
use super::sqlite_sessions;
use std::path::Path;

/// Kilo stores sessions with the same schema as opencode. Its local rows are
/// currently zero-filled, but the collector is wired so populated installs
/// are picked up automatically.
pub fn collect(home: &Path) -> SourceResult {
    let path = home.join(".local/share/kilo/kilo.db");
    sqlite_sessions::collect(&path, "kilo", &["session_v2", "session"])
}
