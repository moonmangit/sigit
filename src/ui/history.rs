//! History editing (`history.rs`): the Commits pane's Edit history section, the preview in the
//! guard's dialog, and the Reword… dialog. Every button simulates first; nothing changes until
//! the dialog's confirm, and the preflight runs again then.
//!
//! The preview is a graphic first: a small commit graph (what leaves, what's new, what's made
//! again), a bar per changed file, and risk chips. The long explanations open on a click.

use rune::{El, Icon, LineCap, Shape, canvas, col, el, row, spacer};
use rune_kit::{Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, TextArea, glyphs};

use super::{diffstat, muted};
use crate::guard::Pending;
use crate::history::{self, Kind, Op, Plan, ResetMode};
use crate::i18n::text;
use crate::model::Commit;
use crate::risk::{Level, Tag};
use crate::{CommitGraph, State};

/// What's open in the history UI (all closed when a new simulation opens the dialog).
#[derive(Clone, Copy, Debug, Default)]
pub struct Open {
    /// The risk's full reasons.
    pub why: bool,
    /// Every changed file, not only the first few.
    pub files: bool,
    /// Every graph row.
    pub rows: bool,
    /// The Edit history section's explainer.
    pub help: bool,
    /// The preview's "about this action" box.
    pub about: bool,
    /// In the guide: the "how it works" notes.
    pub how: bool,
    /// In the guide: the one action opened (its picture and text).
    pub act: Option<Act>,
    /// The preview's animation, when it's open.
    pub anim: Option<Anim>,
}

/// The preview's animation: before → what changes → after, a step every `STEP_SECS`. Each
/// step only sets where every commit sits and how it looks; Rune springs between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anim {
    pub step: u8,
    pub playing: bool,
    /// The clock (seconds) when this step began.
    pub at: f64,
}

impl Anim {
    pub const STEPS: u8 = 3;
    pub const STEP_SECS: f64 = 1.4;

    pub fn start(clock: f64) -> Anim {
        Anim { step: 0, playing: true, at: clock }
    }

    /// The next step when its time came; it stops at the last one.
    pub fn advance(self, clock: f64) -> Anim {
        if !self.playing || clock - self.at < Anim::STEP_SECS {
            return self;
        }
        let step = (self.step + 1).min(Anim::STEPS - 1);
        Anim { step, playing: step + 1 < Anim::STEPS, at: clock }
    }

    /// Seconds until the next step, while playing.
    pub fn wait(self, clock: f64) -> Option<f64> {
        self.playing.then(|| (Anim::STEP_SECS - (clock - self.at)).max(0.0))
    }
}

/// Each history action, for its explanation (what it does, and when people use it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Soft,
    Mixed,
    Hard,
    Revert,
    Pick,
    Reword,
    Drop,
    Squash,
    Fixup,
    Move,
    Amend,
}

impl Act {
    pub const ALL: [Act; 11] = [Act::Soft, Act::Mixed, Act::Hard, Act::Revert, Act::Pick, Act::Reword, Act::Drop, Act::Squash, Act::Fixup, Act::Move, Act::Amend];

    pub fn of(op: &Op) -> Act {
        match op {
            Op::Reset { mode: ResetMode::Soft, .. } => Act::Soft,
            Op::Reset { mode: ResetMode::Mixed, .. } => Act::Mixed,
            Op::Reset { mode: ResetMode::Hard, .. } => Act::Hard,
            Op::Revert { .. } => Act::Revert,
            Op::CherryPick { .. } => Act::Pick,
            Op::Reword { .. } => Act::Reword,
            Op::Drop { .. } => Act::Drop,
            Op::Squash { fixup: false, .. } => Act::Squash,
            Op::Squash { fixup: true, .. } => Act::Fixup,
            Op::Move { .. } => Act::Move,
            Op::Amend => Act::Amend,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Act::Soft => "Reset (soft)",
            Act::Mixed => "Reset (mixed)",
            Act::Hard => "Reset (hard)",
            Act::Revert => "Revert",
            Act::Pick => "Cherry-pick",
            Act::Reword => "Reword",
            Act::Drop => "Drop",
            Act::Squash => "Squash",
            Act::Fixup => "Fixup",
            Act::Move => "Move up / down",
            Act::Amend => "Amend",
        }
    }

    pub fn icon(self) -> &'static Icon {
        match self {
            Act::Soft | Act::Mixed | Act::Hard => &I_RESET,
            Act::Revert => &I_UNDO,
            Act::Pick => &I_ADD,
            Act::Reword | Act::Amend => &I_EDIT,
            Act::Drop => &I_DROP,
            Act::Squash | Act::Fixup => &I_SQUASH,
            Act::Move => &I_MOVE,
        }
    }

    /// What it does.
    pub fn what(self) -> &'static str {
        match self {
            Act::Soft => "Moves the branch to this commit. The changes of the commits after it stay in your files, staged.",
            Act::Mixed => "Moves the branch to this commit. The changes of the commits after it stay in your files, not staged.",
            Act::Hard => "Moves the branch to this commit and makes your files match it. The commits after it leave the branch. sigit stashes your uncommitted changes first.",
            Act::Revert => "Adds a new commit that undoes the changes of this commit. The history before it does not change.",
            Act::Pick => "Copies this commit from another branch onto your branch, as a new commit on top.",
            Act::Reword => "Changes the message of this commit. The files do not change. This commit and the commits after it get new SHAs.",
            Act::Drop => "Removes this commit from the branch. The commits after it are made again without its changes.",
            Act::Squash => "Joins this commit into the commit before it. The new commit keeps both messages.",
            Act::Fixup => "Joins this commit into the commit before it. The new commit keeps only the message of the commit before it.",
            Act::Move => "Swaps this commit with the newer commit after it (up) or the older commit before it (down).",
            Act::Amend => "Puts your staged changes into the last commit. The message stays the same.",
        }
    }

    /// When people commonly use it.
    pub fn when(self) -> &'static str {
        match self {
            Act::Soft => "To join the last few commits into one: reset soft, then make one new commit.",
            Act::Mixed => "To undo the last commits but keep the work, and then commit it again in a different way.",
            Act::Hard => "To throw away local experiments and go back to a known good commit.",
            Act::Revert => "To undo a change that is already pushed or shared. It is the safe undo for a shared branch.",
            Act::Pick => "To bring one fix from another branch, for example a hotfix, without merging the whole branch.",
            Act::Reword => "To fix a typo, add a ticket number, or follow the message rules of the team before you push.",
            Act::Drop => "To remove a debug commit or a change you do not want, before you push.",
            Act::Squash => "To clean up many small \"work in progress\" commits into one before a review.",
            Act::Fixup => "To put a small correction (a typo, a review change) into the commit it belongs to.",
            Act::Move => "To put related commits next to each other before a squash, or to make the history easy to read.",
            Act::Amend => "To add a file you forgot, or to fix a small mistake in the commit you just made.",
        }
    }

    /// Both, for a button's hover help.
    pub fn help(self) -> String {
        tf!("{} Common use: {}", crate::i18n::t(self.what()), crate::i18n::t(self.when()))
    }
}

/// How a commit dot looks in a before / after diagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dot {
    /// Not touched.
    Kept,
    /// The commit you picked.
    Picked,
    /// New on the branch.
    New,
    /// Made again with a new SHA.
    Again,
    /// Leaves the branch.
    Gone,
}

/// A diagram: the commits before and after (oldest first, letters as names), and what happens
/// to the changes of the commits that leave (resets).
struct Picture {
    before: &'static [(&'static str, Dot)],
    after: &'static [(&'static str, Dot)],
    note: Option<&'static str>,
}

