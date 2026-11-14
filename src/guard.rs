//! Guard rails (A1–A6). Every write action is an `Action` value that goes through `request`:
//!
//! 1. Actions that can't hurt (comment, fetch, approve) run at once.
//! 2. The rest are checked on a worker thread against **fresh** GitHub/git state (never the
//!    cache): protected branch, failing checks, sensitive files, secrets, production deploys,
//!    uncommitted changes. The answer is a `Risk` (level + reasons).
//! 3. Low runs; Medium asks; High asks you to type a name. A merge then waits `grace_secs`
//!    with a Cancel button before it runs.
//! 4. It runs, is logged to the activity file, and reversible ones toast an **Undo** for 10 s.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rune::TextEdit;

use crate::activity;
use crate::config::Prefs;
use crate::gh::{self, MergeMethod};
use crate::git::{self, Safety};
use crate::model::{Envs, Outcome, now};
use crate::risk::{self, Level, Risk};
use crate::{State, jobs};

#[derive(Clone, Debug)]
pub enum Action {
    Merge { slug: String, number: u64, title: String, method: MergeMethod, delete_branch: bool },
    Close { slug: String, number: u64, title: String },
    /// `confirm`: show a summary first (the review queue's A key) instead of approving at once.
    Approve { slug: String, number: u64, title: String, body: String, confirm: bool },
    RequestChanges { slug: String, number: u64, body: String },
    Comment { slug: String, number: u64, body: String },
    Rerun { slug: String, id: u64, title: String, workflow: String, sha: String, failed_only: bool },
    CancelRun { slug: String, id: u64, title: String, workflow: String, sha: String },
    Checkout { dir: PathBuf, slug: String, name: String, remote: bool },
    PrCheckout { dir: PathBuf, slug: String, number: u64 },
    Pull { dir: PathBuf },
    Fetch { dir: PathBuf },
    /// `op`: apply, pop, drop.
    Stash { dir: PathBuf, op: &'static str, name: String },
    DeleteBranch { dir: PathBuf, name: String, sha: String, force: bool, unpushed: Vec<String> },
}

impl Action {
    pub fn label(&self) -> String {
        match self {
            Action::Merge { number, .. } => format!("Merge #{number}"),
            Action::Close { number, .. } => format!("Close #{number}"),
            Action::Approve { number, .. } => format!("Approve #{number}"),
            Action::RequestChanges { number, .. } => format!("Request changes on #{number}"),
            Action::Comment { number, .. } => format!("Comment on #{number}"),
            Action::Rerun { id, failed_only, .. } => format!("Re-run {}{id}", if *failed_only { "failed jobs of " } else { "" }),
            Action::CancelRun { id, .. } => format!("Cancel run {id}"),
            Action::Checkout { name, .. } => format!("Checkout {name}"),
            Action::PrCheckout { number, .. } => format!("Checkout #{number}"),
            Action::Pull { .. } => "Pull".into(),
            Action::Fetch { .. } => "Fetch".into(),
            Action::Stash { op, name, .. } => format!("Stash {op} {name}"),
            Action::DeleteBranch { name, .. } => format!("Delete branch {name}"),
        }
    }

    /// What it is, for the dialog's title line.
    pub fn subject(&self) -> String {
        match self {
            Action::Merge { title, .. } | Action::Close { title, .. } | Action::Approve { title, .. } => title.clone(),
            Action::Rerun { title, workflow, .. } | Action::CancelRun { title, workflow, .. } => format!("{workflow}: {title}"),
            Action::Checkout { name, .. } => format!("Switch your working copy to {name}"),
            Action::PrCheckout { number, .. } => format!("Fetch PR #{number}'s branch and switch to it"),
            Action::Pull { .. } => "Fast-forward the current branch".into(),
            Action::DeleteBranch { name, .. } => format!("Delete local branch {name}"),
            other => other.label(),
        }
    }

    fn slug(&self) -> String {
        match self {
            Action::Merge { slug, .. }
            | Action::Close { slug, .. }
            | Action::Approve { slug, .. }
            | Action::RequestChanges { slug, .. }
            | Action::Comment { slug, .. }
            | Action::Rerun { slug, .. }
            | Action::CancelRun { slug, .. }
            | Action::Checkout { slug, .. }
            | Action::PrCheckout { slug, .. } => slug.clone(),
            _ => String::new(),
        }
    }

    /// Runs without asking: nothing to check, nothing to lose.
    fn instant(&self) -> bool {
        matches!(
            self,
            Action::Approve { confirm: false, .. }
                | Action::RequestChanges { .. }
                | Action::Comment { .. }
                | Action::Fetch { .. }
                | Action::Stash { op: "apply", .. }
        )
    }

