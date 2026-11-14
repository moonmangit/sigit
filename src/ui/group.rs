//! A group's dashboard (B5): CI health on each repo's default branch, what's deployed on each
//! environment, commits not yet on production, and every open PR in the group.

use rune::{El, TextStyle, col, el, row, spacer, text};
use rune_kit::{Kit, Spinner};

use super::overlays::hint;
use super::{chip, empty, hue, muted, outcome_color, panel, ref_chip, scroll, status_mark};
use crate::model::{Outcome, ago, ago_str, now};
use crate::risk;
use crate::{Page, State};

fn env_outcome(state: &str) -> Outcome {
    match state {
        "SUCCESS" | "ACTIVE" => Outcome::Success,
        "FAILURE" | "ERROR" => Outcome::Failure,
        "PENDING" | "IN_PROGRESS" | "QUEUED" | "WAITING" => Outcome::Running,
        _ => Outcome::Neutral,
    }
}

pub fn view(s: &State, k: &Kit) -> El {
    let Some(name) = s.group_cur.as_deref() else { return empty(k, "Pick a group") };
    let Some(group) = s.prefs.groups.iter().find(|g| g.name == name) else { return empty(k, "This group no longer exists") };
    if group.repos.is_empty() {
        return empty(k, "This group is empty: right-click repos in the sidebar to add them");
    }
    let t = now();
    let short = |slug: &str| slug.split_once('/').map_or(slug.to_owned(), |(_, n)| n.to_owned());

    // CI health: one row per repo, a dot per workflow (latest run on the default branch).
    let mut health = col().w_full().gap(6.0);
    // Deployments: repos × environments.
    let mut env_names: Vec<String> = Vec::new();
    for slug in &group.repos {
        if let Some(d) = s.group_data.get(slug).and_then(|l| l.data.as_ref()) {
            for e in &d.envs.names {
                if !env_names.contains(e) {
                    env_names.push(e.clone());
                }
            }
        }
    }
    let mut deploy = col().w_full().gap(6.0).child(
        env_names.iter().fold(row().w_full().gap(8.0).child(el().w(200.0)), |r, e| {
            let prod = risk::is_production(e, false, &s.prefs.prod_names);
            r.child(text(if prod { format!("{e} (prod)") } else { e.clone() }).font(k.ty.small_medium).color(if prod { k.c.danger.fg } else { k.c.text_muted }).w(170.0).truncate())
        }).child(text("Not on production yet").font(k.ty.small_medium).color(k.c.text_muted)),
    );
    let mut prs = col().w_full().gap(2.0);
    let mut pr_count = 0;
    for slug in &group.repos {
        let l = s.group_data.get(slug);
        let label = row().w(200.0).gap(6.0).center_y().child(text(short(slug)).font(k.ty.ui_medium).color(k.c.text_base).truncate().shrink().help(slug.clone()));
        let Some(d) = l.and_then(|l| l.data.as_ref()) else {
            let state: El = match l {
                Some(l) if l.loading => Spinner::new().size(12.0).build(k),
                Some(Load { error: Some(e), .. }) => muted(k, format!("couldn't load: {e}")).truncate(),
                _ => muted(k, "—"),
            };
            health = health.child(row().w_full().gap(8.0).center_y().child(label.clone()).child(state));
            continue;
        };
        // Health.
        let mut dots = row().gap(6.0).center_y().flex_wrap();
        for r in &d.health {
            let (slug2, id) = (slug.clone(), r.database_id);
            dots = dots.child(
                row()
                    .gap(4.0)
                    .center_y()
                    .px(6.0)
                    .py(2.0)
                    .radius(999.0)
                    .hover(k.c.hover)
                    .help(format!("{} on {}: {} · {}", r.workflow_name, d.default_branch, if r.outcome().is_running() { r.status.clone() } else { r.conclusion.clone() }, ago_str(&r.created_at, t)))
                    .on_press(move |s: &mut State| {
                        s.select_repo(&slug2, Page::Actions);
                        s.select_run(id);
                    })
                    .child(status_mark(k, r.outcome()))
                    .child(text(r.workflow_name.clone()).font(k.ty.text_xsmall).color(k.c.text_muted)),
            );
        }
        if d.health.is_empty() {
            dots = dots.child(muted(k, "no runs on the default branch"));
        }
        let (slug3, slug4) = (slug.clone(), slug.clone());
        health = health.child(
            row()
                .key(format!("h-{slug}"))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(label.clone().hover(k.c.hover).on_press(move |s: &mut State| s.select_repo(&slug3, Page::Pulls)))
                .child(ref_chip(k, &d.default_branch, hue(0)))
                .child(dots.grow(1.0).shrink()),
        );
        // Deployments.
        let mut r = row().key(format!("d-{slug}")).w_full().gap(8.0).center_y().child(label.clone().hover(k.c.hover).on_press(move |s: &mut State| s.select_repo(&slug4, Page::Envs)));
        for e in &env_names {
            let latest = d.envs.deployments.iter().find(|x| x.environment == *e);
            r = r.child(match latest {
                Some(dep) => row()
                    .w(170.0)
                    .gap(6.0)
                    .center_y()
                    .help(format!("{e}: {} by {} · {}", dep.state.to_lowercase(), dep.creator, ago(dep.created_at, t)))
                    .child(el().size(8.0, 8.0).radius(4.0).fill(outcome_color(k, env_outcome(&dep.state))))
                    .child(text(dep.git_ref.clone()).font(k.ty.text_xsmall).color(k.c.text_base).truncate().shrink())
                    .child(text(dep.sha.get(..7).unwrap_or_default().to_owned()).font(TextStyle { size: 11.0, ..k.ty.mono }).color(k.c.text_faint)),
                None => row().w(170.0).child(muted(k, "—")),
            });
        }
        let drift = d.drift.iter().map(|(env, _, n)| match n {
            Some(0) => format!("{env}: up to date"),
            Some(n) => format!("{env}: {n} commit(s) behind {}", d.default_branch),
            None => format!("{env}: unknown"),
        }).collect::<Vec<_>>().join(" · ");
        let behind = d.drift.iter().any(|(_, _, n)| n.is_some_and(|n| n > 0));
        r = r.child(if drift.is_empty() { muted(k, "no production deploys") } else { chip(k, drift, if behind { Some(Outcome::Running) } else { Some(Outcome::Success) }) });
        deploy = deploy.child(r);
        // PRs.
        for p in &d.prs {
            pr_count += 1;
            let (slug5, n) = (slug.clone(), p.number);
            let (o, passed, total) = p.checks();
            let found = risk::sensitive_files(p.files.iter().map(|f| (f.path.as_str(), f.additions + f.deletions)), &s.prefs.sensitive);
            let mut line = row()
                .key(format!("p-{slug}-{n}"))
                .w_full()
                .gap(10.0)
                .px(8.0)
                .py(5.0)
                .radius(k.radius.md)
                .center_y()
                .hover(k.c.hover)
                .on_press(move |s: &mut State| {
                    s.pr_state = 0;
                    s.select_repo(&slug5, Page::Pulls);
                    s.select_pr(n);
                })
                .child(text(short(slug)).font(k.ty.small).color(k.c.text_muted).w(160.0).truncate())
                .child(text(format!("#{n}")).font(k.ty.small).color(k.c.text_faint).w(50.0))
                .child(text(p.title.clone()).font(k.ty.ui).color(k.c.text_base).truncate().grow(1.0).shrink());
            for label in risk::chip_labels(&found) {
                line = line.child(chip(k, format!("⚠ {label}"), Some(Outcome::Running)));
            }
            line = line
                .child(if total > 0 { row().gap(4.0).center_y().child(status_mark(k, o)).child(muted(k, format!("{passed}/{total}"))) } else { muted(k, "—") })
                .child(text(p.author.login.clone()).font(k.ty.small).color(k.c.text_muted).w(110.0).truncate())
                .child(muted(k, ago_str(&p.updated_at, t)).w(70.0));
            prs = prs.child(line);
        }
    }
    if pr_count == 0 {
        prs = prs.child(muted(k, "No open pull requests in this group"));
    }
    if env_names.is_empty() {
        deploy = col().w_full().child(muted(k, "None of these repos has environments"));
    }

    scroll(
        "group-scroll",
        col()
            .key("group")
            .w_full()
            .padding(24.0)
            .gap(14.0)
            .child(
                row()
                    .gap(10.0)
                    .center_y()
                    .child(text(name.to_owned()).font(TextStyle { size: 22.0, ..k.ty.ui_medium }).color(k.c.text_strong))
                    .child(chip(k, format!("{} repos", group.repos.len()), None))
                    .child(spacer())
                    .child(muted(k, "refreshes every minute while open")),
            )
            .child(panel(k, "g-health", "CI health on default branches", Some(hint(k, "gh-h", "The latest run of each workflow on each repo's default branch. Click a dot to open the run.", 8)), health))
            .child(panel(k, "g-deploy", "Deployments", Some(hint(k, "gh-d", "What's deployed on each environment, and how many commits on the default branch aren't on production yet.", 8)), deploy))
            .child(panel(k, "g-prs", &format!("Open pull requests ({pr_count})"), Some(hint(k, "gh-p", "Every open PR in the group. ⚠ marks sensitive files.", 8)), prs)),
    )
}

use crate::Load;
