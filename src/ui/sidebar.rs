//! The sidebar: Dashboard on top, then your repositories in a virtualised list (hundreds of
//! repos cost the same as ten), in two explicit parts:
//!
//! 1. **Pinned**: repos you pinned (right-click → Pin to top).
//! 2. **Groups**: groups you made, mixing repos from any owners (right-click → a group).
//! 3. **Repositories**: one section per owner (you, then each organization), with every repo
//!    you can reach.
//!
//! Each group and owner section starts with the repo you opened most recently there, marked
//! "recent". Owner sections then show their next repos (recently opened, then recently pushed)
//! up to a limit, with "Show N more"; searching always shows every match.
//!
//! A repo can sit in several sections. Click a section header to fold it (or fold them all from
//! the header); folds are remembered. Drag a group's or an owner's header to move the whole
//! section (or use its right-click menu), and drag pinned repos, and repos inside your groups,
//! into the order you want. Every row shares one layout (`INSET`, `FOLD`, `GAP`, `ICON`), and a
//! hairline splits Groups from Repositories. The filter box narrows
//! everything on each keystroke, and after a short pause searches all of GitHub too ("On GitHub").

use std::borrow::Cow;

use rune::{Drag, El, Icon, Phase, Spring, SpringPair, Transition, Tx, col, el, row, spacer};
use crate::i18n::text;
use rune_kit::shell::{Reorder, Sidebar, SidebarRow};
use rune_kit::{IconButton, IconButtonVariant, Kit, MenuItem, Spinner, TextInput, context_menu, glyphs};

use super::{I_CLOUD, I_INBOX, I_PLUS, I_REPO, chip, muted};
use crate::model::Outcome;
use crate::{MAX_DRAG_GROUPS, MenuTarget, Page, SectionDrag, State};

const ROW: f32 = 40.0;
/// One layout for every row, so icons and labels line up down the list: rows sit `INSET` in
/// from the sides; a section header's fold triangle takes `FOLD` + `GAP`, so its 16 pt icon
/// sits where the repos under it (one `SidebarRow` level in) put theirs, and the labels align.
const INSET: f32 = 8.0;
const FOLD: f32 = 10.0;
const GAP: f32 = 6.0;
const ICON: f32 = 16.0;
/// Where a section's repos, and its "Show more" / missing rows, start.
const NESTED: f32 = INSET + FOLD + GAP;

/// Folding and unfolding a section: quick, no bounce. Its rows drop in from just above and fade
/// in (and back out), the rows below slide on the same spring, and the fold triangle pops to
/// its new direction.
const FOLD_IN: Spring = Spring::new(0.18, 0.0);
const FOLD_OUT: Spring = Spring::new(0.14, 0.0);

fn fold_motion() -> Transition {
    Transition::offset(0.0, -8.0).combined(Transition::opacity()).springs(SpringPair::new(FOLD_IN, FOLD_OUT))
}

/// Fold or unfold section `key`, animated (also from the keyboard).
fn toggle_fold(s: &mut State, tx: &mut Tx, key: &str) {
    tx.animated();
    tx.motion(FOLD_IN);
    s.toggle_fold(key);
}

/// A section of the list. `fold` is its key in `State::collapsed`.
#[derive(Clone)]
enum Section {
    Pinned,
    Group(String),
    Owner(String),
}

impl Section {
    fn fold(&self) -> String {
        match self {
            Section::Pinned => "pinned".into(),
            Section::Group(g) => tf!("g:{g}"),
            Section::Owner(o) => tf!("o:{o}"),
        }
    }
}

/// One line of the flattened list.
enum Item {
    /// The "Groups" / "Repositories" heading that splits the list in two.
    Part(Part),
    /// No groups yet: how to make one.
    NoGroups,
    Header { section: Section, count: usize, folded: bool },
    /// `slot`: (position, count) in a list you can drag to reorder (pinned, or a group of
    /// yours), when dragging is possible (not while filtering). `recent`: the section's most
    /// recently opened repo, shown first.
    Repo { idx: usize, section: Section, slot: Option<(usize, usize)>, recent: bool },
    /// A group member that isn't in the repo list (no longer reachable, or not loaded yet).
    Missing { slug: String, group: String },
    /// "Show N more" at the bottom of a cut-short owner section.
    More { owner: String, hidden: usize },
    /// "Show less" at the bottom of an expanded one.
    Less { owner: String },
    Searching,
    HitsHeader(usize),
    Hit(usize),
    Nothing,
}

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Groups,
    Repos,
}

/// Of `slugs`, the one you opened most recently (if you opened any of them).
fn most_recent<'a>(s: &State, slugs: impl Iterator<Item = &'a String>) -> Option<&'a String> {
    slugs.filter_map(|r| Some((s.prefs.recent.iter().position(|x| x == r)?, r))).min_by_key(|(p, _)| *p).map(|(_, r)| r)
}

