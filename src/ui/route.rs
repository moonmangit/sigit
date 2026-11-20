//! The "Open PR" dialog: the new pull request's fields, editable; Open PR hands it to the guard.

use rune::{El, TextEdit, col, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, SegmentedControl, TextArea, TextInput};

use super::muted;
use crate::i18n::text;
use crate::State;

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
    let input = |e: &TextEdit, label: &str, key: &str, f: fn(&mut crate::PrDraft) -> &mut TextEdit| {
        TextInput::new(e).label(crate::i18n::t(label)).width(None).on_edit(move |s: &mut State| field(s, f)).build(k).key(key.to_owned())
    };
    let mut body = col().w_full().px(16.0).pb(16.0).gap(12.0).child(
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
    Dialog::new("pr-form", tf!("Open PR {head} → {base}", head = d.head, base = d.base)).open(true).fit().width(620.0).on_close(|s: &mut State| s.pr_draft = None).child(body).build(k, layer);
}
