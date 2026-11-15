//! The Activity tab of the Inbox: everything that changed in your tracked repos over a range
//! (default: this week). PRs, commits and lines, CI, deployments and releases, who contributed
//! what, and which repos moved most.

use rune::{El, TextStyle, col, el, row, spacer};
use crate::i18n::text;
use rune_kit::{Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, SegmentedControl, Spinner, TextInput, glyphs};

use super::overlays::hint;
use super::split::{pane, split};
use super::{chip, empty, hue, muted, outcome_color, panel, scroll};
use crate::model::{Outcome, ago, now};
use crate::pulse::{CAP, Preset, Pulse, RepoPulse};
use crate::{Page, State};

fn compact(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => tf!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => tf!("{:.0}k", n as f64 / 1e3),
        n if n >= 1_000 => tf!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

pub fn view(s: &State, k: &Kit) -> El {
    let repos = s.pulse_repos();
    let range = s.pulse.data.as_ref().and_then(|p| p.range.clone()).unwrap_or_else(|| s.pulse_range());

    // Header: range presets, custom dates, scope, manage.
    let pi = Preset::ALL.iter().position(|p| *p == s.pulse_preset);
    let mut controls = row().w_full().gap(10.0).center_y().flex_wrap().child(
        SegmentedControl::new(&crate::i18n::all(&Preset::ALL.map(|p| p.label())), pi)
            .label(crate::i18n::t("Time range"))
            .fit()
            .on_pick(|s: &mut State, i, _| s.set_pulse_preset(Preset::ALL[i]))
            .build(k)
            .key("range")
            .help(crate::i18n::t("What period to sum up. This week starts Monday, in your time zone.")),
    );
    if s.pulse_preset == Preset::Custom {
        let date = |e: &rune::TextEdit, key: &str, label: &str, edit: fn(&mut State) -> &mut rune::TextEdit| {
            TextInput::new(e).placeholder(crate::i18n::t("YYYY-MM-DD")).label(crate::i18n::t(label.to_owned())).width(Some(120.0)).invalid(chrono::NaiveDate::parse_from_str(e.text.trim(), "%Y-%m-%d").is_err() && !e.text.is_empty()).on_edit(edit).on_submit(|s: &mut State, _| s.refresh_pulse()).build(k).key(key.to_owned())
        };
        controls = controls
            .child(date(&s.pulse_from, "from", "From", |s| &mut s.pulse_from))
            .child(muted(k, "to"))
            .child(date(&s.pulse_to, "to", "To", |s| &mut s.pulse_to))
            .child(Button::new(crate::i18n::t("Apply")).variant(ButtonVariant::Submit).help(crate::i18n::t("Load this range (Enter in a date also works)")).on_press(|s: &mut State| s.refresh_pulse()).build(k).key("apply"));
    }
    let mut scopes = vec![tf!("All tracked ({})", s.tracked().len())];
    scopes.extend(s.prefs.groups.iter().map(|g| g.name.clone()));
    let si = match &s.pulse_scope {
        None => 0,
        Some(g) => s.prefs.groups.iter().position(|x| x.name == *g).map_or(0, |i| i + 1),
    };
    controls = controls.child(spacer());
    if scopes.len() > 1 {
        controls = controls.child(
            SegmentedControl::new(&crate::i18n::all(&scopes), Some(si))
                .label(crate::i18n::t("Which repos"))
                .fit()
                .on_pick(|s: &mut State, i, _| {
                    s.pulse_scope = if i == 0 { None } else { s.prefs.groups.get(i - 1).map(|g| g.name.clone()) };
                    s.refresh_pulse();
                })
                .build(k)
                .key("scope")
                .help(crate::i18n::t("All your tracked repos, or just one of your groups")),
        );
    }
    controls = controls.child(Button::new(crate::i18n::t("Manage tracked")).variant(ButtonVariant::Outline).help(crate::i18n::t("Choose which repos this page follows")).on_press(|s: &mut State| s.tracked_open = true).build(k).key("manage"));

    let head = col()
        .w_full()
        .gap(10.0)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(text("Activity").font(TextStyle { size: 22.0, ..k.ty.ui_medium }).color(k.c.text_strong))
                .child(muted(k, tf!("{} · {} repos", crate::i18n::dates(&crate::i18n::t(range.label.as_str())), repos.len())))
                .child(hint(k, "h-activity", "What changed in the repos you track: PRs, commits, lines, CI, deployments, and who did it", 1))
                .child(spacer())
                .child(if s.pulse.loading { row().gap(6.0).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, "Updating…")) } else { row() }),
        )
        .child(controls);

    let body: El = if repos.is_empty() {
        col()
            .w_full()
            .py(50.0)
            .gap(10.0)
            .items_center()
            .child(text("No repos tracked yet").font(TextStyle { size: 18.0, ..k.ty.ui_medium }).color(k.c.text_base))
            .child(muted(k, "Pin repos or put them in a group (they're tracked automatically), or right-click any repo → Track in Activity."))
            .child(Button::new(crate::i18n::t("Manage tracked")).variant(ButtonVariant::Contrast).on_press(|s: &mut State| s.tracked_open = true).build(k).key("manage2"))
    } else {
        match (&s.pulse.data, &s.pulse.error) {
            (Some(p), _) => dashboard(s, k, p),
            (None, Some(e)) => col().w_full().padding(24.0).child(text(tf!("Couldn't load activity: {e}")).font(k.ty.ui).color(k.c.danger.fg).wrap()),
            (None, None) => col().w_full().py(60.0).items_center().gap(10.0).child(Spinner::new().size(18.0).build(k)).child(muted(k, "Reading PRs, commits, runs and deployments…")),
        }
    };
    scroll("activity-scroll", col().key("activity").w_full().padding(24.0).gap(16.0).child(head).child(body))
}

