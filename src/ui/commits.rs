//! Commits: the history as a graph (lanes drawn per row inside a virtualised table, so 1,500
//! commits scroll as smoothly as 15), and a commit's message, files and diff.

use std::borrow::Cow;

use rune::{El, LineCap, Path, Shape, TextStyle, canvas, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, Cell, Kit, SegmentedControl, Table, TableColumn};

use super::{diff_view, diffstat, empty, hue, muted, pending, ref_chip, side_pane};
use crate::git::GraphRow;
use crate::model::{Commit, ago, now};
use crate::{CommitGraph, State};

const LANE: f32 = 14.0;
const ROW_H: f32 = 28.0; // rune-kit's table row height
const MAX_LANES: usize = 14;

fn graph(s: &State) -> Option<&CommitGraph> {
    s.rd().and_then(|d| d.graph.data.as_ref())
}

pub fn view(s: &State, k: &Kit) -> El {
    let toolbar = row()
        .key("c-tools")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            SegmentedControl::new(&["All branches", "Current branch"], Some(if s.all_branches { 0 } else { 1 }))
                .label("Which history")
                .fit()
                .on_pick(|s: &mut State, i, _| {
                    s.all_branches = i == 0;
                    s.refresh_graph();
                })
                .build(k)
                .key("c-scope"),
        )
        .child(spacer())
        .child(muted(k, graph(s).map_or(String::new(), |g| format!("{} commits · {} lanes", g.commits.len(), g.width))));

    let list = match s.rd().and_then(|d| pending(k, &d.graph)) {
        Some(p) => p,
        None if graph(s).is_none_or(|g| g.commits.is_empty()) => empty(k, "No commits"),
        None => {
            let n = graph(s).map_or(0, |g| g.commits.len());
            let mut c = col().w_full().h_full().child(table(s, k));
            if n >= s.commit_limit {
                let loading = s.rd().is_some_and(|d| d.graph.loading);
                c = c.child(super::load_more(k, "c-more", n, crate::COMMIT_PAGE, loading, State::more_commits));
            }
            c
        }
    };
    let mut panes = vec![super::split::pane(col().key("c-list").grow(1.0).min_w(0.0).h_full().radius(k.radius.lg).border(1.0, k.c.border_weak).fill(k.c.layer_01).clip_rounded(k.radius.lg).child(list), 0.6, 360.0)];
    if let Some(pane) = detail(s, k) {
        panes.push(super::split::pane(pane, 0.4, 340.0));
    }
    let body = super::split::split(s, k, "commits", panes).key("c-body").grow(1.0).min_h(0.0);
    col().w_full().h_full().gap(12.0).child(toolbar).child(body)
}

fn table(s: &State, k: &Kit) -> El {
    let lanes = graph(s).map_or(1, |g| g.width.clamp(1, MAX_LANES));
    let graph_w = lanes as f32 * LANE + 10.0;
    let kc = k.clone();
    let now = now();
    Table::new(
        |s: &State| &s.commits_table,
        |s: &mut State| &mut s.commits_table,
        |s: &State| graph(s).map_or(0, |g| g.commits.len()),
        |s: &State, i| Cow::Borrowed(graph(s).and_then(|g| g.commits.get(i)).map_or("", |c| c.sha.as_str())),
    )
    .name("Commits")
    .column(TableColumn::new("graph", "Graph").width(graph_w).resizable(false))
    .column(TableColumn::new("msg", "Message").flex(3.0).min(240.0))
    .column(TableColumn::new("author", "Author").width(140.0).min(80.0))
    .column(TableColumn::new("when", "Date").width(90.0).resizable(false))
    .column(TableColumn::new("sha", "Commit").width(82.0).resizable(false))
    .cell(move |s: &State, i, col| {
        let Some(g) = graph(s) else { return Cell::text("") };
        let (Some(c), Some(gr)) = (g.commits.get(i), g.rows.get(i)) else { return Cell::text("") };
        let k = &kc;
        match col {
            "graph" => Cell::el(lanes_cell(k, gr, c.parents.len() > 1, lanes, graph_w)),
            "msg" => {
                let mut r = row().gap(5.0).center_y().min_w(0.0);
                for name in c.refs.iter().take(4) {
                    r = r.child(ref_badge(s, k, name));
                }
                Cell::el(r.child(text(c.subject.clone()).font(k.ty.ui).color(k.c.text_base).truncate().shrink()))
            }
            "author" => Cell::text(c.author.clone()).muted(),
            "when" => Cell::text(ago(c.time, now)).muted(),
            _ => Cell::text(c.sha.get(..7).unwrap_or_default().to_owned()).mono().muted(),
        }
    })
    .on_select(|s: &mut State, sha, _| s.select_commit(sha))
    .build(s, k)
    .grow(1.0)
    .min_h(0.0)
}

/// HEAD, local branches, remote branches and tags each get their own look.
fn ref_badge(s: &State, k: &Kit, name: &str) -> El {
    if let Some(b) = name.strip_prefix("HEAD -> ") {
        return row().gap(3.0).child(ref_chip(k, "HEAD", hue(6)).help("Your clone is here")).child(super::branch_chip(s, k, b, hue(1)).help("Your current branch (click to copy)"));
    }
    if let Some(t) = name.strip_prefix("tag: ") {
        return super::ref_pill(k, &format!("🏷 {t}"), t, hue(2)).help("A tag (click to copy)");
    }
    let remote = name.contains('/');
    super::branch_chip(s, k, name, if remote { hue(3) } else { hue(1) }).help(if remote { "A branch on GitHub (click to copy)" } else { "A local branch (click to copy)" })
}

