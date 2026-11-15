//! Everything drawn above the page: the guard dialog (A1), the merge countdown (A6), toasts with
//! Undo (A6), the command palette (B3) and the guide.

use super::cmd::CmdTip;
use std::time::Instant;

use rune::{El, Key, TextStyle, col, el, row, spacer};
use crate::i18n::text;
use rune_kit::{Button, ButtonVariant, Dialog, Kit, SegmentedControl, Spinner, Switch, TextInput, scrim};

use super::{chip, muted};
use crate::gh::MergeMethod;
use crate::guard::Action;
use crate::model::Outcome;
use crate::risk::Level;
use crate::State;

// ---------------------------------------------------------------------------------------------
// A1: the guard dialog
// ---------------------------------------------------------------------------------------------

pub fn guard_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(p) = &s.pending else { return };
    let level = p.risk.level;
    let (tone, tone_bg) = match level {
        Level::High => (k.c.danger.fg, k.c.danger.bg),
        Level::Medium => (k.c.warning.fg, k.c.warning.bg),
        Level::Low => (k.c.text_muted, k.c.surface_base),
    };
    let mut body = col().w_full().px(16.0).pb(16.0).gap(12.0);

    // Merge options live in the dialog.
    if let Action::Merge { method, delete_branch, .. } = &p.action {
        let i = MergeMethod::ALL.iter().position(|m| m == method);
        body = body.child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .flex_wrap()
                .child(
                    SegmentedControl::new(&crate::i18n::all(&MergeMethod::ALL.map(|m| m.label())), i)
                        .label(crate::i18n::t("Merge method"))
                        .fit()
                        .on_pick(|s: &mut State, i, _| {
                            if let Some(Action::Merge { method, .. }) = s.pending.as_mut().map(|p| &mut p.action) {
                                *method = MergeMethod::ALL[i];
                            }
                        })
                        .build(k)
                        .key("method")
                        .help(crate::i18n::t("Squash: one commit on the base branch. Merge commit: keeps every commit. Rebase: replays them.")),
                )
                .child(
                    Switch::new(*delete_branch)
                        .label(crate::i18n::t("Delete branch after"))
                        .on_toggle(|s: &mut State| {
                            if let Some(Action::Merge { delete_branch, .. }) = s.pending.as_mut().map(|p| &mut p.action) {
                                *delete_branch ^= true;
                            }
                        })
                        .build(k)
                        .key("delete-branch"),
                ),
        );
    }

    // Risk level + reasons.
    if p.checking {
        body = body.child(row().gap(8.0).center_y().child(Spinner::new().size(14.0).build(k)).child(muted(k, "Checking GitHub and your working copy…")));
    } else {
        let mut box_ = col().w_full().gap(6.0).padding(12.0).radius(k.radius.md).fill(tone_bg).child(
            row()
                .gap(8.0)
                .center_y()
                .child(text(level.label()).font(k.ty.ui_medium).color(tone))
                .child(spacer())
                .child(muted(k, "Why?").help(crate::i18n::t(
                    "sigit checks the base branch's protection, checks, reviews, changed files, the diff for secrets, \
                     production deployments and your working copy before anything is sent. Rules: Settings."),
                )),
        );
        if p.risk.reasons.is_empty() {
            box_ = box_.child(muted(k, "Nothing risky found."));
        }
        for (i, r) in p.risk.reasons.iter().enumerate() {
            box_ = box_.child(row().key(tf!("why-{i}")).w_full().gap(6.0).child(text(if r.starts_with("  •") { "" } else { "•" }).font(k.ty.small).color(tone)).child(text(r.clone()).font(k.ty.small).color(k.c.text_base).wrap().shrink()));
        }
        body = body.child(box_);
    }
    if let Some(b) = &p.blocked {
        body = body.child(col().w_full().gap(6.0).padding(12.0).radius(k.radius.md).fill(k.c.danger.bg).child(text("Can't do this right now").font(k.ty.ui_medium).color(k.c.danger.fg)).child(text(b.clone()).font(k.ty.small).color(k.c.text_base).wrap()));
    }
    if !p.dirty.is_empty() {
        let mut list = col().w_full().gap(2.0).padding(10.0).radius(k.radius.md).fill(k.c.bg_base);
        for f in p.dirty.iter().take(20) {
            list = list.child(text(f.clone()).font(k.ty.mono).color(k.c.text_muted).truncate());
        }
        if p.dirty.len() > 20 {
            list = list.child(muted(k, tf!("…and {} more", p.dirty.len() - 20)));
        }
        body = body.child(muted(k, "You have uncommitted changes. Stash them first (they're kept safe, and Undo or Branches → Stashes brings them back), or cancel:").wrap()).child(list);
    }
    if level == Level::High && !p.checking && p.blocked.is_none()
        && let Some(word) = &p.word
    {
        body = body.child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(tf!("Type {word} to confirm")).font(k.ty.small_medium).color(k.c.text_base))
                .child(
                    TextInput::new(&s.typed)
                        .placeholder(crate::i18n::t(word.clone()))
                        .width(None)
                        .invalid(!s.typed.text.is_empty() && s.typed.text.trim() != word)
                        .on_edit(|s: &mut State| &mut s.typed)
                        .on_submit(|s: &mut State, _| {
                            if s.can_confirm() {
                                s.confirm(false);
                            }
                        })
                        .build(k)
                        .key("typed"),
                ),
        );
    }

    // What will run, as you'd type it (with the stash first when that's the way on offer).
    let stash_first = !p.dirty.is_empty() && p.blocked.is_none();
    body = body.child(super::command_block(s, k, "guard-cmds", "Runs", &p.action.command_lines(stash_first)));

    let verb = match &p.action {
        Action::Merge { .. } => if s.prefs.grace_secs > 0 { tf!("Merge in {}s", s.prefs.grace_secs) } else { "Merge".into() },
        Action::Close { .. } => "Close pull request".into(),
        Action::Approve { .. } => "Approve".into(),
        Action::CancelRun { .. } => "Cancel run".into(),
        Action::Rerun { .. } => "Re-run".into(),
        Action::DeleteBranch { .. } => "Delete branch".into(),
        Action::Stash { op, .. } => tf!("{}{} stash", op[..1].to_uppercase(), &op[1..]),
        other => other.label(),
    };
    let mut buttons = row().w_full().gap(8.0).child(spacer()).child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.pending = None).build(k).key("no"));
    if !p.dirty.is_empty() && p.blocked.is_none() {
        buttons = buttons.child(
            { let tip = s.with_commands(crate::i18n::t("Stashes your changes first, then runs it. Undo pops the stash back."), &p.action.command_lines(true)); Button::new(crate::i18n::t("Stash and continue"))
                .variant(ButtonVariant::Submit)
                
                .on_press(|s: &mut State| s.confirm(true))
                .build(k)
                .key("stash-go").cmd_tip(s, tip) },
        );
    } else {
        buttons = buttons.child(
            Button::new(crate::i18n::t(verb))
                .variant(if level == Level::High { ButtonVariant::Danger } else { ButtonVariant::Submit })
                .disabled(!s.can_confirm())
                .help(crate::i18n::t(match level {
                    Level::High => "Type the name above to enable",
                    _ => "Runs it now",
                }))
                .on_press(|s: &mut State| s.confirm(false))
                .build(k)
                .key("yes"),
        );
    }
    body = body.child(buttons);
    let title = match level {
        _ if p.checking => tf!("{}?", p.action.label()),
        Level::High => tf!("⚠ {}?", p.action.label()),
        _ => tf!("{}?", p.action.label()),
    };
    Dialog::new("guard", crate::i18n::t(title)).description(crate::i18n::t(p.action.subject())).open(true).fit().width(560.0).on_close(|s: &mut State| s.pending = None).child(body).build(k, layer);
}

