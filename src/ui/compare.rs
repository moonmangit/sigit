//! The Dashboard's Comparisons tab: each tracked repo as a branch flow, one lane per shared
//! comparison (from → to, a bar as long as what is waiting), a pane listing what a lane counts, and the
//! per-repo dialog that retargets or turns comparisons off.

use std::collections::HashMap;

use rune::{El, TextEdit, TextStyle, col, el, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, SegmentedControl, Select, Spinner, Switch, TextInput, glyphs};

use super::{I_GEAR, chip, click_to_copy, hue, muted, pending, ref_chip, scroll, side_pane};
use crate::compare::{Cell, Comparison, LooseCommit, PrDetail, RepoCompare, WaitingPr, effective, is_changed};
use crate::lane::{self, FolderShare, Folders};
use crate::lane_model::{LaneModel, OTHER, Segment};
use crate::i18n::text;
use crate::model::{Branch, Outcome, ago_str, now};
use crate::modules;
use crate::{CompareScope, State};

/// Lane layout: the two branch chips, the track between them, and the label after.
const CHIP_W: f32 = 120.0;
const LABEL_W: f32 = 210.0;
/// Bar colour by commits waiting: amber up to this many, red beyond.
const AMBER_UP_TO: u32 = 20;
/// Dots drawn per lane before older ones collapse into "+N".
const MAX_DOTS: usize = 10;
/// The track never gets narrower than this many dots and links need, so a chain can't spill
/// over the To branch.
const TRACK_MIN_W: f32 = 150.0;

pub fn view(s: &State, k: &Kit) -> El {
    let body = if s.tracked().is_empty() {
        notice(k, "Track some repos to compare their branches.").child(
            Button::new(crate::i18n::t("Manage tracked")).variant(ButtonVariant::Contrast).on_press(|s: &mut State| s.tracked_open = true).build(k).key("cmp-manage"),
        )
    } else if s.prefs.comparisons.is_empty() {
        notice(k, "Add a comparison in Settings → Git.")
    } else if let Some(p) = pending(k, &s.compare) {
        p
    } else {
        let rows = s.compare.data.as_deref().unwrap_or(&[]);
        let mut panes = vec![super::split::pane(scroll("cmp-scroll", flows(s, k, rows)), 0.64, 520.0)];
        if let Some(pane) = detail(s, k) {
            panes.push(super::split::pane(pane, 0.36, 320.0));
        }
        super::split::split(s, k, "compare", panes).key("cmp-body").h_full()
    };
    col().key("comparisons").w_full().h_full().padding(24.0).child(body)
}

fn notice(k: &Kit, msg: &str) -> El {
    col().key("cmp-empty").w_full().py(56.0).items_center().gap(12.0).child(text(msg.to_owned()).font(k.ty.ui).color(k.c.text_muted))
}

fn waiting(c: &Cell) -> u32 {
    match c {
        Cell::Waiting { commits, .. } => *commits,
        _ => 0,
    }
}

/// Every tracked repo as a card of branch lanes, the ones with the most waiting first; repos
/// with nothing waiting (and no error) fold into one quiet list at the end.
/// Repos sorted into your sidebar groups, in the sidebar's order (a repo in two groups shows in
/// both, as there), then "Other" for tracked repos in no group. No groups: one plain list.
fn flows(s: &State, k: &Kit, rows: &[RepoCompare]) -> El {
    let groups = &s.prefs.groups;
    if groups.is_empty() {
        return group_block(s, k, rows.iter().collect()).key("cmp-flows");
    }
    let mut out = col().key("cmp-flows").w_full().gap(20.0).child(scope_picker(s, k, rows));
    for (i, (scope, repos)) in sections(groups, rows, s.compare_scope.as_ref()).into_iter().enumerate() {
        let name = match scope {
            CompareScope::Group(g) => g,
            CompareScope::Other => crate::i18n::t("Other"),
        };
        let waiting_here = repos.iter().filter(|r| r.cells.iter().any(|c| waiting(c) > 0)).count();
        let head = row()
            .gap(8.0)
            .center_y()
            .child(rune::icon(&super::I_DASH, 14.0).color(k.c.icon_faint))
            .child(rune::text(name).font(k.ty.ui_medium).color(k.c.text_base))
            .child(muted(k, tf!("{waiting_here} of {n} with work waiting", n = repos.len())));
        out = out.child(col().key(tf!("cmp-g-{i}")).w_full().gap(10.0).child(head).child(group_block(s, k, repos)));
    }
    out
}

/// The repos sorted into your sidebar groups, in the sidebar's order (a repo in two groups is in
/// both), then `Other` for tracked repos in no group; empty sections dropped. With a scope, only
/// that section; a scope naming a group that no longer exists shows them all.
pub(crate) fn sections<'a>(groups: &[crate::config::Group], rows: &'a [RepoCompare], scope: Option<&CompareScope>) -> Vec<(CompareScope, Vec<&'a RepoCompare>)> {
    let mut all: Vec<(CompareScope, Vec<&RepoCompare>)> =
        groups.iter().map(|g| (CompareScope::Group(g.name.clone()), rows.iter().filter(|r| g.repos.contains(&r.slug)).collect())).collect();
    all.push((CompareScope::Other, rows.iter().filter(|r| !groups.iter().any(|g| g.repos.contains(&r.slug))).collect()));
    all.retain(|(_, repos)| !repos.is_empty());
    match scope {
        Some(sc) if all.iter().any(|(x, _)| x == sc) => all.into_iter().filter(|(x, _)| x == sc).collect(),
        _ => all,
    }
}

/// All / each group / Other, like the Activity tab's picker. Picking closes an open lane pane.
fn scope_picker(s: &State, k: &Kit, rows: &[RepoCompare]) -> El {
    let present = sections(&s.prefs.groups, rows, None);
    let mut labels = vec![tf!("All ({n})", n = rows.len())];
    labels.extend(present.iter().map(|(sc, repos)| match sc {
        CompareScope::Group(g) => format!("{g} ({})", repos.len()),
        CompareScope::Other => tf!("Other ({n})", n = repos.len()),
    }));
    let scopes: Vec<Option<CompareScope>> = std::iter::once(None).chain(present.into_iter().map(|(sc, _)| Some(sc))).collect();
    let picked = scopes.iter().position(|x| x.as_ref() == s.compare_scope.as_ref()).unwrap_or(0);
    row().w_full().child(
        SegmentedControl::new(&labels, Some(picked))
            .label(crate::i18n::t("Which group"))
            .fit()
            .on_pick(move |s: &mut State, i, _| {
                let next = scopes.get(i).cloned().flatten();
                if next != s.compare_scope {
                    s.compare_scope = next;
                    s.compare_sel = None;
                }
            })
            .build(k)
            .key("cmp-scope"),
    )
}

