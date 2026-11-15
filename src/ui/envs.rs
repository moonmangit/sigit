//! Environments: what's deployed where right now, and a swimlane of deployments over time.

use rune::{Color, El, Shape, TextStyle, canvas, col, el, row, spacer};
use crate::i18n::text;
use rune_kit::Kit;

use super::{chip, empty, hue, muted, outcome_color, panel, pending, ref_chip, scroll};
use crate::State;
use crate::model::{Deployment, Outcome, ago, now};

fn outcome(state: &str) -> Outcome {
    match state {
        "SUCCESS" | "ACTIVE" => Outcome::Success,
        "FAILURE" | "ERROR" => Outcome::Failure,
        "PENDING" | "IN_PROGRESS" | "QUEUED" | "WAITING" => Outcome::Running,
        _ => Outcome::Neutral, // INACTIVE, DESTROYED
    }
}

pub fn view(s: &State, k: &Kit) -> El {
    if let Some(p) = s.rd().and_then(|d| pending(k, &d.envs)) {
        return p;
    }
    let Some(envs) = s.rd().and_then(|d| d.envs.data.as_ref()) else { return empty(k, "") };
    let slug = s.cur.clone().unwrap_or_default();
    // Every environment that exists, plus any that only appear in deployments.
    let mut names = envs.names.clone();
    for d in &envs.deployments {
        if !names.contains(&d.environment) {
            names.push(d.environment.clone());
        }
    }
    if names.is_empty() {
        return super::empty_state(k, "no-envs", &super::I_CLOUD, "No environments yet", "This repository has no environments or deployments on GitHub. They show here once a workflow deploys to one.", None);
    }
    let now = now();

    let cards = names.iter().enumerate().fold(row().key("env-cards").w_full().gap(12.0).flex_wrap(), |r, (i, name)| {
        let deps: Vec<&Deployment> = envs.deployments.iter().filter(|d| d.environment == *name).collect();
        let mut card = col()
            .key(tf!("env-{i}"))
            .grow(1.0)
            .min_w(240.0)
            .gap(10.0)
            .padding(16.0)
            .radius(k.radius.lg)
            .fill(k.c.layer_01)
            .border(1.0, k.c.border_weak);
        let latest = deps.first();
        let o = latest.map_or(Outcome::Neutral, |d| outcome(&d.state));
        card = card.child(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(el().size(10.0, 10.0).radius(5.0).fill(outcome_color(k, o)))
                .child(text(name.clone()).font(TextStyle { size: 16.0, ..k.ty.ui_medium }).color(k.c.text_strong))
                .child(spacer())
                .child(match latest {
                    Some(d) => chip(k, d.state.to_lowercase().replace('_', " "), Some(o)),
                    None => chip(k, "idle", None),
                }),
        );
        match latest {
            Some(d) => {
                // A link to the deployed commit.
                let url = tf!("https://github.com/{slug}/commit/{}", d.sha);
                card = card
                    .child(
                        row()
                            .gap(8.0)
                            .center_y()
                            .child(ref_chip(k, &d.git_ref, hue(i + 1)))
                            .child(
                                text(d.sha.get(..7).unwrap_or_default().to_owned())
                                    .font(k.ty.mono)
                                    .color(k.c.text_accent)
                                    .on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url))
                                    .help(crate::i18n::t("Open this commit on GitHub")),
                            )
                            .child(super::copy_button(k, &tf!("env-copy-{i}"), d.sha.clone(), "deployed SHA")),
                    )
                    .child(muted(k, tf!("by {} · {}", d.creator, ago(d.created_at, now))))
                    .child(muted(k, tf!("{} of the {} most recent deployments", deps.len(), envs.deployments.len())));
            }
            None => card = card.child(muted(k, "No recent deployments")),
        }
        r.child(card)
    });

    // Swimlanes: one row per environment, a dot per deployment, time left to right.
    let first = envs.deployments.iter().map(|d| d.created_at).filter(|t| *t > 0).min().unwrap_or(now - 86_400);
    let span = (now - first).max(3_600) as f32;
    const W: f32 = 1000.0;
    const ROW: f32 = 30.0;
    let h = ROW * names.len() as f32;
    let mut shape = Shape::new();
    for (i, _) in names.iter().enumerate() {
        let y = ROW * i as f32 + ROW / 2.0;
        shape = shape.line([0.0, y], [W, y], 1.0, rune::LineCap::Butt, k.c.border_weak);
    }
    for d in envs.deployments.iter().rev() {
        let Some(i) = names.iter().position(|n| *n == d.environment) else { continue };
        let x = ((d.created_at - first) as f32 / span * (W - 12.0)) + 6.0;
        let y = ROW * i as f32 + ROW / 2.0;
        shape = shape.fill(circle(x, y, 5.0), outcome_color(k, outcome(&d.state)));
    }
    let labels = names.iter().fold(col().w(120.0), |c, n| c.child(row().h(ROW).center_y().child(text(n.clone()).font(k.ty.small_medium).color(k.c.text_base).truncate())));
    let timeline = col()
        .w_full()
        .gap(6.0)
        .child(row().w_full().child(labels).child(canvas(shape.view_box(W, h)).key("swim").grow(1.0).h(h)))
        .child(row().w_full().pl(120.0).child(muted(k, ago(first, now))).child(spacer()).child(muted(k, "now")));

    let legend = row()
        .gap(14.0)
        .child(legend_item(k, "success", outcome_color(k, Outcome::Success)))
        .child(legend_item(k, "failed", outcome_color(k, Outcome::Failure)))
        .child(legend_item(k, "in progress", outcome_color(k, Outcome::Running)))
        .child(legend_item(k, "inactive", outcome_color(k, Outcome::Neutral)));

    let recent = envs.deployments.iter().take(30).enumerate().fold(col().w_full().gap(2.0), |c, (i, d)| {
        let lane = names.iter().position(|n| *n == d.environment).unwrap_or(0);
        let url = tf!("https://github.com/{slug}/commit/{}", d.sha);
        c.child(
            row()
                .key(tf!("dep-{i}"))
                .w_full()
                .gap(10.0)
                .px(8.0)
                .py(5.0)
                .radius(k.radius.md)
                .center_y()
                .hover(k.c.hover)
                .help(crate::i18n::t("Open the deployed commit on GitHub"))
                .on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url))
                .child(el().size(8.0, 8.0).radius(4.0).fill(outcome_color(k, outcome(&d.state))))
                .child(text(d.environment.clone()).font(k.ty.ui_medium).color(k.c.text_base).w(120.0).truncate())
                .child(ref_chip(k, &d.git_ref, hue(lane + 1)))
                .child(super::click_to_copy(text(d.sha.get(..7).unwrap_or_default().to_owned()).font(k.ty.mono).color(k.c.text_muted), k, d.sha.clone(), "deployed SHA"))
                .child(muted(k, d.creator.clone()).truncate().shrink())
                .child(spacer())
                .child(muted(k, ago(d.created_at, now))),
        )
    });

    scroll(
        "envs-scroll",
        col()
            .w_full()
            .gap(12.0)
            .child(cards)
            .child(panel(k, "swimlane", "Deployments over time", Some(legend), timeline))
            .child(panel(k, "recent", "Recent deployments", None, recent)),
    )
}

fn legend_item(k: &Kit, label: &str, c: Color) -> El {
    row().gap(5.0).center_y().child(el().size(8.0, 8.0).radius(4.0).fill(c)).child(muted(k, label))
}

fn circle(cx: f32, cy: f32, r: f32) -> Vec<[f32; 2]> {
    (0..16)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 16.0;
            [cx + a.cos() * r, cy + a.sin() * r]
        })
        .collect()
}