// ---------------------------------------------------------------------------------------------
// A6: the merge countdown
// ---------------------------------------------------------------------------------------------

pub fn grace_banner(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(g) = &s.grace else { return };
    let left = g.at.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as u32;
    let bar = row()
        .key("grace")
        .gap(14.0)
        .px(18.0)
        .py(12.0)
        .radius(k.radius.lg)
        .fill(k.c.surface_raised)
        .border(1.0, k.c.warning.icon)
        .center_y()
        .child(Spinner::new().size(14.0).color(k.c.warning.icon).build(k))
        .child(text(tf!("{} in {left}s…", g.action.label())).font(k.ty.ui_medium).color(k.c.text_base))
        .child(muted(k, "nothing has been sent yet"))
        .child(
            Button::new(crate::i18n::t("Cancel"))
                .variant(ButtonVariant::Danger)
                .shortcut("Esc")
                .help(crate::i18n::t("Stop it: nothing reaches GitHub"))
                .on_press(|s: &mut State| s.cancel_grace())
                .build(k)
                .key("grace-cancel"),
        );
    let bar = super::cmd::CmdTip::cmd_tip(bar, s, s.with_commands(crate::i18n::t("Runs when the countdown ends"), &g.action.command_lines(false)));
    layer.push(col().key("grace-layer").absolute(0.0, 0.0).w_full().h_full().padding(24.0).items_center().child(spacer()).child(bar));
}