fn items(s: &State) -> Vec<Item> {
    let q = s.repo_filter.text.trim().to_lowercase();
    let matches = |slug: &str| q.is_empty() || slug.to_lowercase().contains(&q);
    let index = |slug: &str| s.repos.iter().position(|r| r.slug == slug);
    let mut out = Vec::new();
    // Typing unfolds every section, so a match is never hidden.
    let folded = |sec: &Section| q.is_empty() && s.collapsed.contains(&sec.fold());

    let push = |out: &mut Vec<Item>, section: Section, members: Vec<Item>| {
        if members.is_empty() {
            return;
        }
        let f = folded(&section);
        out.push(Item::Header { section, count: members.len(), folded: f });
        if !f {
            out.extend(members);
        }
    };

    // 1. Pinned, in the order you pinned them.
    let n = s.prefs.pinned.len();
    let drag = q.is_empty();
    let pinned: Vec<Item> = s
        .prefs
        .pinned
        .iter()
        .enumerate()
        .filter(|(_, p)| matches(p))
        .filter_map(|(pos, p)| Some(Item::Repo { idx: index(p)?, section: Section::Pinned, slot: drag.then_some((pos, n)), recent: false }))
        .collect();
    push(&mut out, Section::Pinned, pinned);

    // 2. Your groups. The one you opened last leads (not draggable); the rest keep your order.
    let part = out.len();
    out.push(Item::Part(Part::Groups));
    for (gi, g) in s.prefs.groups.iter().enumerate() {
        let sec = Section::Group(g.name.clone());
        let shown = |r: &String| matches(r) || g.name.to_lowercase().contains(&q);
        let top = most_recent(s, g.repos.iter().filter(|r| index(r).is_some())).filter(|_| g.repos.len() > 1);
        let rest: Vec<&String> = g.repos.iter().filter(|r| Some(*r) != top).collect();
        let n = rest.len();
        let drag = q.is_empty() && gi < MAX_DRAG_GROUPS;
        let mut members: Vec<Item> = Vec::new();
        if let Some(t) = top.filter(|t| shown(t)) {
            members.push(Item::Repo { idx: index(t).unwrap_or_default(), section: sec.clone(), slot: None, recent: true });
        }
        members.extend(rest.iter().enumerate().filter(|(_, r)| shown(r)).map(|(pos, r)| match index(r) {
            Some(idx) => Item::Repo { idx, section: sec.clone(), slot: drag.then_some((pos, n)), recent: false },
            None => Item::Missing { slug: (*r).clone(), group: g.name.clone() },
        }));
        if members.is_empty() && q.is_empty() {
            // An empty group still shows, so it can be filled or deleted.
            out.push(Item::Header { section: sec, count: 0, folded: false });
        } else {
            push(&mut out, sec, members);
        }
    }
    if s.prefs.groups.is_empty() && q.is_empty() {
        out.push(Item::NoGroups);
    } else if out.len() == part + 1 {
        out.pop(); // filtering left no group: no heading either
    }

    // 3. Repositories, by owner in your order (right-click a header to move it), else you first then A–Z.
    // Each shows its first few repos (Settings): the ones you opened most recently on top, then
    // the most recently pushed. Searching shows every match.
    let part = out.len();
    out.push(Item::Part(Part::Repos));
    for owner in s.owners_sorted() {
        let owner = owner.as_str();
        let sec = Section::Owner(owner.to_owned());
        let mut idxs: Vec<usize> = s.repos.iter().enumerate().filter(|(_, r)| r.owner() == owner && matches(&r.slug)).map(|(i, _)| i).collect();
        if idxs.is_empty() {
            continue;
        }
        let recent = |i: &usize| s.prefs.recent.iter().position(|r| *r == s.repos[*i].slug).unwrap_or(usize::MAX);
        idxs.sort_by_key(|i| (recent(i), *i)); // stable: ties keep local-first, newest-push order
        let total = idxs.len();
        let has_recent = recent(&idxs[0]) != usize::MAX;
        let f = folded(&sec);
        out.push(Item::Header { section: sec.clone(), count: total, folded: f });
        if f {
            continue;
        }
        let expanded = s.expanded_orgs.contains(owner);
        // Settings → Appearance: how many per organization (0: all).
        let limit = if s.prefs.org_limit == 0 { usize::MAX } else { s.prefs.org_limit };
        let cut = q.is_empty() && !expanded && total > limit;
        for (n, idx) in idxs.into_iter().take(if cut { limit } else { total }).enumerate() {
            out.push(Item::Repo { idx, section: sec.clone(), slot: None, recent: n == 0 && has_recent });
        }
        if cut {
            out.push(Item::More { owner: owner.to_owned(), hidden: total - limit });
        } else if q.is_empty() && expanded && total > limit {
            out.push(Item::Less { owner: owner.to_owned() });
        }
    }
    if out.len() == part + 1 && !q.is_empty() {
        out.pop(); // filtering left no repo: no heading either (unfiltered, it keeps its + button)
    }

    if !q.is_empty() {
        let hits: Vec<usize> = if s.repo_hits.0.to_lowercase() == q {
            s.repo_hits.1.iter().enumerate().filter(|(_, h)| index(&h.full_name).is_none()).map(|(i, _)| i).collect()
        } else {
            Vec::new()
        };
        if s.repo_searching {
            out.push(Item::Searching);
        }
        if !hits.is_empty() {
            out.push(Item::HitsHeader(hits.len()));
            out.extend(hits.into_iter().map(Item::Hit));
        }
        if out.is_empty() {
            out.push(Item::Nothing);
        }
    }
    out
}

