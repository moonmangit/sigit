//! sigit: a fast desktop view of your everyday git and GitHub work.
//!
//! - **Inbox**: PRs waiting for your review, your open PRs, assigned issues, recent runs.
//! - **Per repo**: pull requests (review, merge, close, checkout), Actions (re-run, cancel,
//!   job timeline), environments (deployments timeline), commit graph, branches
//!   (checkout, fetch, fast-forward pull).
//!
//! Everything goes through the `gh` and `git` CLIs you already use, on background threads
//! (`jobs.rs`): the window never waits on the network, and switching views shows the last
//! answer at once while a fresh one loads. Nothing here commits or pushes.
//!
//! `cargo run -- [path/to/repo]`. Headless still: `cargo run -- --snap s.png --scene prs`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activity;
mod cache;
mod focus;
mod onboarding;
mod settings;
mod update;
mod window_icon;
mod config;
mod daily;
mod gh;
mod git;
mod guard;
mod jobs;
mod model;
mod pulse;
mod risk;
mod ui;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use rune::{AppSpec, Cx, El, FileDialogOptions, KeyEvent, PointerPress, TextEdit, Tx};
use rune_kit::TableState;
use rune_kit::shell::Reorder;
use rune_kit::shell::PaneState;

use model::{Branch, Cleanup, Commit, CommitDetail, Envs, Inbox, LocalStatus, Notif, Pr, PrDetail, QueueInfo, RepoMeta, Run, RunView, SearchRepo, Stash};

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

/// Something fetched in the background: the last good answer stays on screen while a new one
/// loads, so the UI never blanks.
pub struct Load<T> {
    pub data: Option<T>,
    pub loading: bool,
    pub error: Option<String>,
    /// When `data` was fetched (Unix seconds): what "updated 2m ago" and `stale` read.
    pub at: Option<i64>,
    /// `data` came from the on-disk cache (last session), not from this run.
    pub cached: bool,
}

impl<T> Default for Load<T> {
    fn default() -> Self {
        Load { data: None, loading: false, error: None, at: None, cached: false }
    }
}

impl<T> Load<T> {
    /// Data from the on-disk cache, saved at `at`.
    pub fn from_cache(data: T, at: i64) -> Self {
        Load { data: Some(data), at: Some(at), cached: true, ..Load::default() }
    }

    pub fn start() -> Self {
        Load { loading: true, ..Load::default() }
    }

    pub fn finish(&mut self, r: Result<T, String>) {
        self.loading = false;
        match r {
            Ok(v) => {
                self.data = Some(v);
                self.error = None;
                self.at = Some(model::now());
                self.cached = false;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Worth loading: nothing in flight, and no data yet or data older than `ttl` seconds.
    pub fn stale(&self, ttl: i64) -> bool {
        !self.loading && self.at.is_none_or(|t| model::now() - t >= ttl)
    }
}

/// How long fetched data counts as fresh (seconds). Opening a page, going back, and the 30 s
/// tick reload only what's older; Ctrl+R and finished actions reload everything on the page.
pub mod ttl {
    /// PR lists, environments, the inbox, notifications.
    pub const GH: i64 = 120;
    /// Workflow runs; while one is running.
    pub const RUNS: i64 = 60;
    pub const RUNNING: i64 = 15;
    /// The local clone: status, branches, the commit graph.
    pub const LOCAL: i64 = 20;
    /// Default branch, protection, admin.
    pub const META: i64 = 900;
    /// The Activity page (many requests per repo).
    pub const PULSE: i64 = 300;
    /// An opened PR or run.
    pub const DETAIL: i64 = 120;
}

/// Park the open detail (with its key) in `memo` so coming back to it is instant.
fn stash<K: std::hash::Hash + Eq, T>(memo: &mut HashMap<K, Load<T>>, cur: &mut Load<T>, key: Option<K>) {
    let mut prev = std::mem::take(cur);
    if let Some(k) = key
        && prev.data.is_some()
    {
        prev.loading = false; // its answer will be dropped: it's no longer selected
        if memo.len() >= 200 {
            memo.clear();
        }
        memo.insert(k, prev);
    }
}

pub struct Repo {
    pub slug: String,
    /// The local clone, when there is one (needed for commits, branches, checkout).
    pub path: Option<PathBuf>,
    pub pushed_at: String,
    pub archived: bool,
}

impl Repo {
    pub fn owner(&self) -> &str {
        self.slug.split_once('/').map_or("", |(o, _)| o)
    }
}

/// Groups whose repos can be dragged into order (rune-kit's reorder finds its state through a
/// plain function, so each group gets a fixed slot).
pub const MAX_DRAG_GROUPS: usize = 24;

/// How many rows each list asks GitHub / git for; "Load more" raises it.
pub const PR_PAGE: usize = 50;
pub const RUN_PAGE: usize = 50;
pub const COMMIT_PAGE: usize = 1000;

pub struct CommitGraph {
    /// All branches (true) or the current one.
    pub all: bool,
    pub commits: Vec<Commit>,
    pub rows: Vec<git::GraphRow>,
    pub width: usize,
}

#[derive(Default)]
pub struct RepoData {
    pub meta: Load<RepoMeta>,
    pub stashes: Load<Vec<Stash>>,
    pub cleanup: Load<Vec<Cleanup>>,
    /// Which PR_STATES the `prs` list holds (only open ones are cached).
    pub prs_state: usize,
    pub status: Load<LocalStatus>,
    pub graph: Load<CommitGraph>,
    pub branches: Load<Vec<Branch>>,
    pub prs: Load<Vec<Pr>>,
    pub runs: Load<Vec<Run>>,
    pub envs: Load<Envs>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Inbox,
    Pulls,
    Actions,
    Envs,
    Commits,
    Branches,
    /// A dashboard for one of your groups (B5).
    Group,
}

impl Page {
    pub const REPO: [Page; 5] = [Page::Pulls, Page::Actions, Page::Envs, Page::Commits, Page::Branches];
    pub fn label(self) -> &'static str {
        match self {
            Page::Inbox => "Inbox",
            Page::Pulls => "Pull requests",
            Page::Actions => "Actions",
            Page::Envs => "Environments",
            Page::Commits => "Commits",
            Page::Branches => "Branches",
            Page::Group => "Group",
        }
    }
}

pub const PR_STATES: [&str; 3] = ["open", "merged", "closed"];

/// Somewhere you can go back to: a page, and the repo or group it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub page: Page,
    pub repo: Option<String>,
    pub group: Option<String>,
}

impl Place {
    fn new(page: Page, repo: Option<String>, group: Option<String>) -> Place {
        // Only what the page shows counts, so Inbox → Inbox isn't a step.
        let repo = repo.filter(|_| Page::REPO.contains(&page));
        let group = group.filter(|_| page == Page::Group);
        Place { page, repo, group }
    }