// ---------------------------------------------------------------------------------------------
// A6: toasts with Undo
// ---------------------------------------------------------------------------------------------

pub fn toasts(s: &State, k: &Kit, layer: &mut Vec<El>) {
    if s.toasts.is_empty() {
        return;
    }
    let stack = s.toasts.iter().fold(col().gap(8.0).items_end(), |c, t| {
        let tone = if t.error { k.c.danger.icon } else { k.c.success.icon };
        let mut r = row()
            .key(tf!("toast-{}", t.id))
            .max_w(480.0)
            .gap(10.0)
            .px(14.0)
            .py(10.0)
            .radius(k.radius.md)
            .fill(k.c.surface_raised)
            .border(1.0, k.c.border_weak)
            .center_y()
            .child(el().size(8.0, 8.0).radius(4.0).fill(tone).self_start().mt(5.0));
        // The message, then the commands it ran ("$ …" lines) in mono.
        let (cmds, words): (Vec<&str>, Vec<&str>) = t.text.lines().partition(|l| l.starts_with("$ "));
        let mut msg = col().gap(4.0).shrink().min_w(0.0).child(text(words.join("\n")).font(k.ty.small).color(k.c.text_base).wrap());
        if !cmds.is_empty() {
            let shown: Vec<String> = cmds.iter().map(|c| c.trim_start_matches("$ ").to_owned()).collect();
            msg = msg.child(super::cmd::lines(k, &shown, 11.0).pt(2.0));
        }
        r = r.child(msg);
        if !cmds.is_empty() && !t.error {
            let all = cmds.iter().map(|c| c.trim_start_matches("$ ")).collect::<Vec<_>>().join("\n");
            r = r.child(super::copy_button(k, &tf!("toast-cmd-{}", t.id), all, "commands"));
        }
        if t.error {
            r = r.child(super::copy_button(k, &tf!("toast-copy-{}", t.id), t.text.clone(), "error message"));
        }
        if let Some(undo) = &t.undo {
            let id = t.id;
            r = r.child({ let tip = s.with_commands(crate::i18n::t("Take it back (available for 10 s)"), &undo.command_lines()); Button::new(crate::i18n::t("Undo")).variant(ButtonVariant::Outline).on_press(move |s: &mut State| s.run_undo(id)).build(k).key("undo").cmd_tip(s, tip) });
        }
        c.child(r.token_motion(k.motion.overlay))
    });
    layer.push(col().key("toasts").absolute(0.0, 0.0).w_full().h_full().padding(20.0).items_end().child(spacer()).child(stack));
}

