//! History editing on the current branch of a local clone: reset, revert, cherry-pick, reword,
//! drop, squash / fixup, move, amend, and Undo. Each edit is one transaction in four steps
//! (`docs/superpowers/specs/2026-11-27-history-editing-design.md`):
//!
//! 1. **Snapshot**: branch, tip, upstream, working copy.
//! 2. **Simulate**: the result is made as real commit objects with plumbing (`merge-tree`,
//!    `commit-tree`, `write-tree`). No ref moves, and the index and the files don't change.
//! 3. **Preview**: what leaves the branch, what comes in, the diffstat, pushed commits rewritten.
//! 4. **Preflight, then apply**: fresh state must equal the snapshot (and the remote must equal
//!    the upstream we know), then `git update-ref <ref> <new> <old>` moves the branch only if it
//!    is still at `<old>`, then the index and the files follow.
//!
//! Nothing here pushes.

use std::path::Path;

use crate::gh::{self, Cmd};

const SEP: char = '\u{1f}';

/// The oldest git with `merge-tree --write-tree --merge-base`.
pub const MIN_GIT: (u32, u32) = (2, 40);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetMode {
    pub fn word(self) -> &'static str {
        match self {
            ResetMode::Soft => "soft",
            ResetMode::Mixed => "mixed",
            ResetMode::Hard => "hard",
        }
    }
}

/// One history edit. SHAs are full or short; `simulate` resolves them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Reset { to: String, mode: ResetMode },
    /// A new commit that undoes `sha` (against parent 1 for a merge).
    Revert { sha: String },
    /// A copy of `sha` (from another branch) on top.
    CherryPick { sha: String },
    Reword { sha: String, message: String },
    Drop { sha: String },
    /// Join `sha` with its parent. `fixup`: keep only the parent's message.
    Squash { sha: String, fixup: bool },
    /// Swap `sha` with its child (`up`, toward the tip) or its parent.
    Move { sha: String, up: bool },
    /// Put the staged changes into the last commit.
    Amend,
}

impl Op {
    pub fn label(&self) -> String {
        let short = |s: &str| s.get(..7).unwrap_or(s).to_owned();
        match self {
            Op::Reset { to, mode } => tf!("Reset to {} ({})", short(to), mode.word()),
            Op::Revert { sha } => tf!("Revert {}", short(sha)),
            Op::CherryPick { sha } => tf!("Cherry-pick {}", short(sha)),
            Op::Reword { sha, .. } => tf!("Reword {}", short(sha)),
            Op::Drop { sha } => tf!("Drop {}", short(sha)),
            Op::Squash { sha, fixup: false } => tf!("Squash {} into its parent", short(sha)),
            Op::Squash { sha, fixup: true } => tf!("Fixup {} into its parent", short(sha)),
            Op::Move { sha, up: true } => tf!("Move {} up", short(sha)),
            Op::Move { sha, up: false } => tf!("Move {} down", short(sha)),
            Op::Amend => "Amend the last commit".into(),
        }
    }
}

/// The busy label while `op` is being simulated (a button shows a spinner for it).
pub fn sim_label(op: &Op) -> String {
    format!("Simulating: {}", op.label())
}

/// How the index and the files follow the branch after the ref moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync {
    /// Neither (soft reset; amend, where the index already is the new tree).
    None,
    /// The index only (mixed reset).
    Index,
    /// Index and files replaced (hard reset).
    Hard,
    /// A two-tree merge from old to new: only the files that differ change, and git refuses
    /// if a local change is in the way.
    TwoTree,
}

/// The state an edit was simulated against.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub branch: String,
    pub tip: String,
    /// `origin/main`, when the branch has one.
    pub upstream: Option<String>,
    /// The upstream's SHA as this clone knows it (from the last fetch).
    pub upstream_sha: Option<String>,
    /// The remote and the ref on it (`origin`, `refs/heads/main`), for `ls-remote`.
    pub remote: Option<(String, String)>,
    pub in_progress: Option<&'static str>,
    pub changed: Vec<String>,
    /// The staged tree (`git write-tree`), for amend.
    pub index_tree: Option<String>,
}

/// A commit, for the preview: short SHA and subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub sha: String,
    pub subject: String,
    pub full: String,
}

/// What an edit does to one commit, for the preview's graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Leaves the branch.
    Leaves,
    /// New on the branch.
    New,
    /// Made again (new SHA): reworded, moved, or after an edited commit.
    Rewritten,
    /// Joined into the commit above it (squash, fixup).
    Joined,
}

/// One row of the preview's graph: the old commit, the new one, or both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: Kind,
    pub old: Option<Line>,
    pub new: Option<Line>,
}

/// A file that differs between the old and the new tip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub add: u64,
    pub del: u64,
}

/// The graph's rows, newest first: each new commit with the old one(s) it replaces, then the
/// commits that only leave.
pub fn rows(removed: &[Line], added: &[Line], pairs: &[(String, String)]) -> Vec<Row> {
    let mut out = Vec::new();
    let mut used = std::collections::HashSet::new();
    for a in added {
        let olds: Vec<&Line> = pairs.iter().filter(|(_, n)| *n == a.full).filter_map(|(o, _)| removed.iter().find(|r| r.full == *o)).collect();
        match olds.split_first() {
            None => out.push(Row { kind: Kind::New, old: None, new: Some(a.clone()) }),
            Some((first, rest)) => {
                used.insert(first.full.clone());
                out.push(Row { kind: Kind::Rewritten, old: Some((*first).clone()), new: Some(a.clone()) });
                for o in rest {
                    used.insert(o.full.clone());
                    out.push(Row { kind: Kind::Joined, old: Some((*o).clone()), new: Some(a.clone()) });
                }
            }
        }
    }
    out.extend(removed.iter().filter(|r| !used.contains(&r.full)).map(|r| Row { kind: Kind::Leaves, old: Some(r.clone()), new: None }));
    out
}

/// A simulated edit: the new tip exists as objects; nothing else changed.
#[derive(Clone, Debug)]
pub struct Plan {
    pub op: Op,
    pub snap: Snapshot,
    pub new_tip: String,
    /// On the old tip, not on the new one.
    pub removed: Vec<Line>,
    /// On the new tip, not on the old one.
    pub added: Vec<Line>,
    /// What the edit does to each commit, for the graph.
    pub rows: Vec<Row>,
    /// The files that differ between the old and the new tip (`git diff --numstat`).
    pub files: Vec<FileChange>,
    /// Removed commits that a remote has (any remote branch, not only the upstream): rewriting
    /// them needs a force-push.
    pub pushed_rewritten: usize,
    /// A hard reset: the uncommitted files it would replace (the guard stashes them first).
    pub replaced_files: Vec<String>,
    pub sync: Sync,
}