    /// Changes the working copy: checked for uncommitted changes first (A5).
    fn local_dir(&self) -> Option<&PathBuf> {
        match self {
            Action::Checkout { dir, .. } | Action::PrCheckout { dir, .. } | Action::Pull { dir } | Action::Stash { dir, op: "pop", .. } => Some(dir),
            _ => None,
        }
    }
}

/// The open confirm dialog.
pub struct Pending {
    pub action: Action,
    /// Still checking: the dialog shows a spinner and can't be confirmed yet.
    pub checking: bool,
    pub risk: Risk,
    /// Why it can't run at all (a half-done rebase...).
    pub blocked: Option<String>,
    /// Uncommitted files: the dialog offers "Stash and continue".
    pub dirty: Vec<String>,
    /// For High risk: what to type.
    pub word: Option<String>,
}

/// A merge counting down (A6): it runs at `at` unless cancelled.
pub struct Grace {
    pub action: Action,
    pub at: Instant,
}

/// How to take an action back (A6).
#[derive(Clone, Debug)]
pub enum Undo {
    ReopenPr { slug: String, number: u64 },
    SwitchBack { dir: PathBuf, branch: String },
    RestoreBranch { dir: PathBuf, name: String, sha: String },
    StashPop { dir: PathBuf },
    Prefs(Box<Prefs>),
}

/// What the worker thread found.
struct Assessment {
    risk: Risk,
    blocked: Option<String>,
    dirty: Vec<String>,
    word: Option<String>,
}

/// What `assess` needs from the UI state, copied so it can run on a worker.
struct Context {
    prefs: Prefs,
    envs: Option<Envs>,
}

impl State {
    /// The single entry point for every write action.
    pub fn request(&mut self, action: Action) {
        if action.instant() {
            self.execute(action, false);
            return;
        }
        let ctx = Context { prefs: self.prefs.clone(), envs: self.rd().and_then(|d| d.envs.data.clone()) };
        self.typed = TextEdit::new("");
        self.pending = Some(Pending { action: action.clone(), checking: true, risk: Risk::new(Level::Low), blocked: None, dirty: Vec::new(), word: None });
        let label = action.label();
        jobs::spawn(self, move || assess(&action, &ctx), move |s, a| {
            let Some(p) = s.pending.as_mut().filter(|p| p.action.label() == label) else { return };
            p.checking = false;
            p.risk = a.risk;
            p.blocked = a.blocked;
            p.dirty = a.dirty;
            p.word = a.word;
            // Nothing to warn about: don't make anyone click through an empty dialog.
            let quiet = matches!(p.action, Action::Rerun { .. } | Action::Checkout { .. } | Action::PrCheckout { .. } | Action::Pull { .. } | Action::Stash { .. })
                || matches!(p.action, Action::DeleteBranch { force: false, .. });
            if quiet && p.risk.level == Level::Low && p.blocked.is_none() && p.dirty.is_empty() {
                let Some(p) = s.pending.take() else { return };
                s.execute(p.action, false);
            }
        });
    }

    /// Whether the dialog's confirm button may be pressed.
    pub fn can_confirm(&self) -> bool {
        let Some(p) = &self.pending else { return false };
        !p.checking && p.blocked.is_none() && p.dirty.is_empty() && (p.risk.level < Level::High || p.word.as_deref().is_some_and(|w| self.typed.text.trim() == w))
    }

    /// Confirm pressed. `stash_first`: "Stash and continue".
    pub fn confirm(&mut self, stash_first: bool) {
        let Some(p) = self.pending.take() else { return };
        if matches!(p.action, Action::Merge { .. }) && self.prefs.grace_secs > 0 {
            self.grace = Some(Grace { action: p.action, at: Instant::now() + Duration::from_secs(self.prefs.grace_secs as u64) });
            return;
        }
        self.execute(p.action, stash_first);
    }

    pub fn cancel_grace(&mut self) {
        if let Some(g) = self.grace.take() {
            self.toast(format!("{} cancelled: nothing was sent", g.action.label()), false);
        }
    }

    /// Called from `poll`: runs a merge whose grace period ran out.
    pub fn tick_grace(&mut self) -> bool {
        if self.grace.as_ref().is_some_and(|g| Instant::now() >= g.at) {
            let g = self.grace.take().expect("checked above");
            self.execute(g.action, false);
            return true;
        }
        false
    }

