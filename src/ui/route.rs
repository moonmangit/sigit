//! The Branches pane's "PR route" section: where a feature branch stands on the route's steps
//! (staging, then main), and the buttons that open the next PR.

use rune::{El, TextEdit, col, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, SegmentedControl, Spinner, Switch, TextArea, TextInput, glyphs};

use super::{chip, muted, scroll};
use crate::i18n::text;
use crate::model::{Outcome, ago_str, now};
use crate::route::{self, RouteRow, Step, StepState};
use crate::State;

/// The section for `branch` of `slug`, or None where it doesn't apply (a route target, the
/// repo's default branch).
pub fn route_section(s: &State, k: &Kit, slug: &str, branch: &str) -> Option<El> {
    if !s.shows_route(slug, branch) {
        return None;
    }
    let head = route::head_name(branch).to_owned();
    let (targets, from_repo) = route::route_for(&s.prefs, slug);
    let load = s.route_prs.get(&(slug.to_owned(), head.clone()));
    let slug2 = slug.to_owned();
    let route_btn = Button::new(crate::i18n::t("Route for this repo…")).variant(ButtonVariant::GhostMuted).on_press(move |s: &mut State| s.open_route_edit(&slug2)).build(k).key("route-edit");
    let title = text(crate::i18n::t("PR route")).font(k.ty.ui_medium).color(k.c.text_strong);
    let mut header = row().w_full().gap(8.0).center_y().child(title);
    if load.is_some_and(|l| l.loading && l.data.is_some()) {
        header = header.child(Spinner::new().size(12.0).build(k).key("route-reloading"));
    }
    let mut c = col().key("route-section").w_full().gap(8.0).child(header.child(spacer()).child(route_btn));
    if targets.is_empty() && from_repo {
        let slug3 = slug.to_owned();
        return Some(
            c.child(muted(k, crate::i18n::t("This repo's route has no targets")).wrap()).child(
                Button::new(crate::i18n::t("Edit this repo's route"))
                    .variant(ButtonVariant::Outline)
                    .on_press(move |s: &mut State| s.open_route_edit(&slug3))
                    .build(k)
                    .key("route-repo-empty"),
            ),
        );
    }
    if targets.is_empty() {
        return Some(
            c.child(muted(k, crate::i18n::t("Set up a PR route in Settings → Git to open PRs from here")).wrap()).child(
                Button::new(crate::i18n::t("Open Settings"))
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut State| s.open_settings(2))
                    .build(k)
                    .key("route-settings"),
            ),
        );
    }
    let Some(prs) = load.and_then(|l| l.data.as_ref()) else {
        let body = match load.and_then(|l| l.error.clone()) {
            Some(e) => {
                let (slug, head) = (slug.to_owned(), head);
                row()
                    .key("route-error")
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(muted(k, tf!("Couldn't load this branch's PRs: {error}", error = e)).wrap().grow(1.0).shrink())
                    .child(
                        Button::new(crate::i18n::t("Retry"))
                            .variant(ButtonVariant::GhostMuted)
                            .on_press(move |s: &mut State| s.reload_route_prs(&slug, &head))
                            .build(k)
                            .key("route-retry"),
                    )
            }
            None => row().key("route-loading").gap(8.0).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, crate::i18n::t("Loading PRs…"))),
        };
        return Some(c.child(body));
    };
    if let Some(e) = load.and_then(|l| l.error.clone()) {
        let (slug, head) = (slug.to_owned(), head.clone());
        c = c.child(
            row()
                .key("route-error")
                .w_full()
                .gap(8.0)
                .center_y()
                .child(muted(k, tf!("Couldn't load this branch's PRs: {error}", error = e)).wrap().grow(1.0).shrink())
                .child(
                    Button::new(crate::i18n::t("Retry"))
                        .variant(ButtonVariant::GhostMuted)
                        .on_press(move |s: &mut State| s.reload_route_prs(&slug, &head))
                        .build(k)
                        .key("route-retry"),
                ),
        );
    }
    let (steps, off) = route::steps(targets, prs);
    for (i, step) in steps.iter().enumerate() {
        c = c.child(step_row(s, k, slug, &head, &steps, step, i));
    }
    for p in off {
        let (slug, n) = (slug.to_owned(), p.number);
        c = c.child(
            row()
                .key(tf!("route-also-{}", p.number))
                .w_full()
                .gap(6.0)
                .center_y()
                .on_press(move |s: &mut State| s.open_pr(&slug, n))
                .child(muted(k, crate::i18n::t("Also open")))
                .child(muted(k, format!("#{} → {}", p.number, p.base_ref_name))),
        );
    }
    Some(c)
}

