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
    CreatePr { spec: crate::route::PrSpec },
}

impl Action {
    pub fn label(&self) -> String {
        match self {
            Action::Merge { number, .. } => tf!("Merge #{number}"),
            Action::Close { number, .. } => tf!("Close #{number}"),
            Action::Approve { number, .. } => tf!("Approve #{number}"),
            Action::RequestChanges { number, .. } => tf!("Request changes on #{number}"),
            Action::Comment { number, .. } => tf!("Comment on #{number}"),
            Action::Rerun { id, failed_only, .. } => tf!("Re-run {}{id}", if *failed_only { "failed jobs of " } else { "" }),
            Action::CancelRun { id, .. } => tf!("Cancel run {id}"),
            Action::Checkout { name, .. } => tf!("Checkout {name}"),
            Action::PrCheckout { number, .. } => tf!("Checkout #{number}"),
            Action::Pull { .. } => "Pull".into(),
            Action::Fetch { .. } => "Fetch".into(),
            Action::Stash { op, name, .. } => tf!("Stash {op} {name}"),
            Action::DeleteBranch { name, .. } => tf!("Delete branch {name}"),
            Action::CreatePr { spec, .. } => {
                let (head, base) = (&spec.head, &spec.base);
                tf!("Open PR {head} → {base}")
            }
        }
    }

