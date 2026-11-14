//! Branches: Local / Remote (with 🛡 on protected ones and ahead/behind against upstream),
//! Stashes (A5), and Clean up (B7: local branches that can go; remote ones never).
//! Double-click (or Enter) a branch to check it out; every action goes through the guard.

use std::borrow::Cow;

use rune::{El, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, Cell, Kit, SegmentedControl, Table, TableColumn};

use super::overlays::hint;
use super::{branch_chip, chip, empty, hue, muted, pending};
use crate::guard::Action;
use crate::model::{Branch, Outcome, ago, now};
use crate::State;

fn visible(s: &State) -> Vec<&Branch> {
    let remote = s.branch_view == 1;
    s.rd().and_then(|d| d.branches.data.as_ref()).map(|b| b.iter().filter(|b| b.remote == remote).collect()).unwrap_or_default()
}

const VIEWS: [(&str, &str); 4] = [
    ("Local", "Branches in your clone. Double-click to check one out."),
    ("Remote", "Branches on GitHub (origin/…). Double-click makes a local tracking branch."),
    ("Stashes", "Changes you put aside (git stash). Apply keeps the stash, Pop removes it."),
    ("Clean up", "Local branches that are merged, gone upstream, or untouched for 90 days."),
];

pub fn view(s: &State, k: &Kit) -> El {
    let sel = s.branch_sel.clone();
    let mut toolbar = row()
        .key("b-tools")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            SegmentedControl::new(&VIEWS.map(|v| v.0), Some(s.branch_view))
                .label("Branches")
                .fit()
                .on_pick(|s: &mut State, i, _| {
                    s.branch_view = i;
                    s.branch_sel = None;
                    s.branches_table.clear_selection();
                    if i >= 2 {
                        s.refresh_branch_tools();
                    }
                })
                .build(k)
                .key("b-scope")
                .help(VIEWS[s.branch_view].1),
        )
        .child(hint(k, "h-branches", VIEWS[s.branch_view].1, 7));
    if s.branch_view < 2 {
        toolbar = toolbar.child(muted(k, format!("{} branches · double-click to check out · 🛡 = protected", visible(s).len()))).child(spacer());
        if let Some(name) = sel {
            let remote = s.branch_view == 1;
            toolbar = toolbar.child(
                Button::new(format!("Checkout {name}"))
                    .variant(ButtonVariant::Contrast)
                    .help("Switch your clone to it. Asks first if you have uncommitted changes, and offers to stash them.")
                    .on_press(move |s: &mut State| s.checkout_branch(name.clone(), remote))
                    .build(k)
                    .key("b-checkout"),
            );
        }
    }

    let body = match s.branch_view {
        2 => stashes(s, k),
        3 => cleanup(s, k),
        _ => {
            let list = match s.rd().and_then(|d| pending(k, &d.branches)) {
                Some(p) => p,
                None if visible(s).is_empty() => empty(k, "No branches"),
                None => table(s, k),
            };
            col().key("b-list").w_full().grow(1.0).min_h(0.0).radius(k.radius.lg).border(1.0, k.c.border_weak).fill(k.c.layer_01).clip_rounded(k.radius.lg).child(list)
        }
    };
    col().w_full().h_full().gap(12.0).child(toolbar).child(body)
}