/// One group's repos: cards for those with something waiting (most first), then the calm ones
/// folded into one quiet list.
fn group_block(s: &State, k: &Kit, rows: Vec<&RepoCompare>) -> El {
    let cmps = &s.prefs.comparisons;
    let mut busy: Vec<&RepoCompare> = rows.iter().copied().filter(|r| r.error.is_some() || r.cells.iter().any(|c| waiting(c) > 0)).collect();
    busy.sort_by_key(|r| std::cmp::Reverse(r.cells.iter().map(waiting).max().unwrap_or(0)));
    let calm: Vec<&RepoCompare> = rows.iter().copied().filter(|r| r.error.is_none() && r.cells.iter().all(|c| waiting(c) == 0)).collect();
    let mut c = col().w_full().gap(12.0);
    for rc in &busy {
        c = c.child(repo_card(s, k, rc, cmps));
    }
    if !calm.is_empty() {
        let mut list = col().key("cmp-calm").w_full().px(14.0).py(10.0).gap(6.0).radius(k.radius.lg).fill(k.c.layer_01).border(1.0, k.c.border_weak).child(
            row()
                .gap(8.0)
                .center_y()
                .child(el().size(8.0, 8.0).radius(4.0).fill(k.c.success.icon))
                .child(text(tf!("Nothing waiting ({n})", n = calm.len())).font(k.ty.small_medium).color(k.c.text_muted)),
        );
        for rc in &calm {
            list = list.child(
                row()
                    .key(tf!("cmp-calm-{}", rc.slug))
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(rune::text(rc.slug.clone()).font(k.ty.small).color(k.c.text_faint).grow(1.0).shrink().truncate())
                    .child(muted(k, calm_note(&rc.cells)))
                    .child(edit_button(k, &rc.slug)),
            );
        }
        c = c.child(list);
    }
    c
}

/// Why a calm repo has nothing waiting: up to date, or only missing branches / turned off.
fn calm_note(cells: &[Cell]) -> String {
    if cells.iter().any(|c| matches!(c, Cell::Waiting { .. })) {
        return crate::i18n::t("up to date");
    }
    match cells.iter().find_map(|c| if let Cell::Missing(b) = c { b.first() } else { None }) {
        Some(b) => tf!("no {b} branch"),
        None => crate::i18n::t("off"),
    }
}

fn edit_button(k: &Kit, slug: &str) -> El {
    let target = slug.to_owned();
    IconButton::new(&I_GEAR)
        .variant(IconButtonVariant::GhostMuted)
        .label(crate::i18n::t("Edit comparisons for this repo…"))
        .help(crate::i18n::t("Edit comparisons for this repo…"))
        .on_press(move |s: &mut State| s.open_compare_edit(&target))
        .build(k)
        .key(tf!("cmp-edit-{slug}"))
}

fn repo_card(s: &State, k: &Kit, rc: &RepoCompare, cmps: &[Comparison]) -> El {
    let has_data = rc.cells.iter().any(|c| !matches!(c, Cell::Off));
    let mut card = col().key(tf!("cmp-r-{}", rc.slug)).w_full().px(16.0).py(12.0).gap(8.0).radius(k.radius.lg).fill(k.c.layer_01).border(1.0, k.c.border_weak).child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(rune::text(rc.slug.clone()).font(k.ty.ui_medium).color(k.c.text_base).grow(1.0).shrink().truncate())
            .child(edit_button(k, &rc.slug)),
    );
    if let Some(e) = &rc.error {
        card = card.child(rune::text(e.clone()).font(k.ty.small).color(k.c.danger.fg).wrap().w_full());
    }
    if !has_data {
        return card;
    }
    let eff = effective(cmps, &s.prefs.comparison_overrides, &rc.slug);
    for (j, c) in cmps.iter().enumerate() {
        let cell = rc.cells.get(j).unwrap_or(&Cell::Off);
        let (from, to) = match eff.get(j).cloned().flatten() {
            Some(e) => (e.from, e.to),
            None => (c.from.clone(), c.to.clone()),
        };
        let changed = is_changed(&s.prefs.comparison_overrides, &rc.slug, &c.name);
        let l = Lane { slug: &rc.slug, j, name: &c.name, from: &from, to: &to, cell, changed };
        card = card.child(lane(s, k, &l).key(tf!("cmp-c-{}-{j}", rc.slug)));
    }
    card
}

struct Lane<'a> {
    slug: &'a str,
    j: usize,
    name: &'a str,
    from: &'a str,
    to: &'a str,
    cell: &'a Cell,
    changed: bool,
}

/// One comparison as an arrow from `from` to `to`, with what's waiting drawn as a chain of dots.
fn lane(s: &State, k: &Kit, l: &Lane) -> El {
    // The From chip sits against the track's start, the To chip against its end.
    let chip = |b: &str, h: usize, right: bool| {
        let r = row().w(CHIP_W);
        if right { r.child(spacer()).child(super::ref_chip_fit(k, b, hue(h))) } else { r.child(super::ref_chip_fit(k, b, hue(h))) }
    };
    let quiet = |label: String| text(label).font(k.ty.small).color(k.c.text_faint).truncate();
    let mut title = row().gap(6.0).center_y().child(rune::text(l.name.to_owned()).font(k.ty.small_medium).color(k.c.text_muted).truncate());
    if l.changed {
        title = title.child(el().size(6.0, 6.0).radius(3.0).fill(hue(2)).help(crate::i18n::t("Changed for this repo")));
    }
    let mut label = col().w(LABEL_W).gap(1.0).child(title);
    let track = match l.cell {
        Cell::Off => {
            label = label.child(quiet(crate::i18n::t("off")));
            dashed(k)
        }
        Cell::Missing(b) => {
            label = label.child(quiet(missing_label(b)));
            dashed(k)
        }
        Cell::Waiting { commits: 0, .. } => {
            label = label.child(text("up to date").font(k.ty.small).color(k.c.success.fg));
            row().w_full().center_y().child(el().w(0.0).grow(1.0).h(2.0).fill(k.c.success.icon)).child(text("▶").font(TextStyle { size: 10.0, ..k.ty.small }).color(k.c.success.icon))
        }
        Cell::Waiting { commits, prs, loose, checked } => {
            let tone = if *commits <= AMBER_UP_TO { k.c.warning.icon } else { k.c.danger.icon };
            let mut counts = rune::text(format!("{} · {}", commits_label(*commits), prs_label(prs.len()))).font(k.ty.small).color(k.c.text_base).truncate();
            if checked < commits {
                counts = counts.help(tf!("PRs counted from the first {checked} commits"));
            }
            label = label.child(counts);
            chain(k, prs, loose, tone)
        }
    };
    let mut r = row()
        .w_full()
        .gap(10.0)
        .center_y()
        .px(6.0)
        .py(4.0)
        .radius(k.radius.md)
        .child(chip(l.from, 1, true))
        .child(row().w(0.0).grow(1.0).min_w(TRACK_MIN_W).center_y().clip_rounded(0.0).child(track))
        .child(chip(l.to, 0, false))
        .child(label);
    if waiting(l.cell) > 0 {
        let selected = s.compare_sel.as_ref().is_some_and(|(sl, i)| sl == l.slug && *i == l.j);
        if selected {
            r = r.fill(k.c.surface_hover);
        }
        let (slug, j) = (l.slug.to_owned(), l.j);
        r = r.hover(k.c.hover).role("button").on_press(move |s: &mut State| s.select_lane(&slug, j));
    }
    r
}

