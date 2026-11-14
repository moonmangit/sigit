//! The Activity page's data: what happened in your tracked repos over a time range. PRs opened,
//! merged and closed; commits and lines changed on default branches; Actions runs, success and
//! CI minutes; deployments and releases; and who contributed what.
//!
//! Exact totals come from GitHub's counts. Details (lines, contributors, per-day bars, minutes)
//! come from up to `CAP` items per repo; `capped` says when that cut something.
//!
//! Requests: PR searches cover many repos per query; commits, deployments and releases come from
//! GraphQL batched several repos per query; Actions runs are fetched per repo in parallel.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};

use crate::gh::run;
use crate::model::{Outcome, parse_time};

/// Items per repo read in detail (commits, runs) and merged PRs overall per search.
pub const CAP: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Today,
    Week,
    Last7,
    Month,
    Last30,
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 6] = [Preset::Today, Preset::Week, Preset::Last7, Preset::Month, Preset::Last30, Preset::Custom];
    pub fn label(self) -> &'static str {
        match self {
            Preset::Today => "Today",
            Preset::Week => "This week",
            Preset::Last7 => "7 days",
            Preset::Month => "This month",
            Preset::Last30 => "30 days",
            Preset::Custom => "Custom",
        }
    }
}

/// A time range: [since, until) in Unix seconds, and its local days for per-day bars.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Range {
    pub since: i64,
    pub until: i64,
    pub label: String,
    /// (start of the local day, "Mon 9")
    pub days: Vec<(i64, String)>,
}

fn midnight<Tz: TimeZone>(t: &DateTime<Tz>) -> DateTime<Tz> {
    t.with_hour(0).and_then(|t| t.with_minute(0)).and_then(|t| t.with_second(0)).and_then(|t| t.with_nanosecond(0)).unwrap_or_else(|| t.clone())
}

/// The range for a preset, in `now`'s time zone. `custom`: (from, to) inclusive local dates.
pub fn range<Tz: TimeZone>(preset: Preset, now: DateTime<Tz>, custom: Option<(NaiveDate, NaiveDate)>) -> Range
where
    Tz::Offset: std::fmt::Display,
{
    let today = midnight(&now);
    let (start, end, label) = match preset {
        Preset::Today => (today.clone(), now.clone(), "Today".to_owned()),
        Preset::Week => {
            let back = now.weekday().num_days_from_monday() as i64;
            (today.clone() - Duration::days(back), now.clone(), "This week (since Monday)".to_owned())
        }
        Preset::Last7 => (today.clone() - Duration::days(6), now.clone(), "Last 7 days".to_owned()),
        Preset::Month => (today.clone() - Duration::days(now.day0() as i64), now.clone(), "This month".to_owned()),
        Preset::Last30 => (today.clone() - Duration::days(29), now.clone(), "Last 30 days".to_owned()),
        Preset::Custom => {
            let (a, b) = custom.unwrap_or((today.date_naive(), today.date_naive()));
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            let tz = now.timezone();
            let at = |d: NaiveDate| tz.from_local_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight exists")).earliest();
            match (at(a), at(b + Duration::days(1))) {
                (Some(s), Some(e)) => (s, e.min(now.clone()), format!("{} – {}", a.format("%d %b"), b.format("%d %b %Y"))),
                _ => (today.clone(), now.clone(), "Today".to_owned()),
            }
        }
    };
    let mut days = Vec::new();
    let mut d = midnight(&start);
    while d < end {
        days.push((d.timestamp(), d.format("%a %-d").to_string()));
        d = midnight(&(d + Duration::hours(26)));
    }
    Range { since: start.timestamp(), until: end.timestamp(), label, days }
}

pub fn local_range(preset: Preset, custom: Option<(NaiveDate, NaiveDate)>) -> Range {
    range(preset, Local::now(), custom)
}

/// `2026-11-09T00:00:00Z`: GitHub's search and REST filters, in UTC so no `+` needs escaping.
fn iso(t: i64) -> String {
    DateTime::from_timestamp(t, 0).map_or_else(String::new, |d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

// ---------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PrStat {
    pub slug: String,
    pub number: u64,
    pub title: String,
    pub author: String,
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    pub created: i64,
    pub merged: Option<i64>,
}

/// A PR open right now.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OpenPr {
    pub slug: String,
    pub number: u64,
    pub title: String,
    pub author: String,
    pub created: i64,
    pub updated: i64,
    pub draft: bool,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED or empty.
    pub review: String,
    /// The head commit's checks: SUCCESS, FAILURE, ERROR, PENDING or empty.
    pub ci: String,
}

