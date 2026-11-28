//! Typed rows for what `gh` and `git` print, plus time helpers. Field names follow gh's JSON.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------------------------
// GitHub
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Actor {
    #[serde(default)]
    pub login: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub color: String,
}

/// One entry of a PR's `statusCheckRollup`: a CheckRun or a commit StatusContext.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    #[serde(default)]
    pub name: String,
    /// StatusContext calls it `context`.
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub workflow_name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    /// StatusContext's single field for both.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub details_url: String,
    #[serde(default)]
    pub target_url: String,
}

impl Check {
    pub fn title(&self) -> &str {
        if self.name.is_empty() { &self.context } else { &self.name }
    }
    pub fn outcome(&self) -> Outcome {
        let s = if self.state.is_empty() { self.status.as_str() } else { self.state.as_str() };
        Outcome::from_parts(s, &self.conclusion)
    }
    pub fn url(&self) -> &str {
        if self.details_url.is_empty() { &self.target_url } else { &self.details_url }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub review_decision: String,
    #[serde(default)]
    pub status_check_rollup: Vec<Check>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub changed_files: u64,
    #[serde(default)]
    pub mergeable: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub labels: Vec<Label>,
    /// Changed files, for the sensitive-change chips (A3).
    #[serde(default)]
    pub files: Vec<PrFile>,
}

impl Pr {
    /// The checks folded into one outcome: any failure wins, then anything running.
    pub fn checks(&self) -> (Outcome, usize, usize) {
        let total = self.status_check_rollup.len();
        let passed = self.status_check_rollup.iter().filter(|c| c.outcome() == Outcome::Success).count();
        (Outcome::fold(self.status_check_rollup.iter().map(Check::outcome)), passed, total)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub submitted_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PrFile {
    pub path: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrView {
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub merge_state_status: String,
    #[serde(default)]
    pub reviews: Vec<Review>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub files: Vec<PrFile>,
    #[serde(default)]
    pub status_check_rollup: Vec<Check>,
}

pub struct PrDetail {
    pub view: PrView,
    pub diff: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub database_id: u64,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub workflow_name: String,
    #[serde(default)]
    pub head_branch: String,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub number: u64,
}

impl Run {
    pub fn outcome(&self) -> Outcome {
        Outcome::from_parts(&self.status, &self.conclusion)
    }
    /// Seconds from start to last update (or to now while running).
    pub fn duration(&self, now: i64) -> i64 {
        let start = parse_time(if self.started_at.is_empty() { &self.created_at } else { &self.started_at });
        let end = if self.outcome().is_running() { now } else { parse_time(&self.updated_at).unwrap_or(0) };
        match (start, end) {
            (Some(a), b) if b > 0 => (b - a).max(0),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub completed_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub completed_at: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RunView {
    #[serde(default)]
    pub jobs: Vec<Job>,
}

/// One deployment, flattened from the GraphQL answer.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Deployment {
    pub environment: String,
    pub sha: String,
    pub git_ref: String,
    pub creator: String,
    pub created_at: i64,
    pub state: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Envs {
    pub names: Vec<String>,
    /// Environments with deployment protection (required reviewers or a wait timer).
    pub protected: Vec<String>,
    /// Newest first.
    pub deployments: Vec<Deployment>,
}

/// A search hit for the inbox (PR or issue in any repo).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub author: Actor,
    #[serde(default)]
    pub repository: HitRepo,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub is_draft: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitRepo {
    #[serde(default)]
    pub name_with_owner: String,
}

#[derive(Default, Clone, Deserialize, Serialize)]
pub struct Inbox {
    pub review_requested: Vec<Hit>,
    pub my_prs: Vec<Hit>,
    pub issues: Vec<Hit>,
    /// (repo slug, run) for the latest runs of your recently pushed repos.
    pub runs: Vec<(String, Run)>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRepo {
    pub name_with_owner: String,
    #[serde(default)]
    pub pushed_at: String,
    #[serde(default)]
    pub is_archived: bool,
}

/// A `gh search repos` hit: any repository on GitHub you can see.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRepo {
    pub full_name: String,
    #[serde(default)]
    pub pushed_at: String,
    #[serde(default)]
    pub description: String,
}

// ---------------------------------------------------------------------------------------------
// Local git
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub time: i64,
    pub refs: Vec<String>,
    pub subject: String,
}

#[derive(Clone, Debug, Default)]
pub struct CommitDetail {
    /// Whether the commit is on the current branch (HEAD has it), and whether it is HEAD: from
    /// git, so the Edit history buttons are right whatever the Commits page has loaded.
    pub on_head: Option<bool>,
    pub is_head: bool,
    pub body: String,
    pub files: Vec<(String, u64, u64)>, // path, added, removed
    pub diff: String,
}

#[derive(Clone, Debug, Default)]
pub struct Branch {
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub upstream: String,
    pub ahead: u32,
    pub behind: u32,
    /// Its upstream was deleted (`[gone]`), typically after the PR merged.
    pub gone: bool,
    pub time: i64,
    pub author: String,
    pub subject: String,
}

#[derive(Clone, Debug, Default)]
pub struct LocalStatus {
    pub branch: String,
    pub changed: usize,
    /// Files with staged changes (what Amend would put into the last commit).
    pub staged: usize,
    pub ahead: u32,
    pub behind: u32,
}

/// What the guard rails need to know about a repo: its default branch, which branches are
/// protected, and whether you're an admin (who can bypass protection).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RepoMeta {
    pub default_branch: String,
    pub protected: Vec<String>,
    pub admin: bool,
}

/// A GitHub notification (B1).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Notif {
    pub id: String,
    pub reason: String,
    pub unread: bool,
    pub updated_at: String,
    pub title: String,
    /// "PullRequest", "Issue", "CheckSuite", "Release", ...
    pub kind: String,
    pub repo: String,
    /// The PR or issue number, when the subject has one.
    pub number: Option<u64>,
}

/// Extra facts for a PR in the review queue (B2), from one batched GraphQL query.
#[derive(Clone, Debug, Default)]
pub struct QueueInfo {
    /// When your review was requested (Unix seconds); falls back to the PR's creation.
    pub requested_at: i64,
    pub additions: u64,
    pub deletions: u64,
    /// The head commit's combined check state: SUCCESS, FAILURE, PENDING, ...
    pub ci: String,
    /// For your own PRs: who you're waiting on.
    pub waiting_on: Vec<String>,
}

/// A stash entry (A5).
#[derive(Clone, Debug, Default)]
pub struct Stash {
    /// `stash@{0}`
    pub name: String,
    pub message: String,
    pub time: i64,
}

/// A local branch that could go (B7).
#[derive(Clone, Debug, Default)]
pub struct Cleanup {
    pub name: String,
    pub sha: String,
    pub merged: bool,
    pub gone: bool,
    pub stale_days: Option<i64>,
    /// Commits on it that no remote has: deleting loses them.
    pub unpushed: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Outcomes and time
// ---------------------------------------------------------------------------------------------

/// A PR's checks (Actions and other status checks) on its latest commit, rolled up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ci {
    Passed,
    Running,
    Failing,
    /// No checks ran on it.
    None,
}

impl Ci {
    /// From GitHub's `statusCheckRollup.state`.
    pub fn from_rollup(state: &str) -> Ci {
        match state {
            "SUCCESS" => Ci::Passed,
            "FAILURE" | "ERROR" => Ci::Failing,
            "PENDING" | "EXPECTED" => Ci::Running,
            _ => Ci::None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Ci::Passed => "✓ checks passed",
            Ci::Running => "● checks running",
            Ci::Failing => "✗ checks failing",
            Ci::None => "no checks",
        }
    }
    pub fn outcome(self) -> Option<Outcome> {
        match self {
            Ci::Passed => Some(Outcome::Success),
            Ci::Running => Some(Outcome::Running),
            Ci::Failing => Some(Outcome::Failure),
            Ci::None => None,
        }
    }
    /// Worth a review: every check passed. No checks at all isn't a pass (Actions may not
    /// have run yet, e.g. waiting for approval on a first-time contributor's PR).
    pub fn reviewable(self) -> bool {
        self == Ci::Passed
    }
}

/// The four colours every status in the app maps onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Outcome {
    Failure,
    Running,
    Success,
    Neutral,
}

impl Outcome {
    pub fn from_parts(status: &str, conclusion: &str) -> Outcome {
        let s = status.to_ascii_lowercase();
        let c = conclusion.to_ascii_lowercase();
        match (s.as_str(), c.as_str()) {
            ("in_progress" | "queued" | "pending" | "waiting" | "requested" | "expected", _) => Outcome::Running,
            (_, "success") | ("success", _) => Outcome::Success,
            (_, "failure" | "timed_out" | "startup_failure" | "action_required") | ("failure" | "error", _) => Outcome::Failure,
            _ => Outcome::Neutral,
        }
    }
    /// Worst first: a failure anywhere fails the set.
    pub fn fold(items: impl Iterator<Item = Outcome>) -> Outcome {
        items.min().unwrap_or(Outcome::Neutral)
    }
    pub fn is_running(self) -> bool {
        self == Outcome::Running
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// `2026-11-14T08:36:07Z` → Unix seconds. GitHub always answers in UTC with a `Z`.
pub fn parse_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from civil (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3_600 + mi * 60 + se)
}

/// "just now", "5m", "3h", "2d", "4w", "6mo", "2y".
pub fn ago(then: i64, now: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..60 => crate::i18n::t("just now"),
        60..3_600 => tf!("{}m ago", s / 60),
        3_600..86_400 => tf!("{}h ago", s / 3_600),
        86_400..604_800 => tf!("{}d ago", s / 86_400),
        604_800..2_592_000 => tf!("{}w ago", s / 604_800),
        2_592_000..31_536_000 => tf!("{}mo ago", s / 2_592_000),
        _ => tf!("{}y ago", s / 31_536_000),
    }
}

pub fn ago_str(iso: &str, now: i64) -> String {
    parse_time(iso).map_or_else(String::new, |t| ago(t, now))
}

/// "42s", "3m 05s", "1h 02m".
pub fn duration(secs: i64) -> String {
    match secs {
        s if s < 60 => tf!("{s}s"),
        s if s < 3_600 => tf!("{}m {:02}s", s / 60, s % 60),
        s => tf!("{}h {:02}m", s / 3_600, (s % 3_600) / 60),
    }
}

/// Unix seconds → "2026-11-14" (UTC), for date filters in `gh` and `git` queries.
pub fn ymd(t: i64) -> String {
    // Civil from days (Howard Hinnant's algorithm), the inverse of `parse_time`'s.
    let z = t.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The date ranges the filters offer, in days (0: any time).
pub const FILTER_DAYS: [u32; 4] = [0, 7, 30, 90];

/// What pull requests, runs and commits are narrowed to: a date range and a contributor
/// (sidebar 2's filters, shared by every section). Applied inside the `gh` / `git log` query,
/// so it reaches past the first page of results.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    /// Created in the last this many days (0: any time).
    pub days: u32,
    /// GitHub login (PRs, runs) or git author name / email (commits). Empty: anyone.
    pub who: String,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        self.days == 0 && self.who.is_empty()
    }

    /// The first day in range, as `gh` and `git` take it.
    pub fn since(&self, now: i64) -> Option<String> {
        (self.days > 0).then(|| ymd(now - i64::from(self.days) * 86_400))
    }

    /// For a list loaded without the filter (a group's open PRs): whether this item passes.
    /// `who` is a login, matched exactly (as GitHub matches `--author`).
    pub fn keeps(&self, who: &str, when: &str, now: i64) -> bool {
        (self.who.is_empty() || who.eq_ignore_ascii_case(&self.who))
            && (self.days == 0 || parse_time(when).is_some_and(|t| t >= now - i64::from(self.days) * 86_400))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_days() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(parse_time("2026-11-14T08:36:07Z").unwrap()), "2026-11-14");
        assert_eq!(ymd(parse_time("2024-02-29T23:59:59Z").unwrap()), "2024-02-29");
        assert_eq!(ymd(parse_time("2000-03-01T00:00:00Z").unwrap()), "2000-03-01");
    }