fn key(s: &State, i: usize) -> Cow<'static, str> {
    Cow::Owned(match items(s).get(i) {
        Some(Item::Part(Part::Groups)) => "part:groups".into(),
        Some(Item::Part(Part::Repos)) => "part:repos".into(),
        Some(Item::NoGroups) => "no-groups".into(),
        Some(Item::Header { section, .. }) => tf!("h:{}", section.fold()),
        Some(Item::Repo { idx, section, .. }) => tf!("r:{}:{}", section.fold(), s.repos[*idx].slug),
        Some(Item::Missing { slug, group }) => tf!("m:{group}:{slug}"),
        Some(Item::More { owner, .. }) => tf!("more:{owner}"),
        Some(Item::Less { owner }) => tf!("less:{owner}"),
        Some(Item::Searching) => "searching".into(),
        Some(Item::HitsHeader(_)) => "hits".into(),
        Some(Item::Hit(h)) => tf!("hit:{}", s.repo_hits.1[*h].full_name),
        Some(Item::Nothing) | None => tf!("empty:{i}"),
    })
}

static I_PIN: Icon = Icon {
    name: "g-pin",
    view: 16.0,
    body: r#"<path d="M6 2.5H10L9.5 6.5L12 9H4L6.5 6.5Z M8 9V14" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round" fill="none"/>"#,
};
/// An owner with no badge of its own.
static I_OWNER: Icon = Icon {
    name: "g-owner",
    view: 16.0,
    body: r#"<rect x="2.5" y="2.5" width="11" height="11" rx="3" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="8" cy="6.8" r="1.8" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.8 12C5.4 10.4 6.6 9.7 8 9.7S10.6 10.4 11.2 12" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#,
};
static I_GROUP: Icon = Icon {
    name: "g-group",
    view: 16.0,
    body: r#"<rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="9" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="2.5" y="9" width="11" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/>"#,
};

/// A group's or owner's place in its order (drag handles), and the group/owner it names.
fn order_of(s: &State, group: bool, name: &str) -> Option<usize> {
    if group { s.prefs.groups.iter().position(|g| g.name == name) } else { s.owners_sorted().iter().position(|o| o == name) }
}

/// Dragging a group or owner header: it lands where the header under the pointer is (every
/// row is `ROW` tall, so the pointer's travel counts rows). Releasing it puts it there.
fn drag_section(s: &mut State, d: &mut Drag, group: bool, name: &str) {
    match d.phase {
        Phase::Down => {
            s.section_dragged = false;
            let to = order_of(s, group, name).unwrap_or(0);
            s.section_drag = Some(SectionDrag { group, name: name.to_owned(), to, moved: false });
        }
        Phase::Move => {
            let Some(drag) = s.section_drag.as_ref() else { return };
            if !drag.moved && d.dy.abs() < 4.0 {
                return;
            }
            let to = drop_target(s, group, name, d.dy);
            if let (Some(drag), Some(to)) = (s.section_drag.as_mut(), to) {
                drag.to = to;
                drag.moved = true;
            }
        }
        Phase::Up => {
            let Some(drag) = s.section_drag.take() else { return };
            // A release on the header is followed by its click: that one mustn't fold it. A
            // release elsewhere has no click to skip.
            let on_header = d.x >= 0.0 && d.y >= 0.0 && d.x <= d.w && d.y <= d.h;
            s.section_dragged = drag.moved && on_header;
            if drag.moved && !d.cancelled() {
                if group {
                    s.move_group_to(&drag.name, drag.to);
                } else {
                    s.move_owner_to(&drag.name, drag.to);
                }
            }
        }
    }
}

/// Where the group (or owner) `name` lands when its header is dragged `dy` points: the place
/// of the last header of its kind at or above the row under the pointer (else the first).
fn drop_target(s: &State, group: bool, name: &str, dy: f32) -> Option<usize> {
    let list = items(s);
    let named = |sec: &Section| match sec {
        Section::Group(g) if group => Some(g.clone()),
        Section::Owner(o) if !group => Some(o.clone()),
        _ => None,
    };
    let headers: Vec<(usize, String)> = list.iter().enumerate().filter_map(|(i, it)| match it {
        Item::Header { section, .. } => Some((i, named(section)?)),
        _ => None,
    }).collect();
    let from = headers.iter().find(|(_, n)| n == name)?.0;
    let at = (from as f32 + (dy / ROW).round()).max(0.0) as usize;
    let (_, landing) = headers.iter().rev().find(|(i, _)| *i <= at).or(headers.first())?;
    order_of(s, group, landing)
}

