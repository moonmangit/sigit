//! Sections shown side by side, with a handle between each pair: drag it to resize, double-click
//! it to go back to the default sizes. Sizes are shares of the row's width, saved per layout in
//! prefs.json (`Prefs::splits`), so they hold when the window changes size.

use rune::{Cursor, Drag, El, Phase, col, el, row};
use rune_kit::Kit;

use crate::State;

/// Width of the handle between two sections (the old gap between them).
const GRIP: f32 = 12.0;

/// One section: its content, its default share, and the narrowest it may get (points).
pub struct Pane {
    content: El,
    share: f32,
    min: f32,
}

pub fn pane(content: El, share: f32, min: f32) -> Pane {
    Pane { content, share, min }
}

/// The shares to draw: what you dragged to, or the defaults (normalized to sum to 1).
pub fn shares(saved: Option<&Vec<f32>>, defaults: &[f32]) -> Vec<f32> {
    let v = match saved {
        Some(v) if v.len() == defaults.len() && v.iter().all(|x| x.is_finite() && *x > 0.0) => v.clone(),
        _ => defaults.to_vec(),
    };
    let sum: f32 = v.iter().sum();
    v.iter().map(|x| x / sum.max(f32::EPSILON)).collect()
}

/// Move the boundary after section `i` to `c` (share of the width from the left), keeping both
/// neighbours at least their minimum share.
pub fn drag_boundary(shares: &mut [f32], i: usize, c: f32, mins: &[f32]) {
    let left: f32 = shares[..i].iter().sum();
    let pair = shares[i] + shares[i + 1];
    let (lo, hi) = (mins[i], pair - mins[i + 1]);
    let a = if lo <= hi { (c - left).clamp(lo, hi) } else { pair / 2.0 };
    shares[i] = a;
    shares[i + 1] = pair - a;
}

/// Lay `panes` out side by side under `id` (one key per layout: the same id shares its sizes).
/// A single pane fills the row.
pub fn split(s: &State, k: &Kit, id: &'static str, panes: Vec<Pane>) -> El {
    let n = panes.len();
    let defaults: Vec<f32> = panes.iter().map(|p| p.share).collect();
    let mins: Vec<f32> = panes.iter().map(|p| p.min).collect();
    let shares = shares(s.prefs.splits.get(id), &defaults);
    let mut r = row().key(format!("split-{id}")).w_full().on_layout(move |s: &mut State, [x, _, w, _]| {
        s.split_boxes.insert(id, [x, w]);
    });
    for (i, p) in panes.into_iter().enumerate() {
        if i > 0 {
            r = r.child(grip(k, id, i - 1, defaults.clone(), mins.clone(), n));
        }
        let share = if n == 1 { 1.0 } else { shares[i] };
        r = r.child(col().key(format!("{id}-pane-{i}")).w(0.0).grow(share).min_w(p.min.min(200.0)).min_h(0.0).child(p.content));
    }
    r
}

fn grip(k: &Kit, id: &'static str, i: usize, defaults: Vec<f32>, mins_px: Vec<f32>, n: usize) -> El {
    row()
        .key(format!("{id}-grip-{i}"))
        .w(GRIP)
        .center()
        .radius(4.0)
        .hover(k.c.hover)
        .cursor(Cursor::ResizeColumn)
        .help("Drag to resize · double-click to reset")
        .child(el().w(3.0).h(28.0).radius(1.5).fill(k.c.border_base))
        .on_drag(move |s: &mut State, d: &mut Drag, _| match d.phase {
            Phase::Down => {
                if d.clicks >= 2 && s.prefs.splits.remove(id).is_some() {
                    s.save_prefs();
                }
            }
            Phase::Move => {
                let Some([x0, w]) = s.split_boxes.get(id).copied() else { return };
                let avail = (w - GRIP * (n - 1) as f32).max(1.0);
                let mins: Vec<f32> = mins_px.iter().map(|m| m / avail).collect();
                let mut v = shares(s.prefs.splits.get(id), &defaults);
                let c = (d.window_point()[0] - x0 - i as f32 * GRIP - GRIP / 2.0) / avail;
                drag_boundary(&mut v, i, c, &mins);
                s.prefs.splits.insert(id.to_owned(), v);
            }
            Phase::Up => s.save_prefs(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_fall_back_and_normalize() {
        assert_eq!(shares(None, &[3.0, 1.0]), vec![0.75, 0.25]);
        assert_eq!(shares(Some(&vec![0.5]), &[1.0, 1.0]), vec![0.5, 0.5], "saved for another layout shape");
        assert_eq!(shares(Some(&vec![0.2, 0.8]), &[1.0, 1.0]), vec![0.2, 0.8]);
    }

    #[test]
    fn dragging_keeps_minimums_and_other_panes() {
        let mut v = vec![0.3, 0.3, 0.4];
        drag_boundary(&mut v, 1, 0.9, &[0.1, 0.2, 0.25]);
        assert!((v[1] - 0.45).abs() < 1e-6 && (v[2] - 0.25).abs() < 1e-6, "{v:?}");
        assert_eq!(v[0], 0.3, "the first pane doesn't move");
        drag_boundary(&mut v, 0, 0.0, &[0.1, 0.2, 0.25]);
        assert!((v[0] - 0.1).abs() < 1e-6, "{v:?}");
        assert!((v.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }
}
