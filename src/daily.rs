//! Daily flow (B1–B8): notifications, the review queue, CI triage, group dashboards, watching
//! with desktop notifications, branch clean-up data, the start-up cache and the command palette.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::config::Theme;
use crate::model::{Envs, Notif, Outcome, Pr, Run, now, parse_time};
use crate::{Load, Page, State, cache, gh, git, jobs, risk};

// ---------------------------------------------------------------------------------------------
// B1: notifications
// ---------------------------------------------------------------------------------------------

impl State {
    pub fn refresh_notifications(&mut self) {
        self.notifs.loading = true;
        self.last_notif = Instant::now();
        jobs::spawn(self, gh::notifications, |s, r| {
            s.notifs.finish(r);
            s.prefs.snoozed.retain(|(_, until)| *until > now());
        });
    }

    /// Unread, not snoozed.
    pub fn visible_notifs(&self) -> Vec<&Notif> {
        let t = now();
        self.notifs
            .data
            .iter()
            .flatten()
            .filter(|n| n.unread && !self.prefs.snoozed.iter().any(|(id, until)| *id == n.id && *until > t))
            .collect()
    }

    /// "Done": marks it read on GitHub too.
    pub fn notif_done(&mut self, id: String) {
        if let Some(list) = self.notifs.data.as_mut() {
            list.retain(|n| n.id != id);
        }
        self.act(crate::i18n::t("Mark done"), move || gh::mark_read(&id), |_| {});
    }

    /// Local only: it comes back after `secs`.
    pub fn notif_snooze(&mut self, id: String, secs: i64) {
        self.prefs.snoozed.retain(|(i, _)| *i != id);
        self.prefs.snoozed.push((id, now() + secs));
        self.save_prefs();
    }

    pub fn open_notif(&mut self, n: &Notif, tx: &mut rune::Tx) {
        match (n.kind.as_str(), n.number) {
            ("PullRequest", Some(num)) => self.open_pr(&n.repo.clone(), num),
            // A CI notification doesn't name its run: open the latest failed one.
            ("CheckSuite", _) => self.open_latest_failed(&n.repo.clone(), None),
            ("Issue", Some(num)) => crate::open_url(tx, &format!("https://github.com/{}/issues/{num}", n.repo)),
            ("Release", _) => crate::open_url(tx, &format!("https://github.com/{}/releases", n.repo)),
            _ => crate::open_url(tx, &format!("https://github.com/{}", n.repo)),
        }
    }

    // -----------------------------------------------------------------------------------------
    // B2: review queue
    // -----------------------------------------------------------------------------------------

    /// Waiting times, sizes and CI for the inbox's PRs, in one GraphQL query.
    pub fn refresh_queue(&mut self) {
        let Some(inbox) = &self.inbox.data else { return };
        let keys: Vec<(String, u64)> =
            inbox.review_requested.iter().chain(&inbox.my_prs).map(|h| (h.repository.name_with_owner.clone(), h.number)).collect();
        jobs::spawn(self, move || gh::queue_info(&keys), |s, r| match r {
            Ok(q) => s.queue = q,
            Err(e) => s.toast(crate::i18n::t(tf!("Review queue details failed: {e}")), true),
        });
    }

