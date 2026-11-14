//! Pull requests: a sortable list with check and review state, and a detail pane to read,
//! review, comment, merge, close or check out.

use std::borrow::Cow;

use rune::{El, TextStyle, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, Cell, Kit, Markdown, SegmentedControl, Table, TableColumn, TextArea, TextInput};

use super::{chip, diff_view, diffstat, empty, ext_button, hue, muted, outcome_color, pending, ref_chip, side_pane, status_mark};
use crate::daily::WatchTarget;
use crate::gh::MergeMethod;
use crate::guard::Action;
use crate::risk;
use crate::model::{Outcome, Pr, ago_str, now};
use crate::{PR_STATES, State};

/// The PRs the filter box lets through: loaded ones that match (instant, on every keystroke),
/// then what GitHub's search found beyond them once it answers.
fn visible(s: &State) -> Vec<&Pr> {
    let raw = s.pr_filter.text.trim();
    let q = raw.to_lowercase();
    let mut out: Vec<&Pr> = s
        .rd()
        .and_then(|d| d.prs.data.as_ref())
        .map(|prs| {
            prs.iter()
                .filter(|p| {
                    q.is_empty()
                        || p.title.to_lowercase().contains(&q)
                        || p.author.login.to_lowercase().contains(&q)
                        || p.head_ref_name.to_lowercase().contains(&q)
                        || p.number.to_string() == q.trim_start_matches('#')
                })
                .collect()
        })
        .unwrap_or_default();
    if let (Some(((slug, state, hq), hits)), Some(cur)) = (&s.pr_hits, s.cur.as_deref())
        && slug == cur
        && *state == s.pr_state
        && hq == raw
    {
        for h in hits {
            if !out.iter().any(|p| p.number == h.number) {
                out.push(h);
            }
        }
    }
    out
}

/// A PR by number, from the loaded list or the search results.
fn find(s: &State, n: u64) -> Option<&Pr> {
    let loaded = s.rd().and_then(|d| d.prs.data.as_ref()).and_then(|p| p.iter().find(|p| p.number == n));
    loaded.or_else(|| s.pr_hits.as_ref().and_then(|(_, h)| h.iter().find(|p| p.number == n))).or_else(|| s.focused_pr(n))
}

fn review_label(p: &Pr) -> (&'static str, Option<Outcome>) {
    match p.review_decision.as_str() {
        "APPROVED" => ("Approved", Some(Outcome::Success)),
        "CHANGES_REQUESTED" => ("Changes requested", Some(Outcome::Failure)),
        "REVIEW_REQUIRED" => ("Review required", Some(Outcome::Running)),
        _ => ("—", None),
    }
}

pub fn view(s: &State, k: &Kit) -> El {
    let toolbar = row()
        .key("pr-tools")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            super::with_clear(
                TextInput::new(&s.pr_filter)
                    .placeholder("Filter by title, author, branch or #")
                    .leading_icon(k.glyph.search)
                    .width(Some(320.0))
                    .on_edit(|s: &mut State| &mut s.pr_filter)
                    .on_change(|s: &mut State, _| s.search_prs()),
                &s.pr_filter.text,
                |s| &mut s.pr_filter,
                State::search_prs,
            )
            .build(k)
                .key("pr-filter"),
        )
        .child(
            SegmentedControl::new(&["Open", "Merged", "Closed"], Some(s.pr_state))
                .label("State")
                .fit()
                .on_pick(|s: &mut State, i, _| {
                    s.pr_state = i;
                    s.pr_sel = None;
                    s.refresh_prs();
                })
                .build(k)
                .key("pr-state")
                .help("Which pull requests to list"),
        )
        .child(spacer());
    let toolbar = if s.pr_searching {
        toolbar.child(row().gap(6.0).center_y().child(rune_kit::Spinner::new().size(12.0).build(k)).child(muted(k, "Searching GitHub…")))
    } else {
        toolbar
    };
    let toolbar = toolbar.child(muted(k, format!("{} {}", visible(s).len(), PR_STATES[s.pr_state])));

    let list = match s.rd().and_then(|d| pending(k, &d.prs)) {
        Some(p) => p,
        None if visible(s).is_empty() => empty(k, "No pull requests here"),
        None => {
            // "Load more" pages the unfiltered list; a search already looks past it.
            let loaded = s.rd().and_then(|d| d.prs.data.as_ref()).map_or(0, |p| p.len());
            let mut c = rune::col().w_full().h_full().child(table(s, k));
            if s.pr_filter.text.trim().is_empty() && loaded >= s.pr_limit {
                let loading = s.rd().is_some_and(|d| d.prs.loading);
                c = c.child(super::load_more(k, "pr-more", loaded, crate::PR_PAGE, loading, State::more_prs));
            }
            c
        }
    };
    let mut panes = vec![super::split::pane(col()
            .key("pr-list")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .radius(k.radius.lg)
            .border(1.0, k.c.border_weak)
            .fill(k.c.layer_01)
            .clip_rounded(k.radius.lg)
            .child(list), 0.6, 360.0)];
    if let Some(pane) = detail(s, k) {
        panes.push(super::split::pane(pane, 0.4, 340.0));
    }
    let body = super::split::split(s, k, "prs", panes).key("pr-body").grow(1.0).min_h(0.0);
    col().w_full().h_full().gap(12.0).child(toolbar).child(body)
}