fn header(s: &State, k: &Kit, section: &Section, count: usize, folded: bool) -> El {
    let fold = section.fold();
    let (label, icon): (String, Option<&'static Icon>) = match section {
        Section::Pinned => ("Pinned".into(), Some(&I_PIN)),
        Section::Group(g) => (g.clone(), Some(&I_GROUP)),
        Section::Owner(_) => (String::new(), None),
    };
    let label = match section {
        Section::Owner(o) => o.clone(),
        _ => label,
    };
    // Dragging this header (it dims), or where a dragged one would land (outlined).
    let drag = s.section_drag.as_ref().filter(|d| d.moved);
    let (group, name) = match section {
        Section::Group(g) => (true, g.as_str()),
        Section::Owner(o) => (false, o.as_str()),
        Section::Pinned => (false, ""),
    };
    let lifted = drag.is_some_and(|d| d.group == group && d.name == name && !name.is_empty());
    let target = !lifted && drag.is_some_and(|d| d.group == group && !name.is_empty() && order_of(s, group, name) == Some(d.to));
    let mut inner = row()
        .w_full()
        .h(28.0)
        .px(INSET)
        .gap(GAP)
        .center_y()
        .radius(k.radius.md)
        .hover(k.c.hover)
        .child(
            row().w(FOLD).center().child(
                // One triangle per state, so a fold swaps them with a quick pop.
                rune::icon(if folded { &glyphs::TRIANGLE_RIGHT } else { &glyphs::TRIANGLE_DOWN }, FOLD)
                    .color(k.c.icon_faint)
                    .key(if folded { "fold-closed" } else { "fold-open" })
                    .transition(Transition::scale(0.4).springs(SpringPair::new(FOLD_IN, FOLD_OUT))),
            ),
        );
    let badge = match section {
        Section::Owner(o) => super::org::badge(s, k, o, ICON),
        _ => None,
    };
    inner = match (badge, icon) {
        (Some(b), _) => inner.child(row().size(ICON, ICON).center().child(b)),
        (None, Some(i)) => inner.child(rune::icon(i, ICON).color(k.c.icon_muted)),
        (None, None) => inner.child(rune::icon(&I_OWNER, ICON).color(k.c.icon_muted)),
    };
    inner = inner.child(text(label.clone()).font(k.ty.text_small_medium).color(k.c.text_base).truncate().shrink()).child(spacer());
    // Loading what the section shows: a group's dashboard data.
    let loading = match section {
        Section::Group(g) => s.prefs.groups.iter().find(|x| x.name == *g).is_some_and(|g| g.repos.iter().any(|r| s.group_data.get(r).is_some_and(|l| l.loading))),
        _ => false,
    };
    if loading {
        inner = inner.child(Spinner::new().size(12.0).build(k).key("loading"));
    }
    // Your groups: a dashboard button, and their own menu (rename, delete).
    if let Section::Group(g) = section {
        let g2 = g.clone();
        inner = inner.child(
            IconButton::new(&super::I_DASH)
                .variant(IconButtonVariant::GhostMuted)
                .label(crate::i18n::t("Open dashboard"))
                .help(crate::i18n::t("Group dashboard: CI, deployments and open PRs across these repos"))
                .on_press(move |s: &mut State| s.open_group(&g2))
                .build(k)
                .key("dash")
                .group_opacity(0.0, 1.0),
        );
    }
    inner = inner.child(count_pill(k, count));
    if lifted {
        inner = inner.fill(k.c.hover).opacity(0.6);
    }
    if target {
        inner = inner.border(1.0, k.c.border_focus);
    }
    let draggable = !name.is_empty() && s.repo_filter.text.trim().is_empty();
    let mut r = row()
        .w_full()
        .h(ROW)
        .pt(12.0)
        .center_y()
        .hover_group()
        .role("button")
        .label(crate::i18n::t(tf!("{label}, {count} repositories")))
        .help(crate::i18n::t(if draggable { "Click to fold or unfold. Drag to move it. Right-click for more." } else { "Click to fold or unfold. Right-click to reorder or customize." }))
        .on_press_tx(move |s: &mut State, tx| {
            // The release that ends a drag isn't a click.
            if !std::mem::take(&mut s.section_dragged) {
                toggle_fold(s, tx, &fold);
            }
        })
        .child(inner);
    if draggable {
        let n = name.to_owned();
        r = r.on_drag(move |s: &mut State, d: &mut Drag, _| drag_section(s, d, group, &n));
    }
    if let Section::Group(g) = section {
        let g = g.clone();
        r = r.on_secondary_press(move |s: &mut State, p, _| s.menu = Some((MenuTarget::Group(g.clone()), p)));
    }
    if let Section::Owner(o) = section {
        let o = o.clone();
        r = r.on_secondary_press(move |s: &mut State, p, _| s.menu = Some((MenuTarget::Owner(o.clone()), p)));
    }
    r
}

/// A count in a soft pill, the same at every level.
fn count_pill(k: &Kit, n: usize) -> El {
    row().h(18.0).min_w(18.0).px(6.0).center().radius(9.0).fill(k.c.hover).child(text(n.to_string()).font(k.ty.text_xsmall).color(k.c.text_faint))
}

/// `with_owner`: in Pinned and your groups repos come from several owners, so the second line
/// names the owner.
macro_rules! group_drag_fns {
    ($($i:literal)*) => {
        const GROUP_DRAG: [fn(&mut State) -> &mut Reorder; MAX_DRAG_GROUPS] = [$(|s: &mut State| &mut s.group_drags[$i]),*];
    };
}
group_drag_fns!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23);

fn more_row(k: &Kit, label: String, tip: &str, f: impl Fn(&mut State) + 'static) -> El {
    row()
        .w_full()
        .h(ROW)
        .pl(NESTED)
        .pr(INSET)
        .gap(GAP)
        .center_y()
        .radius(k.radius.md)
        .hover(k.c.hover)
        .role("button")
        .label(crate::i18n::t(label.clone()))
        .help(crate::i18n::t(tip.to_owned()))
        .on_press(f)
        .child(row().w(ICON).center().child(text("⋯").font(k.ty.ui).color(k.c.text_faint)))
        .child(text(label).font(k.ty.small).color(k.c.text_accent))
}

