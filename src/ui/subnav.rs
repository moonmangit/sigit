//! Sidebar 2: the sections of what sidebar 1 picked, and the filters they share. The page's
//! own header (and the window title) already names the repo or group, so this doesn't.
//!
//! - **A repository**: Pull requests, Actions, Environments, Commits, Branches.
//! - **A group**: Overview (its whole dashboard), then one panel at a time (Pull requests,
//!   Actions, Environments), then its repos, to jump into one.
//!
//! Every section shows how many it has (a spinner while it loads). The filters below (a date
//! range and a contributor, picked from a list with profile pictures) stay as you move between
//! sections, repos and groups. Pull requests, Actions and Commits ask GitHub / git for exactly
//! that, so they reach past the first page; a group's lists narrow what they loaded. Drag the
//! right edge to resize (double-click it for the default width).

use rune::{Cursor, Drag, El, Icon, Phase, TextStyle, col, el, row, spacer};
use rune_kit::shell::SidebarRow;
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant, Kit, PopupItem, PopupList, Select, Spinner, TextInput, glyphs};

use super::{I_DASH, I_PR, I_REPO, hue, muted};
use crate::i18n::text;
use crate::model::FILTER_DAYS;
use crate::{GROUP_TABS, Load, Page, SUBNAV_W, State};

const ROW: f32 = 32.0;
const PREF: &str = "subnav";