fn dashboard(s: &State, k: &Kit, p: &Pulse) -> El {
    let mut c = col().w_full().gap(16.0);
    // 1. What you asked first: open PRs and Actions health, side by side.
    c = c.child(
        split(
            s,
            k,
            "heroes",
            vec![
                pane(if p.open_ready { open_prs_panel(k, p) } else { loading_panel(k, "o-prs-loading", "Open pull requests", "Loading open pull requests…") }, 0.5, 380.0),
                pane(
                    if p.repos.iter().any(|r| r.runs_ready) { health_panel(k, p) } else { loading_panel(k, "o-ci-loading", "Actions health", "Loading Actions runs…") },
                    0.5,
                    380.0,
                ),
            ],
        )
        .key("heroes"),
    );
    // 2. Everything else in the range, one quiet line.
    c = c.child(also_strip(k, p));
    for e in &p.errors {
        c = c.child(chip(k, tf!("Some data missing: {e}"), Some(Outcome::Failure)).help(crate::i18n::t(e.clone())));
    }
    // 3. Per-day activity and contributors, on demand.
    c = c.child(
        Button::new(crate::i18n::t(if s.pulse_details { "Hide activity details" } else { "Show activity details (per day, contributors)" }))
            .variant(ButtonVariant::GhostMuted)
            .icon(if s.pulse_details { &glyphs::CHEVRON_UP } else { &glyphs::CHEVRON_DOWN })
            .on_press(|s: &mut State| s.pulse_details = !s.pulse_details)
            .build(k)
            .key("details-toggle"),
    );
    if s.pulse_details {
        c = c.child(
            split(
                s,
                k,
                "activity-details",
                vec![
                    pane(panel(k, "o-daily", "Commits and merges per day", Some(day_legend(k)), day_bars(k, &p.per_day(), 90.0)), 0.5, 360.0),
                    pane(panel(k, "o-people", "Who contributed", Some(muted(k, "share of lines changed")), contributor_rows(k, &p.contributors(), 8)), 0.5, 320.0),
                ],
            ),
        );
    }
    // 4. Each repo: a summary line, details on click.
    let open_all = p.repos.iter().all(|r| s.pulse_open_repos.contains(&r.slug));
    let slugs: Vec<String> = p.repos.iter().map(|r| r.slug.clone()).collect();
    c = c.child(
        row()
            .w_full()
            .gap(10.0)
            .pt(8.0)
            .center_y()
            .child(text("Repositories").font(TextStyle { size: 17.0, ..k.ty.ui_medium }).color(k.c.text_strong))
            .child(muted(k, "click a row for its details"))
            .child(spacer())
            .child(
                Button::new(crate::i18n::t(if open_all { "Collapse all" } else { "Expand all" }))
                    .variant(ButtonVariant::GhostMuted)
                    .on_press(move |s: &mut State| {
                        if s.pulse_open_repos.len() >= slugs.len() {
                            s.pulse_open_repos.clear();
                        } else {
                            s.pulse_open_repos.extend(slugs.iter().cloned());
                        }
                    })
                    .build(k)
                    .key("fold-repos"),
            ),
    );
    // Repos with something wrong first: red workflows, then most open PRs. On a first load the
    // order isn't known until every repo is in, so one steady row stands in for the list rather
    // than rows that jump as each repo lands.
    let ready = p.repos.iter().filter(|r| r.detail_ready && r.runs_ready).count();
    if ready < p.repos.len() {
        c = c.child(
            row()
                .key("repos-loading")
                .w_full()
                .h(64.0)
                .gap(10.0)
                .center()
                .radius(k.radius.lg)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_weak)
                .child(Spinner::new().size(14.0).build(k))
                .child(muted(k, tf!("Loading repositories… {ready} of {}", p.repos.len()))),
        );
    } else {
        let mut repos: Vec<&RepoPulse> = p.repos.iter().collect();
        repos.sort_by_key(|r| std::cmp::Reverse((r.red_now().len(), r.prs_open)));
        for r in repos {
            c = c.child(repo_section(s, k, p, r));
        }
    }
    c.child(muted(k, tf!("Open PRs and PR counts are GitHub's exact numbers. Review / CI state per PR, run health, lines and contributors read up to the latest {CAP} items per repo.")).wrap())
}

// ---------------------------------------------------------------------------------------------
// Open pull requests
// ---------------------------------------------------------------------------------------------

/// A number that, when fresh data changes it, fades into its new value (keyed by the value,
/// so an unchanged number stays put and only changed ones move).
fn value_text(value: String) -> El {
    text(value.clone()).key(tf!("v:{value}")).transition(rune::Transition::blur_replace())
}

fn big_number(k: &Kit, value: String, color: rune::Color, caption: &str) -> El {
    row()
        .gap(10.0)
        .items_end()
        .child(value_text(value).font(TextStyle { size: 40.0, ..k.ty.ui_medium }).color(color))
        .child(text(caption.to_owned()).font(k.ty.ui).color(k.c.text_muted).pb(8.0))
}

fn age_days(created: i64) -> i64 {
    (now() - created).max(0) / 86_400
}

