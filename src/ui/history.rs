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

/// One button that simulates `op`. `help`: what it does, in a sentence (on hover).
fn op_button(s: &State, k: &Kit, key: &str, icon: Option<&'static Icon>, label: &str, help: &str, op: Op) -> El {
    let busy = s.busy.contains(&history::sim_label(&op));
    let mut b = Button::new(crate::i18n::t(if busy { "Simulating…" } else { label }))
        .variant(ButtonVariant::Outline)
        .disabled(any_busy(s) || s.pending.is_some())
        .help(crate::i18n::t(tf!("{} Simulates it first: nothing changes until you confirm.", crate::i18n::t(help))))
        .on_press(move |s: &mut State| s.simulate_history(op.clone()));
    if let Some(i) = icon {
        b = b.icon(i);
    }
    b.build(k).key(key.to_owned())
}

/// A small icon-only button (move up / down), with its name for screen readers.
fn op_icon(s: &State, k: &Kit, key: &str, icon: &'static Icon, label: &str, op: Op) -> El {
    IconButton::new(icon)
        .variant(IconButtonVariant::GhostMuted)
        .label(crate::i18n::t(label))
        .help(crate::i18n::t(tf!("{}: simulated first", crate::i18n::t(label))))
        .disabled(any_busy(s) || s.pending.is_some())
        .on_press(move |s: &mut State| s.simulate_history(op.clone()))
        .build(k)
        .key(key.to_owned())
}

/// A group of buttons under a small icon: what kind of edit they are.
fn group(k: &Kit, key: &str, icon: &'static Icon, title: &str, buttons: El) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .gap(8.0)
        .items_start()
        .child(
            row()
                .w(96.0)
                .pt(7.0)
                .gap(5.0)
                .center_y()
                .child(rune::icon(icon, 13.0).color(k.c.icon_faint))
                .child(text(crate::i18n::t(title)).font(k.ty.small).color(k.c.text_faint)),
        )
        .child(buttons.grow(1.0).shrink().min_w(0.0))
}

