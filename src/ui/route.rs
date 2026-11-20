//! The Branches pane's "PR route" section: where a feature branch stands on the route's steps
//! (staging, then main), and the buttons that open the next PR.

use rune::{El, col, row, spacer};
use rune_kit::{Button, ButtonVariant, Kit, Spinner};

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
    // The branch list says whether it's on GitHub; until it's loaded, don't claim it isn't.
    let unpushed = s.data.get(slug).is_some_and(|d| d.branches.data.is_some()) && !s.is_pushed(slug, &head);
    for (i, step) in steps.iter().enumerate() {
        c = c.child(step_row(s, k, slug, &head, &steps, step, i, unpushed));
    }
    if unpushed {
        c = c.child(muted(k, crate::i18n::t("Push this branch to open a PR")));
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
fn step_row(_s: &State, k: &Kit, slug: &str, head: &str, steps: &[Step], step: &Step, i: usize, unpushed: bool) -> El {
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
    if unpushed {
        return r;
    }
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