// ---------------------------------------------------------------------------------------------
// B3: command palette
// ---------------------------------------------------------------------------------------------

pub fn palette(s: &State, k: &Kit, layer: &mut Vec<El>) {
    if !s.palette {
        return;
    }
    let items = s.palette_items();
    let sel = s.palette_sel.min(items.len().saturating_sub(1));
    let input = TextInput::new(&s.palette_q)
        .placeholder(crate::i18n::t("Jump to a repo, #PR, branch, group, or type a command…"))
        .leading_icon(k.glyph.search)
        .large()
        .width(None)
        .on_edit(|s: &mut State| &mut s.palette_q)
        .on_change(|s: &mut State, _| s.palette_sel = 0)
        .on_key(|s: &mut State, ev: &rune::KeyEvent, tx| {
            let n = s.palette_items().len();
            match ev.key {
                Key::Down => s.palette_sel = (s.palette_sel + 1).min(n.saturating_sub(1)),
                Key::Up => s.palette_sel = s.palette_sel.saturating_sub(1),
                Key::Enter => {
                    if let Some((item, _, _)) = s.palette_items().into_iter().nth(s.palette_sel) {
                        s.run_palette(item, tx);
                    }
                }
                Key::Escape => s.palette = false,
                _ => return false,
            }
            true
        })
        .build(k)
        .key("palette-input");
    let mut list = col().w_full().gap(2.0);
    if items.is_empty() {
        list = list.child(muted(k, "Nothing matches").padding(10.0));
    }
    for (i, (item, title, hint)) in items.into_iter().enumerate() {
        let on = i == sel;
        use crate::daily::PaletteItem as P;
        let glyph: &'static rune::Icon = match &item {
            P::Repo(_) => &super::I_REPO,
            P::Pr(_) => &super::I_PR,
            P::Branch(_) => &super::I_LOGO,
            P::Group(_) => &super::I_DASH,
            P::Page(_) => &super::I_FWD,
            P::Command(_) => &super::I_CMD,
        };
        let mark = row().size(24.0, 24.0).radius(k.radius.sm).center().fill(if on { k.c.surface_base } else { k.c.layer_01 }).child(rune::icon(glyph, 14.0).color(if on { k.c.icon_base } else { k.c.icon_faint }));
        list = list.child(
            row()
                .key(tf!("pal-{i}"))
                .w_full()
                .px(8.0)
                .py(5.0)
                .gap(10.0)
                .radius(k.radius.md)
                .center_y()
                .fill(if on { k.c.surface_hover } else { rune::Color::TRANSPARENT })
                .hover(k.c.hover)
                .on_press_tx(move |s: &mut State, tx| s.run_palette(item.clone(), tx))
                .child(mark)
                .child(text(title).font(k.ty.ui).color(k.c.text_base).truncate().shrink())
                .child(spacer())
                .child(text(hint).font(k.ty.text_xsmall).color(k.c.text_faint)),
        );
    }
    let panel = col()
        .key("palette")
        .w_full()
        .max_w(620.0)
        .gap(8.0)
        .padding(10.0)
        .radius(k.radius.lg)
        .fill(k.c.surface_raised)
        .border(1.0, k.c.border_weak)
        .child(input)
        .child(list)
        .child(row().w_full().px(6.0).child(muted(k, "↑↓ choose · Enter go · Esc close")).child(spacer()).child(muted(k, "Ctrl+K")))
        .modal(true, |s: &mut State| s.palette = false);
    layer.push(scrim("palette-scrim", |s: &mut State| s.palette = false).fill(k.c.scrim));
    layer.push(col().key("palette-layer").absolute(0.0, 0.0).w_full().h_full().pt(90.0).px(16.0).items_center().child(panel));
}

// ---------------------------------------------------------------------------------------------
// The guide
// ---------------------------------------------------------------------------------------------