/// One row's slice of the graph: lines from the row above, the dot, lines to the parents.
fn lanes_cell(k: &Kit, gr: &GraphRow, merge: bool, lanes: usize, w: f32) -> El {
    let x = |lane: f32| 7.0 + lane.min((lanes - 1) as f32) * LANE;
    let y = |t: f32| t * ROW_H;
    let mut shape = Shape::new();
    for seg in &gr.segs {
        let (a, b) = ([x(seg.x0), y(seg.y0)], [x(seg.x1), y(seg.y1)]);
        let c = hue(seg.color);
        if a[0] == b[0] {
            shape = shape.line(a, b, 1.6, LineCap::Butt, c);
        } else {
            // A soft S-curve between lanes.
            let mid = (a[1] + b[1]) / 2.0;
            let p = Path::new().move_to(a).cubic_to([a[0], mid], [b[0], mid], b);
            shape = shape.stroke_path(&p, 1.6, LineCap::Round, c);
        }
    }
    let (cx, cy) = (x(gr.lane as f32), y(0.5));
    let dot = hue(gr.color);
    shape = shape.fill(circle(cx, cy, 4.5), dot);
    if merge {
        // Merge commits are hollow.
        shape = shape.fill(circle(cx, cy, 2.2), k.c.layer_01);
    }
    canvas(shape.view_box(w, ROW_H)).size(w, ROW_H)
}

fn circle(cx: f32, cy: f32, r: f32) -> Vec<[f32; 2]> {
    (0..14)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 14.0;
            [cx + a.cos() * r, cy + a.sin() * r]
        })
        .collect()
}

fn detail(s: &State, k: &Kit) -> Option<El> {
    let sha = s.commit_sel.as_deref()?;
    let c: &Commit = graph(s)?.commits.iter().find(|c| c.sha == sha)?;
    let now = now();
    let mut col_ = col().w_full().gap(12.0);
    col_ = col_.child(
        row()
            .w_full()
            .gap(8.0)
            .items_start()
            .child(text(c.subject.clone()).font(TextStyle { size: 16.0, ..k.ty.ui_medium }).color(k.c.text_strong).wrap().grow(1.0).shrink())
            .child(super::copy_button(k, "c-copy-msg", c.subject.clone(), "commit message")),
    );
    let mut meta = row().w_full().gap(6.0).center_y().flex_wrap().child(muted(k, format!("{} · {}", c.author, ago(c.time, now))));
    for r in &c.refs {
        meta = meta.child(ref_badge(s, k, r));
    }
    col_ = col_.child(meta);
    col_ = col_.child(
        row()
            .w_full()
            .gap(6.0)
            .center_y()
            .child(text(c.sha.clone()).font(k.ty.mono).color(k.c.text_muted).selectable(true).shrink())
            .child(super::copy_button(k, "c-copy-sha", c.sha.clone(), "commit SHA")),
    );
    if !c.parents.is_empty() {
        let parents = c.parents.iter().map(|p| p.get(..7).unwrap_or(p)).collect::<Vec<_>>().join("  ");
        col_ = col_.child(muted(k, format!("parents  {parents}")));
    }
    // Check out a branch that points here.
    let branches: Vec<String> = c
        .refs
        .iter()
        .filter_map(|r| {
            let r = r.strip_prefix("HEAD -> ").unwrap_or(r);
            (!r.starts_with("tag: ") && r != "HEAD").then(|| r.to_owned())
        })
        .collect();
    if !branches.is_empty() {
        let mut actions = row().gap(8.0).flex_wrap();
        for b in branches.into_iter().take(3) {
            let remote = b.contains('/');
            let name = b.clone();
            actions = actions.child(
                Button::new(format!("Checkout {b}"))
                    .variant(ButtonVariant::Outline)
                    .on_press(move |s: &mut State| s.checkout_branch(name.clone(), remote))
                    .build(k)
                    .key(format!("co-{b}")),
            );
        }
        col_ = col_.child(actions);
    }
    if let Some(p) = pending(k, &s.commit_detail) {
        return Some(side_pane(k, "c-pane", col_.child(p)));
    }
    let d = s.commit_detail.data.as_ref()?;
    let body = d.body.split_once('\n').map_or("", |(_, rest)| rest).trim();
    if !body.is_empty() {
        col_ = col_.child(text(body.to_owned()).font(k.ty.small).color(k.c.text_base).wrap().selectable(true));
    }
    let (add, del) = d.files.iter().fold((0, 0), |(a, r), f| (a + f.1, r + f.2));
    let files = d.files.iter().enumerate().fold(col().w_full().gap(2.0), |c, (i, (path, a, r))| {
        c.child(
            row()
                .key(format!("cf-{i}"))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(super::click_to_copy(text(path.clone()).font(k.ty.mono).color(k.c.text_base).truncate().grow(1.0).shrink(), k, path.clone(), "file path"))
                .child(diffstat(k, *a, *r)),
        )
    });
    col_ = col_
        .child(row().w_full().gap(8.0).center_y().child(text(format!("{} files changed", d.files.len())).font(k.ty.small_medium).color(k.c.text_muted)).child(diffstat(k, add, del)))
        .child(files)
        .child(diff_view(k, &format!("cdiff-{sha}"), &d.diff));
    Some(side_pane(k, "c-pane", col_))
}