    /// The review queue, oldest wait first: (index into review_requested, waited seconds).
    pub fn review_queue(&self) -> Vec<(usize, i64)> {
        let Some(inbox) = &self.inbox.data else { return Vec::new() };
        let t = now();
        let mut v: Vec<(usize, i64)> = inbox
            .review_requested
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let since = self.queue.get(&(h.repository.name_with_owner.clone(), h.number)).map(|q| q.requested_at).filter(|t| *t > 0);
                (i, t - since.or_else(|| parse_time(&h.updated_at)).unwrap_or(t))
            })
            .collect();
        // Ready to review (all checks passed) first, then not ready; oldest wait first in each.
        v.sort_by_key(|(i, waited)| (!self.queue_ci(*i).reviewable(), -waited));
        v
    }

    /// The CI state of review-queue entry `i` (from the batched query; None until it loads).
    pub fn queue_ci(&self, i: usize) -> crate::model::Ci {
        let Some(h) = self.inbox.data.as_ref().and_then(|x| x.review_requested.get(i)) else { return crate::model::Ci::None };
        self.queue.get(&(h.repository.name_with_owner.clone(), h.number)).map_or(crate::model::Ci::None, |q| crate::model::Ci::from_rollup(&q.ci))
    }

    /// J/K/Enter/A/C on the inbox.
    pub fn queue_key(&mut self, key: char) -> bool {
        let q = self.review_queue();
        if q.is_empty() {
            return false;
        }
        let Some(inbox) = &self.inbox.data else { return false };
        self.queue_sel = self.queue_sel.min(q.len() - 1);
        let hit = inbox.review_requested[q[self.queue_sel].0].clone();
        let (slug, n) = (hit.repository.name_with_owner.clone(), hit.number);
        match key {
            'j' => self.queue_sel = (self.queue_sel + 1).min(q.len() - 1),
            'k' => self.queue_sel = self.queue_sel.saturating_sub(1),
            '\n' => self.open_pr(&slug, n),
            'a' => self.request(crate::guard::Action::Approve { slug, number: n, title: hit.title, body: String::new(), confirm: true }),
            'c' => {
                self.open_pr(&slug, n);
                self.comment.focused = true;
            }
            _ => return false,
        }
        true
    }

    // -----------------------------------------------------------------------------------------
    // B4: CI triage
    // -----------------------------------------------------------------------------------------

    pub fn load_triage(&mut self, id: u64) {
        if self.triage_for == Some(id) {
            return;
        }
        let Some(slug) = self.cur.clone() else { return };
        self.triage_for = Some(id);
        self.triage = Load::start();
        jobs::spawn(self, move || gh::failed_log(&slug, id, 45), move |s, r| {
            if s.triage_for == Some(id) {
                s.triage.finish(r);
            }
        });
    }

    /// Same workflow, same commit, both failed and passed: probably flaky.
    pub fn flaky(&self, run: &Run) -> bool {
        let Some(runs) = self.rd().and_then(|d| d.runs.data.as_ref()) else { return false };
        let same: Vec<&Run> = runs.iter().filter(|r| r.workflow_name == run.workflow_name && r.head_sha == run.head_sha && !r.head_sha.is_empty()).collect();
        same.iter().any(|r| r.outcome() == Outcome::Failure) && same.iter().any(|r| r.outcome() == Outcome::Success)
    }

    // -----------------------------------------------------------------------------------------
    // B5: group dashboard
    // -----------------------------------------------------------------------------------------

    pub fn open_group(&mut self, name: &str) {
        self.remember(&crate::Place { page: Page::Group, repo: None, group: Some(name.to_owned()) });
        self.group_cur = Some(name.to_owned());
        self.page = Page::Group;
        self.load_group(true);
    }

    /// Load the open group's repos: all of them, or (`only_stale`) the ones not fresh.
    pub fn load_group(&mut self, only_stale: bool) {
        let Some(name) = self.group_cur.clone() else { return };
        let Some(g) = self.prefs.groups.iter().find(|g| g.name == name) else { return };
        let prod_names = self.prefs.prod_names.clone();
        for slug in g.repos.clone() {
            let entry = self.group_data.entry(slug.clone()).or_default();
            if entry.loading || (only_stale && !entry.stale(crate::ttl::GH)) {
                continue;
            }
            entry.loading = true;
            let names = prod_names.clone();
            let key = slug.clone();
            jobs::spawn(self, move || group_repo(&key, &names), move |s, r| s.group_data.entry(slug).or_default().finish(r));
        }
    }

    // -----------------------------------------------------------------------------------------
    // B6: watching
    // -----------------------------------------------------------------------------------------

    pub fn is_watched(&self, t: &WatchTarget) -> bool {
        self.watches.iter().any(|w| w.target == *t)
    }

    pub fn toggle_watch(&mut self, target: WatchTarget, label: String) {
        if let Some(i) = self.watches.iter().position(|w| w.target == target) {
            self.watches.remove(i);
            self.toast(crate::i18n::t(tf!("Stopped watching {label}")), false);
        } else {
            self.toast(crate::i18n::t(tf!("Watching {label}: you'll get a notification when it changes")), false);
            self.watches.push(Watch { target, label, last: String::new(), since: Instant::now() });
            self.poll_watches();
        }
    }

    pub fn watch_branch(&mut self, slug: &str, branch: &str) {
        let t = WatchTarget::Branch { slug: slug.to_owned(), branch: branch.to_owned() };
        if !self.is_watched(&t) {
            self.watches.push(Watch { target: t, label: tf!("CI on {branch}"), last: String::new(), since: Instant::now() });
        }
    }

    /// Every 30 s: check each watch, notify on changes.
    pub fn poll_watches(&mut self) {
        self.last_watch = Instant::now();
        // Branch watches last a working day.
        self.watches.retain(|w| !matches!(w.target, WatchTarget::Branch { .. }) || w.since.elapsed() < Duration::from_secs(12 * 3600));
        if self.watches.is_empty() {
            return;
        }
        let targets: Vec<WatchTarget> = self.watches.iter().map(|w| w.target.clone()).collect();
        jobs::spawn(
            self,
            move || targets.into_iter().map(|t| { let s = watch_state(&t); (t, s) }).collect::<Vec<_>>(),
            |s, results| {
                for (target, state) in results {
                    let Some((summary, done)) = state else { continue };
                    let Some(w) = s.watches.iter_mut().find(|w| w.target == target) else { continue };
                    let changed = !w.last.is_empty() && w.last != summary;
                    let first = w.last.is_empty();
                    w.last = summary.clone();
                    let label = w.label.clone();
                    if changed {
                        s.alert(&label, &summary, &target);
                    }
                    // A watched run that finished is done; so is a PR that merged or closed.
                    if done && (changed || first) && !matches!(target, WatchTarget::Branch { .. }) {
                        if first {
                            s.toast(crate::i18n::t(tf!("{label}: {summary}")), false);
                        }
                        s.watches.retain(|w| w.target != target);
                    }
                }
            },
        );
    }

    /// A toast, and an OS notification if allowed.
    fn alert(&mut self, title: &str, body: &str, target: &WatchTarget) {
        self.toast(crate::i18n::t(tf!("{title}: {body}")), false);
        if !self.prefs.desktop_notify {
            return;
        }
        let (slug, kind, id) = match target {
            WatchTarget::Run { slug, id } => (slug.clone(), "run", id.to_string()),
            WatchTarget::Pr { slug, number } => (slug.clone(), "pr", number.to_string()),
            WatchTarget::Branch { slug, branch } => (slug.clone(), "branch", branch.clone()),
        };
        let n = rune_notify::Notification::new(crate::i18n::t(tf!("{kind}-{id}-{}", now())), title).body(body).info("slug", slug).info("kind", kind).info("id", id);
        let _ = rune_notify::post(&n);
    }

    // -----------------------------------------------------------------------------------------
    // B7: clean-up and stashes
    // -----------------------------------------------------------------------------------------

    pub fn refresh_branch_tools(&mut self) {
        let (Some(slug), Some(dir)) = (self.cur.clone(), self.local_path()) else { return };
        let meta = self.rd().and_then(|d| d.meta.data.clone()).unwrap_or_default();
        let d = self.rd_mut_pub(&slug);
        d.stashes.loading = true;
        d.cleanup.loading = true;
        let (dir2, slug2) = (dir.clone(), slug.clone());
        jobs::spawn(self, move || git::stashes(&dir2), move |s, r| s.rd_mut_pub(&slug2).stashes.finish(r));
        jobs::spawn(
            self,
            move || {
                let default = if meta.default_branch.is_empty() { "main".to_owned() } else { meta.default_branch.clone() };
                git::cleanup(&dir, &default, now(), |b| risk::important_branch(b, &default, &meta.protected))
            },
            move |s, r| s.rd_mut_pub(&slug).cleanup.finish(r),
        );
    }

    // -----------------------------------------------------------------------------------------
    // B8: cache
    // -----------------------------------------------------------------------------------------

    pub fn load_cache(&mut self) {
        let Some(snap) = cache::load() else { return };
        self.me = snap.me;
        for (slug, pushed, archived) in snap.repos {
            self.add_repo_pub(slug.clone(), None);
            if let Some(r) = self.repos.iter_mut().find(|r| r.slug == slug) {
                r.pushed_at = pushed;
                r.archived = archived;
            }
        }
        let at = snap.saved_at;
        if let Some(i) = snap.inbox {
            self.inbox = Load::from_cache(i, at);
        }
        if !snap.notifications.is_empty() {
            self.notifs = Load::from_cache(snap.notifications, at);
        }
        if let Some((key, p)) = snap.pulse {
            self.pulse_seen.insert(key.clone(), (snap.pulse_at.max(1), p.clone()));
            self.pulse_key = key;
            self.pulse = Load::from_cache(p, snap.pulse_at.max(1));
        }
        if let Some((key, rows)) = snap.compare.filter(|(k, _)| *k == self.compare_key_now()) {
            self.compare_key = key;
            self.compare = Load::from_cache(rows, snap.compare_at.max(1));
        }
        for (slug, prs) in snap.prs {
            self.rd_mut_pub(&slug).prs = Load::from_cache(prs, at);
        }
        for (slug, runs) in snap.runs {
            self.rd_mut_pub(&slug).runs = Load::from_cache(runs, at);
        }
        for (slug, envs) in snap.envs {
            self.rd_mut_pub(&slug).envs = Load::from_cache(envs, at);
        }
        for (slug, meta) in snap.meta {
            self.rd_mut_pub(&slug).meta = Load::from_cache(meta, at);
        }
        for (slug, g) in snap.groups {
            self.group_data.insert(slug, Load::from_cache(g, at));
        }
    }

    /// Write the cache on a worker; called every few minutes from `poll`.
    pub fn save_cache(&mut self) {
        self.last_cache = Instant::now();
        if jobs::is_inline_run() {
            return; // headless stills don't overwrite your cache
        }
        let mut prs: HashMap<String, Vec<Pr>> = HashMap::new();
        let mut runs: HashMap<String, Vec<Run>> = HashMap::new();
        let mut envs = HashMap::new();
        let mut meta = HashMap::new();
        for (slug, d) in &self.data {
            if let Some(e) = &d.envs.data {
                envs.insert(slug.clone(), e.clone());
            }
            if let Some(m) = &d.meta.data {
                meta.insert(slug.clone(), m.clone());
            }
            if d.prs_state == 0
                && let Some(p) = &d.prs.data
            {
                prs.insert(slug.clone(), p.iter().take(50).cloned().collect());
            }
            if let Some(r) = &d.runs.data {
                runs.insert(slug.clone(), r.iter().take(50).cloned().collect());
            }
        }
        let snap = cache::Snapshot {
            saved_at: now(),
            me: self.me.clone(),
            repos: self.repos.iter().map(|r| (r.slug.clone(), r.pushed_at.clone(), r.archived)).collect(),
            inbox: self.inbox.data.clone(),
            notifications: self.notifs.data.clone().unwrap_or_default(),
            prs,
            runs,
            envs,
            meta,
            groups: self.group_data.iter().filter_map(|(k, v)| Some((k.clone(), v.data.clone()?))).collect(),
            pulse: self.pulse.data.clone().map(|p| (self.pulse_key.clone(), p)),
            pulse_at: self.pulse.at.unwrap_or(0),
            compare: self.compare.data.clone().map(|c| (self.compare_key.clone(), c)),
            compare_at: self.compare.at.unwrap_or(0),
        };
        jobs::spawn(self, move || cache::save(&snap), |_, _| {});
    }

    // -----------------------------------------------------------------------------------------
    // B3: command palette
    // -----------------------------------------------------------------------------------------

    pub fn open_palette(&mut self) {
        self.palette = true;
        self.palette_q = rune::TextEdit::new("");
        self.palette_q.focused = true;
        self.palette_sel = 0;
    }

    pub fn palette_items(&self) -> Vec<(PaletteItem, String, String)> {
        let q = self.palette_q.text.trim().to_lowercase();
        let mut out: Vec<(i64, PaletteItem, String, String)> = Vec::new();
        // Matches the title as shown (Thai, say) or in English, whichever fits better.
        let mut add = |item: PaletteItem, title: String, hint: String, boost: i64| {
            let shown = crate::i18n::t(title.as_str());
            let score = fuzzy(&q, &shown.to_lowercase()).max(fuzzy(&q, &title.to_lowercase()))
                // The page used to be called Inbox.
                .max(if title == "Dashboard" { fuzzy(&q, "inbox") } else { None });
            if let Some(score) = score {
                out.push((score + boost, item, title, hint));
            }
        };
        // Commands first when the query looks like one.
        for (cmd, hint) in COMMANDS {
            add(PaletteItem::Command(cmd), cmd.to_owned(), hint.to_owned(), 5);
        }
        if self.cur.is_some() {
            for p in Page::REPO {
                add(PaletteItem::Page(p), tf!("Go to {}", crate::i18n::t(p.label())), "this repository".into(), 10);
            }
            if let Some(prs) = self.rd().and_then(|d| d.prs.data.as_ref()) {
                for pr in prs.iter().take(100) {
                    add(PaletteItem::Pr(pr.number), format!("#{} {}", pr.number, pr.title), pr.author.login.clone(), 0);
                }
            }
            if let Some(branches) = self.rd().and_then(|d| d.branches.data.as_ref()) {
                for b in branches.iter().filter(|b| !b.remote).take(200) {
                    add(PaletteItem::Branch(b.name.clone()), tf!("Checkout {}", b.name), "local branch".into(), -5);
                }
            }
        }
        for g in &self.prefs.groups {
            add(PaletteItem::Group(g.name.clone()), tf!("Group: {}", g.name), tf!("{} repos", g.repos.len()), 15);
        }
        for r in &self.repos {
            let boost = if self.prefs.pinned.contains(&r.slug) { 40 } else { 0 }
                + self.prefs.recent.iter().position(|x| *x == r.slug).map_or(0, |i| 30 - i as i64);
            add(PaletteItem::Repo(r.slug.clone()), r.slug.clone(), if r.path.is_some() { "local clone".into() } else { "repository".into() }, boost);
        }
        out.sort_by(|a, b| b.0.cmp(&a.0));
        out.into_iter().take(14).map(|(_, i, t, h)| (i, t, h)).collect()
    }

    pub fn run_palette(&mut self, item: PaletteItem, tx: &mut rune::Tx) {
        self.palette = false;
        match item {
            PaletteItem::Repo(slug) => {
                let page = if self.page == Page::Inbox || self.page == Page::Group { Page::Pulls } else { self.page };
                self.select_repo(&slug, page);
            }
            PaletteItem::Page(p) => self.show(p),
            PaletteItem::Pr(n) => {
                if let Some(slug) = self.cur.clone() {
                    self.open_pr(&slug, n);
                }
            }
            PaletteItem::Branch(name) => {
                if let (Some(dir), Some(slug)) = (self.local_path(), self.cur.clone()) {
                    self.request(crate::guard::Action::Checkout { dir, slug, name, remote: false });
                }
            }
            PaletteItem::Group(g) => self.open_group(&g),
            PaletteItem::Command(c) => match c {
                "Dashboard" => self.show(Page::Inbox),
                "Settings" => self.open_settings(0),
                "Guide" => self.guide_open = true,
                "Refresh" => self.refresh_page(),
                "Fetch" => {
                    if let Some(dir) = self.local_path() {
                        self.request(crate::guard::Action::Fetch { dir });
                    }
                }
                "Pull" => {
                    if let Some(dir) = self.local_path() {
                        self.request(crate::guard::Action::Pull { dir });
                    }
                }
                "Theme: Light" => self.set_theme(Theme::Light),
                "Theme: Dark" => self.set_theme(Theme::Dark),
                "Theme: System" => self.set_theme(Theme::System),
                "Open on GitHub" => {
                    if let Some(slug) = self.cur.clone() {
                        crate::open_url(tx, &format!("https://github.com/{slug}"));
                    }
                }
                _ => {}
            },
        }
    }
}