#[allow(clippy::too_many_arguments)]
fn step_row(_s: &State, k: &Kit, slug: &str, head: &str, steps: &[Step], step: &Step, i: usize) -> El {
    let mut r = row().key(tf!("route-step-{i}")).w_full().gap(8.0).center_y().child(chip(k, (i + 1).to_string(), None)).child(
        text(step.base.clone()).font(k.ty.ui_medium).color(k.c.text_base),
    );
    if let Some(p) = &step.latest {
        let (word, tone) = match step.state {
            StepState::Open { draft: true } => (crate::i18n::t("draft"), Some(Outcome::Running)),
            StepState::Open { draft: false } => (crate::i18n::t("open"), Some(Outcome::Running)),
            StepState::Merged => (crate::i18n::t("merged"), Some(Outcome::Success)),
            _ => (crate::i18n::t("closed"), Some(Outcome::Failure)),
        };
        let mut label = format!("#{}", p.number);
        let when = if matches!(step.state, StepState::Merged) && !p.merged_at.is_empty() { &p.merged_at } else { &p.created_at };
        let age = ago_str(when, now());
        if !age.is_empty() {
            label = format!("{label} · {age}");
        }
        let (slug2, n) = (slug.to_owned(), p.number);
        let state = row()
            .key(tf!("route-pr-{i}"))
            .gap(6.0)
            .center_y()
            .on_press(move |s: &mut State| s.open_pr(&slug2, n))
            .child(chip(k, word, tone))
            .child(muted(k, label));
        r = r.child(state);
        if step.count > 1 {
            r = r.child(muted(k, tf!("({n} PRs)", n = step.count)));
        }
    } else {
        r = r.child(muted(k, crate::i18n::t("no PR yet")));
    }
    r = r.child(spacer());
    let (slug, head) = (slug.to_owned(), head.to_owned());
    let free = matches!(step.state, StepState::None | StepState::Closed);
    if route::can_open(steps, i) {
        let base = step.base.clone();
        r.child(
            Button::new(tf!("Open PR → {base}"))
                .variant(ButtonVariant::Submit)
                .on_press(move |s: &mut State| s.open_pr_dialog(&slug, &head, i))
                .build(k)
                .key(tf!("route-open-{i}")),
        )
    } else if free {
        let prev = route::blocked_by(steps, i).unwrap_or_default();
        r.child(muted(k, tf!("after {prev} is merged"))).child(
            Button::new(crate::i18n::t("Open anyway"))
                .variant(ButtonVariant::GhostMuted)
                .on_press(move |s: &mut State| s.open_pr_dialog(&slug, &head, i))
                .build(k)
                .key(tf!("route-anyway-{i}")),
        )
    } else {
        r
    }
}

/// One field of the open dialog. Edits only reach an open dialog; one landing after it closed
/// goes nowhere.
fn field(s: &mut State, f: fn(&mut crate::PrDraft) -> &mut TextEdit) -> &mut TextEdit {
    match s.pr_draft.as_mut() {
        Some(d) => f(d),
        None => Box::leak(Box::new(TextEdit::new(""))),
    }
}

