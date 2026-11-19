//! What the app remembers, in `%APPDATA%\sigit\` (`~/.config/sigit/` elsewhere):
//! - `repos.json`: the local clones you've added (slug + path).
//! - `prefs.json`: theme, pinned repos, and your own repo groups.
//!
//! No tokens and no GitHub data: just names and paths.

use std::path::PathBuf;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Saved {
    pub slug: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];
    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }
}

/// The interface language. System follows the computer's preferred languages.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Language {
    #[default]
    System,
    English,
    Thai,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::System, Language::English, Language::Thai];
    /// Each language names itself, so you can find yours whatever the app is showing.
    pub fn label(self) -> &'static str {
        match self {
            Language::System => "System",
            Language::English => "English",
            Language::Thai => "ไทย",
        }
    }
}

/// A group you made: any repos, from any owners.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Group {
    pub name: String,
    pub repos: Vec<String>,
}

/// How an owner (you or an organization) looks in the app: a base colour and an icon.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OwnerStyle {
    pub owner: String,
    /// `#rrggbb`, or empty for the default look.
    pub color: String,
    /// One emoji or up to two letters, or empty for the owner's initial.
    pub icon: String,
}

/// Every field defaults, so an older or hand-edited file still loads.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Prefs {
    pub theme: Theme,
    pub language: Language,
    pub pinned: Vec<String>,
    pub groups: Vec<Group>,
    /// Environment names that mean production (A4), besides protected environments.
    pub prod_names: Vec<String>,
    /// Your own sensitive path patterns (A3), on top of the built-in ones.
    pub sensitive: Vec<String>,
    /// Seconds a merge waits before it runs, so it can still be cancelled (A6). 0 = off.
    pub grace_secs: u32,
    /// OS notifications for watched runs and PRs (B6).
    pub desktop_notify: bool,
    /// Watch CI on branches you check out in the app (B6).
    pub auto_watch: bool,
    /// Repos you opened most recently, newest first (B3 ranks them first).
    pub recent: Vec<String>,
    /// Notifications you snoozed: thread id → until (Unix seconds) (B1).
    pub snoozed: Vec<(String, i64)>,
    /// The guide opens by itself until you've seen it once.
    pub seen_guide: bool,
    /// The first-run welcome was shown (or skipped).
    pub seen_onboarding: bool,
    /// Sidebar sections you folded ("pinned", "g:<group>", "o:<owner>").
    pub collapsed: Vec<String>,
    /// Your order for the owner sections; owners not listed follow, you first then A–Z.
    pub owner_order: Vec<String>,
    /// Your colours and icons for owners.
    pub owner_styles: Vec<OwnerStyle>,
    /// Repos the Activity page follows. Empty: your pinned repos plus your groups' repos.
    pub tracked: Vec<String>,
    /// Side-by-side sections you resized: layout id → each section's share of the width.
    pub splits: std::collections::HashMap<String, Vec<f32>>,
    /// Text and controls size (1.0 = 100%).
    pub zoom: f32,
    /// Repos shown per organization in the sidebar before "Show more" (0: all).
    pub org_limit: usize,
    /// Show the gh / git command behind each action: in its tooltip, before it runs, and in
    /// the result and the activity log.
    pub show_commands: bool,
    /// Branch comparisons shown on every repo's Dashboard.
    pub comparisons: Vec<crate::compare::Comparison>,
    /// Per-repo changes to the shared comparisons.
    pub comparison_overrides: Vec<crate::compare::Override>,
    /// Folders whose git clones the Dashboard scan looks through.
    pub clone_folders: Vec<PathBuf>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: Theme::System,
            language: Language::System,
            pinned: Vec::new(),
            groups: Vec::new(),
            prod_names: ["prod", "production", "live", "release"].map(String::from).to_vec(),
            sensitive: Vec::new(),
            grace_secs: 5,
            desktop_notify: true,
            auto_watch: true,
            recent: Vec::new(),
            snoozed: Vec::new(),
            seen_guide: false,
            seen_onboarding: false,
            splits: Default::default(),
            zoom: 1.0,
            org_limit: 7,
            show_commands: true,
            collapsed: Vec::new(),
            owner_order: Vec::new(),
            owner_styles: Vec::new(),
            tracked: Vec::new(),
            comparisons: crate::compare::defaults(),
            comparison_overrides: Vec::new(),
            clone_folders: Vec::new(),
        }
    }
}

/// The app's folder: `%APPDATA%\sigit` on Windows, `$XDG_CONFIG_HOME/sigit` or
/// `~/.config/sigit` elsewhere. Everything the app saves lives here.
pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("sigit"))
}

/// The app was called Rune Git (folder `rune-git`) before it became sigit. On the first launch
/// after the rename, copy that folder's files over so pins, groups and settings carry on. The
/// old folder is left as it was.
pub fn migrate_old_folder() {
    let Some(new) = dir() else { return };
    let Some(old) = new.parent().map(|p| p.join("rune-git")) else { return };
    if new.exists() || !old.is_dir() {
        return;
    }
    if std::fs::create_dir_all(&new).is_err() {
        return;
    }
    for entry in std::fs::read_dir(&old).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name()
        {
            let _ = std::fs::copy(&path, new.join(name));
        }
    }
}

fn read<T: DeserializeOwned + Default>(name: &str) -> T {
    dir().and_then(|d| std::fs::read_to_string(d.join(name)).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write<T: Serialize>(name: &str, value: &T) -> Result<(), String> {
    let dir = dir().ok_or("no config folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    // Write then rename, so a crash mid-write never leaves a half file.
    let f = dir.join(name);
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

pub fn load() -> Vec<Saved> {
    read("repos.json")
}

pub fn save(repos: &[Saved]) -> Result<(), String> {
    write("repos.json", &repos)
}

pub fn load_prefs() -> Prefs {
    read("prefs.json")
}

pub fn save_prefs(p: &Prefs) -> Result<(), String> {
    write("prefs.json", p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_tolerate_missing_fields() {
        let p = Prefs {
            theme: Theme::Dark,
            pinned: vec!["a/b".into()],
            groups: vec![Group { name: "Bet".into(), repos: vec!["x/y".into(), "z/w".into()] }],
            ..Prefs::default()
        };
        let text = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Prefs>(&text).unwrap(), p);
        let partial: Prefs = serde_json::from_str(r#"{"pinned":["a/b"]}"#).unwrap();
        assert_eq!(partial.theme, Theme::System);
        assert_eq!(partial.pinned, vec!["a/b".to_string()]);
        // New fields fill in their defaults for an older file.
        assert_eq!(partial.grace_secs, 5);
        assert!(partial.prod_names.contains(&"production".to_string()));
    }
}