    /// "Pull requests · anomalyco/rune", for the buttons' tooltips.
    pub fn label(&self) -> String {
        match (&self.repo, &self.group) {
            (Some(r), _) => format!("{} · {r}", self.page.label()),
            (_, Some(g)) => format!("Group · {g}"),
            _ => self.page.label().to_owned(),
        }
    }
}

/// What a right-click opened a menu on.
pub enum MenuTarget {
    Repo(String),
    Group(String),
    /// An owner (you or an organization) section header.
    Owner(String),
}

/// The "New group" / "Rename group" dialog.
pub struct GroupEdit {
    /// The group being renamed; None makes a new one.
    pub rename: Option<String>,
    /// A repo to put in the new group (it was opened from that repo's menu).
    pub add: Option<String>,
}

pub struct Toast {
    pub id: u64,
    pub text: String,
    pub error: bool,
    pub born: Instant,
    /// Shown as an "Undo" button for 10 s (A6).
    pub undo: Option<guard::Undo>,
}

pub struct State {
    pub page: Page,
    pub sidebar: PaneState,
    pub repos: Vec<Repo>,
    pub repo_filter: TextEdit,
    /// Your login: your own repos group first.
    pub me: String,
    /// Owners whose repo group is folded in the sidebar.
    pub collapsed: HashSet<String>,
    /// GitHub-wide repo search for the sidebar box: (query it answers, hits).
    pub repo_hits: (String, Vec<SearchRepo>),
    pub repo_search: Arc<AtomicU64>,
    pub repo_searching: bool,
    pub cur: Option<String>,
    pub data: HashMap<String, RepoData>,
    pub inbox: Load<Inbox>,
    // Pull requests
    pub pr_state: usize,
    pub pr_filter: TextEdit,
    /// Server-side PR search: ((repo, state, query) it answers, PRs).
    pub pr_hits: Option<((String, usize, String), Vec<Pr>)>,
    pub pr_search: Arc<AtomicU64>,
    pub pr_searching: bool,
    pub pr_limit: usize,
    pub run_limit: usize,
    pub commit_limit: usize,
    pub prs_table: TableState,
    pub pr_sel: Option<u64>,
    pub pr_detail: Load<PrDetail>,
    /// PRs, runs and commits opened before: shown at once when you go back to them.
    pub pr_memo: HashMap<(String, u64), Load<PrDetail>>,
    pub run_memo: HashMap<(String, u64), Load<RunView>>,
    pub commit_memo: HashMap<(String, String), Load<CommitDetail>>,
    pub pr_tab: usize,
    pub comment: TextEdit,
    // Actions
    pub runs_table: TableState,
    pub run_sel: Option<u64>,
    pub run_detail: Load<RunView>,
    // Commits
    pub all_branches: bool,
    pub commits_table: TableState,
    pub commit_sel: Option<String>,
    pub commit_detail: Load<CommitDetail>,
    // Branches
    /// 0 local, 1 remote, 2 stashes, 3 clean up.
    pub branch_view: usize,
    pub branches_table: TableState,
    pub branch_sel: Option<String>,
    // Guard rails (A1–A6)
    pub pending: Option<guard::Pending>,
    /// What's typed in a High-risk dialog.
    pub typed: TextEdit,
    pub grace: Option<guard::Grace>,
    pub activity: Vec<activity::Entry>,
    pub settings_prod: TextEdit,
    pub settings_sensitive: TextEdit,
    // Daily flow (B1–B8)
    pub notifs: Load<Vec<Notif>>,
    pub last_notif: Instant,
    pub queue: HashMap<(String, u64), QueueInfo>,
    pub queue_sel: usize,
    pub triage: Load<String>,
    pub triage_for: Option<u64>,
    pub group_cur: Option<String>,
    pub group_data: HashMap<String, Load<daily::GroupRepo>>,
    pub watches: Vec<daily::Watch>,
    pub last_watch: Instant,
    pub last_cache: Instant,
    /// The page's freshness check (`ensure_page`) every 30 s.
    pub last_tick: Instant,
    /// Where a link sent you: the row to reveal, the run or PR it opened when outside the
    /// loaded list, and a "latest failed run" still to pick (focus.rs).
    pub reveal: Option<focus::Reveal>,
    pub want_failed: Option<focus::WantFailed>,
    pub focus_run: Option<(String, Run)>,
    pub focus_pr: Option<(String, Pr)>,
    /// The window has its icon (Windows; see window_icon.rs).
    pub window_icon_set: bool,
    /// First run / preflight: git, gh and its sign-in (`onboarding`).
    pub setup: onboarding::Setup,
    pub setup_busy: bool,
    pub setup_at: Instant,
    // The Settings window
    pub settings_open: bool,
    pub settings_sec: usize,
    pub accounts: Load<Vec<gh::Account>>,
    pub git_name: TextEdit,
    pub git_email: TextEdit,
    /// What the global git config held when the window opened (or was last saved).
    pub git_saved: (String, String),
    /// (gh --version, git --version)
    pub versions: Option<(String, String)>,
    /// Settings → About → Check for updates.
    pub update: update::Update,
    /// Where each resizable row is (x, width in window points), for its drag handles.
    pub split_boxes: HashMap<&'static str, [f32; 2]>,
    pub palette: bool,
    pub palette_q: TextEdit,
    pub palette_sel: usize,
    pub guide_open: bool,
    pub guide_sec: usize,
    // Overlays
    pub toasts: Vec<Toast>,
    pub next_toast: u64,
    /// Labels of write actions in flight ("Merging #12"), shown in the toolbar.
    pub busy: Vec<String>,
    /// Theme, pinned repos, your groups: saved to prefs.json on every change.
    pub prefs: config::Prefs,
    pub menu: Option<(MenuTarget, PointerPress)>,
    pub group_edit: Option<GroupEdit>,
    /// Drag to reorder: the pinned list, and one per group (by the group's position).
    pub pin_drag: Reorder,
    pub group_drags: [Reorder; MAX_DRAG_GROUPS],
    /// The "Customize organization" dialog: which owner, and its colour being picked.
    pub owner_edit: Option<(String, String)>,
    /// The dialog's icon field (kept here, not inside the Option, so it always exists).
    pub owner_icon: TextEdit,
    pub owner_hex: TextEdit,
    /// Owner sections showing all their repos instead of the first few.
    pub expanded_orgs: HashSet<String>,
    // Inbox: 0 Activity (tracked repos over a range), 1 Needs you (queue, notifications)
    pub inbox_tab: usize,
    pub pulse: Load<pulse::Pulse>,
    /// What the loaded / loading pulse answers (scope, range), so late answers are dropped.
    pub pulse_key: String,
    /// The fresh Activity page being built in the background while the known one stays shown.
    pub pulse_next: Option<pulse::Pulse>,
    /// Each scope and range loaded this session (key → when, page), so switching back shows it
    /// at once while it refreshes.
    pub pulse_seen: HashMap<String, (i64, pulse::Pulse)>,
    pub pulse_preset: pulse::Preset,
    /// None: every tracked repo; Some(group): just that group's repos.
    pub pulse_scope: Option<String>,
    pub pulse_from: TextEdit,
    pub pulse_to: TextEdit,
    pub tracked_open: bool,
    /// Back / forward history of places (like a browser), newest last.
    pub back: Vec<Place>,
    pub fwd: Vec<Place>,
    /// Going back or forward: don't record that move as a new step.
    pub restoring: bool,
    /// Repo sections opened on the Activity page (they start as one summary line).
    pub pulse_open_repos: HashSet<String>,
    /// Topics opened inside a repo's Activity detail: "owner/repo|prs" and so on.
    pub pulse_topics: HashSet<String>,
    /// Per-day activity and contributors shown on the Activity page.
    pub pulse_details: bool,
    pub tracked_q: TextEdit,
    /// The dialog's name field (kept here, not inside the Option, so it always exists).
    pub group_name: TextEdit,
}

fn init() -> State {
    config::migrate_old_folder(); // before anything reads the folder
    update::clean_up_previous(); // the copy the last update set aside
    let mut s = State {
        page: Page::Inbox,
        sidebar: PaneState::new(264.0).range(200.0, 420.0),
        repos: Vec::new(),
        repo_filter: TextEdit::new(""),
        me: String::new(),
        collapsed: HashSet::new(),
        repo_hits: (String::new(), Vec::new()),
        repo_search: Arc::new(AtomicU64::new(0)),
        repo_searching: false,
        cur: None,
        data: HashMap::new(),
        inbox: Load::default(),
        pr_state: 0,
        pr_filter: TextEdit::new(""),
        pr_hits: None,
        pr_search: Arc::new(AtomicU64::new(0)),
        pr_searching: false,
        pr_limit: PR_PAGE,
        run_limit: RUN_PAGE,
        commit_limit: COMMIT_PAGE,
        prs_table: TableState::new(),
        pr_sel: None,
        pr_detail: Load::default(),
        pr_memo: HashMap::new(),
        run_memo: HashMap::new(),
        commit_memo: HashMap::new(),
        pr_tab: 0,
        comment: TextEdit::new(""),
        runs_table: TableState::new(),
        run_sel: None,
        run_detail: Load::default(),
        all_branches: true,
        commits_table: TableState::new(),
        commit_sel: None,
        commit_detail: Load::default(),
        branch_view: 0,
        branches_table: TableState::new(),
        branch_sel: None,
        pending: None,
        typed: TextEdit::new(""),
        grace: None,
        activity: Vec::new(),
        settings_prod: TextEdit::new(""),
        settings_sensitive: TextEdit::new(""),
        notifs: Load::default(),
        last_notif: Instant::now(),
        queue: HashMap::new(),
        queue_sel: 0,
        triage: Load::default(),
        triage_for: None,
        group_cur: None,
        group_data: HashMap::new(),
        watches: Vec::new(),
        last_watch: Instant::now(),
        last_cache: Instant::now(),
        last_tick: Instant::now(),
        reveal: None,
        want_failed: None,
        focus_run: None,
        focus_pr: None,
        window_icon_set: false,
        setup: onboarding::Setup::Checking,
        setup_busy: false,
        setup_at: Instant::now(),
        settings_open: false,
        settings_sec: 0,
        accounts: Load::default(),
        git_name: TextEdit::new(""),
        git_email: TextEdit::new(""),
        git_saved: (String::new(), String::new()),
        versions: None,
        update: update::Update::NotChecked,
        split_boxes: HashMap::new(),
        palette: false,
        palette_q: TextEdit::new(""),
        palette_sel: 0,
        guide_open: false,
        guide_sec: 0,
        toasts: Vec::new(),
        next_toast: 0,
        busy: Vec::new(),
        prefs: config::load_prefs(),
        menu: None,
        group_edit: None,
        pin_drag: Reorder::new(),
        group_drags: [Reorder::new(); MAX_DRAG_GROUPS],
        owner_edit: None,
        owner_icon: TextEdit::new(""),
        owner_hex: TextEdit::new(""),
        expanded_orgs: HashSet::new(),
        inbox_tab: 0,
        pulse: Load::default(),
        pulse_key: String::new(),
        pulse_next: None,
        pulse_seen: HashMap::new(),
        pulse_preset: pulse::Preset::Week,
        pulse_scope: None,
        pulse_from: TextEdit::new(""),
        pulse_to: TextEdit::new(""),
        tracked_open: false,
        back: Vec::new(),
        fwd: Vec::new(),
        restoring: false,
        pulse_open_repos: HashSet::new(),
        pulse_topics: HashSet::new(),
        pulse_details: false,
        tracked_q: TextEdit::new(""),
        group_name: TextEdit::new(""),
    };
    s.collapsed = s.prefs.collapsed.iter().cloned().collect();
    s.settings_prod = TextEdit::new(s.prefs.prod_names.join(", "));
    s.settings_sensitive = TextEdit::new(s.prefs.sensitive.join("\n"));
    // B8: last session's data first, so the first frame is full; fresh data replaces it.
    s.load_cache();
    for saved in config::load() {
        if saved.path.exists() {
            s.add_repo(saved.slug, Some(saved.path));
        }
    }
    // A repo named on the command line (or the folder we started in) opens first. It isn't
    // saved: only "Add local repo" remembers one.
    let arg = rune::extra_args().into_iter().find(|a| !a.starts_with('-')).map(PathBuf::from);
    let start = arg.or_else(|| std::env::current_dir().ok());
    jobs::spawn(
        &mut s,
        move || {
            let top = start.as_deref().filter(|d| git::is_repo(d)).and_then(git::toplevel)?;
            Some((git::slug_of(&top)?, top))
        },
        |s, found| {
            if let Some((slug, top)) = found {
                s.add_repo(slug.clone(), Some(top));
                s.select_repo(&slug, Page::Pulls);
            }
        },
    );
    // Check git, gh and its sign-in first; GitHub data loads once they're there, and the
    // setup screen (or, on a first run, the welcome) shows otherwise.
    s.run_preflight();
    s
}

// ---------------------------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------------------------

impl State {
    pub fn repo(&self) -> Option<&Repo> {
        let cur = self.cur.as_deref()?;
        self.repos.iter().find(|r| r.slug == cur)
    }

