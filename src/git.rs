//! Local git: log, branches, status, and the safe everyday actions (fetch, fast-forward pull,
//! checkout). Nothing here commits or pushes. Also the commit graph's lane layout.

use std::path::Path;

use crate::gh::{cap, run};
use crate::model::{Branch, Cleanup, Commit, CommitDetail, LocalStatus, Stash};

const SEP: char = '\u{1f}'; // unit separator: can't appear in names or subjects

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run("git", args, Some(dir))
}

/// `owner/name` from the `origin` remote, for https and ssh URLs.
pub fn slug_of(dir: &Path) -> Option<String> {
    let url = git(dir, &["remote", "get-url", "origin"]).ok()?;
    slug_from_url(url.trim())
}

pub fn slug_from_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/").or_else(|| url.strip_prefix("git@github.com:")).or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let (o, n) = (parts.next()?, parts.next()?);
    (!o.is_empty() && !n.is_empty()).then(|| format!("{o}/{n}"))
}

pub fn is_repo(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--git-dir"]).is_ok()
}

/// The repository's top folder, for a path anywhere inside it.
pub fn toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"]).ok().map(|s| std::path::PathBuf::from(s.trim()))
}

pub fn log(dir: &Path, all: bool, limit: usize) -> Result<Vec<Commit>, String> {
    let n = format!("-n{limit}");
    let fmt = format!("--format=%H{SEP}%P{SEP}%an{SEP}%at{SEP}%D{SEP}%s");
    let mut args = vec!["log", "--date-order", n.as_str(), fmt.as_str()];
    if all {
        args.insert(1, "--all");
    }
    let out = git(dir, &args)?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut f = l.split(SEP);
            Some(Commit {
                sha: f.next()?.to_owned(),
                parents: f.next()?.split_whitespace().map(str::to_owned).collect(),
                author: f.next()?.to_owned(),
                time: f.next()?.parse().unwrap_or(0),
                refs: f.next()?.split(", ").filter(|r| !r.is_empty() && *r != "origin/HEAD").map(str::to_owned).collect(),
                subject: f.next().unwrap_or_default().to_owned(),
            })
        })
        .collect())
}

pub fn commit_detail(dir: &Path, sha: &str) -> Result<CommitDetail, String> {
    let body = git(dir, &["show", "-s", "--format=%B", sha])?;
    let numstat = git(dir, &["show", "--numstat", "--format=", sha])?;
    let files = numstat
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let a = f.next()?.parse().unwrap_or(0); // "-" for binary files
            let r = f.next()?.parse().unwrap_or(0);
            Some((f.next()?.to_owned(), a, r))
        })
        .collect();
    let mut diff = git(dir, &["show", "--format=", "--patch", sha])?;
    cap(&mut diff, 400_000);
    Ok(CommitDetail { body: body.trim().to_owned(), files, diff })
}

pub fn branches(dir: &Path) -> Result<Vec<Branch>, String> {
    let fmt = format!("--format=%(HEAD){SEP}%(refname){SEP}%(upstream:short){SEP}%(upstream:track,nobracket){SEP}%(committerdate:unix){SEP}%(authorname){SEP}%(contents:subject)");
    let out = git(dir, &["for-each-ref", "--sort=-committerdate", &fmt, "refs/heads", "refs/remotes"])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut f = l.split(SEP);
            let current = f.next()? == "*";
            let full = f.next()?;
            let (remote, name) = match full.strip_prefix("refs/heads/") {
                Some(n) => (false, n),
                None => (true, full.strip_prefix("refs/remotes/")?),
            };
            if name.ends_with("/HEAD") {
                return None;
            }
            let upstream = f.next()?.to_owned();
            let track = f.next()?;
            let (ahead, behind) = parse_track(track);
            Some(Branch {
                name: name.to_owned(),
                remote,
                current,
                upstream,
                ahead,
                behind,
                gone: track == "gone",
                time: f.next()?.parse().unwrap_or(0),
                author: f.next()?.to_owned(),
                subject: f.next().unwrap_or_default().to_owned(),
            })
        })
        .collect())
}

/// "ahead 2, behind 3" → (2, 3).
fn parse_track(s: &str) -> (u32, u32) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in s.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

