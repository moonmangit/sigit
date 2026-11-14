//! The sidebar: Inbox, then your repositories in a virtualised list (hundreds of repos cost the
//! same as ten), in sections:
//!
//! 1. **Pinned**: repos you pinned (right-click → Pin to top).
//! 2. **Your groups**: groups you made, mixing repos from any owners (right-click → a group).
//! 3. **Owners**: you, then each organization, with every repo you can reach.
//!
//! Owner sections show their first 7 repos (recently opened on top, then recently pushed), with
//! "Show N more"; searching always shows every match.
//!
//! A repo can sit in several sections. Click a section header to fold it (or fold them all from
//! the header); folds are remembered. Drag pinned repos, and repos inside your groups, into the
//! order you want; groups move up and down from their right-click menu. The filter box narrows
//! everything on each keystroke, and after a short pause searches all of GitHub too ("On GitHub").

use std::borrow::Cow;

use rune::{El, Icon, el, row, spacer, text};
use rune_kit::shell::{Reorder, Sidebar, SidebarRow};
use rune_kit::{IconButton, IconButtonVariant, Kit, MenuItem, Spinner, TextInput, context_menu, glyphs};

use super::{I_CLOUD, I_INBOX, I_PLUS, I_REPO, chip, muted};
use crate::model::Outcome;
use crate::{MAX_DRAG_GROUPS, MenuTarget, Page, State};

const ROW: f32 = 40.0;

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
            Section::Group(g) => format!("g:{g}"),
            Section::Owner(o) => format!("o:{o}"),
        }
    }
}

/// One line of the flattened list.
enum Item {
    Header { section: Section, count: usize, folded: bool },
    /// `slot`: (position, count) in a list you can drag to reorder (pinned, or a group of
    /// yours), when dragging is possible (not while filtering).
    Repo { idx: usize, section: Section, slot: Option<(usize, usize)> },
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
        .filter_map(|(pos, p)| Some(Item::Repo { idx: index(p)?, section: Section::Pinned, slot: drag.then_some((pos, n)) }))
        .collect();
    push(&mut out, Section::Pinned, pinned);

    // 2. Your groups.
    for (gi, g) in s.prefs.groups.iter().enumerate() {
        let sec = Section::Group(g.name.clone());
        let n = g.repos.len();
        let drag = q.is_empty() && gi < MAX_DRAG_GROUPS;
        let members: Vec<Item> = g
            .repos
            .iter()
            .enumerate()
            .filter(|(_, r)| matches(r) || g.name.to_lowercase().contains(&q))
            .map(|(pos, r)| match index(r) {
                Some(idx) => Item::Repo { idx, section: sec.clone(), slot: drag.then_some((pos, n)) },
                None => Item::Missing { slug: r.clone(), group: g.name.clone() },
            })
            .collect();
        if members.is_empty() && q.is_empty() {
            // An empty group still shows, so it can be filled or deleted.
            out.push(Item::Header { section: sec, count: 0, folded: false });
        } else {
            push(&mut out, sec, members);
        }
    }

    // 3. Owners in your order (right-click a header to move it), else you first then A–Z.
    // Each shows its first few repos (Settings): the ones you opened most recently on top, then
    // the most recently pushed. Searching shows every match.
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
        let f = folded(&sec);
        out.push(Item::Header { section: sec.clone(), count: total, folded: f });
        if f {
            continue;
        }
        let expanded = s.expanded_orgs.contains(owner);
        // Settings → Appearance: how many per organization (0: all).
        let limit = if s.prefs.org_limit == 0 { usize::MAX } else { s.prefs.org_limit };
        let cut = q.is_empty() && !expanded && total > limit;
        for idx in idxs.into_iter().take(if cut { limit } else { total }) {
            out.push(Item::Repo { idx, section: sec.clone(), slot: None });
        }
        if cut {
            out.push(Item::More { owner: owner.to_owned(), hidden: total - limit });
        } else if q.is_empty() && expanded && total > limit {
            out.push(Item::Less { owner: owner.to_owned() });
        }
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
        Some(Item::Header { section, .. }) => format!("h:{}", section.fold()),
        Some(Item::Repo { idx, section, .. }) => format!("r:{}:{}", section.fold(), s.repos[*idx].slug),
        Some(Item::Missing { slug, group }) => format!("m:{group}:{slug}"),
        Some(Item::More { owner, .. }) => format!("more:{owner}"),
        Some(Item::Less { owner }) => format!("less:{owner}"),
        Some(Item::Searching) => "searching".into(),
        Some(Item::HitsHeader(_)) => "hits".into(),
        Some(Item::Hit(h)) => format!("hit:{}", s.repo_hits.1[*h].full_name),
        Some(Item::Nothing) | None => format!("empty:{i}"),
    })
}