impl Plan {
    pub fn label(&self) -> String {
        tf!("{} on {}", self.op.label(), self.snap.branch)
    }

    pub fn branch_ref(&self) -> String {
        format!("refs/heads/{}", self.snap.branch)
    }

    /// The commands the apply runs, in order, as shown in the dialog. The preflight's
    /// `ls-remote` is read-only.
    pub fn commands(&self) -> Vec<Cmd> {
        let mut v = Vec::new();
        if let Some((remote, r)) = &self.snap.remote {
            v.push(ls_remote_cmd(remote, r));
        }
        v.push(update_ref_cmd(&self.branch_ref(), &self.new_tip, &self.snap.tip, &self.op.label()));
        v.extend(sync_cmd(self.sync, &self.snap.tip, &self.new_tip));
        v
    }

    /// What the user runs to share the edit, when pushed commits were rewritten.
    pub fn push_hint(&self) -> Option<String> {
        let (remote, r) = self.snap.remote.as_ref()?;
        (self.pushed_rewritten > 0).then(|| {
            let branch = r.strip_prefix("refs/heads/").unwrap_or(r);
            gh::line("git", &["push", "--force-with-lease", remote, &format!("{}:{branch}", self.snap.branch)])
        })
    }
}

pub fn ls_remote_cmd(remote: &str, r: &str) -> Cmd {
    Cmd::new("git", &["ls-remote", remote, r])
}

pub fn update_ref_cmd(r: &str, new: &str, old: &str, why: &str) -> Cmd {
    Cmd::new("git", &["update-ref", "-m", &format!("sigit: {why}"), r, new, old])
}

fn sync_cmd(sync: Sync, old: &str, new: &str) -> Option<Cmd> {
    match sync {
        Sync::None => None,
        Sync::Index => Some(Cmd::new("git", &["read-tree", new])),
        Sync::Hard => Some(Cmd::new("git", &["reset", "--hard", new])),
        Sync::TwoTree => Some(Cmd::new("git", &["read-tree", "-m", "-u", old, new])),
    }
}

// ---------------------------------------------------------------------------------------------
// Small git helpers
// ---------------------------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    gh::run("git", args, Some(dir))
}

/// Runs git and says whether it succeeded, without making a failure an error.
fn git_ok(dir: &Path, args: &[&str]) -> Result<bool, String> {
    gh::run_full("git", args, Some(dir), None, &[]).map(|o| o.ok)
}

fn rev(dir: &Path, spec: &str) -> Result<String, String> {
    git(dir, &["rev-parse", "--verify", "--quiet", &format!("{spec}^{{commit}}")]).map(|s| s.trim().to_owned()).map_err(|_| tf!("{spec} isn't a commit here"))
}

fn short(sha: &str) -> String {
    sha.get(..7).unwrap_or(sha).to_owned()
}