impl OpenPr {
    pub fn ci(&self) -> crate::model::Ci {
        crate::model::Ci::from_rollup(&self.ci)
    }
    pub fn failing(&self) -> bool {
        self.ci() == crate::model::Ci::Failing
    }
    /// Waiting on a reviewer and worth reviewing: all checks passed (or none ran).
    pub fn ready_for_review(&self) -> bool {
        self.needs_review() && self.ci().reviewable()
    }
    /// Waiting on a reviewer: not a draft, not approved, no changes requested.
    pub fn needs_review(&self) -> bool {
        !self.draft && self.review != "APPROVED" && self.review != "CHANGES_REQUESTED"
    }
    /// Approved with green (or no) checks: could merge now.
    pub fn ready(&self) -> bool {
        !self.draft && self.review == "APPROVED" && !self.failing() && self.ci != "PENDING"
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CommitStat {
    pub author: String,
    pub additions: u64,
    pub deletions: u64,
    pub at: i64,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RunStat {
    pub workflow: String,
    pub branch: String,
    pub outcome: Option<Outcome>,
    pub secs: i64,
    pub at: i64,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RepoPulse {
    pub slug: String,
    pub default_branch: String,
    pub commits_total: u64,
    pub commits: Vec<CommitStat>,
    pub runs_total: u64,
    /// Exact passed / failed run counts in the range (GitHub's totals, not the sample).
    pub runs_ok_total: u64,
    pub runs_failed_total: u64,
    pub runs: Vec<RunStat>,
    /// (environment, state, at)
    pub deploys: Vec<(String, String, i64)>,
    /// (tag, at)
    pub releases: Vec<(String, i64)>,
    /// The deployment list hit its 100 cap inside the range: there were more.
    pub deploys_capped: bool,
    /// PRs open right now (exact).
    pub prs_open: u64,
    /// Exact PR counts for this repo in the range (GitHub search counts).
    pub prs_merged: u64,
    pub prs_opened: u64,
    pub prs_closed: u64,
    pub error: Option<String>,
}

/// One workflow's runs in the range.
#[derive(Clone, Debug, Default)]
pub struct WorkflowStat {
    pub name: String,
    pub runs: u64,
    pub failed: u64,
    pub ok: u64,
    pub secs: i64,
}

impl RepoPulse {
    /// Runs per workflow, most failures first, then busiest.
    pub fn workflows(&self) -> Vec<WorkflowStat> {
        let mut by: HashMap<&str, WorkflowStat> = HashMap::new();
        for r in &self.runs {
            let e = by.entry(r.workflow.as_str()).or_insert_with(|| WorkflowStat { name: r.workflow.clone(), ..Default::default() });
            e.runs += 1;
            e.secs += r.secs;
            match r.outcome {
                Some(Outcome::Failure) => e.failed += 1,
                Some(Outcome::Success) => e.ok += 1,
                _ => {}
            }
        }
        let mut v: Vec<WorkflowStat> = by.into_values().collect();
        v.sort_by_key(|w| std::cmp::Reverse((w.failed, w.runs)));
        v
    }

    /// Workflows whose latest finished run on the default branch failed: "red right now".
    /// (workflow, when that run started, failures in a row before it went green).
    pub fn red_now(&self) -> Vec<(String, i64)> {
        let mut seen: Vec<&str> = Vec::new();
        let mut out = Vec::new();
        let mut runs: Vec<&RunStat> = self.runs.iter().filter(|r| r.branch == self.default_branch && !r.outcome.is_some_and(|o| o.is_running())).collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.at));
        for r in runs {
            if seen.contains(&r.workflow.as_str()) {
                continue;
            }
            seen.push(&r.workflow);
            if r.outcome == Some(Outcome::Failure) {
                out.push((r.workflow.clone(), r.at));
            }
        }
        out
    }

    /// Pass rate (passed / (passed + failed), exact; None without any), failed (exact), running now.
    pub fn health(&self) -> (Option<f32>, u64, usize) {
        let (ok, failed) = (self.runs_ok_total, self.runs_failed_total);
        let running = self.runs.iter().filter(|r| r.outcome.is_some_and(|o| o.is_running())).count();
        ((ok + failed > 0).then(|| ok as f32 / (ok + failed) as f32), failed, running)
    }

    /// Median run time in seconds.
    pub fn median_secs(&self) -> i64 {
        let mut v: Vec<i64> = self.runs.iter().filter(|r| !r.outcome.is_some_and(|o| o.is_running())).map(|r| r.secs).collect();
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0)
    }

    /// (environment, deploys, failed, latest state, latest at), busiest first.
    pub fn environments(&self) -> Vec<(String, usize, usize, String, i64)> {
        let mut by: HashMap<&str, (usize, usize, String, i64)> = HashMap::new();
        for (env, state, at) in &self.deploys {
            let e = by.entry(env.as_str()).or_insert((0, 0, state.clone(), *at));
            e.0 += 1;
            if state == "FAILURE" || state == "ERROR" {
                e.1 += 1;
            }
            if *at > e.3 {
                e.2 = state.clone();
                e.3 = *at;
            }
        }
        let mut v: Vec<_> = by.into_iter().map(|(k, (n, f, st, at))| (k.to_owned(), n, f, st, at)).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        v
    }

    pub fn lines(&self) -> (u64, u64) {
        self.commits.iter().fold((0, 0), |(a, d), c| (a + c.additions, d + c.deletions))
    }
    pub fn commits_capped(&self) -> bool {
        (self.commits.len() as u64) < self.commits_total
    }
    pub fn runs_capped(&self) -> bool {
        (self.runs.len() as u64) < self.runs_total
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Pulse {
    pub range: Option<Range>,
    /// PRs open right now across the repos (exact), and up to CAP of them in detail.
    pub open_total: u64,
    pub open_prs: Vec<OpenPr>,
    pub repos: Vec<RepoPulse>,
    pub opened_total: u64,
    pub merged_total: u64,
    pub closed_total: u64,
    pub opened: Vec<PrStat>,
    pub merged: Vec<PrStat>,
    pub errors: Vec<String>,
}

/// One person's share, from commits on default branches and merged PRs.
#[derive(Clone, Debug, Default)]
pub struct Contributor {
    pub name: String,
    pub commits: u64,
    pub additions: u64,
    pub deletions: u64,
    pub prs_merged: u64,
    pub repos: Vec<String>,
}

impl Pulse {
    pub fn contributors(&self) -> Vec<Contributor> {
        self.contributors_for(None)
    }

    /// Contributors overall, or for one repo.
    pub fn contributors_for(&self, slug: Option<&str>) -> Vec<Contributor> {
        let mut by: HashMap<String, Contributor> = HashMap::new();
        for r in self.repos.iter().filter(|r| slug.is_none_or(|x| r.slug == x)) {
            for c in &r.commits {
                let e = by.entry(c.author.clone()).or_insert_with(|| Contributor { name: c.author.clone(), ..Default::default() });
                e.commits += 1;
                e.additions += c.additions;
                e.deletions += c.deletions;
                if !e.repos.contains(&r.slug) {
                    e.repos.push(r.slug.clone());
                }
            }
        }
        for p in self.merged.iter().filter(|p| slug.is_none_or(|x| p.slug == x)) {
            let e = by.entry(p.author.clone()).or_insert_with(|| Contributor { name: p.author.clone(), ..Default::default() });
            e.prs_merged += 1;
            if !e.repos.contains(&p.slug) {
                e.repos.push(p.slug.clone());
            }
        }
        let mut v: Vec<Contributor> = by.into_values().collect();
        v.sort_by_key(|c| std::cmp::Reverse((c.additions + c.deletions, c.commits, c.prs_merged)));
        v
    }

    /// (commits, merged PRs) per day of the range.
    pub fn per_day(&self) -> Vec<(String, u64, u64)> {
        self.per_day_for(None)
    }

    /// (passed, failed) runs per day, overall or for one repo.
    pub fn ci_per_day(&self, slug: Option<&str>) -> Vec<(String, u64, u64)> {
        let Some(range) = &self.range else { return Vec::new() };
        let mut out: Vec<(String, u64, u64)> = range.days.iter().map(|(_, l)| (l.clone(), 0, 0)).collect();
        for r in self.repos.iter().filter(|r| slug.is_none_or(|x| r.slug == x)) {
            for x in &r.runs {
                let Some(i) = range.days.iter().rposition(|(start, _)| x.at >= *start) else { continue };
                match x.outcome {
                    Some(Outcome::Success) => out[i].1 += 1,
                    Some(Outcome::Failure) => out[i].2 += 1,
                    _ => {}
                }
            }
        }
        out
    }

    /// Open PRs of one repo (or all), oldest first.
    pub fn open_in(&self, slug: Option<&str>) -> Vec<&OpenPr> {
        let mut v: Vec<&OpenPr> = self.open_prs.iter().filter(|p| slug.is_none_or(|x| p.slug == x)).collect();
        v.sort_by_key(|p| p.created);
        v
    }

    /// Merged PRs of one repo, biggest first.
    pub fn merged_in(&self, slug: &str) -> Vec<&PrStat> {
        let mut v: Vec<&PrStat> = self.merged.iter().filter(|p| p.slug == slug).collect();
        v.sort_by_key(|p| std::cmp::Reverse(p.additions + p.deletions));
        v
    }

    pub fn opened_in(&self, slug: &str) -> usize {
        self.opened.iter().filter(|p| p.slug == slug).count()
    }

    /// Per day, overall or for one repo.
    pub fn per_day_for(&self, slug: Option<&str>) -> Vec<(String, u64, u64)> {
        let Some(range) = &self.range else { return Vec::new() };
        let bucket = |t: i64| range.days.iter().rposition(|(start, _)| t >= *start);
        let mut out: Vec<(String, u64, u64)> = range.days.iter().map(|(_, l)| (l.clone(), 0, 0)).collect();
        for r in self.repos.iter().filter(|r| slug.is_none_or(|x| r.slug == x)) {
            for c in &r.commits {
                if let Some(i) = bucket(c.at) {
                    out[i].1 += 1;
                }
            }
        }
        for p in self.merged.iter().filter(|p| slug.is_none_or(|x| p.slug == x)) {
            if let Some(i) = p.merged.and_then(bucket) {
                out[i].2 += 1;
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------------------------

/// Everything for `repos` over `range`. Blocking: run it on a worker.
pub fn load(repos: &[String], range: &Range) -> Pulse {
    let mut p = Pulse { range: Some(range.clone()), ..Default::default() };
    if repos.is_empty() {
        return p;
    }
    let span = format!("{}..{}", iso(range.since), iso(range.until));
    std::thread::scope(|scope| {
        let merged = scope.spawn(|| search_prs(repos, &format!("is:pr merged:{span}"), CAP));
        let opened = scope.spawn(|| search_prs(repos, &format!("is:pr created:{span}"), 100));
        let closed = scope.spawn(|| search_prs(repos, &format!("is:pr is:closed is:unmerged closed:{span}"), 0));
        let open = scope.spawn(|| search_open(repos));
        let repo_jobs: Vec<_> = repos.chunks(5).map(|chunk| scope.spawn(move || repos_detail(chunk, range))).collect();
        let run_jobs: Vec<_> = repos.iter().map(|slug| scope.spawn(move || (slug.clone(), runs(slug, range)))).collect();

        let take = |h: std::thread::ScopedJoinHandle<'_, Result<(u64, Vec<PrStat>), String>>, errors: &mut Vec<String>| match h.join() {
            Ok(Ok(x)) => x,
            Ok(Err(e)) => {
                errors.push(e);
                (0, Vec::new())
            }
            Err(_) => (0, Vec::new()),
        };
        (p.merged_total, p.merged) = take(merged, &mut p.errors);
        (p.opened_total, p.opened) = take(opened, &mut p.errors);
        p.closed_total = take(closed, &mut p.errors).0;
        match open.join() {
            Ok(Ok((n, list))) => (p.open_total, p.open_prs) = (n, list),
            Ok(Err(e)) => p.errors.push(e),
            Err(_) => {}
        }
        for j in repo_jobs {
            if let Ok(mut v) = j.join() {
                p.repos.append(&mut v);
            }
        }
        let mut runs_by: HashMap<String, Result<(u64, u64, u64, Vec<RunStat>), String>> = run_jobs.into_iter().filter_map(|j| j.join().ok()).collect();
        for r in &mut p.repos {
            match runs_by.remove(&r.slug) {
                Some(Ok((total, ok, failed, list))) => {
                    r.runs_total = total;
                    (r.runs_ok_total, r.runs_failed_total) = (ok, failed);
                    r.runs = list;
                }
                Some(Err(e)) => r.error = Some(r.error.take().map_or(e.clone(), |x| format!("{x}; {e}"))),
                None => {}
            }
        }
    });
    p.repos.sort_by_key(|r| std::cmp::Reverse({
        let (a, d) = r.lines();
        (a + d, r.commits_total)
    }));
    p
}

/// GitHub search over PRs in many repos: `repo:` qualifiers are split into queries short enough
/// for the search API. `cap` 0 only counts.
fn search_prs(repos: &[String], filter: &str, cap: usize) -> Result<(u64, Vec<PrStat>), String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for r in repos {
        let q = format!("repo:{r} ");
        if cur.len() + q.len() + filter.len() > 230 && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.push_str(&q);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    let query = "query($q:String!,$n:Int!,$after:String){search(query:$q,type:ISSUE,first:$n,after:$after){issueCount pageInfo{hasNextPage endCursor} \
                 nodes{...on PullRequest{number title additions deletions changedFiles createdAt mergedAt author{login} repository{nameWithOwner}}}}}";
    let mut total = 0;
    let mut out = Vec::new();
    for c in chunks {
        let q = format!("{c}{filter}");
        let mut after: Option<String> = None;
        let mut taken = 0;
        loop {
            let n = if cap == 0 { 1 } else { (cap - taken).min(100) };
            let mut args = vec!["api".to_owned(), "graphql".into(), "-f".into(), format!("query={query}"), "-F".into(), format!("q={q}"), "-F".into(), format!("n={n}")];
            if let Some(a) = &after {
                args.extend(["-f".into(), format!("after={a}")]);
            }
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let v: serde_json::Value = serde_json::from_str(&run("gh", &refs, None)?).map_err(|e| e.to_string())?;
            let s = &v["data"]["search"];
            if after.is_none() {
                total += s["issueCount"].as_u64().unwrap_or(0);
            }
            if cap == 0 {
                break;
            }
            for n in s["nodes"].as_array().into_iter().flatten() {
                taken += 1;
                out.push(PrStat {
                    slug: n["repository"]["nameWithOwner"].as_str().unwrap_or_default().to_owned(),
                    number: n["number"].as_u64().unwrap_or(0),
                    title: n["title"].as_str().unwrap_or_default().to_owned(),
                    author: n["author"]["login"].as_str().unwrap_or("ghost").to_owned(),
                    additions: n["additions"].as_u64().unwrap_or(0),
                    deletions: n["deletions"].as_u64().unwrap_or(0),
                    files: n["changedFiles"].as_u64().unwrap_or(0),
                    created: n["createdAt"].as_str().and_then(parse_time).unwrap_or(0),
                    merged: n["mergedAt"].as_str().and_then(parse_time),
                });
            }
            let more = s["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
            if !more || taken >= cap {
                break;
            }
            after = s["pageInfo"]["endCursor"].as_str().map(str::to_owned);
        }
    }
    Ok((total, out))
}

/// PRs open now in many repos (search, chunked like `search_prs`), with review and CI state.
fn search_open(repos: &[String]) -> Result<(u64, Vec<OpenPr>), String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for r in repos {
        let q = format!("repo:{r} ");
        if cur.len() + q.len() > 220 && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.push_str(&q);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    let query = "query($q:String!,$after:String){search(query:$q,type:ISSUE,first:100,after:$after){issueCount pageInfo{hasNextPage endCursor} \
                 nodes{...on PullRequest{number title isDraft createdAt updatedAt reviewDecision author{login} repository{nameWithOwner} \
                 commits(last:1){nodes{commit{statusCheckRollup{state}}}}}}}}";
    let (mut total, mut out) = (0, Vec::new());
    for c in chunks {
        let q = format!("{c}is:pr is:open");
        let mut after: Option<String> = None;
        loop {
            let mut args = vec!["api".to_owned(), "graphql".into(), "-f".into(), format!("query={query}"), "-F".into(), format!("q={q}")];
            if let Some(a) = &after {
                args.extend(["-f".into(), format!("after={a}")]);
            }
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let v: serde_json::Value = serde_json::from_str(&run("gh", &refs, None)?).map_err(|e| e.to_string())?;
            let s = &v["data"]["search"];
            if after.is_none() {
                total += s["issueCount"].as_u64().unwrap_or(0);
            }
            for n in s["nodes"].as_array().into_iter().flatten() {
                let t = |k: &str| n[k].as_str().and_then(parse_time).unwrap_or(0);
                out.push(OpenPr {
                    slug: n["repository"]["nameWithOwner"].as_str().unwrap_or_default().to_owned(),
                    number: n["number"].as_u64().unwrap_or(0),
                    title: n["title"].as_str().unwrap_or_default().to_owned(),
                    author: n["author"]["login"].as_str().unwrap_or("ghost").to_owned(),
                    created: t("createdAt"),
                    updated: t("updatedAt"),
                    draft: n["isDraft"].as_bool().unwrap_or(false),
                    review: n["reviewDecision"].as_str().unwrap_or_default().to_owned(),
                    ci: n["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["state"].as_str().unwrap_or_default().to_owned(),
                });
            }
            let more = s["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
            if !more || out.len() >= CAP {
                break;
            }
            after = s["pageInfo"]["endCursor"].as_str().map(str::to_owned);
        }
    }
    Ok((total, out))
}

/// Commits on the default branch, deployments and releases for a few repos in one query, then
/// more commit pages for repos that had more, up to CAP.
fn repos_detail(repos: &[String], range: &Range) -> Vec<RepoPulse> {
    let (since, until) = (iso(range.since), iso(range.until));
    let history = |after: &str| {
        format!(
            "history(since:\"{since}\",until:\"{until}\",first:100{after}){{totalCount pageInfo{{hasNextPage endCursor}} \
             nodes{{additions deletions committedDate author{{name user{{login}}}}}}}}"
        )
    };
    let span = format!("{since}..{until}");
    let mut q = String::from("query{");
    for (i, slug) in repos.iter().enumerate() {
        let Some((o, n)) = slug.split_once('/') else { continue };
        // Exact per-repo PR counts: count-only searches in the same request.
        for (alias, filter) in [("q", "is:pr is:open".to_owned()), ("m", format!("is:pr merged:{span}")), ("o", format!("is:pr created:{span}")), ("c", format!("is:pr is:closed is:unmerged closed:{span}"))] {
            q.push_str(&format!("{alias}{i}:search(query:{:?},type:ISSUE,first:1){{issueCount}} ", format!("repo:{slug} {filter}")));
        }
        q.push_str(&format!(
            "r{i}:repository(owner:{o:?},name:{n:?}){{defaultBranchRef{{name target{{...on Commit{{{}}}}}}} \
             deployments(first:100,orderBy:{{field:CREATED_AT,direction:DESC}}){{nodes{{environment createdAt latestStatus{{state}}}}}} \
             releases(first:20,orderBy:{{field:CREATED_AT,direction:DESC}}){{nodes{{tagName createdAt}}}}}} ",
            history("")
        ));
    }
    q.push('}');
    let v: Result<serde_json::Value, String> =
        run("gh", &["api", "graphql", "-f", &format!("query={q}")], None).and_then(|o| serde_json::from_str(&o).map_err(|e| e.to_string()));
    let mut out = Vec::new();
    for (i, slug) in repos.iter().enumerate() {
        let mut rp = RepoPulse { slug: slug.clone(), ..Default::default() };
        let repo = match &v {
            Ok(v) => v["data"][format!("r{i}")].clone(),
            Err(e) => {
                rp.error = Some(e.clone());
                out.push(rp);
                continue;
            }
        };
        if repo.is_null() {
            rp.error = Some("not accessible".into());
            out.push(rp);
            continue;
        }
        rp.default_branch = repo["defaultBranchRef"]["name"].as_str().unwrap_or_default().to_owned();
        if let Ok(v) = &v {
            let count = |a: &str| v["data"][format!("{a}{i}")]["issueCount"].as_u64().unwrap_or(0);
            (rp.prs_open, rp.prs_merged, rp.prs_opened, rp.prs_closed) = (count("q"), count("m"), count("o"), count("c"));
        }
        let mut h = repo["defaultBranchRef"]["target"]["history"].clone();
        rp.commits_total = h["totalCount"].as_u64().unwrap_or(0);
        loop {
            for c in h["nodes"].as_array().into_iter().flatten() {
                rp.commits.push(CommitStat {
                    author: c["author"]["user"]["login"].as_str().or(c["author"]["name"].as_str()).unwrap_or("unknown").to_owned(),
                    additions: c["additions"].as_u64().unwrap_or(0),
                    deletions: c["deletions"].as_u64().unwrap_or(0),
                    at: c["committedDate"].as_str().and_then(parse_time).unwrap_or(0),
                });
            }
            let more = h["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
            let Some(cursor) = h["pageInfo"]["endCursor"].as_str().filter(|_| more && rp.commits.len() < CAP).map(str::to_owned) else { break };
            let Some((o, n)) = slug.split_once('/') else { break };
            let page = format!(
                "query{{repository(owner:{o:?},name:{n:?}){{defaultBranchRef{{target{{...on Commit{{{}}}}}}}}}}}",
                history(&format!(",after:{cursor:?}"))
            );
            match run("gh", &["api", "graphql", "-f", &format!("query={page}")], None).and_then(|o| serde_json::from_str::<serde_json::Value>(&o).map_err(|e| e.to_string())) {
                Ok(next) => h = next["data"]["repository"]["defaultBranchRef"]["target"]["history"].clone(),
                Err(_) => break,
            }
        }
        let dep_nodes = repo["deployments"]["nodes"].as_array().cloned().unwrap_or_default();
        rp.deploys_capped = dep_nodes.len() >= 100 && dep_nodes.last().and_then(|d| d["createdAt"].as_str()).and_then(parse_time).is_some_and(|t| t >= range.since);
        for d in &dep_nodes {
            let at = d["createdAt"].as_str().and_then(parse_time).unwrap_or(0);
            if at >= range.since && at < range.until {
                rp.deploys.push((d["environment"].as_str().unwrap_or_default().to_owned(), d["latestStatus"]["state"].as_str().unwrap_or("PENDING").to_owned(), at));
            }
        }
        for r in repo["releases"]["nodes"].as_array().into_iter().flatten() {
            let at = r["createdAt"].as_str().and_then(parse_time).unwrap_or(0);
            if at >= range.since && at < range.until {
                rp.releases.push((r["tagName"].as_str().unwrap_or_default().to_owned(), at));
            }
        }
        out.push(rp);
    }
    out
}

/// Actions runs created in the range: the exact total, and up to CAP of them in detail.
fn runs(slug: &str, range: &Range) -> Result<(u64, u64, u64, Vec<RunStat>), String> {
    // Exact passed / failed totals: two count-only requests.
    let count = |status: &str| -> u64 {
        let url = format!("repos/{slug}/actions/runs?created={}..{}&status={status}&per_page=1", iso(range.since), iso(range.until));
        run("gh", &["api", &url, "--jq", ".total_count"], None).ok().and_then(|o| o.trim().parse().ok()).unwrap_or(0)
    };
    let (ok, failed) = std::thread::scope(|sc| {
        let a = sc.spawn(|| count("success"));
        let b = sc.spawn(|| count("failure"));
        (a.join().unwrap_or(0), b.join().unwrap_or(0))
    });
    let (total, list) = runs_detail(slug, range)?;
    Ok((total, ok, failed, list))
}

fn runs_detail(slug: &str, range: &Range) -> Result<(u64, Vec<RunStat>), String> {
    let mut total = 0;
    let mut out = Vec::new();
    for page in 1..=(CAP / 100) {
        let url = format!("repos/{slug}/actions/runs?created={}..{}&per_page=100&page={page}", iso(range.since), iso(range.until));
        let v: serde_json::Value = serde_json::from_str(&run("gh", &["api", &url], None)?).map_err(|e| e.to_string())?;
        total = v["total_count"].as_u64().unwrap_or(0);
        let list = v["workflow_runs"].as_array().cloned().unwrap_or_default();
        for r in &list {
            let s = |k: &str| r[k].as_str().unwrap_or_default().to_owned();
            let o = Outcome::from_parts(&s("status"), &s("conclusion"));
            let start = parse_time(&s("run_started_at")).or_else(|| parse_time(&s("created_at"))).unwrap_or(0);
            let end = parse_time(&s("updated_at")).unwrap_or(start);
            out.push(RunStat { workflow: s("name"), branch: s("head_branch"), outcome: Some(o), secs: (end - start).max(0), at: start });
        }
        if list.len() < 100 {
            break;
        }
    }
    Ok((total, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    #[test]
    fn week_starts_monday_midnight_local() {
        // Saturday 14 Nov 2026, 17:30 in UTC+7.
        let r = range(Preset::Week, at("2026-11-14T17:30:00+07:00"), None);
        assert_eq!(r.since, at("2026-11-09T00:00:00+07:00").timestamp());
        assert_eq!(r.days.len(), 6); // Mon..Sat
        assert_eq!(r.days[0].1, "Mon 9");
    }

    #[test]
    fn today_and_last7() {
        let now = at("2026-11-14T17:30:00+07:00");
        assert_eq!(range(Preset::Today, now, None).since, at("2026-11-14T00:00:00+07:00").timestamp());
        let r = range(Preset::Last7, now, None);
        assert_eq!(r.since, at("2026-11-08T00:00:00+07:00").timestamp());
        assert_eq!(r.days.len(), 7);
    }

    #[test]
    fn custom_is_inclusive_and_ordered() {
        let now = at("2026-11-14T17:30:00+07:00");
        let a = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let b = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let r = range(Preset::Custom, now, Some((a, b)));
        assert_eq!(r.since, at("2026-11-01T00:00:00+07:00").timestamp());
        assert_eq!(r.until, at("2026-11-04T00:00:00+07:00").timestamp());
        assert_eq!(r.days.len(), 3);
    }

    #[test]
    fn red_now_uses_latest_default_branch_run() {
        let run = |wf: &str, br: &str, o: Outcome, at: i64| RunStat { workflow: wf.into(), branch: br.into(), outcome: Some(o), secs: 10, at };
        let r = RepoPulse {
            default_branch: "main".into(),
            runs: vec![
                run("Tests", "main", Outcome::Failure, 30),
                run("Tests", "main", Outcome::Success, 20),
                run("Build", "main", Outcome::Success, 40),
                run("Build", "main", Outcome::Failure, 10),
                run("Lint", "feature", Outcome::Failure, 50),
            ],
            ..Default::default()
        };
        assert_eq!(r.red_now(), vec![("Tests".to_owned(), 30)]); // Build recovered; Lint isn't on main
        let r = RepoPulse { runs_ok_total: 9, runs_failed_total: 1, ..r };
        let (rate, failed, running) = r.health();
        assert_eq!((failed, running), (1, 0));
        assert!((rate.unwrap() - 0.9).abs() < 1e-6);
    }

    #[test]
    fn open_pr_states() {
        let pr = |draft, review: &str, ci: &str| OpenPr { draft, review: review.into(), ci: ci.into(), ..Default::default() };
        assert!(pr(false, "", "SUCCESS").needs_review());
        assert!(!pr(true, "", "SUCCESS").needs_review());
        assert!(pr(false, "APPROVED", "SUCCESS").ready());
        assert!(!pr(false, "APPROVED", "FAILURE").ready());
        assert!(pr(false, "", "ERROR").failing());
        assert!(pr(false, "", "SUCCESS").ready_for_review());
        assert!(!pr(false, "", "FAILURE").ready_for_review());
        assert!(!pr(false, "", "PENDING").ready_for_review());
        assert!(!pr(false, "", "").ready_for_review()); // no checks ran: not a pass
    }

    #[test]
    fn iso_is_utc() {
        assert_eq!(iso(at("2026-11-09T00:00:00+07:00").timestamp()), "2026-11-08T17:00:00Z");
    }

    #[test]
    fn contributors_and_days() {
        let range = range(Preset::Last7, at("2026-11-14T17:30:00+07:00"), None);
        let t = at("2026-11-13T10:00:00+07:00").timestamp();
        let p = Pulse {
            range: Some(range),
            repos: vec![RepoPulse {
                slug: "a/b".into(),
                commits: vec![
                    CommitStat { author: "ann".into(), additions: 50, deletions: 5, at: t },
                    CommitStat { author: "bob".into(), additions: 5, deletions: 1, at: t },
                    CommitStat { author: "ann".into(), additions: 10, deletions: 0, at: t },
                ],
                ..Default::default()
            }],
            merged: vec![PrStat { slug: "a/b".into(), author: "bob".into(), merged: Some(t), ..Default::default() }],
            ..Default::default()
        };
        let c = p.contributors();
        assert_eq!(c[0].name, "ann");
        assert_eq!((c[0].commits, c[0].additions), (2, 60));
        assert_eq!(c[1].prs_merged, 1);
        assert_eq!(p.contributors_for(Some("a/b")).len(), 2);
        assert!(p.contributors_for(Some("x/y")).is_empty());
        let days = p.per_day();
        let fri = days.iter().find(|d| d.0 == "Fri 13").unwrap();
        assert_eq!((fri.1, fri.2), (3, 1));
    }
}