pub const COMMANDS: [(&str, &str); 10] = [
    ("Dashboard", "Ctrl+1"),
    ("Settings", "appearance, accounts, git identity, safety, notifications (Ctrl+,)"),
    ("Guide", "how everything works · F1"),
    ("Refresh", "Ctrl+R"),
    ("Fetch", "git fetch --all --prune"),
    ("Pull", "git pull --ff-only"),
    ("Open on GitHub", "this repository"),
    ("Theme: Light", ""),
    ("Theme: Dark", ""),
    ("Theme: System", ""),
];

#[derive(Clone, Debug, PartialEq)]
pub enum PaletteItem {
    Repo(String),
    Page(Page),
    Pr(u64),
    Branch(String),
    Group(String),
    Command(&'static str),
}

/// Subsequence match: every query character in order. Higher is better: consecutive runs,
/// a match at a word start, and shorter targets win.
pub fn fuzzy(q: &str, target: &str) -> Option<i64> {
    if q.is_empty() {
        return Some(0);
    }
    if let Some(i) = target.find(q) {
        return Some(1000 - i as i64 * 2 - target.len() as i64);
    }
    let t: Vec<char> = target.chars().collect();
    let mut score = 0i64;
    let mut ti = 0;
    let mut prev: Option<usize> = None;
    for qc in q.chars() {
        let found = (ti..t.len()).find(|&i| t[i] == qc)?;
        score += if prev == Some(found.wrapping_sub(1)) { 15 } else { 1 };
        if found == 0 || matches!(t[found - 1], '/' | ' ' | '-' | '_' | '#') {
            score += 10;
        }
        prev = Some(found);
        ti = found + 1;
    }
    Some(score - t.len() as i64 / 4)
}

// ---------------------------------------------------------------------------------------------
// Watch types and worker functions
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum WatchTarget {
    Run { slug: String, id: u64 },
    Pr { slug: String, number: u64 },
    Branch { slug: String, branch: String },
}

pub struct Watch {
    pub target: WatchTarget,
    pub label: String,
    /// The last summary seen; a change raises a notification.
    pub last: String,
    pub since: Instant,
}

/// (summary, finished) for a watch.
fn watch_state(t: &WatchTarget) -> Option<(String, bool)> {
    match t {
        WatchTarget::Run { slug, id } => {
            let r = gh::run_one(slug, *id).ok()?;
            let o = r.outcome();
            let word = if o.is_running() { r.status.replace('_', " ") } else { r.conclusion.clone() };
            Some((word, !o.is_running()))
        }
        WatchTarget::Pr { slug, number } => {
            let p = gh::pr_one(slug, *number).ok()?;
            let (o, passed, total) = p.checks();
            let checks = match o {
                Outcome::Running => "checks running".to_owned(),
                Outcome::Failure => tf!("checks failing ({passed}/{total})"),
                Outcome::Success => "checks passed".to_owned(),
                Outcome::Neutral => "no checks".to_owned(),
            };
            let review = match p.review_decision.as_str() {
                "APPROVED" => "approved",
                "CHANGES_REQUESTED" => "changes requested",
                _ => "awaiting review",
            };
            let done = p.state != "OPEN";
            Some((if done { p.state.to_lowercase() } else { format!("{checks}, {review}") }, done))
        }
        WatchTarget::Branch { slug, branch } => {
            // The newest run on the branch: "Tests: success".
            let r = gh::runs_on(slug, branch, 1).ok()?.into_iter().next()?;
            let o = r.outcome();
            let word = if o.is_running() { "running".to_owned() } else { r.conclusion.clone() };
            Some((tf!("{} #{}: {word}", r.workflow_name, r.number), false))
        }
    }
}

/// One repo's row on a group dashboard.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GroupRepo {
    pub default_branch: String,
    /// Latest run per workflow on the default branch.
    pub health: Vec<Run>,
    pub prs: Vec<Pr>,
    pub envs: Envs,
    /// (production environment, deployed sha, commits on the default branch not deployed).
    pub drift: Vec<(String, String, Option<u32>)>,
}