    pub fn rd(&self) -> Option<&RepoData> {
        self.data.get(self.cur.as_deref()?)
    }

    pub fn rd_mut_pub(&mut self, slug: &str) -> &mut RepoData {
        self.rd_mut(slug)
    }

    pub fn add_repo_pub(&mut self, slug: String, path: Option<PathBuf>) {
        self.add_repo(slug, path);
    }

    fn rd_mut(&mut self, slug: &str) -> &mut RepoData {
        self.data.entry(slug.to_owned()).or_default()
    }

    pub fn local_path(&self) -> Option<PathBuf> {
        self.repo().and_then(|r| r.path.clone())
    }

    fn add_repo(&mut self, slug: String, path: Option<PathBuf>) {
        match self.repos.iter_mut().find(|r| r.slug == slug) {
            Some(r) => {
                if path.is_some() {
                    r.path = path;
                }
            }
            None => {
                // Local clones sort first.
                let at = if path.is_some() { self.repos.iter().take_while(|r| r.path.is_some()).count() } else { self.repos.len() };
                self.repos.insert(at, Repo { slug, path, pushed_at: String::new(), archived: false });
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Back / forward
    // -----------------------------------------------------------------------------------------

    pub fn here(&self) -> Place {
        Place::new(self.page, self.cur.clone(), self.group_cur.clone())
    }

    /// Record where we are before going to `next` (unless it's the same place, or we're
    /// moving through history). A new step clears the forward list, as a browser does.
    pub fn remember(&mut self, next: &Place) {
        let here = self.here();
        if self.restoring || here == *next {
            return;
        }
        self.back.push(here);
        if self.back.len() > 50 {
            self.back.remove(0);
        }
        self.fwd.clear();
    }

    fn go(&mut self, to: Place) {
        self.restoring = true;
        match (&to.page, &to.repo, &to.group) {
            (Page::Group, _, Some(g)) => self.open_group(&g.clone()),
            (page, Some(r), _) => self.select_repo(&r.clone(), *page),
            (page, ..) => self.show(*page),
        }
        self.restoring = false;
    }

    pub fn go_back(&mut self) {
        // Skip places that no longer exist (a deleted group).
        while let Some(to) = self.back.pop() {
            if to.group.as_ref().is_some_and(|g| !self.prefs.groups.iter().any(|x| x.name == *g)) {
                continue;
            }
            let here = self.here();
            self.fwd.push(here);
            self.go(to);
            return;
        }
    }

    pub fn go_forward(&mut self) {
        while let Some(to) = self.fwd.pop() {
            if to.group.as_ref().is_some_and(|g| !self.prefs.groups.iter().any(|x| x.name == *g)) {
                continue;
            }
            let here = self.here();
            self.back.push(here);
            self.go(to);
            return;
        }
    }

    pub fn select_repo(&mut self, slug: &str, page: Page) {
        self.remember(&Place::new(page, Some(slug.to_owned()), None));
        if !self.repos.iter().any(|r| r.slug == slug) {
            self.add_repo(slug.to_owned(), None);
        }
        if self.cur.as_deref() != Some(slug) {
            self.stash_details();
            self.cur = Some(slug.to_owned());
            self.pr_sel = None;
            self.run_sel = None;
            self.commit_sel = None;
            self.branch_sel = None;
            self.prs_table = TableState::new();
            self.runs_table = TableState::new();
            self.commits_table = TableState::new();
            self.branches_table = TableState::new();
            self.pr_filter = TextEdit::new("");
            self.pr_hits = None;
            self.pr_limit = PR_PAGE;
            self.run_limit = RUN_PAGE;
            self.commit_limit = COMMIT_PAGE;
            self.triage_for = None;
            self.prefs.recent.retain(|r| r != slug);
            self.prefs.recent.insert(0, slug.to_owned());
            self.prefs.recent.truncate(12);
            self.save_prefs();
        }
        self.page = page;
        self.ensure_page();
    }

    /// Switch page; load only what it shows that's missing or stale (see `ttl`).
    pub fn show(&mut self, page: Page) {
        self.remember(&Place::new(page, self.cur.clone(), self.group_cur.clone()));
        self.page = page;
        self.ensure_page();
    }

    /// Load what the page shows only where it's missing or older than its `ttl`: opening a
    /// page, going back, and the 30 s tick. Fresh data is shown as is, with no request.
    pub fn ensure_page(&mut self) {
        let ts = |d: Option<&RepoData>, f: fn(&RepoData, i64) -> bool, ttl: i64| d.is_none_or(|d| f(d, ttl));
        match self.page {
            Page::Inbox => {
                if self.inbox.stale(ttl::GH) {
                    self.refresh_inbox();
                }
                if self.notifs.stale(ttl::GH) {
                    self.refresh_notifications();
                }
                if self.pulse.stale(ttl::PULSE) || (!self.pulse.loading && self.pulse_key != self.pulse_key_now()) {
                    self.refresh_pulse();
                }
                return;
            }
            Page::Group => return self.load_group(true),
            _ => {}
        }
        let d = self.rd();
        let (meta, status) = (ts(d, |d, t| d.meta.stale(t), ttl::META), ts(d, |d, t| d.status.stale(t), ttl::LOCAL));
        if meta {
            self.refresh_meta();
        }
        if status && self.local_path().is_some() {
            self.refresh_status();
        }
        let d = self.rd();
        match self.page {
            Page::Pulls => {
                let other_state = d.is_some_and(|d| d.prs_state != self.pr_state && d.prs.data.is_some() && !d.prs.loading);
                if other_state || ts(d, |d, t| d.prs.stale(t), ttl::GH) {
                    self.load_prs();
                }
                if self.pr_sel.is_some() && self.pr_detail.stale(ttl::DETAIL) {
                    self.load_pr_detail();
                }
            }
            Page::Actions => {
                let runs_ttl = if self.runs_running() { ttl::RUNNING } else { ttl::RUNS };
                if ts(d, |d, t| d.runs.stale(t), runs_ttl) {
                    self.load_runs();
                }
                let sel_running = self.run_sel.is_some_and(|id| self.run_running(id));
                if self.run_sel.is_some() && self.run_detail.stale(if sel_running { ttl::RUNNING } else { ttl::DETAIL }) {
                    self.load_run_detail();
                }
            }
            Page::Envs => {
                if ts(d, |d, t| d.envs.stale(t), ttl::GH) {
                    self.refresh_envs();
                }
            }
            Page::Commits => {
                let other = d.is_some_and(|d| !d.graph.loading && d.graph.data.as_ref().is_some_and(|g| g.all != self.all_branches));
                if other || ts(d, |d, t| d.graph.stale(t), ttl::LOCAL) {
                    self.refresh_graph();
                }
                // A commit never changes: its detail is loaded once.
            }
            Page::Branches => {
                let (branches, tools) = (ts(d, |d, t| d.branches.stale(t), ttl::LOCAL), ts(d, |d, t| d.stashes.stale(t) || d.cleanup.stale(t), ttl::LOCAL));
                if branches {
                    self.refresh_branches();
                }
                if tools {
                    self.refresh_branch_tools();
                }
            }
            _ => {}
        }
    }

    /// Any run on the current repo's list still running?
    pub fn runs_running(&self) -> bool {
        self.rd().and_then(|d| d.runs.data.as_ref()).is_some_and(|r| r.iter().any(|r| r.outcome().is_running()))
    }

    fn run_running(&self, id: u64) -> bool {
        self.rd().and_then(|d| d.runs.data.as_ref()).is_some_and(|r| r.iter().any(|r| r.database_id == id && r.outcome().is_running()))
    }

    /// Park the open PR / run / commit detail before switching repo.
    fn stash_details(&mut self) {
        let Some(slug) = self.cur.clone() else { return };
        stash(&mut self.pr_memo, &mut self.pr_detail, self.pr_sel.map(|n| (slug.clone(), n)));
        stash(&mut self.run_memo, &mut self.run_detail, self.run_sel.map(|n| (slug.clone(), n)));
        stash(&mut self.commit_memo, &mut self.commit_detail, self.commit_sel.clone().map(|c| (slug, c)));
    }

    /// Reload everything the page shows (Ctrl+R, the refresh button, after an action).
    pub fn refresh_page(&mut self) {
        match self.page {
            Page::Inbox => {
                self.refresh_inbox();
                self.refresh_notifications();
                self.refresh_pulse();
            }
            Page::Pulls => self.refresh_prs(),
            Page::Actions => self.refresh_runs(),
            Page::Envs => self.refresh_envs(),
            Page::Commits => self.refresh_graph(),
            Page::Branches => {
                self.refresh_branches();
                self.refresh_branch_tools();
            }
            Page::Group => self.load_group(false),
        }
        if !matches!(self.page, Page::Inbox | Page::Group) {
            self.refresh_status();
        }
    }

    fn refresh_repo_list(&mut self) {
        self.inbox.loading = true;
        jobs::spawn(self, gh::all_repos, |s, r| match r {
            Ok((me, mut list)) => {
                s.me = me;
                list.sort_by(|a, b| b.pushed_at.cmp(&a.pushed_at));
                for remote in &list {
                    s.add_repo(remote.name_with_owner.clone(), None);
                    if let Some(r) = s.repos.iter_mut().find(|r| r.slug == remote.name_with_owner) {
                        r.pushed_at = remote.pushed_at.clone();
                        r.archived = remote.is_archived;
                    }
                }
                s.refresh_inbox();
                s.refresh_notifications();
                s.refresh_pulse();
            }
            Err(e) => {
                s.inbox.finish(Err(e.clone()));
                s.toast(format!("gh: {e}"), true);
            }
        });
    }

    pub fn refresh_inbox(&mut self) {
        // Recent runs come from the repos you pushed to most recently.
        let mut by_push: Vec<&Repo> = self.repos.iter().filter(|r| !r.pushed_at.is_empty()).collect();
        by_push.sort_by(|a, b| b.pushed_at.cmp(&a.pushed_at));
        let active: Vec<String> = by_push.into_iter().take(6).map(|r| r.slug.clone()).collect();
        self.inbox.loading = true;
        jobs::spawn(self, move || gh::inbox(active), |s, r| {
            let ok = r.is_ok();
            s.inbox.finish(r);
            if ok {
                s.refresh_queue();
                s.save_cache();
            }
        });
    }

    // -----------------------------------------------------------------------------------------
    // Activity (tracked repos over a time range)
    // -----------------------------------------------------------------------------------------

    /// The repos you track: your list, or by default your pinned repos plus your groups' repos.
    pub fn tracked(&self) -> Vec<String> {
        if !self.prefs.tracked.is_empty() {
            return self.prefs.tracked.clone();
        }
        let mut v: Vec<String> = self.prefs.pinned.clone();
        for g in &self.prefs.groups {
            for r in &g.repos {
                if !v.contains(r) {
                    v.push(r.clone());
                }
            }
        }
        v
    }

    /// The repos the Activity page shows now (all tracked, or one of your groups).
    pub fn pulse_repos(&self) -> Vec<String> {
        match &self.pulse_scope {
            Some(g) => self.prefs.groups.iter().find(|x| x.name == *g).map(|x| x.repos.clone()).unwrap_or_default(),
            None => self.tracked(),
        }
    }

    pub fn is_tracked(&self, slug: &str) -> bool {
        self.tracked().iter().any(|t| t == slug)
    }

    pub fn toggle_tracked(&mut self, slug: &str) {
        let before = self.prefs.clone();
        let mut list = self.tracked();
        let now_tracked = match list.iter().position(|t| t == slug) {
            Some(i) => {
                list.remove(i);
                false
            }
            None => {
                list.push(slug.to_owned());
                true
            }
        };
        self.prefs.tracked = list;
        self.save_prefs();
        self.toast_undo(format!("{} {slug}", if now_tracked { "Tracking" } else { "Stopped tracking" }), Some(guard::Undo::Prefs(Box::new(before))));
        self.refresh_pulse();
    }

    pub fn reset_tracked(&mut self) {
        let before = self.prefs.clone();
        self.prefs.tracked.clear();
        self.save_prefs();
        self.toast_undo("Tracking your pinned repos and your groups again".into(), Some(guard::Undo::Prefs(Box::new(before))));
        self.refresh_pulse();
    }

    pub fn pulse_range(&self) -> pulse::Range {
        let parse = |t: &TextEdit| chrono::NaiveDate::parse_from_str(t.text.trim(), "%Y-%m-%d").ok();
        let custom = match (parse(&self.pulse_from), parse(&self.pulse_to)) {
            (Some(a), Some(b)) => Some((a, b)),
            (Some(a), None) => Some((a, a)),
            _ => None,
        };
        pulse::local_range(self.pulse_preset, custom)
    }

    /// What the Activity page asks for now (scope, range, repos): the loaded pulse answers it
    /// only when its key matches.
    pub fn pulse_key_now(&self) -> String {
        // Which page this is: scope, preset, where the range starts, and the repos. Not where it
        // ends: for "this week" that's now, which moves every second, and a key that changes on
        // its own would never match the cache and would reload the page from scratch.
        let r = self.pulse_range();
        let end = if self.pulse_preset == pulse::Preset::Custom { format!("{}..{}", self.pulse_from.text.trim(), self.pulse_to.text.trim()) } else { String::new() };
        format!("{:?}|{:?}|{}|{end}|{}", self.pulse_scope, self.pulse_preset, r.since, self.pulse_repos().join(","))
    }

    pub fn refresh_pulse(&mut self) {
        let repos = self.pulse_repos();
        let range = self.pulse_range();
        let key = self.pulse_key_now();
        // What's known stays on screen, unchanged, while the fresh page is built aside: last
        // session's (the cache), or this session's for that scope and range. Only a range never
        // loaded has nothing to show, and fills in piece by piece instead.
        if self.pulse_key != key {
            match self.pulse_seen.get(&key) {
                Some((at, known)) => (self.pulse.data, self.pulse.at, self.pulse.cached) = (Some(known.clone()), Some(*at), false),
                None => (self.pulse.data, self.pulse.at) = (Some(pulse::Pulse::starting(&repos, &range)), None),
            }
        } else if self.pulse.data.is_none() {
            self.pulse.data = Some(pulse::Pulse::starting(&repos, &range));
        }
        self.pulse_key = key.clone();
        self.pulse.loading = true;
        self.pulse.error = None;
        self.pulse_next = Some(pulse::Pulse::starting(&repos, &range));
        let parts = pulse::parts(&repos, &range);
        let left = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(parts.len()));
        if parts.is_empty() {
            self.pulse_next = None;
            self.pulse.finish(Ok(pulse::Pulse::starting(&repos, &range)));
            return;
        }
        jobs::spawn_each(self, parts, move |s, part| {
            if s.pulse_key != key {
                return false; // an answer for a scope or range you already left
            }
            // With nothing known yet, the page shows pieces as they land, easing into place.
            let progressive = s.pulse.data.as_ref().is_some_and(|p| !p.complete());
            if let Some(shown) = s.pulse.data.as_mut().filter(|_| progressive) {
                shown.apply(part.clone());
            }
            if let Some(next) = s.pulse_next.as_mut() {
                next.apply(part);
            }
            if left.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) != 1 {
                return progressive;
            }
            // All in: replace what's shown only if something differs, and animate that.
            let Some(next) = s.pulse_next.take() else { return false };
            let changed = s.pulse.data.as_ref() != Some(&next);
            let t = model::now();
            if changed {
                s.pulse.data = Some(next.clone());
            }
            if s.pulse_seen.len() >= 8 && !s.pulse_seen.contains_key(&key) {
                if let Some(oldest) = s.pulse_seen.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) {
                    s.pulse_seen.remove(&oldest);
                }
            }
            s.pulse_seen.insert(key.clone(), (t, next));
            s.pulse.loading = false;
            s.pulse.at = Some(t);
            s.pulse.cached = false;
            s.save_cache();
            changed || progressive
        });
    }

