//! The sandbox: a private copy of a repo's local clone where you can practise history edits on
//! your real commits. Nothing you do there changes the real clone or GitHub.
//!
//! - It is a `git clone --local` into sigit's settings folder (`<config>/sandboxes/owner__name`).
//!   The object files are hard links (git objects never change), so it costs almost no disk;
//!   the refs, the index and the files are its own. (`git worktree` would share the branch refs,
//!   and `--shared` would break when the real clone runs `git gc`.)
//! - `origin` is the real remote again, with the real `origin/*` refs, so "pushed" and
//!   "ahead / behind" read as they do in the real clone. Its push URL is dead: a push fails.
//! - Only the commits are copied: the sandbox starts with a clean checkout of the branch.
//! - `.git/sigit-sandbox` marks it (what it was made from); sigit only deletes a folder that has
//!   that mark and is under the sandbox folder, and the folder scan skips marked clones.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The mark's file name, in the sandbox's `.git` folder.
pub const MARK: &str = "sigit-sandbox";
/// A push URL git can't use: `git push` from a sandbox always fails.
pub const NO_PUSH: &str = "sigit-sandbox://pushing-is-off-in-the-sandbox";

/// What a sandbox was made from (the mark's content).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub slug: String,
    /// The real clone.
    pub real: PathBuf,
    /// The branch it copies (the real clone's branch when it was made).
    pub branch: String,
    /// That branch's tip when it was made.
    pub base: String,
    /// When (Unix seconds).
    pub made: i64,
}

/// A sandbox as the Sandbox page shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub info: Option<Info>,
    /// The sandbox's branch and tip now.
    pub branch: String,
    pub tip: String,
    /// Commits on the sandbox's branch that `base` doesn't have (new or made again), and
    /// commits of `base` that the branch no longer has.
    pub new: usize,
    pub gone: usize,
    /// The real branch's tip now (empty when it's gone).
    pub real_tip: String,
    /// The real clone isn't on `info.branch` any more.
    pub real_branch: String,
    /// Bytes the sandbox adds on disk (hard-linked object files not counted).
    pub size: u64,
}

impl View {
    /// The real branch has new commits (or moved) since the sandbox was made.
    pub fn real_moved(&self) -> bool {
        self.info.as_ref().is_some_and(|i| !self.real_tip.is_empty() && self.real_tip != i.base)
    }
}

/// Where sandboxes live: `<settings folder>/sandboxes`.
pub fn home() -> Option<PathBuf> {
    // Tests (and headless runs) can put sandboxes elsewhere.
    if let Some(dir) = std::env::var_os("SIGIT_SANDBOX_HOME") {
        return Some(PathBuf::from(dir));
    }
    crate::config::dir().map(|d| d.join("sandboxes"))
}

/// `owner/name`'s sandbox folder under `home`.
pub fn dir_for(home: &Path, slug: &str) -> Option<PathBuf> {
    let (owner, name) = slug.split_once('/')?;
    let ok = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.contains(['/', '\\']);
    (ok(owner) && ok(name)).then(|| home.join(format!("{owner}__{name}")))
}

