//! The window: sidebar of repos, toolbar, the current page, dialogs and toasts. Shared
//! widgets for the pages live here too.

mod actions;
mod branches;
pub mod cmd;
mod commits;
mod compare;
mod envs;
mod group;
mod history;
pub use history::{Act as HistoryAct, Anim as HistoryAnim, Open as HistoryOpen, on_head};
mod inbox;
mod org;
mod onboarding;
pub mod overlays;
pub mod split;
mod prs;
pub(crate) use prs::merge_action;
mod pulse;
mod pr_new;
mod settings;
mod sidebar;
mod subnav;

use cmd::CmdTip;
use rune::{Color, Cx, El, Icon, ScrollSpec, TextStyle, col, el, oklch, rich, row, spacer};
use crate::i18n::text;
use rune_kit::shell::{SplitView, Toolbar};
use rune_kit::syntax::{SyntaxTheme, highlight};
use rune_kit::{
    Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, SegmentedControl, Spinner, TextInput,
};

use crate::config::Theme;
use crate::model::Outcome;
use crate::{Load, Page, State};

// ---------------------------------------------------------------------------------------------
// Icons (16-unit line icons, drawn with currentColor)
// ---------------------------------------------------------------------------------------------

macro_rules! icon {
    ($id:ident, $name:literal, $body:literal) => {
        pub static $id: Icon = Icon { name: $name, view: 16.0, body: $body };
    };
}
icon!(I_INBOX, "g-inbox", r#"<path d="M2.5 9.5L4 3.5H12L13.5 9.5V12.5H2.5Z M2.5 9.5H5.5L6.5 11H9.5L10.5 9.5H13.5" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
icon!(I_REPO, "g-repo", r#"<path d="M4 2.5H12.5V11.5H4.5C3.7 11.5 3 12.2 3 13S3.7 14.5 4.5 14.5H12.5V11.5 M3 13V4C3 3.2 3.5 2.5 4 2.5" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
icon!(I_CLOUD, "g-cloud", r#"<path d="M4.5 12.5H11.5C13 12.5 14 11.4 14 10C14 8.7 13 7.6 11.7 7.5C11.4 5.5 9.9 4 8 4C6.3 4 4.9 5.2 4.5 6.8C3.1 7 2 8.2 2 9.7C2 11.2 3.1 12.5 4.5 12.5Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
icon!(I_REFRESH, "g-refresh", r#"<path d="M13 8A5 5 0 1 1 11.5 4.5M13 2.5V5.5H10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(I_PLUS, "g-plus", r#"<path d="M8 3.5V12.5M3.5 8H12.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#);
icon!(I_GEAR, "g-gear", r#"<g transform="scale(0.6667)" stroke="currentColor" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></g>"#);
icon!(I_HELP, "g-help", r#"<circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M6.3 6.3C6.3 5.3 7.1 4.6 8 4.6C8.9 4.6 9.7 5.3 9.7 6.2C9.7 7.4 8 7.5 8 9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/><circle cx="8" cy="11.3" r="0.8" fill="currentColor"/>"#);
icon!(I_EYE, "g-eye", r#"<path d="M1.5 8C3 5 5.3 3.5 8 3.5S13 5 14.5 8C13 11 10.7 12.5 8 12.5S3 11 1.5 8Z" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="8" cy="8" r="2" stroke="currentColor" stroke-width="1.2" fill="none"/>"#);
icon!(I_DASH, "g-dash", r#"<rect x="2.5" y="2.5" width="11" height="11" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M2.5 6.5H13.5M6.5 6.5V13.5" stroke="currentColor" stroke-width="1.2"/>"#);
icon!(I_BACK, "g-back", r#"<path d="M10 3.5L5.5 8L10 12.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(I_FWD, "g-fwd", r#"<path d="M6 3.5L10.5 8L6 12.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(I_LOGO, "g-logo", r#"<circle cx="4.5" cy="4" r="1.6" stroke="currentColor" stroke-width="1.3" fill="none"/><circle cx="4.5" cy="12" r="1.6" stroke="currentColor" stroke-width="1.3" fill="none"/><circle cx="11.5" cy="6.5" r="1.6" stroke="currentColor" stroke-width="1.3" fill="none"/><path d="M4.5 5.6V10.4M11.5 8.1C11.5 10 9 10.2 6 11.2" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none"/>"#);
icon!(I_COPY, "g-copy", r#"<rect x="5.5" y="5.5" width="8" height="8" rx="1.6" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M10.5 5.5V3.9C10.5 3.1 9.9 2.5 9.1 2.5H3.9C3.1 2.5 2.5 3.1 2.5 3.9V9.1C2.5 9.9 3.1 10.5 3.9 10.5H5.5" stroke="currentColor" stroke-width="1.2" fill="none"/>"#);
icon!(I_PR, "g-pr", r#"<circle cx="4.5" cy="3.8" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="4.5" cy="12.2" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11.5" cy="12.2" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.5 5.3V10.7M11.5 10.7V6.5C11.5 5.4 10.6 4.5 9.5 4.5H7.5M8.8 3.2L7.5 4.5L8.8 5.8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(I_CMD, "g-cmd", r#"<path d="M3 4.5L6.5 8L3 11.5M8 11.5H13" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(I_EXTERNAL, "g-ext", r#"<path d="M9 2.5H13.5V7M13.5 2.5L7.5 8.5M11.5 9.5V13.5H2.5V4.5H6.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);

// ---------------------------------------------------------------------------------------------
// Shared widgets
// ---------------------------------------------------------------------------------------------

pub fn outcome_color(k: &Kit, o: Outcome) -> Color {
    match o {
        Outcome::Success => k.c.success.icon,
        Outcome::Failure => k.c.danger.icon,
        Outcome::Running => k.c.warning.icon,
        Outcome::Neutral => k.c.icon_faint,
    }
}

/// A status mark: a spinner while running, else a dot in the outcome's colour.
pub fn status_mark(k: &Kit, o: Outcome) -> El {
    match o {
        Outcome::Running => Spinner::new().size(12.0).color(outcome_color(k, o)).build(k),
        _ => row().size(12.0, 12.0).center().child(el().size(9.0, 9.0).radius(4.5).fill(outcome_color(k, o))),
    }
}

/// A rounded label. `tone`: None = neutral.
pub fn chip(k: &Kit, label: impl Into<String>, tone: Option<Outcome>) -> El {
    let (bg, fg) = match tone {
        Some(Outcome::Success) => (k.c.success.bg, k.c.success.fg),
        Some(Outcome::Failure) => (k.c.danger.bg, k.c.danger.fg),
        Some(Outcome::Running) => (k.c.warning.bg, k.c.warning.fg),
        _ => (k.c.surface_base, k.c.text_muted),
    };
    row().px(7.0).py(1.0).radius(999.0).fill(bg).child(text(label.into()).font(k.ty.text_xsmall_medium).color(fg))
}

/// A search box's ×. rune-kit's `clear_button` only draws the button; its press goes to
/// `on_action`. This clears the text (keeping focus) and runs `after`, since clearing this way
/// doesn't fire `on_change`. Shown only when there's text to clear.
pub fn with_clear(input: TextInput, text: &str, edit: fn(&mut State) -> &mut rune::TextEdit, after: fn(&mut State)) -> TextInput {
    if text.is_empty() {
        return input;
    }
    input.clear_button().on_action(move |s: &mut State, _| {
        edit(s).clear();
        after(s);
    })
}

/// A PR's checks, as one clear chip: ✓ passed / ✗ failing / ● running / no checks.
pub fn ci_chip(k: &Kit, ci: crate::model::Ci) -> El {
    chip(k, ci.label(), ci.outcome()).help(crate::i18n::t(match ci {
        crate::model::Ci::Passed => "Every check on the latest commit passed: ready to review",
        crate::model::Ci::Failing => "A check on the latest commit failed: not ready to review",
        crate::model::Ci::Running => "Checks are still running on the latest commit",
        crate::model::Ci::None => "No checks ran on it (Actions may be waiting for approval): not counted as passed",
    }))
}

/// A branch chip with a 🛡 when the branch is protected or the default (A2).
pub fn branch_chip(s: &State, k: &Kit, name: &str, color: Color) -> El {
    if s.is_protected(name) {
        ref_pill(k, &tf!("🛡 {name}"), name, hue(6)).help(crate::i18n::t("Protected or default branch: changes to it need extra care (merges are High risk). Click to copy its name."))
    } else {
        ref_chip(k, name, color)
    }
}

/// A branch or ref name in mono on a soft accent pill; clicking copies the name.
pub fn ref_chip(k: &Kit, name: &str, color: Color) -> El {
    ref_pill(k, name, name, color)
}

/// A ref pill showing `shown` (with a 🛡 or 🏷, say) that copies `value` when clicked.
pub fn ref_pill(k: &Kit, shown: &str, value: &str, color: Color) -> El {
    click_to_copy(pill_body(k, shown, color, false), k, value.to_owned(), "name")
}

/// [`ref_chip`] for a narrow table column: a long name ends in "…" instead of being cut off.
pub fn ref_chip_fit(k: &Kit, name: &str, color: Color) -> El {
    click_to_copy(pill_body(k, name, color, true), k, name.to_owned(), "name").help_when_truncated(name.to_owned())
}

fn pill_body(k: &Kit, shown: &str, color: Color, fit: bool) -> El {
    let label = text(shown.to_owned()).font(TextStyle { size: 11.0, ..k.ty.mono }).color(k.c.text_base);
    let pill = row().px(6.0).py(1.0).radius(4.0).fill(Color { a: 0.16, ..color }).border(1.0, Color { a: 0.45, ..color });
    if fit { pill.shrink().child(label.truncate().shrink()) } else { pill.child(label) }
}

// ---------------------------------------------------------------------------------------------
// Copying
// ---------------------------------------------------------------------------------------------

/// Put `value` on the clipboard and say so ("Copied commit SHA: 3f2a9c1…").
fn copy_now(s: &mut State, tx: &mut rune::Tx, value: &str, what: &str) {
    tx.copy(value.to_owned());
    let what = crate::i18n::t(what);
    let shown: String = value.chars().take(60).collect();
    let more = if value.chars().count() > 60 { "…" } else { "" };
    s.toast(crate::i18n::t(tf!("Copied {what}: {shown}{more}")), false);
}

/// A small copy icon next to something people often paste elsewhere (a link, a SHA, a path).
pub fn copy_button(k: &Kit, key: &str, value: String, what: &str) -> El {
    let what = crate::i18n::t(what);
    IconButton::new(&I_COPY)
        .variant(IconButtonVariant::GhostMuted)
        .label(crate::i18n::t(tf!("Copy {what}")))
        .help(crate::i18n::t(tf!("Copy {what}")))
        .on_press_tx(move |s: &mut State, tx| copy_now(s, tx, &value, &what))
        .build(k)
        .key(key.to_owned())
}

/// Makes `e` (a name, a SHA, a path) copy `value` when clicked, with a hint on hover.
pub fn click_to_copy(e: El, k: &Kit, value: String, what: &str) -> El {
    let shown = crate::i18n::t(what);
    let what = what.to_owned();
    e.hover(k.c.hover).help(tf!("Click to copy {what}", what = shown)).on_press_tx(move |s: &mut State, tx| copy_now(s, tx, &value, &what))
}

/// Lane / series colours: evenly spaced hues, one lightness.
pub fn hue(i: usize) -> Color {
    const H: [f32; 8] = [250.0, 150.0, 30.0, 300.0, 200.0, 90.0, 0.0, 330.0];
    oklch(0.68, 0.14, H[i % H.len()])
}

/// A button's tooltip: `help` and the commands `action` runs (see [`cmd::CmdTip`]).
pub fn action_help(help: &str, action: &crate::guard::Action) -> cmd::Tip {
    cmd::Tip::new(crate::i18n::t(help), action.command_lines(false))
}

/// The gh / git commands an action runs, as terminal lines ("$ gh pr merge 12 --squash"), with a
/// button to copy them all. Empty when Settings hides commands.
pub fn command_block(s: &State, k: &Kit, key: &str, title: &str, lines: &[String]) -> El {
    if !s.prefs.show_commands || lines.is_empty() {
        return row();
    }
    let list = cmd::lines(k, lines, 12.0).w(0.0).grow(1.0);
    col()
        .key(key.to_owned())
        .w_full()
        .gap(6.0)
        .child(row().w_full().center_y().child(text(title.to_owned()).font(k.ty.small_medium).color(k.c.text_muted)).child(spacer()).child(copy_button(k, &format!("{key}-copy"), lines.join("\n"), "commands")))
        .child(row().w_full().padding(10.0).radius(k.radius.md).fill(k.c.bg_base).border(1.0, k.c.border_weak).child(list))
}

/// Label / value pairs under a detail's title (Status, Branch, Commit…), labels in one
/// column so the values line up.
pub fn facts(k: &Kit, pairs: Vec<(&str, El)>) -> El {
    pairs.into_iter().fold(col().w_full().gap(7.0), |c, (label, value)| {
        c.child(row().w_full().gap(12.0).center_y().child(text(label.to_owned()).font(k.ty.small).color(k.c.text_faint).w(76.0)).child(row().shrink().min_w(0.0).child(value)))
    })
}

/// A bordered surface with a title row.
pub fn panel(k: &Kit, key: &str, title: &str, trailing: Option<El>, body: El) -> El {
    let mut head = row().w_full().center_y().gap(8.0).child(text(title.to_owned()).font(k.ty.ui_medium).color(k.c.text_base)).child(spacer());
    if let Some(t) = trailing {
        head = head.child(t);
    }
    col().key(key.to_owned()).w_full().gap(12.0).padding(16.0).radius(k.radius.lg).fill(k.c.layer_01).border(1.0, k.c.border_weak).child(head).child(body)
}

pub fn muted(k: &Kit, s: impl Into<String>) -> El {
    text(s.into()).font(k.ty.small).color(k.c.text_muted)
}

/// A list the shared filters (sidebar 2) left empty.
pub fn filtered_empty(k: &Kit, key: &str, icon: &'static Icon) -> El {
    empty_state(k, key, icon, "No matches for these filters", "Nothing here was created in that range or by that contributor. Widen the date range or clear the filters in the side menu.", None)
}

pub fn empty(k: &Kit, msg: &str) -> El {
    col().w_full().py(24.0).items_center().child(text(msg.to_owned()).font(k.ty.ui).color(k.c.text_faint))
}

/// What a `Load` shows before its first answer: a spinner, or the error.
pub fn pending<T>(k: &Kit, l: &Load<T>) -> Option<El> {
    if l.data.is_some() {
        return None;
    }
    Some(match &l.error {
        Some(e) => col().w_full().padding(24.0).gap(6.0).child(text("Couldn't load this").font(k.ty.ui_medium).color(k.c.danger.fg)).child(muted(k, e.clone()).wrap()),
        None => col().w_full().py(40.0).items_center().child(Spinner::new().size(18.0).build(k)),
    })
}

/// A coloured unified diff, line numbers off, scrolling both ways.
pub fn diff_view(k: &Kit, key: &str, diff: &str) -> El {
    if diff.trim().is_empty() {
        return empty(k, "No changes to show");
    }
    let spans = SyntaxTheme::from_kit(k).spans(&highlight(Some("diff"), diff), 0);
    col()
        .key(key.to_owned())
        .w_full()
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_weak)
        .clip_rounded(k.radius.md)
        .child(col().w_full().padding(10.0).child(rich(diff.to_owned(), spans).font(k.ty.mono).color(k.c.text_base).exact_lines().selectable(true)).scroll_x())
}

/// "+12 −3" with a five-block bar, GitHub style.
pub fn diffstat(k: &Kit, add: u64, del: u64) -> El {
    let total = (add + del).max(1);
    let green = ((add * 5 + total / 2) / total) as usize;
    let mut bar = row().gap(1.0);
    for i in 0..5 {
        let c = if add + del == 0 { k.c.surface_base } else if i < green { k.c.success.icon } else { k.c.danger.icon };
        bar = bar.child(el().size(6.0, 6.0).radius(1.0).fill(c));
    }
    row()
        .gap(6.0)
        .center_y()
        .child(text(tf!("+{add}")).font(k.ty.text_xsmall_medium).color(k.c.success.fg))
        .child(text(tf!("−{del}")).font(k.ty.text_xsmall_medium).color(k.c.danger.fg))
        .child(bar)
}

/// The bottom of a list that may have more rows on GitHub: "Load N more".
pub fn load_more(k: &Kit, key: &str, shown: usize, page: usize, loading: bool, more: fn(&mut State)) -> El {
    let mut r = row().key(key.to_owned()).w_full().px(12.0).py(8.0).gap(10.0).center_y().border(1.0, k.c.border_weak);
    r = r.child(muted(k, tf!("Showing {shown}")));
    r = r.child(spacer());
    if loading {
        r = r.child(Spinner::new().size(12.0).build(k));
    }
    r.child(Button::new(crate::i18n::t(tf!("Load {page} more"))).variant(ButtonVariant::Outline).disabled(loading).on_press(more).build(k).key("more"))
}

pub fn ext_button(k: &Kit, key: &str, url: String) -> El {
    IconButton::new(&I_EXTERNAL)
        .variant(IconButtonVariant::GhostMuted)
        .label(crate::i18n::t("Open on GitHub"))
        .help(crate::i18n::t("Open on GitHub"))
        .on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url))
        .build(k)
        .key(key.to_owned())
}

pub fn scroll(key: &str, content: El) -> El {
    col().key(key.to_owned()).w_full().h_full().child(content.scroll(ScrollSpec { stick_bottom: false }).shrink().min_h(0.0))
}

/// A detail pane beside a list.
pub fn side_pane(k: &Kit, key: &str, content: El) -> El {
    col()
        .key(key.to_owned())
        .w_full()
        .h_full()
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_weak)
        .radius(k.radius.lg)
        .clip_rounded(k.radius.lg)
        .child(scroll("pane-scroll", col().w_full().padding(18.0).gap(14.0).child(content)))
}

// ---------------------------------------------------------------------------------------------
// Shell
// ---------------------------------------------------------------------------------------------

/// The theme and type for this frame: light or dark, Thai line heights, the modal backdrop.
fn kit(s: &State, cx: &mut Cx<State>) -> Kit {
    let dark = match s.prefs.theme {
        Theme::System => cx.system_dark(),
        Theme::Light => false,
        Theme::Dark => true,
    };
    crate::i18n::apply(s.prefs.language);
    let mut k = Kit::neutral(dark);
    // The kit sets 11 pt small text on a 12 pt line, which crams any description that wraps
    // onto a second line; give it room to breathe.
    k.ty.small.line_height = 16.0;
    k.ty.small_medium.line_height = 16.0;
    if crate::i18n::is_thai() {
        // Thai stacks vowels and tone marks above and below the letters: every style gets about
        // 1.6× its size so wrapped lines don't touch.
        for st in [&mut k.ty.ui, &mut k.ty.ui_medium, &mut k.ty.ui_compact, &mut k.ty.small, &mut k.ty.small_medium, &mut k.ty.body, &mut k.ty.body_medium, &mut k.ty.text_small, &mut k.ty.text_small_medium, &mut k.ty.text_xsmall, &mut k.ty.text_xsmall_medium] {
            st.line_height = st.line_height.max((st.size * 1.6).round());
        }
    }
    // A darker backdrop behind every modal (dialogs, the guide, the palette) so what's in
    // front stands out; the kit's default is 30% black.
    k.c.scrim = Color::BLACK.alpha(if dark { 0.62 } else { 0.48 });
    k
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let k = kit(s, cx);
    cx.zoom(s.prefs.zoom); // Settings → Appearance → Text size
    let mut layer = Vec::new();

    let body = match s.page {
        Page::Inbox => inbox_page(s, &k),
        Page::Group => group::view(s, &k),
        page => repo_page(s, &k, page),
    };
    // Sidebar 2: the sections of the repo or group sidebar 1 picked, and the shared filters.
    let body = match subnav::view(s, &k, &mut layer) {
        Some(nav) => row().w_full().h_full().child(nav).child(col().w(0.0).grow(1.0).min_w(0.0).h_full().child(body)),
        None => body,
    };
    history::reword_dialog(s, &k, &mut layer);
    overlays::guard_dialog(s, &k, &mut layer);
    group_dialog(s, &k, &mut layer);
    org::dialog(s, &k, &mut layer);
    pulse::tracked_dialog(s, &k, &mut layer);
    compare::repo_dialog(s, &k, &mut layer);
    pr_new::pr_dialog(s, &k, &mut layer);
    branches::base_list(s, &k, &mut layer);
    let menu_anchor = sidebar::context(s, &k, &mut layer);
    overlays::palette(s, &k, &mut layer);
    settings::dialog(s, &k, &mut layer);
    overlays::guide(s, &k, &mut layer);
    overlays::grace_banner(s, &k, &mut layer);
    overlays::toasts(s, &k, &mut layer);
    overlays::crash_dialog(s, &k, &mut layer);
    cmd::tip_layer(s, &k, &mut layer);
    // Every plain tooltip in the same card look, fading in and out a little.
    cx.help_look(cmd::help_look(&k));

    let title = match (s.page, s.cur.as_deref(), s.group_cur.as_deref()) {
        (Page::Group, _, Some(g)) if s.group_tab > 0 => tf!("Group · {g} · {}", crate::i18n::t(crate::GROUP_TABS[s.group_tab])),
        (Page::Group, _, Some(g)) => tf!("Group · {g}"),
        (Page::Inbox, ..) | (_, None, _) => crate::i18n::t("Dashboard"),
        (p, Some(slug), _) => tf!("{slug} · {}", crate::i18n::t(p.label())),
    };
    let mut trailing = row().gap(10.0).center_y();
    for b in &s.busy {
        trailing = trailing.child(row().gap(6.0).center_y().child(Spinner::new().size(12.0).build(&k)).child(muted(&k, tf!("{b}…"))));
    }
    let watching = s.watches.len();
    if watching > 0 {
        trailing = trailing.child(
            chip(&k, tf!("👁 {watching}"), None)
                .help(crate::i18n::t(tf!("Watching {watching}: {}. Manage in Settings → Notifications.", s.watches.iter().map(|w| w.label.as_str()).collect::<Vec<_>>().join(", "))))
                .on_press(|s: &mut State| s.open_settings(crate::settings::NOTIFICATIONS)),
        );
    }
    trailing = trailing.child(
        IconButton::new(&I_REFRESH)
            .variant(IconButtonVariant::GhostMuted)
            .label(crate::i18n::t("Refresh"))
            .help(crate::i18n::t("Refresh"))
            .shortcut("Ctrl+R")
            .on_press(|s: &mut State| s.refresh_page())
            .build(&k)
            .key("refresh"),
    );

    // Back / forward, like a browser.
    let nav = |icon: &'static Icon, key: &str, label: &str, shortcut: &str, to: Option<&crate::Place>, go: fn(&mut State)| {
        IconButton::new(icon)
            .variant(IconButtonVariant::GhostMuted)
            .label(crate::i18n::t(label.to_owned()))
            .help(crate::i18n::t(match to {
                Some(p) => tf!("{label} to {}", p.label(), label = crate::i18n::t(label)),
                None => tf!("Nothing to go {}", crate::i18n::t(label.to_lowercase())),
            }))
            .shortcut(shortcut.to_owned())
            .disabled(to.is_none())
            .on_press(go)
            .build(&k)
            .key(key.to_owned())
    };
    let leading = row()
        .gap(2.0)
        .center_y()
        .child(nav(&I_BACK, "nav-back", "Back", "Alt+←", s.back.last(), State::go_back))
        .child(nav(&I_FWD, "nav-fwd", "Forward", "Alt+→", s.fwd.last(), State::go_forward));
    // How fresh this page is (click to reload it).
    let leading = match freshness(s) {
        Some(f) => leading.child(row().pl(6.0).child(overlays::fresh_chip(&k, &f))),
        None => leading,
    };
    let shell = SplitView::new(body.fill(k.c.bg_base))
        .sidebar(&s.sidebar, |s: &mut State| &mut s.sidebar, sidebar::view(s, &k))
        .toolbar(Toolbar::new().leading(leading).title(crate::i18n::t(title)).trailing(trailing))
        .frame_inset(8.0)
        .toolbar_in_panel()
        .build(&k);
    cx.caption_color(if k.dark { Color::WHITE } else { Color::BLACK });
    let ready = !s.setup.covers_app() && onboarding::splash_message(s).is_none();
    let bar = titlebar(&k, cx.chrome_insets(), ready);
    // Setup missing something, or a first run: that screen takes the window.
    let main = if s.setup.covers_app() {
        onboarding::view(s, &k)
    } else if let Some(msg) = onboarding::splash_message(s) {
        onboarding::splash(&k, msg)
    } else {
        shell
    };
    let mut app = col()
        .key("app")
        .w_full()
        .h_full()
        .fill(k.c.page).child(bar).child(col().w_full().grow(1.0).min_h(0.0).child(main));
    if let Some(anchor) = menu_anchor {
        app = app.child(anchor.absolute(0.0, 0.0));
    }
    // Window focus is only told to the root element, which `rune_kit::root` makes: coming back
    // to the window checks the page again.
    rune_kit::root(app, layer).on_window_focus(|s: &mut State, focused, _| s.window_focus(focused))
}

/// What the window shows when building the app's view panicked: the error, a way to copy it,
/// and a way back to a page that works. Plain on purpose, so it can't fail the same way.
pub fn crash_screen(s: &State, cx: &mut Cx<State>) -> El {
    let k = kit(s, cx);
    let report = s.crash.last().cloned().or_else(crate::crash_log::latest);
    let card = overlays::crash_card(&k, report.as_ref(), "This page hit an error, so sigit stopped drawing it. Everything else is still running.")
        .w(640.0)
        .padding(20.0)
        .radius(k.radius.lg)
        .fill(k.c.surface_base)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(overlays::copy_report_button(&k, report.as_ref()))
                .child(
                    Button::new(crate::i18n::t("Go to Dashboard"))
                        .variant(ButtonVariant::Submit)
                        .on_press(|s: &mut State| {
                            s.crash.clear();
                            s.show(Page::Inbox);
                        })
                        .build(&k)
                        .key("crash-inbox"),
                ),
        );
    let bar = titlebar(&k, cx.chrome_insets(), false);
    col().key("app").w_full().h_full().fill(k.c.page).child(bar).child(col().w_full().grow(1.0).center().padding(16.0).child(card))
}

/// New group / rename group: a name field and Save.
fn group_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(edit) = &s.group_edit else { return };
    let title = match (&edit.rename, &edit.add) {
        (Some(old), _) => tf!("Rename {old}"),
        (None, Some(slug)) => tf!("New group with {slug}"),
        (None, None) => "New group".to_owned(),
    };
    let input = TextInput::new(&s.group_name)
        .placeholder(crate::i18n::t("e.g. Payments, Mobile, Platform"))
        .label(crate::i18n::t("Group name"))
        .width(None)
        .on_edit(|s: &mut State| &mut s.group_name)
        .on_submit(|s: &mut State, _| s.finish_group_edit());
    let field = col()
        .w_full()
        .gap(6.0)
        .child(text("Group name").font(k.ty.small_medium).color(k.c.text_base))
        .child(with_clear(input, &s.group_name.text, |s| &mut s.group_name, |_| {}).build(k).key("group-name"))
        .child(muted(k, "Enter saves"));
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(14.0)
        .child(muted(k, "Groups are yours alone (saved on this computer) and can mix repos from any owners.").wrap().w_full())
        .child(field)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.group_edit = None).build(k).key("g-cancel"))
                .child(Button::new(crate::i18n::t("Save")).variant(ButtonVariant::Submit).on_press(|s: &mut State| s.finish_group_edit()).build(k).key("g-save")),
        );
    Dialog::new("group", crate::i18n::t(title)).open(true).fit().width(440.0).on_close(|s: &mut State| s.group_edit = None).child(body).build(k, layer);
}

/// The Inbox: Activity (your tracked repos over a range) and Needs you (queue, notifications).
fn inbox_page(s: &State, k: &Kit) -> El {
    let needs = s.review_queue().len() + s.visible_notifs().len();
    let tabs = SegmentedControl::new(&crate::i18n::all(&["Activity".to_owned(), tf!("Needs you ({needs})"), "Comparisons".to_owned()]), Some(s.inbox_tab))
        .label(crate::i18n::t("Dashboard view"))
        .fit()
        .on_pick(|s: &mut State, i, _| {
            s.inbox_tab = i;
            if i == crate::COMPARE_TAB {
                s.revalidate_page();
            }
        })
        .build(k)
        .key("inbox-tabs")
        .help(crate::i18n::t("Activity: what changed in your tracked repos. Needs you: reviews, notifications, your PRs. Comparisons: commits and PRs waiting between branches."));
    let body = match s.inbox_tab {
        0 => pulse::view(s, k),
        crate::COMPARE_TAB => compare::view(s, k),
        _ => inbox::view(s, k),
    };
    let bar = row().w_full().px(24.0).pt(14.0).gap(10.0).center_y().flex_wrap().child(tabs).child(spacer()).children(scope_picker(s, k));
    col().key("inbox-page").w_full().h_full().child(bar).child(col().w_full().grow(1.0).min_h(0.0).child(body))
}

/// Which repos every Dashboard tab shows: all, one of your groups, or the tracked repos in no
/// group. Hidden until you have a group.
fn scope_picker(s: &State, k: &Kit) -> Option<El> {
    use crate::RepoScope;
    let groups = &s.prefs.groups;
    if groups.is_empty() {
        return None;
    }
    let tracked = s.tracked();
    let ungrouped = tracked.iter().filter(|r| !groups.iter().any(|g| g.repos.contains(r))).count();
    let mut labels = vec![tf!("All ({n})", n = tracked.len())];
    let mut scopes: Vec<Option<RepoScope>> = vec![None];
    for g in groups {
        labels.push(format!("{} ({})", g.name, g.repos.len()));
        scopes.push(Some(RepoScope::Group(g.name.clone())));
    }
    if ungrouped > 0 {
        labels.push(tf!("Other ({n})", n = ungrouped));
        scopes.push(Some(RepoScope::Other));
    }
    let picked = scopes.iter().position(|x| x.as_ref() == s.scope_now()).unwrap_or(0);
    Some(
        SegmentedControl::new(&labels, Some(picked))
            .label(crate::i18n::t("Which repos"))
            .fit()
            .on_pick(move |s: &mut State, i, _| s.set_repo_scope(scopes.get(i).cloned().flatten()))
            .build(k)
            .key("repo-scope")
            .help(crate::i18n::t("Which repos the Dashboard shows, on every tab: all of them, one of your groups, or the tracked repos in no group")),
    )
}

/// How fresh what the page shows is: the oldest of its parts, and whether any is loading.
pub struct Fresh {
    pub loading: bool,
    /// When the oldest shown part was fetched (None: nothing shown yet).
    pub at: Option<i64>,
    /// Some part is from last session's cache.
    pub cached: bool,
    pub error: Option<String>,
    /// How often the page re-checks itself (seconds), for the tooltip.
    pub ttl: i64,
}

impl Fresh {
    fn add<T>(&mut self, l: &crate::Load<T>) {
        self.loading |= l.loading;
        self.cached |= l.cached && l.data.is_some();
        if let Some(t) = l.at.filter(|_| l.data.is_some()) {
            self.at = Some(self.at.map_or(t, |a| a.min(t)));
        }
        if self.error.is_none() {
            self.error = l.error.clone();
        }
    }
}

/// The current page's freshness.
fn freshness(s: &State) -> Option<Fresh> {
    use crate::ttl;
    let mut f = Fresh { loading: false, at: None, cached: false, error: None, ttl: ttl::GH };
    match s.page {
        Page::Inbox if s.inbox_tab == 0 => {
            f.add(&s.pulse);
            f.ttl = ttl::PULSE;
        }
        Page::Inbox if s.inbox_tab == crate::COMPARE_TAB => f.add(&s.compare),
        Page::Inbox => {
            f.add(&s.inbox);
            f.add(&s.notifs);
        }
        Page::Group => {
            let g = s.group_cur.as_ref().and_then(|g| s.prefs.groups.iter().find(|x| x.name == *g))?;
            for r in &g.repos {
                if let Some(l) = s.group_data.get(r) {
                    f.add(l);
                }
            }
        }
        Page::Commits | Page::Branches if s.repo().is_some_and(|r| r.path.is_none()) => return None,
        page => {
            let d = s.rd()?;
            f.add(&d.status);
            match page {
                Page::Pulls => {
                    f.add(&d.prs);
                    if s.pr_sel.is_some() {
                        f.add(&s.pr_detail);
                    }
                }
                Page::Actions => {
                    f.add(&d.runs);
                    if s.run_sel.is_some() {
                        f.add(&s.run_detail);
                    }
                    f.ttl = if s.runs_running() { ttl::RUNNING } else { ttl::RUNS };
                }
                Page::Envs => f.add(&d.envs),
                Page::Commits => {
                    f.add(&d.graph);
                    f.ttl = ttl::LOCAL;
                }
                Page::Branches => {
                    f.add(&d.branches);
                    f.add(&d.stashes);
                    f.add(&d.cleanup);
                    f.ttl = ttl::LOCAL;
                }
                _ => {}
            }
        }
    }
    Some(f)
}

/// Header (repo, branch, local state, fetch/pull) + page tabs + the page.
fn repo_page(s: &State, k: &Kit, page: Page) -> El {
    let Some(repo) = s.repo() else { return empty_state(k, "no-repo", &I_REPO, "Pick a repository", "Choose one in the sidebar, or press Ctrl+K to jump to any repo by name.", None) };
    let d = s.rd();
    // The window title names the repo and the page: this row is its state and actions.
    let mut head = row().key("repo-head").w_full().gap(10.0).center_y().flex_wrap();
    if let Some(st) = d.and_then(|d| d.status.data.as_ref()) {
        head = head.child(branch_chip(s, k, &st.branch, hue(0)).help(crate::i18n::t("The branch your local clone is on")));
        if st.changed > 0 {
            head = head.child(chip(k, tf!("{} changed", st.changed), Some(Outcome::Running)));
        }
        if st.ahead > 0 || st.behind > 0 {
            head = head.child(chip(k, tf!("↑{} ↓{}", st.ahead, st.behind), None));
        }
    } else if repo.path.is_none() {
        head = head.child(chip(k, "GitHub only", None).help(crate::i18n::t("No local clone added: Commits and Branches need one (right-click → or + in the sidebar)")));
    }
    head = head.child(spacer());
    if repo.path.is_some() {
        head = head
            .child({ let tip = s.with_commands(crate::i18n::t("Download new commits and branches from every remote. Changes nothing locally."), &[crate::git::fetch_cmd().line()]); Button::new(crate::i18n::t("Fetch")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.fetch()).build(k).key("fetch").cmd_tip(s, tip) })
            .child({ let tip = s.with_commands(crate::i18n::t("Fast-forward your current branch: never creates a merge commit. Checks for uncommitted changes first."), &[crate::git::pull_cmd().line()]); Button::new(crate::i18n::t("Pull")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.pull()).build(k).key("pull").cmd_tip(s, tip) });
    }
    head = head
        .child(copy_button(k, "repo-clone", tf!("https://github.com/{}.git", repo.slug), "clone URL"))
        .child(ext_button(k, "repo-open", tf!("https://github.com/{}", repo.slug)));

    let body = match page {
        Page::Pulls => prs::view(s, k),
        Page::Actions => actions::view(s, k),
        Page::Envs => envs::view(s, k),
        Page::Commits | Page::Branches if repo.path.is_none() => no_clone(s, k, &repo.slug),
        Page::Commits => commits::view(s, k),
        Page::Branches => branches::view(s, k),
        Page::Inbox | Page::Group => unreachable!(),
    };
    col()
        .key("repo-page")
        .w_full()
        .h_full()
        .padding(16.0)
        .gap(12.0)
        .child(head)
        .child(col().key(tf!("page:{page:?}")).w_full().grow(1.0).min_h(0.0).child(body))
}

fn no_clone(s: &State, k: &Kit, slug: &str) -> El {
    // Clone for me: into the managed folder; or point at a clone you already have.
    let busy = s.busy.contains(&format!("Clone {slug}"));
    let target = s.clone_home().and_then(|h| crate::managed::dir_for(&h, slug));
    let mut actions = row().gap(8.0);
    if let Some(dir) = &target {
        let line = crate::managed::clone_cmd(slug, dir).line();
        let tip = s.with_commands(crate::i18n::t(tf!("Downloads {slug} into {}, then opens it here. A clone already there is used as it is.", dir.display())), &[line]);
        actions = actions.child(
            Button::new(crate::i18n::t(if busy { "Cloning…" } else { "Clone for me" }))
                .variant(ButtonVariant::Contrast)
                .disabled(busy)
                .on_press(|s: &mut State| s.clone_for_me())
                .build(k)
                .key("clone-for-me")
                .cmd_tip(s, tip),
        );
    }
    actions = actions.child(Button::new(crate::i18n::t("Locate local clone…")).variant(ButtonVariant::Outline).on_press_tx(crate::ask_local_repo).build(k).key("locate"));
    let mut c = empty_state(
        k,
        "no-clone",
        &I_REPO,
        "No local clone",
        &tf!("Commits, branches and history editing read a local copy of {slug}. sigit can clone it for you, or you can point it at a clone you already have."),
        Some(actions),
    );
    if let Some(dir) = target {
        c = c.child(muted(k, tf!("It goes in {} (change the folder in Settings → Git)", dir.display())).pt(4.0));
    }
    if busy {
        c = c.child(row().gap(8.0).center_y().child(Spinner::new().size(14.0).build(k)).child(muted(k, "Downloading… large repos can take a minute")));
    }
    c
}

/// A page or panel with nothing to show yet: an icon on a soft tile, a title, one or two
/// lines of explanation, and the action that fills it (if any).
pub fn empty_state(k: &Kit, key: &str, icon: &'static Icon, title: &str, body: &str, action: Option<El>) -> El {
    let mut c = col()
        .key(key.to_owned())
        .w_full()
        .py(56.0)
        .px(24.0)
        .items_center()
        .gap(10.0)
        .child(row().size(48.0, 48.0).radius(k.radius.lg).center().fill(k.c.surface_base).border(1.0, k.c.border_weak).child(rune::icon(icon, 22.0).color(k.c.icon_base)))
        .child(text(title.to_owned()).font(TextStyle { size: 17.0, ..k.ty.ui_medium }).color(k.c.text_strong).pt(4.0))
        .child(text(body.to_owned()).font(k.ty.ui).color(k.c.text_muted).wrap().align(rune::Align::Center).max_w(460.0));
    if let Some(a) = action {
        c = c.child(row().pt(6.0).child(a));
    }
    c
}

// ---------------------------------------------------------------------------------------------
// Title bar (drawn by the app: no native one)
// ---------------------------------------------------------------------------------------------

/// The app icon as drawn in the UI: the white branch mark on the indigo-to-blue rounded tile,
/// the same as `assets/icon.png` (the window, taskbar and installers use that file).
pub fn app_mark(size: f32) -> El {
    let c = |hex: &str| rune::color::hex(hex).unwrap_or(Color::WHITE);
    row()
        .size(size, size)
        .radius(size * 0.225)
        .center()
        .fill(c("#634DFF"))
        .linear_gradient(std::f32::consts::FRAC_PI_4 * 3.0, [(0.0, c("#634DFF")), (1.0, c("#228BFF"))])
        .child(rune::icon(&I_LOGO, size * 0.64).color(Color::WHITE))
}

/// The custom title bar's height (the window's caption buttons take the same height).
pub const TITLEBAR_HEIGHT: f32 = 38.0;

/// The band across the top: the app's mark and name, and a command bar in the middle that
/// opens the palette. The whole band moves the window (double-click maximizes, right-click
/// opens the window menu); it stays clear of the window's own buttons (`insets`).
/// `ready`: the app is past setup (the command bar and app buttons have something to act on).
fn titlebar(k: &Kit, insets: rune::ChromeInsets, ready: bool) -> El {
    let brand = row()
        .gap(8.0)
        .center_y()
        .child(app_mark(22.0))
        .child(text(crate::channel::name()).font(k.ty.ui_medium).color(k.c.text_base));
    let command = row()
        .key("command-bar")
        .w_full()
        .max_w(520.0)
        .h(26.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .fill(k.c.surface_base)
        .border(1.0, k.c.border_weak)
        .hover(k.c.hover)
        .role("button")
        .label(crate::i18n::t("Command palette"))
        .help(crate::i18n::t("Jump to any repo, #PR, branch or group, or run a command"))
        .on_press(|s: &mut State| s.open_palette())
        .child(rune::icon(k.glyph.search, 13.0).color(k.c.icon_faint))
        .child(text("Search or jump to a repo, PR, branch or command…").font(k.ty.small).color(k.c.text_muted).truncate().shrink())
        .child(spacer())
        .child(text("Ctrl+K").font(k.ty.text_xsmall_medium).color(k.c.text_faint));
    let side = || row().w(0.0).grow(1.0).center_y();
    row()
        .key("titlebar")
        .w_full()
        .h(insets.height.max(TITLEBAR_HEIGHT))
        .pl(insets.leading + 12.0)
        .pr(insets.trailing + 12.0)
        .gap(12.0)
        .center_y()
        .window_drag()
        .child(side().child(brand))
        .child(row().w(0.0).grow(1.4).min_w(200.0).center().child(if ready { command } else { row() }))
        .child(side().child(spacer()).child(if ready { app_buttons(k) } else { row() }))
}

/// App-wide buttons at the title bar's end, before the window's own: the guide and Settings.
fn app_buttons(k: &Kit) -> El {
    row()
        .gap(2.0)
        .center_y()
        .child(
            IconButton::new(&I_HELP)
                .variant(IconButtonVariant::GhostMuted)
                .label(crate::i18n::t("Guide"))
                .help(crate::i18n::t("Guide: what every feature does"))
                .shortcut("F1")
                .on_press(|s: &mut State| s.guide_open = true)
                .build(k)
                .key("guide-btn"),
        )
        .child(
            IconButton::new(&I_GEAR)
                .variant(IconButtonVariant::GhostMuted)
                .label(crate::i18n::t("Settings"))
                .help(crate::i18n::t("Settings: appearance, accounts, git identity, safety, notifications"))
                .shortcut("Ctrl+,")
                .on_press(|s: &mut State| s.open_settings(s.settings_sec))
                .build(k)
                .key("settings-btn"),
        )
}