fn table(s: &State, k: &Kit) -> El {
    let kc = k.clone();
    let now = now();
    Table::new(
        |s: &State| &s.prs_table,
        |s: &mut State| &mut s.prs_table,
        |s: &State| visible(s).len(),
        |s: &State, i| Cow::Owned(visible(s).get(i).map_or_else(String::new, |p| p.number.to_string())),
    )
    .name("Pull requests")
    .column(TableColumn::new("num", "#").width(64.0).min(50.0))
    .column(TableColumn::new("title", "Title").flex(3.0).min(220.0))
    .column(TableColumn::new("author", "Author").width(120.0).min(80.0))
    .column(TableColumn::new("checks", "Checks").width(90.0).resizable(false))
    .column(TableColumn::new("review", "Review").width(140.0).min(90.0))
    .column(TableColumn::new("diff", "Changes").width(130.0).resizable(false))
    .column(TableColumn::new("updated", "Updated").width(84.0).resizable(false))
    .cell(move |s: &State, i, col| {
        let v = visible(s);
        let Some(p) = v.get(i) else { return Cell::text("") };
        let k = &kc;
        match col {
            "num" => Cell::text(format!("#{}", p.number)).mono().muted(),
            "title" => {
                let mut r = row().gap(6.0).center_y().min_w(0.0);
                if p.is_draft {
                    r = r.child(chip(k, "Draft", None));
                }
                r = r.child(text(p.title.clone()).font(k.ty.ui).color(k.c.text_base).truncate().shrink());
                let found = risk::sensitive_files(p.files.iter().map(|f| (f.path.as_str(), f.additions + f.deletions)), &s.prefs.sensitive);
                for label in risk::chip_labels(&found) {
                    r = r.child(chip(k, format!("⚠ {label}"), Some(Outcome::Running)).help(format!(
                        "Touches {}: merging it is High risk",
                        found.iter().map(|(p, w)| format!("{p} ({w})")).collect::<Vec<_>>().join(", ")
                    )));
                }
                for l in p.labels.iter().take(2) {
                    let c = rune::color::hex(&format!("#{}", l.color)).unwrap_or(k.c.icon_faint);
                    r = r.child(ref_chip(k, &l.name, c));
                }
                Cell::el(r)
            }
            "author" => Cell::text(p.author.login.clone()).muted(),
            "checks" => {
                let (o, passed, total) = p.checks();
                if total == 0 {
                    Cell::text("—").muted()
                } else {
                    Cell::el(row().gap(6.0).center_y().child(status_mark(k, o)).child(text(format!("{passed}/{total}")).font(k.ty.small).color(k.c.text_muted)))
                }
            }
            "review" => {
                let (label, tone) = review_label(p);
                match tone {
                    Some(_) => Cell::el(chip(k, label, tone)),
                    None => Cell::text(label).muted(),
                }
            }
            "diff" => Cell::el(diffstat(k, p.additions, p.deletions)),
            _ => Cell::text(ago_str(&p.updated_at, now)).muted(),
        }
    })
    .on_select(|s: &mut State, id, _| {
        if let Ok(n) = id.parse() {
            s.select_pr(n);
        }
    })
    .build(s, k)
    .grow(1.0)
    .min_h(0.0)
}