fn table(s: &State, k: &Kit) -> El {
    let kc = k.clone();
    let now = now();
    Table::new(
        |s: &State| &s.branches_table,
        |s: &mut State| &mut s.branches_table,
        |s: &State| visible(s).len(),
        |s: &State, i| Cow::Owned(visible(s).get(i).map_or_else(String::new, |b| b.name.clone())),
    )
    .name("Branches")
    .column(TableColumn::new("name", "Branch").flex(1.5).min(180.0))
    .column(TableColumn::new("track", "Behind / ahead").width(170.0).resizable(false))
    .column(TableColumn::new("subject", "Last commit").flex(2.0).min(200.0))
    .column(TableColumn::new("author", "Author").width(140.0).min(80.0))
    .column(TableColumn::new("when", "Updated").width(90.0).resizable(false))
    .cell(move |s: &State, i, col| {
        let v = visible(s);
        let Some(b) = v.get(i) else { return Cell::text("") };
        let k = &kc;
        match col {
            "name" => {
                let mut r = row().gap(6.0).center_y();
                if b.current {
                    r = r.child(chip(k, "current", Some(Outcome::Success)).help("Your clone is on this branch"));
                }
                if b.gone {
                    r = r.child(chip(k, "gone", Some(Outcome::Neutral)).help("Its remote branch was deleted (usually after its PR merged): a clean-up candidate"));
                }
                Cell::el(r.child(branch_chip(s, k, &b.name, if b.remote { hue(3) } else { hue(1) })))
            }
            "track" => Cell::el(track(k, b)),
            "subject" => Cell::text(b.subject.clone()).muted(),
            "author" => Cell::text(b.author.clone()).muted(),
            _ => Cell::text(ago(b.time, now)).muted(),
        }
    })
    .on_select(|s: &mut State, name, _| s.branch_sel = Some(name.to_owned()))
    .on_open(|s: &mut State, name, _| {
        let remote = s.branch_view == 1;
        s.checkout_branch(name.to_owned(), remote);
    })
    .build(s, k)
    .grow(1.0)
    .min_h(0.0)
}

/// Two bars meeting at the middle: commits behind to the left, ahead to the right.
fn track(k: &Kit, b: &Branch) -> El {
    if b.upstream.is_empty() {
        return text(if b.remote { "" } else { "no upstream" }).font(k.ty.text_xsmall).color(k.c.text_faint).help("Not linked to a remote branch");
    }
    let w = |n: u32| (n as f32).sqrt() * 9.0;
    row()
        .gap(4.0)
        .center_y()
        .help(format!("{} commit(s) to pull, {} not on {} yet", b.behind, b.ahead, b.upstream))
        .child(text(format!("{}", b.behind)).font(k.ty.text_xsmall).color(k.c.text_muted).w(26.0))
        .child(row().w(40.0).child(spacer()).child(el().w(w(b.behind).min(40.0)).h(6.0).radius(2.0).fill(k.c.warning.icon)))
        .child(el().w(1.0).h(12.0).fill(k.c.border_strong))
        .child(row().w(40.0).child(el().w(w(b.ahead).min(40.0)).h(6.0).radius(2.0).fill(k.c.success.icon)))
        .child(text(format!("{}", b.ahead)).font(k.ty.text_xsmall).color(k.c.text_muted))
}

fn stashes(s: &State, k: &Kit) -> El {
    let Some(d) = s.rd() else { return empty(k, "") };
    if let Some(p) = pending(k, &d.stashes) {
        return p;
    }
    let list = d.stashes.data.as_deref().unwrap_or(&[]);
    let Some(dir) = s.local_path() else { return empty(k, "") };
    let mut c = col().w_full().gap(4.0);
    if list.is_empty() {
        c = c.child(empty(k, "No stashes. \"Stash and continue\" in a checkout dialog puts your changes here."));
    }
    let t = now();
    for (i, st) in list.iter().enumerate() {
        let btn = |label: &str, op: &'static str, tip: &str, variant| {
            let (dir, name) = (dir.clone(), st.name.clone());
            Button::new(label.to_owned()).variant(variant).help(tip.to_owned()).on_press(move |s: &mut State| s.request(Action::Stash { dir: dir.clone(), op, name: name.clone() })).build(k).key(format!("{op}-{i}"))
        };
        c = c.child(
            row()
                .key(format!("st-{i}"))
                .w_full()
                .gap(10.0)
                .px(12.0)
                .py(8.0)
                .radius(k.radius.md)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_weak)
                .center_y()
                .child(text(st.name.clone()).font(k.ty.mono).color(k.c.text_muted))
                .child(text(st.message.clone()).font(k.ty.ui).color(k.c.text_base).grow(1.0).truncate().shrink())
                .child(muted(k, ago(st.time, t)))
                .child(btn("Apply", "apply", "Put these changes back into your working copy and keep the stash", ButtonVariant::Outline))
                .child(btn("Pop", "pop", "Put these changes back and remove the stash", ButtonVariant::Outline))
                .child(btn("Drop", "drop", "Delete this stash for good (asks first)", ButtonVariant::GhostMuted)),
        );
    }
    crate::ui::scroll("stash-scroll", c)
}