fn open_prs_panel(k: &Kit, p: &Pulse) -> El {
    let open = p.open_in(None);
    let ready = open.iter().filter(|x| x.ready()).count();
    let review = open.iter().filter(|x| x.ready_for_review()).count();
    let changes = open.iter().filter(|x| !x.draft && x.review == "CHANGES_REQUESTED").count();
    let failing = open.iter().filter(|x| !x.draft && x.failing()).count();
    let running = open.iter().filter(|x| !x.draft && x.ci() == crate::model::Ci::Running).count();
    let no_checks = open.iter().filter(|x| !x.draft && x.ci() == crate::model::Ci::None).count();
    let drafts = open.iter().filter(|x| x.draft).count();
    let label = crate::i18n::dates(&crate::i18n::t(p.range.as_ref().map_or("this range", |r| r.label.as_str()))).to_lowercase();

    let mut c = col()
        .key("open-prs")
        .w_full()
        .gap(14.0)
        .padding(18.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_weak)
        .child(row().w_full().gap(8.0).center_y().child(text("Open pull requests").font(TextStyle { size: 16.0, ..k.ty.ui_medium }).color(k.c.text_base)).child(spacer()).child(hint(k, "h-open", "PRs open right now in your tracked repos, whatever the range", 4)))
        .child(big_number(k, compact(p.open_total), k.c.text_strong, "open now"))
        .child(muted(k, if p.searches_left > 0 { tf!("Counting PRs opened and merged {label}…") } else { tf!("{} opened and {} merged {label}", compact(p.opened_total), compact(p.merged_total)) }));

    // Where they stand: two short lists, a coloured dot, the label and its count.
    let state = |label: &str, n: usize, tone: Option<Outcome>, tip: &str| {
        let dot = match tone {
            Some(o) if n > 0 => outcome_color(k, o),
            _ => k.c.icon_faint,
        };
        row()
            .w_full()
            .gap(8.0)
            .px(10.0)
            .py(4.0)
            .center_y()
            .help(crate::i18n::t(tip.to_owned()))
            .child(el().size(8.0, 8.0).radius(4.0).fill(dot))
            .child(text(label.to_owned()).font(k.ty.small).color(if n > 0 { k.c.text_base } else { k.c.text_muted }).truncate().shrink())
            .child(spacer())
            .child(value_text(n.to_string()).font(k.ty.ui_medium).color(if n > 0 { k.c.text_strong } else { k.c.text_faint }))
    };
    let list = |title: &str, rows: Vec<El>| {
        rows.into_iter().fold(
            col().w(0.0).grow(1.0).py(4.0).radius(k.radius.md).fill(k.c.bg_base).child(text(title.to_owned()).font(k.ty.text_xsmall_medium).color(k.c.text_faint).px(10.0).pt(6.0).pb(2.0)),
            |c, r| c.child(r),
        )
    };
    c = c.child(
        row()
            .w_full()
            .gap(10.0)
            .items_start()
            .child(list(
                "Checks",
                vec![
                    state("Ready for review", review, Some(Outcome::Success), "Waiting on a reviewer and every check passed: worth reviewing now"),
                    state("Checks failing", failing, Some(Outcome::Failure), "A check failed on the latest commit: not worth reviewing yet"),
                    state("Checks running", running, Some(Outcome::Running), "Checks still running on the latest commit"),
                    state("No checks ran", no_checks, None, "No checks ran on the latest commit (Actions may be waiting for approval): not counted as passed"),
                ],
            ))
            .child(list(
                "Review",
                vec![
                    state("Approved, mergeable", ready, Some(Outcome::Success), "Approved and checks green"),
                    state("Changes requested", changes, Some(Outcome::Failure), "A reviewer asked for changes"),
                    state("Drafts", drafts, None, "Still drafts"),
                ],
            )),
    );

    // How old.
    let buckets = [("< 1 day", 0, 1), ("1–7 days", 1, 7), ("1–4 weeks", 7, 28), ("> 4 weeks", 28, i64::MAX)];
    let counts: Vec<usize> = buckets.iter().map(|(_, lo, hi)| open.iter().filter(|x| (*lo..*hi).contains(&age_days(x.created))).count()).collect();
    let total = counts.iter().sum::<usize>().max(1);
    let colors = [k.c.success.icon, hue(0), k.c.warning.icon, k.c.danger.icon];
    let mut bar = row().w_full().h(12.0).radius(4.0).clip_rounded(4.0).fill(k.c.surface_base);
    let mut legend = row().w_full().gap(14.0).flex_wrap();
    for (i, ((label, _, _), n)) in buckets.iter().zip(&counts).enumerate() {
        if *n > 0 {
            bar = bar.child(el().w_frac(*n as f32 / total as f32).h_full().fill(colors[i]).help(crate::i18n::t(tf!("{n} open {label}"))));
        }
        legend = legend.child(row().gap(5.0).center_y().child(el().size(8.0, 8.0).radius(2.0).fill(colors[i])).child(muted(k, tf!("{label} {n}", label = crate::i18n::t(*label)))));
    }
    c = c.child(col().w_full().gap(6.0).child(text("How long they've been open").font(k.ty.small_medium).color(k.c.text_muted)).child(bar).child(legend));

    // Worth reviewing: checks passed, oldest first.
    let mut waiting = col().w_full().gap(2.0).child(text("Ready for review (all checks passed), oldest first").font(k.ty.small_medium).color(k.c.text_muted));
    let oldest: Vec<_> = open.iter().filter(|x| x.ready_for_review()).take(5).collect();
    if oldest.is_empty() {
        waiting = waiting.child(muted(k, if open.is_empty() { "Nothing open 🎉" } else { "Nothing ready: no PR waiting for review has all its checks passed" }));
    }
    for (i, x) in oldest.into_iter().enumerate() {
        waiting = waiting.child(open_pr_row(k, x, &tf!("w-{i}"), true, false));
    }
    let blocked = open.iter().filter(|x| x.needs_review() && !x.ci().reviewable()).count();
    if blocked > 0 {
        waiting = waiting.child(muted(k, tf!("{blocked} more waiting for review haven't passed their checks (failing, running or not run). Open the repo to see them.")).wrap().w_full());
    }
    c = c.child(waiting);
    if (open.len() as u64) < p.open_total {
        c = c.child(muted(k, tf!("States and ages from {} of {} open PRs", open.len(), p.open_total)));
    }
    c
}

/// `states`: show its checks and review chips (off in a list whose heading already says them,
/// like "Ready for review (all checks passed)", so the title gets the room).
fn open_pr_row(k: &Kit, x: &crate::pulse::OpenPr, key: &str, show_repo: bool, states: bool) -> El {
    let (slug, n) = (x.slug.clone(), x.number);
    let (label, tone) = if x.draft {
        ("draft", None)
    } else if x.review == "CHANGES_REQUESTED" {
        ("changes requested", Some(Outcome::Failure))
    } else if x.review == "APPROVED" {
        ("approved", Some(Outcome::Success))
    } else {
        ("needs review", Some(Outcome::Running))
    };
    let days = age_days(x.created);
    let meta = if show_repo { tf!("{} #{} · {}", x.slug.split_once('/').map_or("", |(_, r)| r), x.number, x.author) } else { tf!("#{} · {}", x.number, x.author) };
    row()
        .key(key.to_owned())
        .w_full()
        .gap(8.0)
        .px(6.0)
        .py(5.0)
        .radius(k.radius.md)
        .center_y()
        .hover(k.c.hover)
        .help(crate::i18n::t("Open it"))
        .on_press(move |s: &mut State| s.open_pr(&slug, n))
        .child(col().grow(1.0).shrink().gap(1.0).child(text(x.title.clone()).font(k.ty.ui).color(k.c.text_base).truncate()).child(text(meta).font(k.ty.text_xsmall).color(k.c.text_faint).truncate()))
        .child(if states { super::ci_chip(k, x.ci()) } else { row() })
        .child(if states { chip(k, label, tone) } else { row() })
        .child(text(if days == 0 { "today".to_owned() } else { tf!("{days}d") }).font(k.ty.small_medium).color(if days > 28 { k.c.danger.fg } else if days > 7 { k.c.warning.fg } else { k.c.text_muted }).w(44.0))
}

// ---------------------------------------------------------------------------------------------
// Actions health
// ---------------------------------------------------------------------------------------------

fn rate_color(k: &Kit, rate: f32) -> rune::Color {
    if rate >= 0.9 {
        k.c.success.icon
    } else if rate >= 0.75 {
        k.c.warning.icon
    } else {
        k.c.danger.icon
    }
}

