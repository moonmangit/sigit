//! The setup screen (git or gh missing, or not signed in) and the first-run welcome. Both take
//! the window below the title bar; see `crate::onboarding` for the checks.

use rune::{El, TextStyle, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, Kit, SegmentedControl, Spinner};

use super::{muted, scroll};
use crate::State;
use crate::config::Theme;
use crate::onboarding::{LOGIN, Preflight, RECHECK_SECS, Setup, install_hint};

/// What the splash says, while there's nothing to show yet: the first launch (or one after
/// the cache was cleared) before the startup check and the repo list are back. With last
/// session's data the window opens on it instead.
pub fn splash_message(s: &State) -> Option<&'static str> {
    if s.inbox.data.is_some() {
        return None;
    }
    match s.setup {
        Setup::Checking => Some("Checking git and the GitHub CLI…"),
        Setup::Ready if s.inbox.loading && s.inbox.error.is_none() => Some("Loading your repositories…"),
        _ => None,
    }
}

/// The splash: the app mark, its name, and what it's doing.
pub fn splash(k: &Kit, message: &str) -> El {
    col()
        .key("splash")
        .w_full()
        .h_full()
        .center()
        .gap(16.0)
        .fill(k.c.page)
        .child(super::app_mark(88.0))
        .child(text("sigit").font(TextStyle { size: 30.0, ..k.ty.ui_medium }).color(k.c.text_strong))
        .child(row().gap(10.0).center_y().child(Spinner::new().size(14.0).build(k)).child(muted(k, message.to_owned())))
        .child(text(format!("Version {}", crate::update::current())).font(k.ty.text_xsmall).color(k.c.text_faint))
}

pub fn view(s: &State, k: &Kit) -> El {
    let card = match &s.setup {
        Setup::Missing(p) => setup(s, k, p),
        Setup::Welcome(p) => welcome(s, k, p),
        _ => el(),
    };
    scroll(
        "onboarding-scroll",
        row().key("onboarding").w_full().py(40.0).px(24.0).child(spacer()).child(col().w_full().max_w(660.0).gap(22.0).child(card)).child(spacer()),
    )
    .fill(k.c.page)
}

fn header(k: &Kit, title: &str, sub: &str) -> El {
    col()
        .w_full()
        .gap(10.0)
        .child(super::app_mark(48.0))
        .child(text(title.to_owned()).font(TextStyle { size: 26.0, ..k.ty.ui_medium }).color(k.c.text_strong))
        .child(muted(k, sub.to_owned()).wrap().w_full())
}

// ---------------------------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------------------------

fn setup(s: &State, k: &Kit, p: &Preflight) -> El {
    let gh_ok = p.gh.is_ok();
    let steps = col()
        .w_full()
        .gap(12.0)
        .child(step(k, 1, "Install Git", &p.git, true, install(k, "git")))
        .child(step(k, 2, "Install the GitHub CLI (gh)", &p.gh, true, install(k, "gh")))
        .child(step(
            k,
            3,
            "Sign in to GitHub",
            &p.auth,
            gh_ok,
            col()
                .w_full()
                .gap(8.0)
                .child(muted(k, "gh signs you in through your browser and keeps the token. sigit never sees it.").wrap().w_full())
                .child(command(k, "cmd-login", LOGIN, true)),
        ));
    let status = if s.setup_busy {
        row().gap(8.0).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, "Checking…"))
    } else {
        row().child(muted(k, format!("Checks again by itself every {RECHECK_SECS} s")))
    };
    let mut c = col()
        .w_full()
        .gap(22.0)
        .child(header(k, "Let's get sigit set up", "sigit works through the git and GitHub CLI tools you already use. It needs these three things on this computer:"))
        .child(steps)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(status)
                .child(spacer())
                .child(Button::new("Check now").variant(ButtonVariant::Submit).disabled(s.setup_busy).on_press(|s: &mut State| s.run_preflight()).build(k).key("check-now")),
        );
    if p.git.is_err() || p.gh.is_err() {
        c = c.child(muted(k, "Just installed something and it still says missing? Close and reopen sigit: it only sees programs that were on your PATH when it started.").wrap().w_full());
    }
    c
}