fn group_repo(slug: &str, prod_names: &[String]) -> Result<GroupRepo, String> {
    let meta = gh::repo_meta(slug)?;
    let runs = gh::runs_on(slug, &meta.default_branch, 20).unwrap_or_default();
    let mut health: Vec<Run> = Vec::new();
    for r in runs {
        if !health.iter().any(|h| h.workflow_name == r.workflow_name) {
            health.push(r);
        }
    }
    let prs = gh::prs(slug, "open", 30, None).unwrap_or_default();
    let envs = gh::envs(slug).unwrap_or_default();
    let mut drift = Vec::new();
    for name in &envs.names {
        if !risk::is_production(name, envs.protected.contains(name), prod_names) {
            continue;
        }
        if let Some(d) = envs.deployments.iter().find(|d| d.environment == *name && d.state != "INACTIVE" && d.state != "DESTROYED") {
            drift.push((name.clone(), d.sha.clone(), gh::ahead_by(slug, &d.sha, &meta.default_branch)));
        }
    }
    Ok(GroupRepo { default_branch: meta.default_branch, health, prs, envs, drift })
}

#[cfg(test)]
mod tests {
    use super::{PaletteItem, fuzzy};

    #[test]
    fn fuzzy_ranks_exact_and_word_starts() {
        let exact = fuzzy("rune", "anomalyco/rune").unwrap();
        let spread = fuzzy("rune", "really-unused-name").unwrap();
        assert!(exact > spread);
        assert!(fuzzy("rlcrg", "rust-lang/cargo").is_some());
        assert!(fuzzy("xyz", "anomalyco/rune").is_none());
    }

    #[test]
    fn palette_finds_dashboard_by_old_name() {
        let mut s = crate::State::new(crate::config::Prefs::default());
        s.palette_q = rune::TextEdit::new("inbox");
        assert!(s.palette_items().iter().any(|(item, title, _)| matches!(item, PaletteItem::Command("Dashboard")) && title == "Dashboard"));
    }

    #[test]
    fn page_is_called_dashboard() {
        assert_eq!(crate::Page::Inbox.label(), "Dashboard");
    }
}