/// (title, paragraphs). Each feature: what it's for, how to use it, and its keys.
pub const GUIDE: [(&str, &[&str]); 11] = [
    ("Welcome", &[
        "sigit is a fast window onto your everyday GitHub and git work. It reads through the gh and git you already use, so it never sees a password or token.",
        "It never commits or pushes. Anything that changes something goes through safety checks first (see Safety).",
        "Hover almost anything for a tooltip. Press F1 (or Ctrl+/) for this guide, and Ctrl+K to jump anywhere.",
    ]),
    ("Inbox & notifications", &[
        "The Inbox opens on Activity: what changed in the repos you track over a range (this week by default; Today, 7 days, this month, 30 days or custom dates). PRs opened, merged and closed; commits and lines changed; CI runs, success rate and minutes; deployments and releases; who contributed what share; and each repo's numbers, most changed first.",
        "It has two parts. Overview: totals across every tracked repo, per-day bars, who contributed, and repos at a glance (click one to jump to it). By repository: a section per repo, most changed first, with cards for pull requests, commits and lines, CI per workflow, contributors, and deployments and releases. Fold any section, or all of them.",
        "Tracked repos default to your pinned repos plus your groups. Change them with Manage tracked, or right-click any repo → Track in Activity. Switch the scope to one group to see just that product.",
        "The Needs you tab is your day: PRs waiting for your review, your own PRs, assigned issues, recent CI runs, and your GitHub notifications.",
        "Notifications: Open jumps into the PR or run. Done marks it read on GitHub too. Snooze hides it on this computer for 1 hour or until tomorrow.",
    ]),
    ("Review queue", &[
        "PRs waiting for your review, sorted by how long they've waited. Green under 4 hours, amber under a day, red after that.",
        "Keys (when no text box has focus): J / K move, Enter opens, A approves (with a summary of checks, size and sensitive files first), C opens it with the comment box focused.",
        "Your own PRs show who you're waiting on.",
    ]),
    ("Safety (guard rails)", &[
        "Every action has a risk level. Low runs at once. Needs a look asks first. High shows the reasons in red and you type the repo, branch or environment name to confirm.",
        "Checked before anything is sent: protected or default base branch and its rules, failing or running checks, missing approval, drafts, conflicts, files like workflows, migrations, Terraform, .env, and anything in the diff that looks like a secret.",
        "If you're an admin, GitHub would let a merge through its rules anyway. sigit says so and still asks.",
        "Merges wait a few seconds (Settings) with a Cancel button: nothing is sent until the countdown ends. Esc cancels.",
        "Closing a PR, checking out, deleting a local branch and pin/group edits show Undo for 10 seconds.",
    ]),
    ("Pull requests", &[
        "Filter as you type; after a short pause GitHub is searched too, so older PRs turn up. ⚠ chips mark PRs touching sensitive files.",
        "The detail pane: Conversation (description, reviews, comments, your comment box), Files (with the diff), Checks. A 'Why this needs care' box lists sensitive files and possible secrets.",
        "Watch a PR to get a desktop notification when its checks finish or a review comes in.",
    ]),
    ("Actions & CI triage", &[
        "Each bar is a run: height is duration, colour is the result. Click one to open it.",
        "A failed run shows 'Why it failed': the failing steps' last log lines, with errors highlighted. 'Probably flaky' means the same workflow on the same commit has both failed and passed.",
        "Re-running or cancelling a workflow that deploys to production asks you to type the environment name.",
    ]),
    ("Environments & production", &[
        "What's deployed where right now, and a timeline of deployments. An environment counts as production if its name is in your list (Settings) or it has required reviewers or a wait timer on GitHub.",
    ]),
    ("Commits, branches & clean-up", &[
        "Commits: the graph of every branch. Click a commit for its files and diff. 🛡 marks protected branches.",
        "Branches: Local, Remote, Stashes and Clean up. Checkout asks first if you have uncommitted changes, and offers Stash and continue.",
        "Clean up lists local branches that are merged, gone upstream, or untouched for 90 days. Merged ones delete safely. Unmerged ones list the commits you'd lose and need the name typed. Remote branches are never deleted.",
    ]),
    ("Groups & dashboards", &[
        "Right-click a repo to pin it or add it to one of your groups. Groups can mix owners.",
        "Drag pinned repos, or repos inside one of your groups, to put them in your own order. Right-click a group's header to move the whole group up or down.",
        "Click a section header to fold it; the ^ above the list collapses or expands everything. Folds are remembered.",
        "Each organization shows its 7 repos you opened most recently (then the most recently pushed), with 'Show N more' for the rest. The search box always looks through all of them.",
        "Right-click an organization's header to move it up, down or to the top, or to give it your own colour and icon. The badge then marks its repos everywhere in the sidebar and in the repo header.",
        "Click the ▦ button on a group's header (or pick it in Ctrl+K) for its dashboard: CI health per repo, deployments per environment, commits not yet on production, and every open PR in the group.",
    ]),
    ("Watching", &[
        "Watch a run or a PR (eye button) to get a desktop notification when it changes. Branches you check out in the app are watched automatically for the day (Settings).",
        "Notifications show while sigit is open, even minimized. Click one to jump to it.",
    ]),
    ("Settings & shortcuts", &[
        "Settings (the gear at the top right of the window, or Ctrl+,) opens a Settings window: Appearance (theme, text size, repos per organization, section sizes), Accounts (the GitHub accounts gh is signed in to, and switching between them), Git (your commit name and email), Safety (production names, sensitive paths, merge countdown), Notifications and watches, Data & storage (the cache), the Activity log, Shortcuts and About.",
        "Sections side by side (a list and its detail, the Activity panels, each repo's cards, Needs you) have a handle between them: drag it to resize, double-click it to reset. Sizes are remembered.",
        "Next to Back / Forward, \"updated 2m ago\" says how fresh the page is (\"cached\" = from last session, red = the last update failed). Pages show what they loaded at once, and check again in the background whenever you open one or come back to the window, so you rarely need Refresh (Activity at most once a minute). While a page stays open it re-checks on its own (PRs 2 min, runs 1 min or 15 s while running, local git 20 s, Activity 5 min). Click it or press Ctrl+R to reload everything now.",
        "Back / Forward (top left of the toolbar, or Alt+← / Alt+→) step through the pages and repos you visited, like a browser.",
        "Ctrl+K palette · F1 guide · Ctrl+R refresh · Alt+←/→ back/forward · Ctrl+1…6 Inbox / PRs / Actions / Environments / Commits / Branches · Esc closes the palette or cancels a merge countdown.",
        "Pins, groups, settings, the cache and the activity log are saved on this computer only. Settings → Data & storage shows the folder and opens it.",
    ]),
];