    fn execute(&mut self, action: Action, stash_first: bool) {
        let label = action.label();
        let slug = action.slug();
        // The branch we're on, for "switch back".
        let before = self.rd().and_then(|d| d.status.data.as_ref()).map(|st| st.branch.clone());
        let stash_dir = action.local_dir().cloned().filter(|_| stash_first);
        let a = action.clone();
        let (log_label, log_slug) = (label.clone(), slug.clone());
        let run = move || -> Result<String, String> {
            if let Some(dir) = &stash_dir {
                git::stash_push(dir, &format!("sigit: before {}", a.label()))?;
            }
            match a {
                Action::Merge { slug, number, method, delete_branch, .. } => gh::merge(&slug, number, method, delete_branch),
                Action::Close { slug, number, .. } => gh::close(&slug, number),
                Action::Approve { slug, number, body, .. } => gh::approve(&slug, number, &body),
                Action::RequestChanges { slug, number, body } => gh::request_changes(&slug, number, &body),
                Action::Comment { slug, number, body } => gh::comment(&slug, number, &body),
                Action::Rerun { slug, id, failed_only, .. } => gh::rerun(&slug, id, failed_only),
                Action::CancelRun { slug, id, .. } => gh::cancel(&slug, id),
                Action::Checkout { dir, name, remote, .. } => git::checkout(&dir, &name, remote),
                Action::PrCheckout { dir, slug, number } => gh::checkout_pr(&dir, &slug, number),
                Action::Pull { dir } => git::pull(&dir),
                Action::Fetch { dir } => git::fetch(&dir),
                Action::Stash { dir, op, name } => git::stash_op(&dir, op, &name),
                Action::DeleteBranch { dir, name, force, .. } => git::delete_branch(&dir, &name, force),
            }
        };
        // The activity log is written here, on the worker: no file IO on the UI thread.
        let work = move || {
            let r = run();
            let detail = match &r {
                Ok(out) => out.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("done").to_owned(),
                Err(e) => e.clone(),
            };
            let entry = activity::Entry { at: now(), repo: log_slug, action: log_label, ok: r.is_ok(), detail };
            let logged = activity::append(&entry);
            (r, entry, logged)
        };
        if self.busy.contains(&label) {
            return; // a double click doesn't run it twice
        }
        self.busy.push(label.clone());
        jobs::spawn(self, work, move |s, (r, entry, logged)| {
            s.busy.retain(|b| *b != label);
            let ok = r.is_ok();
            let detail = entry.detail.clone();
            if let Err(e) = logged {
                s.toast(format!("Couldn't write the activity log: {e}"), true);
            }
            s.activity.insert(0, entry);
            if !ok {
                s.toast(format!("{label} failed: {detail}"), true);
                return;
            }
            let undo = match &action {
                Action::Close { slug, number, .. } => Some(Undo::ReopenPr { slug: slug.clone(), number: *number }),
                Action::Checkout { dir, .. } | Action::PrCheckout { dir, .. } => {
                    before.clone().filter(|b| !b.is_empty() && b != "(detached)").map(|branch| Undo::SwitchBack { dir: dir.clone(), branch })
                }
                Action::DeleteBranch { dir, name, sha, .. } => Some(Undo::RestoreBranch { dir: dir.clone(), name: name.clone(), sha: sha.clone() }),
                _ => None,
            };
            let undo = undo.or_else(|| stash_first.then(|| action.local_dir().map(|d| Undo::StashPop { dir: d.clone() })).flatten());
            s.toast_undo(if stash_first { format!("Stashed your changes, then: {detail}") } else { detail }, undo);
            s.after(&action);
        });
    }

    /// What to reload after an action succeeded.
    fn after(&mut self, action: &Action) {
        match action {
            Action::Merge { .. } | Action::Close { .. } | Action::Approve { .. } | Action::RequestChanges { .. } | Action::Comment { .. } => self.refresh_prs(),
            Action::Rerun { .. } | Action::CancelRun { .. } => self.refresh_runs(),
            Action::Checkout { slug, name, .. } => {
                self.refresh_page();
                if self.prefs.auto_watch {
                    let branch = name.split_once('/').filter(|_| name.starts_with("origin/")).map_or(name.as_str(), |(_, b)| b).to_owned();
                    self.watch_branch(slug, &branch);
                }
            }
            Action::PrCheckout { .. } | Action::Pull { .. } | Action::Fetch { .. } => self.refresh_page(),
            Action::Stash { .. } | Action::DeleteBranch { .. } => self.refresh_branch_tools(),
        }
    }