/// One numbered step: done (✓ and what was found) or still to do (why, and how to fix it).
fn step(k: &Kit, n: usize, title: &str, state: &Result<String, String>, enabled: bool, fix: El) -> El {
    let (mark, mark_bg, mark_fg) = match (state, enabled) {
        (Ok(_), _) => ("✓".to_owned(), k.c.success.bg, k.c.success.fg),
        (Err(_), true) => (n.to_string(), k.c.warning.bg, k.c.warning.fg),
        (Err(_), false) => (n.to_string(), k.c.surface_base, k.c.text_faint),
    };
    let mut body = col()
        .w(0.0)
        .grow(1.0)
        .gap(6.0)
        .child(text(title.to_owned()).font(k.ty.ui_medium).color(if enabled { k.c.text_strong } else { k.c.text_faint }));
    match state {
        Ok(found) => body = body.child(muted(k, found.clone())),
        Err(why) => {
            body = body.child(text(why.clone()).font(k.ty.small).color(if enabled { k.c.warning.fg } else { k.c.text_faint }).wrap().w_full());
            if enabled {
                body = body.child(fix);
            }
        }
    }
    row()
        .key(format!("step-{n}"))
        .w_full()
        .gap(14.0)
        .padding(16.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(1.0, if state.is_err() && enabled { k.c.warning.border } else { k.c.border_weak })
        .child(row().size(28.0, 28.0).radius(14.0).center().fill(mark_bg).child(text(mark).font(k.ty.small_medium).color(mark_fg)))
        .child(body)
}

/// How to install `tool`: the package-manager command, and the download page.
fn install(k: &Kit, tool: &'static str) -> El {
    let (cmd, url) = install_hint(tool);
    let mut c = col().w_full().gap(8.0);
    if let Some(cmd) = cmd {
        c = c.child(command(k, &format!("cmd-{tool}"), cmd, true));
    }
    c.child(
        row().child(
            Button::new(format!("Download from {}", url.trim_start_matches("https://")))
                .variant(ButtonVariant::Outline)
                .icon(&super::I_EXTERNAL)
                .on_press_tx(move |_: &mut State, tx| crate::open_url(tx, url))
                .build(k)
                .key(format!("dl-{tool}")),
        ),
    )
}

/// A command to run, with Copy and (where the app can open a terminal) Run.
fn command(k: &Kit, key: &str, cmd: &'static str, runnable: bool) -> El {
    let mut r = row()
        .key(key.to_owned())
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            row()
                .w(0.0)
                .grow(1.0)
                .px(12.0)
                .py(8.0)
                .radius(k.radius.md)
                .fill(k.c.surface_base)
                .border(1.0, k.c.border_weak)
                .child(text(cmd).font(k.ty.mono).color(k.c.text_base).truncate()),
        )
        .child(
            Button::new("Copy")
                .variant(ButtonVariant::Outline)
                .help("Copy the command")
                .on_press_tx(move |s: &mut State, tx| {
                    tx.copy(cmd);
                    s.toast(format!("Copied `{cmd}`"), false);
                })
                .build(k),
        );
    if runnable && (cfg!(windows) || cfg!(target_os = "macos")) {
        r = r.child(Button::new("Run in terminal").variant(ButtonVariant::Outline).help("Open a terminal window running it").on_press(move |s: &mut State| s.run_in_terminal(cmd)).build(k));
    }
    r
}

// ---------------------------------------------------------------------------------------------
// Welcome
// ---------------------------------------------------------------------------------------------

fn welcome(s: &State, k: &Kit, p: &Preflight) -> El {
    let who = p.auth.clone().unwrap_or_default();
    let feature = |key: &str, title: &str, what: &str| {
        col()
            .key(key.to_owned())
            .w(0.0)
            .grow(1.0)
            .min_w(180.0)
            .gap(6.0)
            .padding(16.0)
            .radius(k.radius.lg)
            .fill(k.c.layer_01)
            .border(1.0, k.c.border_weak)
            .child(text(title.to_owned()).font(k.ty.ui_medium).color(k.c.text_strong))
            .child(muted(k, what.to_owned()).wrap().w_full())
    };
    let features = row()
        .w_full()
        .gap(12.0)
        .flex_wrap()
        .child(feature("f-activity", "Activity", "Open PRs, which are ready for review, and how your Actions are doing, across the repos you track. This week by default."))
        .child(feature("f-needs", "Needs you", "Reviews waiting on you (ready ones first), your PRs, notifications. J / K to move, A to approve."))
        .child(feature("f-jump", "Ctrl+K", "Jump to any repo, pull request, branch or group, or run a command."));
    let theme_i = Theme::ALL.iter().position(|t| *t == s.prefs.theme);
    let theme = row()
        .w_full()
        .gap(12.0)
        .center_y()
        .child(col().w(0.0).grow(1.0).gap(2.0).child(text("Theme").font(k.ty.ui_medium).color(k.c.text_base)).child(muted(k, "Change it any time in Settings (Ctrl+,).")))
        .child(SegmentedControl::new(&Theme::ALL.map(|t| t.label()), theme_i).label("Theme").fit().on_pick(|s: &mut State, i, _| s.set_theme(Theme::ALL[i])).build(k).key("w-theme"));
    col()
        .w_full()
        .gap(22.0)
        .child(header(k, "You're all set", &format!("Signed in as {who}. Here's where to start:")))
        .child(features)
        .child(muted(k, "Activity follows your pinned repos and your groups: right-click any repo in the sidebar to pin it, add it to a group, or track it.").wrap().w_full())
        .child(theme)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .child(spacer())
                .child(Button::new("Take the tour").variant(ButtonVariant::Outline).shortcut("F1").on_press(|s: &mut State| s.finish_onboarding(true)).build(k).key("w-tour"))
                .child(Button::new("Get started").variant(ButtonVariant::Submit).on_press(|s: &mut State| s.finish_onboarding(false)).build(k).key("w-start")),
        )
}