/// What's waiting as a git-style chain from From to To: a filled dot per merged PR (oldest
/// merge first), a hollow dot per commit that came from no PR. Long chains keep the newest
/// `MAX_DOTS` and say how many more came before them.
fn chain(k: &Kit, prs: &[WaitingPr], loose: &[LooseCommit], tone: rune::Color) -> El {
    let link = || el().w(0.0).grow(1.0).min_w(3.0).h(2.0).fill(tone);
    let mut items: Vec<(bool, String)> = Vec::with_capacity(prs.len() + loose.len());
    let mut by_merge: Vec<&WaitingPr> = prs.iter().collect();
    by_merge.sort_by(|a, b| a.merged_at.cmp(&b.merged_at));
    items.extend(by_merge.iter().map(|p| (true, format!("#{} {} · {}", p.number, p.title, p.author))));
    items.extend(loose.iter().map(|c| (false, format!("{} {} · {}", c.sha.get(..7).unwrap_or(&c.sha), c.subject, c.author))));
    let hidden = items.len().saturating_sub(MAX_DOTS);
    let mut r = row().w_full().center_y();
    if hidden > 0 {
        r = r.child(text(tf!("+{hidden}")).font(TextStyle { size: 10.0, ..k.ty.small }).color(k.c.text_faint).help(tf!("{hidden} older not drawn")));
    }
    r = r.child(link());
    for (pr, tip) in items.into_iter().skip(hidden) {
        let dot = el().size(9.0, 9.0).radius(4.5);
        let dot = if pr { dot.fill(tone) } else { dot.fill(k.c.layer_01).border(2.0, tone) };
        r = r.child(dot.help(tip)).child(link());
    }
    r.child(text("▶").font(TextStyle { size: 10.0, ..k.ty.small }).color(tone))
}

/// No data for this lane: a faint dotted track.
fn dashed(k: &Kit) -> El {
    let mut r = row().w_full().gap(4.0).center_y();
    for _ in 0..12 {
        r = r.child(el().w(6.0).h(1.0).fill(k.c.border_weak));
    }
    r
}

fn commits_label(n: u32) -> String {
    if n == 1 { crate::i18n::t("1 commit") } else { tf!("{n} commits") }
}

fn prs_label(n: usize) -> String {
    if n == 1 { crate::i18n::t("1 PR") } else { tf!("{n} PRs") }
}

fn missing_label(b: &[String]) -> String {
    match b {
        [a, b, ..] => tf!("— no {a} or {b}"),
        [branch] => tf!("— no {branch}"),
        [] => "—".to_owned(),
    }
}

/// The pane for the picked cell: its merged PRs waiting, then the commits that came from no PR.
fn detail(s: &State, k: &Kit) -> Option<El> {
    let lane = s.open_lane()?;
    let Cell::Waiting { commits, .. } = lane.cell else { return None };
    if *commits == 0 {
        return None;
    }
    let c = col().w_full().gap(14.0).child(pane_head(k, lane.name.clone(), lane.slug, "cmp-close", |s: &mut State| s.compare_sel = None));
    Some(side_pane(k, "cmp-pane", lane_content(s, k, &lane, c)))
}

/// A pane's title (and its repo under it) with a close button.
fn pane_head(k: &Kit, title: String, slug: &str, close_key: &str, close: impl Fn(&mut State) + 'static) -> El {
    row()
        .w_full()
        .gap(8.0)
        .items_start()
        .child(
            col()
                .grow(1.0)
                .shrink()
                .gap(4.0)
                .child(rune::text(title).font(TextStyle { size: 16.0, ..k.ty.ui_medium }).color(k.c.text_strong).wrap())
                .child(rune::text(slug.to_owned()).font(k.ty.small).color(k.c.text_muted)),
        )
        .child(
            IconButton::new(&glyphs::CLOSE)
                .variant(IconButtonVariant::GhostMuted)
                .label(crate::i18n::t("Close"))
                .help(crate::i18n::t("Close"))
                .on_press(close)
                .build(k)
                .key(close_key.to_owned()),
        )
}

/// The Branches page's pane for the picked branch: what it brings to the branch it's compared
/// with (the default one, or the one picked here), as the Comparisons lane pane shows a lane.
pub fn branch_pane(s: &State, k: &Kit) -> Option<El> {
    let (slug, branch) = s.branch_lane_sel.as_ref().filter(|(sl, _)| s.cur.as_ref() == Some(sl))?;
    let mut c = col().w_full().gap(14.0).child(pane_head(k, branch.clone(), slug, "b-pane-close", |s: &mut State| s.close_branch_lane()));
    let Some(base) = s.branch_base_for(slug) else {
        // The repo's default branch isn't known yet; the lane loads once it is (or says why it
        // can't, when the repo's details failed to load).
        let failed = s.data.get(slug).map(|d| &d.meta).filter(|m| !m.loading).and_then(|m| m.error.clone());
        let body = match failed {
            Some(e) => muted(k, tf!("Couldn't compare this branch: {error}", error = e)).wrap(),
            None => loading_line(k, crate::i18n::t("Comparing…")),
        };
        return Some(side_pane(k, "b-pane", c.child(body)));
    };
    c = c.child(
        row().key("b-base-row").w_full().gap(6.0).center_y().child(muted(k, crate::i18n::t("compared with"))).child(
            Select::new("b-base", &[] as &[&str])
                .label(crate::i18n::t("compared with"))
                .current(Some(&base))
                .open(s.branch_base_open)
                .on_toggle(|s: &mut State| s.branch_base_open ^= true)
                .trigger(k)
                .shrink()
                .min_w(0.0),
        ),
    );
    if State::branch_lane_skipped(branch, &base) {
        return Some(side_pane(k, "b-pane", c.child(text("up to date").font(k.ty.small).color(k.c.success.fg))));
    }
    let key = (slug.clone(), branch.clone(), base);
    let rc = s.branch_lanes.get(&key).and_then(|l| l.data.as_ref());
    let body = match (rc, s.open_lane()) {
        (Some(rc), _) if rc.error.is_some() => {
            let e = rc.error.clone().unwrap_or_default();
            c.child(
                row()
                    .key("b-lane-error")
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(muted(k, tf!("Couldn't compare this branch: {error}", error = e)).wrap().grow(1.0).shrink())
                    .child(
                        Button::new(crate::i18n::t("Retry"))
                            .variant(ButtonVariant::GhostMuted)
                            .on_press(move |s: &mut State| {
                                s.branch_lanes.remove(&key);
                                s.select_branch_lane(&key.1);
                            })
                            .build(k)
                            .key("b-lane-retry"),
                    ),
            )
        }
        (Some(_), Some(lane)) => match lane.cell {
            Cell::Missing(b) if b.iter().any(|x| x == branch) => {
                let n = s.rd().and_then(|d| d.branches.data.as_deref()).map_or(0, |list| not_pushed(list, branch));
                let mut c = c.child(rune::text(crate::i18n::t("This branch isn't on GitHub yet")).font(k.ty.ui).color(k.c.text_base).wrap());
                if n > 0 {
                    c = c.child(muted(k, if n == 1 { crate::i18n::t("1 commit not pushed") } else { tf!("{n} commits not pushed") }));
                }
                c
            }
            Cell::Missing(b) => c.child(muted(k, missing_label(b))),
            Cell::Waiting { commits: 0, .. } => c.child(text("up to date").font(k.ty.small).color(k.c.success.fg)),
            Cell::Waiting { .. } => lane_content(s, k, &lane, c),
            Cell::Off => c,
        },
        _ => match s.branch_lanes.get(&key).and_then(|l| l.error.clone()) {
            Some(e) => c.child(muted(k, tf!("Couldn't compare this branch: {error}", error = e)).wrap()),
            None => c.child(loading_line(k, crate::i18n::t("Comparing…"))),
        },
    };
    Some(side_pane(k, "b-pane", body))
}

fn loading_line(k: &Kit, label: String) -> El {
    row().key("b-lane-loading").gap(8.0).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, label))
}

/// Commits on the local branch `name` not on its upstream (0 when it has none or isn't here).
pub(crate) fn not_pushed(branches: &[Branch], name: &str) -> u32 {
    branches.iter().find(|b| !b.remote && b.name == name).map_or(0, |b| b.ahead)
}