pub fn guide(s: &State, k: &Kit, layer: &mut Vec<El>) {
    if !s.guide_open {
        return;
    }
    let sec = s.guide_sec.min(GUIDE.len() - 1);
    let mut nav = col().w(200.0).gap(2.0);
    for (i, (title, _)) in GUIDE.iter().enumerate() {
        nav = nav.child(
            row()
                .key(tf!("g-{i}"))
                .w_full()
                .px(10.0)
                .py(7.0)
                .radius(k.radius.md)
                .fill(if i == sec { k.c.surface_hover } else { rune::Color::TRANSPARENT })
                .hover(k.c.hover)
                .on_press(move |s: &mut State| s.guide_sec = i)
                .child(text(*title).font(if i == sec { k.ty.ui_medium } else { k.ty.ui }).color(k.c.text_base)),
        );
    }
    let (title, paras) = GUIDE[sec];
    let mut content = col().grow(1.0).shrink().gap(12.0).child(text(title).font(TextStyle { size: 18.0, ..k.ty.ui_medium }).color(k.c.text_strong));
    for (i, p) in paras.iter().enumerate() {
        content = content.child(text(*p).key(tf!("gp-{i}")).font(k.ty.body).color(k.c.text_base).wrap().w_full());
    }
    let last = sec + 1 == GUIDE.len();
    let footer = row()
        .w_full()
        .gap(8.0)
        .child(muted(k, tf!("{} of {}", sec + 1, GUIDE.len())))
        .child(spacer())
        .child(Button::new(crate::i18n::t("Back")).variant(ButtonVariant::Outline).disabled(sec == 0).on_press(|s: &mut State| s.guide_sec = s.guide_sec.saturating_sub(1)).build(k).key("g-back"))
        .child(
            Button::new(crate::i18n::t(if last { "Done" } else { "Next" }))
                .variant(ButtonVariant::Submit)
                .on_press(move |s: &mut State| {
                    if last {
                        close_guide(s);
                    } else {
                        s.guide_sec += 1;
                    }
                })
                .build(k)
                .key("g-next"),
        );
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(16.0)
        .child(row().w_full().gap(20.0).items_start().child(nav).child(content))
        .child(footer);
    Dialog::new("guide", crate::i18n::t("sigit guide")).description(crate::i18n::t("What each feature does and how to use it. F1 opens this any time.")).open(true).fit().width(820.0).on_close(close_guide).child(body).build(k, layer);
}