    pub fn set_pulse_preset(&mut self, p: pulse::Preset) {
        self.pulse_preset = p;
        if p != pulse::Preset::Custom {
            self.refresh_pulse();
        } else if self.pulse_from.text.is_empty() {
            // Start the custom range on this week, ready to edit.
            let r = pulse::local_range(pulse::Preset::Week, None);
            let d = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|x| x.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string()).unwrap_or_default();
            self.pulse_from = TextEdit::new(d(r.since));
            self.pulse_to = TextEdit::new(d(r.until));
            self.refresh_pulse();
        }
    }

    /// Default branch, protected branches, admin (A2): badges and merge checks.
    pub fn refresh_meta(&mut self) {
        let Some(slug) = self.cur.clone() else { return };
        self.rd_mut(&slug).meta.loading = true;
        let key = slug.clone();
        jobs::spawn(self, move || gh::repo_meta(&key), move |s, r| s.rd_mut(&slug).meta.finish(r));
    }

    /// Is `name` (local or `origin/…`) protected or the default branch?
    pub fn is_protected(&self, name: &str) -> bool {
        let short = name.strip_prefix("origin/").unwrap_or(name);
        self.rd().and_then(|d| d.meta.data.as_ref()).is_some_and(|m| m.default_branch == short || m.protected.iter().any(|p| p == short))
    }

    pub fn refresh_status(&mut self) {
        let (Some(slug), Some(dir)) = (self.cur.clone(), self.local_path()) else { return };
        self.rd_mut(&slug).status.loading = true;
        jobs::spawn(self, move || git::status(&dir), move |s, r| s.rd_mut(&slug).status.finish(r));
    }