fn health_panel(k: &Kit, p: &Pulse) -> El {
    let t = now();
    let runs: Vec<_> = p.repos.iter().flat_map(|r| r.runs.iter()).collect();
    // Exact totals for the headline; the sample only for running-now, minutes and per-day.
    let ok: u64 = p.repos.iter().map(|r| r.runs_ok_total).sum();
    let failed: u64 = p.repos.iter().map(|r| r.runs_failed_total).sum();
    let running = runs.iter().filter(|r| r.outcome.is_some_and(|o| o.is_running())).count();
    let rate = (ok + failed > 0).then(|| ok as f32 / (ok + failed) as f32);
    let sampled = p.repos.iter().any(|r| r.runs_capped());
    let runs_total: u64 = p.repos.iter().map(|r| r.runs_total).sum();
    let minutes: i64 = runs.iter().map(|r| r.secs).sum::<i64>() / 60;
    let red: Vec<(&str, String, i64, u64)> = p.repos.iter().flat_map(|r| r.red_now().into_iter().map(move |(w, at, id)| (r.slug.as_str(), w, at, id))).collect();

    let mut c = col()
        .key("health")
        .w_full()
        .gap(14.0)
        .padding(18.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_weak)
        .child(row().w_full().gap(8.0).center_y().child(text("Actions health").font(TextStyle { size: 16.0, ..k.ty.ui_medium }).color(k.c.text_base)).child(spacer()).child(hint(k, "h-health", "Pass rate of finished workflow runs in the range, and which workflows are failing on the default branch right now", 5)))
        .children(still_loading(k, p, |r| r.runs_ready, "Actions"))
        .child(match rate {
            Some(r) => big_number(k, tf!("{:.0}%", r * 100.0), rate_color(k, r), "of runs passed"),
            None => big_number(k, "—".into(), k.c.text_muted, "no finished runs"),
        })
        .child(muted(k, tf!("{} runs · {} passed · {} failed · {running} running now · {} CI min{}", compact(runs_total), compact(ok), compact(failed), compact(minutes as u64), if sampled { " (from latest 300/repo)" } else { "" })));

    // Red right now.
    let mut redl = col().w_full().gap(4.0).child(text("Failing on the default branch right now").font(k.ty.small_medium).color(k.c.text_muted));
    if red.is_empty() {
        redl = redl.child(row().gap(8.0).center_y().px(6.0).py(4.0).child(el().size(9.0, 9.0).radius(4.5).fill(k.c.success.icon)).child(text("Every workflow's latest run passed").font(k.ty.ui).color(k.c.success.fg)));
    }
    for (i, (slug, wf, at, id)) in red.iter().enumerate().take(6) {
        let (s2, wf2, id) = (slug.to_string(), wf.clone(), *id);
        redl = redl.child(
            row()
                .key(tf!("red-{i}"))
                .w_full()
                .gap(8.0)
                .px(6.0)
                .py(5.0)
                .radius(k.radius.md)
                .center_y()
                .fill(k.c.danger.bg)
                .hover(k.c.hover)
                .help(crate::i18n::t("Open this failed run"))
                .on_press(move |s: &mut State| open_run_or_failed(s, &s2, id, &wf2))
                .child(el().size(9.0, 9.0).radius(4.5).fill(k.c.danger.icon))
                .child(text(wf.clone()).font(k.ty.ui_medium).color(k.c.text_base).grow(1.0).truncate().shrink())
                .child(muted(k, slug.split_once('/').map_or("", |(_, r)| r).to_owned()))
                .child(muted(k, tf!("failed {}", ago(*at, t)))),
        );
    }
    c = c.child(redl);

    // Passed / failed per day.
    let days = p.ci_per_day(None);
    if !days.is_empty() {
        let peak = days.iter().map(|d| d.1 + d.2).max().unwrap_or(1).max(1);
        const H: f32 = 70.0;
        let mut bars = row().w_full().gap(4.0).items_end();
        for (i, (label, okn, bad)) in days.iter().enumerate() {
            let h = |n: u64| (n as f32 / peak as f32 * H).max(if n > 0 { 2.0 } else { 0.0 });
            // Columns share the width equally (w 0 + grow), and bars narrow to fit long ranges.
            let mut day = col()
                .key(tf!("cd-{i}"))
                .w(0.0)
                .grow(1.0)
                .gap(3.0)
                .items_center()
                .help(crate::i18n::t(tf!("{label}: {okn} passed, {bad} failed")))
                .child(col().w_full().max_w(16.0).h(H).child(spacer()).child(el().w_full().h(h(*bad)).radius(2.0).fill(k.c.danger.icon)).child(el().w_full().h(h(*okn)).radius(2.0).fill(k.c.success.icon)));
            // Every day keeps the same label slot (empty on the unlabelled ones), so all bars
            // stand on one baseline however many days the range has.
            day = day.child(day_label(k, label, days.len() <= 10 || i % 5 == 0));
            bars = bars.child(day);
        }
        let title = if sampled { "Runs per day (latest 300 per repo: busy repos' earlier days are undercounted)" } else { "Runs per day" };
        c = c.child(col().w_full().gap(6.0).child(row().w_full().child(text(title).font(k.ty.small_medium).color(k.c.text_muted).wrap().shrink()).child(spacer()).child(row().gap(10.0).child(row().gap(4.0).center_y().child(el().size(8.0, 8.0).radius(2.0).fill(k.c.success.icon)).child(muted(k, "passed"))).child(row().gap(4.0).center_y().child(el().size(8.0, 8.0).radius(2.0).fill(k.c.danger.icon)).child(muted(k, "failed"))))).child(bars));
    }
    c
}

/// A panel whose data hasn't arrived yet (the page fills in piece by piece).
fn loading_panel(k: &Kit, key: &str, title: &str, what: &str) -> El {
    // About as tall as the filled panel, so the page doesn't jump when it arrives.
    panel(k, key, title, None, row().w_full().h(420.0).center().gap(10.0).child(Spinner::new().size(14.0).build(k)).child(muted(k, what.to_owned())))
}

/// "Still loading 3 of 10 repos", while some repos' pieces are out.
fn still_loading(k: &Kit, p: &Pulse, ready: fn(&RepoPulse) -> bool, what: &str) -> Option<El> {
    let left = p.repos.iter().filter(|r| !ready(r)).count();
    (left > 0).then(|| row().gap(6.0).center_y().child(Spinner::new().size(10.0).build(k)).child(muted(k, tf!("Still loading {what} for {left} of {} repos", p.repos.len()))))
}

