//! sigit: a fast desktop view of your everyday git and GitHub work.
//!
//! - **Dashboard** (formerly Inbox): PRs waiting for your review, your open PRs, assigned issues, recent runs.
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

#[macro_use]
mod i18n;
mod activity;
mod cache;
mod crash_log;
mod focus;
mod onboarding;
mod settings;
mod update;
mod window_icon;
mod compare;
mod config;
mod daily;
mod gh;
mod git;
mod guard;
mod jobs;
mod lane;
mod lane_model;
mod modules;
mod model;
mod pulse;
mod risk;
mod route;
mod scan;
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

/// Which repos the Dashboard's tabs show (picked on its tab bar): one of your sidebar groups, or
/// the tracked repos in no group. `None` in State means all.
#[derive(Clone, Debug, PartialEq)]
pub enum RepoScope {
    Group(String),
    Other,
}

/// The lane whose detail pane is open: a Comparisons cell, or a branch picked on the Branches
/// page compared with its base. `name` is the comparison's name, or the branch's.
pub struct OpenLane<'a> {
    pub slug: &'a str,
    pub name: String,
    pub from: String,
    pub to: String,
    pub cell: &'a compare::Cell,
}

/// How the open comparison lane is read: grouped by contributor, and the folder picked as
/// a filter (path, whether it's a root).
#[derive(Default, Clone, Debug, PartialEq)]
pub struct LaneView {
    pub by_contributor: bool,
    pub folder: Option<(String, bool)>,
    /// The Modules section shows its full list (otherwise one bar and a short legend).
    pub modules_open: bool,
}

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

/// The Dashboard tab that holds the branch comparisons (0 Activity, 1 Needs you).
pub const COMPARE_TAB: usize = 2;

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
    /// Opening a page or coming back to the window re-checks what's older than this.
    pub const REVALIDATE: i64 = 10;
    /// Activity on those occasions: it takes a few seconds and many requests to gather.
    pub const PULSE_REVALIDATE: i64 = 60;
    /// Seconds away from the window before coming back re-checks the page.
    pub const AWAY: u64 = 5;
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
            Page::Inbox => "Dashboard",
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
            (Some(r), _) => tf!("{} · {r}", crate::i18n::t(self.page.label())),
            (_, Some(g)) => tf!("Group · {g}"),
            _ => crate::i18n::t(self.page.label()),
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

/// A repo's "Comparisons for …" dialog: one (From, To, On) line per shared comparison.
pub struct CompareEdit {
    pub slug: String,
    pub rows: Vec<(TextEdit, TextEdit, bool)>,
}

/// The open-PR dialog: the fields of a new PR, editable.
pub struct PrDraft {
    pub slug: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
    pub title: TextEdit,
    pub body: TextEdit,
    pub reviewers: TextEdit,
    pub assignees: TextEdit,
    pub labels: TextEdit,
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
    /// The command hover card showing now (the button the pointer rests on).
    pub cmd_tip: Option<ui::cmd::Tip>,
    /// When the window lost focus, on the app clock (None while it has it).
    pub away_since: Option<f64>,
    /// Seconds since the app started, as `poll` last heard (virtual in a headless run).
    pub clock: f64,
    /// Where each resizable row is (x, width in window points), for its drag handles.
    pub split_boxes: HashMap<&'static str, [f32; 2]>,
    pub palette: bool,
    pub palette_q: TextEdit,
    pub palette_sel: usize,
    pub guide_open: bool,
    pub guide_sec: usize,
    // Overlays
    /// Errors caught instead of closing the app, oldest first: the error dialog shows the first.
    pub crash: Vec<crash_log::Report>,
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
    /// The Comparisons tab: one row per tracked repo.
    pub compare: Load<Vec<compare::RepoCompare>>,
    /// What `compare` answers (repos, comparisons, overrides); a changed key means reload.
    pub compare_key: String,
    /// The cell whose detail pane is open: (repo, comparison index).
    pub compare_sel: Option<(String, usize)>,
    /// Branches compared with a base on the Branches page, by (repo, branch, base); this session only.
    pub branch_lanes: HashMap<(String, String, String), Load<compare::RepoCompare>>,
    /// The branch whose lane is open on the Branches page: (repo, branch without `origin/`).
    pub branch_lane_sel: Option<(String, String)>,
    /// The PRs of a branch, by (repo, head): whether one is already open. Session only.
    pub route_prs: HashMap<(String, String), Load<Vec<route::BranchPr>>>,
    /// What branch lanes compare with (None: the repo's default branch).
    pub branch_base: Option<String>,
    /// The "compared with" list on the branch pane is open.
    pub branch_base_open: bool,
    /// PR details of opened lanes, by (repo, sorted PR numbers); this session only.
    pub lane_details: HashMap<(String, Vec<u64>), Load<Vec<compare::PrDetail>>>,
    /// Files changed by commits that were clicked open, by (repo, sha); this session only.
    pub commit_files: HashMap<(String, String), Load<Vec<(String, u64, u64)>>>,
    /// Commits, by (repo, sha), whose files are shown.
    pub open_commits: HashSet<(String, String)>,
    /// Module maps of loaded commits in each lane's colours, by (repo, lane PR numbers, sha):
    /// worked out when a commit's files or the lane's model arrive, never per frame.
    pub commit_segments: HashMap<(String, Vec<u64>), HashMap<String, Vec<lane_model::Segment>>>,
    /// What the pane shows for each lane in `lane_details`, built once when its details or
    /// layout arrive; the view only reads it.
    pub lane_models: HashMap<(String, Vec<u64>), lane_model::LaneModel>,
    /// The From branch each lane in `lane_details` was opened with: whose layout its model uses.
    pub lane_branches: HashMap<(String, Vec<u64>), String>,
    /// Decodes downloaded images (set up once the window exists).
    pub images: Option<rune::ImageCache>,
    /// Contributors' profile pictures by URL; a URL in `avatars_asked` without an entry here is
    /// loading, or failed (the initial shows). Kept for the session.
    pub avatars: HashMap<String, rune::Image>,
    pub avatars_asked: std::collections::HashSet<String>,
    /// Module layouts learned from repo file trees, by (repo, branch). Kept for the session.
    pub layouts: HashMap<(String, String), Load<modules::Layout>>,
    /// How the open lane's changed code is shown.
    pub lane_view: LaneView,
    /// The Dashboard's group filter, shared by its tabs (None: all). Kept for the session.
    pub repo_scope: Option<RepoScope>,
    pub compare_edit: Option<CompareEdit>,
    pub pr_draft: Option<PrDraft>,
    /// The draft last handed to the guard: it comes back if the PR isn't opened.
    pub last_pr_draft: Option<PrDraft>,
    /// Settings → Git: the shared comparisons being edited (name, from, to), and for each the name
    /// it had when loaded (`None` for a new row), so a rename or a move isn't mistaken for a new one.
    pub cmp_rows: Vec<(TextEdit, TextEdit, TextEdit)>,
    pub cmp_ids: Vec<Option<String>>,
    pub tracked_q: TextEdit,
    /// The dialog's name field (kept here, not inside the Option, so it always exists).
    pub group_name: TextEdit,
}

impl State {
    /// The starting state for a given set of prefs: no disk, no jobs.
    pub fn new(prefs: config::Prefs) -> State {
        State {
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
            cmd_tip: None,
            away_since: None,
            clock: 0.0,
            split_boxes: HashMap::new(),
            palette: false,
            palette_q: TextEdit::new(""),
            palette_sel: 0,
            guide_open: false,
            guide_sec: 0,
            crash: Vec::new(),
            toasts: Vec::new(),
            next_toast: 0,
            busy: Vec::new(),
            prefs,
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
            pulse_from: TextEdit::new(""),
            pulse_to: TextEdit::new(""),
            tracked_open: false,
            back: Vec::new(),
            fwd: Vec::new(),
            restoring: false,
            pulse_open_repos: HashSet::new(),
            pulse_topics: HashSet::new(),
            pulse_details: false,
            compare: Load::default(),
            compare_key: String::new(),
            compare_sel: None,
            branch_lanes: HashMap::new(),
            branch_lane_sel: None,
            route_prs: HashMap::new(),
            branch_base: None,
            branch_base_open: false,
            lane_details: HashMap::new(),
            commit_files: HashMap::new(),
            open_commits: HashSet::new(),
            commit_segments: HashMap::new(),
            lane_models: HashMap::new(),
            lane_branches: HashMap::new(),
            images: None,
            avatars: HashMap::new(),
            avatars_asked: Default::default(),
            layouts: HashMap::new(),
            lane_view: LaneView::default(),
            repo_scope: None,
            compare_edit: None,
            pr_draft: None,
            last_pr_draft: None,
            cmp_rows: Vec::new(),
            cmp_ids: Vec::new(),
            tracked_q: TextEdit::new(""),
            group_name: TextEdit::new(""),
        }
    }
}