fn close_guide(s: &mut State) {
    s.guide_open = false;
    if !s.prefs.seen_guide {
        s.prefs.seen_guide = true;
        s.save_prefs();
    }
}

/// A small "?" next to a section title that explains it (and opens the guide on click).
pub fn hint(k: &Kit, key: &str, tip: &str, section: usize) -> El {
    row()
        .key(key.to_owned())
        .size(16.0, 16.0)
        .radius(8.0)
        .center()
        .border(1.0, k.c.border_base)
        .hover(k.c.hover)
        .help(crate::i18n::t(tf!("{tip}\n(click for the guide)")))
        .on_press(move |s: &mut State| {
            s.guide_open = true;
            s.guide_sec = section;
        })
        .child(text("?").font(k.ty.text_xsmall_medium).color(k.c.text_muted))
}

/// The toolbar's freshness mark for the page: "updated 2m ago", "cached · 3h ago" (from last
/// session), "updating…" or "update failed". Click it to reload the page now.
pub fn fresh_chip(k: &Kit, f: &crate::ui::Fresh) -> El {
    let now = crate::model::now();
    let age = f.at.map(|t| crate::model::ago(t, now));
    let (label, tone) = match (&age, f.loading, f.cached, &f.error) {
        (None, true, ..) => ("loading…".to_owned(), None),
        (Some(a), true, ..) => (tf!("updating · {a}"), None),
        (_, false, _, Some(_)) => (age.as_ref().map_or("load failed".into(), |a| tf!("update failed · {a}")), Some(Outcome::Failure)),
        (Some(a), false, true, None) => (tf!("cached · {a}"), Some(Outcome::Running)),
        (Some(a), false, false, None) => (tf!("updated {a}"), Some(Outcome::Success)),
        (None, false, ..) => ("not loaded".to_owned(), None),
    };
    let mut help = match (f.cached, &f.error) {
        (_, Some(e)) => tf!("The last update failed: {e}
Showing what loaded before."),
        (true, None) => "From last session's cache; fresh data replaces it as it loads.".to_owned(),
        _ => "What this page shows is kept in memory (and on disk between sessions).".to_owned(),
    };
    help.push_str(&tf!(
        "
Opening this page or coming back to the window shows what's cached and checks GitHub again in the background; while it stays open it re-checks every {}. Actions always re-read GitHub first.
Click (or Ctrl+R) to reload everything now.",
        crate::model::duration(f.ttl)
    ));
    let mut c = row().gap(6.0).center_y();
    if f.loading {
        c = c.child(Spinner::new().size(11.0).build(k));
    }
    c.child(chip(k, label, tone))
        .key("freshness")
        .hover(k.c.hover)
        .radius(999.0)
        .help(crate::i18n::t(help))
        .on_press(|s: &mut State| s.refresh_page())
}