/// The mark of the sandbox at `dir`, when there is one.
pub fn info(dir: &Path) -> Option<Info> {
    let text = std::fs::read_to_string(dir.join(".git").join(MARK)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Is `dir` a sandbox sigit made? (The folder scan skips these.)
pub fn is_sandbox(dir: &Path) -> bool {
    dir.join(".git").join(MARK).is_file()
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    crate::gh::unrecorded(|| crate::gh::run("git", args, Some(dir)))
}

fn git_ok(dir: &Path, args: &[&str]) -> Option<String> {
    git(dir, args).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// Make (or make again) `slug`'s sandbox under `home` from the real clone `real`, on the branch
/// the real clone has checked out.
pub fn create(home: &Path, slug: &str, real: &Path) -> Result<Info, String> {
    let dir = dir_for(home, slug).ok_or_else(|| tf!("\"{slug}\" isn't owner/name"))?;
    let branch = git_ok(real, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok_or_else(|| crate::i18n::t("The real clone isn't on a branch (detached HEAD). Check out a branch first."))?;
    let base = git_ok(real, &["rev-parse", "--verify", "HEAD^{commit}"]).ok_or_else(|| tf!("{branch} has no commits yet"))?;
    if dir.exists() {
        remove(home, &dir)?;
    }
    std::fs::create_dir_all(home).map_err(|e| tf!("Couldn't make {}: {e}", home.display()))?;
    let made = make(&dir, real, &branch, &base, slug);
    if made.is_err() {
        // Only what this call just made is there: take it away.
        let _ = force_remove(&dir);
    }
    made
}

fn make(dir: &Path, real: &Path, branch: &str, base: &str, slug: &str) -> Result<Info, String> {
    let real_s = real.to_string_lossy().into_owned();
    let dir_s = dir.to_string_lossy().into_owned();
    crate::gh::unrecorded(|| crate::gh::run("git", &["clone", "--quiet", "--local", "--no-checkout", &real_s, &dir_s], None))?;
    // Mark it at once: a sandbox left half made (sigit quit meanwhile) is still ours to replace.
    let info = Info { slug: slug.to_owned(), real: real.to_path_buf(), branch: branch.to_owned(), base: base.to_owned(), made: crate::model::now() };
    let text = serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(".git").join(MARK), text).map_err(|e| tf!("Couldn't mark the sandbox: {e}"))?;
    // `origin` is the real folder now. Put back every real remote (origin, upstream…) with its
    // URL and a dead push URL, then copy the real remote-tracking refs, the real local branches
    // (cherry-pick sources) and tags from the real folder (no new objects move).
    git(dir, &["remote", "remove", "origin"])?;
    let remotes: Vec<String> = git(real, &["remote"]).unwrap_or_default().lines().map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned).collect();
    for r in &remotes {
        if let Some(url) = git_ok(real, &["remote", "get-url", r]) {
            git(dir, &["remote", "add", "--no-fetch", r, &url])?;
            git(dir, &["config", &format!("remote.{r}.pushurl"), NO_PUSH])?;
        }
    }
    git(dir, &["fetch", "--quiet", "--no-tags", "--update-head-ok", &real_s, "+refs/heads/*:refs/heads/*", "+refs/remotes/*:refs/remotes/*", "+refs/tags/*:refs/tags/*"])?;
    // `refs/remotes/<r>/HEAD` is symbolic in the real clone; set it the same way.
    for r in &remotes {
        if let Some(head) = git_ok(real, &["symbolic-ref", "--quiet", &format!("refs/remotes/{r}/HEAD")]) {
            let _ = git(dir, &["symbolic-ref", &format!("refs/remotes/{r}/HEAD"), &head]);
        }
    }
    // The branch's upstream (when its remote came along), and who you are (the edits make
    // commits). Sandbox commits are never signed: they never leave this folder.
    let upstream = git_ok(real, &["config", "--get", &format!("branch.{branch}.remote")]);
    if let Some(remote) = upstream.filter(|r| remotes.contains(r)) {
        git(dir, &["config", &format!("branch.{branch}.remote"), &remote])?;
        if let Some(merge) = git_ok(real, &["config", "--get", &format!("branch.{branch}.merge")]) {
            git(dir, &["config", &format!("branch.{branch}.merge"), &merge])?;
        }
    }
    for key in ["user.name", "user.email"] {
        if let Some(v) = git_ok(real, &["config", "--get", key]) {
            git(dir, &["config", key, &v])?;
        }
    }
    git(dir, &["config", "commit.gpgsign", "false"])?;
    git(dir, &["checkout", "--quiet", "--force", branch])?;
    Ok(info)
}

/// Delete the sandbox at `dir`: only a marked sandbox directly under `home`.
pub fn remove(home: &Path, dir: &Path) -> Result<(), String> {
    let inside = dir.parent().is_some_and(|p| p == home || p.canonicalize().ok() == home.canonicalize().ok());
    if !inside || !is_sandbox(dir) {
        return Err(tf!("{} isn't a sandbox sigit made: it's left alone", dir.display()));
    }
    force_remove(dir)
}

/// Remove a folder, clearing read-only flags first if needed (git object files are read-only,
/// which stops a delete on Windows).
fn force_remove(dir: &Path) -> Result<(), String> {
    if !dir.exists() || std::fs::remove_dir_all(dir).is_ok() {
        return Ok(());
    }
    fn writable(p: &Path) {
        if let Ok(m) = std::fs::symlink_metadata(p) {
            if m.is_dir() {
                for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                    writable(&e.path());
                }
            }
            let mut perm = m.permissions();
            if perm.readonly() && !m.file_type().is_symlink() {
                #[allow(clippy::permissions_set_readonly_false)]
                perm.set_readonly(false);
                let _ = std::fs::set_permissions(p, perm);
            }
        }
    }
    // Only Windows refuses to delete read-only files; elsewhere leave the (shared) object
    // files' modes alone.
    if cfg!(windows) {
        writable(dir);
    }
    std::fs::remove_dir_all(dir).map_err(|e| tf!("Couldn't delete {}: {e}", dir.display()))
}

/// Read the sandbox at `dir` for the Sandbox page (`None` when there is none).
pub fn view(dir: &Path) -> Option<View> {
    let info = info(dir)?;
    let branch = git_ok(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_default();
    let tip = git_ok(dir, &["rev-parse", "--verify", "HEAD^{commit}"]).unwrap_or_default();
    let count = |range: String| git_ok(dir, &["rev-list", "--count", &range]).and_then(|n| n.parse().ok()).unwrap_or(0);
    let (new, gone) = if tip.is_empty() { (0, 0) } else { (count(format!("{}..{tip}", info.base)), count(format!("{tip}..{}", info.base))) };
    let real_tip = git_ok(&info.real, &["rev-parse", "--verify", &format!("refs/heads/{}^{{commit}}", info.branch)]).unwrap_or_default();
    let real_branch = git_ok(&info.real, &["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_default();
    Some(View { info: Some(info), branch, tip, new, gone, real_tip, real_branch, size: size(dir) })
}

/// Bytes under `dir`, not counting files that are hard links (shared with the real clone).
fn size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in std::fs::read_dir(&p).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else if !shared(&m) {
                total += m.len();
            }
        }
    }
    total
}

#[cfg(unix)]
fn shared(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    m.nlink() > 1
}

#[cfg(not(unix))]
fn shared(_: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) -> String {
        crate::gh::run("git", args, Some(dir)).unwrap()
    }

    /// A bare "GitHub", a real clone with a pushed and an unpushed commit, a second branch.
    fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("sigit-sandbox-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let remote = root.join("remote.git");
        run(&root, &["init", "-q", "--bare", &remote.to_string_lossy()]);
        let real = root.join("real");
        run(&root, &["init", "-q", "-b", "main", &real.to_string_lossy()]);
        run(&real, &["config", "user.name", "T"]);
        run(&real, &["config", "user.email", "t@example.com"]);
        run(&real, &["remote", "add", "origin", &remote.to_string_lossy()]);
        for i in 1..=3 {
            std::fs::write(real.join(format!("f{i}")), "x").unwrap();
            run(&real, &["add", "."]);
            run(&real, &["commit", "-qm", &format!("c{i}")]);
        }
        run(&real, &["push", "-q", "-u", "origin", "main"]);
        std::fs::write(real.join("f4"), "x").unwrap();
        run(&real, &["add", "."]);
        run(&real, &["commit", "-qm", "local4"]);
        run(&real, &["branch", "feat", "HEAD~1"]);
        (root.join("home"), real, root)
    }

    /// Everything in the real clone a sandbox must never change.
    fn record(real: &Path) -> String {
        [run(real, &["for-each-ref"]), run(real, &["rev-parse", "HEAD"]), run(real, &["status", "--porcelain"]), run(real, &["stash", "list"])].join("\n")
    }

    #[test]
    fn a_sandbox_copies_the_branch_and_its_upstream_and_cannot_push() {
        let (home, real, root) = setup("make");
        let before = record(&real);
        let info = create(&home, "acme/app", &real).unwrap();
        let dir = dir_for(&home, "acme/app").unwrap();
        assert_eq!(dir, home.join("acme__app"));
        assert_eq!(info.branch, "main");
        assert_eq!(info.base, run(&real, &["rev-parse", "HEAD"]).trim());
        assert!(is_sandbox(&dir) && crate::scan::scan(&home).found.is_empty(), "the folder scan skips sandboxes");
        // The real upstream, not the real clone's local branches, is `origin/main`.
        assert_eq!(run(&dir, &["rev-parse", "origin/main"]), run(&real, &["rev-parse", "origin/main"]));
        assert_eq!(run(&dir, &["rev-list", "--count", "origin/main..main"]).trim(), "1", "one commit not pushed, as in the real clone");
        assert!(run(&dir, &["rev-parse", "--verify", "feat"]).len() >= 40, "other local branches come too");
        assert_eq!(run(&dir, &["config", "user.email"]).trim(), "t@example.com");
        // Pushing fails, whatever you ask for.
        assert!(crate::gh::run("git", &["push", "origin", "main"], Some(&dir)).is_err());
        assert!(crate::gh::run("git", &["push"], Some(&dir)).is_err());
        // Edit and commit freely: the real clone doesn't change.
        run(&dir, &["reset", "-q", "--hard", "HEAD~2"]);
        std::fs::write(dir.join("new"), "x").unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "-qm", "practice"]);
        run(&dir, &["stash", "list"]);
        assert_eq!(record(&real), before, "the real clone is untouched");
        let v = view(&dir).unwrap();
        assert_eq!((v.new, v.gone), (1, 2));
        assert!(!v.real_moved());
        // The real branch moves: the page says so.
        std::fs::write(real.join("f5"), "x").unwrap();
        run(&real, &["add", "."]);
        run(&real, &["commit", "-qm", "real5"]);
        assert!(view(&dir).unwrap().real_moved());
        // Making it again starts over from the real branch.
        let again = create(&home, "acme/app", &real).unwrap();
        assert_eq!(again.base, run(&real, &["rev-parse", "HEAD"]).trim());
        assert_eq!(view(&dir).unwrap().new, 0);
        remove(&home, &dir).unwrap();
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// The promise, end to end through the app: with the sandbox on, history edits and Undo run
    /// in the sandbox, GitHub actions and actions for the real folder are refused, and the real
    /// clone ends exactly as it began.
    #[test]
    fn the_app_edits_only_the_sandbox() {
        use crate::guard::Action;
        use crate::history::{Op, ResetMode};
        let (home, real, root) = setup("app");
        // Only this test reads it (the others pass `home` themselves).
        unsafe { std::env::set_var("SIGIT_SANDBOX_HOME", &home) };
        let before = record(&real);
        let mut s = crate::State::new(crate::config::Prefs::default());
        s.add_repo_pub("acme/app".into(), Some(real.clone()));
        s.cur = Some("acme/app".into());
        s.page = crate::Page::Sandbox;
        // A GitHub dialog opened before the sandbox comes on can't be confirmed after.
        s.request(Action::Close { slug: "acme/app".into(), number: 1, title: "x".into() });
        assert!(s.pending.is_some());
        s.start_sandbox();
        assert!(s.pending.is_none(), "switching closes it");
        let dir = dir_for(&home, "acme/app").unwrap();
        assert_eq!(s.local_path(), Some(dir.clone()), "the local pages work on the sandbox");
        assert!(s.in_sandbox());
        let tip = |d: &Path| run(d, &["rev-parse", "HEAD"]).trim().to_owned();
        let start = tip(&dir);
        let parent = run(&dir, &["rev-parse", "HEAD~1"]).trim().to_owned();

        // A revert, then a hard reset (it needs the typed branch name), each through the dialog.
        for op in [Op::Revert { sha: parent.clone() }, Op::Reset { to: parent.clone(), mode: ResetMode::Hard }] {
            s.toasts.clear();
            s.simulate_history(op);
            let p = s.pending.as_ref().expect("the dialog opens");
            assert!(p.blocked.is_none(), "{:?}", p.blocked);
            if let Some(w) = p.word.clone() {
                s.typed = crate::TextEdit::new(&w);
            }
            let moved_from = tip(&dir);
            s.confirm(false);
            assert_ne!(tip(&dir), moved_from, "the sandbox branch moved");
            assert_eq!(record(&real), before, "the real clone didn't");
            let id = s.toasts.iter().find(|t| t.undo.is_some()).expect("an Undo toast").id;
            s.run_undo(id);
            assert_eq!(tip(&dir), moved_from, "Undo puts the sandbox back");
        }
        assert_eq!(tip(&dir), start);

        // Nothing on GitHub, and nothing for the real folder, while the sandbox is on.
        s.request(Action::Close { slug: "acme/app".into(), number: 1, title: "x".into() });
        assert!(s.pending.is_none(), "closing a PR is refused");
        s.request(Action::Checkout { dir: real.clone(), slug: "acme/app".into(), name: "feat".into(), remote: false });
        assert!(s.pending.is_none(), "an action for the real folder is refused");
        assert!(s.sandbox_block(&Action::Close { slug: "other/repo".into(), number: 1, title: "x".into() }).is_none(), "other repos aren't in the sandbox");

        // Leave: the pages are back on the real clone; delete: the folder goes, the clone stays.
        s.set_sandbox(false);
        assert_eq!(s.local_path(), Some(real.clone()));
        s.delete_sandbox();
        assert!(!dir.exists() && real.exists());
        assert_eq!(record(&real), before, "the real clone is exactly as it began");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_half_made_sandbox_is_replaced_and_every_remote_comes_along() {
        let (home, real, root) = setup("half");
        // A fork: the branch tracks `upstream`, which has a commit origin doesn't.
        let up = root.join("upstream.git");
        run(&root, &["clone", "-q", "--bare", &root.join("remote.git").to_string_lossy(), &up.to_string_lossy()]);
        run(&real, &["remote", "add", "upstream", &up.to_string_lossy()]);
        run(&real, &["push", "-q", "upstream", "main"]);
        run(&real, &["fetch", "-q", "upstream"]);
        run(&real, &["branch", "-q", "-u", "upstream/main"]);
        // sigit stopped right after the clone: only the mark is there.
        let dir = dir_for(&home, "acme/app").unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git").join(MARK), "{}").unwrap();
        create(&home, "acme/app", &real).expect("a marked, half-made folder is replaced");
        assert_eq!(run(&dir, &["rev-parse", "upstream/main"]), run(&real, &["rev-parse", "upstream/main"]));
        assert_eq!(run(&dir, &["rev-parse", "--abbrev-ref", "main@{upstream}"]).trim(), "upstream/main");
        assert_eq!(run(&dir, &["rev-list", "--count", "@{upstream}..main"]).trim(), "0", "pushed to upstream, as in the real clone");
        for r in ["origin", "upstream"] {
            assert_eq!(run(&dir, &["config", &format!("remote.{r}.pushurl")]).trim(), NO_PUSH);
            assert!(crate::gh::run("git", &["push", r, "main"], Some(&dir)).is_err());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn only_marked_sandboxes_are_deleted() {
        let (home, real, root) = setup("safe");
        assert!(remove(&home, &real).is_err(), "the real clone is never deleted");
        assert!(real.exists());
        let fake = home.join("acme__app");
        std::fs::create_dir_all(fake.join(".git")).unwrap();
        assert!(remove(&home, &fake).is_err(), "an unmarked folder is left alone");
        assert!(create(&home, "acme/app", &real).is_err(), "and isn't replaced");
        assert!(fake.exists());
        run(&real, &["checkout", "-q", "--detach"]);
        let _ = std::fs::remove_dir_all(&fake);
        assert!(create(&home, "acme/app", &real).unwrap_err().contains("detached"));
        assert!(!fake.exists(), "nothing is left behind");
        let _ = std::fs::remove_dir_all(root);
    }
}