/// `git version 2.43.0` → (2, 43).
pub fn parse_version(s: &str) -> Option<(u32, u32)> {
    let v = s.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut it = v.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

fn check_git_version(dir: &Path) -> Result<(), String> {
    let v = git(dir, &["version"])?;
    match parse_version(&v) {
        Some(have) if have >= MIN_GIT => Ok(()),
        _ => Err(tf!("This needs git {}.{} or newer (you have: {})", MIN_GIT.0, MIN_GIT.1, v.trim())),
    }
}

/// What commit-tree needs to copy a commit.
#[derive(Clone, Debug)]
struct Info {
    tree: String,
    parents: Vec<String>,
    author: (String, String, String),
    message: String,
}

fn info(dir: &Path, sha: &str) -> Result<Info, String> {
    let fmt = format!("--format=%T{SEP}%P{SEP}%an{SEP}%ae{SEP}%ad{SEP}%B");
    let out = git(dir, &["log", "-1", "--date=raw", &fmt, sha])?;
    let mut f = out.splitn(6, SEP);
    let mut next = || f.next().map(str::to_owned).ok_or_else(|| tf!("Couldn't read commit {}", short(sha)));
    let tree = next()?;
    let parents = next()?.split_whitespace().map(str::to_owned).collect();
    let author = (next()?, next()?, next()?);
    // `log` adds a newline after %B; the message itself ends in one.
    let mut message = next()?.trim_end_matches('\n').to_owned();
    message.push('\n');
    Ok(Info { tree, parents, author, message })
}

fn subject(msg: &str) -> &str {
    msg.lines().next().unwrap_or_default()
}

enum Merge {
    Tree(String),
    Conflict(Vec<String>),
}

/// A three-way merge into a tree object, with no working copy.
fn merge_tree(dir: &Path, base: &str, ours: &str, theirs: &str) -> Result<Merge, String> {
    let mb = format!("--merge-base={base}");
    let o = gh::run_full("git", &["merge-tree", "--write-tree", "--name-only", "--no-messages", &mb, ours, theirs], Some(dir), None, &[])?;
    let mut lines = o.stdout.lines();
    let tree = lines.next().unwrap_or_default().trim().to_owned();
    match o.code {
        Some(0) if !tree.is_empty() => Ok(Merge::Tree(tree)),
        Some(1) => {
            let mut files: Vec<String> = lines.take_while(|l| !l.is_empty()).map(str::to_owned).collect();
            files.dedup();
            Ok(Merge::Conflict(files))
        }
        _ => Err(if o.stderr.is_empty() { "git merge-tree failed".into() } else { o.stderr }),
    }
}

/// A commit object. `author`: copied from the original commit; `None` = you.
fn commit_tree(dir: &Path, tree: &str, parents: &[&str], message: &str, author: Option<&(String, String, String)>) -> Result<String, String> {
    let mut args = vec!["commit-tree", tree];
    // `commit-tree` signs only when asked: keep signing when you sign your commits.
    if git(dir, &["config", "--bool", "commit.gpgsign"]).is_ok_and(|v| v.trim() == "true") {
        args.push("-S");
    }
    for p in parents {
        args.push("-p");
        args.push(p);
    }
    args.extend(["-F", "-"]);
    let env: Vec<(&str, &str)> = author.map_or_else(Vec::new, |(n, e, d)| vec![("GIT_AUTHOR_NAME", n.as_str()), ("GIT_AUTHOR_EMAIL", e.as_str()), ("GIT_AUTHOR_DATE", d.as_str())]);
    let o = gh::run_full("git", &args, Some(dir), Some(message), &env)?;
    if !o.ok {
        return Err(if o.stderr.is_empty() { "git commit-tree failed".into() } else { o.stderr });
    }
    Ok(o.stdout.trim().to_owned())
}

fn conflict(what: &str, files: &[String]) -> String {
    let mut s = tf!("{what} would conflict in: {}", files.iter().take(8).cloned().collect::<Vec<_>>().join(", "));
    if files.len() > 8 {
        s.push_str(&tf!(" (and {} more)", files.len() - 8));
    }
    s.push_str(&crate::i18n::t(". Nothing was changed. Resolve it in your terminal, or pick another edit."));
    s
}

// ---------------------------------------------------------------------------------------------
// 1. Snapshot
// ---------------------------------------------------------------------------------------------

pub fn snapshot(dir: &Path) -> Result<Snapshot, String> {
    let head = git(dir, &["symbolic-ref", "-q", "HEAD"]).map_err(|_| crate::i18n::t("You're on a detached HEAD: switch to a branch to edit its history"))?;
    let branch = head.trim().strip_prefix("refs/heads/").ok_or_else(|| crate::i18n::t("HEAD isn't on a local branch"))?.to_owned();
    let tip = rev(dir, "HEAD").map_err(|_| crate::i18n::t("This branch has no commits yet"))?;
    let safety = crate::git::safety(dir)?;
    let upstream = git(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let upstream_sha = upstream.as_ref().and_then(|_| rev(dir, "@{upstream}").ok());
    let cfg = |k: &str| git(dir, &["config", "--get", &format!("branch.{branch}.{k}")]).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let remote = upstream.as_ref().and_then(|_| Some((cfg("remote")?, cfg("merge")?))).filter(|(r, _)| r != ".");
    let index_tree = git(dir, &["write-tree"]).ok().map(|s| s.trim().to_owned());
    Ok(Snapshot { branch, tip, upstream, upstream_sha, remote, in_progress: safety.in_progress, changed: safety.changed, index_tree })
}

// ---------------------------------------------------------------------------------------------
// 2. Simulate
// ---------------------------------------------------------------------------------------------

/// One step of a replay, oldest first.
enum Step {
    /// Make this commit again on the new parent; `Some` message replaces its own.
    Pick(String, Option<String>),
    /// Join this commit into the commit made just before. `fixup`: keep that one's message.
    Fold(String, bool),
}

/// Old → new SHA for each commit an edit made again (or joined into another), oldest first.
type Pairs = Vec<(String, String)>;

/// Makes `steps` again on top of `base`. A commit whose parent didn't change is kept as it is.
/// Returns the new tip and the old → new pairs.
fn replay(dir: &Path, base: &str, steps: &[Step]) -> Result<(String, Pairs), String> {
    let mut cur = base.to_owned();
    let mut pairs: Pairs = Vec::new();
    // The commit made last: its parent, message and author, for a Fold.
    let mut last: Option<(Option<String>, String, (String, String, String))> = None;
    for step in steps {
        match step {
            Step::Pick(sha, msg) => {
                let c = info(dir, sha)?;
                let parent = c.parents.first().cloned().unwrap_or_default();
                if parent == cur && msg.is_none() {
                    last = Some((Some(parent), c.message, c.author));
                    cur = sha.clone();
                    continue;
                }
                let tree = if parent == cur {
                    c.tree.clone()
                } else {
                    match merge_tree(dir, &parent, &cur, sha)? {
                        Merge::Tree(t) => t,
                        Merge::Conflict(f) => return Err(conflict(&tf!("Making {} again", short(sha)), &f)),
                    }
                };
                let message = msg.clone().unwrap_or(c.message);
                let new = commit_tree(dir, &tree, &[&cur], &message, Some(&c.author))?;
                last = Some((Some(cur.clone()), message, c.author));
                pairs.push((sha.clone(), new.clone()));
                cur = new;
            }
            Step::Fold(sha, fixup) => {
                let c = info(dir, sha)?;
                let (prev_parent, prev_msg, prev_author) = last.clone().ok_or_else(|| crate::i18n::t("Nothing to squash into"))?;
                let parent = c.parents.first().cloned().unwrap_or_default();
                let tree = if parent == cur {
                    c.tree.clone()
                } else {
                    match merge_tree(dir, &parent, &cur, sha)? {
                        Merge::Tree(t) => t,
                        Merge::Conflict(f) => return Err(conflict(&tf!("Squashing {}", short(sha)), &f)),
                    }
                };
                let message = if *fixup { prev_msg.clone() } else { format!("{}\n\n{}", prev_msg.trim_end(), c.message) };
                let parents: Vec<&str> = prev_parent.iter().map(String::as_str).collect();
                let new = commit_tree(dir, &tree, &parents, &message, Some(&prev_author))?;
                last = Some((prev_parent, message, prev_author));
                // The commit it joins into is replaced too: point its pair at the new one.
                if pairs.iter().any(|(_, n)| *n == cur) {
                    for p in pairs.iter_mut().filter(|(_, n)| *n == cur) {
                        p.1 = new.clone();
                    }
                } else {
                    pairs.push((cur.clone(), new.clone()));
                }
                pairs.push((sha.clone(), new.clone()));
                cur = new;
            }
        }
    }
    Ok((cur, pairs))
}

/// The commits from `sha` (included) to the tip, oldest first, checked: on the branch, not the
/// root, no merge commit, so a replay can make them again.
fn linear_from(dir: &Path, tip: &str, sha: &str) -> Result<(String, Vec<String>), String> {
    if !git_ok(dir, &["merge-base", "--is-ancestor", sha, tip])? {
        return Err(tf!("{} isn't on this branch", short(sha)));
    }
    let c = info(dir, sha)?;
    let base = match c.parents.as_slice() {
        [] => return Err(crate::i18n::t("This is the first commit of the history: it can't be edited here")),
        [p] => p.clone(),
        _ => return Err(crate::i18n::t("This is a merge commit: only Revert and Reset work on merges")),
    };
    let range = format!("{base}..{tip}");
    let merges = git(dir, &["rev-list", "--merges", &range])?;
    if !merges.trim().is_empty() {
        return Err(tf!("There's a merge commit between {} and the tip: editing would flatten it. Use Revert or Reset instead.", short(sha)));
    }
    let list = git(dir, &["rev-list", "--reverse", &range])?.lines().map(str::to_owned).collect();
    Ok((base, list))
}

fn needs_identity(dir: &Path) -> Result<(), String> {
    if git_ok(dir, &["var", "GIT_COMMITTER_IDENT"])? {
        Ok(())
    } else {
        Err(crate::i18n::t("Set your commit name and email first (Settings → Git)"))
    }
}

/// Simulate `op` on the current branch: the new tip as objects, and the preview.
pub fn simulate(dir: &Path, op: &Op) -> Result<Plan, String> {
    let snap = snapshot(dir)?;
    if let Some(what) = snap.in_progress {
        return Err(tf!("A {what} is half done in this working copy. Finish or abort it first."));
    }
    let tip = snap.tip.clone();
    let sync = match op {
        Op::Reset { mode: ResetMode::Soft, .. } | Op::Amend => Sync::None,
        Op::Reset { mode: ResetMode::Mixed, .. } => Sync::Index,
        Op::Reset { mode: ResetMode::Hard, .. } => Sync::Hard,
        _ => Sync::TwoTree,
    };
    if !matches!(op, Op::Reset { .. }) {
        needs_identity(dir)?;
    }
    if !matches!(op, Op::Reset { .. } | Op::Amend) {
        check_git_version(dir)?;
    }
    let mut pairs: Pairs = Vec::new();
    let mut replayed = |r: (String, Pairs)| {
        pairs = r.1;
        r.0
    };
    let new_tip = match op {
        Op::Reset { to, .. } => {
            let to = rev(dir, to)?;
            if to == tip {
                return Err(tf!("{} is already at {}", snap.branch, short(&to)));
            }
            to
        }
        Op::Revert { sha } => {
            let sha = rev(dir, sha)?;
            if !git_ok(dir, &["merge-base", "--is-ancestor", &sha, &tip])? {
                return Err(tf!("{} isn't on this branch: nothing to revert", short(&sha)));
            }
            let c = info(dir, &sha)?;
            let Some(parent) = c.parents.first() else { return Err(crate::i18n::t("The first commit of the history can't be reverted")) };
            let tree = match merge_tree(dir, &sha, &tip, parent)? {
                Merge::Tree(t) => t,
                Merge::Conflict(f) => return Err(conflict(&tf!("Reverting {}", short(&sha)), &f)),
            };
            let mut msg = format!("Revert \"{}\"\n\nThis reverts commit {sha}", subject(&c.message));
            if c.parents.len() > 1 {
                msg.push_str(&format!(", reversing\nchanges made to {parent}"));
            }
            msg.push_str(".\n");
            commit_tree(dir, &tree, &[&tip], &msg, None)?
        }
        Op::CherryPick { sha } => {
            let sha = rev(dir, sha)?;
            if git_ok(dir, &["merge-base", "--is-ancestor", &sha, &tip])? {
                return Err(tf!("{} is already on {}", short(&sha), snap.branch));
            }
            let c = info(dir, &sha)?;
            let parent = match c.parents.as_slice() {
                [p] => p.clone(),
                [] => return Err(crate::i18n::t("The first commit of a history can't be cherry-picked here")),
                _ => return Err(crate::i18n::t("This is a merge commit: cherry-pick needs a single-parent commit")),
            };
            let tree = match merge_tree(dir, &parent, &tip, &sha)? {
                Merge::Tree(t) => t,
                Merge::Conflict(f) => return Err(conflict(&tf!("Cherry-picking {}", short(&sha)), &f)),
            };
            if tree == info(dir, &tip)?.tree {
                return Err(tf!("{}'s changes are already on {}: nothing to pick", short(&sha), snap.branch));
            }
            commit_tree(dir, &tree, &[&tip], &c.message, Some(&c.author))?
        }
        Op::Reword { sha, message } => {
            let sha = rev(dir, sha)?;
            let message = message.trim();
            if message.is_empty() {
                return Err(crate::i18n::t("The message is empty"));
            }
            let (base, list) = linear_from(dir, &tip, &sha)?;
            let steps: Vec<Step> = list.into_iter().map(|c| { let m = (c == sha).then(|| format!("{message}\n")); Step::Pick(c, m) }).collect();
            replayed(replay(dir, &base, &steps)?)
        }
        Op::Drop { sha } => {
            let sha = rev(dir, sha)?;
            let (base, list) = linear_from(dir, &tip, &sha)?;
            let steps: Vec<Step> = list.into_iter().filter(|c| *c != sha).map(|c| Step::Pick(c, None)).collect();
            replayed(replay(dir, &base, &steps)?)
        }
        Op::Squash { sha, fixup } => {
            let sha = rev(dir, sha)?;
            let (_, list) = linear_from(dir, &tip, &sha)?;
            let parent = info(dir, &sha)?.parents.first().cloned().unwrap_or_default();
            let (base, _) = linear_from(dir, &tip, &parent).map_err(|e| tf!("Can't squash into the commit before it: {e}"))?;
            let mut steps = vec![Step::Pick(parent, None)];
            for c in list {
                steps.push(if c == sha { Step::Fold(c, *fixup) } else { Step::Pick(c, None) });
            }
            replayed(replay(dir, &base, &steps)?)
        }
        Op::Move { sha, up } => {
            let sha = rev(dir, sha)?;
            let (older, newer) = if *up {
                let (_, list) = linear_from(dir, &tip, &sha)?;
                let child = list.get(1).cloned().ok_or_else(|| crate::i18n::t("It's already the newest commit"))?;
                (sha, child)
            } else {
                let parent = info(dir, &sha)?.parents.first().cloned().ok_or_else(|| crate::i18n::t("It's already the oldest commit"))?;
                (parent, sha)
            };
            let (base, list) = linear_from(dir, &tip, &older).map_err(|e| tf!("Can't move it: {e}"))?;
            let mut steps = vec![Step::Pick(newer.clone(), None), Step::Pick(older.clone(), None)];
            steps.extend(list.into_iter().filter(|c| *c != older && *c != newer).map(|c| Step::Pick(c, None)));
            replayed(replay(dir, &base, &steps)?)
        }
        Op::Amend => {
            let index = snap.index_tree.clone().ok_or_else(|| crate::i18n::t("Couldn't read the staged changes (is a merge half done?)"))?;
            let c = info(dir, &tip)?;
            if index == c.tree {
                return Err(crate::i18n::t("Nothing is staged: stage the changes to put into the last commit first (git add)"));
            }
            let parents: Vec<&str> = c.parents.iter().map(String::as_str).collect();
            let new = commit_tree(dir, &index, &parents, &c.message, Some(&c.author))?;
            pairs = vec![(tip.clone(), new.clone())];
            new
        }
    };
    preview(dir, op.clone(), snap, new_tip, sync, &pairs)
}

fn lines(dir: &Path, range: &[&str]) -> Result<Vec<Line>, String> {
    let fmt = format!("--format=%H{SEP}%h{SEP}%s");
    let mut args = vec!["log", "-n", "200", &fmt];
    args.extend_from_slice(range);
    Ok(git(dir, &args)?
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, SEP);
            let full = f.next()?.to_owned();
            Some(Line { sha: f.next()?.to_owned(), subject: f.next().unwrap_or_default().to_owned(), full })
        })
        .collect())
}