fn also_strip(k: &Kit, p: &Pulse) -> El {
    let commits: u64 = p.repos.iter().map(|r| r.commits_total).sum();
    let (add, del) = p.repos.iter().fold((0, 0), |(a, d), r| {
        let (x, y) = r.lines();
        (a + x, d + y)
    });
    let deploys: usize = p.repos.iter().map(|r| r.deploys.len()).sum();
    let deploy_fail: usize = p.repos.iter().flat_map(|r| r.deploys.iter()).filter(|d| d.1 == "FAILURE" || d.1 == "ERROR").count();
    let capped = p.repos.iter().any(|r| r.deploys_capped);
    let releases: usize = p.repos.iter().map(|r| r.releases.len()).sum();
    let people = p.contributors().len();
    // A number whose piece hasn't arrived shows "…", not a zero.
    let (prs_in, repos_in) = (p.searches_left == 0, p.repos.iter().all(|r| r.detail_ready));
    let or_wait = |ready: bool, v: String| if ready { v } else { "…".to_owned() };
    let item = |label: &str, value: String, tip: &str| col().gap(1.0).help(crate::i18n::t(tip.to_owned())).child(value_text(value).font(TextStyle { size: 17.0, ..k.ty.ui_medium }).color(k.c.text_base)).child(muted(k, label));
    row()
        .key("also")
        .w_full()
        .gap(28.0)
        .px(18.0)
        .py(12.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_weak)
        .flex_wrap()
        .center_y()
        .child(muted(k, "Also in this range"))
        .child(item("PRs merged", or_wait(prs_in, compact(p.merged_total)), "Pull requests merged"))
        .child(item("closed unmerged", or_wait(prs_in, compact(p.closed_total)), "Pull requests closed without merging"))
        .child(item("commits", or_wait(repos_in, compact(commits)), "Commits on default branches"))
        .child(item("lines", or_wait(repos_in, tf!("+{} −{}", compact(add), compact(del))), "Lines added / removed by those commits (latest 300 per repo)"))
        .child(item(
            "deploys",
            or_wait(repos_in, tf!("{deploys}{}{}", if capped { "+" } else { "" }, if deploy_fail > 0 { tf!(" ({deploy_fail} failed)") } else { String::new() })),
            "Deployments, and how many failed",
        ))
        .child(item("releases", or_wait(repos_in, releases.to_string()), "Releases published"))
        .child(item("contributors", or_wait(repos_in && prs_in, people.to_string()), "People who committed or merged"))
        .children((p.searches_left > 0 || p.repos.iter().any(|r| !r.detail_ready)).then(|| row().pl(12.0).gap(6.0).center_y().child(Spinner::new().size(10.0).build(k)).child(muted(k, "loading…"))))
}

// ---------------------------------------------------------------------------------------------
// One repo: a summary row, and its details when opened
// ---------------------------------------------------------------------------------------------

fn section_key(slug: &str) -> String {
    tf!("sec-{}", slug.replace('/', "-"))
}