    pub fn run_undo(&mut self, toast: u64) {
        let Some(i) = self.toasts.iter().position(|t| t.id == toast) else { return };
        let Some(undo) = self.toasts.remove(i).undo else { return };
        let (label, work): (String, Box<dyn FnOnce() -> Result<String, String> + Send>) = match undo {
            Undo::ReopenPr { slug, number } => (format!("Reopen #{number}"), Box::new(move || gh::reopen(&slug, number))),
            Undo::SwitchBack { dir, branch } => (format!("Switch back to {branch}"), Box::new(move || git::checkout(&dir, &branch, false))),
            Undo::RestoreBranch { dir, name, sha } => (format!("Restore {name}"), Box::new(move || git::restore_branch(&dir, &name, &sha))),
            Undo::StashPop { dir } => ("Pop stash".into(), Box::new(move || git::stash_op(&dir, "pop", "stash@{0}"))),
            Undo::Prefs(p) => {
                self.prefs = *p;
                self.save_prefs();
                self.toast("Undone".into(), false);
                return;
            }
        };
        self.act(label, work, |s| s.refresh_page());
    }
}

/// The worker-thread half of `request`: read fresh state, decide the risk.
fn assess(action: &Action, ctx: &Context) -> Assessment {
    let mut a = Assessment { risk: Risk::new(Level::Low), blocked: None, dirty: Vec::new(), word: None };
    let repo_name = |slug: &str| slug.rsplit('/').next().unwrap_or(slug).to_owned();

    // A5: the working copy first, for anything that changes it.
    if let Some(dir) = action.local_dir() {
        match git::safety(dir) {
            Ok(Safety { in_progress: Some(what), .. }) => {
                a.blocked = Some(format!(
                    "A {what} is half done in this working copy. Finish or abort it in your terminal or editor (git {what} --continue or --abort) first."
                ));
            }
            Ok(st) => {
                if !st.changed.is_empty() {
                    a.risk.because(Level::Medium, format!("{} uncommitted change(s) would get in the way", st.changed.len()));
                    a.dirty = st.changed;
                }
                if st.orphaned > 0 {
                    a.risk.because(Level::Medium, format!("You're on a detached HEAD with {} commit(s) on no branch: switching away makes them hard to find", st.orphaned));
                }
            }
            Err(e) => a.blocked = Some(format!("Couldn't read the working copy: {e}")),
        }
    }

    match action {
        Action::Merge { slug, number, .. } => {
            a.risk.because(Level::Medium, "Merging can't be undone (only reverted with a new commit)");
            a.word = Some(repo_name(slug));
            assess_pr(&mut a.risk, slug, *number, ctx, true);
        }
        Action::Close { slug, number, .. } => {
            a.risk.because(Level::Medium, "Closing stops reviews and CI on it (you can reopen it)");
            assess_pr(&mut a.risk, slug, *number, ctx, false);
        }
        Action::Approve { slug, number, .. } => {
            // The queue's A key: a summary of what you're approving.
            assess_pr(&mut a.risk, slug, *number, ctx, false);
            a.risk.level = Level::Low;
        }
        Action::Rerun { slug, workflow, sha, .. } | Action::CancelRun { slug, workflow, sha, .. } => {
            if matches!(action, Action::CancelRun { .. }) {
                a.risk.because(Level::Medium, "Cancelling stops every job in the run where it is");
            }
            // A4: does this run deploy, and to production?
            let envs = gh::envs(slug).ok().or_else(|| ctx.envs.clone());
            let deployed_to: Vec<String> = envs
                .as_ref()
                .map(|e| {
                    let mut v: Vec<String> = e.deployments.iter().filter(|d| !sha.is_empty() && d.sha == *sha).map(|d| d.environment.clone()).collect();
                    v.dedup();
                    v
                })
                .unwrap_or_default();
            let prod: Vec<&String> = deployed_to
                .iter()
                .filter(|env| risk::is_production(env, envs.as_ref().is_some_and(|e| e.protected.contains(env)), &ctx.prefs.prod_names))
                .collect();
            if let Some(env) = prod.first() {
                a.risk.because(Level::High, format!("This commit is deployed to {env}, a production environment: re-running or cancelling can change what's live"));
                a.word = Some((*env).clone());
            } else if risk::looks_like_deploy(workflow) {
                a.risk.because(Level::Medium, format!("\"{workflow}\" looks like a deploy workflow"));
                if !deployed_to.is_empty() {
                    a.risk.reasons.push(format!("This commit was deployed to: {}", deployed_to.join(", ")));
                }
            }
        }
        Action::Stash { op: "drop", name, .. } => a.risk.because(Level::Medium, format!("Dropping {name} deletes those changes for good")),
        Action::DeleteBranch { name, force: true, unpushed, .. } => {
            a.risk.because(Level::High, format!("{name} isn't merged: deleting it with -D loses work no remote has"));
            for c in unpushed.iter().take(8) {
                a.risk.reasons.push(format!("  • {c}"));
            }
            a.word = Some(name.clone());
        }
        _ => {}
    }
    if a.risk.level == Level::High && a.word.is_none() {
        a.word = Some(repo_name(&action.slug()));
    }
    a
}

/// A1–A3 for a pull request, from fresh GitHub state.
fn assess_pr(r: &mut Risk, slug: &str, number: u64, ctx: &Context, merging: bool) {
    let n = number.to_string();
    let view: Result<serde_json::Value, String> = gh::run(
        "gh",
        &["pr", "view", &n, "-R", slug, "--json", "state,isDraft,baseRefName,reviewDecision,mergeStateStatus,statusCheckRollup,files,additions,deletions"],
        None,
    )
    .and_then(|o| serde_json::from_str(&o).map_err(|e| e.to_string()));
    let Ok(v) = view else {
        r.because(Level::Medium, "Couldn't read the pull request's current state from GitHub");
        return;
    };
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
    if s("state") != "OPEN" {
        r.because(Level::High, format!("It's no longer open ({}): someone acted on it meanwhile", s("state").to_lowercase()));
    }
    let checks: Vec<crate::model::Check> = serde_json::from_value(v["statusCheckRollup"].clone()).unwrap_or_default();
    let failing = checks.iter().filter(|c| c.outcome() == Outcome::Failure).count();
    let running = checks.iter().filter(|c| c.outcome() == Outcome::Running).count();
    let passing = checks.iter().filter(|c| c.outcome() == Outcome::Success).count();
    let (add, del) = (v["additions"].as_u64().unwrap_or(0), v["deletions"].as_u64().unwrap_or(0));
    let files: Vec<(String, u64)> = v["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| (f["path"].as_str().unwrap_or_default().to_owned(), f["additions"].as_u64().unwrap_or(0) + f["deletions"].as_u64().unwrap_or(0)))
        .collect();

    if !merging {
        // A summary rather than warnings (approve from the queue, close).
        r.reasons.push(format!("Checks: {passing} passing, {failing} failing, {running} running"));
        r.reasons.push(format!("+{add} −{del} in {} files", files.len()));
    }
    let found = risk::sensitive_files(files.iter().map(|(p, c)| (p.as_str(), *c)), &ctx.prefs.sensitive);
    for (path, what) in found.iter().take(6) {
        r.because(if merging { Level::High } else { Level::Low }, format!("Changes {what}: {path}"));
    }
    if !merging {
        return;
    }

    if v["isDraft"].as_bool().unwrap_or(false) {
        r.because(Level::High, "It's still a draft");
    }
    if failing > 0 {
        r.because(Level::High, format!("{failing} of {} checks are failing", checks.len()));
    } else if running > 0 {
        r.because(Level::Medium, format!("{running} check(s) are still running"));
    }
    match s("reviewDecision").as_str() {
        "CHANGES_REQUESTED" => r.because(Level::High, "A reviewer requested changes"),
        "REVIEW_REQUIRED" => r.because(Level::Medium, "It hasn't been approved yet"),
        _ => {}
    }
    match s("mergeStateStatus").as_str() {
        "DIRTY" => r.because(Level::High, "It has merge conflicts"),
        "BLOCKED" => r.because(Level::High, "GitHub reports it as blocked by branch rules"),
        "BEHIND" => r.because(Level::Medium, "Its branch is behind the base branch"),
        _ => {}
    }
    // A2: the base branch.
    let base = s("baseRefName");
    if let Ok(meta) = gh::repo_meta(slug) {
        let protected = meta.protected.contains(&base);
        if protected || base == meta.default_branch {
            let rules = gh::branch_rules(slug, &base);
            let what = if protected { "protected" } else { "the default branch" };
            let rule_text = if rules.is_empty() { String::new() } else { format!(" (rules: {})", rules.join(", ")) };
            r.because(Level::High, format!("{base} is {what}{rule_text}"));
            let problems = failing > 0 || s("reviewDecision") != "APPROVED";
            if meta.admin && problems {
                r.because(Level::High, "You're an admin, so GitHub would let this through anyway. Your team's rules say it shouldn't");
            }
        }
    }
    // A3: secrets in the diff.
    if let Ok(mut diff) = gh::run("gh", &["pr", "diff", &n, "-R", slug], None) {
        gh::cap(&mut diff, 2_000_000);
        for (file, line, what) in risk::secret_lines(&diff).into_iter().take(4) {
            r.because(Level::High, format!("Possible {what} added in {file}: {line}"));
        }
    }
}
