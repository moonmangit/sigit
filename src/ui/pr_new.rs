//! The "New pull request" form: From and To, Ready or Draft, title, body, people and labels,
//! with the helpers (comparison, fill buttons, blocked reason). Create PR hands it to the guard.

use rune::{El, TextEdit, col, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, SegmentedControl, Select, Spinner, TextArea, TextInput};

use super::muted;
use crate::i18n::text;
use crate::State;

/// One field of the open form. Edits only reach an open form; one landing after it closed
/// goes nowhere.
fn field(s: &mut State, f: fn(&mut crate::PrDraft) -> &mut TextEdit) -> &mut TextEdit {
    match s.pr_draft.as_mut() {
        Some(d) => f(d),
        None => Box::leak(Box::new(TextEdit::new(""))),
    }
}

/// The repo's branch names, from GitHub and the clone, each once, without `origin/`, A-Z.
fn branch_names(s: &State, slug: &str) -> Vec<String> {
    s.pr_branch_names(slug).unwrap_or_default()
}

/// "3 commits ahead", a spinner while the comparison loads, nothing if it failed.
fn comparison_line(s: &State, k: &Kit, d: &crate::PrDraft) -> Option<El> {
    if d.from.is_empty() || d.to.is_empty() || d.from == d.to {
        return None;
    }
    if let Some(n) = s.pr_ahead() {
        let line = if n == 1 { crate::i18n::t("1 commit ahead") } else { tf!("{n} commits ahead") };
        return Some(muted(k, line).key("prd-ahead"));
    }
    let loading = s.pr_compare.get(&(d.slug.clone(), d.from.clone(), d.to.clone())).is_some_and(|l| l.loading);
    loading.then(|| row().key("prd-loading").gap(8.0).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, crate::i18n::t("Comparing…"))))
}

/// A labelled branch dropdown; `which` is 1 for From, 2 for To.
fn picker(s: &State, k: &Kit, which: u8, label: &str, current: &str, key: &str) -> El {
    col()
        .w(0.0)
        .grow(1.0)
        .gap(4.0)
        .child(text(crate::i18n::t(label)).font(k.ty.small_medium).color(k.c.text_base))
        .child(
            Select::new(key.to_owned(), &[] as &[&str])
                .label(crate::i18n::t(label))
                .current(Some(current).filter(|c| !c.is_empty()))
                .placeholder(crate::i18n::t("Pick a branch"))
                .open(s.pr_pick_open == which)
                .on_toggle(move |s: &mut State| s.pr_pick_open = if s.pr_pick_open == which { 0 } else { which })
                .trigger(k),
        )
}

/// "New pull request acme/api": the fields, editable; Create PR hands the PR to the guard.
pub fn pr_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(d) = &s.pr_draft else { return };
    let blocked = s.pr_blocked();
    let input = |e: &TextEdit, label: &str, key: &str, f: fn(&mut crate::PrDraft) -> &mut TextEdit| {
        TextInput::new(e).label(crate::i18n::t(label)).width(None).on_edit(move |s: &mut State| field(s, f)).build(k).key(key.to_owned())
    };
    let mut body = col().w_full().px(16.0).pb(16.0).gap(12.0).child(muted(k, d.slug.clone())).child(
        row()
            .w_full()
            .gap(8.0)
            .child(picker(s, k, 1, "From", &d.from, "prd-from"))
            .child(text("→").font(k.ty.small).color(k.c.text_faint))
            .child(picker(s, k, 2, "To", &d.to, "prd-to")),
    );
    if let Some(line) = comparison_line(s, k, d) {
        body = body.child(line);
    }
    let ready = s.pr_ahead().is_some();
    body = body
        .child(
            SegmentedControl::new(&crate::i18n::all(&["Ready for review", "Draft"]), Some(usize::from(d.draft)))
                .fit()
                .on_pick(|s: &mut State, i, _| {
                    if let Some(d) = s.pr_draft.as_mut() {
                        d.draft = i == 1;
                    }
                })
                .build(k)
                .key("prd-draft"),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(row().w(0.0).grow(1.0).child(input(&d.title, "Title", "prd-title", |d| &mut d.title)))
                .child(Button::new(crate::i18n::t("Use last commit")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.fill_pr_title()).build(k).key("prd-fill-title")),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(crate::i18n::t("Body")).font(k.ty.small_medium).color(k.c.text_base))
                .child(spacer())
                .child(
                    Button::new(crate::i18n::t("List commits"))
                        .variant(ButtonVariant::GhostMuted)
                        .disabled(!ready)
                        .on_press(|s: &mut State| s.fill_pr_body())
                        .build(k)
                        .key("prd-fill-body"),
                ),
        )
        .child(TextArea::new(&d.body).rows(5).width(None).on_edit(|s: &mut State| field(s, |d| &mut d.body)).build(k).key("prd-body"))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(row().w(0.0).grow(1.0).child(input(&d.reviewers, "Reviewers", "prd-reviewers", |d| &mut d.reviewers)))
                .child(row().w(0.0).grow(1.0).child(input(&d.assignees, "Assignees", "prd-assignees", |d| &mut d.assignees))),
        )
        .child(input(&d.labels, "Labels", "prd-labels", |d| &mut d.labels));
    if let Some(why) = &blocked {
        let line = match s.pr_already_open() {
            Some(n) => {
                let slug = d.slug.clone();
                Button::new(why.clone())
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |s: &mut State| {
                        s.pr_draft = None;
                        s.open_pr(&slug, n);
                    })
                    .build(k)
                    .key("prd-blocked")
            }
            None => text(why.clone()).font(k.ty.small).color(k.c.warning.fg).wrap().key("prd-blocked"),
        };
        body = body.child(line);
    }
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.pr_draft = None).build(k).key("prd-cancel"))
            .child(Button::new(crate::i18n::t("Create PR")).variant(ButtonVariant::Submit).disabled(blocked.is_some()).on_press(|s: &mut State| s.submit_pr_draft()).build(k).key("prd-open")),
    );
    Dialog::new("pr-form", crate::i18n::t("New pull request")).open(true).fit().width(620.0).on_close(|s: &mut State| s.pr_draft = None).child(body).build(k, layer);
    // The open branch list, anchored to its trigger (built after the dialog so it sits on top).
    let which = s.pr_pick_open;
    if which == 1 || which == 2 {
        let (key, current) = if which == 1 { ("prd-from", &d.from) } else { ("prd-to", &d.to) };
        let names = branch_names(s, &d.slug);
        let _trigger = Select::new(key.to_owned(), &names)
            .label(crate::i18n::t(if which == 1 { "From" } else { "To" }))
            .current(Some(current.as_str()))
            .open(true)
            .on_close(|s: &mut State| s.pr_pick_open = 0)
            .on_choose(move |s: &mut State, v: &str, _| {
                s.pr_pick_open = 0;
                if which == 1 { s.set_pr_from(v) } else { s.set_pr_to(v) }
            })
            .build(k, layer);
    }
}
