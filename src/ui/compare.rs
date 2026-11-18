//! The Dashboard's Comparisons tab: one row per tracked repo, one column per shared branch
//! comparison ("N commits · M PRs" waiting), a pane listing what a cell counts, and the
//! per-repo dialog that retargets or turns comparisons off.

use rune::{El, TextEdit, TextStyle, col, el, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, Switch, TextInput, glyphs};

use super::{I_GEAR, click_to_copy, hue, muted, pending, ref_chip, scroll, side_pane};
use crate::compare::{Cell, Comparison, RepoCompare, effective, is_changed};
use crate::i18n::text;
use crate::model::{ago_str, now};
use crate::State;

/// Shares of the table's width: the repo name, then each comparison.
const NAME_GROW: f32 = 1.0;
const MENU_W: f32 = 32.0;

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
        let mut panes = vec![super::split::pane(scroll("cmp-scroll", table(s, k, rows)), 0.64, 520.0)];
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

fn table(s: &State, k: &Kit, rows: &[RepoCompare]) -> El {
    let cmps = &s.prefs.comparisons;
    let mut head = row().key("cmp-head").w_full().px(14.0).py(10.0).gap(12.0).child(el().w(0.0).grow(NAME_GROW));
    for (j, c) in cmps.iter().enumerate() {
        head = head.child(
            col()
                .key(tf!("cmp-h-{j}"))
                .w(0.0)
                .grow(1.0)
                .gap(2.0)
                .child(rune::text(c.name.clone()).font(k.ty.small_medium).color(k.c.text_base).truncate())
                .child(rune::text(arrow(c)).font(TextStyle { size: 11.0, ..k.ty.mono }).color(k.c.text_faint).truncate()),
        );
    }
    let mut t = col().w_full().radius(k.radius.lg).fill(k.c.layer_01).border(1.0, k.c.border_weak).clip_rounded(k.radius.lg).child(head.child(el().w(MENU_W)));
    for (i, rc) in rows.iter().enumerate() {
        t = t.child(rule(k)).child(repo_row(s, k, i, rc, cmps));
    }
    t
}

fn rule(k: &Kit) -> El {
    el().w_full().h(1.0).fill(k.c.border_weak)
}

fn arrow(c: &Comparison) -> String {
    format!("{} → {}", c.from, c.to)
}

fn repo_row(s: &State, k: &Kit, i: usize, rc: &RepoCompare, cmps: &[Comparison]) -> El {
    let error = rc.error.as_ref().map(|e| rune::text(e.clone()).font(k.ty.small).color(k.c.danger.fg).wrap().w_full());
    let has_data = rc.cells.iter().any(|c| !matches!(c, Cell::Off));
    let mut name = col().w(0.0).grow(NAME_GROW).gap(3.0).child(rune::text(rc.slug.clone()).font(k.ty.ui).color(k.c.text_base).truncate());
    let mut r = row().key(tf!("cmp-r-{i}")).w_full().px(14.0).py(8.0).gap(12.0).center_y();
    if let (Some(e), false) = (error.clone(), has_data) {
        // Nothing came back for this repo: its error takes the cells' place.
        r = r.child(name).child(row().w(0.0).grow(cmps.len().max(1) as f32).child(e));
    } else {
        if let Some(e) = error {
            name = name.child(e);
        }
        r = r.child(name);
        let eff = effective(cmps, &s.prefs.comparison_overrides, &rc.slug);
        for (j, c) in cmps.iter().enumerate() {
            let cell = rc.cells.get(j).unwrap_or(&Cell::Off);
            let changed = is_changed(&s.prefs.comparison_overrides, &rc.slug, &c.name).then(|| eff.get(j).cloned().flatten());
            r = r.child(cell_view(s, k, &rc.slug, j, cell, changed).key(tf!("cmp-c-{i}-{j}")));
        }
    }
    let slug = rc.slug.clone();
    let menu = IconButton::new(&I_GEAR)
        .variant(IconButtonVariant::GhostMuted)
        .label(crate::i18n::t("Edit comparisons for this repo…"))
        .help(crate::i18n::t("Edit comparisons for this repo…"))
        .on_press(move |s: &mut State| s.open_compare_edit(&slug))
        .build(k)
        .key(tf!("cmp-edit-{i}"));
    r.child(row().w(MENU_W).child(menu))
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

/// One cell. `changed`: Some when this repo changed it, holding its own branches (None inside
/// when it's turned off).
fn cell_view(s: &State, k: &Kit, slug: &str, j: usize, cell: &Cell, changed: Option<Option<Comparison>>) -> El {
    let quiet = |label: String| text(label).font(k.ty.small).color(k.c.text_faint).wrap().shrink();
    let mut main = row().gap(6.0).center_y();
    let mut c = col().w(0.0).grow(1.0).px(8.0).py(5.0).gap(2.0).radius(k.radius.md);
    match cell {
        Cell::Off => main = main.child(quiet(crate::i18n::t("off"))),
        Cell::Missing(b) => main = main.child(quiet(missing_label(b))),
        Cell::Waiting { commits: 0, .. } => main = main.child(quiet(crate::i18n::t("up to date"))),
        Cell::Waiting { commits, prs, checked, .. } => {
            let label = format!("{} · {}", commits_label(*commits), prs_label(prs.len()));
            main = main.child(rune::text(label).font(k.ty.small_medium).color(k.c.text_base).wrap().shrink());
            if checked < commits {
                c = c.help(tf!("PRs counted from the first {checked} commits"));
            }
            let sel = s.compare_sel.as_ref().is_some_and(|(sl, i)| sl == slug && *i == j);
            if sel {
                c = c.fill(k.c.surface_hover);
            }
            let slug = slug.to_owned();
            c = c.hover(k.c.hover).role("button").on_press(move |s: &mut State| {
                let pick = (slug.clone(), j);
                s.compare_sel = if s.compare_sel.as_ref() == Some(&pick) { None } else { Some(pick) };
            });
        }
    }
    if changed.is_some() {
        main = main.child(el().size(6.0, 6.0).radius(3.0).fill(hue(2)).help(crate::i18n::t("Changed for this repo")));
    }
    c = c.child(main);
    if let Some(Some(e)) = changed {
        c = c.child(rune::text(arrow(&e)).font(TextStyle { size: 11.0, ..k.ty.mono }).color(k.c.text_faint).truncate());
    }
    c
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