fn repo_section(s: &State, k: &Kit, p: &Pulse, r: &RepoPulse) -> El {
    let t = now();
    let open_here = p.open_in(Some(&r.slug));
    let needs_review = open_here.iter().filter(|x| x.ready_for_review()).count();
    let failing_prs = open_here.iter().filter(|x| x.failing()).count();
    let (rate, failed, running) = r.health();
    let red = r.red_now();
    let expanded = s.pulse_open_repos.contains(&r.slug);
    let (slug, slug2) = (r.slug.clone(), r.slug.clone());

    // The summary row: the two things that matter, readable at a glance.
    let summary = row()
        .w_full()
        .gap(14.0)
        .center_y()
        .flex_wrap()
        .hover(k.c.hover)
        .radius(k.radius.md)
        .help(crate::i18n::t(if expanded { "Hide details" } else { "Show details" }))
        .on_press(move |s: &mut State| {
            if !s.pulse_open_repos.remove(&slug) {
                s.pulse_open_repos.insert(slug.clone());
            }
        })
        .child(rune::icon(if expanded { &glyphs::TRIANGLE_DOWN } else { &glyphs::TRIANGLE_RIGHT }, 10.0).color(k.c.icon_faint))
        .child(repo_name(s, k, &r.slug, 230.0))
        .child(
            row()
                .min_w(250.0)
                .gap(8.0)
                .center_y()
                .child(value_text(if r.detail_ready { tf!("{} open", compact(r.prs_open)) } else { "… open".to_owned() }).font(k.ty.ui_medium).color(k.c.text_base))
                .child(if needs_review > 0 { chip(k, tf!("{needs_review} ready for review"), Some(Outcome::Success)).help(crate::i18n::t("Waiting on a reviewer with all checks passed")) } else { row() })
                .child(if failing_prs > 0 { chip(k, tf!("{failing_prs} checks failing"), Some(Outcome::Failure)) } else { row() }),
        )
        .child(
            row()
                .min_w(230.0)
                .gap(8.0)
                .center_y()
                .child(el().size(9.0, 9.0).radius(4.5).fill(rate.map_or(k.c.icon_faint, |x| rate_color(k, x))))
                .child(value_text(if !r.runs_ready { "… passed".to_owned() } else { rate.map_or("no runs".to_owned(), |x| tf!("{:.0}% passed", x * 100.0)) }).font(k.ty.ui_medium).color(k.c.text_base))
                .child(if !red.is_empty() { chip(k, tf!("{} red now", red.len()), Some(Outcome::Failure)) } else if failed > 0 { muted(k, tf!("{failed} failed")) } else { row() }),
        )
        .child(spacer())
        .children((!r.detail_ready || !r.runs_ready).then(|| row().center_y().child(Spinner::new().size(12.0).build(k)).help(crate::i18n::t("This repo's numbers are still coming in"))))
        .child(
            Button::new(crate::i18n::t("Open repo"))
                .variant(ButtonVariant::Outline)
                .help(crate::i18n::t("Its pull requests, Actions, environments, commits and branches"))
                .on_press(move |s: &mut State| s.select_repo(&slug2, Page::Pulls))
                .build(k)
                .key("open"),
        );
    let sec = col()
        .key(section_key(&r.slug))
        .w_full()
        .gap(12.0)
        .padding(12.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, if red.is_empty() { k.c.border_weak } else { k.c.danger.icon })
        .child(summary);
    if !expanded {
        return sec;
    }

    // 1. What needs attention, in plain words (or that nothing does).
    let mut attention: Vec<(Outcome, String, Option<i64>, Option<(String, u64)>)> = Vec::new();
    for (wf, at, id) in &red {
        attention.push((Outcome::Failure, tf!("{wf} is failing on {}", r.default_branch), Some(*at), Some((wf.clone(), *id))));
    }
    if needs_review > 0 {
        attention.push((Outcome::Success, tf!("{needs_review} pull request{} ready for review (all checks passed)", plural(needs_review)), None, None));
    }
    if failing_prs > 0 {
        attention.push((Outcome::Failure, tf!("{failing_prs} pull request{} with failing checks", plural(failing_prs)), None, None));
    }
    if running > 0 {
        attention.push((Outcome::Running, tf!("{running} workflow run{} in progress", plural(running)), None, None));
    }
    let mut first = col().key("attention").w_full().gap(6.0).pb(4.0);
    if attention.is_empty() {
        first = first.child(line(k, Outcome::Success, "Nothing needs attention: checks are green and no reviews are waiting.".into(), None, t));
    }
    for (i, (o, what, at, run)) in attention.into_iter().take(4).enumerate() {
        let mut l = line(k, o, what, at, t).key(tf!("att-{i}"));
        if let Some((wf, id)) = run {
            let slug = r.slug.clone();
            l = l.radius(k.radius.sm).hover(k.c.hover).help(crate::i18n::t("Open this failed run")).on_press(move |s: &mut State| open_run_or_failed(s, &slug, id, &wf));
        }
        first = first.child(l);
    }

    // 2. One line per topic; click for its details.
    let (a, d) = r.lines();
    let people = p.contributors_for(Some(&r.slug));
    let envs = r.environments();
    let failed_deploys: usize = envs.iter().map(|e| e.2).sum();
    // The one or two numbers that matter most in each topic, as coloured badges.
    let mut pr_badges = Vec::new();
    if needs_review > 0 {
        pr_badges.push(chip(k, tf!("{needs_review} ready for review"), Some(Outcome::Success)));
    }
    if failing_prs > 0 {
        pr_badges.push(chip(k, tf!("{failing_prs} failing checks"), Some(Outcome::Failure)));
    }
    if pr_badges.is_empty() {
        pr_badges.push(chip(k, if r.prs_open == 0 { "none open".to_owned() } else { tf!("{} open", compact(r.prs_open)) }, None));
    }
    let mut ci_badges = Vec::new();
    match rate {
        Some(x) => {
            let tone = if x >= 0.95 { Outcome::Success } else if x >= 0.8 { Outcome::Running } else { Outcome::Failure };
            ci_badges.push(chip(k, tf!("{:.0}% passed", x * 100.0), Some(tone)));
        }
        None => ci_badges.push(chip(k, "no runs", None)),
    }
    if !red.is_empty() {
        ci_badges.push(chip(k, tf!("{} red now", red.len()), Some(Outcome::Failure)).help(crate::i18n::t("Workflows whose latest run on the default branch failed")));
    }
    let code_badges = vec![chip(k, tf!("{} commits", compact(r.commits_total)), if r.commits_total > 0 { Some(Outcome::Neutral) } else { None })];
    let mut deploy_badges = Vec::new();
    if failed_deploys > 0 {
        deploy_badges.push(chip(k, tf!("{failed_deploys} failed"), Some(Outcome::Failure)));
    } else if !r.deploys.is_empty() {
        deploy_badges.push(chip(k, "all succeeded", Some(Outcome::Success)));
    }
    if !r.releases.is_empty() {
        deploy_badges.push(chip(k, tf!("{} release{}", r.releases.len(), plural(r.releases.len())), None));
    }

    let topics = col()
        .key("topics")
        .w_full()
        .gap(2.0)
        .child(topic(
            s,
            k,
            &r.slug,
            "prs",
            "Pull requests",
            pr_badges,
            tf!("{} open · {} opened, {} merged, {} closed in this range", compact(r.prs_open), compact(r.prs_opened), compact(r.prs_merged), compact(r.prs_closed)),
            &|| {
                let mut c = col().w_full().gap(4.0);
                if open_here.is_empty() {
                    c = c.child(muted(k, "No open pull requests"));
                }
                for (i, x) in open_here.iter().take(8).enumerate() {
                    c = c.child(open_pr_row(k, x, &tf!("op-{i}"), false, true));
                }
                if open_here.len() > 8 {
                    c = c.child(muted(k, tf!("…and {} more: Open repo to see all", open_here.len() - 8)));
                }
                c
            },
        ))
        .child(topic(
            s,
            k,
            &r.slug,
            "ci",
            "Actions",
            ci_badges,
            match rate {
                Some(_) => tf!("{} runs · {failed} failed · median {}", compact(r.runs_total), crate::model::duration(r.median_secs())),
                None => "No runs in this range".into(),
            },
            &|| {
                let mut c = col().w_full().gap(3.0).child(muted(k, "Each workflow in this range, the ones with failures first. Click one to open its latest failed run (or latest run)."));
                for (i, w) in r.workflows().iter().take(8).enumerate() {
                    let tone = if w.failed == 0 { k.c.success.icon } else if (w.failed as f32 / w.runs.max(1) as f32) < 0.1 { k.c.warning.icon } else { k.c.danger.icon };
                    let (sl, wf, id) = (r.slug.clone(), w.name.clone(), w.focus_run);
                    c = c.child(
                        row()
                            .key(tf!("w-{i}"))
                            .w_full()
                            .gap(8.0)
                            .px(4.0)
                            .py(3.0)
                            .radius(k.radius.sm)
                            .center_y()
                            .hover(k.c.hover)
                            .help(crate::i18n::t(tf!("{}: {} runs, {} failed, average {}", w.name, w.runs, w.failed, crate::model::duration(w.secs / w.runs.max(1) as i64))))
                            .on_press(move |s: &mut State| open_run_or_failed(s, &sl, id, &wf))
                            .child(el().size(8.0, 8.0).radius(4.0).fill(tone))
                            .child(text(w.name.clone()).font(k.ty.small).color(k.c.text_base).truncate().grow(1.0).shrink())
                            .child(muted(k, if w.failed > 0 { tf!("{} of {} failed", w.failed, w.runs) } else { tf!("{} runs, all passed", w.runs) })),
                    );
                }
                c
            },
        ))
        .child(topic(
            s,
            k,
            &r.slug,
            "code",
            "Code changes",
            code_badges,
            tf!("on {} · +{} −{} lines · {} contributor{}", r.default_branch, compact(a), compact(d), people.len(), plural(people.len())),
            &|| {
                if people.is_empty() {
                    muted(k, "No commits on the default branch in this range")
                } else {
                    col().w_full().gap(4.0).child(muted(k, "Who changed it (share of lines changed)")).child(contributor_rows(k, &people, 6))
                }
            },
        ));
    let topics = if r.deploys.is_empty() && r.releases.is_empty() {
        topics
    } else {
        topics.child(topic(
            s,
            k,
            &r.slug,
            "deploys",
            "Deployments & releases",
            deploy_badges,
            tf!(
                "{}{} deploys to {} environment{} · {} release{}",
                r.deploys.len(),
                if r.deploys_capped { "+" } else { "" },
                envs.len(),
                plural(envs.len()),
                r.releases.len(),
                plural(r.releases.len())
            ),
            &|| {
                let mut c = col().w_full().gap(4.0);
                for (i, (env, n, f, last, at)) in envs.iter().take(6).enumerate() {
                    let o = match last.as_str() {
                        "SUCCESS" | "ACTIVE" => Outcome::Success,
                        "FAILURE" | "ERROR" => Outcome::Failure,
                        "INACTIVE" | "DESTROYED" => Outcome::Neutral,
                        _ => Outcome::Running,
                    };
                    let what = tf!("{env}: {n} deploy{}{}", plural(*n), if *f > 0 { tf!(", {f} failed") } else { String::new() });
                    c = c.child(line(k, o, what, Some(*at), t).key(tf!("env-{i}")));
                }
                for (i, (tag, at)) in r.releases.iter().take(4).enumerate() {
                    c = c.child(line(k, Outcome::Neutral, tf!("Released {tag}"), Some(*at), t).key(tf!("rel-{i}")));
                }
                c
            },
        ))
    };

    sec.child(col().key("detail").w_full().gap(8.0).pb(4.0).child(first).child(el().w_full().h(1.0).fill(k.c.border_weak)).child(topics))
}