fn preview(dir: &Path, op: Op, snap: Snapshot, new_tip: String, sync: Sync, pairs: &[(String, String)]) -> Result<Plan, String> {
    let (old, new) = (snap.tip.as_str(), new_tip.as_str());
    let removed = lines(dir, &[old, &format!("^{new}")])?;
    let added = lines(dir, &[new, &format!("^{old}")])?;
    let files = git(dir, &["diff", "--numstat", old, new])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let add = f.next()?.parse().unwrap_or(0); // "-" for binary files
            let del = f.next()?.parse().unwrap_or(0);
            Some(FileChange { path: f.next()?.to_owned(), add, del })
        })
        .collect();
    // Removed commits that some remote branch has: all removed, minus those on no remote.
    let pushed_rewritten = if removed.is_empty() {
        0
    } else {
        let count = |extra: &[&str]| -> usize {
            let not_new = format!("^{new}");
            let mut args = vec!["rev-list", "--count", old, not_new.as_str()];
            args.extend_from_slice(extra);
            git(dir, &args).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(0)
        };
        count(&[]).saturating_sub(count(&["--not", "--remotes"]))
    };
    let replaced_files = if sync == Sync::Hard { snap.changed.clone() } else { Vec::new() };
    let rows = rows(&removed, &added, pairs);
    Ok(Plan { op, snap, new_tip, removed, added, rows, files, pushed_rewritten, replaced_files, sync })
}