/// Every section's fold key, for "collapse all".
fn fold_keys(s: &State) -> Vec<String> {
    let mut keys = vec!["pinned".to_owned()];
    keys.extend(s.prefs.groups.iter().map(|g| tf!("g:{}", g.name)));
    for r in &s.repos {
        let k = tf!("o:{}", r.owner());
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys
}

fn repo_row(s: &State, k: &Kit, idx: usize, section: &Section, slot: Option<(usize, usize)>, recent: bool) -> El {
    let with_owner = !matches!(section, Section::Owner(_));
    let repo = &s.repos[idx];
    let (_, name) = repo.slug.split_once('/').unwrap_or(("", &repo.slug));
    let slug = repo.slug.clone();
    let menu_slug = repo.slug.clone();
    let selected = s.page != Page::Inbox && s.cur.as_deref() == Some(&repo.slug);
    let mut r = SidebarRow::new(name.to_owned()).height(ROW).depth(1);
    r = match super::org::badge(s, k, repo.owner(), ICON) {
        Some(b) => r.avatar(b),
        None => r.icon(if repo.path.is_some() { &I_REPO } else { &I_CLOUD }),
    };
    let mut r = r
        .selected(selected)
        .on_select(move |s: &mut State, _| {
            let page = if matches!(s.page, Page::Inbox | Page::Group) { Page::Pulls } else { s.page };
            s.select_repo(&slug, page);
        });
    // A second line only where it says something: whose repo (in mixed sections), local, archived.
    let mut second: Vec<&str> = Vec::new();
    if with_owner {
        second.push(repo.owner());
    }
    if repo.path.is_some() {
        second.push("local clone");
    } else if repo.archived {
        second.push("archived");
    }
    if !second.is_empty() {
        r = r.subtitle(second.join(" · "));
    }
    if repo.archived {
        r = r.subtitle_color(k.c.text_faint);
    }
    let pinned = s.prefs.pinned.contains(&repo.slug) && !matches!(section, Section::Pinned);
    // Loading this repo's pages (the open one).
    let loading = s.data.get(&repo.slug).is_some_and(|d| d.prs.loading || d.runs.loading || d.envs.loading || d.graph.loading || d.branches.loading || d.status.loading);
    if recent || pinned || loading {
        let mut t = row().gap(GAP).center_y();
        if loading {
            t = t.child(Spinner::new().size(12.0).build(k).key("loading"));
        }
        if recent {
            t = t.child(
                row()
                    .h(18.0)
                    .px(6.0)
                    .center()
                    .radius(9.0)
                    .fill(k.c.hover)
                    .child(text("recent").font(k.ty.text_xsmall_medium).color(k.c.text_accent))
                    .help(crate::i18n::t("The repo you opened most recently here")),
            );
        }
        if pinned {
            t = t.child(rune::icon(&I_PIN, 11.0).color(k.c.icon_faint));
        }
        r = r.trailing(t);
    }
    // Drag to reorder, within pinned or within one of your groups.
    let mut tip = "Right-click: pin, add to a group, open on GitHub";
    if let Some((pos, count)) = slot {
        match section {
            Section::Pinned => {
                r = r.reorder(&s.pin_drag, pos, count, |s: &mut State| &mut s.pin_drag, |s: &mut State, from, to| s.move_pinned(from, to));
                tip = "Drag to reorder your pinned repos · Right-click for more";
            }
            Section::Group(name) => {
                if let Some(gi) = s.prefs.groups.iter().position(|g| g.name == *name).filter(|gi| *gi < MAX_DRAG_GROUPS) {
                    // The recent repo leads the group outside the drag: positions count the rest.
                    let top = most_recent(s, s.prefs.groups[gi].repos.iter().filter(|r| s.repos.iter().any(|x| x.slug == **r))).filter(|_| s.prefs.groups[gi].repos.len() > 1).cloned();
                    r = r.reorder(&s.group_drags[gi], pos, count, GROUP_DRAG[gi], move |s: &mut State, from, to| s.move_in_group(gi, top.as_deref(), from, to));
                    tip = "Drag to reorder this group · Right-click for more";
                }
            }
            Section::Owner(_) => {}
        }
    }
    r.build(k).help(crate::i18n::t(tip)).on_secondary_press(move |s: &mut State, p, _| s.menu = Some((MenuTarget::Repo(menu_slug.clone()), p)))
}

/// The heading of one of the list's two parts, with its buttons.
fn part_row(s: &State, k: &Kit, part: Part) -> El {
    let mut r = row().w_full().h(28.0).gap(2.0).center_y();
    match part {
        Part::Groups => {
            r = r.child(text("GROUPS").font(k.ty.text_xsmall_medium).color(k.c.text_faint)).child(spacer()).child(
                IconButton::new(&I_GROUP)
                    .variant(IconButtonVariant::GhostMuted)
                    .label(crate::i18n::t("New group"))
                    .help(crate::i18n::t("New group…"))
                    .on_press(|s: &mut State| s.open_group_dialog(None, None))
                    .build(k)
                    .key("new-group"),
            );
        }
        Part::Repos => {
            let keys = fold_keys(s);
            let all_folded = keys.iter().all(|k| s.collapsed.contains(k));
            r = r
                .child(text(tf!("REPOSITORIES · {}", s.repos.len())).font(k.ty.text_xsmall_medium).color(k.c.text_faint))
                .child(spacer())
                .children(s.repos_loading.then(|| Spinner::new().size(12.0).build(k).key("loading").help(crate::i18n::t("Loading your repositories from GitHub…"))))
                .child(
                    IconButton::new(if all_folded { &glyphs::CHEVRON_DOWN } else { &glyphs::CHEVRON_UP })
                        .variant(IconButtonVariant::GhostMuted)
                        .label(crate::i18n::t(if all_folded { "Expand all" } else { "Collapse all" }))
                        .help(crate::i18n::t(if all_folded { "Expand all sections" } else { "Collapse all sections (remembered)" }))
                        .on_press(move |s: &mut State| s.toggle_fold_all(keys.clone()))
                        .build(k)
                        .key("fold-all"),
                )
                .child(
                    IconButton::new(&I_PLUS)
                        .variant(IconButtonVariant::GhostMuted)
                        .label(crate::i18n::t("Add local repo"))
                        .help(crate::i18n::t("Add a local clone…"))
                        .on_press_tx(crate::ask_local_repo)
                        .build(k)
                        .key("add-repo"),
                );
        }
    }
    // A hairline over each part, so groups and repositories read as two blocks.
    col().w_full().h(ROW).px(INSET).pt(6.0).gap(5.0).child(el().w_full().h(1.0).fill(k.c.border_weak)).child(r)
}

fn build(s: &State, i: usize, k: &Kit) -> El {
    let list = items(s);
    let Some(item) = list.get(i) else { return row() };
    // What folding shows and hides moves in and out (headers and part headings stay put).
    let folds = matches!(item, Item::Repo { .. } | Item::Missing { .. } | Item::More { .. } | Item::Less { .. });
    let e = match item {
        Item::Header { section, count, folded } => header(s, k, section, *count, *folded),
        Item::Part(part) => part_row(s, k, *part),
        Item::NoGroups => more_row(k, "New group…".to_owned(), "Make a group of repos from any owners, with its own dashboard", |s: &mut State| s.open_group_dialog(None, None)),
        Item::Repo { idx, section, slot, recent } => repo_row(s, k, *idx, section, *slot, *recent),
        Item::Missing { slug, group } => {
            let (slug2, group2) = (slug.clone(), group.clone());
            row()
                .w_full()
                .h(ROW)
                .pl(NESTED)
                .pr(INSET)
                .gap(GAP)
                .center_y()
                .child(rune::icon(&I_CLOUD, ICON).color(k.c.icon_faint))
                .child(text(slug.clone()).font(k.ty.small).color(k.c.text_faint).truncate().shrink())
                .child(spacer())
                .child(
                    IconButton::new(&glyphs::CLOSE)
                        .variant(IconButtonVariant::GhostMuted)
                        .label(crate::i18n::t("Remove from group"))
                        .help(crate::i18n::t("Not reachable: remove it from this group"))
                        .on_press(move |s: &mut State| s.toggle_in_group(&group2, &slug2))
                        .build(k),
                )
        }
        Item::Searching => row().h(ROW).px(INSET).gap(GAP).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, "Searching GitHub…")),
        Item::HitsHeader(n) => row()
            .w_full()
            .h(ROW)
            .px(INSET)
            .pt(12.0)
            .center_y()
            .child(text("ON GITHUB").font(k.ty.text_xsmall_medium).color(k.c.text_muted))
            .child(spacer())
            .child(text(n.to_string()).font(k.ty.text_xsmall).color(k.c.text_faint)),
        Item::Hit(h) => {
            let hit = &s.repo_hits.1[*h];
            let slug = hit.full_name.clone();
            let (owner, name) = slug.split_once('/').unwrap_or(("", &slug));
            SidebarRow::new(name.to_owned())
                .icon(&I_CLOUD)
                .subtitle(owner.to_owned())
                .height(ROW)
                .on_select(move |s: &mut State, _| s.select_repo(&slug.clone(), Page::Pulls))
                .build(k)
        }
        Item::Nothing => row().h(ROW).px(INSET).center_y().child(muted(k, "No repositories match")),
        Item::More { owner, hidden } => {
            let o = owner.clone();
            more_row(k, tf!("Show {hidden} more"), "Recently opened repos stay on top. Or type in the search box to find any repo.", move |s: &mut State| {
                s.expanded_orgs.insert(o.clone());
            })
        }
        Item::Less { owner } => {
            let o = owner.clone();
            more_row(k, "Show less".to_owned(), "Back to the first few", move |s: &mut State| {
                s.expanded_orgs.remove(&o);
            })
        }
    };
    if folds { e.transition(fold_motion()) } else { e }
}