pub fn status(dir: &Path) -> Result<LocalStatus, String> {
    let out = git(dir, &["status", "--porcelain=v2", "--branch"])?;
    let mut st = LocalStatus::default();
    for l in out.lines() {
        if let Some(b) = l.strip_prefix("# branch.head ") {
            st.branch = b.to_owned();
        } else if let Some(ab) = l.strip_prefix("# branch.ab ") {
            let mut it = ab.split_whitespace();
            st.ahead = it.next().and_then(|a| a.trim_start_matches('+').parse().ok()).unwrap_or(0);
            st.behind = it.next().and_then(|b| b.trim_start_matches('-').parse().ok()).unwrap_or(0);
        } else if !l.starts_with('#') {
            st.changed += 1;
        }
    }
    Ok(st)
}

pub fn fetch(dir: &Path) -> Result<String, String> {
    git(dir, &["fetch", "--all", "--prune"]).map(|_| "Fetched all remotes".into())
}

/// Fast-forward only: a pull here never creates a merge commit.
pub fn pull(dir: &Path) -> Result<String, String> {
    git(dir, &["pull", "--ff-only"]).map(|o| o.lines().last().unwrap_or("Up to date").to_owned())
}

/// Switch to a branch. A remote branch (`origin/x`) becomes a local tracking branch `x`.
pub fn checkout(dir: &Path, name: &str, remote: bool) -> Result<String, String> {
    if remote {
        let local = name.split_once('/').map_or(name, |(_, b)| b);
        git(dir, &["switch", local]).or_else(|_| git(dir, &["switch", "--track", name]))?;
        Ok(format!("Switched to {local}"))
    } else {
        git(dir, &["switch", name]).map(|_| format!("Switched to {name}"))
    }
}

// ---------------------------------------------------------------------------------------------
// A5: is it safe to switch branches / pull?
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Safety {
    /// Uncommitted changes (porcelain paths), staged or not, plus untracked files.
    pub changed: Vec<String>,
    /// "merge", "rebase", "cherry-pick" or "revert" when one is half done.
    pub in_progress: Option<&'static str>,
    /// On a detached HEAD whose commits are on no branch: switching away loses track of them.
    pub orphaned: usize,
    pub branch: String,
}

pub fn safety(dir: &Path) -> Result<Safety, String> {
    let out = git(dir, &["status", "--porcelain=v2", "--branch", "--untracked-files=normal"])?;
    let mut st = Safety::default();
    for l in out.lines() {
        if let Some(b) = l.strip_prefix("# branch.head ") {
            st.branch = b.to_owned();
        } else if let Some(path) = porcelain_path(l) {
            st.changed.push(path.to_owned());
        }
    }
    let git_dir = git(dir, &["rev-parse", "--git-dir"])?;
    let gd = dir.join(git_dir.trim());
    st.in_progress = [("MERGE_HEAD", "merge"), ("rebase-merge", "rebase"), ("rebase-apply", "rebase"), ("CHERRY_PICK_HEAD", "cherry-pick"), ("REVERT_HEAD", "revert")]
        .into_iter()
        .find(|(f, _)| gd.join(f).exists())
        .map(|(_, what)| what);
    if st.branch == "(detached)" {
        st.orphaned = git(dir, &["rev-list", "--count", "HEAD", "--not", "--branches", "--remotes"]).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(0);
    }
    Ok(st)
}

/// The path of a `git status --porcelain=v2` entry. Fields before it are fixed per entry type,
/// so paths with spaces survive; a rename's "\torig" is dropped.
fn porcelain_path(l: &str) -> Option<&str> {
    let fields = match l.chars().next()? {
        '1' => 9,
        '2' => 10,
        'u' => 11,
        '?' | '!' => 2,
        _ => return None,
    };
    let path = l.splitn(fields, ' ').nth(fields - 1)?;
    Some(path.split('\t').next().unwrap_or(path))
}

pub fn stash_push(dir: &Path, message: &str) -> Result<String, String> {
    git(dir, &["stash", "push", "--include-untracked", "-m", message]).map(|_| format!("Stashed: {message}"))
}

pub fn stashes(dir: &Path) -> Result<Vec<Stash>, String> {
    let out = git(dir, &["stash", "list", &format!("--format=%gd{SEP}%gs{SEP}%ct")])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut f = l.split(SEP);
            Some(Stash { name: f.next()?.to_owned(), message: f.next()?.to_owned(), time: f.next()?.parse().unwrap_or(0) })
        })
        .collect())
}