/// The open "compared with" list on the branch pane (`options`: the default branch, then the
/// repo's other branches), anchored to its trigger.
pub fn base_list(s: &State, k: &Kit, options: &[String], layer: &mut Vec<El>) {
    let Some((slug, _)) = &s.branch_lane_sel else { return };
    let current = s.branch_base_for(slug);
    let default = s.data.get(slug).and_then(|d| d.meta.data.as_ref()).map(|m| m.default_branch.clone());
    let _trigger = Select::new("b-base", options)
        .label(crate::i18n::t("compared with"))
        .current(current.as_deref())
        .open(true)
        .on_close(|s: &mut State| s.branch_base_open = false)
        .on_choose(move |s: &mut State, v: &str, _| s.set_branch_base(if default.as_deref() == Some(v) { None } else { Some(v.to_owned()) }))
        .build(k, layer);
}

/// What a lane brings (shared by the Comparisons and Branches panes): from → to, PRs and the
/// commits that came from no PR, each with its module map, after `c` (the pane's header).
fn lane_content(s: &State, k: &Kit, lane: &crate::OpenLane, mut c: El) -> El {
    let slug = lane.slug;
    let Cell::Waiting { commits, prs, loose, checked } = lane.cell else { return c };
    let now = now();
    c = c.child(
        row()
            .w_full()
            .gap(6.0)
            .center_y()
            .flex_wrap()
            .child(ref_chip(k, &lane.from, hue(1)))
            .child(text("→").font(k.ty.small).color(k.c.text_faint))
            .child(ref_chip(k, &lane.to, hue(0)))
            .child(muted(k, commits_label(*commits))),
    );
    if checked < commits {
        c = c.child(muted(k, tf!("PRs counted from the first {checked} commits")));
    }

    // Details (labels, summary, size, author profile, files) arrive after the lane opens; until
    // then the rows show what the comparison already knows. Keyed as `State::lane_key`.
    let mut nums: Vec<u64> = prs.iter().map(|p| p.number).collect();
    nums.sort_unstable();
    let key = (slug.to_owned(), nums);
    let load = s.lane_details.get(&key);
    // Summaries, colours and the details by number, worked out when the details or layout came in.
    let model = s.lane_models.get(&key);
    if !prs.is_empty() {
        // Shrinks (labels end in "…") rather than spill past a squeezed pane.
        let mut switch = row().w_full().gap(8.0).center_y().child(
            SegmentedControl::new(&crate::i18n::all(&["By PR", "By contributor"]), Some(usize::from(s.lane_view.by_contributor)))
                .label(crate::i18n::t("Group PRs"))
                .fit()
                .on_pick(|s: &mut State, i, _| s.lane_view.by_contributor = i == 1)
                .build(k)
                .key("cmp-view")
                .shrink()
                .min_w(0.0),
        );
        if load.is_some_and(|l| l.loading) {
            switch = switch.child(Spinner::new().size(12.0).build(k));
        }
        c = c.child(switch);
    }
    if let Some(e) = load.and_then(|l| l.error.as_ref()) {
        c = c.child(
            row()
                .key("cmp-details-error")
                .w_full()
                .gap(8.0)
                .center_y()
                .child(muted(k, tf!("Couldn't load PR details: {error}", error = e)).wrap().grow(1.0).shrink())
                .child(Button::new(crate::i18n::t("Retry")).variant(ButtonVariant::GhostMuted).on_press(|s: &mut State| s.load_lane_details(true)).build(k).key("cmp-details-retry")),
        );
    }

    // Modules learned from this repo's tree, when it has any; the folder diagram otherwise
    // (and while the tree loads, or if it couldn't be read).
    let sel = s.lane_view.folder.as_ref();
    let active = model.and_then(|m| match &m.summary {
        Some(sm) => active_in_summary(sel, sm),
        None => active_folder(sel, &m.folders).cloned(),
    });
    if let Some(m) = model.filter(|m| !m.folders.top.is_empty()) {
        c = c.child(match &m.summary {
            Some(sm) => modules_view(k, sm, active.as_ref(), s.lane_view.modules_open),
            None => changed_code(k, &m.folders, active.as_ref()),
        });
    }

    let no_details = HashMap::new();
    let by_number = model.map_or(&no_details, |m| &m.details);
    let shown = filter_prs(prs, model, active.as_ref());
    let detail_of = |n: u64| by_number.get(&n);
    let pr_map = |n: u64| model.and_then(|m| m.per_pr.get(&n));
    let mut list = col().w_full().gap(2.0);
    if s.lane_view.by_contributor {
        let shown_by_number: HashMap<u64, &WaitingPr> = shown.iter().map(|p| (p.number, *p)).collect();
        for (gi, g) in lane::by_author(&shown, by_number).iter().enumerate() {
            let mut group = col().key(tf!("cmp-author-{gi}")).w_full().gap(2.0).child(author_heading(s, k, g, model.and_then(|m| m.per_author.get(&g.person.login))));
            for p in g.prs.iter().filter_map(|n| shown_by_number.get(n)) {
                group = group.child(pr_row(k, slug, p, detail_of(p.number), false, now, pr_map(p.number)));
            }
            list = list.child(group);
        }
    } else {
        for p in shown {
            list = list.child(pr_row(k, slug, p, detail_of(p.number), true, now, pr_map(p.number)));
        }
    }
    c = c.child(section(k, tf!("Merged PRs waiting ({n})", n = prs.len()), list));

    if !loose.is_empty() {
        let mut list = col().w_full().gap(2.0);
        for (i, l) in loose.iter().enumerate() {
            let sha = click_to_copy(rune::text(l.sha.get(..7).unwrap_or(&l.sha).to_owned()).font(k.ty.mono).color(k.c.text_muted).w(64.0), k, l.sha.clone(), "commit SHA");
            let mut body = col()
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(rune::text(l.subject.clone()).font(k.ty.ui).color(k.c.text_base).wrap())
                .child(rune::text(l.author.clone()).font(k.ty.small).color(k.c.text_faint));
            let open = (slug.to_owned(), l.sha.clone());
            if s.open_commits.contains(&open) {
                body = body.child(commit_map(s, k, &key, &open));
            }
            list = list.child(
                row()
                    .key(tf!("cmp-loose-{i}"))
                    .w_full()
                    .px(8.0)
                    .py(6.0)
                    .gap(10.0)
                    .items_start()
                    .radius(k.radius.md)
                    .hover(k.c.hover)
                    .role("button")
                    .help(crate::i18n::t("Show the modules this commit touched"))
                    .on_press(move |s: &mut State| s.toggle_commit(&open.0, &open.1))
                    .child(sha)
                    .child(body),
            );
        }
        c = c.child(section(k, tf!("Commits without a PR ({n})", n = loose.len()), list));
    }
    c
}