    /// What it is, for the dialog's title line.
    pub fn subject(&self) -> String {
        match self {
            Action::Merge { title, .. } | Action::Close { title, .. } | Action::Approve { title, .. } => title.clone(),
            Action::Rerun { title, workflow, .. } | Action::CancelRun { title, workflow, .. } => tf!("{workflow}: {title}"),
            Action::Checkout { name, .. } => tf!("Switch your working copy to {name}"),
            Action::PrCheckout { number, .. } => tf!("Fetch PR #{number}'s branch and switch to it"),
            Action::Pull { .. } => "Fast-forward the current branch".into(),
            Action::DeleteBranch { name, .. } => tf!("Delete local branch {name}"),
            Action::CreatePr { spec, .. } => spec.title.clone(),
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
            Action::CreatePr { spec, .. } => spec.slug.clone(),
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

    /// The gh / git commands it runs, in order, as built for running (`execute`) and shown
    /// (the button's tooltip, the confirm dialog). A remote checkout lists both its forms: the
    /// second runs only when the first can't (no local copy of the branch yet).
    pub fn commands(&self) -> Vec<gh::Cmd> {
        match self {
            Action::Merge { slug, number, method, delete_branch, .. } => vec![gh::merge_cmd(slug, *number, *method, *delete_branch)],
            Action::Close { slug, number, .. } => vec![gh::close_cmd(slug, *number)],
            Action::Approve { slug, number, body, .. } => vec![gh::approve_cmd(slug, *number, body)],
            Action::RequestChanges { slug, number, body } => vec![gh::request_changes_cmd(slug, *number, body)],
            Action::Comment { slug, number, body } => vec![gh::comment_cmd(slug, *number, body)],
            Action::Rerun { slug, id, failed_only, .. } => vec![gh::rerun_cmd(slug, *id, *failed_only)],
            Action::CancelRun { slug, id, .. } => vec![gh::cancel_cmd(slug, *id)],
            Action::Checkout { name, remote, .. } => git::checkout_cmds(name, *remote),
            Action::PrCheckout { slug, number, .. } => vec![gh::checkout_pr_cmd(slug, *number)],
            Action::Pull { .. } => vec![git::pull_cmd()],
            Action::Fetch { .. } => vec![git::fetch_cmd()],
            Action::Stash { op, name, .. } => vec![git::stash_op_cmd(op, name)],
            Action::DeleteBranch { name, force, .. } => vec![git::delete_branch_cmd(name, *force)],
            Action::CreatePr { spec, .. } => vec![gh::create_pr_cmd(spec)],
        }
    }

    /// Where its commands run: the local clone for git ones, anywhere for gh ones (they name
    /// the repo with `-R`).
    fn cwd(&self) -> Option<&PathBuf> {
        match self {
            Action::Checkout { dir, .. } | Action::PrCheckout { dir, .. } | Action::Pull { dir } | Action::Fetch { dir } | Action::Stash { dir, .. } | Action::DeleteBranch { dir, .. } => Some(dir),
            _ => None,
        }
    }

    /// The command lines, with the stash that "Stash and continue" runs first.
    pub fn command_lines(&self, stash_first: bool) -> Vec<String> {
        let mut out = Vec::new();
        if stash_first {
            out.push(git::stash_push_cmd(&format!("sigit: before {}", self.label())).line());
        }
        out.extend(self.commands().iter().map(gh::Cmd::line));
        out
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

impl Undo {
    /// What taking it back runs (nothing for a settings change: that's local).
    pub fn command_lines(&self) -> Vec<String> {
        match self {
            Undo::ReopenPr { slug, number } => vec![gh::reopen_cmd(slug, *number).line()],
            Undo::SwitchBack { branch, .. } => git::checkout_cmds(branch, false).iter().map(gh::Cmd::line).collect(),
            Undo::RestoreBranch { name, sha, .. } => vec![git::restore_branch_cmd(name, sha).line()],
            Undo::StashPop { .. } => vec![git::stash_op_cmd("pop", "stash@{0}").line()],
            Undo::Prefs(_) => Vec::new(),
        }
    }
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

    /// The confirm dialog was dismissed: a cancelled CreatePr gives the user's draft back.
    pub fn cancel_pending(&mut self) {
        if let Some(p) = self.pending.take()
            && matches!(p.action, Action::CreatePr { .. })
        {
            self.restore_pr_draft();
        }
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
            self.toast(crate::i18n::t(tf!("{} cancelled: nothing was sent", g.action.label())), false);
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
            match &a {
                // Their own messages ("Switched to x", "Fetched all remotes").
                Action::Checkout { dir, name, remote, .. } => git::checkout(dir, name, *remote),
                Action::Pull { dir } => git::pull(dir),
                Action::Fetch { dir } => git::fetch(dir),
                Action::Stash { dir, op, name } => git::stash_op(dir, op, name),
                Action::DeleteBranch { dir, name, force, .. } => git::delete_branch(dir, name, *force),
                // The rest: each command in turn, in the clone where there is one.
                _ => {
                    let mut out = String::new();
                    for c in a.commands() {
                        out = c.run(a.cwd().map(PathBuf::as_path))?;
                    }
                    Ok(out)
                }
            }
        };
        // The activity log is written here, on the worker: no file IO on the UI thread.
        let work = move || {
            // The commands that actually ran, for the result toast and the activity log.
            let (r, commands) = gh::recording(run);
            let detail = match &r {
                Ok(out) => out.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("done").to_owned(),
                Err(e) => e.clone(),
            };
            let entry = activity::Entry { at: now(), repo: log_slug, action: log_label, ok: r.is_ok(), detail, commands };
            let logged = activity::append(&entry);
            (r, entry, logged)
        };
        if self.busy.contains(&label) {
            return; // a double click doesn't run it twice
        }
        self.busy.push(label.clone());
        jobs::spawn(self, work, move |s, done| s.action_finished(action, label, before, stash_first, done));
    }

    /// An action's command came back: log it, toast, and reload what it touched. Split from
    /// `execute` so tests can feed it a result without running anything.
    pub(crate) fn action_finished(&mut self, action: Action, label: String, before: Option<String>, stash_first: bool, (r, entry, logged): (Result<String, String>, activity::Entry, Result<(), String>)) {
        self.busy.retain(|b| *b != label);
        let ok = r.is_ok();
        let detail = entry.detail.clone();
        if let Err(e) = logged {
            self.toast(crate::i18n::t(tf!("Couldn't write the activity log: {e}")), true);
        }
        let ran = self.ran_lines(&entry.commands);
        self.activity.insert(0, entry);
        if !ok {
            self.toast(crate::i18n::t(tf!("{label} failed: {detail}")) + &ran, true);
            self.action_failed(&action);
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
        let msg = match &action {
            Action::CreatePr { spec, .. } => pr_number_from_url(&detail).map(|n| { let base = &spec.base; tf!("Opened #{n} → {base}") }),
            _ => None,
        };
        let msg = msg.unwrap_or_else(|| if stash_first { tf!("Stashed your changes, then: {detail}") } else { detail });
        self.toast_undo(crate::i18n::t(msg) + &ran, undo);
        self.after(&action);
    }

    /// What to reload after an action failed: a PR that already exists changes what the form shows.
    pub(crate) fn action_failed(&mut self, action: &Action) {
        if let Action::CreatePr { spec, .. } = action {
            self.restore_pr_draft();
            self.reload_branch_prs(&spec.slug, &spec.head);
        }
    }

    /// What to reload after an action succeeded.
    fn after(&mut self, action: &Action) {
        match action {
            Action::Merge { slug, .. } => {
                self.refresh_prs();
                // A merge changes every branch's PRs: refetch each when it's next needed.
                self.branch_prs.retain(|(sl, _), _| sl != slug);
                if let Some(head) = self.branch_lane_sel.as_ref().filter(|(sl, _)| sl == slug).map(|(_, b)| b.clone()) {
                    self.reload_branch_prs(slug, &head);
                }
            }
            Action::Close { .. } | Action::Approve { .. } | Action::RequestChanges { .. } | Action::Comment { .. } => self.refresh_prs(),
            Action::CreatePr { spec, .. } => {
                self.last_pr_draft = None;
                let last = crate::config::PrLast {
                    slug: spec.slug.clone(),
                    base: spec.base.clone(),
                    draft: spec.draft,
                    reviewers: spec.reviewers.clone(),
                    assignees: spec.assignees.clone(),
                    labels: spec.labels.clone(),
                };
                match self.prefs.pr_last.iter_mut().find(|p| p.slug == spec.slug) {
                    Some(p) => *p = last,
                    None => self.prefs.pr_last.push(last),
                }
                self.save_prefs();
                self.refresh_prs();
                self.reload_branch_prs(&spec.slug, &spec.head);
            }
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
            Undo::ReopenPr { slug, number } => (tf!("Reopen #{number}"), Box::new(move || gh::reopen_cmd(&slug, number).run(None))),
            Undo::SwitchBack { dir, branch } => (tf!("Switch back to {branch}"), Box::new(move || git::checkout(&dir, &branch, false))),
            Undo::RestoreBranch { dir, name, sha } => (tf!("Restore {name}"), Box::new(move || git::restore_branch(&dir, &name, &sha))),
            Undo::StashPop { dir } => ("Pop stash".into(), Box::new(move || git::stash_op(&dir, "pop", "stash@{0}"))),
            Undo::Prefs(p) => {
                self.prefs = *p;
                self.save_prefs();
                self.toast(crate::i18n::t("Undone"), false);
                return;
            }
        };
        self.act(crate::i18n::t(label), work, |s| s.refresh_page());
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
                a.blocked = Some(tf!(
                    "A {what} is half done in this working copy. Finish or abort it in your terminal or editor (git {what} --continue or --abort) first."
                ));
            }
            Ok(st) => {
                if !st.changed.is_empty() {
                    a.risk.because(Level::Medium, tf!("{} uncommitted change(s) would get in the way", st.changed.len()));
                    a.dirty = st.changed;
                }
                if st.orphaned > 0 {
                    a.risk.because(Level::Medium, tf!("You're on a detached HEAD with {} commit(s) on no branch: switching away makes them hard to find", st.orphaned));
                }
            }
            Err(e) => a.blocked = Some(tf!("Couldn't read the working copy: {e}")),
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
                a.risk.because(Level::High, tf!("This commit is deployed to {env}, a production environment: re-running or cancelling can change what's live"));
                a.word = Some((*env).clone());
            } else if risk::looks_like_deploy(workflow) {
                a.risk.because(Level::Medium, tf!("\"{workflow}\" looks like a deploy workflow"));
                if !deployed_to.is_empty() {
                    a.risk.reasons.push(tf!("This commit was deployed to: {}", deployed_to.join(", ")));
                }
            }
        }
        Action::Stash { op: "drop", name, .. } => a.risk.because(Level::Medium, tf!("Dropping {name} deletes those changes for good")),
        Action::DeleteBranch { name, force: true, unpushed, .. } => {
            a.risk.because(Level::High, tf!("{name} isn't merged: deleting it with -D loses work no remote has"));
            for c in unpushed.iter().take(8) {
                a.risk.reasons.push(tf!("  • {c}"));
            }
            a.word = Some(name.clone());
        }
        Action::CreatePr { spec, .. } => a.risk = create_pr_risk(&spec.base, &ctx.prefs.prod_names),
        _ => {}
    }
    if a.risk.level == Level::High && a.word.is_none() {
        a.word = Some(repo_name(&action.slug()));
    }
    a
}

/// Opening a PR is harmless (close it again); it asks only when it targets production.
pub(crate) fn create_pr_risk(base: &str, prod_names: &[String]) -> Risk {
    let mut r = Risk::new(Level::Low);
    r.because(Level::Low, "Opens a PR; you can close it");
    if risk::is_production(base, false, prod_names) {
        r.because(Level::Medium, tf!("{base} is a production branch"));
    }
    r
}

/// The PR number from the URL `gh pr create` prints: the last path segment.
pub(crate) fn pr_number_from_url(out: &str) -> Option<u64> {
    out.trim().rsplit('/').next()?.parse().ok()
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
        r.because(Level::High, tf!("It's no longer open ({}): someone acted on it meanwhile", crate::i18n::t(s("state").to_lowercase())));
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
        r.reasons.push(tf!("Checks: {passing} passing, {failing} failing, {running} running"));
        r.reasons.push(tf!("+{add} −{del} in {} files", files.len()));
    }
    let found = risk::sensitive_files(files.iter().map(|(p, c)| (p.as_str(), *c)), &ctx.prefs.sensitive);
    for (path, what) in found.iter().take(6) {
        let what = crate::i18n::t(what.as_str());
        r.because(if merging { Level::High } else { Level::Low }, tf!("Changes {what}: {path}"));
    }
    if !merging {
        return;
    }