/// The Edit history section of a commit's pane: three groups of labelled buttons.
pub fn actions(s: &State, k: &Kit, g: &CommitGraph, c: &Commit) -> Option<El> {
    let branch = s.rd().and_then(|d| d.status.data.as_ref()).map(|st| st.branch.clone()).filter(|b| !b.is_empty() && b != "(detached)")?;
    s.local_path()?;
    let sha = c.sha.clone();
    let here = on_head(g, &sha);
    let is_head = head_sha(g) == Some(sha.as_str());
    let merge = c.parents.len() > 1;
    let root = c.parents.is_empty();
    let mut out = col().key("h-actions").w_full().gap(8.0);

    // Title: the branch, and an info button that opens the explainer.
    let open = s.history_open.help;
    out = out.child(
        row()
            .w_full()
            .gap(6.0)
            .center_y()
            .child(rune::icon(&I_EDIT, 13.0).color(k.c.icon_faint))
            .child(text(tf!("Edit history of {branch}")).font(k.ty.small_medium).color(k.c.text_muted).truncate().shrink().min_w(0.0))
            .child(spacer())
            .child(
                IconButton::new(&I_INFO)
                    .variant(IconButtonVariant::GhostMuted)
                    .label(crate::i18n::t("How editing history works"))
                    .help(crate::i18n::t(if open { "Hide" } else { "How editing history works" }))
                    .on_press(|s: &mut State| s.history_open.help ^= true)
                    .build(k)
                    .key("h-help"),
            ),
    );
    if open {
        let line = |icon: &'static Icon, t: &str| row().gap(8.0).items_start().child(rune::icon(icon, 13.0).color(k.c.icon_faint)).child(text(crate::i18n::t(t)).font(k.ty.small).color(k.c.text_muted).wrap().shrink());
        out = out.child(
            col()
                .key("h-help-box")
                .w_full()
                .gap(6.0)
                .padding(10.0)
                .radius(k.radius.md)
                .fill(k.c.bg_base)
                .child(line(&I_STAR, "Each button simulates first: you see the new history before anything changes."))
                .child(line(&I_SHIELD, "Before it runs, a preflight checks that the branch, its upstream and the remote didn't change."))
                .child(line(&I_UNDO, "Undo (10 s) puts the branch back. sigit never pushes."))
                .child(line(&I_FILE, "Your uncommitted changes stay; a hard reset stashes them first.")),
        );
    }

    if !is_head {
        let modes = [(ResetMode::Soft, "Soft", "Move the branch here; the changes after it stay staged."), (ResetMode::Mixed, "Mixed", "Move the branch here; the changes after it stay in your files, unstaged."), (ResetMode::Hard, "Hard", "Move the branch here and make your files match it.")];
        let buttons = modes.into_iter().fold(row().gap(6.0).flex_wrap(), |r, (mode, label, help)| r.child(op_button(s, k, &tf!("h-reset-{}", mode.word()), None, label, help, Op::Reset { to: sha.clone(), mode })));
        out = out.child(group(k, "h-g-reset", &I_RESET, "Reset here", buttons));
    }
    let mut add = row().gap(6.0).flex_wrap();
    let mut has_add = false;
    if here && !root {
        add = add.child(op_button(s, k, "h-revert", Some(&I_UNDO), "Revert", "A new commit on top that undoes this one.", Op::Revert { sha: sha.clone() }));
        has_add = true;
    }
    if !here && !merge && !root {
        add = add.child(op_button(s, k, "h-pick", Some(&I_ADD), &tf!("Cherry-pick onto {branch}"), "A copy of this commit on top of your branch.", Op::CherryPick { sha: sha.clone() }));
        has_add = true;
    }
    if has_add {
        out = out.child(group(k, "h-g-add", &I_ADD, "Add a commit", add));
    }
    if here && !merge && !root {
        // The full message comes with the commit's detail: wait for it, so the body is kept.
        let message = s.commit_detail.data.as_ref().map(|d| d.body.clone());
        let loaded = message.is_some();
        let message = message.unwrap_or_default();
        let id = sha.clone();
        let mut rw = row().gap(6.0).flex_wrap().child(
            Button::new(crate::i18n::t("Reword…"))
                .variant(ButtonVariant::Outline)
                .icon(&I_EDIT)
                .disabled(s.pending.is_some() || !loaded)
                .help(crate::i18n::t("Change this commit's message; the commits after it are made again."))
                .on_press(move |s: &mut State| s.start_reword(id.clone(), message.clone()))
                .build(k)
                .key("h-reword"),
        );
        rw = rw
            .child(op_button(s, k, "h-drop", Some(&I_DROP), "Drop", "Remove this commit; the commits after it are made again.", Op::Drop { sha: sha.clone() }))
            .child(op_button(s, k, "h-squash", Some(&I_SQUASH), "Squash", "Join it with the commit before it, keeping both messages.", Op::Squash { sha: sha.clone(), fixup: false }))
            .child(op_button(s, k, "h-fixup", Some(&I_SQUASH), "Fixup", "Join it with the commit before it, keeping that commit's message.", Op::Squash { sha: sha.clone(), fixup: true }));
        // Move up / down stay side by side when the row wraps.
        let mut moves = row().gap(2.0).center_y().radius(k.radius.md).border(1.0, k.c.border_weak);
        if !is_head {
            moves = moves.child(op_icon(s, k, "h-up", &glyphs::CHEVRON_UP, "Move up", Op::Move { sha: sha.clone(), up: true }));
        }
        moves = moves.child(op_icon(s, k, "h-down", &glyphs::CHEVRON_DOWN, "Move down", Op::Move { sha: sha.clone(), up: false }));
        rw = rw.child(moves);
        out = out.child(group(k, "h-g-rewrite", &I_EDIT, "Rewrite", rw));
    }
    Some(out)
}

/// Amend, for the Commits toolbar: put the staged changes into the last commit.
pub fn amend_button(s: &State, k: &Kit) -> Option<El> {
    s.local_path()?;
    Some(op_button(s, k, "h-amend", Some(&I_EDIT), "Amend last commit", "Put your staged changes into the last commit.", Op::Amend))
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
        .child(spacer());
    for (kind, help) in [(Kind::Leaves, "Commits that leave the branch"), (Kind::New, "New commits"), (Kind::Rewritten, "Commits made again with new SHAs"), (Kind::Joined, "Commits joined into the one above")] {
        let n = count(kind);
        if n > 0 {
            head = head.child(count_pill(k, kind, n, help));
        }
    }
    c = c.child(head);

    // The graph, newest first.
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

    // Files: a bar each; the total in the title.
    const FILES: usize = 4;
    if plan.files.is_empty() {
        c = c.child(row().gap(6.0).center_y().child(rune::icon(&I_FILE, 13.0).color(k.c.icon_faint)).child(muted(k, "Same files: only the history changes")));
    } else {
        let (add, del) = plan.files.iter().fold((0, 0), |(a, d), f| (a + f.add, d + f.del));
        c = c.child(
            row()
                .gap(6.0)
                .center_y()
                .child(rune::icon(&I_FILE, 13.0).color(k.c.icon_faint))
                .child(text(tf!("{} file(s)", plan.files.len())).font(k.ty.small_medium).color(k.c.text_base))
                .child(diffstat(k, add, del)),
        );
        let n = if open.files { plan.files.len() } else { plan.files.len().min(FILES) };
        let mut files = col().key("h-files").w_full().gap(3.0).pl(19.0);
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
