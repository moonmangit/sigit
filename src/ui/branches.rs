//! Branches: Local / Remote (with 🛡 on protected ones and ahead/behind against upstream),
//! Stashes (A5), and Clean up (B7: local branches that can go; remote ones never).
//! Double-click (or Enter) a branch to check it out; every action goes through the guard.

use super::cmd::CmdTip;
use std::borrow::Cow;

use rune::{El, col, el, row, spacer};
use crate::i18n::text;
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
            SegmentedControl::new(&crate::i18n::all(&VIEWS.map(|v| v.0)), Some(s.branch_view))
                .label(crate::i18n::t("Branches"))
                .fit()
                .on_pick(|s: &mut State, i, _| {
                    s.branch_view = i;
                    s.close_branch_lane();
                    if i >= 2 {
                        s.refresh_branch_tools();
                    }
                })
                .build(k)
                .key("b-scope")
                .help(crate::i18n::t(VIEWS[s.branch_view].1)),
        )
        .child(hint(k, "h-branches", VIEWS[s.branch_view].1, 7));
    if s.branch_view < 2 {
        toolbar = toolbar.child(muted(k, { let n = visible(s).len(); tf!("{n} branch{}", if n == 1 { "" } else { "es" }) }).help(crate::i18n::t("Double-click a branch to check it out. 🛡 marks protected or default branches."))).child(spacer());
        if let Some(name) = sel {
            let remote = s.branch_view == 1;
            toolbar = toolbar.child(
                { let tip = s.with_commands(crate::i18n::t("Switch your clone to it. Asks first if you have uncommitted changes, and offers to stash them."), &crate::git::checkout_cmds(&name, remote).iter().map(crate::gh::Cmd::line).collect::<Vec<_>>()); Button::new(crate::i18n::t(tf!("Check out {}", short_name(&name))))
                    .variant(ButtonVariant::Contrast)
                    
                    .on_press(move |s: &mut State| s.checkout_branch(name.clone(), remote))
                    .build(k)
                    .key("b-checkout").cmd_tip(s, tip) },
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
            let list = col().key("b-list").w_full().h_full().min_h(0.0).radius(k.radius.lg).border(1.0, k.c.border_weak).fill(k.c.layer_01).clip_rounded(k.radius.lg).child(list);
            // The picked branch's lane beside the table, sized like the Comparisons pane.
            let mut panes = vec![super::split::pane(list, 0.64, 520.0)];
            if let Some(pane) = super::compare::branch_pane(s, k) {
                panes.push(super::split::pane(pane, 0.36, 320.0));
            }
            super::split::split(s, k, "branches", panes).key("b-body").w_full().grow(1.0).min_h(0.0)
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
    .column(TableColumn::new("name", crate::i18n::t("Branch")).flex(1.5).min(180.0))
    .column(TableColumn::new("track", crate::i18n::t("Behind / ahead")).width(170.0).resizable(false))
    .column(TableColumn::new("subject", crate::i18n::t("Last commit")).flex(2.0).min(200.0))
    .column(TableColumn::new("author", crate::i18n::t("Author")).width(140.0).min(80.0))
    .column(TableColumn::new("when", crate::i18n::t("Updated")).width(90.0).resizable(false))
    .cell(move |s: &State, i, col| {
        let v = visible(s);
        let Some(b) = v.get(i) else { return Cell::text("") };
        let k = &kc;
        match col {
            "name" => {
                let mut r = row().gap(6.0).center_y();
                if b.current {
                    r = r.child(chip(k, "current", Some(Outcome::Success)).help(crate::i18n::t("Your clone is on this branch")));
                }
                if b.gone {
                    r = r.child(chip(k, "gone", Some(Outcome::Neutral)).help(crate::i18n::t("Its remote branch was deleted (usually after its PR merged): a clean-up candidate")));
                }
                Cell::el(r.child(branch_chip(s, k, &b.name, if b.remote { hue(3) } else { hue(1) })))
            }
            "track" => Cell::el(track(k, b)),
            "subject" => Cell::text(b.subject.clone()).muted(),
            "author" => Cell::text(b.author.clone()).muted(),
            _ => Cell::text(ago(b.time, now)).muted(),
        }
    })
    .on_select(|s: &mut State, name, _| {
        s.branch_sel = Some(name.to_owned());
        s.select_branch_lane(name);
    })
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
        return text(if b.remote { "" } else { "no upstream" }).font(k.ty.text_xsmall).color(k.c.text_faint).help(crate::i18n::t("Not linked to a remote branch"));
    }
    let w = |n: u32| (n as f32).sqrt() * 9.0;
    row()
        .gap(4.0)
        .center_y()
        .help(crate::i18n::t(tf!("{} commit(s) to pull, {} not on {} yet", b.behind, b.ahead, b.upstream)))
        .child(text(tf!("{}", b.behind)).font(k.ty.text_xsmall).color(k.c.text_muted).w(26.0))
        .child(row().w(40.0).child(spacer()).child(el().w(w(b.behind).min(40.0)).h(6.0).radius(2.0).fill(k.c.warning.icon)))
        .child(el().w(1.0).h(12.0).fill(k.c.border_strong))
        .child(row().w(40.0).child(el().w(w(b.ahead).min(40.0)).h(6.0).radius(2.0).fill(k.c.success.icon)))
        .child(text(tf!("{}", b.ahead)).font(k.ty.text_xsmall).color(k.c.text_muted))
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
            { let tip = s.with_commands(crate::i18n::t(tip.to_owned()), &[crate::git::stash_op_cmd(op, &name).line()]); Button::new(crate::i18n::t(label.to_owned())).variant(variant).on_press(move |s: &mut State| s.request(Action::Stash { dir: dir.clone(), op, name: name.clone() })).build(k).key(tf!("{op}-{i}")).cmd_tip(s, tip) }
        };
        c = c.child(
            row()
                .key(tf!("st-{i}"))
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
                { let tip = s.with_commands(crate::i18n::t("Deletes each one: only branches already merged into the default branch, each with Undo"), &all.iter().map(|(n, _)| crate::git::delete_branch_cmd(n, false).line()).collect::<Vec<_>>()); Button::new(crate::i18n::t(tf!("Delete all {} merged", merged.len())))
                    .variant(ButtonVariant::Outline)
                    
                    .on_press(move |s: &mut State| {
                        for (name, sha) in &all {
                            s.request(Action::DeleteBranch { dir: dir2.clone(), name: name.clone(), sha: sha.clone(), force: false, unpushed: Vec::new() });
                        }
                    })
                    .build(k)
                    .key("del-merged").cmd_tip(s, tip) },
            ),
        );
    }
    for (i, b) in list.iter().enumerate() {
        let mut why = row().gap(6.0).center_y();
        if b.merged {
            why = why.child(chip(k, "merged", Some(Outcome::Success)).help(crate::i18n::t("Fully merged into the default branch: safe to delete")));
        }
        if b.gone {
            why = why.child(chip(k, "gone upstream", Some(Outcome::Neutral)).help(crate::i18n::t("Its GitHub branch was deleted")));
        }
        if let Some(days) = b.stale_days {
            why = why.child(chip(k, tf!("untouched {days}d"), Some(Outcome::Neutral)));
        }
        let force = !b.merged;
        if force && !b.unpushed.is_empty() {
            why = why.child(chip(k, tf!("{} commit(s) only here", b.unpushed.len()), Some(Outcome::Failure)).help(crate::i18n::t(b.unpushed.join("\n"))));
        }
        let (dir2, name, sha, unpushed) = (dir.clone(), b.name.clone(), b.sha.clone(), b.unpushed.clone());
        c = c.child(
            row()
                .key(tf!("cl-{i}"))
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
                    { let tip = s.with_commands(crate::i18n::t(if force { "Not merged: deleting needs -D and loses the commits listed. You'll type the branch name to confirm. Undo restores it." } else { "Merged: safe delete. Undo restores it." }), &[crate::git::delete_branch_cmd(&b.name, force).line()]); Button::new(crate::i18n::t("Delete"))
                        .variant(if force { ButtonVariant::Danger } else { ButtonVariant::Outline })
                        
                        .on_press(move |s: &mut State| s.request(Action::DeleteBranch { dir: dir2.clone(), name: name.clone(), sha: sha.clone(), force, unpushed: unpushed.clone() }))
                        .build(k)
                        .key(tf!("del-{i}")).cmd_tip(s, tip) },
                ),
        );
    }
    crate::ui::scroll("cleanup-scroll", c)
}