fn cleanup(s: &State, k: &Kit) -> El {
    let Some(d) = s.rd() else { return empty(k, "") };
    if let Some(p) = pending(k, &d.cleanup) {
        return p;
    }
    let list = d.cleanup.data.as_deref().unwrap_or(&[]);
    let Some(dir) = s.local_path() else { return empty(k, "") };
    let mut c = col().w_full().gap(6.0).child(muted(k, "Only local branches are listed: deleting a branch on GitHub would be a push, which this app never does. The current, default and protected branches are never offered.").wrap());
    if list.is_empty() {
        c = c.child(empty(k, "Nothing to clean up 🎉"));
    }
    let merged: Vec<&crate::model::Cleanup> = list.iter().filter(|x| x.merged).collect();
    if merged.len() > 1 {
        let all: Vec<(String, String)> = merged.iter().map(|x| (x.name.clone(), x.sha.clone())).collect();
        let dir2 = dir.clone();
        c = c.child(
            row().w_full().child(spacer()).child(
                Button::new(format!("Delete all {} merged", merged.len()))
                    .variant(ButtonVariant::Outline)
                    .help("git branch -d on each: only branches already merged into the default branch, each with Undo")
                    .on_press(move |s: &mut State| {
                        for (name, sha) in &all {
                            s.request(Action::DeleteBranch { dir: dir2.clone(), name: name.clone(), sha: sha.clone(), force: false, unpushed: Vec::new() });
                        }
                    })
                    .build(k)
                    .key("del-merged"),
            ),
        );
    }
    for (i, b) in list.iter().enumerate() {
        let mut why = row().gap(6.0).center_y();
        if b.merged {
            why = why.child(chip(k, "merged", Some(Outcome::Success)).help("Fully merged into the default branch: safe to delete"));
        }
        if b.gone {
            why = why.child(chip(k, "gone upstream", Some(Outcome::Neutral)).help("Its GitHub branch was deleted"));
        }
        if let Some(days) = b.stale_days {
            why = why.child(chip(k, format!("untouched {days}d"), Some(Outcome::Neutral)));
        }
        let force = !b.merged;
        if force && !b.unpushed.is_empty() {
            why = why.child(chip(k, format!("{} commit(s) only here", b.unpushed.len()), Some(Outcome::Failure)).help(b.unpushed.join("\n")));
        }
        let (dir2, name, sha, unpushed) = (dir.clone(), b.name.clone(), b.sha.clone(), b.unpushed.clone());
        c = c.child(
            row()
                .key(format!("cl-{i}"))
                .w_full()
                .gap(10.0)
                .px(12.0)
                .py(8.0)
                .radius(k.radius.md)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_weak)
                .center_y()
                .child(super::ref_chip(k, &b.name, hue(1)))
                .child(why)
                .child(spacer())
                .child(
                    Button::new("Delete")
                        .variant(if force { ButtonVariant::Danger } else { ButtonVariant::Outline })
                        .help(if force { "Not merged: deleting needs -D and loses the commits listed. You'll type the branch name to confirm. Undo restores it." } else { "Merged: safe delete (git branch -d). Undo restores it." })
                        .on_press(move |s: &mut State| s.request(Action::DeleteBranch { dir: dir2.clone(), name: name.clone(), sha: sha.clone(), force, unpushed: unpushed.clone() }))
                        .build(k)
                        .key(format!("del-{i}")),
                ),
        );
    }
    crate::ui::scroll("cleanup-scroll", c)
}