static I_PIN: Icon = Icon {
    name: "g-pin",
    view: 16.0,
    body: r#"<path d="M6 2.5H10L9.5 6.5L12 9H4L6.5 6.5Z M8 9V14" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round" fill="none"/>"#,
};
static I_GROUP: Icon = Icon {
    name: "g-group",
    view: 16.0,
    body: r#"<rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="9" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="2.5" y="9" width="11" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/>"#,
};

fn header(s: &State, k: &Kit, section: &Section, count: usize, folded: bool) -> El {
    let fold = section.fold();
    let (label, icon): (String, Option<&'static Icon>) = match section {
        Section::Pinned => ("PINNED".into(), Some(&I_PIN)),
        Section::Group(g) => (g.to_uppercase(), Some(&I_GROUP)),
        Section::Owner(o) => (o.to_uppercase(), None),
    };
    let mut r = row()
        .w_full()
        .h(ROW)
        .px(8.0)
        .pt(12.0)
        .gap(6.0)
        .center_y()
        .radius(k.radius.md)
        .hover(k.c.hover)
        .role("button")
        .label(format!("{label}, {count} repositories"))
        .help("Click to fold or unfold. Right-click to reorder or customize.")
        .on_press(move |s: &mut State| s.toggle_fold(&fold))
        .child(rune::icon(if folded { &glyphs::TRIANGLE_RIGHT } else { &glyphs::TRIANGLE_DOWN }, 10.0).color(k.c.icon_faint));
    if let Some(i) = icon {
        r = r.child(rune::icon(i, 12.0).color(k.c.icon_muted));
    }
    if let Section::Owner(o) = section
        && let Some(b) = super::org::badge(s, k, o, 16.0)
    {
        r = r.child(b);
    }
    r = r
        .child(text(label).font(k.ty.text_xsmall_medium).color(k.c.text_muted).truncate().shrink())
        .child(spacer())
        .child(text(count.to_string()).font(k.ty.text_xsmall).color(k.c.text_faint));
    // Your groups: a dashboard button, and their own menu (rename, delete).
    if let Section::Group(g) = section {
        let g2 = g.clone();
        r = r.child(
            IconButton::new(&super::I_DASH)
                .variant(IconButtonVariant::GhostMuted)
                .label("Open dashboard")
                .help("Group dashboard: CI, deployments and open PRs across these repos")
                .on_press(move |s: &mut State| s.open_group(&g2))
                .build(k)
                .key("dash"),
        );
        let g = g.clone();
        r = r.on_secondary_press(move |s: &mut State, p, _| s.menu = Some((MenuTarget::Group(g.clone()), p)));
    }
    if let Section::Owner(o) = section {
        let o = o.clone();
        r = r.on_secondary_press(move |s: &mut State, p, _| s.menu = Some((MenuTarget::Owner(o.clone()), p)));
    }
    r
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
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .hover(k.c.hover)
        .role("button")
        .label(label.clone())
        .help(tip.to_owned())
        .on_press(f)
        .child(text("⋯").font(k.ty.ui).color(k.c.text_faint))
        .child(text(label).font(k.ty.small).color(k.c.text_accent))
}

/// Every section's fold key, for "collapse all".
fn fold_keys(s: &State) -> Vec<String> {
    let mut keys = vec!["pinned".to_owned()];
    keys.extend(s.prefs.groups.iter().map(|g| format!("g:{}", g.name)));
    for r in &s.repos {
        let k = format!("o:{}", r.owner());
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys
}

fn repo_row(s: &State, k: &Kit, idx: usize, section: &Section, slot: Option<(usize, usize)>) -> El {
    let with_owner = !matches!(section, Section::Owner(_));
    let repo = &s.repos[idx];
    let (_, name) = repo.slug.split_once('/').unwrap_or(("", &repo.slug));
    let slug = repo.slug.clone();
    let menu_slug = repo.slug.clone();
    let selected = s.page != Page::Inbox && s.cur.as_deref() == Some(&repo.slug);
    let mut r = SidebarRow::new(name.to_owned()).height(ROW);
    r = match super::org::badge(s, k, repo.owner(), 16.0) {
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
    if s.prefs.pinned.contains(&repo.slug) {
        r = r.trailing(rune::icon(&I_PIN, 11.0).color(k.c.icon_faint));
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
                    r = r.reorder(&s.group_drags[gi], pos, count, GROUP_DRAG[gi], move |s: &mut State, from, to| s.move_in_group(gi, from, to));
                    tip = "Drag to reorder this group · Right-click for more";
                }
            }
            Section::Owner(_) => {}
        }
    }
    r.build(k).help(tip).on_secondary_press(move |s: &mut State, p, _| s.menu = Some((MenuTarget::Repo(menu_slug.clone()), p)))
}

