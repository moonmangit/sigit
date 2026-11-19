//! The Dashboard's Comparisons tab: each tracked repo as a branch flow, one lane per shared
//! comparison (from → to, a bar as long as what is waiting), a pane listing what a lane counts, and the
//! per-repo dialog that retargets or turns comparisons off.

use rune::{El, TextEdit, TextStyle, col, el, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, Switch, TextInput, glyphs};

use super::{I_GEAR, click_to_copy, hue, muted, pending, ref_chip, scroll, side_pane};
use crate::compare::{Cell, Comparison, LooseCommit, RepoCompare, WaitingPr, effective, is_changed};
use crate::i18n::text;
use crate::model::{ago_str, now};
use crate::State;

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
        if let Some(pane) = detail(s, k, rows) {
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
fn flows(s: &State, k: &Kit, rows: &[RepoCompare]) -> El {
    let cmps = &s.prefs.comparisons;
    let mut busy: Vec<&RepoCompare> = rows.iter().filter(|r| r.error.is_some() || r.cells.iter().any(|c| waiting(c) > 0)).collect();
    busy.sort_by_key(|r| std::cmp::Reverse(r.cells.iter().map(waiting).max().unwrap_or(0)));
    let calm: Vec<&RepoCompare> = rows.iter().filter(|r| r.error.is_none() && r.cells.iter().all(|c| waiting(c) == 0)).collect();
    let mut c = col().key("cmp-flows").w_full().gap(12.0);
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
        let pick = (l.slug.to_owned(), l.j);
        r = r.hover(k.c.hover).role("button").on_press(move |s: &mut State| {
            s.compare_sel = if s.compare_sel.as_ref() == Some(&pick) { None } else { Some(pick.clone()) };
        });
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
fn detail(s: &State, k: &Kit, rows: &[RepoCompare]) -> Option<El> {
    let (slug, j) = s.compare_sel.as_ref()?;
    let rc = rows.iter().find(|r| &r.slug == slug)?;
    let Some(Cell::Waiting { commits, prs, loose, checked }) = rc.cells.get(*j) else { return None };
    if *commits == 0 {
        return None;
    }
    let shared = s.prefs.comparisons.get(*j)?;
    let eff = effective(&s.prefs.comparisons, &s.prefs.comparison_overrides, slug).get(*j).cloned().flatten().unwrap_or_else(|| shared.clone());
    let now = now();
    let mut c = col().w_full().gap(14.0);
    c = c.child(
        row()
            .w_full()
            .gap(8.0)
            .items_start()
            .child(
                col()
                    .grow(1.0)
                    .shrink()
                    .gap(4.0)
                    .child(rune::text(shared.name.clone()).font(TextStyle { size: 16.0, ..k.ty.ui_medium }).color(k.c.text_strong).wrap())
                    .child(rune::text(slug.clone()).font(k.ty.small).color(k.c.text_muted)),
            )
            .child(
                IconButton::new(&glyphs::CLOSE)
                    .variant(IconButtonVariant::GhostMuted)
                    .label(crate::i18n::t("Close"))
                    .help(crate::i18n::t("Close"))
                    .on_press(|s: &mut State| s.compare_sel = None)
                    .build(k)
                    .key("cmp-close"),
            ),
    );
    c = c.child(
        row()
            .gap(6.0)
            .center_y()
            .child(ref_chip(k, &eff.from, hue(1)))
            .child(text("→").font(k.ty.small).color(k.c.text_faint))
            .child(ref_chip(k, &eff.to, hue(0)))
            .child(muted(k, commits_label(*commits))),
    );
    if checked < commits {
        c = c.child(muted(k, tf!("PRs counted from the first {checked} commits")));
    }

    let mut list = col().w_full().gap(2.0);
    for (i, p) in prs.iter().enumerate() {
        let (sl, n) = (slug.clone(), p.number);
        list = list.child(
            row()
                .key(tf!("cmp-pr-{i}"))
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
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .gap(2.0)
                        .child(rune::text(p.title.clone()).font(k.ty.ui).color(k.c.text_base).wrap())
                        .child(rune::text(format!("{} · {}", p.author, ago_str(&p.merged_at, now))).font(k.ty.small).color(k.c.text_faint)),
                ),
        );
    }
    c = c.child(section(k, tf!("Merged PRs waiting ({n})", n = prs.len()), list));

    if !loose.is_empty() {
        let mut list = col().w_full().gap(2.0);
        for (i, l) in loose.iter().enumerate() {
            let sha = click_to_copy(rune::text(l.sha.get(..7).unwrap_or(&l.sha).to_owned()).font(k.ty.mono).color(k.c.text_muted).w(64.0), k, l.sha.clone(), "commit SHA");
            list = list.child(
                row()
                    .key(tf!("cmp-loose-{i}"))
                    .w_full()
                    .px(8.0)
                    .py(6.0)
                    .gap(10.0)
                    .items_start()
                    .child(sha)
                    .child(
                        col()
                            .grow(1.0)
                            .shrink()
                            .gap(2.0)
                            .child(rune::text(l.subject.clone()).font(k.ty.ui).color(k.c.text_base).wrap())
                            .child(rune::text(l.author.clone()).font(k.ty.small).color(k.c.text_faint)),
                    ),
            );
        }
        c = c.child(section(k, tf!("Commits without a PR ({n})", n = loose.len()), list));
    }
    Some(side_pane(k, "cmp-pane", c))
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