fn init() -> State {
    config::migrate_old_folder(); // before anything reads the folder
    update::clean_up_previous(); // the copy the last update set aside
    let mut s = State::new(config::load_prefs());
    s.collapsed = s.prefs.collapsed.iter().cloned().collect();
    // Messages built before the first frame (toasts, risk checks) are in the right language too.
    i18n::apply(s.prefs.language);
    s.settings_prod = TextEdit::new(s.prefs.prod_names.join(", "));
    s.settings_sensitive = TextEdit::new(s.prefs.sensitive.join("\n"));
    // B8: last session's data first, so the first frame is full; fresh data replaces it.
    s.load_cache();
    for saved in config::load() {
        if saved.path.exists() {
            s.add_repo(saved.slug, Some(saved.path));
        }
    }
    if !jobs::is_inline_run() {
        s.rescan_folders(false);
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
            // A base picked in one repo means nothing in another.
            self.branch_base = None;
            self.close_branch_lane();
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
        self.revalidate_page();
    }

    /// Switch page: what's cached shows at once, and is checked again (`revalidate_page`).
    pub fn show(&mut self, page: Page) {
        self.remember(&Place::new(page, self.cur.clone(), self.group_cur.clone()));
        if self.page != page {
            self.branch_base_open = false;
        }
        self.page = page;
        self.revalidate_page();
    }

    /// The window lost or got focus: coming back after a while checks the page again.
    pub fn window_focus(&mut self, focused: bool) {
        if !focused {
            self.away_since = Some(self.clock);
            return;
        }
        if self.away_since.take().is_some_and(|t| self.clock - t >= ttl::AWAY as f64) {
            self.revalidate_page();
        }
    }

    /// The 30 s tick: reload what the page shows where it's missing or older than its `ttl`.
    /// Fresh data is shown as is, with no request.
    pub fn ensure_page(&mut self) {
        self.load_stale(i64::MAX);
    }

    /// Opening a page (a tab, a repo, back / forward) or coming back to the window: what's
    /// cached shows at once and is checked again in the background, so the page is current
    /// without pressing Refresh. Anything fetched in the last few seconds isn't asked again
    /// (flicking between tabs doesn't refetch), and Activity, which is slow to gather, at most
    /// once a minute.
    pub fn revalidate_page(&mut self) {
        self.load_stale(ttl::REVALIDATE);
    }

    /// Load what the page shows where it's missing or older than its `ttl`, or than `cap`
    /// seconds when that's shorter.
    fn load_stale(&mut self, cap: i64) {
        let ts = |d: Option<&RepoData>, f: fn(&RepoData, i64) -> bool, ttl: i64| d.is_none_or(|d| f(d, ttl));
        match self.page {
            Page::Inbox => {
                if self.inbox.stale(ttl::GH.min(cap)) {
                    self.refresh_inbox();
                }
                if self.notifs.stale(ttl::GH.min(cap)) {
                    self.refresh_notifications();
                }
                if self.pulse.stale(ttl::PULSE.min(cap.max(ttl::PULSE_REVALIDATE))) || (!self.pulse.loading && self.pulse_key != self.pulse_key_now()) {
                    self.refresh_pulse();
                }
                if self.inbox_tab == COMPARE_TAB && (self.compare.stale(ttl::GH.min(cap)) || (!self.compare.loading && self.compare_key != self.compare_key_now())) {
                    self.refresh_compare();
                }
                // A refresh that landed while another tab showed may have changed the open
                // lane's PRs: load what it lacks.
                self.load_new_lane_details();
                return;
            }
            Page::Group => return self.load_group(true),
            _ => {}
        }
        let d = self.rd();
        let (meta, status) = (ts(d, |d, t| d.meta.stale(t), ttl::META), ts(d, |d, t| d.status.stale(t), ttl::LOCAL.min(cap)));
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
                if other_state || ts(d, |d, t| d.prs.stale(t), ttl::GH.min(cap)) {
                    self.load_prs();
                }
                if self.pr_sel.is_some() && self.pr_detail.stale(ttl::DETAIL.min(cap)) {
                    self.load_pr_detail();
                }
            }
            Page::Actions => {
                let runs_ttl = if self.runs_running() { ttl::RUNNING.min(cap) } else { ttl::RUNS.min(cap) };
                if ts(d, |d, t| d.runs.stale(t), runs_ttl) {
                    self.load_runs();
                }
                let sel_running = self.run_sel.is_some_and(|id| self.run_running(id));
                if self.run_sel.is_some() && self.run_detail.stale(if sel_running { ttl::RUNNING.min(cap) } else { ttl::DETAIL.min(cap) }) {
                    self.load_run_detail();
                }
            }
            Page::Envs => {
                if ts(d, |d, t| d.envs.stale(t), ttl::GH.min(cap)) {
                    self.refresh_envs();
                }
            }
            Page::Commits => {
                let other = d.is_some_and(|d| !d.graph.loading && d.graph.data.as_ref().is_some_and(|g| g.all != self.all_branches));
                if other || ts(d, |d, t| d.graph.stale(t), ttl::LOCAL.min(cap)) {
                    self.refresh_graph();
                }
                // A commit never changes: its detail is loaded once.
            }
            Page::Branches => {
                let (branches, tools) = (ts(d, |d, t| d.branches.stale(t), ttl::LOCAL.min(cap)), ts(d, |d, t| d.stashes.stale(t) || d.cleanup.stale(t), ttl::LOCAL.min(cap)));
                if branches {
                    self.refresh_branches();
                }
                if tools {
                    self.refresh_branch_tools();
                }
                if self.branch_view <= 1 && self.branch_lane_sel.as_ref().is_some_and(|(slug, _)| self.cur.as_ref() == Some(slug)) {
                    self.load_branch_lane();
                    self.load_new_lane_details();
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
                if self.inbox_tab == COMPARE_TAB {
                    self.refresh_compare();
                }
            }
            Page::Pulls => self.refresh_prs(),
            Page::Actions => self.refresh_runs(),
            Page::Envs => self.refresh_envs(),
            Page::Commits => self.refresh_graph(),
            Page::Branches => {
                self.refresh_branches();
                self.refresh_branch_tools();
                // The open branch's comparison too (it may be on GitHub now).
                if self.forget_branch_lane() {
                    self.load_branch_lane();
                    self.load_lane_details(false);
                }
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
                s.toast(crate::i18n::t(tf!("gh: {e}")), true);
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

    /// What the Comparisons tab asks for now: tracked repos, comparisons and overrides.
    pub fn compare_key_now(&self) -> String {
        serde_json::to_string(&(self.tracked(), &self.prefs.comparisons, &self.prefs.comparison_overrides)).unwrap_or_default()
    }

    /// The open lane's repo and sorted PR numbers: what `lane_details` is keyed by.
    pub fn lane_key(&self) -> Option<(String, Vec<u64>)> {
        let lane = self.open_lane()?;
        let compare::Cell::Waiting { prs, .. } = lane.cell else { return None };
        let mut nums: Vec<u64> = prs.iter().map(|p| p.number).collect();
        nums.sort_unstable();
        Some((lane.slug.to_owned(), nums))
    }

    /// The open lane's repo and From branch (as this repo compares it), whose tree its modules
    /// are learned from.
    pub fn lane_branch(&self) -> Option<(String, String)> {
        let lane = self.open_lane()?;
        Some((lane.slug.to_owned(), lane.from))
    }

    /// The lane the page shows: the picked Comparisons cell on the Dashboard's Comparisons tab,
    /// the picked branch against its base on Branches (local or remote). None while it loads.
    pub fn open_lane(&self) -> Option<OpenLane<'_>> {
        match self.page {
            Page::Inbox if self.inbox_tab == COMPARE_TAB => {
                let (slug, j) = self.compare_sel.as_ref()?;
                let rc = self.compare.data.as_ref()?.iter().find(|r| &r.slug == slug)?;
                let cell = rc.cells.get(*j)?;
                let c = compare::effective(&self.prefs.comparisons, &self.prefs.comparison_overrides, slug).get(*j).cloned().flatten()?;
                Some(OpenLane { slug: &rc.slug, name: c.name, from: c.from, to: c.to, cell })
            }
            Page::Branches if self.branch_view <= 1 => {
                let (slug, branch) = self.branch_lane_sel.as_ref()?;
                if self.cur.as_ref() != Some(slug) {
                    return None;
                }
                let base = self.branch_base_for(slug)?;
                let key = (slug.clone(), branch.clone(), base);
                let rc = self.branch_lanes.get(&key)?.data.as_ref()?;
                let cell = rc.cells.first()?;
                let (_, branch, base) = key;
                Some(OpenLane { slug, name: branch.clone(), from: branch, to: base, cell })
            }
            _ => None,
        }
    }

    /// What branch lanes in `slug` compare with: the picked base, or the repo's default branch.
    pub fn branch_base_for(&self, slug: &str) -> Option<String> {
        if let Some(b) = &self.branch_base {
            return Some(b.clone());
        }
        self.data.get(slug)?.meta.data.as_ref().map(|m| m.default_branch.clone()).filter(|b| !b.is_empty())
    }

    /// Open `branch`'s lane on the Branches page (a remote `origin/x` is compared as `x`).
    pub fn select_branch_lane(&mut self, branch: &str) {
        let Some(slug) = self.cur.clone() else { return };
        let branch = branch.strip_prefix("origin/").unwrap_or(branch).to_owned();
        self.branch_lane_sel = Some((slug.clone(), branch.clone()));
        self.lane_view.folder = None;
        self.load_branch_lane();
        self.load_lane_details(false);
    }

    /// Close the Branches page's lane pane (and the branch picked in the table).
    pub fn close_branch_lane(&mut self) {
        self.branch_lane_sel = None;
        self.branch_base_open = false;
        self.branch_sel = None;
        self.branches_table.clear_selection();
    }

    /// Nothing to compare: the base itself, or the remote's HEAD pointer.
    pub fn branch_lane_skipped(branch: &str, base: &str) -> bool {
        branch == base || branch == "HEAD"
    }

    /// Compare branch lanes with `base` (None: the default branch) from now on.
    pub fn set_branch_base(&mut self, base: Option<String>) {
        self.branch_base = base;
        self.branch_base_open = false;
        self.load_branch_lane();
        self.load_lane_details(false);
    }

    /// Drop the open branch lane's comparison so the next load asks again; false when no branch
    /// lane is open (or its base isn't known yet).
    fn forget_branch_lane(&mut self) -> bool {
        let Some((slug, branch)) = self.branch_lane_sel.clone() else { return false };
        let Some(base) = self.branch_base_for(&slug) else { return false };
        self.branch_lanes.remove(&(slug, branch, base));
        true
    }

    /// Fetch the open branch lane against its base, unless it's loaded or on the way.
    fn load_branch_lane(&mut self) {
        let Some((slug, branch)) = self.branch_lane_sel.clone() else { return };
        let Some(base) = self.branch_base_for(&slug) else { return };
        if Self::branch_lane_skipped(&branch, &base) {
            return;
        }
        let key = (slug, branch, base);
        if self.branch_lanes.contains_key(&key) {
            return;
        }
        self.branch_lanes.insert(key.clone(), Load::start());
        let (slug, from, to) = key.clone();
        jobs::spawn(self, move || compare::fetch_branch(&slug, &from, &to), move |s, rc| {
            if let Some(l) = s.branch_lanes.get_mut(&key) {
                l.finish(Ok(rc));
            }
            s.load_lane_details(false);
        });
    }

    /// Learn the open lane's module layout, once per repo and branch per session (again with
    /// `retry`). A failure just leaves the folder diagram.
    pub fn load_layout(&mut self, retry: bool) {
        let Some(key) = self.lane_branch() else { return };
        if !retry && self.layouts.get(&key).is_some_and(|l| l.data.is_some() || l.loading) {
            return;
        }
        let entry = self.layouts.entry(key.clone()).or_default();
        entry.loading = true;
        entry.error = None;
        let (slug, branch) = key.clone();
        jobs::spawn(self, move || modules::fetch_tree(&slug, &branch).map(|files| modules::learn(&files)), move |s, out| s.layout_arrived(&key, out));
    }

    /// A repo branch's layout arrived: store it and rebuild the models of lanes that use it.
    fn layout_arrived(&mut self, key: &(String, String), out: Result<modules::Layout, String>) {
        if let Some(l) = self.layouts.get_mut(key) {
            l.finish(out);
        }
        let lanes: Vec<(String, Vec<u64>)> = self.lane_branches.iter().filter(|(k, b)| k.0 == key.0 && **b == key.1).map(|(k, _)| k.clone()).collect();
        for k in lanes {
            self.rebuild_lane_model(&k);
        }
    }

    /// A lane's PR details arrived: store them, rebuild its model, fetch its authors' pictures.
    fn details_arrived(&mut self, key: &(String, Vec<u64>), out: Result<Vec<compare::PrDetail>, String>) {
        if let Some(l) = self.lane_details.get_mut(key) {
            l.finish(out);
        }
        self.rebuild_lane_model(key);
        self.load_avatars(key);
    }

    /// Work out what the pane shows for lane `key` from its details and its branch's layout
    /// (none while its details aren't loaded).
    pub fn rebuild_lane_model(&mut self, key: &(String, Vec<u64>)) {
        let Some(details) = self.lane_details.get(key).and_then(|l| l.data.as_deref()) else {
            self.lane_models.remove(key);
            return;
        };
        let layout = self.lane_branches.get(key).and_then(|b| self.layouts.get(&(key.0.clone(), b.clone()))).and_then(|l| l.data.as_ref());
        let model = lane_model::build(details, layout);
        // Loaded commits of this repo get their maps in this lane's colours.
        let commits: Vec<(String, Vec<lane_model::Segment>)> = self
            .commit_files
            .iter()
            .filter(|((slug, _), _)| *slug == key.0)
            .filter_map(|((_, sha), l)| l.data.as_ref().map(|files| (sha.clone(), lane_model::segments(&model, layout, files))))
            .collect();
        if !commits.is_empty() {
            self.commit_segments.entry(key.clone()).or_default().extend(commits);
        }
        self.lane_models.insert(key.clone(), model);
    }

    /// A commit's files arrived: store them and work out its map for each of its repo's lanes
    /// (and the open lane, even before its model is in: then every file is "other").
    pub fn commit_files_arrived(&mut self, key: &(String, String), out: Result<Vec<(String, u64, u64)>, String>) {
        let Some(l) = self.commit_files.get_mut(key) else { return };
        l.finish(out);
        let Some(files) = l.data.clone() else { return };
        let mut lanes: Vec<(String, Vec<u64>)> = self.lane_models.keys().filter(|k| k.0 == key.0).cloned().collect();
        if let Some(open) = self.lane_key().filter(|k| k.0 == key.0 && !lanes.contains(k)) {
            lanes.push(open);
        }
        let empty = lane_model::LaneModel::default();
        for lane in lanes {
            let layout = self.lane_branches.get(&lane).and_then(|b| self.layouts.get(&(lane.0.clone(), b.clone()))).and_then(|l| l.data.as_ref());
            let segs = lane_model::segments(self.lane_models.get(&lane).unwrap_or(&empty), layout, &files);
            self.commit_segments.entry(lane).or_default().insert(key.1.clone(), segs);
        }
    }

    /// Open the lane (repo, comparison `j`), or close it when it's already open.
    pub fn select_lane(&mut self, slug: &str, j: usize) {
        let pick = (slug.to_owned(), j);
        self.compare_sel = if self.compare_sel.as_ref() == Some(&pick) { None } else { Some(pick) };
        self.lane_view.folder = None;
        self.load_lane_details(false);
    }

    /// Fetch the open lane's PR details unless they're loaded or on the way (or `retry`).
    pub fn load_lane_details(&mut self, retry: bool) {
        let Some(key) = self.lane_key() else { return };
        if key.1.is_empty() {
            return; // only loose commits: nothing to fetch
        }
        // The same PRs reached from another branch use that branch's layout from now on.
        if let Some((_, branch)) = self.lane_branch()
            && self.lane_branches.get(&key) != Some(&branch)
        {
            self.lane_branches.insert(key.clone(), branch);
            self.rebuild_lane_model(&key);
        }
        self.load_layout(retry);
        if !retry && self.lane_details.get(&key).is_some_and(|l| l.data.is_some() || l.loading) {
            return;
        }
        let entry = self.lane_details.entry(key.clone()).or_default();
        entry.loading = true;
        entry.error = None;
        let (slug, nums) = key.clone();
        jobs::spawn(self, move || compare::fetch_details(&slug, nums), move |s, out| s.details_arrived(&key, out));
    }

    /// A commit's files are worth fetching: never asked for, or the last try failed and
    /// nothing is in flight.
    pub fn wants_commit_fetch(entry: Option<&Load<Vec<(String, u64, u64)>>>) -> bool {
        entry.is_none_or(|l| !l.loading && l.data.is_none() && l.error.is_some())
    }

    /// Open or close a commit's file list; opening fetches its files once per session
    /// (again after an error).
    pub fn toggle_commit(&mut self, slug: &str, sha: &str) {
        let key = (slug.to_owned(), sha.to_owned());
        if !self.open_commits.remove(&key) {
            self.open_commits.insert(key.clone());
            if Self::wants_commit_fetch(self.commit_files.get(&key)) {
                self.commit_files.insert(key.clone(), Load::start());
                let (slug, sha) = key.clone();
                jobs::spawn(self, move || compare::fetch_commit_files(&slug, &sha), move |s, out| s.commit_files_arrived(&key, out));
            }
        }
    }

    /// Load the open lane's details when its PR set has none yet (a refresh that landed while
    /// the lane wasn't shown). One that failed waits for Retry rather than retrying each tick.
    fn load_new_lane_details(&mut self) {
        if self.lane_needs_details() {
            self.load_lane_details(false);
        }
    }

    /// The open lane has PRs whose details were never asked for.
    fn lane_needs_details(&self) -> bool {
        self.lane_key().is_some_and(|k| !k.1.is_empty() && !self.lane_details.contains_key(&k))
    }

    /// Fetch the profile pictures of a lane's authors that aren't loaded or asked for yet, each
    /// once per session, in the background.
    pub fn load_avatars(&mut self, key: &(String, Vec<u64>)) {
        if self.images.is_none() {
            return; // headless runs and tests: initials only
        }
        let urls: Vec<String> = self
            .lane_details
            .get(key)
            .and_then(|l| l.data.as_ref())
            .map(|d| d.iter().map(|p| p.author.avatar_url.clone()).filter(|u| u.starts_with(gh::AVATAR_HOST)).collect())
            .unwrap_or_default();
        for url in urls {
            if !self.avatars_asked.insert(url.clone()) {
                continue;
            }
            let fetch = url.clone();
            jobs::spawn(self, move || gh::avatar_bytes(&fetch), move |s, out| {
                if let (Ok(bytes), Some(cache)) = (out, s.images.as_ref()) {
                    let image = cache.load(bytes);
                    s.avatars.insert(url, image);
                }
            });
        }
    }

    /// Load the Comparisons tab: each tracked repo (A-Z) against the comparisons that apply to it.
    pub fn refresh_compare(&mut self) {
        let mut slugs = self.tracked();
        slugs.sort_by_key(|s| s.to_lowercase());
        if slugs.is_empty() || self.prefs.comparisons.is_empty() {
            self.compare = Load::default();
            self.compare_key.clear();
            self.compare_sel = None;
            self.compare_edit = None;
            return;
        }
        let rows: Vec<(String, Vec<Option<compare::Comparison>>)> = slugs
            .into_iter()
            .map(|slug| {
                let cells = compare::effective(&self.prefs.comparisons, &self.prefs.comparison_overrides, &slug);
                (slug, cells)
            })
            .collect();
        let key = self.compare_key_now();
        if self.compare_key != key {
            // Rows for other repos or comparisons must never sit under the new key, and the
            // detail pane and dialog point at comparisons by position.
            self.compare = Load::default();
            self.compare_sel = None;
            self.compare_edit = None;
        }
        self.compare_key = key.clone();
        self.compare.loading = true;
        self.compare.error = None;
        jobs::spawn(self, move || compare::fetch(rows), move |s, out| {
            if s.compare_key != key {
                return;
            }
            s.compare.finish(Ok(out));
            // The open lane's PR set may have changed with the fresh data, and with it its key.
            s.load_lane_details(false);
            s.save_cache();
        });
    }

    /// Open the comparison dialog for `slug`, each line showing what applies to it now.
    pub fn open_compare_edit(&mut self, slug: &str) {
        let shared = &self.prefs.comparisons;
        let rows = compare::effective(shared, &self.prefs.comparison_overrides, slug)
            .into_iter()
            .zip(shared)
            .map(|(e, c)| {
                let on = e.is_some();
                let e = e.unwrap_or_else(|| c.clone());
                (TextEdit::new(&e.from), TextEdit::new(&e.to), on)
            })
            .collect();
        self.compare_edit = Some(CompareEdit { slug: slug.to_owned(), rows });
    }

    /// Keep the dialog's lines that differ from the shared comparisons as this repo's overrides.
    pub fn save_compare_edit(&mut self) {
        let Some(edit) = self.compare_edit.take() else { return };
        let rows: Vec<(String, String, bool)> = edit.rows.iter().map(|(f, t, on)| (f.text.clone(), t.text.clone(), *on)).collect();
        let mine = compare::edit_overrides(&self.prefs.comparisons, &edit.slug, &rows);
        self.prefs.comparison_overrides.retain(|o| o.slug != edit.slug);
        self.prefs.comparison_overrides.extend(mine);
        self.save_prefs();
        self.refresh_compare();
    }

    /// Drop this repo's overrides: it follows the shared comparisons again.
    pub fn reset_compare_edit(&mut self) {
        let Some(edit) = self.compare_edit.take() else { return };
        self.prefs.comparison_overrides.retain(|o| o.slug != edit.slug);
        self.save_prefs();
        self.refresh_compare();
    }

    /// The scope that applies now: the picked one, unless its group no longer exists.
    pub fn scope_now(&self) -> Option<&RepoScope> {
        self.repo_scope.as_ref().filter(|sc| match sc {
            RepoScope::Group(g) => self.prefs.groups.iter().any(|x| x.name == *g),
            RepoScope::Other => true,
        })
    }

    /// Whether `slug` is in the Dashboard's picked scope (any repo, with no scope).
    pub fn in_scope(&self, slug: &str) -> bool {
        match self.scope_now() {
            None => true,
            Some(RepoScope::Group(g)) => self.prefs.groups.iter().any(|x| x.name == *g && x.repos.iter().any(|r| r == slug)),
            Some(RepoScope::Other) => !self.prefs.groups.iter().any(|x| x.repos.iter().any(|r| r == slug)),
        }
    }

    /// Pick the Dashboard's scope: closes an open lane pane and reloads Activity for it.
    pub fn set_repo_scope(&mut self, scope: Option<RepoScope>) {
        if scope == self.repo_scope {
            return;
        }
        self.repo_scope = scope;
        self.compare_sel = None;
        self.queue_sel = 0;
        if self.pulse_key != self.pulse_key_now() {
            self.refresh_pulse();
        }
    }

    /// The repos the Activity page shows now (all tracked, or one of your groups).
    pub fn pulse_repos(&self) -> Vec<String> {
        match self.scope_now() {
            Some(RepoScope::Group(g)) => self.prefs.groups.iter().find(|x| x.name == *g).map(|x| x.repos.clone()).unwrap_or_default(),
            _ => self.tracked().into_iter().filter(|r| self.in_scope(r)).collect(),
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
        self.toast_undo(crate::i18n::t(tf!("{} {slug}", if now_tracked { "Tracking" } else { "Stopped tracking" })), Some(guard::Undo::Prefs(Box::new(before))));
        self.refresh_pulse();
    }

    pub fn reset_tracked(&mut self) {
        let before = self.prefs.clone();
        self.prefs.tracked.clear();
        self.save_prefs();
        self.toast_undo(crate::i18n::t("Tracking your pinned repos and your groups again"), Some(guard::Undo::Prefs(Box::new(before))));
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
        format!("{:?}|{:?}|{}|{end}|{}", self.scope_now(), self.pulse_preset, r.since, self.pulse_repos().join(","))
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
        jobs::spawn(self, move || gh::repo_meta(&key), move |s, r| {
            s.rd_mut(&slug).meta.finish(r);
            // A branch picked before the default branch was known compares with it now.
            if s.page == Page::Branches {
                s.load_branch_lane();
                s.load_lane_details(false);
            }
        });
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

    /// Fetch the PRs of `head` in `slug` (the old list stays up meanwhile).
    pub fn reload_route_prs(&mut self, slug: &str, head: &str) {
        let key = (slug.to_owned(), head.to_owned());
        let entry = self.route_prs.entry(key.clone()).or_default();
        entry.loading = true;
        entry.error = None;
        let (work_slug, work_head) = key.clone();
        // Tests never shell out: the lookup is offline there.
        let fetch = move || if cfg!(test) { Err("offline".to_owned()) } else { gh::branch_prs(&work_slug, &work_head) };
        jobs::spawn(self, fetch, move |s, result| {
            if let Some(l) = s.route_prs.get_mut(&key) {
                l.finish(result);
            }
        });
    }

    /// Is `head` on GitHub: the repo's branch list has `origin/{head}`.
    pub fn is_pushed(&self, slug: &str, head: &str) -> bool {
        let remote = format!("origin/{head}");
        self.data.get(slug).and_then(|d| d.branches.data.as_ref()).is_some_and(|l| l.iter().any(|b| b.remote && b.name == remote))
    }

    /// Whether `head` is on GitHub: Some(true/false) once the branch list is loaded, None before.
    pub fn pushed_state(&self, slug: &str, head: &str) -> Option<bool> {
        self.data.get(slug).and_then(|d| d.branches.data.as_ref()).map(|_| self.is_pushed(slug, head))
    }

    /// Open the dialog that creates a PR of `head` into `base`; the title starts as the
    /// branch's last commit subject.
    pub fn open_pr_dialog(&mut self, slug: &str, head: &str, base: &str) {
        let branches = self.data.get(slug).and_then(|d| d.branches.data.as_ref());
        let remote = format!("origin/{head}");
        let subject = branches
            .and_then(|l| l.iter().find(|b| !b.remote && b.name == head).or_else(|| l.iter().find(|b| b.remote && b.name == remote)))
            .map(|b| b.subject.clone())
            .unwrap_or_default();
        self.pr_draft = Some(PrDraft {
            slug: slug.to_owned(),
            head: head.to_owned(),
            base: base.to_owned(),
            draft: false,
            title: TextEdit::new(&subject),
            body: TextEdit::new(""),
            reviewers: TextEdit::new(""),
            assignees: TextEdit::new(""),
            labels: TextEdit::new(""),
        });
    }

    /// Hand the dialog's PR to the guard's confirm flow and close the dialog. The draft is kept
    /// in `last_pr_draft` until the PR is opened, so a cancel or a failure gives it back.
    pub fn submit_pr_draft(&mut self) {
        let Some(d) = self.pr_draft.as_ref() else { return };
        if d.title.text.trim().is_empty() {
            return;
        }
        // Only a head known to be missing from GitHub stops here; before the branch list
        // loads the guard's confirm still shows the command.
        if self.pushed_state(&d.slug, &d.head) == Some(false) {
            return;
        }
        let Some(d) = self.pr_draft.take() else { return };
        let spec = route::PrSpec {
            slug: d.slug.clone(),
            head: d.head.clone(),
            base: d.base.clone(),
            draft: d.draft,
            title: d.title.text.clone(),
            body: d.body.text.clone(),
            reviewers: d.reviewers.text.clone(),
            assignees: d.assignees.text.clone(),
            labels: d.labels.text.clone(),
        };
        self.last_pr_draft = Some(d);
        self.request(guard::Action::CreatePr { spec });
    }

    /// Reopen the dialog with the draft that was submitted, unless another one is open.
    pub fn restore_pr_draft(&mut self) {
        if let Some(d) = self.last_pr_draft.take()
            && self.pr_draft.is_none()
        {
            self.pr_draft = Some(d);
        }
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
                Err(e) => s.toast(crate::i18n::t(tf!("Repo search failed: {e}")), true),
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
                    Err(e) => s.toast(crate::i18n::t(tf!("PR search failed: {e}")), true),
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
        jobs::spawn(self, move || gh::recording(work), move |s, (r, commands)| {
            s.busy.retain(|b| *b != label);
            let ran = s.ran_lines(&commands);
            match r {
                Ok(out) => {
                    let line = out.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_owned);
                    s.toast(crate::i18n::t(line.unwrap_or_else(|| tf!("{label}: done"))) + &ran, false);
                    after(s);
                }
                Err(e) => s.toast(crate::i18n::t(tf!("{label} failed: {e}")) + &ran, true),
            }
        });
    }

    /// "\n$ gh pr merge 12 --squash …" for a toast, when Settings shows commands; else empty.
    pub fn ran_lines(&self, commands: &[String]) -> String {
        if !self.prefs.show_commands || commands.is_empty() {
            return String::new();
        }
        commands.iter().map(|c| format!("\n$ {c}")).collect()
    }

    /// A button's tooltip: `help` and the commands it runs (see `ui::cmd::CmdTip`).
    pub fn with_commands(&self, help: impl Into<String>, commands: &[String]) -> ui::cmd::Tip {
        ui::cmd::Tip::new(help, commands.to_vec())
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
            self.toast(crate::i18n::t(tf!("Couldn't save preferences: {e}")), true);
        }
    }

    pub fn set_language(&mut self, l: config::Language) {
        self.prefs.language = l;
        self.save_prefs();
    }

    pub fn set_theme(&mut self, t: config::Theme) {
        self.prefs.theme = t;
        self.save_prefs();
    }

    pub fn toggle_pin(&mut self, slug: &str) {
        let before = self.prefs.clone();
        let pinned = !self.prefs.pinned.iter().any(|p| p == slug);
        self.toast_undo(crate::i18n::t(tf!("{} {slug}", if pinned { "Pinned" } else { "Unpinned" })), Some(guard::Undo::Prefs(Box::new(before))));
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
        self.toast_undo(crate::i18n::t(tf!("Updated group {group}")), Some(guard::Undo::Prefs(Box::new(before))));
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
        self.toast_undo(crate::i18n::t(tf!("Moved {owner}")), Some(guard::Undo::Prefs(Box::new(before))));
    }

    pub fn reset_owner_order(&mut self) {
        let before = self.prefs.clone();
        self.prefs.owner_order.clear();
        self.save_prefs();
        self.toast_undo(crate::i18n::t("Organizations back in the default order (you first, then A–Z)"), Some(guard::Undo::Prefs(Box::new(before))));
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
        self.toast_undo(crate::i18n::t(if reset { tf!("{owner}: default look") } else { tf!("{owner}: saved") }), Some(guard::Undo::Prefs(Box::new(before))));
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
        self.toast_undo(crate::i18n::t(tf!("Deleted group {name}")), Some(guard::Undo::Prefs(Box::new(before))));
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
            self.toast(crate::i18n::t(if taken { tf!("There's already a group called {name}") } else { "A group needs a name".into() }), true);
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
        self.toast(crate::i18n::t("Safety settings saved"), false);
    }

    /// Remembers every repo that has a local clone, so the next launch finds them.
    fn save_repo_list(&mut self) {
        let saved: Vec<config::Saved> =
            self.repos.iter().filter_map(|r| Some(config::Saved { slug: r.slug.clone(), path: r.path.clone()? })).collect();
        if let Err(e) = config::save(&saved) {
            self.toast(crate::i18n::t(tf!("Couldn't save the repo list: {e}")), true);
        }
    }

    /// Adds the clones a scan found. A repo that already has a path keeps it. Returns
    /// (new, already known) and saves the repo list once when anything is new.
    pub fn add_clones(&mut self, scans: Vec<scan::Scan>) -> (usize, usize) {
        let (mut new, mut known) = (0, 0);
        for f in scans.into_iter().flat_map(|sc| sc.found) {
            if self.repos.iter().any(|r| r.slug == f.slug && r.path.is_some()) {
                known += 1;
            } else {
                self.add_repo(f.slug, Some(f.path));
                new += 1;
            }
        }
        if new > 0 && !jobs::is_inline_run() {
            self.save_repo_list();
        }
        (new, known)
    }

    /// Scans every clone folder in the background and adds what it finds. `announce` is a
    /// scan the user asked for (always answered); a launch scan only speaks when it adds clones.
    pub fn rescan_folders(&mut self, announce: bool) {
        let folders = self.prefs.clone_folders.clone();
        if folders.is_empty() {
            return;
        }
        jobs::spawn(
            self,
            move || folders.into_iter().map(|f| { let r = scan::scan(&f); (f, r) }).collect::<Vec<_>>(),
            move |s, done| {
                let stopped: Vec<String> = done.iter().filter(|(_, r)| r.stopped_early).map(|(f, _)| f.display().to_string()).collect();
                let (total, skipped) = (done.iter().map(|(_, r)| r.found.len()).sum::<usize>(), done.iter().map(|(_, r)| r.skipped).sum::<usize>());
                let (new, known) = s.add_clones(done.into_iter().map(|(_, r)| r).collect());
                if announce {
                    let mut msg = crate::i18n::t(tf!("Found {total} clones · {new} new · {known} already known · {skipped} skipped"));
                    if !stopped.is_empty() {
                        let folder = stopped.join(", ");
                        msg.push_str(&crate::i18n::t(tf!(" · stopped early in {folder}")));
                    }
                    s.toast(msg, false);
                } else if new > 0 {
                    s.toast(crate::i18n::t(tf!("Added {new} new clones from your folders")), false);
                }
                if new > 0 {
                    s.revalidate_page();
                }
            },
        );
    }

    /// Settings: stop scanning a folder. Clones already added stay.
    pub fn remove_clone_folder(&mut self, i: usize) {
        if i < self.prefs.clone_folders.len() {
            self.prefs.clone_folders.remove(i);
            self.save_prefs();
        }
    }

    fn add_clone_folder(&mut self, dir: PathBuf) {
        if !self.prefs.clone_folders.contains(&dir) {
            self.prefs.clone_folders.push(dir);
            self.save_prefs();
        }
        self.rescan_folders(true);
    }

    fn add_local_repo(&mut self, dir: PathBuf) {
        let Some(top) = git::toplevel(&dir) else {
            self.toast(crate::i18n::t(tf!("{} isn't a git repository", dir.display())), true);
            return;
        };
        let Some(slug) = git::slug_of(&top) else {
            self.toast(crate::i18n::t("Its `origin` remote isn't on GitHub"), true);
            return;
        };
        self.add_repo(slug.clone(), Some(top));
        self.save_repo_list();
        self.select_repo(&slug, Page::Commits);
    }
}

/// "Add local repo…" / "Locate local clone…": the OS folder picker.
pub fn ask_local_repo(s: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions { title: "Choose a local git repository".into(), directory: s.local_path(), ..Default::default() };
    tx.open_folder(options, |s: &mut State, r, _| match r {
        Ok(Some(dir)) => s.add_local_repo(dir),
        Ok(None) => {}
        Err(e) => s.toast(crate::i18n::t(e.to_string()), true),
    });
}

/// Settings → Git "Add folder…": the OS folder picker, then a scan of the folder.
pub fn ask_clone_folder(_s: &mut State, tx: &mut Tx) {
    let options = FileDialogOptions { title: "Choose a folder that holds your git clones".into(), ..Default::default() };
    tx.open_folder(options, |s: &mut State, r, _| match r {
        Ok(Some(dir)) => s.add_clone_folder(dir),
        Ok(None) => {}
        Err(e) => s.toast(crate::i18n::t(e.to_string()), true),
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
/// Rune calls these three into sigit every frame and on every key. A panic in one is caught
/// here and shown in the error dialog instead of closing the app (`crash_log`).
fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    let changed = crash_log::catch(|| poll_inner(s, clock, tx)).unwrap_or(true);
    let new = crash_log::take();
    let shown = !new.is_empty();
    s.crash.extend(new);
    changed || shown
}

fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    crash_log::catch(|| on_key_inner(s, ev, tx)).unwrap_or(true)
}

/// The app, or when building it panicked, a screen with the error and a way out.
fn view(s: &State, cx: &mut Cx<State>) -> El {
    match crash_log::catch(|| ui::view(s, cx)) {
        Some(el) => el,
        None => ui::crash_screen(s, cx),
    }
}

fn poll_inner(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    s.clock = clock;
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
fn on_key_inner(s: &mut State, ev: &KeyEvent, _: &mut Tx) -> bool {
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

/// The theme's Inter and IBM Plex Mono, plus Noto Sans Thai (SIL Open Font License, see
/// `assets/fonts/NotoSansThai-OFL.txt`) so Thai looks the same on every computer.
const FONTS: &[&[u8]] = &[
    rune_theme_opencode::FONTS[0],
    rune_theme_opencode::FONTS[1],
    rune_theme_opencode::FONTS[2],
    include_bytes!("../assets/fonts/NotoSansThai.ttf"),
];
/// Latin in Inter; Thai, which Inter lacks, falls through to Noto Sans Thai.
const FAMILIES: (&str, &str) = ("Inter, Noto Sans Thai", "IBM Plex Mono, Noto Sans Thai");

/// The Comparisons tab with fixed rows (no network): waiting, up to date, missing branches,
/// an error row, a turned-off cell and a retargeted one.
fn compare_fixture(s: &mut State) {
    use compare::{Cell, LooseCommit, Override, RepoCompare, WaitingPr};
    s.prefs.tracked = ["acme/web", "acme/api", "acme/docs", "acme/admin", "acme/gone"].map(String::from).to_vec();
    s.prefs.comparisons = compare::defaults();
    // Comparisons are grouped like the sidebar; acme/docs and acme/gone land in "Other".
    s.prefs.groups = vec![
        config::Group { name: "Storefront".into(), repos: vec!["acme/web".into(), "acme/api".into()] },
        config::Group { name: "Back office".into(), repos: vec!["acme/admin".into(), "acme/api".into()] },
    ];
    s.prefs.comparison_overrides = vec![
        Override { slug: "acme/web".into(), name: "To production".into(), to: Some("master".into()), ..Default::default() },
        Override { slug: "acme/admin".into(), name: "Back-merge".into(), off: true, ..Default::default() },
    ];
    let pr = |number: u64, title: &str, author: &str, day: u32| WaitingPr { number, title: title.into(), author: author.into(), merged_at: format!("2026-11-{day:02}T09:00:00Z") };
    let loose = |sha: &str, subject: &str| LooseCommit { sha: sha.into(), subject: subject.into(), author: "mai".into() };
    let wait = |commits, prs, loose, checked| Cell::Waiting { commits, prs, loose, checked };
    let missing = |b: &[&str]| Cell::Missing(b.iter().map(|b| b.to_string()).collect());
    let row = |slug: &str, cells, error: Option<&str>| RepoCompare { slug: slug.into(), cells, error: error.map(str::to_owned) };
    let many = (0..39).map(|n| pr(300 + n, &format!("Sample change {n}"), "ploy", 1 + n as u32 % 18)).collect();
    let rows = vec![
        row("acme/admin", vec![wait(149, many, vec![loose("9c1e2d4a", "Sample hotfix")], 100), Cell::Off, wait(0, vec![], vec![], 0)], None),
        row(
            "acme/api",
            vec![
                wait(8, vec![pr(141, "Sample rate limit", "ploy", 18), pr(139, "Sample bet history", "nok", 17), pr(137, "Sample payout fix", "ploy", 15)], vec![loose("4be07f11", "Sample header logo")], 8),
                wait(0, vec![], vec![], 0),
                wait(7, (116..123).map(|n| pr(n, &format!("Sample release {n}"), "nok", 10)).collect(), vec![], 7),
            ],
            None,
        ),
        row("acme/docs", vec![missing(&["dev", "staging"]), missing(&["staging", "dev"]), missing(&["staging", "main"])], None),
        row("acme/gone", vec![Cell::Off, Cell::Off, Cell::Off], Some("Could not resolve to a Repository with the name 'acme/gone'.")),
        row("acme/web", vec![missing(&["dev"]), missing(&["dev"]), wait(1, vec![pr(88, "Sample footer links", "mai", 12)], vec![], 1)], None),
    ];
    s.inbox_tab = COMPARE_TAB;
    s.compare = Load::default();
    s.compare.finish(Ok(rows));
    s.compare_key = s.compare_key_now();
    // Details for acme/api "To staging" (the lane the scenes open): two authors, three folders.
    let person = |login: &str, name: &str, company: &str| compare::Person { login: login.into(), name: name.into(), company: company.into(), ..Default::default() };
    let mut ploy = person("ploy", "Ploy Srisuk", "Acme Labs");
    // One author with a picture (the app icon stands in, so scenes need no network), one without.
    ploy.avatar_url = format!("{}u/1?s=64", gh::AVATAR_HOST);
    // Snapshot runs have no window waker; a cache of their own still decodes.
    let cache = s.images.get_or_insert_with(|| rune::ImageCache::new(std::sync::Arc::new(|| {})));
    {
        let icon: &[u8] = include_bytes!("../assets/icon-256.png");
        s.avatars.insert(ploy.avatar_url.clone(), cache.load(icon.to_vec()));
    }
    let nok = person("nok", "Nok Chai", "");
    let detail = |number: u64, labels: &[&str], summary: &str, author: &compare::Person, files: &[(&str, u64, u64)]| compare::PrDetail {
        number,
        labels: labels.iter().map(|l| l.to_string()).collect(),
        summary: summary.into(),
        additions: files.iter().map(|f| f.1).sum(),
        deletions: files.iter().map(|f| f.2).sum(),
        author: author.clone(),
        files: files.iter().map(|(p, a, r)| (p.to_string(), *a, *r)).collect(),
        ..Default::default()
    };
    let details = vec![
        detail(141, &["feature"], "Sample summary: limit agent requests to 60 a minute.", &ploy, &[("src/feature/agent/service/limit.ts", 120, 14), ("docs/limits.md", 18, 0)]),
        compare::PrDetail {
            added_files: vec!["src/feature/history/view/History.vue".into(), "src/feature/history/service/history.ts".into(), "src/feature/history/route.ts".into()],
            ..detail(139, &["feature", "report"], "Sample summary: show each bet's history on its own page.", &nok, &[("src/feature/history/view/History.vue", 210, 0), ("src/feature/history/service/history.ts", 35, 0), ("src/feature/history/route.ts", 12, 0)])
        },
        detail(137, &["bug"], "", &ploy, &[("src/feature/agent/view/Payout.vue", 12, 9), ("src/shared/component/Table.vue", 6, 2)]),
    ];
    s.lane_details.insert(("acme/api".into(), vec![137, 139, 141]), Load { data: Some(details), ..Default::default() });
    // The module layout learned from acme/api's dev branch (src/feature/* are modules).
    let mut tree: Vec<String> = Vec::new();
    for m in ["agent", "report", "history", "auth"] {
        for f in ["view/Main.vue", "service/index.ts", "locale/en.ts", "route.ts"] {
            tree.push(format!("src/feature/{m}/{f}"));
        }
    }
    tree.extend(["src/shared/component/Table.vue".to_owned(), "docs/limits.md".to_owned()]);
    s.layouts.insert(("acme/api".into(), "dev".into()), Load { data: Some(modules::learn(&tree)), ..Default::default() });
    let key = ("acme/api".to_string(), vec![137, 139, 141]);
    s.lane_branches.insert(key.clone(), "dev".into());
    s.rebuild_lane_model(&key);
}

/// The commit without a PR in acme/api's "To staging" lane, opened, its files loaded.
fn open_commit_fixture(s: &mut State) {
    let key = ("acme/api".to_string(), "4be07f11".to_string());
    let files = vec![("src/feature/agent/view/Header.vue".to_string(), 14, 3), ("src/shared/component/Table.vue".to_string(), 4, 1), ("assets/logo.svg".to_string(), 6, 0)];
    s.commit_files.insert(key.clone(), Load::start());
    s.commit_files_arrived(&key, Ok(files));
    s.open_commits.insert(key);
}

/// acme/api's Branches page (Local) with feat/rate-limit picked: its lane against main brings
/// the PRs and commit of the "To staging" lane, so the details and layout are reused (no network).
fn branches_fixture(s: &mut State) {
    compare_fixture(s);
    let slug = "acme/api".to_string();
    s.add_repo(slug.clone(), Some(PathBuf::from("C:/fixture/acme-api")));
    s.cur = Some(slug.clone());
    s.page = Page::Branches;
    s.branch_view = 0;
    let b = |name: &str, remote: bool, current: bool, ahead: u32, behind: u32, subject: &str| model::Branch {
        name: name.into(),
        remote,
        current,
        upstream: if remote { String::new() } else { format!("origin/{name}") },
        ahead,
        behind,
        time: model::now() - 3600 * i64::from(ahead + behind + 1),
        author: "ploy".into(),
        subject: subject.into(),
        ..Default::default()
    };
    let list = vec![
        b("main", false, true, 0, 0, "Sample release 2026.11"),
        b("feat/rate-limit", false, false, 2, 0, "Sample rate limit"),
        b("fix/payout", false, false, 0, 3, "Sample payout fix"),
        b("origin/main", true, false, 0, 0, "Sample release 2026.11"),
        b("origin/dev", true, false, 0, 0, "Sample bet history"),
    ];
    let d = s.rd_mut(&slug);
    d.branches = Load::default();
    d.branches.finish(Ok(list));
    d.meta = Load::default();
    d.meta.finish(Ok(model::RepoMeta { default_branch: "main".into(), ..Default::default() }));
    let cell = s.compare.data.as_ref().and_then(|r| r.iter().find(|r| r.slug == slug)).and_then(|r| r.cells.first().cloned()).unwrap_or(compare::Cell::Off);
    let mut lane = Load::default();
    lane.finish(Ok(compare::RepoCompare { slug: slug.clone(), cells: vec![cell], error: None }));
    s.branch_lanes.insert((slug.clone(), "feat/rate-limit".into(), "main".into()), lane);
    s.branch_lane_sel = Some((slug, "feat/rate-limit".into()));
    s.branch_sel = Some("feat/rate-limit".into());
    s.branches_table.select("feat/rate-limit");
}

fn main() {
    config::migrate_old_folder(); // before the crash log makes the folder
    crash_log::install();
    #[cfg(target_os = "macos")]
    gh::fix_path();
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
            ("comparisons", |s: &mut State| {
                s.show(Page::Inbox);
                compare_fixture(s);
                s.compare_sel = Some(("acme/api".into(), 0));
            }),
            ("comparisons-by-contributor", |s: &mut State| {
                s.show(Page::Inbox);
                compare_fixture(s);
                s.compare_sel = Some(("acme/api".into(), 0));
                s.lane_view.by_contributor = true;
                open_commit_fixture(s);
            }),
            ("branches-detail", |s: &mut State| {
                branches_fixture(s);
            }),
            ("pr-dialog", |s: &mut State| {
                // feat/rate-limit pushed, the new-PR dialog open on it.
                branches_fixture(s);
                if let Some(b) = s.rd_mut("acme/api").branches.data.as_mut() {
                    b.push(model::Branch { name: "origin/feat/rate-limit".into(), remote: true, ..Default::default() });
                }
                s.open_pr_dialog("acme/api", "feat/rate-limit", "main");
            }),
            ("comparisons-modules-open", |s: &mut State| {
                // The lane pane with the Modules section expanded.
                s.show(Page::Inbox);
                compare_fixture(s);
                s.compare_sel = Some(("acme/api".into(), 0));
                s.lane_view.modules_open = true;
            }),
            ("comparisons-group", |s: &mut State| {
                // The Comparisons tab filtered to one group.
                s.show(Page::Inbox);
                compare_fixture(s);
                s.compare_sel = None;
                s.repo_scope = Some(RepoScope::Group("Back office".into()));
            }),
            ("comparisons-edit", |s: &mut State| {
                s.show(Page::Inbox);
                compare_fixture(s);
                s.open_compare_edit("acme/web");
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
                // Any repo with a local clone (Commits reads one).
                let local = s.repos.iter().find(|r| r.path.is_some()).map_or_else(|| "anomalyco/rune".to_owned(), |r| r.slug.clone());
                s.select_repo(&local, Page::Commits);
                if let Some(sha) = s.rd().and_then(|d| d.graph.data.as_ref()).and_then(|g| g.commits.get(1)).map(|c| c.sha.clone()) {
                    s.select_commit(&sha);
                }
            }),
            ("branches", |s: &mut State| {
                s.branch_view = 1;
                let local = s.repos.iter().find(|r| r.path.is_some()).map_or_else(|| "anomalyco/rune".to_owned(), |r| r.slug.clone());
                s.select_repo(&local, Page::Branches);
            }),
            ("merge", |s: &mut State| {
                // Only the risk check runs (read-only); nothing is merged without the dialog.
                s.select_repo("anomalyco/rune", Page::Pulls);
                let pr = s.rd().and_then(|d| d.prs.data.as_ref()).and_then(|p| p.first()).map(|p| (p.number, p.title.clone()));
                if let Some((number, title)) = pr {
                    s.request(ui::merge_action("anomalyco/rune", number, &title));
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
            ("settings-safety", |s: &mut State| s.open_settings(3)),
            ("settings-about", |s: &mut State| s.open_settings(8)),
            ("cmd-failed-run", |s: &mut State| {
                // The first run, shown as failed (Re-run failed jobs, Why it failed).
                s.select_repo("anomalyco/rune", Page::Actions);
                let id = s.data.get_mut("anomalyco/rune").and_then(|d| d.runs.data.as_mut()).and_then(|r| r.first_mut()).map(|r| {
                    r.status = "completed".into();
                    r.conclusion = "failure".into();
                    r.database_id
                });
                if let Some(id) = id {
                    s.select_run(id);
                }
            }),
            ("cmd-running-run", |s: &mut State| {
                // The first run, shown as running (Cancel run).
                s.select_repo("anomalyco/rune", Page::Actions);
                let id = s.data.get_mut("anomalyco/rune").and_then(|d| d.runs.data.as_mut()).and_then(|r| r.first_mut()).map(|r| {
                    r.status = "in_progress".into();
                    r.conclusion = String::new();
                    r.database_id
                });
                if let Some(id) = id {
                    s.select_run(id);
                }
            }),
            ("cmd-stash-continue", |s: &mut State| {
                // A checkout that finds uncommitted changes: the dialog offers Stash and continue.
                let local = s.repos.iter().find(|r| r.path.is_some()).map(|r| (r.slug.clone(), r.path.clone().unwrap_or_default()));
                if let Some((slug, dir)) = local {
                    s.select_repo(&slug, Page::Branches);
                    s.pending = Some(guard::Pending {
                        action: guard::Action::Checkout { dir, slug, name: "dev".into(), remote: false },
                        checking: false,
                        risk: risk::Risk::new(risk::Level::Medium),
                        blocked: None,
                        dirty: vec!["src/cli.rs".into(), "README.md".into()],
                        word: None,
                    });
                }
            }),
            ("cmd-comment", |s: &mut State| {
                // A PR with a short description, so the comment box and its button are in view.
                s.select_repo("anomalyco/rune", Page::Pulls);
                s.comment = TextEdit::new("Looks good, thanks!");
                let n = s.rd().and_then(|d| d.prs.data.as_ref()).and_then(|p| p.iter().find(|p| p.number == 942).or(p.last())).map(|p| p.number);
                if let Some(n) = n {
                    s.select_pr(n);
                }
            }),
            ("cmd-pr-local", |s: &mut State| {
                // A PR in a repo with a local clone (this folder stands in for it), so "Check out
                // locally" shows. Nothing runs.
                s.select_repo("anomalyco/rune", Page::Pulls);
                s.add_repo_pub("anomalyco/rune".into(), std::env::current_dir().ok());
                if let Some(n) = s.rd().and_then(|d| d.prs.data.as_ref()).and_then(|p| p.first()).map(|p| p.number) {
                    s.select_pr(n);
                }
            }),
            ("cmd-countdown", |s: &mut State| {
                // A merge counting down (an hour away, so nothing ever runs).
                s.select_repo("anomalyco/rune", Page::Pulls);
                let action = guard::Action::Merge { slug: "anomalyco/rune".into(), number: 1004, title: String::new(), method: gh::MergeMethod::Squash, delete_branch: true };
                s.grace = Some(guard::Grace { action, at: Instant::now() + Duration::from_secs(3600) });
            }),
            ("aged-prs", |s: &mut State| {
                // Pull requests fetched a minute ago: fresh for the 30 s tick, so only a
                // revalidation (coming back to the window) reloads them.
                s.select_repo("anomalyco/rune", Page::Pulls);
                if let Some(d) = s.data.get_mut("anomalyco/rune") {
                    d.prs.at = Some(model::now() - 60);
                    d.status.at = Some(model::now() - 60);
                }
            }),
            // Screens for checking each button's command tooltip (sample data where the real
            // repo has none; nothing runs).
            ("cmd-stashes", |s: &mut State| {
                let local = s.repos.iter().find(|r| r.path.is_some()).map(|r| r.slug.clone()).unwrap_or_default();
                s.select_repo(&local, Page::Branches);
                s.branch_view = 2;
                s.refresh_branch_tools();
                if let Some(d) = s.data.get_mut(&local) {
                    let t = model::now();
                    d.stashes.data = Some(vec![
                        model::Stash { name: "stash@{0}".into(), message: "On main: sigit: before Checkout dev".into(), time: t - 600 },
                        model::Stash { name: "stash@{1}".into(), message: "WIP on feat: try a new layout".into(), time: t - 86_400 },
                    ]);
                }
            }),
            ("cmd-cleanup", |s: &mut State| {
                let local = s.repos.iter().find(|r| r.path.is_some()).map(|r| r.slug.clone()).unwrap_or_default();
                s.select_repo(&local, Page::Branches);
                s.branch_view = 3;
                s.refresh_branch_tools();
                if let Some(d) = s.data.get_mut(&local) {
                    d.cleanup.data = Some(vec![
                        model::Cleanup { name: "fix/old-typo".into(), sha: "a1b2c3d".into(), merged: true, gone: true, stale_days: None, unpushed: Vec::new() },
                        model::Cleanup { name: "spike/idea".into(), sha: "d4e5f6a".into(), merged: false, gone: false, stale_days: Some(120), unpushed: vec!["d4e5f6a try it".into()] },
                        model::Cleanup { name: "feat/done".into(), sha: "b7c8d9e".into(), merged: true, gone: false, stale_days: None, unpushed: Vec::new() },
                    ]);
                }
            }),
            ("cmd-undo", |s: &mut State| {
                s.select_repo("anomalyco/rune", Page::Pulls);
                s.toast_undo("✓ Closed pull request #1004".into(), Some(guard::Undo::ReopenPr { slug: "anomalyco/rune".into(), number: 1004 }));
            }),
            ("cmd-accounts", |s: &mut State| {
                s.open_settings(1);
                let acct = |login: &str, active: bool| gh::Account { host: "github.com".into(), login: login.into(), active, ok: true, scopes: "repo, workflow".into(), source: "keyring".into(), protocol: "https".into() };
                s.accounts.data = Some(vec![acct("octocat", true), acct("octo-work", false)]);
            }),
            ("cmd-git-identity", |s: &mut State| {
                s.open_settings(2);
                s.git_name = TextEdit::new("Ann Lee");
                s.git_email = TextEdit::new("ann@example.com");
            }),
            ("cmd-branch-checkout", |s: &mut State| {
                let local = s.repos.iter().find(|r| r.path.is_some()).map(|r| r.slug.clone()).unwrap_or_default();
                s.branch_view = 1;
                s.select_repo(&local, Page::Branches);
                s.branch_sel = Some("origin/main".into());
            }),
            ("cmd-commit-checkout", |s: &mut State| {
                let local = s.repos.iter().find(|r| r.path.is_some()).map(|r| r.slug.clone()).unwrap_or_default();
                s.select_repo(&local, Page::Commits);
                // A commit with a branch on it, so it offers "Check out".
                let sha = s.rd().and_then(|d| d.graph.data.as_ref()).and_then(|g| g.commits.iter().find(|c| c.refs.iter().any(|r| !r.starts_with("tag: ") && !r.starts_with("HEAD")))).map(|c| c.sha.clone());
                if let Some(sha) = sha {
                    s.select_commit(&sha);
                }
            }),
            ("activity-log-commands", |s: &mut State| {
                // The activity log with entries that ran commands (sample entries, nothing runs).
                s.open_settings(6);
                let t = model::now();
                let e = |at: i64, action: &str, ok: bool, detail: &str, commands: &[&str]| activity::Entry {
                    at: t - at, repo: "anomalyco/rune".into(), action: action.into(), ok, detail: detail.into(), commands: commands.iter().map(|c| (*c).to_owned()).collect(),
                };
                s.activity = vec![
                    e(60, "Merge #1004", true, "✓ Squashed and merged pull request #1004", &["gh pr merge 1004 -R anomalyco/rune --squash --delete-branch"]),
                    e(600, "Checkout origin/feat", true, "Switched to feat", &["git stash push --include-untracked -m \"sigit: before Checkout origin/feat\"", "git switch feat", "git switch --track origin/feat"]),
                    e(3600, "Re-run failed jobs of 4521", false, "run 4521 cannot be rerun; its workflow file may be broken", &["gh run rerun 4521 -R anomalyco/rune --failed"]),
                ];
            }),
            ("merge-no-commands", |s: &mut State| {
                // The merge dialog with Settings → Show gh / git commands off.
                s.prefs.show_commands = false;
                s.select_repo("anomalyco/rune", Page::Pulls);
                let pr = s.rd().and_then(|d| d.prs.data.as_ref()).and_then(|p| p.first()).map(|p| (p.number, p.title.clone()));
                if let Some((number, title)) = pr {
                    s.request(ui::merge_action("anomalyco/rune", number, &title));
                }
            }),
            ("toast-commands", |s: &mut State| {
                // What a finished action's result looks like (nothing actually runs).
                s.select_repo("anomalyco/rune", Page::Pulls);
                let a = guard::Action::Merge { slug: "anomalyco/rune".into(), number: 1004, title: String::new(), method: gh::MergeMethod::Squash, delete_branch: true };
                let ran = s.ran_lines(&a.command_lines(false));
                s.toast(format!("✓ Squashed and merged pull request #1004{ran}"), false);
            }),
            // The light theme (scenes otherwise keep the saved theme, whatever --theme says).
            ("light-needs", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                s.inbox_tab = 1;
                s.show(Page::Inbox);
            }),
            ("light-comparisons", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                s.show(Page::Inbox);
                compare_fixture(s);
                s.compare_sel = Some(("acme/admin".into(), 0));
            }),
            ("light-comparisons-by-contributor", |s: &mut State| {
                // The open lane in By contributor with the docs folder picked as a filter.
                s.prefs.theme = config::Theme::Light;
                s.show(Page::Inbox);
                compare_fixture(s);
                s.compare_sel = Some(("acme/api".into(), 0));
                s.lane_view = LaneView { by_contributor: true, folder: Some(("docs".into(), false)), ..Default::default() };
            }),
            ("light-activity", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                s.show(Page::Inbox);
            }),
            ("light-prs", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                s.select_repo("anomalyco/rune", Page::Pulls);
                if let Some(n) = s.rd().and_then(|d| d.prs.data.as_ref()).and_then(|p| p.first()).map(|p| p.number) {
                    s.select_pr(n);
                }
            }),
            ("light-envs", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                s.select_repo("anomalyco/rune", Page::Envs);
            }),
            ("light-branches-detail", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                branches_fixture(s);
            }),
            ("light-branches", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                let local = s.repos.iter().find(|r| r.path.is_some()).map_or_else(|| "anomalyco/rune".to_owned(), |r| r.slug.clone());
                s.select_repo(&local, Page::Branches);
            }),
            ("light-settings", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                s.open_settings(0);
            }),
            ("light-commits", |s: &mut State| {
                s.prefs.theme = config::Theme::Light;
                let local = s.repos.iter().find(|r| r.path.is_some()).map_or_else(|| "anomalyco/rune".to_owned(), |r| r.slug.clone());
                s.select_repo(&local, Page::Commits);
                if let Some(sha) = s.rd().and_then(|d| d.graph.data.as_ref()).and_then(|g| g.commits.get(1)).map(|c| c.sha.clone()) {
                    s.select_commit(&sha);
                }
            }),
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
        fonts: FONTS,
        families: FAMILIES,
        poll: Some(poll),
        setup: Some(|s: &mut State, waker| s.images = Some(rune::ImageCache::new(waker))),
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
    // An error the catches above couldn't stop ends the event loop: save it and start again.
    if crash_log::catch(|| app.run()).is_none() {
        crash_log::restart();
        std::process::exit(101);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn commit_files_retry_after_error() {
        let mut errored: Load<Vec<(String, u64, u64)>> = Load::start();
        errored.finish(Err("offline".into()));
        let mut loaded: Load<Vec<(String, u64, u64)>> = Load::start();
        loaded.finish(Ok(vec![]));
        assert!(State::wants_commit_fetch(None));
        assert!(State::wants_commit_fetch(Some(&errored)));
        assert!(!State::wants_commit_fetch(Some(&Load::start())));
        assert!(!State::wants_commit_fetch(Some(&loaded)));
    }

    #[test]
    fn open_commit_is_per_repo() {
        let mut s = State::new(config::Prefs::default());
        let mut l: Load<Vec<(String, u64, u64)>> = Load::start();
        l.finish(Ok(vec![]));
        s.commit_files.insert(("a/r".to_owned(), "abc".to_owned()), l);
        s.toggle_commit("a/r", "abc");
        assert!(s.open_commits.contains(&("a/r".to_owned(), "abc".to_owned())));
        assert!(!s.open_commits.contains(&("b/r".to_owned(), "abc".to_owned())));
    }

    #[test]
    fn toggle_commit_closes_without_fetch() {
        let mut s = State::new(config::Prefs::default());
        let key = ("o/r".to_owned(), "abc".to_owned());
        let mut l: Load<Vec<(String, u64, u64)>> = Load::start();
        l.finish(Ok(vec![("a".into(), 1, 0)]));
        s.commit_files.insert(key.clone(), l);
        s.open_commits.insert(key.clone());
        s.toggle_commit("o/r", "abc");
        assert!(!s.open_commits.contains(&key));
        assert_eq!(s.commit_files[&key].data.as_deref(), Some(&[("a".to_owned(), 1, 0)][..]));
    }

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

    /// Opening a page re-checks data the background tick still counts as fresh, but not what
    /// was fetched a moment ago (flicking between tabs) or Activity within its minute.
    #[test]
    fn revalidate_on_open() {
        let mut prs: Load<u32> = Load::default();
        prs.finish(Ok(1));
        prs.at = Some(model::now() - 30);
        assert!(!prs.stale(ttl::GH), "the 30 s tick leaves a 30 s old PR list");
        assert!(prs.stale(ttl::GH.min(ttl::REVALIDATE)), "opening the page checks it again");
        prs.at = Some(model::now() - 3);
        assert!(!prs.stale(ttl::GH.min(ttl::REVALIDATE)), "just fetched: no second request");
        let pulse_cap = ttl::PULSE.min(ttl::REVALIDATE.max(ttl::PULSE_REVALIDATE));
        prs.at = Some(model::now() - 30);
        assert!(!prs.stale(pulse_cap), "Activity waits a minute between checks");
        prs.at = Some(model::now() - 90);
        assert!(prs.stale(pulse_cap));
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
    fn refresh_compare_skips_when_nothing_to_compare() {
        let mut s = State::new(config::Prefs::default());
        s.prefs.tracked.clear();
        s.prefs.pinned.clear();
        s.prefs.groups.clear();
        s.refresh_compare();
        assert!(!s.compare.loading && s.compare.data.is_none());
    }

    fn loaded_compare(s: &mut State) {
        s.compare = Load::from_cache(vec![compare::RepoCompare { slug: "a/b".into(), cells: vec![], error: None }], 5);
        s.compare_key = "old".into();
    }

    #[test]
    fn refresh_compare_clears_old_rows_when_no_repos() {
        let mut s = State::new(config::Prefs::default());
        s.prefs.tracked.clear();
        s.prefs.pinned.clear();
        s.prefs.groups.clear();
        loaded_compare(&mut s);
        s.refresh_compare();
        assert!(s.compare.data.is_none() && s.compare.at.is_none() && s.compare_key.is_empty());
    }

    #[test]
    fn refresh_compare_clears_old_rows_when_no_comparisons() {
        let mut s = State::new(config::Prefs::default());
        s.prefs.tracked = vec!["a/b".into()];
        s.prefs.comparisons.clear();
        loaded_compare(&mut s);
        s.refresh_compare();
        assert!(s.compare.data.is_none() && s.compare.at.is_none() && s.compare_key.is_empty());
    }

    #[test]
    fn compare_edit_prefills_saves_and_resets() {
        let mut s = State::new(config::Prefs::default());
        s.prefs.comparison_overrides = vec![
            compare::Override { slug: "a/b".into(), name: "To production".into(), to: Some("master".into()), ..Default::default() },
            compare::Override { slug: "x/y".into(), name: "To staging".into(), off: true, ..Default::default() },
        ];
        s.open_compare_edit("a/b");
        let e = s.compare_edit.as_ref().unwrap();
        let shown: Vec<_> = e.rows.iter().map(|(f, t, on)| (f.text.as_str(), t.text.as_str(), *on)).collect();
        assert_eq!(shown, vec![("dev", "staging", true), ("staging", "dev", true), ("staging", "master", true)]);

        let e = s.compare_edit.as_mut().unwrap();
        e.rows[0].2 = false;
        e.rows[2].1 = TextEdit::new("main"); // back to shared
        s.save_compare_edit();
        assert!(s.compare_edit.is_none());
        let mine: Vec<_> = s.prefs.comparison_overrides.iter().filter(|o| o.slug == "a/b").map(|o| (o.name.as_str(), o.off)).collect();
        assert_eq!(mine, vec![("To staging", true)]);
        assert!(s.prefs.comparison_overrides.iter().any(|o| o.slug == "x/y"), "other repos keep theirs");

        s.open_compare_edit("a/b");
        s.reset_compare_edit();
        assert!(s.compare_edit.is_none());
        assert_eq!(s.prefs.comparison_overrides.iter().map(|o| o.slug.as_str()).collect::<Vec<_>>(), vec!["x/y"]);
    }

    #[test]
    fn refresh_compare_clears_positional_state_when_key_changes() {
        let mut s = State::new(config::Prefs::default());
        s.prefs.tracked = vec!["a/b".into()];
        s.compare_key = "old".into();
        s.compare_sel = Some(("a/b".into(), 1));
        s.open_compare_edit("a/b");
        s.refresh_compare();
        assert!(s.compare_sel.is_none() && s.compare_edit.is_none());

        s.prefs.comparisons.clear();
        s.compare_sel = Some(("a/b".into(), 0));
        s.open_compare_edit("a/b");
        s.refresh_compare();
        assert!(s.compare_sel.is_none() && s.compare_edit.is_none());
    }

    #[test]
    fn model_rebuilds_on_each_input_once() {
        let mut s = lane_state();
        s.compare_sel = Some(("a/b".into(), 0));
        let key = s.lane_key().unwrap();
        let branch = s.lane_branch().unwrap();
        // Both already on the way, so opening the lane fetches nothing; it notes the lane's branch.
        s.lane_details.insert(key.clone(), Load::start());
        s.layouts.insert(branch.clone(), Load::start());
        s.load_lane_details(false);
        assert!(!s.lane_models.contains_key(&key));
        let d = compare::PrDetail { number: 7, files: vec![("src/feature/agent/view/a.vue".into(), 5, 1), ("docs/x.md".into(), 2, 0)], ..Default::default() };
        s.details_arrived(&key, Ok(vec![d]));
        assert!(s.lane_models[&key].summary.is_none(), "no layout yet: folders only");
        assert_eq!(s.lane_models[&key].details.len(), 1);
        let mut tree = Vec::new();
        for m in ["agent", "report", "auth"] {
            tree.extend([format!("src/feature/{m}/view/a.vue"), format!("src/feature/{m}/service/a.ts")]);
        }
        s.layout_arrived(&branch, Ok(modules::learn(&tree)));
        let built = s.lane_models[&key].clone();
        assert!(built.summary.is_some());
        assert_eq!(built.colors.get("src/feature/agent"), Some(&0));
        s.rebuild_lane_model(&key);
        assert_eq!(s.lane_models[&key], built, "same inputs, same model");
        // A layout for another branch of the repo leaves this lane's model alone.
        s.layout_arrived(&("a/b".to_string(), "elsewhere".to_string()), Ok(modules::Layout::default()));
        assert_eq!(s.lane_models[&key], built);
    }

    fn lane_state() -> State {
        let waiting = |n: u64| compare::Cell::Waiting {
            commits: 1,
            prs: vec![compare::WaitingPr { number: n, title: "t".into(), author: "a".into(), merged_at: "2026-01-01T00:00:00Z".into() }],
            loose: vec![],
            checked: 1,
        };
        let mut s = State::new(config::Prefs::default());
        s.inbox_tab = COMPARE_TAB;
        s.compare = Load { data: Some(vec![compare::RepoCompare { slug: "a/b".into(), cells: vec![waiting(7), waiting(9)], error: None }]), ..Default::default() };
        s
    }

    #[test]
    fn lane_details_are_keyed_by_lane() {
        let mut s = lane_state();
        s.compare_sel = Some(("a/b".into(), 0));
        let k0 = s.lane_key().unwrap();
        s.compare_sel = Some(("a/b".into(), 1));
        let k1 = s.lane_key().unwrap();
        assert_ne!(k0, k1);
        assert_eq!(k0, ("a/b".to_string(), vec![7]));
        s.lane_details.insert(k0.clone(), Load::start());
        assert!(!s.lane_details.contains_key(&k1));
    }

    #[test]
    fn switching_lane_clears_folder_filter() {
        let mut s = lane_state();
        s.lane_view.by_contributor = true;
        for j in 0..2 {
            let key = ("a/b".to_string(), vec![if j == 0 { 7 } else { 9 }]);
            s.lane_details.insert(key, Load::start());
        }
        s.select_lane("a/b", 0);
        s.lane_view.folder = Some(("src".into(), false));
        s.select_lane("a/b", 1);
        assert_eq!(s.compare_sel, Some(("a/b".into(), 1)));
        assert_eq!(s.lane_view.folder, None);
        assert!(s.lane_view.by_contributor);
        s.select_lane("a/b", 1);
        assert_eq!(s.compare_sel, None);
    }

    #[test]
    fn lane_key_follows_the_pr_set_after_a_refresh() {
        let mut s = lane_state();
        s.compare_sel = Some(("a/b".into(), 0));
        let old = s.lane_key().unwrap();
        s.lane_details.insert(old.clone(), Load { data: Some(vec![]), ..Default::default() });
        let waiting = |nums: &[u64]| compare::Cell::Waiting {
            commits: 2,
            prs: nums.iter().map(|&n| compare::WaitingPr { number: n, title: "t".into(), author: "a".into(), merged_at: "2026-01-01T00:00:00Z".into() }).collect(),
            loose: vec![],
            checked: 2,
        };
        s.compare = Load { data: Some(vec![compare::RepoCompare { slug: "a/b".into(), cells: vec![waiting(&[7, 8]), waiting(&[9])], error: None }]), ..Default::default() };
        let new = s.lane_key().unwrap();
        assert_ne!(old, new);
        assert_eq!(new, ("a/b".to_string(), vec![7, 8]));
        // Pre-seed the new key as loading so the reload is a no-op instead of a network call.
        s.lane_details.insert(new.clone(), Load::start());
        s.load_lane_details(false);
        assert!(s.lane_details[&new].loading);
    }

    #[test]
    fn lane_without_prs_loads_nothing() {
        let mut s = lane_state();
        s.compare = Load {
            data: Some(vec![compare::RepoCompare {
                slug: "a/b".into(),
                cells: vec![compare::Cell::Waiting { commits: 1, prs: vec![], loose: vec![], checked: 1 }],
                error: None,
            }]),
            ..Default::default()
        };
        s.compare_sel = Some(("a/b".into(), 0));
        s.load_lane_details(false);
        assert!(s.lane_details.is_empty());
    }

    #[test]
    fn loaded_lane_is_not_refetched() {
        let mut s = lane_state();
        s.compare_sel = Some(("a/b".into(), 0));
        let key = s.lane_key().unwrap();
        s.lane_details.insert(key.clone(), Load { data: Some(vec![]), ..Default::default() });
        s.load_lane_details(false);
        let l = &s.lane_details[&key];
        assert!(!l.loading && l.data.is_some());
    }

    #[test]
    fn lane_refreshed_off_tab_loads_on_return() {
        let mut s = lane_state();
        s.compare_sel = Some(("a/b".into(), 0));
        let old = s.lane_key().unwrap();
        s.lane_details.insert(old, Load { data: Some(vec![]), ..Default::default() });
        assert!(!s.lane_needs_details());
        // Off the tab, a refresh lands with another PR set; its completion can't load the lane.
        s.inbox_tab = 0;
        let mut fresh = s.compare.data.clone().unwrap();
        fresh[0].cells[0] = branch_cell(2, 8);
        s.compare = Load { data: Some(fresh), ..Default::default() };
        s.load_lane_details(false);
        assert!(!s.lane_details.contains_key(&("a/b".to_string(), vec![8])));
        assert!(!s.lane_needs_details());
        // Back on Comparisons the revalidate pass sees the new PR set as missing.
        s.inbox_tab = COMPARE_TAB;
        assert!(s.lane_needs_details());
        s.lane_details.insert(("a/b".into(), vec![8]), Load::start());
        assert!(!s.lane_needs_details());
    }

    #[test]
    fn branch_lane_refreshed_off_page_loads_on_return() {
        let mut s = branch_state();
        s.branch_lanes.insert(("a/b".into(), "feat".into(), "main".into()), branch_lane(branch_cell(3, 42)));
        s.branch_lane_sel = Some(("a/b".into(), "feat".into()));
        s.page = Page::Inbox;
        assert!(!s.lane_needs_details());
        s.page = Page::Branches;
        assert!(s.lane_needs_details());
    }

    fn branch_cell(commits: u32, pr: u64) -> compare::Cell {
        compare::Cell::Waiting {
            commits,
            prs: if commits == 0 { vec![] } else { vec![compare::WaitingPr { number: pr, title: "t".into(), author: "a".into(), merged_at: "2026-01-01T00:00:00Z".into() }] },
            loose: vec![],
            checked: commits,
        }
    }

    fn branch_lane(cell: compare::Cell) -> Load<compare::RepoCompare> {
        Load { data: Some(compare::RepoCompare { slug: "a/b".into(), cells: vec![cell], error: None }), ..Default::default() }
    }

    /// A repo "a/b" whose default branch is main, with its Branches page open.
    fn branch_state() -> State {
        let mut s = lane_state();
        s.cur = Some("a/b".into());
        s.rd_mut("a/b").meta = Load { data: Some(RepoMeta { default_branch: "main".into(), ..Default::default() }), ..Default::default() };
        s.page = Page::Branches;
        s
    }

    #[test]
    fn is_pushed_uses_origin_name() {
        let mut s = branch_state();
        let b = |name: &str, remote: bool| model::Branch { name: name.into(), remote, ..Default::default() };
        s.rd_mut("a/b").branches = Load { data: Some(vec![b("feat/login-v2", false), b("origin/feat/login-v2", true), b("wip", false)]), ..Default::default() };
        assert!(s.is_pushed("a/b", "feat/login-v2"));
        assert!(!s.is_pushed("a/b", "wip"));
        assert!(!s.is_pushed("other/repo", "feat/login-v2"));
    }

    #[test]
    fn create_pr_failure_reloads_statuses() {
        let mut s = branch_state();
        let spec = route::PrSpec { slug: "a/b".into(), head: "feat".into(), base: "main".into(), draft: false, title: String::new(), body: String::new(), reviewers: String::new(), assignees: String::new(), labels: String::new() };
        let key = ("a/b".to_string(), "feat".to_string());
        assert!(!s.route_prs.contains_key(&key));
        let action = guard::Action::CreatePr { spec };
        let entry = activity::Entry { at: 0, repo: "a/b".into(), action: action.label(), ok: false, detail: "a pull request already exists".into(), commands: vec![] };
        // The guard's own handling of a command that failed.
        s.action_finished(action.clone(), action.label(), None, false, (Err(entry.detail.clone()), entry, Ok(())));
        assert!(s.route_prs.contains_key(&key), "a failed CreatePr asks for the statuses again");
    }

    /// A repo "a/b" with `feat-a` pushed and with no PRs yet.
    fn dialog_state() -> State {
        let mut s = branch_state();
        let b = |name: &str, remote: bool| model::Branch { name: name.into(), remote, subject: "add login".into(), ..Default::default() };
        s.rd_mut("a/b").branches = Load { data: Some(vec![b("feat-a", false), b("origin/feat-a", true)]), ..Default::default() };
        s.route_prs.insert(("a/b".into(), "feat-a".into()), Load { data: Some(vec![]), ..Default::default() });
        s
    }

    #[test]
    fn open_pr_dialog_starts_from_the_last_commit() {
        let mut s = dialog_state();
        s.open_pr_dialog("a/b", "feat-a", "main");
        let d = s.pr_draft.as_ref().unwrap();
        assert_eq!((d.title.text.as_str(), d.body.text.as_str(), d.labels.text.as_str()), ("add login", "", ""));
        assert!(!d.draft);
        assert_eq!((d.slug.as_str(), d.head.as_str(), d.base.as_str()), ("a/b", "feat-a", "main"));
    }

    #[test]
    fn submit_requests_create_pr() {
        let mut s = dialog_state();
        s.open_pr_dialog("a/b", "feat-a", "main");
        s.submit_pr_draft();
        assert!(s.pr_draft.is_none());
        let Some(guard::Pending { action: guard::Action::CreatePr { spec }, .. }) = &s.pending else { panic!("no CreatePr pending") };
        assert_eq!((spec.base.as_str(), spec.head.as_str(), spec.draft), ("main", "feat-a", false));
    }

    #[test]
    fn edited_fields_reach_the_spec() {
        let mut s = dialog_state();
        s.open_pr_dialog("a/b", "feat-a", "main");
        s.pr_draft.as_mut().unwrap().title.text = "my own title".into();
        s.submit_pr_draft();
        let Some(guard::Pending { action: guard::Action::CreatePr { spec, .. }, .. }) = &s.pending else { panic!("no CreatePr pending") };
        assert_eq!(spec.title, "my own title");
    }

    /// Submit the open dialog and hand back the CreatePr action the guard holds.
    fn submitted(s: &mut State) -> guard::Action {
        s.submit_pr_draft();
        s.pending.as_ref().expect("no CreatePr pending").action.clone()
    }

    fn finish_with(s: &mut State, action: guard::Action, r: Result<String, String>) {
        let entry = activity::Entry { at: 0, repo: "a/b".into(), action: action.label(), ok: r.is_ok(), detail: r.clone().unwrap_or_else(|e| e), commands: vec![] };
        s.action_finished(action.clone(), action.label(), None, false, (r, entry, Ok(())));
    }

    #[test]
    fn failed_create_pr_reopens_the_dialog_with_the_edits() {
        let mut s = dialog_state();
        s.open_pr_dialog("a/b", "feat-a", "main");
        s.pr_draft.as_mut().unwrap().title.text = "my own title".into();
        let action = submitted(&mut s);
        assert!(s.pr_draft.is_none() && s.last_pr_draft.is_some());
        s.pending = None;
        finish_with(&mut s, action, Err("boom".into()));
        assert_eq!(s.pr_draft.as_ref().map(|d| d.title.text.as_str()), Some("my own title"));
        assert!(s.last_pr_draft.is_none());
    }

    #[test]
    fn cancelling_the_confirm_reopens_the_dialog_with_the_edits() {
        let mut s = dialog_state();
        s.open_pr_dialog("a/b", "feat-a", "main");
        s.pr_draft.as_mut().unwrap().title.text = "my own title".into();
        submitted(&mut s);
        s.cancel_pending();
        assert!(s.pending.is_none());
        assert_eq!(s.pr_draft.as_ref().map(|d| d.title.text.as_str()), Some("my own title"));
    }

    #[test]
    fn cancelling_another_action_leaves_the_dialog_alone() {
        let mut s = dialog_state();
        s.open_pr_dialog("a/b", "feat-a", "main");
        submitted(&mut s);
        s.pending = None;
        s.request(guard::Action::Close { slug: "a/b".into(), number: 1, title: String::new() });
        s.cancel_pending();
        assert!(s.pr_draft.is_none(), "only a cancelled CreatePr gives the draft back");
    }

    #[test]
    fn created_pr_clears_the_kept_draft() {
        let mut s = dialog_state();
        s.open_pr_dialog("a/b", "feat-a", "main");
        let action = submitted(&mut s);
        s.pending = None;
        finish_with(&mut s, action, Ok("https://github.com/a/b/pull/9".into()));
        assert!(s.last_pr_draft.is_none());
        assert!(s.pr_draft.is_none());
    }

    #[test]
    fn submit_refuses_a_blank_title() {
        let mut s = dialog_state();
        s.open_pr_dialog("a/b", "feat-a", "main");
        s.pr_draft.as_mut().unwrap().title.text = "   ".into();
        s.submit_pr_draft();
        assert!(s.pending.is_none() && s.pr_draft.is_some());
    }

    #[test]
    fn merge_drops_the_repos_cached_statuses() {
        let mut s = dialog_state();
        s.route_prs.insert(("a/b".into(), "other".into()), Load { data: Some(vec![]), ..Default::default() });
        s.route_prs.insert(("c/d".into(), "other".into()), Load { data: Some(vec![]), ..Default::default() });
        s.branch_lane_sel = Some(("a/b".into(), "feat-a".into()));
        let action = guard::Action::Merge { slug: "a/b".into(), number: 1, title: String::new(), method: gh::MergeMethod::Squash, delete_branch: false };
        finish_with(&mut s, action, Ok("merged".into()));
        assert!(!s.route_prs.contains_key(&("a/b".to_string(), "other".to_string())));
        assert!(s.route_prs.contains_key(&("c/d".to_string(), "other".to_string())), "other repos keep theirs");
        let picked = s.route_prs.get(&("a/b".to_string(), "feat-a".to_string())).expect("the picked branch is fetched again");
        // Tests run jobs inline and the lookup is offline: a fetch leaves "offline" behind.
        assert!(picked.data.is_none() && picked.error.as_deref() == Some("offline"));
    }

    #[test]
    fn submit_refuses_a_known_unpushed_head() {
        let mut s = dialog_state();
        let b = |name: &str, remote: bool| model::Branch { name: name.into(), remote, ..Default::default() };
        s.rd_mut("a/b").branches = Load { data: Some(vec![b("feat-a", false)]), ..Default::default() };
        s.open_pr_dialog("a/b", "feat-a", "main");
        s.submit_pr_draft();
        assert!(s.pending.is_none());
        assert!(s.pr_draft.is_some());
    }

    #[test]
    fn submit_allows_head_when_branches_not_loaded() {
        let mut s = dialog_state();
        s.rd_mut("a/b").branches = Load::default();
        s.open_pr_dialog("a/b", "feat-a", "main");
        s.pr_draft.as_mut().unwrap().title.text = "add login".into(); // no branch list, so no subject to start from
        s.submit_pr_draft();
        assert!(s.pr_draft.is_none());
        assert!(matches!(s.pending, Some(guard::Pending { action: guard::Action::CreatePr { .. }, .. })));
    }

    #[test]
    fn pushed_state_is_three_way() {
        let mut s = dialog_state();
        assert_eq!(s.pushed_state("a/b", "feat-a"), Some(true));
        assert_eq!(s.pushed_state("a/b", "other"), Some(false));
        s.rd_mut("a/b").branches = Load::default();
        assert_eq!(s.pushed_state("a/b", "feat-a"), None);
    }

    #[test]
    fn open_lane_follows_the_page() {
        let mut s = branch_state();
        s.prefs.comparisons = vec![compare::Comparison { name: "To staging".into(), from: "dev".into(), to: "staging".into() }];
        s.compare_sel = Some(("a/b".into(), 0));
        s.branch_lanes.insert(("a/b".into(), "feat".into(), "main".into()), branch_lane(branch_cell(3, 42)));
        s.branch_lane_sel = Some(("a/b".into(), "feat".into()));

        s.page = Page::Inbox;
        s.inbox_tab = COMPARE_TAB;
        let lane = s.open_lane().unwrap();
        assert_eq!((lane.slug, lane.name.as_str(), lane.from.as_str(), lane.to.as_str()), ("a/b", "To staging", "dev", "staging"));
        assert_eq!(s.lane_key(), Some(("a/b".to_string(), vec![7])));
        assert_eq!(s.lane_branch(), Some(("a/b".to_string(), "dev".to_string())));

        s.page = Page::Branches;
        let lane = s.open_lane().unwrap();
        assert_eq!((lane.slug, lane.name.as_str(), lane.from.as_str(), lane.to.as_str()), ("a/b", "feat", "feat", "main"));
        assert_eq!(s.lane_key(), Some(("a/b".to_string(), vec![42])));
        assert_eq!(s.lane_branch(), Some(("a/b".to_string(), "feat".to_string())));

        // Stashes and clean up show no lane; nor does another Dashboard tab.
        s.branch_view = 2;
        assert!(s.open_lane().is_none());
        s.page = Page::Inbox;
        s.inbox_tab = 0;
        assert!(s.open_lane().is_none());
    }

    #[test]
    fn branch_lane_keyed_by_base() {
        let mut s = branch_state();
        s.branch_lanes.insert(("a/b".into(), "feat".into(), "main".into()), branch_lane(branch_cell(3, 42)));
        s.branch_lane_sel = Some(("a/b".into(), "feat".into()));
        // Main's lane details and layout are already on the way: going back loads nothing.
        s.lane_details.insert(("a/b".into(), vec![42]), Load::start());
        s.layouts.insert(("a/b".into(), "feat".into()), Load::start());
        assert_eq!(s.branch_base_for("a/b").as_deref(), Some("main"));
        s.branch_lanes.insert(("a/b".into(), "feat".into(), "dev".into()), Load::start());
        s.set_branch_base(Some("dev".into()));
        assert_eq!(s.branch_base_for("a/b").as_deref(), Some("dev"));
        assert!(s.branch_lanes[&("a/b".to_string(), "feat".to_string(), "dev".to_string())].loading);
        // Dev's lane is still loading: no cell, and never main's.
        assert!(s.open_lane().is_none());
        s.set_branch_base(None);
        assert_eq!(s.open_lane().unwrap().to, "main");
    }

    #[test]
    fn branch_lane_up_to_date() {
        let mut s = branch_state();
        s.branch_lanes.insert(("a/b".into(), "feat".into(), "main".into()), branch_lane(branch_cell(0, 0)));
        s.branch_lane_sel = Some(("a/b".into(), "feat".into()));
        let lane = s.open_lane().unwrap();
        assert_eq!(lane.cell, &branch_cell(0, 0));
    }

    #[test]
    fn remote_branch_name_drops_origin() {
        let mut s = branch_state();
        s.branch_view = 1;
        s.lane_view.folder = Some(("src".into(), false));
        s.branch_lanes.insert(("a/b".into(), "feat/x".into(), "main".into()), Load::start());
        s.select_branch_lane("origin/feat/x");
        assert_eq!(s.branch_lane_sel, Some(("a/b".into(), "feat/x".into())));
        assert_eq!(s.lane_view.folder, None);
        assert_eq!(s.branch_lanes.len(), 1);
    }

    #[test]
    fn base_or_head_is_never_fetched() {
        assert!(State::branch_lane_skipped("main", "main"));
        assert!(State::branch_lane_skipped("HEAD", "main"));
        assert!(!State::branch_lane_skipped("feat", "main"));
        let mut s = branch_state();
        s.select_branch_lane("main");
        assert_eq!(s.branch_lane_sel, Some(("a/b".into(), "main".into())));
        assert!(s.branch_lanes.is_empty(), "comparing main with itself asks for nothing");
    }

    #[test]
    fn closing_the_branch_pane_clears_it() {
        let mut s = branch_state();
        s.branch_lane_sel = Some(("a/b".into(), "feat".into()));
        s.branch_sel = Some("feat".into());
        s.branch_base_open = true;
        s.close_branch_lane();
        assert!(s.branch_lane_sel.is_none() && s.branch_sel.is_none() && !s.branch_base_open);
    }

    #[test]
    fn refresh_forgets_only_the_open_branch_lane() {
        let mut s = branch_state();
        assert!(!s.forget_branch_lane(), "no branch open: nothing to refresh");
        let main = ("a/b".to_string(), "feat".to_string(), "main".to_string());
        let dev = ("a/b".to_string(), "feat".to_string(), "dev".to_string());
        s.branch_lanes.insert(main.clone(), branch_lane(compare::Cell::Missing(vec!["feat".into()])));
        s.branch_lanes.insert(dev.clone(), branch_lane(branch_cell(3, 42)));
        s.branch_lane_sel = Some(("a/b".into(), "feat".into()));
        assert!(s.forget_branch_lane());
        assert!(!s.branch_lanes.contains_key(&main), "the next load asks GitHub again");
        assert!(s.branch_lanes.contains_key(&dev), "other bases stay");
        // No default branch known yet: nothing to forget.
        s.rd_mut("a/b").meta = Load::default();
        assert!(!s.forget_branch_lane());
    }

    /// Nothing on "c/d"'s Environments page is stale, so opening it fetches nothing.
    fn quiet_repo(s: &mut State, slug: &str) {
        let d = s.rd_mut(slug);
        d.meta = Load::start();
        d.envs = Load::start();
    }

    #[test]
    fn switching_repo_drops_the_picked_base() {
        let mut s = branch_state();
        s.branch_base = Some("dev".into());
        s.branch_lane_sel = Some(("a/b".into(), "feat".into()));
        quiet_repo(&mut s, "c/d");
        s.select_repo("c/d", Page::Envs);
        assert_eq!(s.branch_base, None);
        assert!(s.branch_lane_sel.is_none());
    }

    #[test]
    fn leaving_the_page_closes_the_base_list() {
        let mut s = branch_state();
        quiet_repo(&mut s, "a/b");
        s.branch_base_open = true;
        s.show(Page::Envs);
        assert!(!s.branch_base_open);
    }

    /// Lane "a/b" [42] with a model, and a commit in a/b whose files touch two of its folders.
    fn commit_state() -> (State, (String, Vec<u64>)) {
        let mut s = branch_state();
        let key = ("a/b".to_string(), vec![42]);
        let mut d = compare::PrDetail { number: 42, files: vec![("src/agent/x.ts".into(), 9, 0), ("docs/y.md".into(), 1, 0)], ..Default::default() };
        d.author.login = "a".into();
        s.lane_details.insert(key.clone(), Load { data: Some(vec![d]), ..Default::default() });
        (s, key)
    }

    #[test]
    fn commit_segments_follow_the_lane_model() {
        let (mut s, key) = commit_state();
        s.rebuild_lane_model(&key);
        let files = vec![("docs/z.md".to_string(), 2, 0), ("lib/q.rs".to_string(), 2, 0)];
        s.commit_files.insert(("a/b".into(), "abc".into()), Load::start());
        s.commit_files_arrived(&("a/b".into(), "abc".into()), Ok(files.clone()));
        let segs = &s.commit_segments[&("a/b".to_string(), vec![42])]["abc"];
        let docs = segs.iter().find(|x| x.key == "docs").unwrap();
        assert_eq!(Some(&docs.color), s.lane_models[&key].colors.get("docs"), "same colour as the lane's main bar");
        assert!(segs.iter().any(|x| x.color == lane_model::OTHER));
        // A commit loaded before the lane's model is worked out gets its bar when the model comes.
        let (mut s, key) = commit_state();
        s.commit_files.insert(("a/b".into(), "abc".into()), Load { data: Some(files), ..Default::default() });
        s.rebuild_lane_model(&key);
        assert!(s.commit_segments.get(&("a/b".to_string(), vec![42])).is_some_and(|m| m.contains_key("abc")));
    }

    #[test]
    fn commit_files_error_keeps_no_segments() {
        let (mut s, key) = commit_state();
        s.rebuild_lane_model(&key);
        s.commit_files.insert(("a/b".into(), "abc".into()), Load::start());
        s.commit_files_arrived(&("a/b".into(), "abc".into()), Err("boom".into()));
        assert!(s.commit_segments.is_empty());
        assert_eq!(s.commit_files[&("a/b".to_string(), "abc".to_string())].error.as_deref(), Some("boom"));
    }

    fn cmp_rows(s: &mut State, rows: &[(&str, &str, &str)]) {
        s.cmp_rows = rows.iter().map(|(n, f, t)| (TextEdit::new(*n), TextEdit::new(*f), TextEdit::new(*t))).collect();
    }

    #[test]
    fn save_comparisons_rejects_duplicates_and_keeps_overrides_on_rename() {
        let mut s = State::new(config::Prefs::default());
        s.open_cmp_rows();
        let before = s.prefs.comparisons.clone();
        cmp_rows(&mut s, &[("A", "dev", "staging"), ("A", "staging", "main")]);
        assert_eq!(s.save_comparisons(), Err("Two comparisons are called A".to_owned()));
        assert_eq!(s.prefs.comparisons, before);
        cmp_rows(&mut s, &[("A", "dev", "")]);
        assert_eq!(s.save_comparisons(), Err("Every comparison needs a name and two branches".to_owned()));
        cmp_rows(&mut s, &[("A", "dev", "dev")]);
        assert_eq!(s.save_comparisons(), Err("A compares dev with itself".to_owned()));
        assert_eq!(s.prefs.comparisons, before);

        s.prefs.comparison_overrides = vec![compare::Override { slug: "a/b".into(), name: "Back-merge".into(), off: true, ..Default::default() }];
        s.open_cmp_rows();
        s.cmp_rows[1].0 = TextEdit::new("Hotfix back");
        assert_eq!(s.save_comparisons(), Ok(()));
        assert_eq!(s.prefs.comparisons[1].name, "Hotfix back");
        assert_eq!(s.prefs.comparison_overrides[0].name, "Hotfix back");
    }

    #[test]
    fn save_comparisons_handles_swapped_names_removal_and_moves() {
        let mut s = State::new(config::Prefs::default());
        s.prefs.comparison_overrides = vec![
            compare::Override { slug: "a/b".into(), name: "To staging".into(), off: true, ..Default::default() },
            compare::Override { slug: "a/b".into(), name: "To production".into(), off: true, ..Default::default() },
        ];
        s.open_cmp_rows();
        s.cmp_rows[0].0 = TextEdit::new("To production"); // names trade places
        s.cmp_rows[2].0 = TextEdit::new("To staging");
        s.cmp_remove(1);
        assert_eq!(s.save_comparisons(), Ok(()));
        let names: Vec<_> = s.prefs.comparisons.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["To production", "To staging"]);
        let offs: Vec<_> = s.prefs.comparison_overrides.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(offs, vec!["To production", "To staging"], "overrides trade names with their comparisons");
        assert_eq!(s.prefs.comparisons[0].from, "dev");

        s.open_cmp_rows();
        s.cmp_move(1, -1);
        assert_eq!(s.save_comparisons(), Ok(()));
        assert_eq!(s.prefs.comparisons[0].name, "To staging", "a move is not a rename");
        assert_eq!(s.prefs.comparison_overrides.len(), 2);
        s.cmp_swap(0);
        assert_eq!((s.cmp_rows[0].1.text.as_str(), s.cmp_rows[0].2.text.as_str()), ("main", "staging"));
    }

    #[test]
    fn snapshot_round_trips_compare() {
        let rc = compare::RepoCompare {
            slug: "a/b".into(),
            cells: vec![compare::Cell::Waiting { commits: 2, prs: vec![], loose: vec![], checked: 2 }],
            error: None,
        };
        let snap = cache::Snapshot { compare: Some(("k".into(), vec![rc.clone()])), compare_at: 9, ..Default::default() };
        let back: cache::Snapshot = serde_json::from_str(&serde_json::to_string(&snap).unwrap()).unwrap();
        assert_eq!(back.compare, Some(("k".into(), vec![rc])));
        assert_eq!(back.compare_at, 9);
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

    fn found(slug: &str, path: &str) -> scan::Found {
        scan::Found { slug: slug.into(), path: PathBuf::from(path), depth: 1 }
    }

    #[test]
    fn add_clones_keeps_known_paths_and_counts() {
        let mut s = State::new(config::Prefs::default());
        s.add_repo("a/b".into(), Some(PathBuf::from("P1")));
        s.add_repo("x/y".into(), None);
        let scans = vec![
            scan::Scan { found: vec![found("a/b", "P2"), found("c/d", "P3")], skipped: 0, stopped_early: false },
            scan::Scan { found: vec![found("c/d", "P4")], skipped: 1, stopped_early: false },
        ];
        assert_eq!(s.add_clones(scans), (1, 2), "c/d is new once; a/b and the repeat of c/d were known");
        let path = |slug: &str| s.repos.iter().find(|r| r.slug == slug).unwrap().path.clone();
        assert_eq!(path("a/b"), Some(PathBuf::from("P1")), "a saved path is kept");
        assert_eq!(path("c/d"), Some(PathBuf::from("P3")));
        let order: Vec<&str> = s.repos.iter().map(|r| r.slug.as_str()).collect();
        assert_eq!(order, ["a/b", "c/d", "x/y"], "local clones sort before repos without a path");
    }

    #[test]
    fn add_clones_gives_a_pathless_repo_its_clone() {
        let mut s = State::new(config::Prefs::default());
        s.add_repo("x/y".into(), None);
        let scans = vec![scan::Scan { found: vec![found("x/y", "P9")], skipped: 0, stopped_early: false }];
        assert_eq!(s.add_clones(scans), (1, 0));
        assert_eq!(s.repos[0].path, Some(PathBuf::from("P9")));
    }

    #[test]
    fn remove_clone_folder_only_drops_the_folder() {
        let mut s = State::new(config::Prefs::default());
        s.prefs.clone_folders = vec![PathBuf::from("A"), PathBuf::from("B")];
        s.add_repo("a/b".into(), Some(PathBuf::from("A/b")));
        s.remove_clone_folder(0);
        s.remove_clone_folder(7);
        assert_eq!(s.prefs.clone_folders, vec![PathBuf::from("B")]);
        assert_eq!(s.repos.len(), 1);
    }

    /// Reads the real disk: run by hand with `-- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_scan() {
        let r = scan::scan(std::path::Path::new("C:/Users/seaga/Desktop/Project"));
        println!("found={} skipped={} stopped_early={}", r.found.len(), r.skipped, r.stopped_early);
        for f in &r.found {
            println!("{} {} depth {}", f.slug, f.path.display(), f.depth);
        }
    }
}