macro_rules! icon {
    ($id:ident, $name:literal, $body:literal) => {
        static $id: Icon = Icon { name: $name, view: 16.0, body: $body };
    };
}
icon!(I_RUN, "g-run", r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M6.8 5.8L10.2 8L6.8 10.2Z" stroke="currentColor" stroke-width="1.1" stroke-linejoin="round" fill="none"/>"#);
icon!(I_ENV, "g-env", r#"<rect x="2.5" y="3" width="11" height="4" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="2.5" y="9" width="11" height="4" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="5" cy="5" r="0.7" fill="currentColor"/><circle cx="5" cy="11" r="0.7" fill="currentColor"/>"#);
icon!(I_COMMIT, "g-commit", r#"<circle cx="8" cy="8" r="2.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M1.5 8H5.5M10.5 8H14.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#);
icon!(I_BRANCH, "g-branch", r#"<circle cx="5" cy="3.8" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="5" cy="12.2" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11" cy="5.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M5 5.3V10.7M11 7C11 9 8.5 9.5 5.6 10.8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#);
icon!(I_PERSON, "g-person", r#"<circle cx="8" cy="5.5" r="2.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M3 13.5C3.6 11 5.6 9.5 8 9.5S12.4 11 13 13.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#);

fn page_icon(p: Page) -> &'static Icon {
    match p {
        Page::Pulls => &I_PR,
        Page::Actions => &I_RUN,
        Page::Envs => &I_ENV,
        Page::Commits => &I_COMMIT,
        Page::Branches => &I_BRANCH,
        Page::Inbox | Page::Group => &I_DASH,
    }
}

/// Sidebar 2's width: what you dragged it to, or the default.
pub fn width(s: &State) -> f32 {
    let (def, min, max) = SUBNAV_W;
    s.prefs.splits.get(PREF).and_then(|v| v.first().copied()).filter(|w| w.is_finite()).map_or(def, |w| w.clamp(min, max))
}

fn heading(k: &Kit, label: &str) -> El {
    row().w_full().px(8.0).pt(14.0).pb(4.0).child(text(label.to_owned()).font(k.ty.text_xsmall_medium).color(k.c.text_faint))
}

/// What a section row ends with: a spinner while it loads, else its count ("30+" when the
/// list stops at the page size), else nothing.
#[derive(Clone, Copy)]
enum Tail {
    None,
    Loading,
    Count(usize, bool),
}

fn tail<T>(l: Option<&Load<T>>, n: impl Fn(&T) -> usize, cap: Option<usize>) -> Tail {
    match l {
        Some(l) if l.loading => Tail::Loading,
        Some(Load { data: Some(d), .. }) => {
            let n = n(d);
            Tail::Count(n, cap.is_some_and(|c| n >= c))
        }
        _ => Tail::None,
    }
}

fn tail_el(k: &Kit, t: Tail, selected: bool) -> Option<El> {
    match t {
        Tail::None => None,
        Tail::Loading => Some(Spinner::new().size(12.0).build(k)),
        Tail::Count(n, more) => Some(
            row()
                .h(18.0)
                .min_w(18.0)
                .px(6.0)
                .center()
                .radius(9.0)
                .fill(if selected { k.c.layer_01 } else { k.c.hover })
                .child(text(if more { tf!("{n}+") } else { n.to_string() }).font(k.ty.text_xsmall_medium).color(k.c.text_muted)),
        ),
    }
}

fn section_row(k: &Kit, key: String, label: &str, icon: &'static Icon, selected: bool, t: Tail, go: impl Fn(&mut State) + 'static) -> El {
    let mut r = SidebarRow::new(crate::i18n::t(label)).icon(icon).height(ROW).selected(selected).on_select(move |s: &mut State, _| go(s));
    if let Some(e) = tail_el(k, t, selected) {
        r = r.trailing(e);
    }
    r.build(k).key(key)
}

/// Sidebar 2 for the current page, or None where there is nothing to pick (the Dashboard).
pub fn view(s: &State, k: &Kit, layer: &mut Vec<El>) -> Option<El> {
    let mut c = col().key("subnav").w_full().h_full().px(6.0).pt(10.0).pb(8.0).gap(2.0);
    let filters_used = match s.page {
        Page::Inbox => return None,
        Page::Group => {
            let name = s.group_cur.clone()?;
            let group = s.prefs.groups.iter().find(|g| g.name == name)?;
            let loads: Vec<Option<&Load<crate::daily::GroupRepo>>> = group.repos.iter().map(|r| s.group_data.get(r)).collect();
            let loading = loads.iter().any(|l| l.is_some_and(|l| l.loading));
            let datas: Vec<&crate::daily::GroupRepo> = loads.iter().filter_map(|l| l.and_then(|l| l.data.as_ref())).collect();
            let t = crate::model::now();
            let count = |n: usize| if loading { Tail::Loading } else if datas.is_empty() { Tail::None } else { Tail::Count(n, false) };
            let prs = datas.iter().map(|d| d.prs.iter().filter(|p| s.filter.keeps(&p.author.login, &p.updated_at, t)).count()).sum();
            let workflows = datas.iter().map(|d| d.health.len()).sum();
            let mut envs: Vec<&String> = datas.iter().flat_map(|d| d.envs.names.iter()).collect();
            envs.sort();
            envs.dedup();
            let tails = [if loading { Tail::Loading } else { Tail::Count(group.repos.len(), false) }, count(prs), count(workflows), count(envs.len())];
            for (i, label) in GROUP_TABS.iter().enumerate() {
                let icon = [&I_DASH, &I_PR, &I_RUN, &I_ENV][i];
                c = c.child(section_row(k, tf!("gt-{i}"), label, icon, s.group_tab == i, tails[i], move |s| s.show_group_tab(i)));
            }
            if !group.repos.is_empty() {
                c = c.child(heading(k, "REPOSITORIES"));
                for (slug, l) in group.repos.iter().zip(&loads) {
                    let name = slug.split_once('/').map_or(slug.as_str(), |(_, n)| n).to_owned();
                    let sl = slug.clone();
                    let mut r = SidebarRow::new(name).height(ROW).on_select(move |s: &mut State, _| s.select_repo(&sl, Page::Pulls));
                    r = match super::org::badge(s, k, slug.split_once('/').map_or("", |(o, _)| o), 16.0) {
                        Some(b) => r.avatar(b),
                        None => r.icon(&I_REPO),
                    };
                    if l.is_some_and(|l| l.loading) {
                        r = r.trailing(Spinner::new().size(12.0).build(k));
                    } else if let Some(n) = l.and_then(|l| l.data.as_ref()).map(|d| d.prs.len())
                        && let Some(e) = tail_el(k, Tail::Count(n, false), false)
                    {
                        r = r.trailing(e.help(crate::i18n::t(tf!("{n} open pull requests"))));
                    }
                    c = c.child(r.build(k).help(crate::i18n::t(slug.clone())).key(tf!("gr-{slug}")));
                }
            }
            // Overview and Pull requests list PRs; the rest don't use the filters.
            s.group_tab <= 1
        }
        page => {
            let repo = s.repo()?;
            let d = s.rd();
            for p in Page::REPO {
                let needs_clone = matches!(p, Page::Commits | Page::Branches) && repo.path.is_none();
                let t = match p {
                    _ if needs_clone => Tail::None,
                    Page::Pulls => tail(d.map(|d| &d.prs), Vec::len, Some(s.pr_limit)),
                    Page::Actions => tail(d.map(|d| &d.runs), Vec::len, Some(s.run_limit)),
                    Page::Envs => tail(d.map(|d| &d.envs), |e| e.names.len(), None),
                    Page::Commits => tail(d.map(|d| &d.graph), |g| g.commits.len(), Some(s.commit_limit)),
                    Page::Branches => tail(d.map(|d| &d.branches), |b| b.iter().filter(|b| !b.remote).count(), None),
                    _ => Tail::None,
                };
                let mut r = SidebarRow::new(crate::i18n::t(p.label())).icon(page_icon(p)).height(ROW).selected(page == p).on_select(move |s: &mut State, _| s.show(p));
                if needs_clone {
                    r = r.subtitle(crate::i18n::t("needs a local clone")).subtitle_color(k.c.text_faint).height(40.0);
                }
                if let Some(e) = tail_el(k, t, page == p) {
                    r = r.trailing(e);
                }
                c = c.child(r.build(k).key(tf!("sp-{p:?}")));
            }
            matches!(page, Page::Pulls | Page::Actions | Page::Commits)
        }
    };
    let c = c.child(spacer()).child(filters(s, k, filters_used, layer));
    Some(
        row()
            .key("subnav-box")
            .w(width(s))
            .h_full()
            .fill(k.c.bg_base)
            .child(col().w(0.0).grow(1.0).min_w(0.0).h_full().child(c))
            .child(edge(k)),
    )
}

/// The right edge: drag to resize, double-click for the default width.
fn edge(k: &Kit) -> El {
    row()
        .key("subnav-edge")
        .w(7.0)
        .h_full()
        .center()
        .cursor(Cursor::ResizeColumn)
        .hover_group()
        .help(crate::i18n::t("Drag to resize · double-click to reset"))
        .child(el().w(1.0).h_full().fill(k.c.border_weak).group_opacity(1.0, 0.0))
        .child(el().w(3.0).h(28.0).radius(1.5).fill(k.c.border_base).group_opacity(0.0, 1.0))
        .on_drag(|s: &mut State, d: &mut Drag, _| match d.phase {
            Phase::Down => {
                if d.clicks >= 2 && s.prefs.splits.remove(PREF).is_some() {
                    s.save_prefs();
                }
                d.data = width(s);
            }
            Phase::Move => {
                let (_, min, max) = SUBNAV_W;
                s.prefs.splits.insert(PREF.to_owned(), vec![(d.data + d.dx).clamp(min, max)]);
            }
            Phase::Up => {
                if d.cancelled() {
                    s.prefs.splits.insert(PREF.to_owned(), vec![d.data]);
                }
                s.save_prefs();
            }
        })
}

/// A contributor's profile picture, or their initial on a colour of their own.
fn avatar(s: &State, k: &Kit, id: &str, login: bool, size: f32) -> El {
    let url = crate::gh::avatar_url(id);
    if login && let Some(img) = s.avatars.get(&url).filter(|i| i.size().is_some() && i.error().is_none()) {
        return el().size(size, size).radius(size / 2.0).clip_rounded(size / 2.0).pixels(img.pixels());
    }
    let initial = id.chars().next().map_or("?".to_owned(), |c| c.to_uppercase().to_string());
    row()
        .size(size, size)
        .radius(size / 2.0)
        .center()
        .fill(hue(id.bytes().map(usize::from).sum()))
        .child(rune::text(initial).font(TextStyle { size: size * 0.5, ..k.ty.ui_medium }).color(rune::Color::WHITE))
}

/// The shared filters: date range, contributor, and whether this section uses them.
fn filters(s: &State, k: &Kit, used: bool, layer: &mut Vec<El>) -> El {
    let w = width(s) - 22.0;
    let label = |d: u32| if d == 0 { crate::i18n::t("Any time") } else { crate::i18n::t(tf!("Last {d} days")) };
    let labels: Vec<String> = FILTER_DAYS.iter().map(|d| label(*d)).collect();
    let choices = labels.clone();
    let range = Select::new("filter-days", &labels)
        .label(crate::i18n::t("Created in the last"))
        .width(w)
        .current(Some(&label(s.filter.days)))
        .open(s.filter_days_open)
        .on_toggle(|s: &mut State| {
            s.filter_who_open = false;
            s.filter_days_open = !s.filter_days_open;
        })
        .on_close(|s: &mut State| s.filter_days_open = false)
        .on_choose(move |s: &mut State, v: &str, _| {
            s.filter_days_open = false;
            if let Some(i) = choices.iter().position(|c| c == v) {
                s.set_filter_days(FILTER_DAYS[i]);
            }
        })
        .build(k, layer)
        .help(crate::i18n::t("Only what was created in the last 7, 30 or 90 days"));

    // The contributor: typed, or picked from the list that opens above the box.
    let input = TextInput::new(&s.filter_who)
        .placeholder(crate::i18n::t("Contributor"))
        .width(None)
        .on_edit(|s: &mut State| &mut s.filter_who)
        .on_change(|s: &mut State, _| {
            s.toggle_contributors(true);
            s.edit_filter_who();
        })
        .on_submit(|s: &mut State, _| {
            s.filter_who_open = false;
            s.apply_filter_who();
        });
    let input = super::with_clear(input, &s.filter_who.text, |s| &mut s.filter_who, State::apply_filter_who).build(k).key("filter-who");
    let lead = row().w(22.0).center().child(if s.filter.who.is_empty() {
        rune::icon(&I_PERSON, 14.0).color(k.c.icon_faint)
    } else {
        avatar(s, k, &s.filter.who, s.page != Page::Commits, 18.0)
    });
    let open = s.filter_who_open;
    let who = row()
        .key("filter-who-box")
        .w_full()
        .gap(2.0)
        .center_y()
        .child(lead)
        .child(col().w(0.0).grow(1.0).min_w(0.0).child(input))
        .child(
            IconButton::new(if open { &glyphs::CHEVRON_DOWN } else { &glyphs::CHEVRON_UP })
                .variant(IconButtonVariant::GhostMuted)
                .label(crate::i18n::t("Pick a contributor"))
                .help(crate::i18n::t("Pick a contributor"))
                .on_press(move |s: &mut State| {
                    s.filter_days_open = false;
                    s.toggle_contributors(!open);
                })
                .build(k)
                .key("filter-who-pick"),
        )
        .help(crate::i18n::t("A GitHub login for pull requests and runs; a git author name or email for commits"));
    if open {
        let q = s.filter_who.text.trim().to_lowercase();
        // The box still shows the applied filter: list everyone, not just that one.
        let everyone = q == s.filter.who.to_lowercase();
        let items: Vec<PopupItem> = s
            .contributors()
            .iter()
            .filter(|c| everyone || c.id.to_lowercase().contains(&q) || c.name.to_lowercase().contains(&q))
            .take(80)
            .map(|c| {
                let detail = if c.login && c.id == s.me { crate::i18n::t("you") } else { c.name.clone() };
                let mut i = PopupItem::new(c.id.clone(), c.id.clone()).description(detail).leading(avatar(s, k, &c.id, c.login, 20.0));
                if c.id.eq_ignore_ascii_case(&s.filter.who) {
                    i = i.trailing(rune::icon(&glyphs::CHECK, 12.0).color(k.c.icon_muted));
                }
                i
            })
            .collect();
        let loading = s.cur.as_ref().is_some_and(|c| s.pr_people.get(c).is_some_and(|l| l.loading));
        PopupList::new("filter-who-list", "filter-who-box")
            .width(w.max(240.0))
            .max_height(320.0)
            .items(items)
            .label(crate::i18n::t("Contributors"))
            .empty_message(crate::i18n::t(if loading { "Loading contributors…" } else { "Nobody matches" }))
            .on_choose(|s: &mut State, id: &str, _| s.pick_contributor(id))
            .on_close(|s: &mut State| s.filter_who_open = false)
            .build(k, layer);
    }

    let mut quick = row().w_full().gap(4.0).center_y();
    if !s.me.is_empty() && s.filter.who != s.me {
        let me = s.me.clone();
        quick = quick.child(Button::new(crate::i18n::t("Me")).variant(ButtonVariant::Ghost).on_press(move |s: &mut State| s.pick_contributor(&me)).build(k).key("filter-me"));
    }
    quick = quick.child(spacer());
    if !s.filter.is_empty() {
        quick = quick.child(Button::new(crate::i18n::t("Clear")).variant(ButtonVariant::Ghost).on_press(|s: &mut State| s.clear_filter()).build(k).key("filter-clear"));
    }
    let mut c = col()
        .key("filters")
        .w_full()
        .gap(6.0)
        .px(2.0)
        .pt(10.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_weak)
        .child(row().w_full().pt(2.0).child(text("FILTERS").font(k.ty.text_xsmall_medium).color(k.c.text_faint)))
        .child(range)
        .child(who)
        .child(quick);
    if !used {
        c = c.child(muted(k, "Not used in this section").font(k.ty.text_xsmall));
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_is_the_saved_one_within_bounds() {
        let mut s = State::new(crate::config::Prefs::default());
        let (def, min, max) = SUBNAV_W;
        assert_eq!(width(&s), def);
        s.prefs.splits.insert(PREF.into(), vec![250.0]);
        assert_eq!(width(&s), 250.0);
        s.prefs.splits.insert(PREF.into(), vec![10.0]);
        assert_eq!(width(&s), min);
        s.prefs.splits.insert(PREF.into(), vec![9999.0]);
        assert_eq!(width(&s), max);
        s.prefs.splits.insert(PREF.into(), vec![f32::NAN]);
        assert_eq!(width(&s), def);
    }

    #[test]
    fn section_counts_show_loading_counts_and_more() {
        let none: Option<&Load<Vec<u8>>> = None;
        assert!(matches!(tail(none, Vec::len, None), Tail::None));
        let mut l: Load<Vec<u8>> = Load::start();
        assert!(matches!(tail(Some(&l), Vec::len, None), Tail::Loading));
        l.finish(Ok(vec![1, 2, 3]));
        assert!(matches!(tail(Some(&l), Vec::len, Some(3)), Tail::Count(3, true)), "a full page: there may be more");
        assert!(matches!(tail(Some(&l), Vec::len, Some(30)), Tail::Count(3, false)));
    }
}