impl Act {
    /// The before / after picture. B is the commit you picked (C for squash and fixup).
    fn picture(self) -> Picture {
        use Dot::*;
        const ABCD: &[(&str, Dot)] = &[("A", Kept), ("B", Picked), ("C", Kept), ("D", Kept)];
        match self {
            Act::Soft => Picture { before: ABCD, after: &[("A", Kept), ("B", Picked), ("C", Gone), ("D", Gone)], note: Some("C + D changes: staged") },
            Act::Mixed => Picture { before: ABCD, after: &[("A", Kept), ("B", Picked), ("C", Gone), ("D", Gone)], note: Some("C + D changes: in files") },
            Act::Hard => Picture { before: ABCD, after: &[("A", Kept), ("B", Picked), ("C", Gone), ("D", Gone)], note: Some("C + D changes: gone") },
            Act::Revert => Picture { before: ABCD, after: &[("A", Kept), ("B", Picked), ("C", Kept), ("D", Kept), ("−B", New)], note: None },
            Act::Pick => Picture { before: &[("A", Kept), ("C", Kept), ("D", Kept), ("B", Picked)], after: &[("A", Kept), ("C", Kept), ("D", Kept), ("B′", New)], note: Some("B is on another branch") },
            Act::Reword => Picture { before: ABCD, after: &[("A", Kept), ("B′", Again), ("C′", Again), ("D′", Again)], note: Some("B′: new message") },
            Act::Drop => Picture { before: ABCD, after: &[("A", Kept), ("B", Gone), ("C′", Again), ("D′", Again)], note: None },
            Act::Squash => Picture { before: &[("A", Kept), ("B", Kept), ("C", Picked), ("D", Kept)], after: &[("A", Kept), ("BC", Again), ("D′", Again)], note: Some("BC: both messages") },
            Act::Fixup => Picture { before: &[("A", Kept), ("B", Kept), ("C", Picked), ("D", Kept)], after: &[("A", Kept), ("BC", Again), ("D′", Again)], note: Some("BC: B's message") },
            Act::Move => Picture { before: ABCD, after: &[("A", Kept), ("C′", Again), ("B′", Again), ("D′", Again)], note: Some("B moved up") },
            Act::Amend => Picture { before: &[("A", Kept), ("B", Kept), ("C", Kept), ("D", Picked)], after: &[("A", Kept), ("B", Kept), ("C", Kept), ("D′", Again)], note: Some("D′ = D + staged") },
        }
    }
}

/// One dot with its letter.
fn dot(k: &Kit, label: &str, d: Dot) -> El {
    let (fill, border, fg) = match d {
        Dot::Kept => (k.c.surface_base, k.c.border_strong, k.c.text_muted),
        Dot::Picked => (k.c.surface_base, k.c.text_strong, k.c.text_strong),
        Dot::New => (k.c.success.bg, k.c.success.icon, k.c.success.fg),
        Dot::Again => (k.c.warning.bg, k.c.warning.icon, k.c.warning.fg),
        Dot::Gone => (k.c.bg_base, k.c.danger.icon, k.c.danger.fg),
    };
    row()
        .size(26.0, 22.0)
        .radius(11.0)
        .center()
        .fill(fill)
        .border(if d == Dot::Picked { 2.0 } else { 1.2 }, border)
        .child(text(label.to_owned()).font(k.ty.text_xsmall_medium).color(fg))
}

/// A chain of dots, oldest left. Commits that leave sit after a gap, crossed out by colour.
fn chain(k: &Kit, title: &str, nodes: &[(&str, Dot)]) -> El {
    let mut r = row().gap(0.0).center_y().child(text(crate::i18n::t(title)).font(k.ty.small).color(k.c.text_faint).w(50.0));
    let mut first = true;
    let link = || el().size(10.0, 1.5).fill(k.c.border_strong);
    for (label, d) in nodes.iter().filter(|(_, d)| *d != Dot::Gone) {
        if !first {
            r = r.child(link());
        }
        r = r.child(dot(k, label, *d));
        first = false;
    }
    let gone: Vec<_> = nodes.iter().filter(|(_, d)| *d == Dot::Gone).collect();
    if !gone.is_empty() {
        r = r.child(el().w(12.0)).child(text("✕").font(k.ty.small).color(k.c.danger.fg)).child(el().w(4.0));
        for (i, (label, d)) in gone.iter().enumerate() {
            if i > 0 {
                r = r.child(el().w(3.0));
            }
            r = r.child(dot(k, label, *d));
        }
    }
    r
}

/// The before / after diagram of an action, with its note.
fn picture(k: &Kit, key: &str, act: Act) -> El {
    let p = act.picture();
    let mut c = col().key(key.to_owned()).gap(5.0).padding(8.0).radius(k.radius.md).border(1.0, k.c.border_weak).child(chain(k, "Before", p.before)).child(chain(k, "After", p.after));
    if let Some(n) = p.note {
        c = c.child(text(crate::i18n::t(n)).font(k.ty.small).color(k.c.text_muted).pl(50.0));
    }
    c
}

/// The colours of the diagrams, once.
fn legend(k: &Kit) -> El {
    [(Dot::Picked, "your commit"), (Dot::New, "new"), (Dot::Again, "made again"), (Dot::Gone, "leaves")]
        .into_iter()
        .fold(row().gap(10.0).center_y().flex_wrap(), |r, (d, t)| r.child(row().gap(4.0).center_y().child(dot(k, "", d).size(14.0, 12.0)).child(text(crate::i18n::t(t)).font(k.ty.small).color(k.c.text_muted))))
}

/// A section's header: icon, title, what's on the right, then a thin line.
pub fn section(k: &Kit, key: &str, icon: &'static Icon, title: &str, trailing: Option<El>) -> El {
    let mut r = row().key(key.to_owned()).w_full().gap(6.0).center_y().child(rune::icon(icon, 13.0).color(k.c.icon_faint)).child(text(crate::i18n::t(title)).font(k.ty.small_medium).color(k.c.text_muted));
    if let Some(t) = trailing {
        r = r.child(t);
    }
    r.child(el().h(1.0).grow(1.0).fill(k.c.border_weak))
}