pub fn view(s: &State, k: &Kit) -> El {
    let reviews = s.inbox.data.as_ref().map_or(0, |i| i.review_requested.len());
    let mut inbox = SidebarRow::new(crate::i18n::t("Dashboard")).icon(&I_INBOX).height(32.0).selected(s.page == Page::Inbox).on_select(|s: &mut State, _| s.show(Page::Inbox));
    if reviews > 0 {
        inbox = inbox.trailing(chip(k, reviews.to_string(), Some(Outcome::Running)));
    }
    let filter = TextInput::new(&s.repo_filter)
        .placeholder(crate::i18n::t("Search repositories"))
        .leading_icon(k.glyph.search)
        .width(None)
        .on_edit(|s: &mut State| &mut s.repo_filter)
        .on_change(|s: &mut State, _| s.search_repos());
    let filter = super::with_clear(filter, &s.repo_filter.text, |s| &mut s.repo_filter, State::search_repos)
        .build(k)
        .key("repo-filter");
    let count = items(s).len();
    let kc = k.clone();
    Sidebar::new()
        .label(crate::i18n::t("Navigation"))
        .header(inbox.build(k).key("nav-inbox"))
        .header(rune::col().w_full().px(8.0).pt(6.0).pb(2.0).child(filter))
        .row_height(ROW)
        .virtual_rows(count, |s: &State, i| key(s, i), move |s: &State, i| build(s, i, &kc))
        .build(k)
}