// ---------------------------------------------------------------------------------------------
// 4. Preflight and apply
// ---------------------------------------------------------------------------------------------

/// What the preflight found: `Err` blocks (simulate again); the `Ok` list is warnings.
pub fn preflight(dir: &Path, plan: &Plan) -> Result<Vec<String>, String> {
    let again = crate::i18n::t("Nothing was changed: simulate it again.");
    let now = snapshot(dir).map_err(|e| tf!("{e}. {again}"))?;
    let was = &plan.snap;
    if now.branch != was.branch {
        return Err(tf!("You're on {} now, not {}. {again}", now.branch, was.branch));
    }
    if now.tip != was.tip {
        return Err(tf!("{} moved since the simulation ({} → {}). {again}", was.branch, short(&was.tip), short(&now.tip)));
    }
    if let Some(what) = now.in_progress {
        return Err(tf!("A {what} started in this working copy. {again}"));
    }
    if now.upstream_sha != was.upstream_sha {
        return Err(tf!("{} changed since the simulation (a fetch?). {again}", was.upstream.clone().unwrap_or_else(|| "The upstream".into())));
    }
    // A two-tree merge refuses when an uncommitted change is in a file the edit changes: say so
    // now instead of failing at the end.
    if plan.sync == Sync::TwoTree && !now.changed.is_empty() {
        let differ = git(dir, &["diff", "--name-only", &was.tip, &plan.new_tip])?;
        let in_way: Vec<&str> = differ.lines().filter(|f| now.changed.iter().any(|c| c == f)).collect();
        if !in_way.is_empty() {
            return Err(tf!("Your uncommitted changes to {} are in files this edit changes. Commit or stash them first.", in_way.iter().take(6).copied().collect::<Vec<_>>().join(", ")));
        }
    }
    if plan.op == Op::Amend && now.index_tree.as_deref() != Some(&info(dir, &plan.new_tip)?.tree) {
        return Err(tf!("The staged changes changed since the simulation. {again}"));
    }
    let mut warnings = Vec::new();
    if let Some((remote, r)) = &was.remote {
        // No prompt may wait for a password here: a remote that needs one counts as unreachable.
        let reached = gh::run_full("git", &["ls-remote", remote, r], Some(dir), None, &[]).and_then(|o| if o.ok { Ok(o.stdout) } else { Err(o.stderr) });
        match reached {
            Ok(out) => {
                let theirs = out.split_whitespace().next().map(str::to_owned);
                if theirs != was.upstream_sha {
                    let up = was.upstream.clone().unwrap_or_default();
                    return Err(match theirs {
                        None => tf!("{r} is gone from {remote}. Fetch, then simulate again."),
                        Some(t) => tf!("{remote} has {} on {r}, but your {up} is at {}: someone pushed. Fetch, then simulate again.", short(&t), was.upstream_sha.as_deref().map_or("(none)".into(), short)),
                    });
                }
            }
            Err(e) if plan.pushed_rewritten > 0 => return Err(tf!("Couldn't reach {remote} to check for new commits ({e}). This edit rewrites pushed commits, so it waits until the check works.")),
            Err(e) => warnings.push(tf!("Couldn't reach {remote} to check for new commits: {e}")),
        }
    }
    Ok(warnings)
}

/// Moves `r` from `old` to `new` only if it's still at `old`, then the index and the files.
/// If the files can't follow, the ref goes back.
fn move_ref(dir: &Path, r: &str, old: &str, new: &str, sync: Sync, why: &str) -> Result<(), String> {
    update_ref_cmd(r, new, old, why).run(Some(dir)).map_err(|e| tf!("The branch moved meanwhile, so nothing was changed ({e})"))?;
    if let Some(c) = sync_cmd(sync, old, new)
        && let Err(e) = c.run(Some(dir))
    {
        let back = update_ref_cmd(r, old, new, &format!("{why} (rolled back)")).run(Some(dir));
        return Err(match back {
            Ok(_) => tf!("Your files couldn't follow ({e}), so the branch was put back. Nothing was changed."),
            Err(b) => tf!("Your files couldn't follow ({e}), and putting the branch back failed too ({b}). Run: git update-ref {r} {old}"),
        });
    }
    Ok(())
}

/// The preflight, then the transaction. Uses the simulated tip; nothing is made again.
pub fn apply(dir: &Path, plan: &Plan) -> Result<String, String> {
    preflight(dir, plan)?;
    move_ref(dir, &plan.branch_ref(), &plan.snap.tip, &plan.new_tip, plan.sync, &plan.op.label())?;
    let mut msg = tf!("{}: {} is now at {}", plan.op.label(), plan.snap.branch, short(&plan.new_tip));
    if let Some(cmd) = plan.push_hint() {
        msg.push_str(&tf!(". To share it: {cmd}"));
    }
    Ok(msg)
}