fn build(s: &State, i: usize, k: &Kit) -> El {
    let list = items(s);
    let Some(item) = list.get(i) else { return row() };
    match item {
        Item::Header { section, count, folded } => header(s, k, section, *count, *folded),
        Item::Repo { idx, section, slot } => repo_row(s, k, *idx, section, *slot),
        Item::Missing { slug, group } => {
            let (slug2, group2) = (slug.clone(), group.clone());
            row()
                .h(ROW)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .child(text(slug.clone()).font(k.ty.small).color(k.c.text_faint).truncate().shrink())
                .child(spacer())
                .child(
                    IconButton::new(&glyphs::CLOSE)
                        .variant(IconButtonVariant::GhostMuted)
                        .label("Remove from group")
                        .help("Not reachable: remove it from this group")
                        .on_press(move |s: &mut State| s.toggle_in_group(&group2, &slug2))
                        .build(k),
                )
        }
        Item::Searching => row().h(ROW).px(10.0).gap(8.0).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, "Searching GitHub…")),
        Item::HitsHeader(n) => row()
            .h(ROW)
            .px(8.0)
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
        Item::Nothing => row().h(ROW).px(10.0).center_y().child(muted(k, "No repositories match")),
        Item::More { owner, hidden } => {
            let o = owner.clone();
            more_row(k, format!("Show {hidden} more"), "Recently opened repos stay on top. Or type in the search box to find any repo.", move |s: &mut State| {
                s.expanded_orgs.insert(o.clone());
            })
        }
        Item::Less { owner } => {
            let o = owner.clone();
            more_row(k, "Show less".to_owned(), "Back to the first few", move |s: &mut State| {
                s.expanded_orgs.remove(&o);
            })
        }
    }
}