/// Under an opened commit: its module map once its files are in, a spinner while they load,
/// or why they couldn't (press to try again).
fn commit_map(s: &State, k: &Kit, lane: &(String, Vec<u64>), open: &(String, String)) -> El {
    let load = s.commit_files.get(open);
    if let Some(e) = load.filter(|l| l.data.is_none() && !l.loading).and_then(|l| l.error.as_ref()) {
        let retry = open.clone();
        return row()
            .key("cmp-commit-error")
            .w_full()
            .gap(6.0)
            .center_y()
            .role("button")
            .help(e.clone())
            .on_press(move |s: &mut State| {
                s.open_commits.remove(&retry);
                s.toggle_commit(&retry.0, &retry.1);
            })
            .child(muted(k, crate::i18n::t("Couldn't load this commit's files")).wrap().shrink())
            .child(text("Retry").font(k.ty.small_medium).color(k.c.text_accent));
    }
    if load.is_none_or(|l| l.data.is_none()) {
        return row().key("cmp-commit-loading").gap(6.0).center_y().child(Spinner::new().size(10.0).build(k)).child(muted(k, crate::i18n::t("Loading files…")));
    }
    let segs = s.commit_segments.get(lane).and_then(|m| m.get(&open.1));
    match segs {
        Some(v) if !v.is_empty() => item_bar(k, v).key("cmp-commit-map").mt(2.0),
        _ => el(),
    }
}

/// A segment's colour: the main bar's hue for its row, neutral for files outside the rows.
fn tone(k: &Kit, color: usize) -> rune::Color {
    if color == OTHER { k.c.border_base } else { hue(color) }
}

/// An item's module map: a 4 point bar split by module (main-bar colours), each slice naming its
/// module and share on hover.
pub fn item_bar(k: &Kit, segs: &[Segment]) -> El {
    let mut bar = row().w_full().h(4.0).radius(2.0).clip_rounded(2.0).fill(k.c.surface_base);
    for sg in segs.iter().filter(|x| x.share > 0.0) {
        let pct = (sg.share * 100.0).round() as u32;
        bar = bar.child(el().w_frac(sg.share).h_full().fill(tone(k, sg.color)).help(format!("{} · {pct}%", sg.label)));
    }
    bar
}

/// The picked folder among the lane's current folders; none (no filter) when it isn't there any more.
fn active_folder<'a>(sel: Option<&(String, bool)>, f: &'a Folders) -> Option<&'a FolderShare> {
    let (path, root) = sel?;
    f.top.iter().find(|x| &x.path == path && x.root == *root)
}

/// The picked module or shared folder among the summary's rows, as a filter; none when it isn't
/// there any more.
fn active_in_summary(sel: Option<&(String, bool)>, sm: &modules::Summary) -> Option<FolderShare> {
    let (path, root) = sel?;
    if let Some(m) = sm.modules.iter().find(|m| &m.path == path && !root) {
        return Some(FolderShare { label: m.name.clone(), path: m.path.clone(), root: false, added: m.added, removed: m.removed, share: 0.0 });
    }
    sm.shared.top.iter().take(modules::MAX_SHARED_ROWS).find(|x| &x.path == path && x.root == *root).cloned()
}

/// The lane's PRs that touched `active` (all of them with no folder picked): a lookup in the
/// sets the model worked out for its rows.
fn filter_prs<'a>(prs: &'a [WaitingPr], model: Option<&LaneModel>, active: Option<&FolderShare>) -> Vec<&'a WaitingPr> {
    let Some(f) = active else { return prs.iter().collect() };
    let touched = model.and_then(|m| m.touched.get(&(f.path.clone(), f.root)));
    prs.iter().filter(|p| touched.is_some_and(|t| t.contains(&p.number))).collect()
}

/// Pick `fs` as the filter, or drop it when it's already picked.
fn toggle_folder(fs: &FolderShare) -> impl Fn(&mut State) + 'static {
    let pick = (fs.path.clone(), fs.root);
    move |s: &mut State| {
        s.lane_view.folder = if s.lane_view.folder.as_ref() == Some(&pick) { None } else { Some(pick.clone()) };
    }
}

fn folder_label(fs: &FolderShare) -> String {
    if fs.root { crate::i18n::t("(root)") } else { fs.label.clone() }
}

/// "Changed code": one bar split by folder (width = share of lines changed) and a legend;
/// clicking a segment or row filters the PRs to that folder.
fn changed_code(k: &Kit, f: &Folders, active: Option<&FolderShare>) -> El {
    let lines: u64 = f.top.iter().map(|x| x.added + x.removed).sum::<u64>() + f.others_lines;
    let mut bar = row().key("cmp-fold-bar").w_full().h(10.0).radius(4.0).clip_rounded(4.0).fill(k.c.surface_base);
    let mut legend = col().w_full().gap(1.0);
    for (i, fs) in f.top.iter().enumerate() {
        let on = active == Some(fs);
        let tone = if active.is_none() || on { hue(i) } else { rune::Color { a: 0.35, ..hue(i) } };
        bar = bar.child(el().w_frac(fs.share).h_full().fill(tone).role("button").help(fs.path.clone()).on_press(toggle_folder(fs)));
        let pct = (fs.share * 100.0).round() as u32;
        legend = legend.child(
            row()
                .key(tf!("cmp-fold-{i}"))
                .w_full()
                .px(6.0)
                .py(3.0)
                .gap(8.0)
                .center_y()
                .radius(k.radius.md)
                .border(1.0, if on { k.c.border_focus } else { rune::Color::TRANSPARENT })
                .hover(k.c.hover)
                .role("button")
                .help(fs.path.clone())
                .on_press(toggle_folder(fs))
                .child(el().size(8.0, 8.0).radius(2.0).fill(hue(i)))
                .child(rune::text(folder_label(fs)).font(k.ty.small).color(k.c.text_base).grow(1.0).shrink().truncate())
                .child(muted(k, format!("{pct}% · +{} −{}", fs.added, fs.removed))),
        );
    }
    if f.others > 0 {
        let share = if lines == 0 { 0.0 } else { f.others_lines as f32 / lines as f32 };
        bar = bar.child(el().w_frac(share).h_full().fill(k.c.border_base));
        legend = legend.child(
            row()
                .key("cmp-fold-other")
                .w_full()
                .px(6.0)
                .py(3.0)
                .gap(8.0)
                .center_y()
                .child(el().size(8.0, 8.0).radius(2.0).fill(k.c.border_base))
                .child(muted(k, tf!("other {n} folders", n = f.others)).grow(1.0).shrink().truncate())
                .child(muted(k, format!("{}%", (share * 100.0).round() as u32))),
        );
    }
    let mut body = col().w_full().gap(6.0).child(bar).child(legend);
    if let Some(fs) = active {
        let folder = if fs.path.is_empty() { crate::i18n::t("(root)") } else { fs.path.clone() };
        body = body.child(
            row()
                .key("cmp-fold-filter")
                .w_full()
                .gap(8.0)
                .center_y()
                .child(muted(k, tf!("Showing PRs that touched {folder}")).wrap().grow(1.0).shrink())
                .child(Button::new(crate::i18n::t("Clear")).variant(ButtonVariant::GhostMuted).on_press(|s: &mut State| s.lane_view.folder = None).build(k).key("cmp-fold-clear")),
        );
    }
    section(k, crate::i18n::t("Changed code"), body).key("cmp-changed")
}

