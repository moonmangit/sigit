//! B8: the last answers, saved to `%APPDATA%\sigit\cache.json`, so the next launch shows a
//! full window in its first frame while fresh data loads. Shown with an "updated … ago" mark;
//! nothing is ever acted on from the cache (actions re-read GitHub first).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::daily::GroupRepo;
use crate::model::{Envs, Inbox, Notif, Pr, RepoMeta, Run};
use crate::pulse::Pulse;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub saved_at: i64,
    pub me: String,
    /// (slug, pushed_at, archived)
    pub repos: Vec<(String, String, bool)>,
    pub inbox: Option<Inbox>,
    pub notifications: Vec<Notif>,
    /// Open PRs and latest runs per repo you looked at.
    pub prs: HashMap<String, Vec<Pr>>,
    pub runs: HashMap<String, Vec<Run>>,
    pub envs: HashMap<String, Envs>,
    pub meta: HashMap<String, RepoMeta>,
    /// Group dashboard rows per repo.
    pub groups: HashMap<String, GroupRepo>,
    /// The Activity page: (what it answers, see `State::pulse_key_now`), and when it loaded.
    pub pulse: Option<(String, Pulse)>,
    pub pulse_at: i64,
    /// The Comparisons tab: (what it answers, see `State::compare_key_now`), and when it loaded.
    pub compare: Option<(String, Vec<crate::compare::RepoCompare>)>,
    pub compare_at: i64,
}

/// Where the cache lives (Settings → Data shows it).
pub fn file() -> Option<PathBuf> {
    Some(crate::config::dir()?.join("cache.json"))
}

pub fn load() -> Option<Snapshot> {
    serde_json::from_str(&std::fs::read_to_string(file()?).ok()?).ok()
}

pub fn save(s: &Snapshot) -> Result<(), String> {
    let f = file().ok_or("no config folder")?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

/// Delete the cache file (what's in memory stays until you restart).
pub fn clear() -> Result<(), String> {
    match file() {
        Some(f) if f.exists() => std::fs::remove_file(f).map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}