/// What the branch pane can compare with: the default branch first, then the repo's branches
/// (remote ones without `origin/`) A–Z, each once, leaving out the one picked.
pub(crate) fn base_options<'a>(default: &str, picked: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut rest: Vec<String> = names.map(|n| n.strip_prefix("origin/").unwrap_or(n).to_owned()).filter(|n| n != default && n != picked && n != "HEAD").collect();
    // Case-insensitive A–Z, ties by exact name, so a name and its remote copy end up adjacent.
    rest.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b)));
    rest.dedup();
    std::iter::once(default.to_owned()).chain(rest).collect()
}

/// The open "compared with" list (its options worked out only while it's open).
pub fn base_list(s: &State, k: &Kit, layer: &mut Vec<rune::El>) {
    if !s.branch_base_open || s.page != crate::Page::Branches || s.branch_view > 1 {
        return;
    }
    let Some((slug, picked)) = s.branch_lane_sel.as_ref().filter(|(sl, _)| s.cur.as_ref() == Some(sl)) else { return };
    let Some(default) = s.data.get(slug).and_then(|d| d.meta.data.as_ref()).map(|m| m.default_branch.clone()).filter(|b| !b.is_empty()) else { return };
    let names = s.rd().and_then(|d| d.branches.data.as_deref()).unwrap_or(&[]);
    let options = base_options(&default, picked, names.iter().map(|b| b.name.as_str()));
    super::compare::base_list(s, k, &options, layer);
}

/// A branch name short enough for a button: long ones keep their start and end.
fn short_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= 32 {
        return name.to_owned();
    }
    tf!("{}…{}", chars[..20].iter().collect::<String>(), chars[chars.len() - 10..].iter().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::base_options;

    #[test]
    fn base_options_default_first_then_a_to_z() {
        let names = ["origin/feat", "feat", "zeta", "origin/main", "main", "origin/alpha"];
        assert_eq!(base_options("main", "zeta", names.into_iter()), ["main", "alpha", "feat"]);
        // The default even when it's the branch picked.
        assert_eq!(base_options("main", "main", ["dev", "main"].into_iter()), ["main", "dev"]);
        // Case variants (Feat, feat) don't split a name from its remote copy.
        let names = ["feat", "Feat", "origin/feat", "origin/Feat"];
        assert_eq!(base_options("main", "x", names.into_iter()), ["main", "Feat", "feat"]);
    }
}