pub fn view(s: &State, k: &Kit) -> El {
    let reviews = s.inbox.data.as_ref().map_or(0, |i| i.review_requested.len());
    let mut inbox = SidebarRow::new("Inbox").icon(&I_INBOX).height(32.0).selected(s.page == Page::Inbox).on_select(|s: &mut State, _| s.show(Page::Inbox));
    if reviews > 0 {
        inbox = inbox.trailing(chip(k, reviews.to_string(), Some(Outcome::Running)));
    }
    let filter = TextInput::new(&s.repo_filter)
        .placeholder("Search repositories")
        .leading_icon(k.glyph.search)
        .width(None)
        .on_edit(|s: &mut State| &mut s.repo_filter)
        .on_change(|s: &mut State, _| s.search_repos());
    let filter = super::with_clear(filter, &s.repo_filter.text, |s| &mut s.repo_filter, State::search_repos)
        .build(k)
        .key("repo-filter");
    let new_group = IconButton::new(&I_GROUP)
        .variant(IconButtonVariant::GhostMuted)
        .label("New group")
        .help("New group…")
        .on_press(|s: &mut State| s.open_group_dialog(None, None))
        .build(k)
        .key("new-group");
    let add = IconButton::new(&I_PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .label("Add local repo")
        .help("Add a local clone…")
        .on_press_tx(crate::ask_local_repo)
        .build(k)
        .key("add-repo");
    let keys = fold_keys(s);
    let all_folded = keys.iter().all(|k| s.collapsed.contains(k));
    let fold_all = IconButton::new(if all_folded { &glyphs::CHEVRON_DOWN } else { &glyphs::CHEVRON_UP })
        .variant(IconButtonVariant::GhostMuted)
        .label(if all_folded { "Expand all" } else { "Collapse all" })
        .help(if all_folded { "Expand all sections" } else { "Collapse all sections (remembered)" })
        .on_press(move |s: &mut State| s.toggle_fold_all(keys.clone()))
        .build(k)
        .key("fold-all");
    let head = row()
        .w_full()
        .pt(12.0)
        .pb(4.0)
        .px(8.0)
        .gap(2.0)
        .center_y()
        .child(text(format!("REPOSITORIES · {}", s.repos.len())).font(k.ty.text_xsmall_medium).color(k.c.text_faint))
        .child(spacer())
        .child(fold_all)
        .child(new_group)
        .child(add);
    let count = items(s).len();
    let kc = k.clone();
    Sidebar::new()
        .label("Navigation")
        .header(inbox.build(k).key("nav-inbox"))
        .header(head)
        .header(rune::col().w_full().px(8.0).pb(6.0).child(filter))
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
                .group_label(slug.clone())
                .item(MenuItem::new(if pinned { "Unpin" } else { "Pin to top" }).icon(&I_PIN).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.toggle_pin(&sl);
                }))
                .separator()
                .group_label("Groups");
            for g in &s.prefs.groups {
                let (gn, sl) = (g.name.clone(), slug.clone());
                menu = menu.item(MenuItem::new(g.name.clone()).checked(g.repos.contains(slug)).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.toggle_in_group(&gn, &sl);
                }));
            }
            let sl = slug.clone();
            menu = menu.item(MenuItem::new("New group with this repo…").icon(&I_GROUP).on_select(move |s: &mut State| {
                s.menu = None;
                s.open_group_dialog(None, Some(sl.clone()));
            }));
            let (sl, tracked) = (slug.clone(), s.is_tracked(slug));
            menu = menu.separator().item(MenuItem::new(if tracked { "Stop tracking in Activity" } else { "Track in Activity" }).checked(tracked).on_select(move |s: &mut State| {
                s.menu = None;
                s.toggle_tracked(&sl);
            }));
            let url = format!("https://github.com/{slug}");
            menu = menu.separator().item(MenuItem::new("Open on GitHub").on_select_tx(move |s: &mut State, tx| {
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
                .group_label(owner.clone())
                .item(MenuItem::new("Move up").icon(&glyphs::CHEVRON_UP).disabled(i == 0).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_owner(&o1, Some(-1));
                }))
                .item(MenuItem::new("Move down").icon(&glyphs::CHEVRON_DOWN).disabled(i >= last).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_owner(&o2, Some(1));
                }))
                .item(MenuItem::new("Move to top").disabled(i == 0).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_owner(&o3, None);
                }))
                .item(MenuItem::new("Reset order").description("you first, then A–Z").disabled(s.prefs.owner_order.is_empty()).on_select(|s: &mut State| {
                    s.menu = None;
                    s.reset_owner_order();
                }))
                .separator()
                .item(MenuItem::new("Customize colour & icon…").on_select(move |s: &mut State| {
                    s.menu = None;
                    s.open_owner_edit(&o4);
                }));
            let url = format!("https://github.com/{owner}");
            menu = menu.separator().item(MenuItem::new("Open on GitHub").on_select_tx(move |s: &mut State, tx| {
                s.menu = None;
                crate::open_url(tx, &url);
            }));
        }
        MenuTarget::Group(name) => {
            let (n1, n2, n3, n4, n5) = (name.clone(), name.clone(), name.clone(), name.clone(), name.clone());
            let i = s.prefs.groups.iter().position(|g| g.name == *name).unwrap_or(0);
            let last = s.prefs.groups.len().saturating_sub(1);
            menu = menu
                .group_label(name.clone())
                .item(MenuItem::new("Move group up").icon(&glyphs::CHEVRON_UP).disabled(i == 0).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_group(&n4, -1);
                }))
                .item(MenuItem::new("Move group down").icon(&glyphs::CHEVRON_DOWN).disabled(i >= last).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.move_group(&n5, 1);
                }))
                .separator()
                .item(MenuItem::new("Open dashboard").icon(&super::I_DASH).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.open_group(&n3);
                }))
                .item(MenuItem::new("Rename…").on_select(move |s: &mut State| {
                    s.menu = None;
                    s.open_group_dialog(Some(n1.clone()), None);
                }))
                .item(MenuItem::new("Delete group").danger(true).on_select(move |s: &mut State| {
                    s.menu = None;
                    s.delete_group(&n2);
                }));
        }
    }
    Some(menu.build(k, layer))
}
