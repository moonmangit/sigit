//! The Branches pane's "PR route" section: where a feature branch stands on the route's steps
//! (staging, then main), and the buttons that open the next PR.

use rune::{El, TextEdit, col, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, SegmentedControl, Spinner, TextArea, TextInput};

use super::{chip, muted};
use crate::i18n::text;
use crate::model::{Outcome, ago_str, now};
use crate::route::{self, Step, StepState};
use crate::State;

/// The section for `branch` of `slug`, or None where it doesn't apply (a route target, the
/// repo's default branch).
pub fn route_section(s: &State, k: &Kit, slug: &str, branch: &str) -> Option<El> {
    if !s.shows_route(slug, branch) {
        return None;
    }
    let head = route::head_name(branch).to_owned();
    let (targets, _) = route::route_for(&s.prefs, slug);
    let route_btn = Button::new(crate::i18n::t("Route for this repo…")).variant(ButtonVariant::GhostMuted).on_press(|_: &mut State| {}).build(k).key("route-edit");
    let title = text(crate::i18n::t("PR route")).font(k.ty.ui_medium).color(k.c.text_strong);
    let mut c = col().key("route-section").w_full().gap(8.0).child(row().w_full().gap(8.0).center_y().child(title).child(spacer()).child(route_btn));
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
    let load = s.route_prs.get(&(slug.to_owned(), head.clone()));
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
        let age = ago_str(&p.created_at, now());
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
                .child(Button::new(crate::i18n::t("Open PR")).variant(ButtonVariant::Submit).disabled(!pushed).on_press(|s: &mut State| s.submit_pr_draft()).build(k).key("prd-open")),
        );
    Dialog::new("route-pr", tf!("Open PR {head} → {base}", head = d.head, base = d.base)).open(true).fit().width(620.0).on_close(|s: &mut State| s.pr_draft = None).child(body).build(k, layer);
}