    if v["isDraft"].as_bool().unwrap_or(false) {
        r.because(Level::High, "It's still a draft");
    }
    if failing > 0 {
        r.because(Level::High, tf!("{failing} of {} checks are failing", checks.len()));
    } else if running > 0 {
        r.because(Level::Medium, tf!("{running} check(s) are still running"));
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
            let what = crate::i18n::t(if protected { "protected" } else { "the default branch" });
            let rule_text = if rules.is_empty() { String::new() } else { tf!(" (rules: {})", rules.join(", ")) };
            r.because(Level::High, tf!("{base} is {what}{rule_text}"));
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
            let what = crate::i18n::t(what);
            r.because(Level::High, tf!("Possible {what} added in {file}: {line}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::PrSpec;

    fn spec() -> PrSpec {
        PrSpec {
            slug: "o/r".into(),
            head: "feat-a".into(),
            base: "main".into(),
            draft: false,
            title: "Add a".into(),
            body: String::new(),
            reviewers: String::new(),
            assignees: String::new(),
            labels: String::new(),
        }
    }

    #[test]
    fn create_pr_risk_levels() {
        let names = Prefs::default().prod_names;
        assert_eq!(create_pr_risk("staging", &names).level, Level::Low);
        assert_eq!(create_pr_risk("production", &names).level, Level::Medium);
    }

    #[test]
    fn create_pr_action_commands() {
        let s = spec();
        let a = Action::CreatePr { spec: s.clone() };
        assert_eq!(a.commands(), vec![gh::create_pr_cmd(&s)]);
        assert_eq!(a.label(), "Open PR feat-a → main");
        assert_eq!(a.subject(), "Add a");
    }

    #[test]
    fn pr_number_parses_from_url() {
        assert_eq!(pr_number_from_url("https://github.com/o/r/pull/161"), Some(161));
        assert_eq!(pr_number_from_url("https://github.com/o/r/pull/161\n"), Some(161));
        assert_eq!(pr_number_from_url("done"), None);
    }
}