/// "Modules": each module the lane touches (learned from the repo's tree), how big a share of the
/// change it is, whether it's new or removed, which of its parts changed and in which PRs; then
/// the code outside modules as "Shared code" folders. Clicking a row filters the PRs to it.
fn modules_view(k: &Kit, sm: &modules::Summary, active: Option<&FolderShare>, open: bool) -> El {
    let lines = |a: u64, r: u64| a + r;
    let most = sm.modules.iter().map(|m| lines(m.added, m.removed)).chain(sm.shared.top.iter().map(|f| lines(f.added, f.removed))).max().unwrap_or(0).max(1);
    let picked = |path: &str, root: bool| active.is_some_and(|a| a.path == path && a.root == root);
    let row_for = |key: String, i: usize, name: String, badge: Option<(String, Outcome)>, detail: String, prs: String, size: u64, on: bool, pick: (String, bool)| {
        let mut head = row().w_full().gap(6.0).center_y().child(rune::text(name).font(k.ty.small_medium).color(k.c.text_base).shrink().truncate());
        if let Some((label, o)) = badge {
            head = head.child(chip(k, label, Some(o)));
        }
        head = head.child(spacer()).child(muted(k, prs));
        let bar = row().w_full().h(6.0).radius(3.0).fill(k.c.surface_base).child(el().w_frac(size as f32 / most as f32).h_full().radius(3.0).fill(hue(i)));
        row()
            .key(key)
            .w_full()
            .px(6.0)
            .py(5.0)
            .radius(k.radius.md)
            .border(1.0, if on { k.c.border_focus } else { rune::Color::TRANSPARENT })
            .hover(k.c.hover)
            .role("button")
            .on_press(move |s: &mut State| {
                s.lane_view.folder = if s.lane_view.folder.as_ref() == Some(&pick) { None } else { Some(pick.clone()) };
            })
            .child(col().w_full().gap(3.0).child(head).child(bar).child(muted(k, detail).wrap()))
    };
    let toggle = text(if open { "Hide details" } else { "Show details" })
        .font(k.ty.small_medium)
        .color(k.c.text_accent)
        .hover(k.c.hover)
        .role("button")
        .on_press(|s: &mut State| s.lane_view.modules_open = !s.lane_view.modules_open)
        .key("cmp-mod-toggle");
    let shared: Vec<&FolderShare> = sm.shared.top.iter().take(modules::MAX_SHARED_ROWS).collect();
    // The title line carries the Show / Hide details toggle, so neither state spends a row on it.
    let titled = |body: El, toggle: El| {
        col()
            .key("cmp-modules")
            .w_full()
            .gap(6.0)
            .child(row().w_full().center_y().child(rune::text(crate::i18n::t("Modules")).font(k.ty.small_medium).color(k.c.text_muted)).child(spacer()).child(toggle))
            .child(body)
    };
    if !open {
        return titled(modules_bar(k, sm, &shared, active), toggle);
    }
    let mut body = col().w_full().gap(2.0);
    for (i, m) in sm.modules.iter().enumerate() {
        let badge = match m.kind {
            modules::Kind::New => Some((crate::i18n::t("NEW"), Outcome::Success)),
            modules::Kind::Removed => Some((crate::i18n::t("REMOVED"), Outcome::Failure)),
            modules::Kind::Changed => None,
        };
        let mut parts: Vec<String> = m.parts.iter().map(|(p, n)| if *n > 1 { format!("{p} ×{n}") } else { p.clone() }).collect();
        if m.other > 0 {
            parts.push(if m.other > 1 { tf!("other ×{n}", n = m.other) } else { crate::i18n::t("other") });
        }
        let detail = format!("{} · +{} −{}", parts.join(" · "), m.added, m.removed);
        let prs = m.prs.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
        body = body.child(row_for(tf!("cmp-mod-{}", m.path), i, m.name.clone(), badge, detail, prs, lines(m.added, m.removed), picked(&m.path, false), (m.path.clone(), false)));
    }
    if sm.more_modules > 0 {
        body = body.child(row().px(6.0).child(muted(k, tf!("{n} more modules", n = sm.more_modules))));
    }
    if !shared.is_empty() {
        body = body.child(row().px(6.0).pt(6.0).child(rune::text(crate::i18n::t("Shared code")).font(k.ty.small_medium).color(k.c.text_muted)));
        for (i, fs) in shared.iter().enumerate() {
            let name = if fs.path.is_empty() { crate::i18n::t("(root)") } else { fs.path.clone() };
            let detail = format!("+{} −{}", fs.added, fs.removed);
            body = body.child(row_for(tf!("cmp-shared-{i}"), sm.modules.len() + i, name, None, detail, String::new(), lines(fs.added, fs.removed), picked(&fs.path, fs.root), (fs.path.clone(), fs.root)));
        }
    }
    if let Some(f) = filter_line(k, active) {
        body = body.child(f);
    }
    titled(body, toggle)
}

/// "Showing PRs that touched …" with Clear, when a module or folder is picked.
fn filter_line(k: &Kit, active: Option<&FolderShare>) -> Option<El> {
    let a = active?;
    let folder = if a.path.is_empty() { crate::i18n::t("(root)") } else { a.path.clone() };
    Some(
        row()
            .key("cmp-fold-filter")
            .w_full()
            .pt(4.0)
            .gap(8.0)
            .center_y()
            .child(muted(k, tf!("Showing PRs that touched {folder}")).wrap().grow(1.0).shrink())
            .child(Button::new(crate::i18n::t("Clear")).variant(ButtonVariant::GhostMuted).on_press(|s: &mut State| s.lane_view.folder = None).build(k).key("cmp-fold-clear")),
    )
}

/// The collapsed Modules section: one bar split by module, then shared code (width = share of
/// lines changed), and a short legend that wraps; segments and legend items filter the PRs.
fn modules_bar(k: &Kit, sm: &modules::Summary, shared: &[&FolderShare], active: Option<&FolderShare>) -> El {
    // (name, new?, lines, colour index, filter)
    let mut items: Vec<(String, bool, u64, usize, (String, bool))> = sm
        .modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.clone(), m.kind == modules::Kind::New, m.added + m.removed, i, (m.path.clone(), false)))
        .collect();
    items.extend(shared.iter().enumerate().map(|(i, f)| {
        let name = if f.path.is_empty() { crate::i18n::t("(root)") } else { f.path.clone() };
        (name, false, f.added + f.removed, sm.modules.len() + i, (f.path.clone(), f.root))
    }));
    let total: u64 = items.iter().map(|x| x.2).sum::<u64>().max(1);
    let pick = |p: (String, bool)| {
        move |s: &mut State| {
            s.lane_view.folder = if s.lane_view.folder.as_ref() == Some(&p) { None } else { Some(p.clone()) };
        }
    };
    let mut bar = row().key("cmp-mod-bar").w_full().h(10.0).radius(4.0).clip_rounded(4.0).fill(k.c.surface_base);
    let mut legend = row().w_full().gap(10.0).flex_wrap().center_y();
    for (name, new, lines, i, filter) in items {
        let on = active.is_some_and(|a| a.path == filter.0 && a.root == filter.1);
        let tone = if active.is_none() || on { hue(i) } else { rune::Color { a: 0.35, ..hue(i) } };
        let pct = (lines as f32 * 100.0 / total as f32).round() as u32;
        bar = bar.child(el().w_frac(lines as f32 / total as f32).h_full().fill(tone).role("button").help(format!("{name} · {pct}%")).on_press(pick(filter.clone())));
        let mut item = row()
            .key(tf!("cmp-mod-leg-{i}"))
            .gap(5.0)
            .center_y()
            .px(4.0)
            .radius(k.radius.sm)
            .border(1.0, if on { k.c.border_focus } else { rune::Color::TRANSPARENT })
            .hover(k.c.hover)
            .role("button")
            .on_press(pick(filter))
            .child(el().size(8.0, 8.0).radius(2.0).fill(hue(i)))
            .child(rune::text(name).font(k.ty.small).color(k.c.text_base))
            .child(muted(k, format!("{pct}%")));
        if new {
            item = item.child(rune::text(crate::i18n::t("NEW")).font(k.ty.text_xsmall).color(k.c.success.fg));
        }
        legend = legend.child(item);
    }
    let mut body = col().w_full().gap(6.0).child(bar).child(legend);
    if let Some(f) = filter_line(k, active) {
        body = body.child(f);
    }
    body
}