/// `op`: "apply", "pop" or "drop".
pub fn stash_op(dir: &Path, op: &str, name: &str) -> Result<String, String> {
    git(dir, &["stash", op, name]).map(|_| format!("Stash {op}: {name}"))
}

// ---------------------------------------------------------------------------------------------
// B7: local branch clean-up (never remote: deleting a remote branch is a push)
// ---------------------------------------------------------------------------------------------

/// Local branches that could go: merged into `default`, gone upstream, or untouched for 90 days.
/// `keep` decides what's never offered (current, default, protected, important names).
pub fn cleanup(dir: &Path, default: &str, now: i64, keep: impl Fn(&str) -> bool) -> Result<Vec<Cleanup>, String> {
    let merged: Vec<String> = git(dir, &["branch", "--merged", default, "--format=%(refname:short)"])
        .map(|o| o.lines().map(|l| l.trim().to_owned()).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for b in branches(dir)?.into_iter().filter(|b| !b.remote && !b.current && !keep(&b.name)) {
        let is_merged = merged.contains(&b.name);
        let stale = (now - b.time) / 86_400;
        let stale_days = (stale >= 90).then_some(stale);
        if !is_merged && !b.gone && stale_days.is_none() {
            continue;
        }
        let sha = git(dir, &["rev-parse", &b.name]).map(|s| s.trim().to_owned()).unwrap_or_default();
        let unpushed = if is_merged {
            Vec::new()
        } else {
            git(dir, &["log", &b.name, "--not", "--remotes", "-n", "20", "--format=%h %s"]).map(|o| o.lines().map(str::to_owned).collect()).unwrap_or_default()
        };
        out.push(Cleanup { name: b.name, sha, merged: is_merged, gone: b.gone, stale_days, unpushed });
    }
    Ok(out)
}

/// `-d` refuses unmerged branches; `-D` (force) deletes anyway.
pub fn delete_branch(dir: &Path, name: &str, force: bool) -> Result<String, String> {
    git(dir, &["branch", if force { "-D" } else { "-d" }, name]).map(|_| format!("Deleted branch {name}"))
}

/// Undo a delete: the branch again, at the commit it pointed to.
pub fn restore_branch(dir: &Path, name: &str, sha: &str) -> Result<String, String> {
    git(dir, &["branch", name, sha]).map(|_| format!("Restored branch {name}"))
}

// ---------------------------------------------------------------------------------------------
// Commit graph
// ---------------------------------------------------------------------------------------------

/// A line in one row's graph cell, in lane units: x is a lane index, y is 0 (row top),
/// 0.5 (the commit dot) or 1 (row bottom). `color` is a lane colour index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub color: usize,
}

#[derive(Clone, Debug, Default)]
pub struct GraphRow {
    pub lane: usize,
    pub color: usize,
    pub segs: Vec<Seg>,
}