/// "Open PR feat-a → main": the preset's fields, editable; Open PR hands the PR to the guard.
pub fn pr_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(d) = &s.pr_draft else { return };
    let pushed = s.pushed_state(&d.slug, &d.head) != Some(false);
    let note = if d.from_repo { tf!("preset: {base} (this repo)", base = d.base) } else { tf!("preset: {base} (general)", base = d.base) };
    let input = |e: &TextEdit, label: &str, key: &str, f: fn(&mut crate::PrDraft) -> &mut TextEdit| {
        TextInput::new(e).label(crate::i18n::t(label)).width(None).on_edit(move |s: &mut State| field(s, f)).build(k).key(key.to_owned())
    };
    let mut body = col().w_full().px(16.0).pb(16.0).gap(12.0).child(muted(k, note)).child(
        SegmentedControl::new(&crate::i18n::all(&["Ready for review", "Draft"]), Some(usize::from(d.draft)))
            .fit()
            .on_pick(|s: &mut State, i, _| {
                if let Some(d) = s.pr_draft.as_mut() {
                    d.draft = i == 1;
                }
            })
            .build(k)
            .key("prd-draft"),
    );
    if !pushed {
        body = body.child(muted(k, tf!("Push {head} to GitHub first", head = d.head)).wrap().key("prd-unpushed"));
    }
    let body = body
        .child(input(&d.title, "Title", "prd-title", |d| &mut d.title))
        .child(text(crate::i18n::t("Body")).font(k.ty.small_medium).color(k.c.text_base))
        .child(TextArea::new(&d.body).rows(5).width(None).on_edit(|s: &mut State| field(s, |d| &mut d.body)).build(k).key("prd-body"))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(row().w(0.0).grow(1.0).child(input(&d.reviewers, "Reviewers", "prd-reviewers", |d| &mut d.reviewers)))
                .child(row().w(0.0).grow(1.0).child(input(&d.assignees, "Assignees", "prd-assignees", |d| &mut d.assignees))),
        )
        .child(input(&d.labels, "Labels", "prd-labels", |d| &mut d.labels))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.pr_draft = None).build(k).key("prd-cancel"))
                .child(Button::new(crate::i18n::t("Open PR")).variant(ButtonVariant::Submit).disabled(!pushed || d.title.text.trim().is_empty()).on_press(|s: &mut State| s.submit_pr_draft()).build(k).key("prd-open")),
        );
    Dialog::new("route-pr", tf!("Open PR {head} → {base}", head = d.head, base = d.base)).open(true).fit().width(620.0).on_close(|s: &mut State| s.pr_draft = None).child(body).build(k, layer);
}

type Rows = fn(&mut State) -> &mut Vec<RouteRow>;
type Cell = fn(&mut RouteRow) -> &mut TextEdit;

/// A field of route row `i` in the editor `edit` points at. Edits reach only a row that exists.
fn cell(s: &mut State, edit: Rows, i: usize, f: Cell) -> &mut TextEdit {
    match edit(s).get_mut(i) {
        Some(r) => f(r),
        None => Box::leak(Box::new(TextEdit::new(""))),
    }
}

