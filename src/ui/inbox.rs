//! Inbox: what needs you today, across every repo: the review queue (B2), your PRs and who
//! you're waiting on, GitHub notifications with Done / Snooze (B1), recent runs, issues.

use rune::{El, TextStyle, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, Kit};

use super::overlays::hint;
use super::{chip, empty, muted, outcome_color, panel, pending, scroll, status_mark};
use crate::model::{Hit, Outcome, ago_str, duration, now};
use crate::State;

/// Green under 4 h, amber under a day, red after.
fn wait_tone(secs: i64) -> Outcome {
    match secs {
        s if s < 4 * 3600 => Outcome::Success,
        s if s < 86_400 => Outcome::Running,
        _ => Outcome::Failure,
    }
}

fn waited(secs: i64) -> String {
    match secs {
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

fn group_title(k: &Kit, label: String, tone: Outcome) -> El {
    row().w_full().gap(6.0).pt(6.0).pb(2.0).px(6.0).center_y().child(rune::el().size(8.0, 8.0).radius(4.0).fill(outcome_color(k, tone))).child(text(label).font(k.ty.small_medium).color(k.c.text_muted))
}

fn ci_outcome(state: &str) -> Option<Outcome> {
    match state {
        "SUCCESS" => Some(Outcome::Success),
        "FAILURE" | "ERROR" => Some(Outcome::Failure),
        "PENDING" | "EXPECTED" => Some(Outcome::Running),
        _ => None,
    }
}

pub fn view(s: &State, k: &Kit) -> El {
    if let Some(p) = pending(k, &s.inbox) {
        return col().w_full().h_full().padding(24.0).child(p);
    }
    let Some(inbox) = s.inbox.data.as_ref() else { return empty(k, "") };
    let t = now();
    let failing = inbox.runs.iter().filter(|(_, r)| r.outcome() == Outcome::Failure).count();
    let notifs = s.visible_notifs();
    let queue = s.review_queue();

    let tile = |key: &str, label: &str, n: usize, tone: Option<Outcome>, tip: &str| {
        let color = match tone {
            Some(o) if n > 0 => outcome_color(k, o),
            _ => k.c.text_strong,
        };
        col()
            .key(key.to_owned())
            .grow(1.0)
            .min_w(170.0)
            .gap(4.0)
            .padding(16.0)
            .radius(k.radius.lg)
            .fill(k.c.layer_01)
            .border(1.0, k.c.border_weak)
            .help(tip.to_owned())
            .child(muted(k, label))
            .child(text(n.to_string()).font(TextStyle { size: 28.0, ..k.ty.ui_medium }).color(color))
    };
    let tiles = row()
        .key("tiles")
        .w_full()
        .gap(12.0)
        .flex_wrap()
        .child(tile("t-review", "Waiting for your review", queue.len(), Some(Outcome::Running), "PRs where someone asked for your review"))
        .child(tile("t-notif", "Unread notifications", notifs.len(), Some(Outcome::Running), "GitHub notifications: mentions, comments, CI, review requests"))
        .child(tile("t-mine", "Your open pull requests", inbox.my_prs.len(), None, "PRs you opened that are still open"))
        .child(tile("t-issues", "Issues assigned to you", inbox.issues.len(), None, "Open issues assigned to you, in any repo"))
        .child(tile("t-failing", "Failing runs (recent)", failing, Some(Outcome::Failure), "Failed workflow runs in your most active repos"));

    // B2: the review queue.
    let mut q = col().w_full().gap(2.0);
    if queue.is_empty() {
        q = q.child(empty(k, "Nothing waiting on you 🎉"));
    }
    let sel = s.queue_sel.min(queue.len().saturating_sub(1));
    let ready = queue.iter().filter(|(i, _)| s.queue_ci(*i).reviewable()).count();
    if !queue.is_empty() {
        q = q.child(group_title(k, format!("Ready for review: all checks passed ({ready})"), Outcome::Success));
        if ready == 0 {
            q = q.child(muted(k, "None yet: no PR waiting on you has all its checks passed").padding(6.0));
        }
    }
    for (pos, (i, secs)) in queue.iter().enumerate() {
        if pos == ready {
            q = q.child(group_title(k, format!("Not ready yet: checks failing, running or not run ({})", queue.len() - ready), Outcome::Failure));
        }
        let h = &inbox.review_requested[*i];
        let info = s.queue.get(&(h.repository.name_with_owner.clone(), h.number));
        let mut r = hit_row(k, h, true, t, pos, pos == sel);
        let mut extra = row().gap(6.0).center_y();
        extra = extra.child(super::ci_chip(k, s.queue_ci(*i)));
        if let Some(i) = info {
            extra = extra.child(muted(k, format!("+{} −{}", i.additions, i.deletions)).help("Size of the change"));
        }
        extra = extra.child(chip(k, format!("waiting {}", waited(*secs)), Some(wait_tone(*secs))).help("How long since your review was requested"));
        r = r.child(extra);
        q = q.child(r);
    }
    let queue_panel = panel(
        k,
        "review",
        "Review queue",
        Some(
            row()
                .gap(8.0)
                .center_y()
                .child(muted(k, "J/K move · Enter open · A approve · C comment"))
                .child(hint(k, "h-queue", "PRs whose checks all passed come first (the ones worth reviewing), oldest wait first. Keys work when no text box has focus.", 2)),
        ),
        q,
    );

    // Your PRs and who you're waiting on.
    let mut mine = col().w_full().gap(2.0);
    if inbox.my_prs.is_empty() {
        mine = mine.child(empty(k, "No open pull requests"));
    }
    for (i, h) in inbox.my_prs.iter().enumerate() {
        let info = s.queue.get(&(h.repository.name_with_owner.clone(), h.number));
        let mut r = hit_row(k, h, true, t, 1000 + i, false);
        if let Some(info) = info {
            if let Some(o) = ci_outcome(&info.ci) {
                r = r.child(status_mark(k, o).help(format!("CI: {}", info.ci.to_lowercase())));
            }
            if !info.waiting_on.is_empty() {
                let since = (t - info.requested_at).max(0);
                r = r.child(chip(k, format!("waiting on {} · {}", info.waiting_on.join(", "), waited(since)), Some(wait_tone(since))));
            }
        }
        mine = mine.child(r);
    }

    // B1: notifications.
    let mut nl = col().w_full().gap(2.0);
    if notifs.is_empty() {
        nl = nl.child(empty(k, "All caught up"));
    }
    for (i, n) in notifs.iter().take(25).enumerate() {
        let (n1, n2, n3, n4) = ((*n).clone(), n.id.clone(), n.id.clone(), n.id.clone());
        let reason = n.reason.replace('_', " ");
        nl = nl.child(
            row()
                .key(format!("n-{i}"))
                .w_full()
                .gap(8.0)
                .px(8.0)
                .py(5.0)
                .radius(k.radius.md)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .gap(2.0)
                        .radius(k.radius.md)
                        .hover(k.c.hover)
                        .on_press_tx(move |s: &mut State, tx| s.open_notif(&n1, tx))
                        .help("Open it here")
                        .child(text(n.title.clone()).font(k.ty.ui).color(k.c.text_base).truncate())
                        .child(text(format!("{} · {} · {}", n.repo.split_once('/').map_or(n.repo.as_str(), |(_, r)| r), reason, ago_str(&n.updated_at, t))).font(k.ty.text_xsmall).color(k.c.text_faint).truncate()),
                )
                .child(Button::new("1h").variant(ButtonVariant::GhostMuted).help("Snooze for an hour (on this computer only)").on_press(move |s: &mut State| s.notif_snooze(n2.clone(), 3600)).build(k).key("s1"))
                .child(Button::new("Tomorrow").variant(ButtonVariant::GhostMuted).help("Snooze until tomorrow (on this computer only)").on_press(move |s: &mut State| s.notif_snooze(n3.clone(), 18 * 3600)).build(k).key("s2"))
                .child(Button::new("Done").variant(ButtonVariant::Outline).help("Mark as read on GitHub").on_press(move |s: &mut State| s.notif_done(n4.clone())).build(k).key("done")),
        );
    }

    let runs_body = if inbox.runs.is_empty() {
        empty(k, "No recent runs in your active repos")
    } else {
        inbox.runs.iter().take(12).enumerate().fold(col().w_full().gap(2.0), |c, (i, (slug, r))| {
            let (slug2, id) = (slug.clone(), r.database_id);
            c.child(
                row()
                    .key(format!("run-{i}"))
                    .w_full()
                    .gap(10.0)
                    .px(8.0)
                    .py(6.0)
                    .radius(k.radius.md)
                    .center_y()
                    .hover(k.c.hover)
                    .help("Open the run")
                    .on_press(move |s: &mut State| s.open_run(&slug2, id))
                    .child(status_mark(k, r.outcome()))
                    .child(
                        col()
                            .grow(1.0)
                            .shrink()
                            .gap(2.0)
                            .child(text(r.display_title.clone()).font(k.ty.ui).color(k.c.text_base).truncate())
                            .child(text(format!("{} · {} · {}", short_repo(slug), r.workflow_name, r.head_branch)).font(k.ty.text_xsmall).color(k.c.text_faint).truncate()),
                    )
                    .child(muted(k, duration(r.duration(t))))
                    .child(muted(k, ago_str(&r.created_at, t)).w(64.0)),
            )
        })
    };
    let issues = if inbox.issues.is_empty() {
        empty(k, "No issues assigned to you")
    } else {
        inbox.issues.iter().enumerate().fold(col().w_full().gap(2.0), |c, (i, h)| c.child(hit_row(k, h, false, t, 2000 + i, false)))
    };

    let left = col()
        .w_full()
        .gap(12.0)
        .child(queue_panel)
        .child(panel(k, "mine", "Your open pull requests", Some(hint(k, "h-mine", "Your PRs, with their CI and who you're waiting on", 2)), mine))
        .child(panel(k, "issues", "Assigned issues", None, issues));
    let right = col()
        .w_full()
        .gap(12.0)
        .child(panel(k, "notifs", &format!("Notifications ({})", notifs.len()), Some(hint(k, "h-notif", "From GitHub. Done marks read there too; snooze is local.", 1)), nl))
        .child(panel(k, "runs", "Recent workflow runs", Some(muted(k, "your 6 most active repos")), runs_body));

    let mut header = row().w_full().center_y().child(
        col()
            .gap(2.0)
            .child(text("Today").font(TextStyle { size: 22.0, ..k.ty.ui_medium }).color(k.c.text_strong))
            .child(muted(k, "Everything waiting on you, across your repositories")),
    );
    header = header.child(spacer());
    if let Some(e) = &s.inbox.error {
        header = header.child(chip(k, format!("Last refresh failed: {e}"), Some(Outcome::Failure)));
    }
    scroll(
        "inbox-scroll",
        col().key("inbox").w_full().padding(24.0).gap(16.0).child(header).child(tiles).child(super::split::split(s, k, "needs-you", vec![super::split::pane(left, 0.5, 380.0), super::split::pane(right, 0.5, 380.0)])),
    )
}

/// "owner/name" → "name".
fn short_repo(slug: &str) -> &str {
    slug.split_once('/').map_or(slug, |(_, n)| n)
}

fn hit_row(k: &Kit, h: &Hit, is_pr: bool, now: i64, i: usize, selected: bool) -> El {
    let slug = h.repository.name_with_owner.clone();
    let (n, url) = (h.number, h.url.clone());
    let mut line = row().gap(6.0).center_y().child(text(h.title.clone()).font(k.ty.ui).color(k.c.text_base).truncate().shrink());
    if h.is_draft {
        line = line.child(chip(k, "Draft", None));
    }
    let r = row()
        .key(format!("hit-{i}"))
        .w_full()
        .gap(10.0)
        .px(8.0)
        .py(6.0)
        .radius(k.radius.md)
        .center_y()
        .fill(if selected { k.c.surface_hover } else { rune::Color::TRANSPARENT })
        .border(if selected { 1.0 } else { 0.0 }, k.c.border_focus)
        .hover(k.c.hover)
        .child(
            col()
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(line)
                .child(text(format!("{} #{} · {} · updated {}", short_repo(&slug), n, h.author.login, ago_str(&h.updated_at, now))).font(k.ty.text_xsmall).color(k.c.text_faint)),
        );
    if is_pr {
        r.help("Open the pull request").on_press(move |s: &mut State| s.open_pr(&slug, n))
    } else {
        // Issues open on GitHub: this app is about PRs, runs and code.
        r.help("Open on GitHub").on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url))
    }
}
