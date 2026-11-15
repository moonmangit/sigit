//! Actions: a bar per recent run (height = duration, colour = outcome), the run list, and a
//! run's jobs and steps on a shared timeline, with re-run and cancel.

use super::cmd::CmdTip;
use std::borrow::Cow;

use rune::{El, TextStyle, col, el, row, spacer};
use crate::i18n::text;
use rune_kit::{Button, ButtonVariant, Cell, Kit, Table, TableColumn};

use super::{chip, empty, ext_button, hue, muted, outcome_color, pending, side_pane, status_mark};
use crate::model::{Outcome, Run, ago_str, duration, now, parse_time};
use crate::daily::WatchTarget;
use crate::guard::Action;
use crate::{State, risk};

fn runs(s: &State) -> &[Run] {
    s.rd().and_then(|d| d.runs.data.as_deref()).unwrap_or(&[])
}

pub fn view(s: &State, k: &Kit) -> El {
    if let Some(p) = s.rd().and_then(|d| pending(k, &d.runs)) {
        return p;
    }
    let all = runs(s);
    if all.is_empty() {
        return super::empty_state(k, "runs-empty", &super::I_REFRESH, "No workflow runs", "GitHub Actions hasn't run anything in this repository yet. Runs show here, newest first, as soon as a workflow starts.", None);
    }
    let now = now();

    // Summary + history chart, oldest on the left.
    let done: Vec<&Run> = all.iter().filter(|r| !r.outcome().is_running()).collect();
    let ok = done.iter().filter(|r| r.outcome() == Outcome::Success).count();
    let mut durs: Vec<i64> = done.iter().map(|r| r.duration(now)).collect();
    durs.sort_unstable();
    let median = durs.get(durs.len() / 2).copied().unwrap_or(0);
    let longest = all.iter().map(|r| r.duration(now)).max().unwrap_or(1).max(1);
    let bars = all.iter().rev().enumerate().fold(row().key("bars").w_full().h(72.0).gap(3.0).items_end(), |r, (i, run)| {
        let o = run.outcome();
        let h = (run.duration(now) as f32 / longest as f32 * 64.0).max(4.0) + 4.0;
        let id = run.database_id;
        let selected = s.run_sel == Some(id);
        r.child(
            el().key(tf!("b-{i}"))
                .grow(1.0)
                .h(h)
                .radius(2.0)
                .fill(rune::Color { a: if selected { 1.0 } else { 0.75 }, ..outcome_color(k, o) })
                .border(if selected { 1.5 } else { 0.0 }, k.c.text_strong)
                .hover(k.c.hover)
                .help(crate::i18n::t(tf!("{} · {} · {}", run.workflow_name, run.display_title, duration(run.duration(now)))))
                .on_press(move |s: &mut State| s.select_run(id)),
        )
    });
    let stat = |label: &str, value: String| col().gap(2.0).child(muted(k, label)).child(text(value).font(TextStyle { size: 18.0, ..k.ty.ui_medium }).color(k.c.text_strong));
    let summary = col()
        .key("summary")
        .w_full()
        .gap(10.0)
        .padding(14.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_weak)
        .child(
            row()
                .w_full()
                .gap(28.0)
                .child(stat("Success rate", tf!("{:.0}%", ok as f32 / done.len().max(1) as f32 * 100.0)))
                .child(stat("Median duration", duration(median)))
                .child(stat("Running", all.iter().filter(|r| r.outcome().is_running()).count().to_string()))
                .child(stat("Runs shown", all.len().to_string()))
                .child(spacer())
                .child(muted(k, "bar height = duration · click a bar to open the run")),
        )
        .child(bars);

    let kc = k.clone();
    let table = Table::new(
        |s: &State| &s.runs_table,
        |s: &mut State| &mut s.runs_table,
        |s: &State| runs(s).len(),
        |s: &State, i| Cow::Owned(runs(s).get(i).map_or_else(String::new, |r| r.database_id.to_string())),
    )
    .name("Workflow runs")
    .column(TableColumn::new("status", crate::i18n::t("")).width(36.0).resizable(false))
    .column(TableColumn::new("title", crate::i18n::t("Run")).flex(3.0).min(200.0))
    .column(TableColumn::new("workflow", crate::i18n::t("Workflow")).width(140.0).min(90.0))
    .column(TableColumn::new("branch", crate::i18n::t("Branch")).width(160.0).min(90.0))
    .column(TableColumn::new("event", crate::i18n::t("Event")).width(110.0).min(70.0))
    .column(TableColumn::new("dur", crate::i18n::t("Duration")).width(90.0).resizable(false))
    .column(TableColumn::new("ago", crate::i18n::t("Started")).width(84.0).resizable(false))
    .cell(move |s: &State, i, col| {
        let Some(r) = runs(s).get(i) else { return Cell::text("") };
        let k = &kc;
        match col {
            "status" => Cell::el(status_mark(k, r.outcome())),
            "title" => Cell::text(tf!("{}  #{}", r.display_title, r.number)),
            "workflow" => Cell::text(r.workflow_name.clone()).muted(),
            "branch" => Cell::el(super::ref_chip_fit(k, &r.head_branch, hue(1))),
            "event" => Cell::text(r.event.clone()).muted(),
            "dur" => Cell::text(duration(r.duration(now))).mono().muted(),
            _ => Cell::text(ago_str(&r.created_at, now)).muted(),
        }
    })
    .on_select(|s: &mut State, id, _| {
        if let Ok(id) = id.parse() {
            s.select_run(id);
        }
    })
    .build(s, k)
    .grow(1.0)
    .min_h(0.0);

    let mut list = col().key("runs-list").grow(1.0).min_w(0.0).h_full().radius(k.radius.lg).border(1.0, k.c.border_weak).fill(k.c.layer_01).clip_rounded(k.radius.lg).child(table);
    if all.len() >= s.run_limit {
        let loading = s.rd().is_some_and(|d| d.runs.loading);
        list = list.child(super::load_more(k, "runs-more", all.len(), crate::RUN_PAGE, loading, State::more_runs));
    }
    let mut panes = vec![super::split::pane(list, 0.6, 360.0)];
    if let Some(pane) = detail(s, k) {
        panes.push(super::split::pane(pane, 0.4, 340.0));
    }
    let body = super::split::split(s, k, "runs", panes).key("runs-body").grow(1.0).min_h(0.0);
    col().w_full().h_full().gap(12.0).child(summary).child(body)
}