/// Lay commits (newest first, each after all its children, as `--date-order` gives) out in
/// lanes. Each lane waits for one commit; a commit takes the lane waiting for it, merges in
/// other lanes waiting for it too, then waits for its first parent there and opens lanes for
/// the other parents.
pub fn layout(commits: &[Commit]) -> (Vec<GraphRow>, usize) {
    // (sha waited for, colour) per lane.
    let mut lanes: Vec<Option<(String, usize)>> = Vec::new();
    let mut next_color = 0usize;
    let mut width = 1;
    let mut rows = Vec::with_capacity(commits.len());
    for c in commits {
        let mut segs = Vec::new();
        let waiting: Vec<usize> = lanes.iter().enumerate().filter(|(_, l)| l.as_ref().is_some_and(|(s, _)| *s == c.sha)).map(|(i, _)| i).collect();
        let (lane, color) = match waiting.first() {
            Some(&i) => (i, lanes[i].as_ref().map_or(0, |l| l.1)),
            None => {
                // A branch tip: the first free lane, in a new colour.
                let i = lanes.iter().position(Option::is_none).unwrap_or_else(|| {
                    lanes.push(None);
                    lanes.len() - 1
                });
                next_color += 1;
                (i, next_color)
            }
        };
        // Lines coming in from the row above.
        for (i, l) in lanes.iter().enumerate() {
            let Some((_, col)) = l else { continue };
            if waiting.contains(&i) {
                segs.push(Seg { x0: i as f32, y0: 0.0, x1: lane as f32, y1: 0.5, color: *col });
            } else {
                segs.push(Seg { x0: i as f32, y0: 0.0, x1: i as f32, y1: 1.0, color: *col }); // passes through
            }
        }
        for &i in &waiting {
            lanes[i] = None;
        }
        // Lines going down to the parents.
        for (pi, p) in c.parents.iter().enumerate() {
            if pi == 0 {
                // The first parent always continues straight down this lane, even when another
                // lane already waits for it: both meet at the parent, and the main line stays put.
                lanes[lane] = Some((p.clone(), color));
                segs.push(Seg { x0: lane as f32, y0: 0.5, x1: lane as f32, y1: 1.0, color });
                continue;
            }
            let existing = lanes.iter().position(|l| l.as_ref().is_some_and(|(s, _)| s == p));
            let (target, col) = match existing {
                Some(t) => (t, lanes[t].as_ref().map_or(color, |l| l.1)),
                None => {
                    let t = lanes.iter().position(Option::is_none).unwrap_or_else(|| {
                        lanes.push(None);
                        lanes.len() - 1
                    });
                    next_color += 1;
                    lanes[t] = Some((p.clone(), next_color));
                    (t, next_color)
                }
            };
            segs.push(Seg { x0: lane as f32, y0: 0.5, x1: target as f32, y1: 1.0, color: col });
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        width = width.max(lanes.len()).max(lane + 1);
        rows.push(GraphRow { lane, color, segs });
    }
    (rows, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(sha: &str, parents: &[&str]) -> Commit {
        Commit { sha: sha.into(), parents: parents.iter().map(|p| p.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn slugs_from_remote_urls() {
        assert_eq!(slug_from_url("https://github.com/anomalyco/rune.git").as_deref(), Some("anomalyco/rune"));
        assert_eq!(slug_from_url("git@github.com:octo-org/octo-app.git").as_deref(), Some("octo-org/octo-app"));
        assert_eq!(slug_from_url("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn porcelain_paths_keep_spaces() {
        assert_eq!(porcelain_path("1 .M N... 100644 100644 100644 aaa bbb src/my file.rs"), Some("src/my file.rs"));
        assert_eq!(porcelain_path("2 R. N... 100644 100644 100644 aaa bbb R100 new name.rs\told.rs"), Some("new name.rs"));
        assert_eq!(porcelain_path("? notes/to do.txt"), Some("notes/to do.txt"));
        assert_eq!(porcelain_path("# branch.head main"), None);
    }

    #[test]
    fn tracks() {
        assert_eq!(parse_track("ahead 2, behind 3"), (2, 3));
        assert_eq!(parse_track("behind 1"), (0, 1));
        assert_eq!(parse_track(""), (0, 0));
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let (rows, width) = layout(&[c("c", &["b"]), c("b", &["a"]), c("a", &[])]);
        assert_eq!(width, 1);
        assert!(rows.iter().all(|r| r.lane == 0));
    }

    #[test]
    fn a_merge_opens_and_closes_a_lane() {
        // m merges f into b; f and b both come from a.
        let commits = [c("m", &["b", "f"]), c("f", &["a"]), c("b", &["a"]), c("a", &[])];
        let (rows, width) = layout(&commits);
        assert_eq!(width, 2);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1); // f in its own lane
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[3].lane, 0); // both lanes join at a
        // a has two lines coming in: one from lane 0, one from lane 1.
        assert_eq!(rows[3].segs.iter().filter(|s| s.y1 == 0.5).count(), 2);
    }

    #[test]
    fn two_tips_get_two_lanes() {
        let (rows, width) = layout(&[c("x", &["a"]), c("y", &["a"]), c("a", &[])]);
        assert_eq!(width, 2);
        assert_ne!(rows[0].lane, rows[1].lane);
        assert_ne!(rows[0].color, rows[1].color);
    }
}

/// Your git identity for commits on this computer: (user.name, user.email) from the global
/// config, empty when unset.
pub fn global_identity() -> (String, String) {
    let get = |k: &str| crate::gh::run("git", &["config", "--global", "--get", k], None).map(|o| o.trim().to_owned()).unwrap_or_default();
    (get("user.name"), get("user.email"))
}

/// Set (or, with an empty value, unset) a global git setting.
pub fn set_global(key: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        // Unsetting a key that isn't set fails: nothing to do either way.
        let _ = crate::gh::run("git", &["config", "--global", "--unset", key], None);
        return Ok(());
    }
    crate::gh::run("git", &["config", "--global", key, value], None).map(|_| ())
}
