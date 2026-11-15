//! A6: every write action the app runs, appended to `%APPDATA%\sigit\activity.jsonl`
//! (one JSON object per line, rotated to `activity.1.jsonl` at 5 MB). Local only.

use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Entry {
    /// Unix seconds.
    pub at: i64,
    pub repo: String,
    /// What the app did, as the toolbar named it ("Merge #943", "Checkout dev").
    pub action: String,
    pub ok: bool,
    /// gh/git's last line of output, or the error.
    pub detail: String,
    /// The gh / git commands it ran, as you'd type them (older entries have none).
    #[serde(default)]
    pub commands: Vec<String>,
}

fn file() -> Option<PathBuf> {
    Some(crate::config::dir()?.join("activity.jsonl"))
}

pub fn append(e: &Entry) -> Result<(), String> {
    let f = file().ok_or("no config folder")?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if std::fs::metadata(&f).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
        let _ = std::fs::rename(&f, f.with_file_name("activity.1.jsonl"));
    }
    let line = serde_json::to_string(e).map_err(|e| e.to_string())?;
    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(&f).map_err(|e| e.to_string())?;
    writeln!(out, "{line}").map_err(|e| e.to_string())
}

/// The last 30 days, newest first.
pub fn recent(now: i64) -> Vec<Entry> {
    let Some(text) = file().and_then(|f| std::fs::read_to_string(f).ok()) else { return Vec::new() };
    let mut v: Vec<Entry> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).filter(|e: &Entry| now - e.at < 30 * 86_400).collect();
    v.reverse();
    v
}