fn detail(s: &State, k: &Kit) -> Option<El> {
    let id = s.run_sel?;
    let slug = s.cur.clone()?;
    let run = runs(s).iter().find(|r| r.database_id == id)?.clone();
    let now = now();
    let o = run.outcome();
    let mut c = col().w_full().gap(14.0);
    c = c.child(
        row()
            .w_full()
            .gap(8.0)
            .items_start()
            .child(status_mark(k, o))
            .child(text(run.display_title.clone()).font(TextStyle { size: 16.0, ..k.ty.ui_medium }).color(k.c.text_strong).wrap().grow(1.0).shrink())
            .child(super::copy_button(k, "run-copy", run.url.clone(), "link to this run"))
            .child(ext_button(k, "run-open", run.url.clone())),
    );
    let status = chip(k, if o.is_running() { run.status.clone() } else { run.conclusion.clone() }, Some(o));
    let sha = super::click_to_copy(text(run.head_sha.get(..7).unwrap_or_default().to_owned()).font(k.ty.mono).color(k.c.text_base), k, run.head_sha.clone(), "commit SHA");
    c = c.child(super::facts(
        k,
        vec![
            ("Status", status),
            ("Workflow", text(tf!("{} #{}", run.workflow_name, run.number)).font(k.ty.small).color(k.c.text_base)),
            ("Trigger", text(run.event.replace('_', " ")).font(k.ty.small).color(k.c.text_base)),
            ("Branch", super::ref_chip_fit(k, &run.head_branch, hue(1))),
            ("Commit", sha),
            ("Started", text(tf!("{} · took {}", ago_str(&run.created_at, now), duration(run.duration(now)))).font(k.ty.small).color(k.c.text_base)),
        ],
    ));

    // A4: say so up front when this looks like a deploy.
    if risk::looks_like_deploy(&run.workflow_name) {
        c = c.child(chip(k, "deploy workflow: re-run / cancel are checked against production", Some(Outcome::Running)).help(crate::i18n::t("Re-running or cancelling a run whose commit is deployed to a production environment needs the environment name typed")));
    }
    if s.flaky(&run) {
        c = c.child(chip(k, "Probably flaky: the same workflow on this commit has both failed and passed", Some(Outcome::Running)).help(crate::i18n::t("A failure that passes on re-run usually isn't caused by the code")));
    }
    let mut actions = row().w_full().gap(8.0).flex_wrap();
    let base = |failed_only: bool| Action::Rerun {
        slug: slug.clone(),
        id,
        title: run.display_title.clone(),
        workflow: run.workflow_name.clone(),
        sha: run.head_sha.clone(),
        failed_only,
    };
    if o == Outcome::Failure {
        let a = base(true);
        actions = actions.child(
            { let tip = super::action_help("Run only the jobs that failed again", &a); Button::new(crate::i18n::t("Re-run failed jobs"))
                .variant(ButtonVariant::Submit)
                
                .on_press(move |s: &mut State| s.request(a.clone()))
                .build(k)
                .key("rerun-failed").cmd_tip(s, tip) },
        );
    }
    if !o.is_running() {
        let a = base(false);
        actions = actions.child(
            { let tip = super::action_help("Run the whole workflow again on the same commit", &a); Button::new(crate::i18n::t("Re-run all jobs"))
                .variant(ButtonVariant::Outline)
                
                .on_press(move |s: &mut State| s.request(a.clone()))
                .build(k)
                .key("rerun-all").cmd_tip(s, tip) },
        );
    } else {
        let a = Action::CancelRun { slug: slug.clone(), id, title: run.display_title.clone(), workflow: run.workflow_name.clone(), sha: run.head_sha.clone() };
        actions = actions.child(
            { let tip = super::action_help("Stop every job where it is (asks first)", &a); Button::new(crate::i18n::t("Cancel run"))
                .variant(ButtonVariant::Danger)
                
                .on_press(move |s: &mut State| s.request(a.clone()))
                .build(k)
                .key("cancel").cmd_tip(s, tip) },
        );
    }
    // B6: watch it.
    let target = WatchTarget::Run { slug: slug.clone(), id };
    let watched = s.is_watched(&target);
    let label = tf!("{} #{}", run.workflow_name, run.number);
    if o.is_running() || watched {
        actions = actions.child(
            Button::new(crate::i18n::t(if watched { "Watching" } else { "Watch" }))
                .variant(if watched { ButtonVariant::Contrast } else { ButtonVariant::Ghost })
                .icon(&super::I_EYE)
                .help(crate::i18n::t("Get a desktop notification when this run finishes"))
                .on_press(move |s: &mut State| s.toggle_watch(target.clone(), label.clone()))
                .build(k)
                .key("watch"),
        );
    }
    c = c.child(actions);

    if let Some(p) = pending(k, &s.run_detail) {
        return Some(side_pane(k, "run-pane", c.child(p)));
    }
    if o == Outcome::Failure {
        let body: El = if s.triage_for != Some(id) {
            { let tip = s.with_commands(crate::i18n::t("Load the failing steps' last log lines"), &[crate::gh::failed_log_cmd(&slug, id).line()]); Button::new(crate::i18n::t("Show why it failed"))
                .variant(ButtonVariant::Outline)
                
                .on_press(move |s: &mut State| s.load_triage(id))
                .build(k)
                .key("load-triage").cmd_tip(s, tip) }
        } else if let Some(p) = pending(k, &s.triage) {
            p
        } else {
            let log = s.triage.data.clone().unwrap_or_default();
            let mut lines = col().w_full().gap(0.0);
            for (i, l) in log.lines().enumerate() {
                let lower = l.to_lowercase();
                let bad = ["error", "failed", "panicked", "fatal", "exception", "assert"].iter().any(|w| lower.contains(w));
                lines = lines.child(
                    text(l.to_owned())
                        .key(tf!("log-{i}"))
                        .font(rune::TextStyle { size: 11.5, ..k.ty.mono })
                        .color(if bad { k.c.danger.fg } else { k.c.text_muted })
                        .fill(if bad { k.c.danger.bg } else { rune::Color::TRANSPARENT }),
                );
            }
            if log.trim().is_empty() {
                lines = lines.child(muted(k, "GitHub returned no failed-step log (it may have expired)."));
            }
            col()
                .w_full()
                .radius(k.radius.md)
                .fill(k.c.bg_base)
                .border(1.0, k.c.border_weak)
                .clip_rounded(k.radius.md)
                .child(col().w_full().padding(10.0).child(lines).scroll_x())
                .child(row().w_full().px(10.0).pb(8.0).child(spacer()).child(rune_kit::Button::new(crate::i18n::t("Copy")).variant(ButtonVariant::GhostMuted).help(crate::i18n::t("Copy the log lines")).on_press_tx(move |_: &mut State, tx| {
                    tx.copy(log.clone());
                }).build(k).key("copy-log")))
        };
        c = c.child(
            col()
                .key("triage")
                .w_full()
                .gap(8.0)
                .child(row().gap(8.0).center_y().child(text("Why it failed").font(k.ty.small_medium).color(k.c.danger.fg)).child(super::overlays::hint(k, "h-triage", "The failing steps' last lines, errors highlighted", 5)))
                .child(body),
        );
    }
    let jobs = &s.run_detail.data.as_ref()?.jobs;
    if jobs.is_empty() {
        return Some(side_pane(k, "run-pane", c.child(empty(k, "No jobs yet"))));
    }

    // One time axis for every job and step: the run's first start to its last end.
    let t = |a: &str| parse_time(a);
    let start = jobs.iter().filter_map(|j| t(&j.started_at)).min().unwrap_or(now);
    let end = jobs.iter().map(|j| t(&j.completed_at).filter(|e| *e > 0).unwrap_or(now)).max().unwrap_or(now).max(start + 1);
    let span = (end - start) as f32;
    let bar = |key: String, a: &str, b: &str, o: Outcome, h: f32| {
        let s0 = t(a).unwrap_or(start);
        let e0 = t(b).filter(|e| *e >= s0).unwrap_or(if o.is_running() { now } else { s0 });
        let off = ((s0 - start) as f32 / span).clamp(0.0, 1.0);
        let len = (((e0 - s0) as f32) / span).clamp(0.004, 1.0 - off);
        row()
            .key(key)
            .w_full()
            .h(h)
            .child(el().w_frac(off).h_full())
            .child(el().w_frac(len).h_full().radius(3.0).fill(outcome_color(k, o)))
    };
    let mut timeline = col().key("timeline").w_full().gap(4.0).child(
        row()
            .w_full()
            .child(muted(k, "0s").w(210.0))
            .child(spacer())
            .child(muted(k, duration(end - start))),
    );
    for (ji, j) in jobs.iter().enumerate() {
        let jo = Outcome::from_parts(&j.status, &j.conclusion);
        let jd = t(&j.completed_at).unwrap_or(now) - t(&j.started_at).unwrap_or(now);
        let url = j.url.clone();
        timeline = timeline.child(
            row()
                .key(tf!("job-{ji}"))
                .w_full()
                .gap(8.0)
                .pt(6.0)
                .center_y()
                .hover(k.c.hover)
                .on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url))
                .child(row().w(202.0).gap(6.0).center_y().child(status_mark(k, jo)).child(text(j.name.clone()).font(k.ty.ui_medium).color(k.c.text_base).truncate().shrink()))
                .child(col().grow(1.0).child(bar(tf!("jb-{ji}"), &j.started_at, &j.completed_at, jo, 12.0)))
                .child(text(duration(jd.max(0))).font(k.ty.text_xsmall).color(k.c.text_muted).w(56.0)),
        );
        // Steps: every step of a failed job, otherwise only the long ones.
        for (si, st) in j.steps.iter().enumerate() {
            let so = Outcome::from_parts(&st.status, &st.conclusion);
            let sd = t(&st.completed_at).unwrap_or(now) - t(&st.started_at).unwrap_or(now);
            if jo != Outcome::Failure && sd < (span as i64 / 25).max(3) && so != Outcome::Failure {
                continue;
            }
            timeline = timeline.child(
                row()
                    .key(tf!("st-{ji}-{si}"))
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(row().w(202.0).pl(20.0).child(text(st.name.clone()).font(k.ty.text_xsmall).color(if so == Outcome::Failure { k.c.danger.fg } else { k.c.text_muted }).truncate().shrink()))
                    .child(col().grow(1.0).child(bar(tf!("sb-{ji}-{si}"), &st.started_at, &st.completed_at, so, 6.0)))
                    .child(text(duration(sd.max(0))).font(k.ty.text_xsmall).color(k.c.text_faint).w(56.0)),
            );
        }
    }
    c = c.child(col().w_full().gap(8.0).child(text("Jobs timeline").font(k.ty.small_medium).color(k.c.text_muted)).child(timeline));
    Some(side_pane(k, "run-pane", c))
}