pub static I_MESSAGE: Icon = Icon { name: "h-message", view: 16.0, body: r#"<path d="M2.5 3.5H13.5V10.5H7L4 13V10.5H2.5Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"# };
pub static I_CHANGES: Icon = Icon { name: "h-changes", view: 16.0, body: r#"<path d="M4 2.5H9.5L12 5V13.5H4Z M6 8H10 M8 6V10" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round" fill="none"/>"# };
static I_COMMITS: Icon = Icon { name: "h-commits", view: 16.0, body: r#"<circle cx="8" cy="4" r="1.8" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="8" cy="12" r="1.8" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 5.8V10.2" stroke="currentColor" stroke-width="1.2"/>"# };

/// A folding row: triangle, optional icon, label. `on` is open.
fn fold_row(k: &Kit, key: &str, on: bool, icon: Option<&'static Icon>, label: &str, press: impl Fn(&mut State) + Send + Sync + 'static) -> El {
    let mut r = row()
        .key(key.to_owned())
        .w_full()
        .gap(6.0)
        .py(3.0)
        .center_y()
        .child(rune::icon(if on { &glyphs::TRIANGLE_DOWN } else { &glyphs::TRIANGLE_RIGHT }, 9.0).color(k.c.icon_faint));
    if let Some(i) = icon {
        r = r.child(rune::icon(i, 13.0).color(if on { k.c.icon_base } else { k.c.icon_faint }));
    }
    r.child(text(crate::i18n::t(label)).font(if on { k.ty.small_medium } else { k.ty.small }).color(if on { k.c.text_base } else { k.c.text_muted }))
        .on_press(move |s: &mut State| press(s))
}

/// The ⓘ guide: action names only; one opens at a time to its picture and text. The "how it
/// works" notes fold too.
fn guide(s: &State, k: &Kit) -> El {
    let o = s.history_open;
    let mut c = col().key("h-help-box").w_full().gap(2.0);
    for (i, a) in Act::ALL.iter().copied().enumerate() {
        let on = o.act == Some(a);
        c = c.child(fold_row(k, &tf!("h-guide-row-{i}"), on, Some(a.icon()), a.name(), move |s| s.history_open.act = if s.history_open.act == Some(a) { None } else { Some(a) }));
        if on {
            c = c.child(col().w_full().gap(8.0).pl(15.0).pb(6.0).child(legend(k)).child(about_block(k, &tf!("h-guide-{i}"), a, false)));
        }
    }
    c = c.child(el().w_full().h(1.0).my(4.0).fill(k.c.border_weak)).child(fold_row(k, "h-how", o.how, None, "How it works", |s| s.history_open.how ^= true));
    if o.how {
        let line = |icon: &'static Icon, t: &str| row().gap(8.0).items_start().child(rune::icon(icon, 13.0).color(k.c.icon_faint)).child(text(crate::i18n::t(t)).font(k.ty.small).color(k.c.text_muted).wrap().shrink());
        c = c.child(
            col()
                .w_full()
                .gap(6.0)
                .pl(15.0)
                .pt(4.0)
                .child(line(&I_STAR, "Each button simulates first: you see the new history before anything changes."))
                .child(line(&I_SHIELD, "Before it runs, a preflight checks that the branch, its upstream and the remote didn't change."))
                .child(line(&I_UNDO, "Undo (10 s) puts the branch back. sigit never pushes."))
                .child(line(&I_FILE, "Your uncommitted changes stay; a hard reset stashes them first.")),
        );
    }
    c
}

/// An action's name, what it does and its common use, as a block.
/// `named`: with its icon and name on top (the guide's row already shows them).
fn about_block(k: &Kit, key: &str, act: Act, named: bool) -> El {
    let mut c = col().key(key.to_owned()).w_full().gap(3.0);
    if named {
        c = c.child(row().gap(6.0).center_y().child(rune::icon(act.icon(), 13.0).color(k.c.icon_base)).child(text(crate::i18n::t(act.name())).font(k.ty.small_medium).color(k.c.text_base)));
    }
    c.child(row().w_full().pl(19.0).child(picture(k, &format!("{key}-pic"), act)))
        .child(text(crate::i18n::t(act.what())).font(k.ty.small).color(k.c.text_base).wrap().pl(19.0))
        .child(
            row()
                .w_full()
                .gap(5.0)
                .items_start()
                .pl(19.0)
                .child(text(crate::i18n::t("Common use:")).font(k.ty.small_medium).color(k.c.text_muted))
                .child(text(crate::i18n::t(act.when())).font(k.ty.small).color(k.c.text_muted).wrap().shrink()),
        )
}

macro_rules! glyph {
    ($id:ident, $name:literal, $body:literal) => {
        static $id: Icon = Icon { name: $name, view: 16.0, body: $body };
    };
}
glyph!(I_RESET, "h-reset", r#"<path d="M3.5 6.5H10A3 3 0 0 1 10 12.5H6 M6 3.5L3 6.5L6 9.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
glyph!(I_ADD, "h-add", r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.3" fill="none"/><path d="M8 5.5V10.5M5.5 8H10.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#);
glyph!(I_EDIT, "h-edit", r#"<path d="M10.5 3L13 5.5L6 12.5H3.5V10Z M9 4.5L11.5 7" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" fill="none"/>"#);
glyph!(I_DROP, "h-drop", r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.3" fill="none"/><path d="M5.5 8H10.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#);
glyph!(I_SQUASH, "h-squash", r#"<path d="M4 3V6.5L8 9.5L12 6.5V3 M8 9.5V13.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
glyph!(I_SHIELD, "h-shield", r#"<path d="M8 2L13 4V8C13 11 10.8 13.2 8 14C5.2 13.2 3 11 3 8V4Z M5.8 8L7.3 9.5L10.3 6.5" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" stroke-linecap="round" fill="none"/>"#);
glyph!(I_UNDO, "h-undo", r#"<path d="M3 7H10A3 3 0 0 1 10 13H7 M5.5 4.5L3 7L5.5 9.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/><circle cx="12.5" cy="3.5" r="1.2" fill="currentColor"/>"#);
glyph!(I_PUSHED, "h-pushed", r#"<path d="M4.5 12.5H11.5C13 12.5 14 11.4 14 10C14 8.7 13 7.6 11.7 7.5C11.4 5.5 9.9 4 8 4C6.3 4 4.9 5.2 4.5 6.8C3.1 7 2 8.2 2 9.7C2 11.2 3.1 12.5 4.5 12.5Z M8 11V7.5 M6.5 9L8 7.5L9.5 9" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round" fill="none"/>"#);
glyph!(I_STAR, "h-star", r#"<path d="M8 2.5L9.6 6L13.3 6.3L10.5 8.8L11.3 12.5L8 10.6L4.7 12.5L5.5 8.8L2.7 6.3L6.4 6Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
glyph!(I_FILE, "h-file", r#"<path d="M4 2.5H9.5L12 5V13.5H4Z M9.5 2.5V5H12" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
glyph!(I_WARN, "h-warn", r#"<path d="M8 2.5L14 13H2Z M8 6.5V9.5" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" stroke-linecap="round" fill="none"/><circle cx="8" cy="11.3" r="0.8" fill="currentColor"/>"#);
glyph!(I_MOVE, "h-move", r#"<path d="M5 13V3 M2.5 5.5L5 3L7.5 5.5 M11 3V13 M8.5 10.5L11 13L13.5 10.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
glyph!(I_PLAY, "h-play", r#"<path d="M5 3.5L12.5 8L5 12.5Z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" fill="none"/>"#);
glyph!(I_INFO, "h-info", r#"<circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 7.2V11" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="8" cy="5" r="0.8" fill="currentColor"/>"#);

/// Whether `sha` is on the current branch, from the loaded graph: walk the parents from the
/// commit HEAD points at. (Commits past the loaded page count as not on it.)
pub fn on_head(g: &CommitGraph, sha: &str) -> bool {
    let Some(head) = head_sha(g) else { return false };
    let by_sha: std::collections::HashMap<&str, &Commit> = g.commits.iter().map(|c| (c.sha.as_str(), c)).collect();
    let mut todo = vec![head];
    let mut seen = std::collections::HashSet::new();
    while let Some(c) = todo.pop() {
        if c == sha {
            return true;
        }
        if seen.insert(c)
            && let Some(commit) = by_sha.get(c)
        {
            todo.extend(commit.parents.iter().map(String::as_str));
        }
    }
    false
}

/// The HEAD commit's SHA in the loaded graph.
fn head_sha(g: &CommitGraph) -> Option<&str> {
    g.commits.iter().find(|c| c.refs.iter().any(|r| r.starts_with("HEAD -> "))).map(|c| c.sha.as_str())
}

fn any_busy(s: &State) -> bool {
    s.busy.iter().any(|b| b.starts_with("Simulating: "))
}

/// One button that simulates `op`. On hover: what it does and its common use.
fn op_button(s: &State, k: &Kit, key: &str, icon: Option<&'static Icon>, label: &str, op: Op) -> El {
    let busy = s.busy.contains(&history::sim_label(&op));
    let mut b = Button::new(crate::i18n::t(if busy { "Simulating…" } else { label }))
        .variant(ButtonVariant::Outline)
        .disabled(any_busy(s) || s.pending.is_some())
        .help(Act::of(&op).help())
        .on_press(move |s: &mut State| s.simulate_history(op.clone()));
    if let Some(i) = icon {
        b = b.icon(i);
    }
    b.build(k).key(key.to_owned())
}

/// A group of buttons: a small label with its icon on top, the buttons under it.
fn group(k: &Kit, key: &str, icon: &'static Icon, title: &str, buttons: El) -> El {
    col()
        .key(key.to_owned())
        .w_full()
        .gap(6.0)
        .child(row().gap(5.0).center_y().child(rune::icon(icon, 12.0).color(k.c.icon_faint)).child(text(crate::i18n::t(title)).font(k.ty.small).color(k.c.text_faint)))
        .child(buttons.w_full())
}

/// The Edit history section of a commit's pane: three groups of labelled buttons.
pub fn actions(s: &State, k: &Kit, g: &CommitGraph, c: &Commit) -> Option<El> {
    let branch = s.rd().and_then(|d| d.status.data.as_ref()).map(|st| st.branch.clone()).filter(|b| !b.is_empty() && b != "(detached)")?;
    s.local_path()?;
    let sha = c.sha.clone();
    // From git when the commit's detail has loaded; the loaded graph until then (it can't see
    // past a date or contributor filter).
    let detail = s.commit_detail.data.as_ref();
    let here = detail.and_then(|d| d.on_head).unwrap_or_else(|| on_head(g, &sha));
    let is_head = detail.map_or_else(|| head_sha(g) == Some(sha.as_str()), |d| d.is_head);
    let merge = c.parents.len() > 1;
    let root = c.parents.is_empty();
    let card = col().key("h-actions").w_full().gap(10.0).padding(12.0).radius(k.radius.lg).border(1.0, k.c.border_weak).fill(k.c.layer_01);

    // Header: what this is, on which branch, and the guide.
    let open = s.history_open.help;
    let mut out = card.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(rune::icon(&I_EDIT, 14.0).color(k.c.icon_base))
            .child(text(crate::i18n::t("Edit history")).font(k.ty.ui_medium).color(k.c.text_strong))
            .child(text(branch.clone()).font(k.ty.mono).color(k.c.text_muted).truncate().shrink().min_w(0.0).help_when_truncated(branch.clone()))
            .child(spacer())
            .child(
                IconButton::new(&I_INFO)
                    .variant(IconButtonVariant::GhostMuted)
                    .label(crate::i18n::t("What each action does"))
                    .help(crate::i18n::t(if open { "Hide" } else { "What each action does, and when to use it" }))
                    .on_press(|s: &mut State| s.history_open.help ^= true)
                    .build(k)
                    .key("h-help"),
            ),
    );
    out = out.child(el().w_full().h(1.0).fill(k.c.border_weak));

    // The buttons, grouped by what you want to do with this commit.
    let buttons = || row().gap(6.0).flex_wrap();
    if !is_head {
        let modes = [(ResetMode::Soft, "Soft"), (ResetMode::Mixed, "Mixed"), (ResetMode::Hard, "Hard")];
        let b = modes.into_iter().fold(buttons(), |r, (mode, label)| r.child(op_button(s, k, &tf!("h-reset-{}", mode.word()), None, label, Op::Reset { to: sha.clone(), mode })));
        out = out.child(group(k, "h-g-reset", &I_RESET, "Go back to here", b));
    }
    if here && !root {
        let mut b = buttons().child(op_button(s, k, "h-revert", Some(&I_UNDO), "Revert", Op::Revert { sha: sha.clone() }));
        if !merge {
            b = b.child(op_button(s, k, "h-drop", Some(&I_DROP), "Drop", Op::Drop { sha: sha.clone() }));
        }
        out = out.child(group(k, "h-g-undo", &I_UNDO, "Undo this commit", b));
    }
    if here && !merge && !root {
        // The full message comes with the commit's detail: wait for it, so the body is kept.
        let message = s.commit_detail.data.as_ref().map(|d| d.body.clone());
        let loaded = message.is_some();
        let message = message.unwrap_or_default();
        let id = sha.clone();
        let mut b = buttons().child(
            Button::new(crate::i18n::t("Reword…"))
                .variant(ButtonVariant::Outline)
                .icon(&I_EDIT)
                .disabled(s.pending.is_some() || !loaded)
                .help(Act::Reword.help())
                .on_press(move |s: &mut State| s.start_reword(id.clone(), message.clone()))
                .build(k)
                .key("h-reword"),
        );
        b = b
            .child(op_button(s, k, "h-squash", Some(&I_SQUASH), "Squash", Op::Squash { sha: sha.clone(), fixup: false }))
            .child(op_button(s, k, "h-fixup", Some(&I_SQUASH), "Fixup", Op::Squash { sha: sha.clone(), fixup: true }));
        if !is_head {
            b = b.child(op_button(s, k, "h-up", Some(&glyphs::CHEVRON_UP), "Move up", Op::Move { sha: sha.clone(), up: true }));
        }
        b = b.child(op_button(s, k, "h-down", Some(&glyphs::CHEVRON_DOWN), "Move down", Op::Move { sha: sha.clone(), up: false }));
        out = out.child(group(k, "h-g-change", &I_EDIT, "Change this commit", b));
    }
    if !here && !merge && !root {
        let b = buttons().child(op_button(s, k, "h-pick", Some(&I_ADD), "Cherry-pick", Op::CherryPick { sha: sha.clone() }));
        out = out.child(group(k, "h-g-pick", &I_ADD, &tf!("Copy to {branch}"), b));
    }
    if open {
        out = out.child(el().w_full().h(1.0).fill(k.c.border_weak)).child(guide(s, k));
    }
    Some(out)
}

/// Amend, for the Commits toolbar: put the staged changes into the last commit.
pub fn amend_button(s: &State, k: &Kit) -> Option<El> {
    s.local_path()?;
    Some(op_button(s, k, "h-amend", Some(&I_EDIT), "Amend last commit", Op::Amend))
}

// ---------------------------------------------------------------------------------------------
// The preview
// ---------------------------------------------------------------------------------------------

const NODE_W: f32 = 22.0;
const ROW_H: f32 = 26.0;

fn circle(cx: f32, cy: f32, r: f32) -> Vec<[f32; 2]> {
    (0..16)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 16.0;
            [cx + a.cos() * r, cy + a.sin() * r]
        })
        .collect()
}

/// The colour of a kind, and its mark.
fn tone(k: &Kit, kind: Kind) -> (rune::Color, &'static str) {
    match kind {
        Kind::Leaves => (k.c.danger.icon, "−"),
        Kind::New => (k.c.success.icon, "+"),
        Kind::Rewritten => (k.c.warning.icon, "~"),
        Kind::Joined => (k.c.warning.icon, "↳"),
    }
}

/// One node of the graph: the branch line through it, and a dot whose shape says the kind
/// (filled: on the new branch; hollow: leaves; small: joined into the row above).
fn node(k: &Kit, kind: Kind, first: bool, last: bool) -> El {
    let (c, _) = tone(k, kind);
    let x = NODE_W / 2.0;
    let mid = ROW_H / 2.0;
    let mut shape = Shape::new();
    let line = k.c.text_faint;
    if !first {
        shape = shape.line([x, 0.0], [x, mid], 1.5, LineCap::Butt, line);
    }
    if !last {
        shape = shape.line([x, mid], [x, ROW_H], 1.5, LineCap::Butt, line);
    }
    shape = match kind {
        Kind::Leaves => shape.fill(circle(x, mid, 5.0), c).fill(circle(x, mid, 3.2), k.c.bg_base),
        Kind::Joined => shape.fill(circle(x, mid, 3.4), c),
        _ => shape.fill(circle(x, mid, 5.0), c),
    };
    canvas(shape.view_box(NODE_W, ROW_H)).size(NODE_W, ROW_H)
}

fn sha_text(k: &Kit, sha: &str, color: rune::Color) -> El {
    text(sha.to_owned()).font(k.ty.mono).color(color)
}

/// One graph row: node, mark, SHA(s), subject. A rewritten row reads `old → new`.
fn graph_row(k: &Kit, i: usize, r: &history::Row, first: bool, last: bool) -> El {
    let (c, mark) = tone(k, r.kind);
    let mut line = row().key(tf!("h-row-{i}")).w_full().h(ROW_H).gap(6.0).center_y().child(node(k, r.kind, first, last)).child(text(mark).font(k.ty.mono).color(c).w(10.0));
    let (old, new) = (r.old.as_ref(), r.new.as_ref());
    match r.kind {
        Kind::Leaves => {
            let o = old.expect("a leaving row has its old commit");
            line = line.child(sha_text(k, &o.sha, k.c.danger.fg)).child(text(o.subject.clone()).font(k.ty.small).color(k.c.text_faint).truncate().shrink().min_w(0.0));
        }
        Kind::New => {
            let n = new.expect("a new row has its new commit");
            line = line.child(sha_text(k, &n.sha, k.c.success.fg)).child(text(n.subject.clone()).font(k.ty.small).color(k.c.text_base).truncate().shrink().min_w(0.0));
        }
        Kind::Rewritten | Kind::Joined => {
            let (o, n) = (old.expect("has old"), new.expect("has new"));
            let joined = r.kind == Kind::Joined;
            line = line
                .child(sha_text(k, &o.sha, k.c.text_faint))
                .child(text("→").font(k.ty.small).color(k.c.text_faint))
                .child(sha_text(k, &n.sha, if joined { k.c.text_faint } else { k.c.warning.fg }));
            let subject = if joined { o.subject.clone() } else { n.subject.clone() };
            let changed = !joined && o.subject != n.subject;
            let mut t = text(subject).font(k.ty.small).color(if joined { k.c.text_faint } else { k.c.text_base }).truncate().shrink().min_w(0.0);
            if changed {
                t = t.help(crate::i18n::t(tf!("Was: {}", o.subject)));
            }
            line = line.child(t);
        }
    }
    line
}

/// A count pill: `−2`, `+1`, `~3`.
fn count_pill(k: &Kit, kind: Kind, n: usize, help: &str) -> El {
    let (c, mark) = tone(k, kind);
    row()
        .gap(4.0)
        .center_y()
        .px(7.0)
        .py(2.0)
        .radius(k.radius.sm)
        .fill(k.c.surface_base)
        .child(el().size(7.0, 7.0).radius(4.0).fill(c))
        .child(text(format!("{mark}{n}")).font(k.ty.text_xsmall_medium).color(k.c.text_base))
        .help(crate::i18n::t(help))
}

/// The open / closed toggle under a list: `▸ +5 more` / `▾ fewer`.
fn more_toggle(k: &Kit, key: &str, open: bool, hidden: usize, flip: fn(&mut State)) -> El {
    let label = if open { crate::i18n::t("Show fewer") } else { tf!("+{hidden} more") };
    row()
        .key(key.to_owned())
        .gap(5.0)
        .center_y()
        .child(rune::icon(if open { &glyphs::TRIANGLE_DOWN } else { &glyphs::TRIANGLE_RIGHT }, 9.0).color(k.c.icon_faint))
        .child(text(label).font(k.ty.small).color(k.c.text_muted))
        .on_press(move |s: &mut State| flip(s))
}

/// The simulated outcome, at the top of the guard's dialog: counts, the graph, the files.
pub fn preview(s: &State, k: &Kit, plan: &Plan) -> El {
    let short = |x: &str| x.get(..7).unwrap_or(x).to_owned();
    let open = s.history_open;
    let count = |kind: Kind| plan.rows.iter().filter(|r| r.kind == kind).count();
    let mut c = col().key("h-preview").w_full().gap(10.0).padding(12.0).radius(k.radius.md).fill(k.c.bg_base).border(1.0, k.c.border_weak);

    // Header: the tip moving, and the counts as pills.
    let mut head = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(rune::icon(&I_STAR, 14.0).color(k.c.icon_base))
        .child(text(crate::i18n::t("Simulated")).font(k.ty.ui_medium).color(k.c.text_strong).help(crate::i18n::t("Nothing has changed yet: these commits exist only as objects until you confirm.")))
        .child(sha_text(k, &short(&plan.snap.tip), k.c.text_muted))
        .child(text("→").font(k.ty.small).color(k.c.text_faint))
        .child(sha_text(k, &short(&plan.new_tip), k.c.text_base))
        .child(
            IconButton::new(&I_INFO)
                .variant(IconButtonVariant::GhostMuted)
                .label(crate::i18n::t(tf!("About {}", crate::i18n::t(Act::of(&plan.op).name()))))
                .help(crate::i18n::t(if open.about { "Hide" } else { "What this action does, and when to use it" }))
                .on_press(|s: &mut State| s.history_open.about ^= true)
                .build(k)
                .key("h-about"),
        )
        .child(spacer());
    for (kind, help) in [(Kind::Leaves, "Commits that leave the branch"), (Kind::New, "New commits"), (Kind::Rewritten, "Commits made again with new SHAs"), (Kind::Joined, "Commits joined into the one above")] {
        let n = count(kind);
        if n > 0 {
            head = head.child(count_pill(k, kind, n, help));
        }
    }
    c = c.child(head);
    if open.about {
        c = c.child(col().w_full().gap(8.0).padding(10.0).radius(k.radius.md).fill(k.c.surface_base).child(legend(k)).child(about_block(k, "h-about-box", Act::of(&plan.op), true)));
    }

    // The graph, newest first.
    // Play: the edit as an animation on the real commits, with the neighbours under it.
    let play = Button::new(crate::i18n::t(if open.anim.is_some() { "Replay" } else { "Play" }))
        .variant(ButtonVariant::GhostMuted)
        .icon(&I_PLAY)
        .help(crate::i18n::t("Watch the edit happen on your real commits, step by step"))
        .on_press_tx(|s: &mut State, tx| {
            s.play_history_anim();
            // The first step's time: don't wait for the next idle poll.
            tx.wake_after(std::time::Duration::from_secs_f64(Anim::STEP_SECS));
        })
        .build(k)
        .key("h-play");
    c = c.child(section(k, "h-sec-commits", &I_COMMITS, "Commits", None).child(play));
    if let Some(a) = open.anim {
        c = c.child(animation(k, plan, a));
    } else {
        const ROWS: usize = 6;
        let shown = if open.rows { plan.rows.len() } else { plan.rows.len().min(ROWS) };
        let mut graph = col().key("h-graph").w_full();
        for (i, r) in plan.rows.iter().take(shown).enumerate() {
            graph = graph.child(graph_row(k, i, r, i == 0, i + 1 == shown && shown == plan.rows.len()));
        }
        c = c.child(graph);
        if plan.rows.len() > ROWS {
            c = c.child(more_toggle(k, "h-rows-more", open.rows, plan.rows.len() - ROWS, |s| s.history_open.rows ^= true));
        }
    }

    // Files: a bar each; the count and total in the section's header.
    const FILES: usize = 4;
    if plan.files.is_empty() {
        c = c.child(section(k, "h-sec-files", &I_FILE, "Files", None)).child(muted(k, "Same files: only the history changes"));
    } else {
        let (add, del) = plan.files.iter().fold((0, 0), |(a, d), f| (a + f.add, d + f.del));
        let total = row().gap(6.0).center_y().child(text(tf!("{} file(s)", plan.files.len())).font(k.ty.small).color(k.c.text_faint)).child(diffstat(k, add, del));
        c = c.child(section(k, "h-sec-files", &I_FILE, "Files", Some(total)));
        let n = if open.files { plan.files.len() } else { plan.files.len().min(FILES) };
        let mut files = col().key("h-files").w_full().gap(3.0);
        for (i, f) in plan.files.iter().take(n).enumerate() {
            files = files.child(
                row()
                    .key(tf!("h-file-{i}"))
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(text(f.path.clone()).font(k.ty.mono).color(k.c.text_muted).truncate().grow(1.0).shrink().min_w(0.0).help_when_truncated(f.path.clone()))
                    .child(diffstat(k, f.add, f.del)),
            );
        }
        c = c.child(files);
        if plan.files.len() > FILES {
            c = c.child(more_toggle(k, "h-files-more", open.files, plan.files.len() - FILES, |s| s.history_open.files ^= true));
        }
    }
    if !plan.replaced_files.is_empty() {
        let list = plan.replaced_files.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
        c = c.child(row().gap(6.0).center_y().child(rune::icon(&I_WARN, 13.0).color(k.c.danger.icon)).child(text(tf!("Replaced: {list}")).font(k.ty.small).color(k.c.danger.fg).truncate().shrink().min_w(0.0)));
    }
    c
}

/// The steps' names; `step_text` says what each shows for the action.
const STEP_TEXT: [&str; 3] = ["Before", "What changes", "After"];

/// How a commit glides to its next place or fades: slower than the UI's own springs, so the
/// eye can follow one commit; well under `Anim::STEP_SECS`, so each step settles.
const MOVE: rune::Spring = rune::Spring::new(0.7, 0.0);

/// The commit the action is about (ringed in the animation).
fn target_sha(plan: &Plan) -> Option<String> {
    match &plan.op {
        Op::Reset { to, .. } => Some(to.clone()),
        Op::Amend => Some(plan.snap.tip.clone()),
        Op::Revert { sha } | Op::CherryPick { sha } | Op::Reword { sha, .. } | Op::Drop { sha } | Op::Squash { sha, .. } | Op::Move { sha, .. } => Some(sha.clone()),
    }
    // The op holds the SHA as it was typed or picked: make it the full one the lines carry.
    .map(|t| plan.removed.iter().chain(&plan.context).chain(plan.source.iter()).find(|l| l.full.starts_with(&t) || l.sha == t).map_or(t, |l| l.full.clone()))
}

/// What a step shows, for this action, with its real names (the caption also carries the
/// meaning when motion is reduced).
fn step_text(plan: &Plan, step: u8) -> String {
    let b = &plan.snap.branch;
    let short = |x: &str| x.get(..7).unwrap_or(x).to_owned();
    let t = target_sha(plan).map(|x| short(&x)).unwrap_or_default();
    let new_tip = short(&plan.new_tip);
    if step == 0 {
        return tf!("Your branch now: the {b} label is at its tip. The ringed commit is the one you picked; the grey ones under the edit stay as they are.");
    }
    let changes = step == 1;
    match &plan.op {
        Op::Reset { mode, .. } if changes => tf!("{b} will point at {t}: the red commits above it leave {b}."),
        Op::Reset { mode: ResetMode::Soft, .. } => tf!("{b} now points at {t}. The changes of the commits that left stay in your files, staged."),
        Op::Reset { mode: ResetMode::Mixed, .. } => tf!("{b} now points at {t}. The changes of the commits that left stay in your files, not staged."),
        Op::Reset { mode: ResetMode::Hard, .. } => tf!("{b} now points at {t}, and your files match it. Your uncommitted work was stashed first."),
        Op::Revert { .. } if changes => tf!("{t} is the commit to undo. It stays in the history."),
        Op::Revert { .. } => tf!("A new commit {new_tip} on top undoes {t}. Nothing leaves {b}."),
        Op::CherryPick { .. } if changes => {
            let from = plan.source.as_ref().and_then(|s| s.kept_by.first().cloned()).unwrap_or_else(|| crate::i18n::t("another branch"));
            tf!("{t} on {from} is copied.")
        }
        Op::CherryPick { .. } => tf!("The copy {new_tip} is the new tip of {b}. The original stays where it was."),
        Op::Reword { .. } if changes => tf!("{t} gets a new message. The commits after it are made again (amber)."),
        Op::Reword { .. } => tf!("The same files with new SHAs from {t} up; {b} points at {new_tip}."),
        Op::Drop { .. } if changes => tf!("{t} leaves {b}. The commits after it are made again without its changes."),
        Op::Drop { .. } => tf!("{t} is gone from {b}, and the commits after it moved down into its place."),
        Op::Squash { fixup: false, .. } if changes => tf!("{t} joins the commit before it."),
        Op::Squash { fixup: true, .. } if changes => tf!("{t} joins the commit before it; only that commit's message stays."),
        Op::Squash { fixup: false, .. } => tf!("One commit now holds both changes and both messages."),
        Op::Squash { fixup: true, .. } => tf!("One commit now holds both changes, with the older message."),
        Op::Move { up, .. } if changes => if *up { tf!("{t} and the newer commit after it swap places.") } else { tf!("{t} and the older commit before it swap places.") },
        Op::Move { .. } => tf!("The same changes in a new order, with new SHAs; {b} points at {new_tip}."),
        Op::Amend if changes => tf!("Your staged changes go into {t}, the last commit."),
        Op::Amend => tf!("{t} is made again as {new_tip}, with your staged changes and the same message."),
    }
}

/// A small label pill (a branch, a remote branch, a tag).
fn ref_pill(k: &Kit, name: &str, strong: bool) -> El {
    row()
        .px(5.0)
        .h(16.0)
        .center_y()
        .radius(k.radius.sm)
        .fill(if strong { k.c.surface_base } else { k.c.bg_base })
        .border(1.0, if strong { k.c.border_strong } else { k.c.border_weak })
        .child(text(name.to_owned()).font(k.ty.text_xsmall_medium).color(if strong { k.c.text_base } else { k.c.text_muted }).truncate())
        .shrink()
        .min_w(0.0)
}

/// The stage's layout: the branch label's column, then one lane (the branch) or two (what sits
/// beside it: the cherry-pick's source, or commits another ref still has).
const STAGE_W: f32 = 500.0;
const GUTTER: f32 = 78.0;

/// The edit as an animation on the real commits, among their neighbours and labels: every commit
/// is one element that stays mounted, and each step only moves it (its slot, its lane), fades it
/// or colours it. The checked-out branch's label moves from the old tip to the new one; commits
/// that leave but that a remote or another branch still has move to the second lane instead of
/// vanishing; a cherry-pick's copy travels from its source in the second lane.
fn animation(k: &Kit, plan: &Plan, a: Anim) -> El {
    const CAP: usize = 6;
    let (nodes, hidden) = history::nodes(plan, CAP);
    let step = a.step;
    let after = step >= 2;
    // Old commits another ref still has: they get a ghost that stays, in the second lane.
    let ghosts: Vec<&history::Node> = nodes.iter().filter(|n| matches!(n.kind, Some(Kind::Leaves | Kind::Rewritten | Kind::Joined)) && n.old.as_ref().is_some_and(|l| !l.kept_by.is_empty()) && n.before.is_some()).collect();
    let amend = plan.op == Op::Amend;
    let two = plan.source.is_some() || !ghosts.is_empty() || amend;
    // The commit you picked: ringed in every step, as in the guide's pictures.
    let target = target_sha(plan);
    let lane = if two { (STAGE_W - GUTTER) / 2.0 } else { STAGE_W - GUTTER };
    let x2 = GUTTER + lane;
    let rows = nodes.iter().flat_map(|n| [n.before, n.after]).flatten().max().map_or(1, |m| m + 1).max(1);
    let h = rows as f32 * ROW_H;
    let mut stage = col().key("h-anim").w(STAGE_W).h(h).clip_rounded(0.0);
    // The lanes' lines: the branch always; the second lane when something is in it.
    stage = stage.child(el().key("h-anim-line").absolute(0.0, 0.0).size(1.5, h).offset(GUTTER + NODE_W / 2.0 - 0.75, 0.0).fill(k.c.text_faint));
    if two {
        let shown = if plan.source.is_some() { 0.6 } else if after { 0.6 } else { 0.0 };
        stage = stage.child(el().key("h-anim-line2").absolute(0.0, 0.0).size(1.0, h).offset(x2 + NODE_W / 2.0 - 0.5, 0.0).fill(k.c.border_strong).opacity(shown).opacity_motion(MOVE));
    }

    // One commit's row: dot, SHA, its labels, subject.
    // `ring`: your commit, followed through the edit (made again, it keeps the ring).
    let commit_row = |key: String, line: Option<&history::Line>, tone: rune::Color, hollow: bool, faint: bool, pills: Vec<String>, w: f32, ring: bool| -> El {
        let (sha, subject) = line.map_or((String::new(), String::new()), |l| (l.sha.clone(), l.subject.clone()));
        let dot = if ring {
            // Your commit: a bigger dot with a strong ring.
            el().size(14.0, 14.0).radius(7.0).fill(if hollow { k.c.bg_base } else { tone }).border(2.5, k.c.text_strong)
        } else {
            el().size(11.0, 11.0).radius(6.0).fill(if hollow { k.c.bg_base } else { tone }).border(1.5, tone)
        };
        let mut r = row()
            .key(key)
            .absolute(0.0, 0.0)
            .w(w)
            .h(ROW_H)
            .gap(6.0)
            .center_y()
            .child(row().w(NODE_W).center().child(dot))
            .child(text(sha).font(k.ty.mono).color(if faint { k.c.text_faint } else { tone }));
        for p in pills.iter().take(1) {
            r = r.child(ref_pill(k, p, false).max_w(96.0));
        }
        r.child(text(subject).font(k.ty.small).color(if faint { k.c.text_faint } else { k.c.text_base }).truncate().shrink().min_w(0.0))
    };

    for n in &nodes {
        let slot = if after { n.after.or(n.before) } else { n.before.or(n.after) }.unwrap_or(0);
        let kind = n.kind;
        // Where it sits (x) and how visible it is, by step.
        let (mut x, mut o) = (GUTTER, 1.0);
        let mut y = slot as f32 * ROW_H;
        if n.side {
            x = x2;
            o = if step == 0 { 0.75 } else { 1.0 };
        } else {
            match kind {
                // A cherry-picked copy: hidden at its source, shows there, then travels in.
                // A cherry-picked copy: hidden on its source until the last step, then it comes out
                // of the source and travels to the top of the branch.
                Some(Kind::New) if !after && n.from.is_some() => {
                    (x, o) = (x2, 0.0);
                    y = n.from.unwrap_or(0) as f32 * ROW_H;
                }
                Some(Kind::New) if !after => (x, o) = (GUTTER - 24.0, 0.0),
                Some(Kind::Leaves) if step == 1 => (x, o) = (GUTTER + 24.0, 0.45),
                Some(Kind::Leaves) if after => (x, o) = (GUTTER + 36.0, 0.0),
                Some(Kind::Joined) if after => o = 0.0,
                _ => {}
            }
        }
        let tone = match kind {
            Some(kd) if step >= 1 => tone(k, kd).0,
            _ => k.c.text_faint,
        };
        // A joined commit keeps its own name while it goes into the one it joins.
        let line = if after && kind != Some(Kind::Joined) { n.new.as_ref().or(n.old.as_ref()) } else { n.old.as_ref().or(n.new.as_ref()) };
        let faint = kind.is_none() && !n.side;
        let pills = if n.side { line.map(|l| l.kept_by.clone()).unwrap_or_default() } else { line.map(|l| l.refs.clone()).unwrap_or_default() };
        // The cherry-pick's source lights up in "What changes": this is what gets copied.
        let lit = n.side && step == 1;
        let ring = [n.old.as_ref(), n.new.as_ref()].into_iter().flatten().any(|l| Some(l.full.as_str()) == target.as_deref());
        let node = commit_row(format!("h-an-{}", n.key), line, if lit { k.c.success.icon } else if n.side { k.c.text_muted } else { tone }, kind.is_none() && !lit, faint, pills, lane, ring)
            .offset(x, y)
            .offset_motion(MOVE)
            .opacity(o)
            .opacity_motion(MOVE);
        stage = stage.child(node);
    }

    // What another ref still has: ghosts that wait on the commit, then step aside to the second
    // lane in the last step, with the ref that keeps them.
    for g in &ghosts {
        let old = g.old.as_ref();
        let slot = g.before.unwrap_or(0) as f32 * ROW_H;
        let (x, o) = if after { (x2, 0.55) } else { (GUTTER, 0.0) };
        let pills = old.map(|l| l.kept_by.clone()).unwrap_or_default();
        let node = commit_row(format!("h-an-ghost-{}", g.key), old, k.c.text_muted, true, true, pills, lane, false).offset(x, slot).offset_motion(MOVE).opacity(o).opacity_motion(MOVE);
        stage = stage.child(node);
    }

    if hidden > 0 {
        // Between the edit and the commits under it, which keep their rows.
        let slot = plan.added.len().min(CAP).max(plan.removed.len().min(CAP));
        stage = stage.child(row().key("h-an-more").absolute(0.0, 0.0).h(ROW_H).offset(GUTTER + NODE_W + 8.0, slot as f32 * ROW_H).offset_motion(MOVE).center_y().child(muted(k, tf!("+{hidden} more"))));
    }

    // Amend: the staged changes wait beside the tip, then go into it.
    if amend {
        let row_y = nodes.iter().find(|n| n.old.as_ref().is_some_and(|l| l.full == plan.snap.tip)).and_then(|n| if after { n.after } else { n.before }).unwrap_or(0) as f32 * ROW_H;
        let (x, o) = if after { (GUTTER + 24.0, 0.0) } else { (x2, if step == 1 { 1.0 } else { 0.8 }) };
        let staged = row()
            .key("h-an-staged")
            .absolute(0.0, 0.0)
            .h(ROW_H)
            .gap(6.0)
            .center_y()
            .offset(x, row_y)
            .offset_motion(MOVE)
            .opacity(o)
            .opacity_motion(MOVE)
            .child(row().w(NODE_W).center().child(el().size(11.0, 11.0).radius(3.0).fill(if step >= 1 { k.c.success.bg } else { k.c.bg_base }).border(1.5, k.c.success.icon)))
            .child(text(crate::i18n::t("staged changes")).font(k.ty.small).color(k.c.success.fg));
        stage = stage.child(staged);
    }

    // The checked-out branch's label: at the old tip, then it moves to the new tip.
    let tip_slot = |sha: &str, use_after: bool| {
        nodes
            .iter()
            .filter(|n| !n.side)
            .find(|n| if use_after { n.new.as_ref().is_some_and(|l| l.full == sha) && n.kind != Some(Kind::Joined) } else { n.old.as_ref().is_some_and(|l| l.full == sha) })
            .and_then(|n| if use_after { n.after } else { n.before })
            .unwrap_or(0)
    };
    let label_slot = if after { tip_slot(&plan.new_tip, true) } else { tip_slot(&plan.snap.tip, false) };
    let label = row()
        .key("h-an-branch")
        .absolute(0.0, 0.0)
        .w(GUTTER - 8.0)
        .h(ROW_H)
        .center_y()
        .offset(0.0, label_slot as f32 * ROW_H)
        .offset_motion(MOVE)
        .child(ref_pill(k, &plan.snap.branch, true).max_w(GUTTER - 8.0))
        .help(crate::i18n::t(tf!("Where {} points: it moves to the new tip", plan.snap.branch)));
    stage = stage.child(label);

    // The steps: click one to look at it; the caption says what it shows.
    let mut steps = row().w_full().gap(6.0).center_y();
    for (i, name) in STEP_TEXT.iter().enumerate() {
        let on = i as u8 == step;
        let at = i as u8;
        steps = steps.child(
            row()
                .key(tf!("h-step-{i}"))
                .gap(5.0)
                .center_y()
                .px(8.0)
                .py(3.0)
                .radius(k.radius.sm)
                .fill(if on { k.c.surface_base } else { k.c.bg_base })
                .border(1.0, if on { k.c.border_strong } else { k.c.border_weak })
                .child(text(format!("{}", i + 1)).font(k.ty.text_xsmall_medium).color(if on { k.c.text_strong } else { k.c.text_faint }))
                .child(text(crate::i18n::t(*name)).font(k.ty.small).color(if on { k.c.text_base } else { k.c.text_muted }))
                .on_press(move |s: &mut State| {
                    if let Some(a) = s.history_open.anim.as_mut() {
                        *a = Anim { step: at, playing: false, at: s.clock };
                    }
                }),
        );
        if i + 1 < STEP_TEXT.len() {
            steps = steps.child(text("→").font(k.ty.small).color(k.c.text_faint));
        }
    }
    steps = steps.child(spacer()).child(
        IconButton::new(&glyphs::CLOSE)
            .variant(IconButtonVariant::GhostMuted)
            .label(crate::i18n::t("Close the animation"))
            .help(crate::i18n::t("Close the animation"))
            .on_press(|s: &mut State| s.history_open.anim = None)
            .build(k)
            .key("h-anim-close"),
    );
    let mut lines = vec![step_text(plan, step)];
    if after && let Some(holder) = ghosts.iter().find_map(|g| g.old.as_ref().and_then(|l| l.kept_by.first())) {
        lines.push(tf!("Dimmed on the right: old commits that {holder} still has. They stay there until you force-push."));
    }
    let caption = lines.into_iter().fold(col().w_full().gap(2.0), |c, l| c.child(text(l).font(k.ty.small).color(k.c.text_muted).wrap()));
    col().key("h-anim-box").w_full().gap(8.0).child(steps).child(stage).child(caption)
}

/// A chip: icon and a few words, tinted by the risk.
fn chip(k: &Kit, key: &str, icon: &'static Icon, label: String, fg: rune::Color, bg: rune::Color, help: &str) -> El {
    row()
        .key(key.to_owned())
        .gap(5.0)
        .center_y()
        .px(8.0)
        .py(4.0)
        .radius(k.radius.sm)
        .fill(bg)
        .child(rune::icon(icon, 13.0).color(fg))
        .child(text(label).font(k.ty.small_medium).color(fg))
        .help(crate::i18n::t(help))
}

/// The risk as chips (always shown: they're what you decide on), with the full reasons behind
/// "Why?". Replaces the reasons list in the guard dialog for a history edit.
pub fn risk_strip(s: &State, k: &Kit, p: &Pending) -> El {
    let (fg, bg) = match p.risk.level {
        Level::High => (k.c.danger.fg, k.c.danger.bg),
        Level::Medium => (k.c.warning.fg, k.c.warning.bg),
        Level::Low => (k.c.success.fg, k.c.success.bg),
    };
    let mut chips = row().key("h-risk").w_full().gap(6.0).center_y().flex_wrap();
    chips = chips.child(chip(k, "h-level", if p.risk.level == Level::Low { &I_SHIELD } else { &I_WARN }, crate::i18n::t(p.risk.level.label()), fg, bg, "How careful sigit is before running it"));
    for (i, (tag, label)) in p.risk.tags.iter().enumerate() {
        let (icon, help) = match tag {
            Tag::Pushed => (&I_PUSHED, "Some of these commits are already on a remote: sharing the edit needs a force-push"),
            Tag::Important => (&I_STAR, "The default, a protected, or a main/release branch"),
            Tag::Files => (&I_FILE, "Files in your working copy are replaced"),
            Tag::Warn => (&I_WARN, "The remote couldn't be checked for new commits"),
        };
        chips = chips.child(chip(k, &tf!("h-tag-{i}"), icon, label.clone(), fg, bg, help));
    }
    let quiet = (k.c.text_muted, k.c.surface_base);
    chips = chips
        .child(chip(k, "h-preflight", &I_SHIELD, crate::i18n::t("Preflight"), quiet.0, quiet.1, "Before it runs, sigit checks again that the branch, its upstream and the remote didn't change. If one did, nothing changes."))
        .child(chip(k, "h-undo", &I_UNDO, crate::i18n::t("Undo 10 s"), quiet.0, quiet.1, "Undo puts the branch back for 10 seconds after; the reflog keeps the old tip longer"))
        .child(spacer());
    let why = s.history_open.why;
    if !p.risk.reasons.is_empty() {
        chips = chips.child(
            row()
                .key("h-why")
                .gap(5.0)
                .center_y()
                .child(rune::icon(if why { &glyphs::TRIANGLE_DOWN } else { &glyphs::TRIANGLE_RIGHT }, 9.0).color(k.c.icon_faint))
                .child(text(crate::i18n::t("Why?")).font(k.ty.small).color(k.c.text_muted))
                .help(crate::i18n::t(if why { "Hide the reasons" } else { "Show the reasons" }))
                .on_press(|s: &mut State| s.history_open.why ^= true),
        );
    }
    let mut out = col().w_full().gap(8.0).child(chips);
    if why {
        let mut list = col().key("h-why-box").w_full().gap(6.0).padding(10.0).radius(k.radius.md).fill(bg);
        for (i, r) in p.risk.reasons.iter().enumerate() {
            let sub = r.starts_with("  •");
            list = list.child(row().key(tf!("h-why-{i}")).w_full().gap(6.0).child(text(if sub { "" } else { "•" }).font(k.ty.small).color(fg)).child(text(r.trim_start_matches("  • ").to_owned()).font(k.ty.small).color(k.c.text_base).wrap().shrink().selectable(true)));
        }
        out = out.child(list);
    }
    out
}

pub fn reword_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some((sha, msg)) = &s.reword else { return };
    let empty = msg.text.trim().is_empty();
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(12.0)
        .child(TextArea::new(msg).rows(8).width(None).on_edit(|s: &mut State| &mut s.reword.get_or_insert_with(|| (String::new(), rune::TextEdit::new(""))).1).build(k).key("rw-msg"))
        .child(muted(k, "The first line is the subject. The commits after this one are made again with new SHAs.").wrap())
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(spacer())
                .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.reword = None).build(k).key("rw-no"))
                .child(Button::new(crate::i18n::t("Simulate")).variant(ButtonVariant::Submit).disabled(empty).help(crate::i18n::t("Shows the outcome first: nothing changes until you confirm")).on_press(|s: &mut State| s.submit_reword()).build(k).key("rw-go")),
        );
    let title = tf!("Reword {}", sha.get(..7).unwrap_or(sha));
    Dialog::new("reword", crate::i18n::t(title)).open(true).fit().width(560.0).on_close(|s: &mut State| s.reword = None).child(body).build(k, layer);
}