/// One author's heading in By contributor: who, where they work, what they brought, their profile.
/// The author's GitHub picture once it has loaded; until then (or without one) their initial on
/// a colour picked from their login.
fn avatar(s: &State, k: &Kit, p: &crate::compare::Person, size: f32) -> El {
    if let Some(img) = s.avatars.get(&p.avatar_url).filter(|i| i.size().is_some() && i.error().is_none()) {
        return el().size(size, size).radius(size / 2.0).clip_rounded(size / 2.0).pixels(img.pixels()).key(tf!("cmp-avatar-{}", p.login));
    }
    let initial = p.name.chars().chain(p.login.chars()).next().map_or("?".to_owned(), |c| c.to_uppercase().to_string());
    row()
        .size(size, size)
        .radius(size / 2.0)
        .center()
        .fill(hue(p.login.bytes().map(usize::from).sum()))
        .child(rune::text(initial).font(TextStyle { size: size * 0.45, ..k.ty.ui_medium }).color(rune::Color::WHITE))
        .key(tf!("cmp-avatar-{}", p.login))
}

fn author_heading(s: &State, k: &Kit, g: &lane::AuthorGroup, map: Option<&Vec<Segment>>) -> El {
    let p = &g.person;
    let mut who = row().w_full().gap(6.0).center_y().flex_wrap();
    if p.name.is_empty() {
        who = who.child(rune::text(p.login.clone()).font(k.ty.ui_medium).color(k.c.text_base));
    } else {
        who = who.child(rune::text(p.name.clone()).font(k.ty.ui_medium).color(k.c.text_base)).child(muted(k, format!("@{}", p.login)));
    }
    if !p.company.is_empty() {
        who = who.child(muted(k, format!("· {}", p.company)));
    }
    let (n, a, d) = (g.prs.len(), g.added, g.removed);
    let totals = if n == 1 { tf!("1 PR · +{a} −{d}") } else { tf!("{n} PRs · +{a} −{d}") };
    let mut stats = row().w_full().gap(8.0).center_y().child(muted(k, totals).grow(1.0).shrink());
    if !p.login.is_empty() && p.login != "ghost" && !p.bot {
        let url = format!("https://github.com/{}", p.login);
        stats = stats.child(
            text("Profile")
                .font(k.ty.small_medium)
                .color(k.c.text_accent)
                .hover(k.c.hover)
                .role("button")
                .help(url.clone())
                .on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url))
                .key(tf!("cmp-profile-{}", p.login)),
        );
    }
    row()
        .w_full()
        .px(8.0)
        .pt(10.0)
        .pb(4.0)
        .gap(10.0)
        .items_start()
        .child(avatar(s, k, p, 32.0))
        .child(col().grow(1.0).shrink().gap(2.0).child(who).child(stats).children(map.filter(|m| !m.is_empty()).map(|m| item_bar(k, m).key("cmp-author-map").mt(2.0))))
}

/// A merged PR waiting: number, title, labels, summary, then who/when and its size.
fn pr_row(k: &Kit, slug: &str, p: &WaitingPr, d: Option<&PrDetail>, with_author: bool, now: i64, map: Option<&Vec<Segment>>) -> El {
    let (sl, n) = (slug.to_owned(), p.number);
    let mut body = col().grow(1.0).shrink().gap(2.0).child(rune::text(p.title.clone()).font(k.ty.ui).color(k.c.text_base).wrap());
    if let Some(d) = d {
        if !d.labels.is_empty() {
            body = body.child(d.labels.iter().fold(row().w_full().gap(4.0).flex_wrap(), |r, l| r.child(chip(k, l.clone(), None))));
        }
        if !d.summary.is_empty() {
            body = body.child(muted(k, d.summary.clone()).wrap());
        }
    }
    let when = ago_str(&p.merged_at, now);
    let who_when = if with_author { format!("{} · {when}", p.author) } else { when };
    let mut meta = row().w_full().gap(8.0).center_y().flex_wrap().child(rune::text(who_when).font(k.ty.small).color(k.c.text_faint));
    if let Some(d) = d {
        meta = meta.child(muted(k, format!("+{} −{}", d.additions, d.deletions)));
    }
    row()
        .key(tf!("cmp-pr-{n}"))
        .w_full()
        .px(8.0)
        .py(6.0)
        .gap(10.0)
        .items_start()
        .radius(k.radius.md)
        .hover(k.c.hover)
        .role("button")
        .on_press(move |s: &mut State| s.open_pr(&sl, n))
        .child(rune::text(format!("#{}", p.number)).font(k.ty.mono).color(k.c.text_muted).w(44.0))
        .child(body.child(meta).children(map.filter(|m| !m.is_empty()).map(|m| item_bar(k, m).key("cmp-pr-map").mt(2.0))))
}

fn section(k: &Kit, title: String, body: El) -> El {
    col().w_full().gap(6.0).child(rune::text(title).font(k.ty.small_medium).color(k.c.text_muted)).child(body)
}

/// A field of the open dialog's line `i` (`to`: the To field). Edits only reach an open
/// dialog; one landing after it closed goes nowhere.
fn field(s: &mut State, i: usize, to: bool) -> &mut TextEdit {
    match s.compare_edit.as_mut().and_then(|e| e.rows.get_mut(i)) {
        Some(r) if to => &mut r.1,
        Some(r) => &mut r.0,
        None => Box::leak(Box::new(TextEdit::new(""))),
    }
}

/// "Comparisons for owner/repo": per shared comparison, this repo's From / To and an On switch.
pub fn repo_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(edit) = &s.compare_edit else { return };
    let mut lines = col().w_full().gap(10.0);
    for (i, (c, (from, to, on))) in s.prefs.comparisons.iter().zip(&edit.rows).enumerate() {
        let input = |e: &TextEdit, label: &str, shared: &str, to: bool| {
            TextInput::new(e).label(crate::i18n::t(label)).placeholder(shared.to_owned()).width(None).on_edit(move |s: &mut State| field(s, i, to)).build(k).key(tf!("cmp-{}-{i}", if to { "to" } else { "from" }))
        };
        lines = lines.child(
            row()
                .key(tf!("cmp-line-{i}"))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(rune::text(c.name.clone()).font(k.ty.ui).color(k.c.text_base).w(120.0).truncate())
                .child(row().w(0.0).grow(1.0).child(input(from, "From", &c.from, false)))
                .child(text("→").font(k.ty.ui).color(k.c.text_faint))
                .child(row().w(0.0).grow(1.0).child(input(to, "To", &c.to, true)))
                .child(
                    Switch::new(*on)
                        .label(crate::i18n::t("On"))
                        .hide_label()
                        .on_toggle(move |s: &mut State| {
                            if let Some(r) = s.compare_edit.as_mut().and_then(|e| e.rows.get_mut(i)) {
                                r.2 ^= true;
                            }
                        })
                        .build(k)
                        .key(tf!("cmp-on-{i}")),
                ),
        );
    }
    let body = col().w_full().px(16.0).pb(16.0).gap(14.0).child(lines).child(
        row()
            .w_full()
            .gap(8.0)
            .child(Button::new(crate::i18n::t("Reset to shared")).variant(ButtonVariant::GhostMuted).on_press(|s: &mut State| s.reset_compare_edit()).build(k).key("cmp-reset"))
            .child(spacer())
            .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.compare_edit = None).build(k).key("cmp-cancel"))
            .child(Button::new(crate::i18n::t("Save")).variant(ButtonVariant::Submit).on_press(|s: &mut State| s.save_compare_edit()).build(k).key("cmp-save")),
    );
    Dialog::new("compare-edit", tf!("Comparisons for {slug}", slug = edit.slug)).open(true).fit().width(620.0).on_close(|s: &mut State| s.compare_edit = None).child(body).build(k, layer);
}