    /// Reload the PR list (and the open PR).
    pub fn refresh_prs(&mut self) {
        self.load_prs();
        if self.pr_sel.is_some() {
            self.load_pr_detail();
        }
    }

    fn load_prs(&mut self) {
        let Some(slug) = self.cur.clone() else { return };
        let state = PR_STATES[self.pr_state];
        let limit = self.pr_limit;
        self.rd_mut(&slug).prs.loading = true;
        let key = slug.clone();
        jobs::spawn(self, move || gh::prs(&key, state, limit, None), move |s, r| {
            // A late answer for a filter the user already left is dropped.
            if PR_STATES[s.pr_state] == state {
                let d = s.rd_mut(&slug);
                d.prs.finish(r);
                d.prs_state = PR_STATES.iter().position(|x| *x == state).unwrap_or(0);
            }
        });
        self.search_prs();
    }

    pub fn select_pr(&mut self, n: u64) {
        if self.pr_sel != Some(n) {
            let Some(slug) = self.cur.clone() else { return };
            stash(&mut self.pr_memo, &mut self.pr_detail, self.pr_sel.map(|p| (slug.clone(), p)));
            self.pr_detail = self.pr_memo.remove(&(slug, n)).unwrap_or_default();
            self.pr_sel = Some(n);
            self.pr_tab = 0;
            self.prs_table.select(n.to_string());
            if self.pr_detail.stale(ttl::DETAIL) {
                self.load_pr_detail();
            }
        }
    }

    fn load_pr_detail(&mut self) {
        let (Some(slug), Some(n)) = (self.cur.clone(), self.pr_sel) else { return };
        self.pr_detail.loading = true;
        let key = slug.clone();
        jobs::spawn(self, move || gh::pr_detail(&key, n), move |s, r| {
            if s.cur.as_deref() == Some(&slug) && s.pr_sel == Some(n) {
                s.pr_detail.finish(r);
            }
        });
    }

    /// Reload the run list (and the open run).
    pub fn refresh_runs(&mut self) {
        self.load_runs();
        if self.run_sel.is_some() {
            self.load_run_detail();
        }
    }

    fn load_runs(&mut self) {
        let Some(slug) = self.cur.clone() else { return };
        self.rd_mut(&slug).runs.loading = true;
        let key = slug.clone();
        let limit = self.run_limit;
        jobs::spawn(self, move || gh::runs(&key, limit), move |s, r| {
            s.rd_mut(&slug).runs.finish(r);
            s.keep_focused_run();
        });
    }

    pub fn select_run(&mut self, id: u64) {
        if self.run_sel != Some(id) {
            let Some(slug) = self.cur.clone() else { return };
            stash(&mut self.run_memo, &mut self.run_detail, self.run_sel.map(|r| (slug.clone(), r)));
            self.run_detail = self.run_memo.remove(&(slug, id)).unwrap_or_default();
            self.run_sel = Some(id);
            self.runs_table.select(id.to_string());
            if self.run_detail.stale(if self.run_running(id) { ttl::RUNNING } else { ttl::DETAIL }) {
                self.load_run_detail();
            }
        }
    }

    fn load_run_detail(&mut self) {
        let (Some(slug), Some(id)) = (self.cur.clone(), self.run_sel) else { return };
        self.run_detail.loading = true;
        let key = slug.clone();
        jobs::spawn(self, move || gh::run_detail(&key, id), move |s, r| {
            if s.cur.as_deref() == Some(&slug) && s.run_sel == Some(id) {
                s.run_detail.finish(r);
            }
        });
    }

    pub fn refresh_envs(&mut self) {
        let Some(slug) = self.cur.clone() else { return };
        self.rd_mut(&slug).envs.loading = true;
        let key = slug.clone();
        jobs::spawn(self, move || gh::envs(&key), move |s, r| s.rd_mut(&slug).envs.finish(r));
    }

    pub fn refresh_graph(&mut self) {
        let (Some(slug), Some(dir)) = (self.cur.clone(), self.local_path()) else { return };
        let all = self.all_branches;
        let limit = self.commit_limit;
        self.rd_mut(&slug).graph.loading = true;
        jobs::spawn(
            self,
            move || {
                git::log(&dir, all, limit).map(|commits| {
                    let (rows, width) = git::layout(&commits);
                    CommitGraph { all, commits, rows, width }
                })
            },
            move |s, r| {
                if s.all_branches == all {
                    s.rd_mut(&slug).graph.finish(r);
                }
            },
        );
    }

    pub fn select_commit(&mut self, sha: &str) {
        if self.commit_sel.as_deref() == Some(sha) {
            return;
        }
        let (Some(dir), Some(slug)) = (self.local_path(), self.cur.clone()) else { return };
        stash(&mut self.commit_memo, &mut self.commit_detail, self.commit_sel.clone().map(|c| (slug.clone(), c)));
        self.commit_sel = Some(sha.to_owned());
        if let Some(memo) = self.commit_memo.remove(&(slug, sha.to_owned())) {
            self.commit_detail = memo;
            return; // a commit never changes
        }
        self.commit_detail = Load::start();
        let sha = sha.to_owned();
        let key = sha.clone();
        jobs::spawn(self, move || git::commit_detail(&dir, &key), move |s, r| {
            if s.commit_sel.as_deref() == Some(&sha) {
                s.commit_detail.finish(r);
            }
        });
    }

    pub fn refresh_branches(&mut self) {
        let (Some(slug), Some(dir)) = (self.cur.clone(), self.local_path()) else { return };
        self.rd_mut(&slug).branches.loading = true;
        jobs::spawn(self, move || git::branches(&dir), move |s, r| s.rd_mut(&slug).branches.finish(r));
    }

    // -----------------------------------------------------------------------------------------
    // Search as you type: the box filters what's loaded on every keystroke; after a 300 ms pause
    // GitHub is searched too, for what isn't loaded
    // -----------------------------------------------------------------------------------------

    pub fn search_repos(&mut self) {
        let q = self.repo_filter.text.trim().to_owned();
        if q.chars().count() < 2 {
            jobs::cancel(&self.repo_search);
            self.repo_searching = false;
            self.repo_hits = (String::new(), Vec::new());
            return;
        }
        self.repo_searching = true;
        let key = q.clone();
        let latest = self.repo_search.clone();
        jobs::debounced(self, &latest, Duration::from_millis(300), move || gh::search_repos(&key), move |s, r| {
            s.repo_searching = false;
            match r {
                Ok(hits) => s.repo_hits = (q, hits),
                Err(e) => s.toast(format!("Repo search failed: {e}"), true),
            }
        });
    }

    pub fn search_prs(&mut self) {
        let q = self.pr_filter.text.trim().to_owned();
        let Some(slug) = self.cur.clone().filter(|_| !q.is_empty()) else {
            jobs::cancel(&self.pr_search);
            self.pr_searching = false;
            self.pr_hits = None;
            return;
        };
        let state = self.pr_state;
        let key = (slug.clone(), state, q.clone());
        self.pr_searching = true;
        let latest = self.pr_search.clone();
        jobs::debounced(
            self,
            &latest,
            Duration::from_millis(300),
            move || gh::prs(&slug, PR_STATES[state], 100, Some(&q)),
            move |s, r| {
                s.pr_searching = false;
                match r {
                    Ok(prs) => s.pr_hits = Some((key, prs)),
                    Err(e) => s.toast(format!("PR search failed: {e}"), true),
                }
            },
        );
    }

    pub fn more_prs(&mut self) {
        self.pr_limit += PR_PAGE;
        self.refresh_prs();
    }

    pub fn more_runs(&mut self) {
        self.run_limit += RUN_PAGE;
        self.refresh_runs();
    }

    pub fn more_commits(&mut self) {
        self.commit_limit += COMMIT_PAGE;
        self.refresh_graph();
    }

    // -----------------------------------------------------------------------------------------
    // Actions: each runs in the background, then toasts and refreshes what it changed
    // -----------------------------------------------------------------------------------------

