//! History editing (`history.rs`): the Commits pane's Edit history section, the preview in the
//! guard's dialog, and the Reword… dialog. Every button simulates first; nothing changes until
//! the dialog's confirm, and the preflight runs again then.

use rune::{El, col, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, TextArea};

use super::muted;
use crate::history::{self, Op, Plan, ResetMode};
use crate::i18n::text;
use crate::model::Commit;
use crate::{CommitGraph, State};

/// Whether `sha` is on the current branch, from the loaded graph: walk the parents from the
/// commit HEAD points at. (Commits past the loaded page count as not on it.)
pub fn on_head(g: &CommitGraph, sha: &str) -> bool {
    let Some(head) = g.commits.iter().find(|c| c.refs.iter().any(|r| r.starts_with("HEAD -> "))) else { return false };
    let by_sha: std::collections::HashMap<&str, &Commit> = g.commits.iter().map(|c| (c.sha.as_str(), c)).collect();
    let mut todo = vec![head.sha.as_str()];
    let mut seen = std::collections::HashSet::new();
    while let Some(c) = todo.pop() {
        if c == sha {
            return true;
        }
        if seen.insert(c)
            && let Some(commit) = by_sha.get(c)
        {
            todo.extend(commit.parents.iter().map(String::as_str));
        }
    }
    false
}

/// The HEAD commit's SHA in the loaded graph.
fn head_sha(g: &CommitGraph) -> Option<&str> {
    g.commits.iter().find(|c| c.refs.iter().any(|r| r.starts_with("HEAD -> "))).map(|c| c.sha.as_str())
}

/// One button that simulates `op`. `help`: what it does, in a sentence.
fn op_button(s: &State, k: &Kit, key: &str, label: &str, help: &str, op: Op) -> El {
    let busy = s.busy.contains(&history::sim_label(&op));
    let any_busy = s.busy.iter().any(|b| b.starts_with("Simulating: "));
    Button::new(crate::i18n::t(if busy { "Simulating…" } else { label }))
        .variant(ButtonVariant::Outline)
        .disabled(any_busy || s.pending.is_some())
        .help(crate::i18n::t(tf!("{} Simulates it first: nothing changes until you confirm.", crate::i18n::t(help))))
        .on_press(move |s: &mut State| s.simulate_history(op.clone()))
        .build(k)
        .key(key.to_owned())
}

/// The Edit history section of a commit's pane.
pub fn actions(s: &State, k: &Kit, g: &CommitGraph, c: &Commit) -> Option<El> {
    let branch = s.rd().and_then(|d| d.status.data.as_ref()).map(|st| st.branch.clone()).filter(|b| !b.is_empty() && b != "(detached)")?;
    s.local_path()?;
    let sha = c.sha.clone();
    let here = on_head(g, &sha);
    let is_head = head_sha(g) == Some(sha.as_str());
    let merge = c.parents.len() > 1;
    let root = c.parents.is_empty();
    let mut ops = row().gap(6.0).flex_wrap();
    if !is_head {
        for (mode, label, help) in [
            (ResetMode::Soft, "Reset (soft)", "Move the branch here; the changes after it stay staged."),
            (ResetMode::Mixed, "Reset (mixed)", "Move the branch here; the changes after it stay in your files, unstaged."),
            (ResetMode::Hard, "Reset (hard)", "Move the branch here and make your files match it."),
        ] {
            ops = ops.child(op_button(s, k, &tf!("h-reset-{}", mode.word()), label, help, Op::Reset { to: sha.clone(), mode }));
        }
    }
    if here {
        if !root {
            ops = ops.child(op_button(s, k, "h-revert", "Revert", "A new commit on top that undoes this one.", Op::Revert { sha: sha.clone() }));
        }
        if !merge && !root {
            // The full message comes with the commit's detail: wait for it, so the body is kept.
            let message = s.commit_detail.data.as_ref().map(|d| d.body.clone());
            let loaded = message.is_some();
            let message = message.unwrap_or_default();
            let id = sha.clone();
            ops = ops.child(
                Button::new(crate::i18n::t("Reword…"))
                    .variant(ButtonVariant::Outline)
                    .disabled(s.pending.is_some() || !loaded)
                    .help(crate::i18n::t("Change this commit's message; the commits after it are made again."))
                    .on_press(move |s: &mut State| s.start_reword(id.clone(), message.clone()))
                    .build(k)
                    .key("h-reword"),
            );
            ops = ops
                .child(op_button(s, k, "h-drop", "Drop", "Remove this commit; the commits after it are made again.", Op::Drop { sha: sha.clone() }))
                .child(op_button(s, k, "h-squash", "Squash into parent", "Join it with the commit before it, keeping both messages.", Op::Squash { sha: sha.clone(), fixup: false }))
                .child(op_button(s, k, "h-fixup", "Fixup into parent", "Join it with the commit before it, keeping that commit's message.", Op::Squash { sha: sha.clone(), fixup: true }));
            if !is_head {
                ops = ops.child(op_button(s, k, "h-up", "Move up", "Swap it with the newer commit after it.", Op::Move { sha: sha.clone(), up: true }));
            }
            ops = ops.child(op_button(s, k, "h-down", "Move down", "Swap it with the older commit before it.", Op::Move { sha: sha.clone(), up: false }));
        }
    } else if !merge && !root {
        ops = ops.child(op_button(s, k, "h-pick", &tf!("Cherry-pick onto {branch}"), "A copy of this commit on top of your branch.", Op::CherryPick { sha: sha.clone() }));
    }
    let title = col()
        .w_full()
        .gap(2.0)
        .child(text(tf!("Edit history of {branch}")).font(k.ty.small_medium).color(k.c.text_muted).truncate().w_full())
        .child(text(crate::i18n::t("Simulated first · Undo · never pushes")).font(k.ty.small).color(k.c.text_faint).wrap());
    Some(col().key("h-actions").w_full().gap(8.0).child(title).child(ops))
}

