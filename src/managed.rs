//! Clones sigit makes for you: "Clone for me" on a repo that has no local clone. sigit runs
//! `gh repo clone` (gh signs in, private repos included) into a folder it manages, by default
//! `~/sigit/repos/<owner>/<name>` (Settings → Git changes it), and adds the clone like any other.
//! A folder that already holds a clone of the same repo is used as it is.

use std::path::{Path, PathBuf};

use crate::gh::Cmd;

/// The default home of managed clones: `~/sigit/repos`.
pub fn default_home() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").filter(|_| cfg!(windows)).or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join("sigit").join("repos"))
}

/// Where `owner/name` goes under `home`.
pub fn dir_for(home: &Path, slug: &str) -> Option<PathBuf> {
    let (owner, name) = slug.split_once('/')?;
    let ok = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.contains(['/', '\\']);
    (ok(owner) && ok(name)).then(|| home.join(owner).join(name))
}

pub fn clone_cmd(slug: &str, dir: &Path) -> Cmd {
    Cmd::new("gh", &["repo", "clone", slug, &dir.to_string_lossy()])
}

/// What's at the target now.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// Nothing (or an empty folder): clone into it.
    Free,
    /// Already a clone of this repo: use it.
    Ours,
    /// Something else is there: don't touch it.
    Taken(String),
}

pub fn check(dir: &Path, slug: &str) -> Target {
    if !dir.exists() || std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_none()) {
        return Target::Free;
    }
    match crate::git::toplevel(dir) {
        Some(top) if top == dir || top.canonicalize().ok() == dir.canonicalize().ok() => match crate::git::slug_of(dir) {
            Some(s) if s.eq_ignore_ascii_case(slug) => Target::Ours,
            Some(s) => Target::Taken(tf!("{} already holds a clone of {s}", dir.display())),
            None => Target::Taken(tf!("{} already holds a git repository that isn't {slug}", dir.display())),
        },
        _ => Target::Taken(tf!("{} already exists and isn't a clone of {slug}", dir.display())),
    }
}

/// Clone `slug` under `home` (or find the clone already there). Returns its folder.
pub fn clone(home: &Path, slug: &str) -> Result<PathBuf, String> {
    let dir = dir_for(home, slug).ok_or_else(|| tf!("\"{slug}\" isn't owner/name"))?;
    match check(&dir, slug) {
        Target::Ours => return Ok(dir),
        Target::Taken(why) => return Err(tf!("{why}. Move it, or pick another folder for clones in Settings → Git.")),
        Target::Free => {}
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| tf!("Couldn't make {}: {e}", parent.display()))?;
    }
    clone_cmd(slug, &dir).run(None)?;
    match check(&dir, slug) {
        Target::Ours => Ok(dir),
        _ => Err(tf!("The clone in {} didn't come out as {slug}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_and_what_is_there() {
        let home = std::env::temp_dir().join(format!("sigit-managed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(dir_for(&home, "acme/app").unwrap(), home.join("acme").join("app"));
        assert!(dir_for(&home, "acme").is_none());
        assert!(dir_for(&home, "../x").is_none() && dir_for(&home, "a/..").is_none());
        let dir = dir_for(&home, "acme/app").unwrap();
        assert_eq!(check(&dir, "acme/app"), Target::Free, "nothing there yet");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(check(&dir, "acme/app"), Target::Free, "an empty folder is free");
        let run = |args: &[&str]| crate::gh::run("git", args, Some(&dir)).unwrap();
        run(&["init", "-q"]);
        run(&["remote", "add", "origin", "https://github.com/acme/app.git"]);
        assert_eq!(check(&dir, "acme/app"), Target::Ours, "a clone of the same repo is used");
        assert_eq!(check(&dir, "ACME/App"), Target::Ours, "GitHub names ignore case");
        assert!(matches!(check(&dir, "acme/other"), Target::Taken(_)), "another repo's clone is left alone");
        let other = home.join("acme").join("notes");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("a.txt"), "x").unwrap();
        assert!(matches!(check(&other, "acme/notes"), Target::Taken(_)), "a folder with files is left alone");
        assert!(clone(&home, "acme/notes").unwrap_err().contains("Settings"));
        assert_eq!(clone(&home, "acme/app").unwrap(), dir, "an existing clone needs no download");
        let _ = std::fs::remove_dir_all(&home);
    }
}