    pub fn act(
        &mut self,
        label: String,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
        after: impl FnOnce(&mut State) + Send + 'static,
    ) {
        if self.busy.contains(&label) {
            return; // a double click doesn't run it twice
        }
        self.busy.push(label.clone());
        jobs::spawn(self, work, move |s, r| {
            s.busy.retain(|b| *b != label);
            match r {
                Ok(out) => {
                    let line = out.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_owned);
                    s.toast(line.unwrap_or_else(|| format!("{label}: done")), false);
                    after(s);
                }
                Err(e) => s.toast(format!("{label} failed: {e}"), true),
            }
        });
    }

    // -----------------------------------------------------------------------------------------
    // Theme, pins and groups (local only)
    // -----------------------------------------------------------------------------------------

    pub fn save_prefs(&mut self) {
        // Headless stills and tests never touch your real settings.
        if jobs::is_inline_run() {
            return;
        }
        if let Err(e) = config::save_prefs(&self.prefs) {
            self.toast(format!("Couldn't save preferences: {e}"), true);
        }
    }

    pub fn set_theme(&mut self, t: config::Theme) {
        self.prefs.theme = t;
        self.save_prefs();
    }

    pub fn toggle_pin(&mut self, slug: &str) {
        let before = self.prefs.clone();
        let pinned = !self.prefs.pinned.iter().any(|p| p == slug);
        self.toast_undo(format!("{} {slug}", if pinned { "Pinned" } else { "Unpinned" }), Some(guard::Undo::Prefs(Box::new(before))));
        match self.prefs.pinned.iter().position(|p| p == slug) {
            Some(i) => {
                self.prefs.pinned.remove(i);
            }
            None => self.prefs.pinned.push(slug.to_owned()),
        }
        self.save_prefs();
    }

    pub fn toggle_in_group(&mut self, group: &str, slug: &str) {
        let before = self.prefs.clone();
        self.toast_undo(format!("Updated group {group}"), Some(guard::Undo::Prefs(Box::new(before))));
        let Some(g) = self.prefs.groups.iter_mut().find(|g| g.name == group) else { return };
        match g.repos.iter().position(|r| r == slug) {
            Some(i) => {
                g.repos.remove(i);
            }
            None => g.repos.push(slug.to_owned()),
        }
        self.save_prefs();
    }

    // -----------------------------------------------------------------------------------------
    // Folding and manual order
    // -----------------------------------------------------------------------------------------

    /// Fold or unfold one sidebar section; remembered across launches.
    pub fn toggle_fold(&mut self, key: &str) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_owned());
        }
        self.save_folds();
    }

    /// Collapse all when anything is open, otherwise expand all.
    pub fn toggle_fold_all(&mut self, keys: Vec<String>) {
        if keys.iter().any(|k| !self.collapsed.contains(k)) {
            self.collapsed.extend(keys);
        } else {
            self.collapsed.clear();
        }
        self.save_folds();
    }

    fn save_folds(&mut self) {
        let mut v: Vec<String> = self.collapsed.iter().cloned().collect();
        v.sort();
        self.prefs.collapsed = v;
        self.save_prefs();
    }

    /// Owners in sidebar order: yours first (`owner_order`), then the rest, you first then A–Z.
    pub fn owners_sorted(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for r in &self.repos {
            let o = r.owner();
            if !seen.iter().any(|x| x == o) {
                seen.push(o.to_owned());
            }
        }
        seen.sort_by_key(|o| (*o != self.me, o.to_lowercase()));
        let mut out: Vec<String> = self.prefs.owner_order.iter().filter(|o| seen.contains(o)).cloned().collect();
        out.extend(seen.into_iter().filter(|o| !self.prefs.owner_order.contains(o)));
        out
    }

    /// Move an owner section up (-1) or down (+1), or to the top (`None`).
    pub fn move_owner(&mut self, owner: &str, by: Option<isize>) {
        let before = self.prefs.clone();
        let mut order = self.owners_sorted();
        let Some(i) = order.iter().position(|o| o == owner) else { return };
        match by {
            None => {
                let x = order.remove(i);
                order.insert(0, x);
            }
            Some(by) => {
                let j = i as isize + by;
                if j < 0 || j as usize >= order.len() {
                    return;
                }
                order.swap(i, j as usize);
            }
        }
        self.prefs.owner_order = order;
        self.save_prefs();
        self.toast_undo(format!("Moved {owner}"), Some(guard::Undo::Prefs(Box::new(before))));
    }

    pub fn reset_owner_order(&mut self) {
        let before = self.prefs.clone();
        self.prefs.owner_order.clear();
        self.save_prefs();
        self.toast_undo("Organizations back in the default order (you first, then A–Z)".into(), Some(guard::Undo::Prefs(Box::new(before))));
    }

    pub fn owner_style(&self, owner: &str) -> Option<&config::OwnerStyle> {
        self.prefs.owner_styles.iter().find(|s| s.owner == owner && (!s.color.is_empty() || !s.icon.is_empty()))
    }

    pub fn open_owner_edit(&mut self, owner: &str) {
        let st = self.prefs.owner_styles.iter().find(|s| s.owner == owner).cloned().unwrap_or_default();
        self.owner_icon = TextEdit::new(st.icon);
        self.owner_hex = TextEdit::new(st.color.clone());
        self.owner_edit = Some((owner.to_owned(), st.color));
    }

    /// Save the dialog. `reset`: back to the default look.
    pub fn save_owner_edit(&mut self, reset: bool) {
        let Some((owner, color)) = self.owner_edit.take() else { return };
        // A typed hex wins over the swatch when it's valid.
        let typed = self.owner_hex.text.trim().to_owned();
        let typed = if typed.is_empty() || typed.starts_with('#') { typed } else { format!("#{typed}") };
        let color = if rune::color::hex(&typed).is_some() && typed.len() == 7 { typed.to_lowercase() } else { color };
        let icon: String = self.owner_icon.text.trim().chars().take(2).collect();
        let before = self.prefs.clone();
        self.prefs.owner_styles.retain(|s| s.owner != owner);
        if !reset && (!color.is_empty() || !icon.is_empty()) {
            self.prefs.owner_styles.push(config::OwnerStyle { owner: owner.clone(), color, icon });
        }
        self.save_prefs();
        self.toast_undo(if reset { format!("{owner}: default look") } else { format!("{owner}: saved") }, Some(guard::Undo::Prefs(Box::new(before))));
    }

    /// A pinned repo dragged from `from` to `to`.
    pub fn move_pinned(&mut self, from: usize, to: usize) {
        if from < self.prefs.pinned.len() && to < self.prefs.pinned.len() && from != to {
            let x = self.prefs.pinned.remove(from);
            self.prefs.pinned.insert(to, x);
            self.save_prefs();
        }
    }

    /// A repo inside group number `g` dragged from `from` to `to`.
    pub fn move_in_group(&mut self, g: usize, from: usize, to: usize) {
        let Some(group) = self.prefs.groups.get_mut(g) else { return };
        if from < group.repos.len() && to < group.repos.len() && from != to {
            let x = group.repos.remove(from);
            group.repos.insert(to, x);
            self.save_prefs();
        }
    }

    /// Move a whole group up (-1) or down (+1) among your groups.
    pub fn move_group(&mut self, name: &str, by: isize) {
        let Some(i) = self.prefs.groups.iter().position(|g| g.name == name) else { return };
        let j = i as isize + by;
        if j < 0 || j as usize >= self.prefs.groups.len() {
            return;
        }
        self.prefs.groups.swap(i, j as usize);
        // The drag slots follow their groups.
        self.group_drags.swap(i.min(MAX_DRAG_GROUPS - 1), (j as usize).min(MAX_DRAG_GROUPS - 1));
        self.save_prefs();
    }

    pub fn delete_group(&mut self, name: &str) {
        let before = self.prefs.clone();
        self.toast_undo(format!("Deleted group {name}"), Some(guard::Undo::Prefs(Box::new(before))));
        self.prefs.groups.retain(|g| g.name != name);
        self.collapsed.remove(&format!("g:{name}"));
        self.save_prefs();
    }

    pub fn open_group_dialog(&mut self, rename: Option<String>, add: Option<String>) {
        self.group_name = TextEdit::new(rename.clone().unwrap_or_default());
        self.group_name.focused = true;
        self.group_edit = Some(GroupEdit { rename, add });
    }

    /// Save the group dialog: a new group (with the repo it came from), or a rename.
    pub fn finish_group_edit(&mut self) {
        let Some(edit) = self.group_edit.take() else { return };
        let name = self.group_name.text.trim().to_owned();
        let taken = self.prefs.groups.iter().any(|g| g.name == name && Some(&g.name) != edit.rename.as_ref());
        if name.is_empty() || taken {
            self.toast(if taken { format!("There's already a group called {name}") } else { "A group needs a name".into() }, true);
            self.group_edit = Some(edit);
            return;
        }
        match edit.rename {
            Some(old) => {
                if let Some(g) = self.prefs.groups.iter_mut().find(|g| g.name == old) {
                    g.name = name.clone();
                }
                if self.collapsed.remove(&format!("g:{old}")) {
                    self.collapsed.insert(format!("g:{name}"));
                }
            }
            None => self.prefs.groups.push(config::Group { name, repos: edit.add.into_iter().collect() }),
        }
        self.save_prefs();
    }

    pub fn toast(&mut self, text: String, error: bool) {
        self.next_toast += 1;
        self.toasts.push(Toast { id: self.next_toast, text, error, born: Instant::now(), undo: None });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /// A success toast that may carry an Undo button.
    pub fn toast_undo(&mut self, text: String, undo: Option<guard::Undo>) {
        self.next_toast += 1;
        self.toasts.push(Toast { id: self.next_toast, text, error: false, born: Instant::now(), undo });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    pub fn fetch(&mut self) {
        let Some(dir) = self.local_path() else { return };
        self.request(guard::Action::Fetch { dir });
    }

    pub fn pull(&mut self) {
        let Some(dir) = self.local_path() else { return };
        self.request(guard::Action::Pull { dir });
    }

    pub fn checkout_branch(&mut self, name: String, remote: bool) {
        let (Some(dir), Some(slug)) = (self.local_path(), self.cur.clone()) else { return };
        self.request(guard::Action::Checkout { dir, slug, name, remote });
    }

    /// Settings: save the edited production names and sensitive patterns.
    pub fn save_safety_settings(&mut self) {
        self.prefs.prod_names = self.settings_prod.text.split(',').map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect();
        self.prefs.sensitive = self.settings_sensitive.text.lines().map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect();
        self.save_prefs();
        self.toast("Safety settings saved".into(), false);
    }

    fn add_local_repo(&mut self, dir: PathBuf) {
        let Some(top) = git::toplevel(&dir) else {
            self.toast(format!("{} isn't a git repository", dir.display()), true);
            return;
        };
        let Some(slug) = git::slug_of(&top) else {
            self.toast("Its `origin` remote isn't on GitHub".into(), true);
            return;
        };
        self.add_repo(slug.clone(), Some(top));
        let saved: Vec<config::Saved> =
            self.repos.iter().filter_map(|r| Some(config::Saved { slug: r.slug.clone(), path: r.path.clone()? })).collect();
        if let Err(e) = config::save(&saved) {
            self.toast(format!("Couldn't save the repo list: {e}"), true);
        }
        self.select_repo(&slug, Page::Commits);
    }
}

/// "Add local repo…" / "Locate local clone…": the OS folder picker.
pub fn ask_local_repo(s: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions { title: "Choose a local git repository".into(), directory: s.local_path(), ..Default::default() };
    tx.open_folder(options, |s: &mut State, r, _| match r {
        Ok(Some(dir)) => s.add_local_repo(dir),
        Ok(None) => {}
        Err(e) => s.toast(e.to_string(), true),
    });
}

pub fn open_url(tx: &mut Tx, url: &str) {
    if !url.is_empty() {
        let _ = tx.open_url(url);
    }
}

// ---------------------------------------------------------------------------------------------
// Frame hooks
// ---------------------------------------------------------------------------------------------

/// Timers: toasts fade (5 s, 10 s with Undo); a merge's grace period runs out; the page reloads
/// what went stale (`ttl`: runs every 15 s while one is running); watches every 30 s;
/// notifications every 60 s; the cache is saved every 3 minutes.
fn poll(s: &mut State, _: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    let before = s.toasts.len();
    s.toasts.retain(|t| t.born.elapsed() < Duration::from_secs(if t.undo.is_some() { 10 } else { 5 }));
    changed |= s.toasts.len() != before;
    if s.grace.is_some() {
        s.tick_grace();
        changed = true; // the countdown redraws every tick
        tx.wake_after(Duration::from_millis(250));
    }
    if !s.watches.is_empty() && s.last_watch.elapsed() > Duration::from_secs(30) {
        s.poll_watches();
    }
    if s.last_notif.elapsed() > Duration::from_secs(60) {
        s.refresh_notifications();
    }
    if s.last_cache.elapsed() > Duration::from_secs(180) {
        s.save_cache();
    }
    // Keep the timers above ticking even when nothing else wakes the window.
    tx.wake_after(Duration::from_secs(if s.watches.is_empty() { 30 } else { 10 }));
    if !s.window_icon_set {
        s.window_icon_set = window_icon::apply();
    }
    changed |= s.poll_setup();
    if s.reveal.is_some() || s.want_failed.is_some() {
        changed |= s.poll_focus(tx);
        tx.wake_after(Duration::from_millis(200)); // until its row is in
    }
    if matches!(s.setup, onboarding::Setup::Missing(_)) {
        // Waiting on you to install or sign in: re-check on time.
        tx.wake_after(Duration::from_secs(1));
    }
    let running = s.page == Page::Actions && s.runs_running();
    if s.last_tick.elapsed() > Duration::from_secs(if running { 5 } else { 30 }) {
        s.last_tick = Instant::now();
        s.ensure_page();
        changed = true; // "updated 2m ago" moves on
    }
    if !s.toasts.is_empty() || running {
        tx.wake_after(Duration::from_secs(1));
    }
    changed
}

/// Ctrl+K palette, F1 guide, Ctrl+R refresh, Ctrl+1…6 pages (Cmd on macOS); on the Inbox,
/// J/K/Enter/A/C work the review queue (only when no text field has focus: fields take letters
/// first).
fn on_key(s: &mut State, ev: &KeyEvent, _: &mut Tx) -> bool {
    use rune::Key;
    if ev.is_command('k') {
        s.open_palette();
        return true;
    }
    if (ev.mods.alt && ev.key == Key::Left) || ev.is_command('[') {
        s.go_back();
        return true;
    }
    if (ev.mods.alt && ev.key == Key::Right) || ev.is_command(']') {
        s.go_forward();
        return true;
    }
    if ev.is_command(',') {
        s.open_settings(s.settings_sec);
        return true;
    }
    if ev.key == Key::Function(1) || (ev.is_command('/')) {
        s.guide_open = true;
        return true;
    }
    if ev.key == Key::Escape {
        if s.palette {
            s.palette = false;
            return true;
        }
        if s.settings_open {
            s.close_settings();
            return true;
        }
        if s.grace.is_some() {
            s.cancel_grace();
            return true;
        }
    }
    if s.page == Page::Inbox && s.inbox_tab == 1 && !ev.mods.cmd && !ev.mods.alt && s.pending.is_none() && !s.palette {
        let c = match &ev.key {
            Key::Text(t) if t.len() == 1 => t.chars().next().map(|c| c.to_ascii_lowercase()),
            Key::Enter => Some('\n'),
            _ => None,
        };
        if let Some(c) = c
            && s.queue_key(c)
        {
            return true;
        }
    }
    if ev.is_command('r') {
        s.refresh_page();
        return true;
    }
    let pages = [Page::Inbox, Page::Pulls, Page::Actions, Page::Envs, Page::Commits, Page::Branches];
    for (i, p) in pages.into_iter().enumerate() {
        if ev.is_command(char::from(b'1' + i as u8)) && (p == Page::Inbox || s.cur.is_some()) {
            s.show(p);
            return true;
        }
    }
    false
}

fn view(s: &State, cx: &mut Cx<State>) -> El {
    ui::view(s, cx)
}

fn main() {
    let spec = AppSpec {
        scenes: &[
            ("inbox", |s: &mut State| s.show(Page::Inbox)),
            ("prs", |s: &mut State| {
                s.select_repo("anomalyco/rune", Page::Pulls);
                if let Some(n) = s.rd().and_then(|d| d.prs.data.as_ref()).and_then(|p| p.first()).map(|p| p.number) {
                    s.select_pr(n);
                }
            }),
            ("actions", |s: &mut State| {
                s.select_repo("anomalyco/rune", Page::Actions);
                if let Some(id) = s.rd().and_then(|d| d.runs.data.as_ref()).and_then(|r| r.first()).map(|r| r.database_id) {
                    s.select_run(id);
                }
            }),
            ("envs", |s: &mut State| s.select_repo("anomalyco/rune", Page::Envs)),
            ("groups", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                s.prefs.pinned = vec!["anomalyco/rune".into(), "cli/cli".into()];
                s.prefs.groups = vec![config::Group {
                    name: "Rust tooling".into(),
                    repos: vec!["rust-lang/cargo".into(), "rust-lang/rustup".into(), "tokio-rs/tokio".into()],
                }];
                s.collapsed.insert(format!("o:{}", s.me));
                s.menu = Some((MenuTarget::Repo("rust-lang/cargo".into()), PointerPress { x: 210.0, y: 330.0 }));
            }),
            ("orgs", |s: &mut State| {
                s.prefs.owner_order = vec!["rust-lang".into(), "tokio-rs".into()];
                s.prefs.owner_styles = vec![
                    config::OwnerStyle { owner: "rust-lang".into(), color: "#e0a030".into(), icon: "🦀".into() },
                    config::OwnerStyle { owner: "tokio-rs".into(), color: "#7a5cff".into(), icon: "⚡".into() },
                    config::OwnerStyle { owner: "cli".into(), color: "#2fa37a".into(), icon: String::new() },
                ];
                s.prefs.pinned = vec!["rust-lang/cargo".into(), "cli/cli".into()];
                s.collapsed = ["o:rust-lang", "o:tokio-rs", "o:anomalyco", "o:cli"].map(String::from).into_iter().collect();
                s.open_owner_edit("tokio-rs");
            }),
            ("history", |s: &mut State| {
                s.show(Page::Inbox);
                s.select_repo("anomalyco/rune", Page::Pulls);
                s.show(Page::Actions);
                s.go_back();
            }),
            ("activity", |s: &mut State| {
                s.prefs.tracked = vec!["anomalyco/rune".into(), "cli/cli".into(), "rust-lang/rust-analyzer".into(), "tokio-rs/axum".into()];
                s.show(Page::Inbox);
            }),
            ("activity-repo", |s: &mut State| {
                // One repo's detail open, with its Actions topic expanded.
                s.show(Page::Inbox);
                let first = s.pulse.data.as_ref().and_then(|p| p.repos.iter().find(|r| r.runs_total > 0).or(p.repos.first())).map(|r| r.slug.clone());
                if let Some(slug) = first {
                    s.pulse_open_repos.insert(slug.clone());
                    s.pulse_topics.insert(format!("{slug}|ci"));
                }
            }),
            ("activity-month", |s: &mut State| {
                s.pulse_preset = pulse::Preset::Month;
                s.pulse_details = true;
                s.show(Page::Inbox);
            }),
            ("activity-30", |s: &mut State| {
                s.pulse_preset = pulse::Preset::Last30;
                s.pulse_details = true;
                s.show(Page::Inbox);
            }),
            ("activity-loading", |s: &mut State| {
                // The Activity page part-way through loading: the open-PR list and two repos'
                // runs are in, everything else is still coming.
                s.show(Page::Inbox);
                if let Some(done) = s.pulse.data.clone() {
                    let mut p = pulse::Pulse::starting(&done.repos.iter().map(|r| r.slug.clone()).collect::<Vec<_>>(), done.range.as_ref().unwrap());
                    p.apply(pulse::Part::Open(Ok((done.open_total, done.open_prs.clone()))));
                    for r in done.repos.iter().take(2) {
                        p.apply(pulse::Part::Runs(r.slug.clone(), Ok((r.runs_total, r.runs_ok_total, r.runs_failed_total, r.runs.clone()))));
                    }
                    s.pulse.data = Some(p);
                    s.pulse.loading = true;
                }
            }),
            ("needs", |s: &mut State| {
                s.inbox_tab = 1;
                s.show(Page::Inbox);
            }),
            ("limit", |s: &mut State| {
                s.prefs.pinned.clear();
                s.prefs.groups.clear();
                s.prefs.owner_order = vec!["cli".into(), "rust-lang".into(), "tokio-rs".into()];
                s.prefs.recent = vec!["rust-lang/cargo".into(), "cli/go-gh".into()];
                s.collapsed.clear();
                s.expanded_orgs.insert("tokio-rs".into());
            }),
            ("sorting", |s: &mut State| {
                s.prefs.pinned = vec!["anomalyco/rune".into(), "cli/cli".into(), "rust-lang/rustup".into()];
                s.prefs.groups = vec![config::Group {
                    name: "Rust tooling".into(),
                    repos: vec!["rust-lang/cargo".into(), "rust-lang/rustup".into(), "tokio-rs/tokio".into()],
                }];
                s.collapsed.clear();
            }),
            ("newgroup", |s: &mut State| {
                s.prefs.theme = config::Theme::Dark;
                s.open_group_dialog(None, Some("rust-lang/cargo".into()));
                s.group_name = TextEdit::new("Rust tooling");
            }),
            ("search", |s: &mut State| {
                s.repo_filter = TextEdit::new("rune");
                s.search_repos();
                s.select_repo("anomalyco/rune", Page::Pulls);
                s.pr_state = 1;
                s.pr_filter = TextEdit::new("text field");
                s.refresh_prs();
            }),
            ("commits", |s: &mut State| {
                s.select_repo("anomalyco/rune", Page::Commits);
                if let Some(sha) = s.rd().and_then(|d| d.graph.data.as_ref()).and_then(|g| g.commits.get(1)).map(|c| c.sha.clone()) {
                    s.select_commit(&sha);
                }
            }),
            ("branches", |s: &mut State| {
                s.branch_view = 1;
                s.select_repo("anomalyco/rune", Page::Branches);
            }),
            ("merge", |s: &mut State| {
                // Only the risk check runs (read-only); nothing is merged without the dialog.
                s.select_repo("anomalyco/rune", Page::Pulls);
                let pr = s.rd().and_then(|d| d.prs.data.as_ref()).and_then(|p| p.first()).map(|p| (p.number, p.title.clone()));
                if let Some((number, title)) = pr {
                    s.request(guard::Action::Merge { slug: "anomalyco/rune".into(), number, title, method: gh::MergeMethod::Squash, delete_branch: true });
                }
            }),
            ("palette", |s: &mut State| {
                s.select_repo("anomalyco/rune", Page::Pulls);
                s.open_palette();
                s.palette_q = TextEdit::new("rune");
            }),
            ("settings", |s: &mut State| s.open_settings(0)),
            ("setup", |s: &mut State| {
                // What a new machine sees: gh missing, so not signed in either.
                let git = Ok("git version 2.54.0".to_owned());
                let missing = |w: &str| Err(w.to_owned());
                s.setup_at = Instant::now() + Duration::from_secs(3600);
                s.setup = onboarding::Setup::Missing(onboarding::Preflight { git, gh: missing("`gh` isn't installed, or isn't on your PATH"), auth: missing("Install the GitHub CLI first") });
            }),
            ("setup-login", |s: &mut State| {
                let ok = |w: &str| Ok(w.to_owned());
                s.setup_at = Instant::now() + Duration::from_secs(3600);
                s.setup = onboarding::Setup::Missing(onboarding::Preflight { git: ok("git version 2.54.0"), gh: ok("gh version 2.98.0"), auth: Err("Not signed in to GitHub".into()) });
            }),
            ("splash", |s: &mut State| {
                // A first launch: nothing cached yet, the startup check still running.
                s.inbox = Load::default();
                s.setup = onboarding::Setup::Checking;
            }),
            ("welcome", |s: &mut State| {
                let ok = |w: &str| Ok(w.to_owned());
                s.setup = onboarding::Setup::Welcome(onboarding::Preflight { git: ok("git"), gh: ok("gh"), auth: ok("octocat on github.com") });
            }),
            ("settings-git", |s: &mut State| s.open_settings(2)),
            ("updates", |s: &mut State| {
                s.open_settings(8);
                s.check_for_updates(); // the real check, against the public releases
            }),
            ("update-available", |s: &mut State| {
                s.open_settings(8);
                s.update = update::Update::Available { tag: "v0.2.0".into(), version: "0.2.0".into(), url: String::new() };
            }),
            ("guide", |s: &mut State| s.guide_open = true),
            ("group", |s: &mut State| {
                s.prefs.groups = vec![config::Group { name: "Demo".into(), repos: vec!["anomalyco/rune".into(), "cli/cli".into()] }];
                s.open_group("Demo");
            }),
            ("triage", |s: &mut State| {
                s.select_repo("anomalyco/rune", Page::Actions);
                let failed = s.rd().and_then(|d| d.runs.data.as_ref()).and_then(|r| r.iter().find(|r| r.outcome() == model::Outcome::Failure)).map(|r| r.database_id);
                if let Some(id) = failed {
                    s.select_run(id);
                    s.load_triage(id);
                }
            }),
        ],
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("sigit", (1440.0, 900.0), init, view)
    };
    let headless = std::env::args().any(|a| a == "--snap" || a == "--tree" || a == "--script");
    // B6: OS notifications under the app's own name; a click opens what it's about. Registering
    // writes the name under HKCU\Software\Classes\AppUserModelId, so headless runs skip it.
    if !headless {
        let mut identity = rune_notify::Identity::new("dev.sigit.app", "sigit");
        // The notification icon is read from a file: keep a copy of the app icon in our folder.
        if let Some(dir) = config::dir() {
            let icon = dir.join("icon.png");
            let bytes: &[u8] = include_bytes!("../assets/icon-256.png");
            if std::fs::metadata(&icon).map(|m| m.len() as usize).ok() != Some(bytes.len()) {
                let _ = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&icon, bytes));
            }
            if icon.exists() {
                identity = identity.icon(icon);
            }
        }
        let _ = rune_notify::register(&identity);
    }
    rune_notify::on_click(|click| {
        let (slug, kind, id) = (click.get("slug").map(str::to_owned), click.get("kind").map(str::to_owned), click.get("id").map(str::to_owned));
        jobs::update(move |s| {
            let (Some(slug), Some(kind), Some(id)) = (slug, kind, id) else { return };
            match kind.as_str() {
                "run" => {
                    s.select_repo(&slug, Page::Actions);
                    if let Ok(id) = id.parse() {
                        s.select_run(id);
                    }
                }
                "pr" => {
                    s.select_repo(&slug, Page::Pulls);
                    if let Ok(n) = id.parse() {
                        s.select_pr(n);
                    }
                }
                _ => s.select_repo(&slug, Page::Actions),
            }
        });
    });
    // No native title bar: the app draws its own (`ui::titlebar`); Rune keeps the window's
    // minimize / maximize / close buttons over its trailing edge (traffic lights on macOS).
    let app = rune::App::new(spec).titlebar(rune::Titlebar::new(ui::TITLEBAR_HEIGHT));
    // Headless runs (`--snap`, `--tree`) load inline so the still has data in it.
    if !headless {
        jobs::install(app.handle());
    }
    app.run();
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn stale_rules() {
        let mut l: Load<u32> = Load::default();
        assert!(l.stale(60), "nothing loaded yet");
        l.loading = true;
        assert!(!l.stale(60), "already loading: don't ask twice");
        l.finish(Ok(1));
        assert!(!l.stale(60), "just loaded");
        l.at = Some(model::now() - 61);
        assert!(l.stale(60), "older than its ttl");
        l.finish(Err("offline".into()));
        assert_eq!(l.data, Some(1), "a failed update keeps what was shown");
    }

    #[test]
    fn stash_parks_and_drops_in_flight() {
        let mut memo: HashMap<u64, Load<u32>> = HashMap::new();
        let mut cur = Load { data: Some(7), loading: true, ..Load::default() };
        stash(&mut memo, &mut cur, Some(1));
        assert!(cur.data.is_none());
        assert!(!memo[&1].loading, "a parked load isn't waiting on an answer any more");
        let mut empty: Load<u32> = Load::start();
        stash(&mut memo, &mut empty, Some(2));
        assert!(!memo.contains_key(&2), "nothing to park");
    }

    #[test]
    fn snapshot_round_trips_pulse() {
        let mut p = pulse::Pulse::default();
        p.open_total = 4;
        p.repos.push(pulse::RepoPulse { slug: "a/b".into(), runs: vec![pulse::RunStat { outcome: Some(model::Outcome::Failure), ..Default::default() }], ..Default::default() });
        let snap = cache::Snapshot { pulse: Some(("k".into(), p)), pulse_at: 9, ..Default::default() };
        let back: cache::Snapshot = serde_json::from_str(&serde_json::to_string(&snap).unwrap()).unwrap();
        let (k, p) = back.pulse.unwrap();
        assert_eq!((k.as_str(), p.open_total, p.repos[0].runs[0].outcome), ("k", 4, Some(model::Outcome::Failure)));
    }
}