/// Amend, for the Commits toolbar: put the staged changes into the last commit.
pub fn amend_button(s: &State, k: &Kit) -> Option<El> {
    s.local_path()?;
    Some(op_button(s, k, "h-amend", "Amend last commit", "Put your staged changes into the last commit.", Op::Amend))
}

/// A list of commits under a heading, for the preview.
fn commit_list(k: &Kit, key: &str, title: String, lines: &[history::Line]) -> El {
    let mut c = col().key(key.to_owned()).w_full().gap(3.0).child(text(title).font(k.ty.small_medium).color(k.c.text_base));
    for (i, l) in lines.iter().take(6).enumerate() {
        c = c.child(
            row()
                .key(tf!("{key}-{i}"))
                .w_full()
                .gap(8.0)
                .child(text(l.sha.clone()).font(k.ty.mono).color(k.c.text_muted))
                .child(text(l.subject.clone()).font(k.ty.small).color(k.c.text_base).truncate().shrink().min_w(0.0)),
        );
    }
    if lines.len() > 6 {
        c = c.child(muted(k, tf!("…and {} more", lines.len() - 6)));
    }
    c
}

/// The simulated outcome, at the top of the guard's dialog.
pub fn preview(k: &Kit, plan: &Plan) -> El {
    let short = |s: &str| s.get(..7).unwrap_or(s).to_owned();
    let b = &plan.snap.branch;
    let mut c = col().key("h-preview").w_full().gap(10.0).padding(12.0).radius(k.radius.md).fill(k.c.bg_base).border(1.0, k.c.border_weak);
    c = c.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(text(crate::i18n::t("Simulated outcome")).font(k.ty.ui_medium).color(k.c.text_strong))
            .child(spacer())
            .child(text(tf!("{b}: {} → {}", short(&plan.snap.tip), short(&plan.new_tip))).font(k.ty.mono).color(k.c.text_muted)),
    );
    if !plan.removed.is_empty() {
        c = c.child(commit_list(k, "h-removed", { let n = plan.removed.len(); tf!("Leave {b} ({n})") }, &plan.removed));
    }
    if !plan.added.is_empty() {
        c = c.child(commit_list(k, "h-added", { let n = plan.added.len(); tf!("New on {b} ({n})") }, &plan.added));
    }
    if !plan.replaced_files.is_empty() {
        let list = plan.replaced_files.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
        c = c.child(text(tf!("Uncommitted files replaced: {list}")).font(k.ty.small).color(k.c.danger.fg).wrap());
    }
    // The last line is the summary: keep it, and at most 8 file lines before it.
    let stat = match plan.stat.lines().collect::<Vec<_>>().as_slice() {
        [] => crate::i18n::t("No file changes between the old and the new tip"),
        all if all.len() > 9 => format!("{}\n…\n{}", all[..8].join("\n"), all[all.len() - 1]),
        all => all.join("\n"),
    };
    c = c.child(text(crate::i18n::t("Files, old tip → new tip")).font(k.ty.small_medium).color(k.c.text_base)).child(
        text(stat).font(k.ty.mono).color(k.c.text_muted).wrap().selectable(true).w_full(),
    );
    c.child(muted(k, "Before it runs, sigit checks again that the branch, its upstream and the remote didn't change (preflight). If one did, nothing changes.").wrap())
}

/// Reword…: edit the message, then simulate.
pub fn reword_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some((sha, msg)) = &s.reword else { return };
    let empty = msg.text.trim().is_empty();
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(12.0)
        .child(TextArea::new(msg).rows(8).width(None).on_edit(|s: &mut State| &mut s.reword.get_or_insert_with(|| (String::new(), rune::TextEdit::new(""))).1).build(k).key("rw-msg"))
        .child(muted(k, "The first line is the subject. The commits after this one are made again with new SHAs.").wrap())
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.reword = None).build(k).key("rw-no"))
                .child(Button::new(crate::i18n::t("Simulate")).variant(ButtonVariant::Submit).disabled(empty).help(crate::i18n::t("Shows the outcome first: nothing changes until you confirm")).on_press(|s: &mut State| s.submit_reword()).build(k).key("rw-go")),
        );
    let title = tf!("Reword {}", sha.get(..7).unwrap_or(sha));
    Dialog::new("reword", crate::i18n::t(title)).open(true).fit().width(560.0).on_close(|s: &mut State| s.reword = None).child(body).build(k, layer);
}
