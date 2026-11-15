//! Links that lead somewhere specific (a red workflow, a failing check, a PR in a list) open
//! that exact thing: the page, the item selected with its details open, and its row scrolled
//! into view and highlighted once it's in the table. An item outside the loaded page of the
//! list (a run from last week) is fetched on its own so it still shows.

use std::time::{Duration, Instant};

use crate::model::{Outcome, Pr, Run};
use crate::{Page, State, gh, jobs};

/// A row to scroll into view and highlight when it appears in its page's table.
#[derive(Clone, Debug)]
pub struct Reveal {
    pub page: Page,
    pub id: String,
    pub since: Instant,
}

/// "The latest failed run" wanted on a repo's Actions, once its runs are in (for links that
/// don't say which run, like a CI notification).
#[derive(Clone, Debug)]
pub struct WantFailed {
    pub slug: String,
    pub workflow: Option<String>,
    pub since: Instant,
}

/// How long a reveal waits for its row before giving up (a list that never loads).
const WAIT: Duration = Duration::from_secs(20);

impl State {
    /// Open run `id` of `slug` on Actions: selected, details open, row revealed.
    pub fn open_run(&mut self, slug: &str, id: u64) {
        self.select_repo(slug, Page::Actions);
        self.select_run(id);
        self.reveal = Some(Reveal { page: Page::Actions, id: id.to_string(), since: Instant::now() });
        let listed = self.rd().and_then(|d| d.runs.data.as_ref()).is_some_and(|r| r.iter().any(|r| r.database_id == id));
        if !listed {
            let (key, slug) = (slug.to_owned(), slug.to_owned());
            jobs::spawn(self, move || gh::run_one(&key, id), move |s, r| match r {
                Ok(run) => {
                    s.focus_run = Some((slug, run));
                    s.keep_focused_run();
                }
                Err(e) => s.toast(crate::i18n::t(tf!("Couldn't load that run: {e}")), true),
            });
        }
    }

    /// Open PR `n` of `slug`: selected, details open, row revealed (a merged or closed one is
    /// fetched on its own, since the list shows open ones).
    pub fn open_pr(&mut self, slug: &str, n: u64) {
        self.pr_state = 0;
        self.select_repo(slug, Page::Pulls);
        self.select_pr(n);
        self.reveal = Some(Reveal { page: Page::Pulls, id: n.to_string(), since: Instant::now() });
        let listed = self.rd().and_then(|d| d.prs.data.as_ref()).is_some_and(|p| p.iter().any(|p| p.number == n));
        if !listed {
            let (key, slug) = (slug.to_owned(), slug.to_owned());
            jobs::spawn(self, move || gh::pr_full(&key, n), move |s, r| match r {
                Ok(pr) => s.focus_pr = Some((slug, pr)),
                Err(e) => s.toast(crate::i18n::t(tf!("Couldn't load that pull request: {e}")), true),
            });
        }
    }

    /// Open the latest failed run on `slug` (of `workflow`, if given): for links that don't
    /// name a run. Resolved once the repo's runs are in.
    pub fn open_latest_failed(&mut self, slug: &str, workflow: Option<String>) {
        self.select_repo(slug, Page::Actions);
        self.want_failed = Some(WantFailed { slug: slug.to_owned(), workflow, since: Instant::now() });
        self.resolve_want_failed();
    }

    /// The run a link opened stays in the list when the list reloads without it (it's older
    /// than the loaded page), in date order.
    pub fn keep_focused_run(&mut self) {
        let Some((slug, run)) = self.focus_run.clone() else { return };
        let Some(list) = self.data.get_mut(&slug).and_then(|d| d.runs.data.as_mut()) else { return };
        if list.iter().any(|r| r.database_id == run.database_id) {
            return;
        }
        let at = list.iter().position(|r| r.created_at < run.created_at).unwrap_or(list.len());
        list.insert(at, run);
    }

    /// The PR a link opened, when it isn't in the loaded list.
    pub fn focused_pr(&self, n: u64) -> Option<&Pr> {
        self.focus_pr.as_ref().filter(|(slug, p)| self.cur.as_deref() == Some(slug) && p.number == n).map(|(_, p)| p)
    }

    fn resolve_want_failed(&mut self) {
        let Some(want) = self.want_failed.clone() else { return };
        if self.cur.as_deref() != Some(&want.slug) || want.since.elapsed() > WAIT {
            self.want_failed = None;
            return;
        }
        let Some(d) = self.rd() else { return };
        let Some(runs) = d.runs.data.as_ref() else { return };
        let of_workflow = |r: &&Run| want.workflow.as_ref().is_none_or(|w| r.workflow_name == *w);
        let failed = runs.iter().filter(of_workflow).find(|r| r.outcome() == Outcome::Failure).map(|r| r.database_id);
        let pick = failed.or_else(|| runs.iter().find(of_workflow).map(|r| r.database_id));
        if pick.is_none() && d.runs.loading {
            return; // the fresh list may have it
        }
        self.want_failed = None;
        match pick {
            Some(id) => self.open_run(&want.slug, id),
            None => self.toast(crate::i18n::t("No recent run to show for that"), false),
        }
    }

    /// From `poll`: scroll a revealed row into view once it's in its table, and resolve a
    /// pending "latest failed run". True when something changed.
    pub fn poll_focus(&mut self, tx: &mut rune::Tx) -> bool {
        self.resolve_want_failed();
        let Some(r) = self.reveal.clone() else { return false };
        if self.page != r.page || r.since.elapsed() > WAIT {
            self.reveal = None;
            return false;
        }
        let present = match r.page {
            Page::Actions => self.rd().and_then(|d| d.runs.data.as_ref()).is_some_and(|l| l.iter().any(|x| x.database_id.to_string() == r.id)),
            Page::Pulls => self.rd().and_then(|d| d.prs.data.as_ref()).is_some_and(|l| l.iter().any(|x| x.number.to_string() == r.id)),
            _ => false,
        };
        if !present {
            return false;
        }
        match r.page {
            Page::Actions => self.runs_table.reveal(r.id, tx),
            _ => self.prs_table.reveal(r.id, tx),
        }
        self.reveal = None;
        true
    }
}