fn detail(s: &State, k: &Kit) -> Option<El> {
    let n = s.pr_sel?;
    let slug = s.cur.clone()?;
    let pr = find(s, n)?.clone();
    let now = now();
    let mut c = col().w_full().gap(14.0);

    // Title and branch flow.
    let state_chip = match (pr.state.as_str(), pr.is_draft) {
        ("MERGED", _) => chip(k, "Merged", Some(Outcome::Success)),
        ("CLOSED", _) => chip(k, "Closed", Some(Outcome::Failure)),
        (_, true) => chip(k, "Draft", None),
        _ => chip(k, "Open", Some(Outcome::Running)),
    };
    c = c.child(
        row()
            .w_full()
            .gap(8.0)
            .items_start()
            .child(text(pr.title.clone()).font(TextStyle { size: 17.0, ..k.ty.ui_medium }).color(k.c.text_strong).wrap().grow(1.0).shrink())
            .child(super::copy_button(k, "pr-copy", pr.url.clone(), "link to this PR"))
            .child(ext_button(k, "pr-open", pr.url.clone())),
    );
    c = c.child(
        row()
            .w_full()
            .gap(6.0)
            .center_y()
            .flex_wrap()
            .child(state_chip)
            .child(muted(k, format!("#{} by {} · updated {}", pr.number, pr.author.login, ago_str(&pr.updated_at, now))))
            .child(ref_chip(k, &pr.head_ref_name, hue(1)).help("The PR's branch (click to copy)"))
            .child(muted(k, "→"))
            .child(super::branch_chip(s, k, &pr.base_ref_name, hue(0)).help("Where it merges into (click to copy)"))
            .child(diffstat(k, pr.additions, pr.deletions)),
    );

    // Actions: every one goes through the guard rails (risk check, confirm, undo).
    let open = pr.state == "OPEN";
    let mut actions = row().w_full().gap(8.0).flex_wrap();
    if open {
        let (s1, s2, s3, s4) = (slug.clone(), slug.clone(), slug.clone(), slug.clone());
        let (t1, t2) = (pr.title.clone(), pr.title.clone());
        actions = actions
            .child(
                Button::new("Approve")
                    .variant(ButtonVariant::Submit)
                    .help("Approve this PR. Anything in the comment box becomes the review's message.")
                    .on_press(move |s: &mut State| {
                        let body = s.comment.text.trim().to_owned();
                        s.comment = rune::TextEdit::new("");
                        s.request(Action::Approve { slug: s1.clone(), number: n, title: String::new(), body, confirm: false });
                    })
                    .build(k)
                    .key("approve"),
            )
            .child(
                Button::new("Request changes")
                    .variant(ButtonVariant::Outline)
                    .help("Block the merge until changes are made. Write what should change in the comment box first.")
                    .on_press(move |s: &mut State| {
                        let body = s.comment.text.trim().to_owned();
                        if body.is_empty() {
                            s.toast("Write what should change in the comment box first".into(), true);
                            s.comment.focused = true;
                            return;
                        }
                        s.comment = rune::TextEdit::new("");
                        s.request(Action::RequestChanges { slug: s2.clone(), number: n, body });
                    })
                    .build(k)
                    .key("changes"),
            )
            .child(
                Button::new("Merge…")
                    .variant(ButtonVariant::Contrast)
                    .help("Checks protection, CI, reviews, sensitive files and secrets first; then a countdown you can cancel")
                    .on_press(move |s: &mut State| {
                        s.request(Action::Merge { slug: s3.clone(), number: n, title: t1.clone(), method: MergeMethod::Squash, delete_branch: true })
                    })
                    .build(k)
                    .key("merge"),
            )
            .child(
                Button::new("Close")
                    .variant(ButtonVariant::Ghost)
                    .help("Close without merging (Undo reopens it)")
                    .on_press(move |s: &mut State| s.request(Action::Close { slug: s4.clone(), number: n, title: t2.clone() }))
                    .build(k)
                    .key("close"),
            );
    }
    if let Some(dir) = s.local_path() {
        let slug = slug.clone();
        actions = actions.child(
            Button::new("Checkout")
                .variant(ButtonVariant::Outline)
                .help("Fetch the PR's branch and switch your local clone to it (gh pr checkout). Asks first if you have uncommitted changes.")
                .on_press(move |s: &mut State| s.request(Action::PrCheckout { dir: dir.clone(), slug: slug.clone(), number: n }))
                .build(k)
                .key("checkout"),
        );
    }
    // B6: watch it.
    let target = WatchTarget::Pr { slug: slug.clone(), number: n };
    let watched = s.is_watched(&target);
    let label = format!("PR #{n}");
    actions = actions.child(
        Button::new(if watched { "Watching" } else { "Watch" })
            .variant(if watched { ButtonVariant::Contrast } else { ButtonVariant::Ghost })
            .icon(&super::I_EYE)
            .help("Get a notification when its checks finish, a review arrives, or it's merged")
            .on_press(move |s: &mut State| s.toggle_watch(target.clone(), label.clone()))
            .build(k)
            .key("watch"),
    );
    c = c.child(actions);

    let files = s.pr_detail.data.as_ref().map_or(pr.changed_files as usize, |d| d.view.files.len());
    let checks = pr.status_check_rollup.len();
    c = c.child(
        SegmentedControl::new(&["Conversation".to_owned(), format!("Files ({files})"), format!("Checks ({checks})")], Some(s.pr_tab))
            .label("Pull request view")
            .fit()
            .on_pick(|s: &mut State, i, _| s.pr_tab = i)
            .build(k)
            .key("pr-tab"),
    );

    if let Some(p) = pending(k, &s.pr_detail) {
        return Some(side_pane(k, "pr-pane", c.child(p)));
    }
    let d = s.pr_detail.data.as_ref()?;
    let found = risk::sensitive_files(d.view.files.iter().map(|f| (f.path.as_str(), f.additions + f.deletions)), &s.prefs.sensitive);
    let secrets = risk::secret_lines(&d.diff);
    if !found.is_empty() || !secrets.is_empty() {
        let mut care = col()
            .key("care")
            .w_full()
            .gap(4.0)
            .padding(12.0)
            .radius(k.radius.md)
            .fill(k.c.warning.bg)
            .child(row().gap(8.0).center_y().child(text("⚠ Why this needs care").font(k.ty.ui_medium).color(k.c.warning.fg)).child(super::overlays::hint(k, "h-care", "Sensitive files and possible secrets. Merging this PR is High risk.", 3)));
        for (path, what) in found.iter().take(8) {
            care = care.child(row().gap(6.0).child(text(what.clone()).font(k.ty.small_medium).color(k.c.text_base)).child(text(path.clone()).font(k.ty.mono).color(k.c.text_muted).truncate().shrink()));
        }
        for (file, line, what) in secrets.iter().take(5) {
            care = care.child(col().gap(1.0).child(text(format!("Possible {what} in {file}")).font(k.ty.small_medium).color(k.c.danger.fg)).child(text(line.clone()).font(k.ty.mono).color(k.c.text_muted).truncate()));
        }
        c = c.child(care);
    }
    match s.pr_tab {
        0 => {
            let body = if d.view.body.trim().is_empty() { "_No description._".to_owned() } else { d.view.body.clone() };
            c = c.child(col().key(format!("body-{n}")).w_full().child(Markdown::new(body).scroll_wide(true).build(k)));
            if !d.view.reviews.is_empty() {
                let reviews = d.view.reviews.iter().enumerate().fold(col().w_full().gap(6.0), |c, (i, r)| {
                    let tone = match r.state.as_str() {
                        "APPROVED" => Some(Outcome::Success),
                        "CHANGES_REQUESTED" => Some(Outcome::Failure),
                        _ => None,
                    };
                    c.child(
                        row()
                            .key(format!("rev-{i}"))
                            .gap(8.0)
                            .center_y()
                            .child(text(r.author.login.clone()).font(k.ty.ui_medium).color(k.c.text_base))
                            .child(chip(k, r.state.replace('_', " ").to_lowercase(), tone))
                            .child(muted(k, ago_str(&r.submitted_at, now))),
                    )
                });
                c = c.child(section(k, "Reviews", reviews));
            }
            for (i, cm) in d.view.comments.iter().enumerate() {
                c = c.child(
                    col()
                        .key(format!("cm-{i}"))
                        .w_full()
                        .gap(6.0)
                        .padding(12.0)
                        .radius(k.radius.md)
                        .fill(k.c.bg_base)
                        .border(1.0, k.c.border_weak)
                        .child(row().gap(8.0).child(text(cm.author.login.clone()).font(k.ty.ui_medium).color(k.c.text_base)).child(muted(k, ago_str(&cm.created_at, now))))
                        .child(Markdown::new(cm.body.clone()).build(k)),
                );
            }
            if open {
                let slug = slug.clone();
                c = c.child(
                    col()
                        .key("composer")
                        .w_full()
                        .gap(8.0)
                        .child(TextArea::new(&s.comment).placeholder("Leave a comment (also the body for Approve / Request changes)").auto_grow(3, 10).width(None).on_edit(|s: &mut State| &mut s.comment).build(k).key("comment-box"))
                        .child(
                            row().w_full().child(spacer()).child(
                                Button::new("Comment")
                                    .variant(ButtonVariant::Outline)
                                    .help("Post the comment on the PR")
                                    .on_press(move |s: &mut State| {
                                        let body = s.comment.text.trim().to_owned();
                                        if body.is_empty() {
                                            return;
                                        }
                                        s.comment = rune::TextEdit::new("");
                                        s.request(Action::Comment { slug: slug.clone(), number: n, body });
                                    })
                                    .build(k)
                                    .key("send"),
                            ),
                        ),
                );
            }
        }
        1 => {
            let list = d.view.files.iter().enumerate().fold(col().w_full().gap(2.0), |c, (i, f)| {
                c.child(
                    row()
                        .key(format!("f-{i}"))
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(super::click_to_copy(text(f.path.clone()).font(k.ty.mono).color(k.c.text_base).truncate().grow(1.0).shrink(), k, f.path.clone(), "file path"))
                        .child(diffstat(k, f.additions, f.deletions)),
                )
            });
            c = c.child(section(k, "Files changed", list)).child(diff_view(k, &format!("diff-{n}"), &d.diff));
        }
        _ => {
            let rollup = if d.view.status_check_rollup.is_empty() { &pr.status_check_rollup } else { &d.view.status_check_rollup };
            if rollup.is_empty() {
                c = c.child(empty(k, "No checks on this pull request"));
            }
            let mut sorted: Vec<_> = rollup.iter().collect();
            sorted.sort_by_key(|ch| ch.outcome());
            for (i, ch) in sorted.into_iter().enumerate() {
                let url = ch.url().to_owned();
                // Actions checks link to /actions/runs/<id>/job/...: triage it here.
                let run_id: Option<u64> = url.split("/actions/runs/").nth(1).and_then(|r| r.split('/').next()).and_then(|r| r.parse().ok());
                let triage = (ch.outcome() == Outcome::Failure).then_some(run_id).flatten().map(|id| {
                    Button::new("Why it failed")
                        .variant(ButtonVariant::Outline)
                        .help("Open the run's failing log lines in the app")
                        .on_press(move |s: &mut State| {
                            if let Some(slug) = s.cur.clone() {
                                s.open_run(&slug, id);
                                s.load_triage(id);
                            }
                        })
                        .build(k)
                        .key(format!("why-{i}"))
                });
                c = c.child(
                    row()
                        .key(format!("ck-{i}"))
                        .w_full()
                        .gap(10.0)
                        .px(8.0)
                        .py(6.0)
                        .radius(k.radius.md)
                        .center_y()
                        .hover(k.c.hover)
                        .on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url))
                        .child(status_mark(k, ch.outcome()))
                        .child(text(ch.title().to_owned()).font(k.ty.ui).color(k.c.text_base).grow(1.0).truncate().shrink())
                        .child(text(ch.workflow_name.clone()).font(k.ty.small).color(outcome_color(k, Outcome::Neutral)))
                        .help("Open this check on GitHub"),
                );
                if let Some(b) = triage {
                    c = c.child(row().w_full().pl(32.0).child(b));
                }
            }
        }
    }
    Some(side_pane(k, "pr-pane", c))
}

fn section(k: &Kit, title: &str, body: El) -> El {
    col().w_full().gap(8.0).child(text(title.to_owned()).font(k.ty.small_medium).color(k.c.text_muted)).child(body)
}