/// The right-click menu, drawn on the overlay layer at the pointer. Returns its (empty) anchor,
/// which goes anywhere in the tree.
pub fn context(s: &State, k: &Kit, layer: &mut Vec<El>) -> Option<El> {
    let (target, at) = s.menu.as_ref()?;
    let mut menu = context_menu("ctx", el().key("ctx-anchor").size(0.0, 0.0), Some(*at)).min_width(220.0).on_close(|s: &mut State| s.menu = None);
    match target {
        MenuTarget::Repo(slug) => {
            let pinned = s.prefs.pinned.contains(slug);
            let sl = slug.clone();
            menu = menu
                .group_label(crate::i18n::t(slug.clone()))
                .item(MenuItem::new(crate::i18n::t(if pinned { "Unpin" } else { "Pin to top" })).icon(&I_PIN).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.toggle_pin(&sl);
                }))
                .separator()
                .group_label(crate::i18n::t("Groups"));
            for g in &s.prefs.groups {
                let (gn, sl) = (g.name.clone(), slug.clone());
                menu = menu.item(MenuItem::new(crate::i18n::t(g.name.clone())).checked(g.repos.contains(slug)).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.toggle_in_group(&gn, &sl);
                }));
            }
            let sl = slug.clone();
            menu = menu.item(MenuItem::new(crate::i18n::t("New group with this repo…")).icon(&I_GROUP).on_select(move |s: &mut State| {
                s.menu = None;
                s.open_group_dialog(None, Some(sl.clone()));
            }));
            let (sl, tracked) = (slug.clone(), s.is_tracked(slug));
            menu = menu.separator().item(MenuItem::new(crate::i18n::t(if tracked { "Stop tracking in Activity" } else { "Track in Activity" })).checked(tracked).on_select(move |s: &mut State| {
                s.menu = None;
                s.toggle_tracked(&sl);
            }));
            let url = tf!("https://github.com/{slug}");
            menu = menu.separator().item(MenuItem::new(crate::i18n::t("Open on GitHub")).on_select_tx(move |s: &mut State, tx| {
                s.menu = None;
                crate::open_url(tx, &url);
            }));
        }
        MenuTarget::Owner(owner) => {
            let order = s.owners_sorted();
            let i = order.iter().position(|o| o == owner).unwrap_or(0);
            let last = order.len().saturating_sub(1);
            let (o1, o2, o3, o4) = (owner.clone(), owner.clone(), owner.clone(), owner.clone());
            menu = menu
                .group_label(crate::i18n::t(owner.clone()))
                .item(MenuItem::new(crate::i18n::t("Move up")).icon(&glyphs::CHEVRON_UP).disabled(i == 0).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_owner(&o1, Some(-1));
                }))
                .item(MenuItem::new(crate::i18n::t("Move down")).icon(&glyphs::CHEVRON_DOWN).disabled(i >= last).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_owner(&o2, Some(1));
                }))
                .item(MenuItem::new(crate::i18n::t("Move to top")).disabled(i == 0).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_owner(&o3, None);
                }))
                .item(MenuItem::new(crate::i18n::t("Reset order")).description(crate::i18n::t("you first, then A–Z")).disabled(s.prefs.owner_order.is_empty()).on_select(|s: &mut State| {
                    s.menu = None;
                    s.reset_owner_order();
                }))
                .separator()
                .item(MenuItem::new(crate::i18n::t("Customize colour & icon…")).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.open_owner_edit(&o4);
                }));
            let url = tf!("https://github.com/{owner}");
            menu = menu.separator().item(MenuItem::new(crate::i18n::t("Open on GitHub")).on_select_tx(move |s: &mut State, tx| {
                s.menu = None;
                crate::open_url(tx, &url);
            }));
        }
        MenuTarget::Group(name) => {
            let (n1, n2, n3, n4, n5) = (name.clone(), name.clone(), name.clone(), name.clone(), name.clone());
            let i = s.prefs.groups.iter().position(|g| g.name == *name).unwrap_or(0);
            let last = s.prefs.groups.len().saturating_sub(1);
            menu = menu
                .group_label(crate::i18n::t(name.clone()))
                .item(MenuItem::new(crate::i18n::t("Move group up")).icon(&glyphs::CHEVRON_UP).disabled(i == 0).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_group(&n4, -1);
                }))
                .item(MenuItem::new(crate::i18n::t("Move group down")).icon(&glyphs::CHEVRON_DOWN).disabled(i >= last).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_group(&n5, 1);
                }))
                .separator()
                .item(MenuItem::new(crate::i18n::t("Open dashboard")).icon(&super::I_DASH).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.open_group(&n3);
                }))
                .item(MenuItem::new(crate::i18n::t("Rename…")).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.open_group_dialog(Some(n1.clone()), None);
                }))
                .item(MenuItem::new(crate::i18n::t("Delete group")).danger(true).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.delete_group(&n2);
                }));
        }
    }
    Some(menu.build(k, layer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    /// The list as short labels, to compare its shape.
    fn shape(s: &State) -> Vec<String> {
        items(s)
            .iter()
            .map(|i| match i {
                Item::Part(Part::Groups) => "GROUPS".into(),
                Item::Part(Part::Repos) => "REPOS".into(),
                Item::NoGroups => "no-groups".into(),
                Item::Header { section, count, folded } => format!("[{}:{count}{}]", section.fold(), if *folded { " folded" } else { "" }),
                Item::Repo { idx, recent, slot, .. } => format!("{}{}{}", s.repos[*idx].slug, if *recent { " recent" } else { "" }, if slot.is_some() { " drag" } else { "" }),
                Item::Missing { slug, .. } => format!("missing {slug}"),
                Item::More { hidden, .. } => format!("more {hidden}"),
                Item::Less { .. } => "less".into(),
                Item::Searching => "searching".into(),
                Item::HitsHeader(_) | Item::Hit(_) => "hit".into(),
                Item::Nothing => "nothing".into(),
            })
            .collect()
    }

    fn state_with(repos: &[&str]) -> State {
        let mut s = State::new(config::Prefs::default());
        for r in repos {
            s.add_repo((*r).into(), None);
        }
        s
    }

    #[test]
    fn groups_and_repositories_are_two_parts() {
        let s = state_with(&["x/a"]);
        assert_eq!(shape(&s), ["GROUPS", "no-groups", "REPOS", "[o:x:1]", "x/a"]);
        // An empty repo list still shows its heading (it holds the add button).
        let s = state_with(&[]);
        assert_eq!(shape(&s), ["GROUPS", "no-groups", "REPOS"]);
    }

    #[test]
    fn the_last_opened_repo_leads_its_group_and_owner() {
        let mut s = state_with(&["x/a", "x/b", "x/c"]);
        s.prefs.groups = vec![config::Group { name: "g".into(), repos: vec!["x/a".into(), "x/b".into(), "x/c".into()] }];
        s.prefs.recent = vec!["x/c".into(), "x/b".into()];
        assert_eq!(
            shape(&s),
            ["GROUPS", "[g:g:3]", "x/c recent", "x/a drag", "x/b drag", "REPOS", "[o:x:3]", "x/c recent", "x/b", "x/a"],
        );
        // Nothing opened yet: no "recent", group order as made.
        s.prefs.recent.clear();
        assert_eq!(shape(&s)[1..5], ["[g:g:3]", "x/a drag", "x/b drag", "x/c drag"]);
    }

    #[test]
    fn folded_sections_hide_their_rows_and_searching_drops_empty_parts() {
        let mut s = state_with(&["x/a", "y/b"]);
        s.prefs.groups = vec![config::Group { name: "g".into(), repos: vec!["y/b".into()] }];
        s.collapsed.insert("o:x".into());
        assert_eq!(shape(&s), ["GROUPS", "[g:g:1]", "y/b drag", "REPOS", "[o:x:1 folded]", "[o:y:1]", "y/b"]);
        // Searching unfolds everything, and no drag while it filters.
        s.repo_filter = rune::TextEdit::new("x/");
        assert_eq!(shape(&s), ["REPOS", "[o:x:1]", "x/a"]);
    }

    #[test]
    fn dragged_headers_land_on_the_header_under_the_pointer() {
        let mut s = State::new(config::Prefs::default());
        s.prefs.groups = ["a", "b", "c"].map(|n| config::Group { name: n.into(), repos: Vec::new() }).to_vec();
        // Empty groups are one header row each: one row down is the next group.
        assert_eq!(drop_target(&s, true, "a", ROW), Some(1));
        assert_eq!(drop_target(&s, true, "a", ROW * 9.0), Some(2), "past the last one: the last place");
        assert_eq!(drop_target(&s, true, "c", -ROW * 9.0), Some(0), "above the first: the first place");
        assert_eq!(drop_target(&s, true, "b", 0.0), Some(1));
        // Owners: a section's repos count as rows between headers.
        for slug in ["x/one", "x/two", "y/one", "z/one"] {
            s.add_repo(slug.into(), None);
        }
        let order = s.owners_sorted();
        let first = order[0].clone();
        // The first owner's header, 2 repos, then the next header: 3 rows down.
        assert_eq!(drop_target(&s, false, &first, ROW * 3.0), Some(1));
        assert_eq!(drop_target(&s, false, &first, ROW * 1.0), Some(0), "still inside its own section");
    }
}