/// The route's steps as editable cards (target branch, Draft, the preset's fields, and the
/// buttons that move or remove it) and "Add target". `edit` says which rows of the state it
/// edits, so Settings and the per-repo dialog share it.
pub fn route_editor(_s: &State, k: &Kit, rows: &[RouteRow], edit: Rows, pfx: &str) -> El {
    let n = rows.len();
    let mut list = col().w_full().gap(10.0);
    for (i, r) in rows.iter().enumerate() {
        let input = |e: &TextEdit, label: &str, key: &str, f: Cell| {
            TextInput::new(e).label(crate::i18n::t(label)).width(None).on_edit(move |s: &mut State| cell(s, edit, i, f)).build(k).key(tf!("{pfx}-{key}-{i}"))
        };
        let icon = |g: &'static rune::Icon, label: &str, off: bool, key: &str, press: fn(&mut State, Rows, usize)| {
            IconButton::new(g)
                .variant(IconButtonVariant::GhostMuted)
                .label(crate::i18n::t(label))
                .help(crate::i18n::t(label))
                .disabled(off)
                .on_press(move |s: &mut State| press(s, edit, i))
                .build(k)
                .key(tf!("{pfx}-{key}-{i}"))
        };
        let draft = Switch::new(r.draft)
            .label(crate::i18n::t("Draft"))
            .hide_label()
            .on_toggle(move |s: &mut State| {
                if let Some(r) = edit(s).get_mut(i) {
                    r.draft ^= true;
                }
            })
            .build(k)
            .key(tf!("{pfx}-draft-{i}"));
        let card = col()
            .key(tf!("{pfx}-row-{i}"))
            .w_full()
            .gap(8.0)
            .px(12.0)
            .py(12.0)
            .radius(k.radius.md)
            .border(1.0, k.c.border_weak)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(chip(k, (i + 1).to_string(), None))
                    .child(row().w(0.0).grow(1.0).child(input(&r.base, "Target branch", "route-base", |r| &mut r.base)))
                    .child(muted(k, crate::i18n::t("Draft")))
                    .child(draft)
                    .child(icon(&glyphs::CHEVRON_UP, "Move up", i == 0, "route-up", |s, e, i| move_row(s, e, i, -1)))
                    .child(icon(&glyphs::CHEVRON_DOWN, "Move down", i + 1 == n, "route-down", |s, e, i| move_row(s, e, i, 1)))
                    .child(icon(&glyphs::CLOSE, "Remove", false, "route-remove", |s, e, i| {
                        if i < e(s).len() {
                            e(s).remove(i);
                        }
                    })),
            )
            .child(input(&r.title, "Title", "route-title", |r| &mut r.title))
            .child(text(crate::i18n::t("Body")).font(k.ty.small_medium).color(k.c.text_base))
            .child(TextArea::new(&r.body).rows(3).width(None).on_edit(move |s: &mut State| cell(s, edit, i, |r| &mut r.body)).build(k).key(tf!("{pfx}-body-{i}")))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .child(row().w(0.0).grow(1.0).child(input(&r.reviewers, "Reviewers", "route-reviewers", |r| &mut r.reviewers)))
                    .child(row().w(0.0).grow(1.0).child(input(&r.assignees, "Assignees", "route-assignees", |r| &mut r.assignees))),
            )
            .child(input(&r.labels, "Labels", "route-labels", |r| &mut r.labels));
        list = list.child(card);
    }
    list.child(
        row().w_full().child(
            Button::new(crate::i18n::t("Add target"))
                .variant(ButtonVariant::Outline)
                .on_press(move |s: &mut State| edit(s).push(RouteRow::blank()))
                .build(k)
                .key(tf!("{pfx}-add")),
        ),
    )
}

fn move_row(s: &mut State, edit: Rows, i: usize, by: isize) {
    let v = edit(s);
    if let (Some(j), true) = (i.checked_add_signed(by).filter(|j| *j < v.len()), i < v.len()) {
        v.swap(i, j);
    }
}

fn dialog_rows(s: &mut State) -> &mut Vec<RouteRow> {
    match s.route_edit.as_mut() {
        Some(e) => &mut e.1,
        None => Box::leak(Box::new(Vec::new())),
    }
}

/// "Route for owner/repo": the repo's own route, which replaces the general one.
pub fn route_edit_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some((slug, rows)) = &s.route_edit else { return };
    let own = route::route_for(&s.prefs, slug).1;
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(12.0)
        .child(muted(k, crate::i18n::t("This replaces the general route for this repo. {branch}, {target} and {subject} fill in.")).wrap())
        .child(col().w_full().h(440.0).child(scroll("route-edit-scroll", route_editor(s, k, rows, dialog_rows, "dlg-route"))))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(Button::new(crate::i18n::t("Use the general route")).variant(ButtonVariant::GhostMuted).disabled(!own).on_press(|s: &mut State| s.drop_route_edit()).build(k).key("route-use-general"))
                .child(spacer())
                .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.route_edit = None).build(k).key("route-edit-cancel"))
                .child(Button::new(crate::i18n::t("Save")).variant(ButtonVariant::Submit).on_press(|s: &mut State| s.save_route_edit()).build(k).key("route-edit-save")),
        );
    Dialog::new("route-edit", tf!("Route for {slug}", slug = slug)).open(true).fit().width(660.0).on_close(|s: &mut State| s.route_edit = None).child(body).build(k, layer);
}