#[cfg(test)]
mod tests {
    use super::{active_folder, filter_prs, sections};
    use crate::CompareScope;
    use crate::compare::{PrDetail, RepoCompare, WaitingPr};
    use crate::config::Group;
    use crate::lane::by_author;
    use crate::lane_model::build;

    fn repo(slug: &str) -> RepoCompare {
        RepoCompare { slug: slug.into(), cells: Vec::new(), error: None }
    }
    fn names(v: &[(CompareScope, Vec<&RepoCompare>)]) -> Vec<(CompareScope, Vec<String>)> {
        v.iter().map(|(sc, rs)| (sc.clone(), rs.iter().map(|r| r.slug.clone()).collect())).collect()
    }

    #[test]
    fn module_and_shared_rows_filter_the_prs() {
        let mut tree = Vec::new();
        for m in ["agent", "report", "auth"] {
            tree.extend([format!("src/feature/{m}/view/a.vue"), format!("src/feature/{m}/service/a.ts")]);
        }
        let layout = crate::modules::learn(&tree);
        let prs = [pr(1, "a"), pr(2, "b"), pr(3, "c")];
        let details = [
            detail(1, "a", &["src/feature/agent/view/a.vue"]),
            detail(2, "b", &["src/feature/report/service/a.ts"]),
            detail(3, "c", &["src/shared/component/Table.vue", "src/feature/agent/service/a.ts"]),
        ];
        let m = build(&details, Some(&layout));
        let sm = m.summary.clone().unwrap();
        let agent = super::active_in_summary(Some(&("src/feature/agent".to_string(), false)), &sm);
        assert_eq!(filter_prs(&prs, Some(&m), agent.as_ref()).iter().map(|p| p.number).collect::<Vec<_>>(), [1, 3]);
        let shared = super::active_in_summary(Some(&("src/shared/component".to_string(), true)), &sm);
        assert_eq!(filter_prs(&prs, Some(&m), shared.as_ref()).iter().map(|p| p.number).collect::<Vec<_>>(), [3]);
        assert!(super::active_in_summary(Some(&("src/feature/gone".to_string(), false)), &sm).is_none());
    }

    #[test]
    fn sections_follow_groups_then_other() {
        let groups = vec![Group { name: "office".into(), repos: vec!["a/1".into(), "a/2".into()] }];
        let rows = [repo("a/1"), repo("a/2"), repo("b/3")];
        assert_eq!(
            names(&sections(&groups, &rows, None)),
            vec![(CompareScope::Group("office".into()), vec!["a/1".into(), "a/2".into()]), (CompareScope::Other, vec!["b/3".into()])]
        );
    }

    #[test]
    fn scope_shows_only_the_picked_section() {
        let groups = vec![Group { name: "office".into(), repos: vec!["a/1".into()] }, Group { name: "games".into(), repos: vec!["g/1".into()] }];
        let rows = [repo("a/1"), repo("g/1"), repo("b/3")];
        let games = sections(&groups, &rows, Some(&CompareScope::Group("games".into())));
        assert_eq!(names(&games), vec![(CompareScope::Group("games".into()), vec!["g/1".into()])]);
        let other = sections(&groups, &rows, Some(&CompareScope::Other));
        assert_eq!(names(&other), vec![(CompareScope::Other, vec!["b/3".into()])]);
    }

    #[test]
    fn scope_for_a_removed_group_shows_everything() {
        let groups = vec![Group { name: "office".into(), repos: vec!["a/1".into()] }];
        let rows = [repo("a/1"), repo("b/3")];
        assert_eq!(sections(&groups, &rows, Some(&CompareScope::Group("gone".into()))).len(), 2);
    }

    fn pr(number: u64, author: &str) -> WaitingPr {
        WaitingPr { number, title: String::new(), author: author.into(), merged_at: format!("2026-11-{number:02}") }
    }
    fn detail(number: u64, login: &str, files: &[&str]) -> PrDetail {
        let mut d = PrDetail { number, files: files.iter().map(|p| (p.to_string(), 1, 0)).collect(), ..PrDetail::default() };
        d.author.login = login.into();
        d
    }

    #[test]
    fn folder_filter_keeps_prs_that_touched_it() {
        let prs = [pr(1, "a"), pr(2, "b"), pr(3, "a")];
        let list = [detail(1, "a", &["src/agent/x.ts"]), detail(2, "b", &["src/report/y.ts"]), detail(3, "a", &["src/report/z.ts", "src/agent/w.ts"])];
        let m = build(&list, None);
        let f = &m.folders;
        let sel = ("src/report".to_string(), false);
        let active = active_folder(Some(&sel), f);
        assert_eq!(active.map(|f| f.label.as_str()), Some("report"));
        let shown: Vec<u64> = filter_prs(&prs, Some(&m), active).iter().map(|p| p.number).collect();
        assert_eq!(shown, [2, 3]);
        // By contributor: groups come from the filtered PRs, so an author left with none is gone.
        let sel = ("src/agent".to_string(), false);
        let g = by_author(&filter_prs(&prs, Some(&m), active_folder(Some(&sel), f)), &m.details);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].person.login, "a");
        assert_eq!(g[0].prs, [3, 1]);
    }

    #[test]
    fn folder_filter_off_when_unset_or_gone() {
        let prs = [pr(1, "a"), pr(2, "b")];
        let m = build(&[detail(1, "a", &["src/agent/x.ts"]), detail(2, "b", &["src/report/y.ts"])], None);
        let f = &m.folders;
        assert_eq!(filter_prs(&prs, Some(&m), active_folder(None, f)).len(), 2);
        let gone = ("lib/old".to_string(), false);
        assert!(active_folder(Some(&gone), f).is_none());
        assert_eq!(filter_prs(&prs, Some(&m), active_folder(Some(&gone), f)).len(), 2);
        // Same path but not the root group: not a match.
        assert!(active_folder(Some(&("src/agent".to_string(), true)), f).is_none());
    }
}

#[cfg(test)]
mod bar_tests {
    use super::{not_pushed, tone};
    use crate::lane_model::OTHER;
    use crate::model::Branch;
    use crate::ui::hue;

    #[test]
    fn other_is_neutral_and_rows_keep_the_main_bar_hue() {
        let k = rune_kit::Kit::neutral(true);
        assert_eq!(tone(&k, OTHER), k.c.border_base);
        assert_eq!(tone(&k, 2), hue(2));
    }

    #[test]
    fn not_pushed_counts_the_local_branch_ahead() {
        let b = |name: &str, remote: bool, ahead: u32| Branch { name: name.into(), remote, ahead, ..Branch::default() };
        let list = [b("origin/feat", true, 0), b("feat", false, 4), b("dev", false, 1)];
        assert_eq!(not_pushed(&list, "feat"), 4);
        assert_eq!(not_pushed(&list, "gone"), 0);
    }
}