/// Open run `id` (with its row revealed), or, for data cached before runs kept their ids, the
/// latest failed run of `workflow`.
fn open_run_or_failed(s: &mut State, slug: &str, id: u64, workflow: &str) {
    if id != 0 {
        s.open_run(slug, id);
    } else {
        s.open_latest_failed(slug, Some(workflow.to_owned()));
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// One plain sentence with a status dot, and when it happened.
fn line(k: &Kit, o: Outcome, what: String, at: Option<i64>, now: i64) -> El {
    let mut r = row()
        .w_full()
        .gap(GRID_GAP)
        .px(GRID_PAD)
        .center_y()
        .child(lead(el().size(8.0, 8.0).radius(4.0).fill(outcome_color(k, o))))
        .child(text(what).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink());
    if let Some(at) = at {
        r = r.child(muted(k, ago(at, now)));
    }
    r
}

// A repo detail's rows share one grid, so dots, arrows, titles, badges and text line up:
// [lead][title][badges][headline], and opened details start under the title.
const GRID_PAD: f32 = 8.0;
const GRID_GAP: f32 = 10.0;
const LEAD_W: f32 = 14.0;
const TITLE_W: f32 = 180.0;
const BADGES_W: f32 = 280.0;

/// The first column: a status dot or the open/close arrow, centred.
fn lead(e: El) -> El {
    row().w(LEAD_W).center().child(e)
}

/// A topic of a repo's detail: badges for what matters most, a muted headline for the rest,
/// and its details on click.
#[allow(clippy::too_many_arguments)]
fn topic(s: &State, k: &Kit, slug: &str, id: &str, title: &str, badges: Vec<El>, headline: String, details: &dyn Fn() -> El) -> El {
    let key = tf!("{slug}|{id}");
    let open = s.pulse_topics.contains(&key);
    let toggle = key.clone();
    let head = row()
        .key(tf!("t-{id}"))
        .w_full()
        .gap(GRID_GAP)
        .px(GRID_PAD)
        .py(7.0)
        .radius(k.radius.md)
        .center_y()
        .hover(k.c.hover)
        .role("button")
        .label(crate::i18n::t(title.to_owned()))
        .help(crate::i18n::t(if open { tf!("Hide {}", title.to_lowercase()) } else { tf!("Show {}", title.to_lowercase()) }))
        .on_press(move |s: &mut State| {
            if !s.pulse_topics.remove(&toggle) {
                s.pulse_topics.insert(toggle.clone());
            }
        })
        .child(lead(rune::icon(if open { &glyphs::TRIANGLE_DOWN } else { &glyphs::TRIANGLE_RIGHT }, 9.0).color(k.c.icon_faint)))
        .child(text(title.to_owned()).font(k.ty.ui_medium).color(k.c.text_base).w(TITLE_W).truncate())
        .child(badges.into_iter().fold(row().w(BADGES_W).gap(6.0).center_y(), |r, b| r.child(b)))
        .child(text(headline).font(k.ty.small).color(k.c.text_muted).truncate().grow(1.0).shrink());
    let mut c = col().key(tf!("topic-{id}")).w_full().child(head);
    if open {
        // Details start under the title.
        c = c.child(col().w_full().pl(GRID_PAD + LEAD_W + GRID_GAP).pr(GRID_PAD).pt(4.0).pb(10.0).child(details()));
    }
    c
}

// ---------------------------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------------------------


fn repo_name(s: &State, k: &Kit, slug: &str, w: f32) -> El {
    let (owner, name) = slug.split_once('/').unwrap_or(("", slug));
    let mut r = row().w(w).gap(6.0).center_y();
    if let Some(b) = super::org::badge(s, k, owner, 16.0) {
        r = r.child(b);
    }
    r.child(text(name.to_owned()).font(k.ty.ui_medium).color(k.c.text_base).truncate().shrink()).help(crate::i18n::t(slug.to_owned()))
}


fn day_legend(k: &Kit) -> El {
    row()
        .gap(12.0)
        .child(row().gap(5.0).center_y().child(el().size(8.0, 8.0).radius(2.0).fill(hue(0))).child(muted(k, "commits")))
        .child(row().gap(5.0).center_y().child(el().size(8.0, 8.0).radius(2.0).fill(hue(1))).child(muted(k, "PRs merged")))
}

fn day_bars(k: &Kit, days: &[(String, u64, u64)], h: f32) -> El {
    if days.is_empty() {
        return empty(k, "No days in range");
    }
    let peak = days.iter().map(|d| d.1.max(d.2)).max().unwrap_or(1).max(1);
    let small = h < 80.0;
    let long = days.len() > 10;
    let mut bars = row().w_full().gap(if small || long { 3.0 } else { 6.0 }).items_end();
    for (i, (label, c, m)) in days.iter().enumerate() {
        let bar = |n: u64, color| el().w(0.0).grow(1.0).max_w(if small { 7.0 } else { 14.0 }).h((n as f32 / peak as f32 * h).max(if n > 0 { 3.0 } else { 1.0 })).radius(2.0).fill(color);
        let mut day = col()
            .key(tf!("day-{i}"))
            .w(0.0)
            .grow(1.0)
            .gap(4.0)
            .items_center()
            .help(crate::i18n::t(tf!("{label}: {c} commits, {m} PRs merged")))
            .child(row().w_full().max_w(if small { 16.0 } else { 30.0 }).h(h).gap(if long { 1.0 } else { 2.0 }).items_end().justify_center().child(bar(*c, hue(0))).child(bar(*m, hue(1))));
        // Short ranges label every day; long ones every few. Every day keeps the slot, so the
        // bars share one baseline.
        day = day.child(day_label(k, label, !long || i % 5 == 0));
        bars = bars.child(day);
    }
    bars
}

/// A day's label under its bar, or the same-sized empty slot when the range is too long to
/// label every day. Labels may be wider than their bar, so they don't widen the column.
fn day_label(k: &Kit, label: &str, shown: bool) -> El {
    let slot = row().h(14.0).w(0.0).center();
    if shown { slot.child(text(crate::i18n::dates(label)).font(k.ty.text_xsmall).color(k.c.text_faint)) } else { slot }
}

fn contributor_rows(k: &Kit, people: &[crate::pulse::Contributor], top: usize) -> El {
    let total: u64 = people.iter().map(|c| c.additions + c.deletions).sum::<u64>().max(1);
    let mut list = col().w_full().gap(6.0);
    if people.is_empty() {
        return list.child(muted(k, "Nobody committed or merged in this range"));
    }
    for (i, c) in people.iter().take(top).enumerate() {
        let share = (c.additions + c.deletions) as f32 / total as f32;
        list = list.child(
            row()
                .key(tf!("c-{i}"))
                .w_full()
                .gap(10.0)
                .center_y()
                .help(crate::i18n::t(tf!("{}: {} commits, +{} −{}, {} PRs merged, in {}", c.name, c.commits, c.additions, c.deletions, c.prs_merged, c.repos.join(", "))))
                .child(text(c.name.clone()).font(k.ty.ui_medium).color(k.c.text_base).w(130.0).truncate())
                .child(row().grow(1.0).h(10.0).radius(3.0).fill(k.c.surface_base).child(el().w_frac(share.max(0.004)).h_full().radius(3.0).fill(hue(i))))
                .child(text(tf!("{:.0}%", share * 100.0)).font(k.ty.small_medium).color(k.c.text_base).w(40.0))
                .child(muted(k, tf!("{} commits · {} PRs", c.commits, c.prs_merged)).w(140.0)),
        );
    }
    if people.len() > top {
        list = list.child(muted(k, tf!("…and {} more", people.len() - top)));
    }
    list
}

/// Choose which repos the page follows.
pub fn tracked_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    if !s.tracked_open {
        return;
    }
    let tracked = s.tracked();
    let mut list = col().w_full().gap(2.0);
    if tracked.is_empty() {
        list = list.child(muted(k, "Nothing yet. Add repos below, or pin repos / make groups (those are tracked by default)."));
    }
    for (i, slug) in tracked.iter().enumerate() {
        let sl = slug.clone();
        let mut r = row().key(tf!("t-{i}")).w_full().gap(8.0).px(6.0).py(3.0).center_y();
        if let Some(b) = super::org::badge(s, k, slug.split('/').next().unwrap_or(""), 16.0) {
            r = r.child(b);
        }
        list = list.child(
            r.child(text(slug.clone()).font(k.ty.ui).color(k.c.text_base).grow(1.0).truncate().shrink()).child(
                IconButton::new(&glyphs::CLOSE).variant(IconButtonVariant::GhostMuted).label(crate::i18n::t("Stop tracking")).help(crate::i18n::t("Stop tracking (Undo brings it back)")).on_press(move |s: &mut State| s.toggle_tracked(&sl)).build(k),
            ),
        );
    }
    // Add: matching repos not tracked yet.
    let q = s.tracked_q.text.trim().to_lowercase();
    let mut add = col().w_full().gap(2.0);
    if !q.is_empty() {
        for (i, r) in s.repos.iter().filter(|r| !tracked.contains(&r.slug) && r.slug.to_lowercase().contains(&q)).take(6).enumerate() {
            let sl = r.slug.clone();
            add = add.child(
                row()
                    .key(tf!("a-{i}"))
                    .w_full()
                    .gap(8.0)
                    .px(6.0)
                    .py(3.0)
                    .center_y()
                    .child(text(r.slug.clone()).font(k.ty.ui).color(k.c.text_base).grow(1.0).truncate().shrink())
                    .child(Button::new(crate::i18n::t("Track")).variant(ButtonVariant::Outline).on_press(move |s: &mut State| s.toggle_tracked(&sl)).build(k).key("add")),
            );
        }
    }
    let search = TextInput::new(&s.tracked_q).placeholder(crate::i18n::t("Add a repo: type to search")).leading_icon(k.glyph.search).width(None).on_edit(|s: &mut State| &mut s.tracked_q);
    let search = super::with_clear(search, &s.tracked_q.text, |s| &mut s.tracked_q, |_| {}).build(k).key("track-q");
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(12.0)
        .child(muted(k, if s.prefs.tracked.is_empty() { "Following your pinned repos and your groups' repos (the default). Changing the list makes it your own." } else { "Your own list. Right-click any repo in the sidebar → Track in Activity also works." }).wrap().w_full())
        .child(col().w_full().max_h(280.0).child(list).scroll(rune::ScrollSpec { stick_bottom: false }))
        .child(search)
        .child(add)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(Button::new(crate::i18n::t("Reset to pinned + groups")).variant(ButtonVariant::GhostMuted).disabled(s.prefs.tracked.is_empty()).on_press(|s: &mut State| s.reset_tracked()).build(k).key("t-reset"))
                .child(spacer())
                .child(Button::new(crate::i18n::t("Done")).variant(ButtonVariant::Submit).on_press(|s: &mut State| s.tracked_open = false).build(k).key("t-done")),
        );
    Dialog::new("tracked", crate::i18n::t(tf!("Tracked repos ({})", tracked.len()))).open(true).fit().width(520.0).on_close(|s: &mut State| s.tracked_open = false).child(body).build(k, layer);
}