/// Undo an edit: the branch back from `from` to `to`, only if it's still at `from` and checked
/// out. A hard reset's files come back with a two-tree merge, so nothing new is overwritten.
pub fn undo(dir: &Path, branch: &str, from: &str, to: &str, sync: Sync) -> Result<String, String> {
    let now = snapshot(dir)?;
    if now.branch != branch {
        return Err(tf!("You're on {} now, not {branch}: switch back to undo", now.branch));
    }
    let sync = if sync == Sync::Hard { Sync::TwoTree } else { sync };
    move_ref(dir, &format!("refs/heads/{branch}"), from, to, sync, "undo")?;
    Ok(tf!("{branch} is back at {}", short(to)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Repo(PathBuf);

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    impl Repo {
        fn new(name: &str) -> Repo {
            let dir = std::env::temp_dir().join(format!("sigit-history-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let r = Repo(dir);
            r.git(&["init", "-q", "-b", "main"]);
            r.git(&["config", "user.name", "Test"]);
            r.git(&["config", "user.email", "t@example.com"]);
            r.git(&["config", "commit.gpgsign", "false"]);
            r
        }
        fn git(&self, args: &[&str]) -> String {
            gh::run("git", args, Some(&self.0)).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
        }
        fn write(&self, file: &str, body: &str) {
            std::fs::write(self.0.join(file), body).unwrap();
        }
        fn read(&self, file: &str) -> String {
            std::fs::read_to_string(self.0.join(file)).unwrap_or_default()
        }
        fn commit(&self, file: &str, body: &str, msg: &str) -> String {
            self.write(file, body);
            self.git(&["add", file]);
            self.git(&["commit", "-q", "-m", msg]);
            self.head()
        }
        fn head(&self) -> String {
            self.git(&["rev-parse", "HEAD"]).trim().to_owned()
        }
        fn subjects(&self) -> Vec<String> {
            self.git(&["log", "--format=%s"]).lines().map(str::to_owned).collect()
        }
        fn status(&self) -> String {
            self.git(&["status", "--porcelain"])
        }
        fn run(&self, op: Op) -> Plan {
            let plan = simulate(&self.0, &op).unwrap_or_else(|e| panic!("simulate {op:?}: {e}"));
            apply(&self.0, &plan).unwrap_or_else(|e| panic!("apply {op:?}: {e}"));
            plan
        }
        /// a.txt over three commits, b.txt in its own.
        fn abc(name: &str) -> (Repo, [String; 4]) {
            let r = Repo::new(name);
            let a = r.commit("a.txt", "1\n", "one");
            let b = r.commit("a.txt", "1\n2\n", "two");
            let c = r.commit("b.txt", "b\n", "three");
            let d = r.commit("a.txt", "1\n2\n3\n", "four");
            (r, [a, b, c, d])
        }
    }

    #[test]
    fn version_parses() {
        assert_eq!(parse_version("git version 2.43.0"), Some((2, 43)));
        assert_eq!(parse_version("git version 2.39.3 (Apple Git-146)"), Some((2, 39)));
        assert!(parse_version("git version 2.39.3").unwrap() < MIN_GIT);
    }

    #[test]
    fn simulate_changes_nothing() {
        let (r, [_, _, _, d]) = Repo::abc("sim");
        r.write("a.txt", "dirty\n");
        let plan = simulate(&r.0, &Op::Drop { sha: d.clone() }).unwrap();
        assert_ne!(plan.new_tip, d);
        assert_eq!(r.head(), d, "the branch didn't move");
        assert_eq!(r.read("a.txt"), "dirty\n", "the files didn't change");
        assert_eq!(plan.removed.len(), 1);
        assert_eq!(plan.added.len(), 0);
        assert!(plan.files.iter().any(|f| f.path == "a.txt" && f.del == 1));
        assert_eq!(plan.rows.iter().map(|r| r.kind).collect::<Vec<_>>(), [Kind::Leaves]);
    }

    #[test]
    fn reset_modes() {
        let (r, [a, b, _, d]) = Repo::abc("reset");
        let plan = r.run(Op::Reset { to: b.clone(), mode: ResetMode::Soft });
        assert_eq!(r.head(), b);
        assert_eq!(plan.removed.len(), 2);
        assert!(r.status().contains("A  b.txt"), "soft keeps the changes staged: {}", r.status());
        r.git(&["reset", "-q", "--hard", &d]);
        r.run(Op::Reset { to: b.clone(), mode: ResetMode::Mixed });
        assert!(r.status().contains("?? b.txt"), "mixed un-stages: {}", r.status());
        r.git(&["reset", "-q", "--hard", &d]);
        r.run(Op::Reset { to: a.clone(), mode: ResetMode::Hard });
        assert_eq!(r.read("a.txt"), "1\n");
        assert_eq!(r.status(), "");
    }

    #[test]
    fn revert_adds_a_commit() {
        let (r, [_, b, _, d]) = Repo::abc("revert");
        // Reverting "two" while "four" builds on it conflicts; revert "three" instead.
        assert!(simulate(&r.0, &Op::Revert { sha: b }).unwrap_err().contains("conflict"));
        let c = r.git(&["rev-parse", "HEAD~1"]).trim().to_owned();
        let plan = r.run(Op::Revert { sha: c.clone() });
        assert_eq!(plan.removed.len(), 0);
        assert_eq!(plan.added.len(), 1);
        assert_eq!(r.git(&["rev-parse", "HEAD~1"]).trim(), d);
        assert!(!r.0.join("b.txt").exists());
        assert!(r.subjects()[0].starts_with("Revert \"three\""));
        assert_eq!(r.git(&["log", "-1", "--format=%an"]).trim(), "Test");
    }

    #[test]
    fn cherry_pick_from_another_branch() {
        let r = Repo::new("pick");
        r.commit("a.txt", "1\n", "one");
        r.git(&["switch", "-q", "-c", "side"]);
        let s = r.commit("s.txt", "side\n", "side work");
        r.git(&["switch", "-q", "main"]);
        r.commit("m.txt", "main\n", "main work");
        assert!(simulate(&r.0, &Op::CherryPick { sha: r.head() }).unwrap_err().contains("already on"));
        r.run(Op::CherryPick { sha: s });
        assert_eq!(r.subjects(), ["side work", "main work", "one"]);
        assert_eq!(r.read("s.txt"), "side\n");
        assert_eq!(r.status(), "");
    }

    #[test]
    fn reword_keeps_older_commits_and_trees() {
        let (r, [a, b, _, d]) = Repo::abc("reword");
        let tree = r.git(&["rev-parse", "HEAD^{tree}"]);
        let plan = r.run(Op::Reword { sha: b, message: "two, better\n\nbody".into() });
        assert_eq!(r.subjects(), ["four", "three", "two, better", "one"]);
        assert_eq!(r.git(&["rev-parse", "HEAD~3"]).trim(), a, "the commit before is kept as it is");
        assert_eq!(r.git(&["rev-parse", "HEAD^{tree}"]), tree, "the files are the same");
        assert_eq!(plan.removed.len(), 3);
        assert_ne!(r.head(), d);
        assert_eq!(r.git(&["log", "-1", "--format=%B", "HEAD~2"]).trim(), "two, better\n\nbody");
    }

    #[test]
    fn drop_squash_fixup_and_move() {
        let (r, [_, _, c, _]) = Repo::abc("drop");
        r.run(Op::Drop { sha: c });
        assert_eq!(r.subjects(), ["four", "two", "one"]);
        assert!(!r.0.join("b.txt").exists());

        let (r, [_, _, c, _]) = Repo::abc("squash");
        r.run(Op::Squash { sha: c, fixup: false });
        assert_eq!(r.subjects(), ["four", "two", "one"]);
        assert_eq!(r.git(&["log", "-1", "--format=%B", "HEAD~1"]).trim(), "two\n\nthree");
        assert_eq!(r.read("a.txt"), "1\n2\n3\n");
        assert_eq!(r.read("b.txt"), "b\n");

        let (r, [_, _, c, _]) = Repo::abc("fixup");
        r.run(Op::Squash { sha: c, fixup: true });
        assert_eq!(r.subjects(), ["four", "two", "one"]);
        assert_eq!(r.read("b.txt"), "b\n");

        let (r, [_, _, c, _]) = Repo::abc("move");
        r.run(Op::Move { sha: c.clone(), up: true });
        assert_eq!(r.subjects(), ["three", "four", "two", "one"]);
        r.run(Op::Move { sha: r.head(), up: false });
        assert_eq!(r.subjects(), ["four", "three", "two", "one"]);
        assert_eq!(r.status(), "");
    }

    #[test]
    fn refuses_root_merges_and_conflicts() {
        let (r, [a, b, _, _]) = Repo::abc("refuse");
        assert!(simulate(&r.0, &Op::Drop { sha: a.clone() }).unwrap_err().contains("first commit"));
        assert!(simulate(&r.0, &Op::Squash { sha: b.clone(), fixup: false }).unwrap_err().contains("first commit"));
        // Dropping "two" makes "four" conflict (it changes the line "two" added).
        assert!(simulate(&r.0, &Op::Drop { sha: b }).unwrap_err().contains("conflict"));
        r.git(&["switch", "-q", "-c", "side", &a]);
        r.commit("s.txt", "s\n", "side");
        r.git(&["switch", "-q", "main"]);
        r.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
        let c = r.git(&["rev-parse", "HEAD~1"]).trim().to_owned();
        assert!(simulate(&r.0, &Op::Drop { sha: c }).unwrap_err().contains("merge commit"));
        // A merge can be reverted (against parent 1).
        r.run(Op::Revert { sha: r.head() });
        assert!(!r.0.join("s.txt").exists());
    }

    #[test]
    fn amend_puts_staged_changes_in() {
        let (r, [_, _, c, d]) = Repo::abc("amend");
        assert!(simulate(&r.0, &Op::Amend).unwrap_err().contains("Nothing is staged"));
        r.write("a.txt", "1\n2\n3\n4\n");
        r.git(&["add", "a.txt"]);
        r.write("b.txt", "unstaged\n");
        let plan = r.run(Op::Amend);
        assert_eq!(plan.removed[0].sha, short(&d));
        assert_eq!(r.git(&["rev-parse", "HEAD~1"]).trim(), c);
        assert_eq!(r.subjects()[0], "four");
        assert_eq!(r.git(&["show", "HEAD:a.txt"]), "1\n2\n3\n4\n");
        assert_eq!(r.read("b.txt"), "unstaged\n", "unstaged files stay as they are");
        assert_eq!(r.status(), " M b.txt\n");
    }

    #[test]
    fn preflight_stops_when_the_branch_moved() {
        let (r, [_, b, _, _]) = Repo::abc("preflight");
        let plan = simulate(&r.0, &Op::Reword { sha: b, message: "x".into() }).unwrap();
        r.commit("c.txt", "c\n", "five");
        let moved = r.head();
        let e = apply(&r.0, &plan).unwrap_err();
        assert!(e.contains("moved since the simulation"), "{e}");
        assert_eq!(r.head(), moved, "nothing changed");
        // The compare-and-swap refuses on its own too.
        let e = move_ref(&r.0, "refs/heads/main", &plan.snap.tip, &plan.new_tip, Sync::None, "t").unwrap_err();
        assert!(e.contains("moved meanwhile"), "{e}");
        assert_eq!(r.head(), moved);
    }

    #[test]
    fn preflight_checks_the_remote() {
        let up = Repo::new("remote-up");
        up.commit("a.txt", "1\n", "one");
        let r = Repo(std::env::temp_dir().join(format!("sigit-history-remote-clone-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&r.0);
        gh::run("git", &["clone", "-q", up.0.to_str().unwrap(), r.0.to_str().unwrap()], None).unwrap();
        r.git(&["config", "user.name", "Test"]);
        r.git(&["config", "user.email", "t@example.com"]);
        r.commit("b.txt", "b\n", "local");
        // Rewriting the pushed "one" counts as pushed.
        let pushed = simulate(&r.0, &Op::Reword { sha: r.head(), message: "local!".into() }).unwrap();
        assert_eq!(pushed.pushed_rewritten, 0, "only the unpushed commit is rewritten");
        let one = r.git(&["rev-parse", "HEAD~1"]).trim().to_owned();
        let plan = simulate(&r.0, &Op::Reset { to: one.clone(), mode: ResetMode::Soft }).unwrap();
        assert_eq!(plan.pushed_rewritten, 0);
        assert!(plan.commands().iter().any(|c| c.line().starts_with("git ls-remote origin refs/heads/main")));
        // Someone pushes to the remote: the preflight sees it without a fetch.
        up.commit("a.txt", "1\n2\n", "theirs");
        let e = apply(&r.0, &plan).unwrap_err();
        assert!(e.contains("someone pushed"), "{e}");
        assert_eq!(r.subjects(), ["local", "one"]);
    }

    #[test]
    fn rollback_when_files_cant_follow_and_undo() {
        let (r, [_, _, c, d]) = Repo::abc("rollback");
        let plan = simulate(&r.0, &Op::Drop { sha: c.clone() }).unwrap();
        // A local change to b.txt is in the way of deleting it: the preflight says so.
        r.write("b.txt", "mine\n");
        let e = apply(&r.0, &plan).unwrap_err();
        assert!(e.contains("b.txt") && e.contains("Commit or stash"), "{e}");
        // If the files still can't follow after the ref moved, the ref goes back.
        let e = move_ref(&r.0, "refs/heads/main", &plan.snap.tip, &plan.new_tip, Sync::TwoTree, "t").unwrap_err();
        assert!(e.contains("put back"), "{e}");
        assert_eq!(r.head(), d);
        assert_eq!(r.read("b.txt"), "mine\n");
        r.git(&["checkout", "--", "b.txt"]);
        let plan = r.run(Op::Drop { sha: c });
        let msg = undo(&r.0, "main", &plan.new_tip, &plan.snap.tip, plan.sync).unwrap();
        assert!(msg.contains("back at"));
        assert_eq!(r.head(), d);
        assert_eq!(r.read("b.txt"), "b\n");
        assert_eq!(r.status(), "");
        // Undo again refuses: the branch isn't where the edit left it.
        assert!(undo(&r.0, "main", &plan.new_tip, &plan.snap.tip, plan.sync).is_err());
    }

    #[test]
    fn graph_rows_pair_old_and_new() {
        let kinds = |p: &Plan| p.rows.iter().map(|r| r.kind).collect::<Vec<_>>();
        let (r, [_, b, c, _]) = Repo::abc("rows");
        // Reword "two": it and the two after it are made again, one row each.
        let p = simulate(&r.0, &Op::Reword { sha: b.clone(), message: "two!".into() }).unwrap();
        assert_eq!(kinds(&p), [Kind::Rewritten; 3]);
        assert_eq!(p.rows[2].old.as_ref().unwrap().subject, "two");
        assert_eq!(p.rows[2].new.as_ref().unwrap().subject, "two!");
        // Squash "three" into "two": two rows land on one new commit.
        let p = simulate(&r.0, &Op::Squash { sha: c.clone(), fixup: false }).unwrap();
        assert_eq!(kinds(&p), [Kind::Rewritten, Kind::Rewritten, Kind::Joined]);
        assert_eq!(p.rows[1].new, p.rows[2].new);
        // Drop "three": it only leaves.
        let p = simulate(&r.0, &Op::Drop { sha: c }).unwrap();
        assert_eq!(kinds(&p), [Kind::Rewritten, Kind::Leaves]);
        // Revert: one new commit.
        let p = simulate(&r.0, &Op::Revert { sha: r.git(&["rev-parse", "HEAD~1"]).trim().to_owned() }).unwrap();
        assert_eq!(kinds(&p), [Kind::New]);
    }

    #[test]
    fn uncommitted_changes_elsewhere_stay() {
        let (r, [_, _, c, _]) = Repo::abc("dirty");
        r.write("notes.txt", "mine\n");
        r.write("a.txt", "1\n2\n3\nmore\n");
        r.run(Op::Drop { sha: c });
        assert_eq!(r.subjects(), ["four", "two", "one"]);
        assert_eq!(r.read("a.txt"), "1\n2\n3\nmore\n");
        assert_eq!(r.read("notes.txt"), "mine\n");
    }

    #[test]
    fn pushed_commits_count_on_any_remote() {
        let up = Repo::new("pushed-up");
        up.commit("a.txt", "1\n", "one");
        up.commit("a.txt", "1\n2\n", "two");
        let r = Repo(std::env::temp_dir().join(format!("sigit-history-pushed-clone-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&r.0);
        gh::run("git", &["clone", "-q", up.0.to_str().unwrap(), r.0.to_str().unwrap()], None).unwrap();
        r.git(&["config", "user.name", "Test"]);
        r.git(&["config", "user.email", "t@example.com"]);
        // On the upstream: rewriting "two" needs a force-push.
        let plan = simulate(&r.0, &Op::Reword { sha: r.head(), message: "two!".into() }).unwrap();
        assert_eq!(plan.pushed_rewritten, 1);
        assert_eq!(plan.push_hint().unwrap(), "git push --force-with-lease origin main:main");
        // A branch with no upstream that holds origin/main's commits: still counted.
        r.git(&["switch", "-q", "--no-track", "-c", "feat", "origin/main"]);
        let plan = simulate(&r.0, &Op::Reword { sha: r.head(), message: "two!".into() }).unwrap();
        assert_eq!(plan.pushed_rewritten, 1);
        assert!(plan.snap.remote.is_none() && plan.push_hint().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn keeps_signing_commits() {
        use std::os::unix::fs::PermissionsExt;
        let (r, [_, b, _, _]) = Repo::abc("sign");
        // A stand-in for gpg that "signs" anything.
        let gpg = r.0.join("fake-gpg.sh");
        std::fs::write(&gpg, "#!/bin/sh\ncat >/dev/null\necho '[GNUPG:] SIG_CREATED D 1 8 00 1 X' >&2\nprintf -- '-----BEGIN PGP SIGNATURE-----\\n\\nfake\\n-----END PGP SIGNATURE-----\\n'\n").unwrap();
        std::fs::set_permissions(&gpg, std::fs::Permissions::from_mode(0o755)).unwrap();
        r.git(&["config", "gpg.program", gpg.to_str().unwrap()]);
        r.git(&["config", "user.signingkey", "X"]);
        r.git(&["config", "commit.gpgsign", "true"]);
        r.run(Op::Reword { sha: b, message: "two, signed".into() });
        assert!(r.git(&["cat-file", "commit", "HEAD"]).contains("gpgsig"), "the new tip is signed");
    }

    #[test]
    fn command_lines() {
        let snap = Snapshot { branch: "main".into(), tip: "a".repeat(40), remote: Some(("origin".into(), "refs/heads/main".into())), ..Default::default() };
        let plan = Plan { op: Op::Drop { sha: "c".repeat(40) }, snap, new_tip: "b".repeat(40), removed: vec![], added: vec![], rows: vec![], files: vec![], pushed_rewritten: 2, replaced_files: vec![], sync: Sync::TwoTree };
        let lines: Vec<String> = plan.commands().iter().map(Cmd::line).collect();
        let (a, b) = ("a".repeat(40), "b".repeat(40));
        assert_eq!(lines, [
            "git ls-remote origin refs/heads/main".to_owned(),
            format!("git update-ref -m \"sigit: Drop ccccccc\" refs/heads/main {b} {a}"),
            format!("git read-tree -m -u {a} {b}"),
        ]);
        assert_eq!(plan.push_hint().unwrap(), "git push --force-with-lease origin main:main");
    }
}