    #[test]
    fn filter_since_and_keeps() {
        let now = parse_time("2026-11-23T12:00:00Z").unwrap();
        assert_eq!(Filter::default().since(now), None);
        let f = Filter { days: 7, who: "OctoCat".into() };
        assert_eq!(f.since(now).as_deref(), Some("2026-11-16"));
        assert!(f.keeps("octocat", "2026-11-20T00:00:00Z", now));
        assert!(!f.keeps("octocat", "2026-11-01T00:00:00Z", now));
        assert!(!f.keeps("someone", "2026-11-20T00:00:00Z", now));
        assert!(!f.keeps("octocat2", "2026-11-20T00:00:00Z", now));
        assert!(Filter::default().keeps("anyone", "", now));
    }

    #[test]
    fn parses_github_times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-11-14T08:36:07Z"), Some(1_794_645_367));
        assert_eq!(parse_time("bad"), None);
    }

    #[test]
    fn outcomes() {
        assert_eq!(Outcome::from_parts("completed", "success"), Outcome::Success);
        assert_eq!(Outcome::from_parts("COMPLETED", "FAILURE"), Outcome::Failure);
        assert_eq!(Outcome::from_parts("in_progress", ""), Outcome::Running);
        assert_eq!(Outcome::from_parts("completed", "skipped"), Outcome::Neutral);
        assert_eq!(Outcome::from_parts("SUCCESS", ""), Outcome::Success); // StatusContext state
        assert_eq!(Outcome::fold([Outcome::Success, Outcome::Failure, Outcome::Running].into_iter()), Outcome::Failure);
    }

    #[test]
    fn formats_ages_and_durations() {
        assert_eq!(ago(0, 30), "just now");
        assert_eq!(ago(0, 7_200), "2h ago");
        assert_eq!(duration(185), "3m 05s");
    }
}
