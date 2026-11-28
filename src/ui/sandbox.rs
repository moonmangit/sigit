//! The Sandbox page (sidebar 2): start, enter, leave, reset or delete the current repo's
//! practice copy (`sandbox.rs`), and the banner every repo page shows while it's on.

use rune::{El, Icon, col, el, row, spacer};
use rune_kit::{Button, ButtonVariant, Kit, Spinner};

use super::muted;
use crate::State;
use crate::i18n::text;
use crate::model::{ago, now};
use crate::sandbox::View;

pub static I_SANDBOX: Icon = Icon {
    name: "sb-sandbox",
    view: 16.0,
    body: r#"<path d="M2 12.5H14" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><path d="M3.5 12.5C4 9.5 6 8 8 8S12 9.5 12.5 12.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 8V3.5L11 4.6L8 5.7" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#,
};
static I_CLONE: Icon = Icon {
    name: "sb-clone",
    view: 16.0,
    body: r#"<rect x="3" y="2.5" width="10" height="11" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M5.5 5.5H10.5M5.5 8H10.5M5.5 10.5H8.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#,
};
static I_ARROW: Icon = Icon { name: "sb-arrow", view: 16.0, body: r#"<path d="M2.5 8H13M9.5 4.5L13 8L9.5 11.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static I_YES: Icon = Icon { name: "sb-yes", view: 16.0, body: r#"<path d="M3.5 8.5L6.5 11.5L12.5 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
static I_NO: Icon = Icon { name: "sb-no", view: 16.0, body: r#"<path d="M4.5 4.5L11.5 11.5M11.5 4.5L4.5 11.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>"# };

const BUSY: &str = "Making the sandbox…";

/// The page.
pub fn view(s: &State, k: &Kit) -> El {
    let slug = s.cur.clone().unwrap_or_default();
    let load = s.sandbox.get(&slug);
    let busy = s.busy.contains(&crate::i18n::t(BUSY));
    let branch = s.rd().and_then(|d| d.status.data.as_ref()).map(|st| st.branch.clone()).unwrap_or_default();
    let body = match load.and_then(|l| l.data.as_ref()) {
        _ if busy => making(k),
        Some(Some(v)) => present(s, k, v),
        Some(None) => absent(s, k, &branch),
        None => row().w_full().py(40.0).center().child(Spinner::new().size(18.0).build(k)),
    };
    col().key("sandbox-page").w_full().h_full().max_w(760.0).gap(14.0).child(body).child(how(s, k))
}

fn making(k: &Kit) -> El {
    row().key("sb-making").w_full().py(40.0).gap(10.0).center().child(Spinner::new().size(16.0).build(k)).child(muted(k, crate::i18n::t(BUSY)))
}

/// A box in the "your clone → sandbox" picture.
fn tile(k: &Kit, key: &str, icon: &'static Icon, title: String, sub: String, strong: bool) -> El {
    let (fill, line) = if strong { (k.c.warning.bg, k.c.warning.border) } else { (k.c.surface_base, k.c.border_weak) };
    col()
        .key(key.to_owned())
        .w(200.0)
        .padding(12.0)
        .gap(4.0)
        .radius(k.radius.md)
        .fill(fill)
        .border(1.0, line)
        .child(row().gap(6.0).center_y().child(rune::icon(icon, 15.0).color(if strong { k.c.warning.icon } else { k.c.icon_base })).child(text(title).font(k.ty.ui_medium).color(k.c.text_strong)))
        .child(text(sub).font(k.ty.small).color(k.c.text_muted).wrap())
}

/// Your clone → copy → sandbox, with what the sandbox can and can't do.
fn picture(k: &Kit, branch: &str) -> El {
    let arrow = col()
        .gap(2.0)
        .items_center()
        .child(rune::icon(&I_ARROW, 18.0).color(k.c.icon_muted))
        .child(text(crate::i18n::t("copy")).font(k.ty.small).color(k.c.text_faint));
    row()
        .key("sb-picture")
        .gap(12.0)
        .center_y()
        .flex_wrap()
        .child(tile(k, "sb-real", &I_CLONE, crate::i18n::t("Your clone"), crate::i18n::t(tf!("{branch}: stays as it is")), false))
        .child(arrow)
        .child(tile(k, "sb-copy", &I_SANDBOX, crate::i18n::t("Sandbox"), crate::i18n::t("Your commits and branches. Edit anything."), true))
}

fn can(k: &Kit, yes: bool, label: &str) -> El {
    let (icon, color) = if yes { (&I_YES, k.c.success.icon) } else { (&I_NO, k.c.danger.icon) };
    row().gap(5.0).center_y().child(rune::icon(icon, 13.0).color(color)).child(text(crate::i18n::t(label)).font(k.ty.small).color(k.c.text_muted))
}

fn cans(k: &Kit) -> El {
    row()
        .key("sb-cans")
        .gap(16.0)
        .flex_wrap()
        .child(can(k, true, "Reset, revert, reword, squash, drop…"))
        .child(can(k, true, "Undo"))
        .child(can(k, true, "Fetch and pull"))
        .child(can(k, false, "Push"))
        .child(can(k, false, "Changes on GitHub"))
}

/// No sandbox yet.
fn absent(s: &State, k: &Kit, branch: &str) -> El {
    let branch = if branch.is_empty() { crate::i18n::t("your branch") } else { branch.to_owned() };
    col()
        .key("sb-absent")
        .w_full()
        .gap(14.0)
        .pt(8.0)
        .child(text(crate::i18n::t(tf!("Practise on a copy of {branch}"))).font(rune::TextStyle { size: 17.0, ..k.ty.ui_medium }).color(k.c.text_strong))
        .child(muted(k, crate::i18n::t("Try any history edit on your real commits. Your real clone and GitHub don't change.")))
        .child(picture(k, &branch))
        .child(cans(k))
        .child(
            row().pt(4.0).child(
                Button::new(crate::i18n::t("Start sandbox"))
                    .variant(ButtonVariant::Contrast)
                    .disabled(s.repo().is_none_or(|r| r.path.is_none()))
                    .on_press(|s: &mut State| s.start_sandbox())
                    .build(k)
                    .key("sb-start")
                    .help(crate::i18n::t("Copies the commits of your clone (not uncommitted files) into a private folder, and switches the Commits and Branches pages to it.")),
            ),
        )
}

/// "1.2 MB"
fn bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{} KB", n >> 10),
        n => format!("{n} B"),
    }
}

fn stat(k: &Kit, key: &str, value: String, label: &str, color: rune::Color) -> El {
    col()
        .key(key.to_owned())
        .gap(2.0)
        .min_w(90.0)
        .child(text(value).font(rune::TextStyle { size: 17.0, ..k.ty.ui_medium }).color(color))
        .child(text(crate::i18n::t(label)).font(k.ty.small).color(k.c.text_faint))
}

/// There is a sandbox: its state and controls.
fn present(s: &State, k: &Kit, v: &View) -> El {
    let on = s.in_sandbox();
    let Some(info) = v.info.as_ref() else { return el() };
    let short = |sha: &str| sha.chars().take(7).collect::<String>();
    let status = if on {
        row().gap(5.0).center_y().px(8.0).py(3.0).radius(k.radius.sm).fill(k.c.warning.bg).child(rune::icon(&I_SANDBOX, 13.0).color(k.c.warning.icon)).child(text(crate::i18n::t("On")).font(k.ty.small_medium).color(k.c.warning.fg))
    } else {
        row().gap(5.0).center_y().px(8.0).py(3.0).radius(k.radius.sm).fill(k.c.surface_base).child(text(crate::i18n::t("Off: pages show your real clone")).font(k.ty.small_medium).color(k.c.text_muted))
    };
    let head = row()
        .key("sb-head")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(rune::icon(&I_SANDBOX, 18.0).color(k.c.warning.icon))
        .child(text(crate::i18n::t(tf!("Sandbox of {}", info.branch))).font(rune::TextStyle { size: 17.0, ..k.ty.ui_medium }).color(k.c.text_strong))
        .child(status)
        .child(spacer());
    let stats = row()
        .key("sb-stats")
        .gap(24.0)
        .flex_wrap()
        .child(stat(k, "sb-new", format!("+{}", v.new), "new or remade commits", if v.new > 0 { k.c.success.fg } else { k.c.text_base }))
        .child(stat(k, "sb-gone", format!("−{}", v.gone), "commits gone", if v.gone > 0 { k.c.danger.fg } else { k.c.text_base }))
        .child(stat(k, "sb-age", ago(info.made, now()), "made", k.c.text_base))
        .child(stat(k, "sb-size", bytes(v.size), "extra disk", k.c.text_base));
    let tips = row()
        .key("sb-tips")
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(muted(k, crate::i18n::t(tf!("Started from {} · now {}", short(&info.base), if v.tip.is_empty() { "—".to_owned() } else { short(&v.tip) }))))
        .children(s.cur.as_deref().and_then(|c| s.sandbox_dir_of(c)).map(|d| {
            let path = d.display().to_string();
            row().gap(4.0).center_y().child(muted(k, "·")).child(muted(k, path.clone()).help(crate::i18n::t("The sandbox folder: open it in a terminal or editor to change files, then stage them for Amend."))).child(super::copy_button(k, "sb-path", path, "folder path"))
        }));

    let mut buttons = row().key("sb-buttons").gap(8.0).flex_wrap();
    if on {
        buttons = buttons
            .child(Button::new(crate::i18n::t("Go to Commits")).variant(ButtonVariant::Contrast).on_press(|s: &mut State| s.show(crate::Page::Commits)).build(k).key("sb-commits"))
            .child(Button::new(crate::i18n::t("Leave sandbox")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.set_sandbox(false)).build(k).key("sb-leave"));
    } else {
        buttons = buttons.child(
            Button::new(crate::i18n::t("Enter sandbox"))
                .variant(ButtonVariant::Contrast)
                .on_press(|s: &mut State| s.set_sandbox(true))
                .build(k)
                .key("sb-enter")
                .help(crate::i18n::t("The Commits and Branches pages work on the sandbox until you leave it.")),
        );
    }
    buttons = buttons
        .child(Button::new(crate::i18n::t("Reset…")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.sandbox_ask = Some("reset")).build(k).key("sb-reset").help(crate::i18n::t("Throw the practice away and copy your real branch again.")))
        .child(Button::new(crate::i18n::t("Delete…")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.sandbox_ask = Some("delete")).build(k).key("sb-delete").help(crate::i18n::t("Remove the sandbox folder. Your real clone doesn't change.")));

    let mut c = col().key("sb-present").w_full().gap(14.0).pt(8.0).child(head).child(stats).child(tips);
    if v.real_moved() {
        c = c.child(note(k, "sb-moved", crate::i18n::t(tf!("Your real {} has changed since the sandbox started. Reset to practise on its new commits.", info.branch))));
    } else if !v.real_branch.is_empty() && v.real_branch != info.branch {
        c = c.child(note(k, "sb-other", crate::i18n::t(tf!("Your real clone is on {} now. Reset makes the sandbox from that branch.", v.real_branch))));
    }
    c = c.child(buttons);
    if let Some(what) = s.sandbox_ask {
        c = c.child(ask(k, what));
    }
    c.child(cans(k))
}

fn note(k: &Kit, key: &str, msg: String) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .gap(8.0)
        .padding(10.0)
        .radius(k.radius.md)
        .fill(k.c.warning.bg)
        .border(1.0, k.c.warning.border)
        .child(text(msg).font(k.ty.small).color(k.c.warning.fg).wrap().grow(1.0))
}

/// "Sure?" for reset and delete: the practice in the sandbox goes.
fn ask(k: &Kit, what: &'static str) -> El {
    let (msg, verb) = match what {
        "reset" => ("Reset the sandbox? Your practice edits go, and it starts again from your real branch.", "Reset sandbox"),
        _ => ("Delete the sandbox? Your practice edits go. Your real clone doesn't change.", "Delete sandbox"),
    };
    row()
        .key("sb-ask")
        .w_full()
        .gap(10.0)
        .padding(12.0)
        .center_y()
        .flex_wrap()
        .radius(k.radius.md)
        .fill(k.c.surface_base)
        .border(1.0, k.c.border_base)
        .child(text(crate::i18n::t(msg)).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).min_w(200.0))
        .child(
            Button::new(crate::i18n::t(verb))
                .variant(ButtonVariant::Danger)
                .on_press(move |s: &mut State| if what == "reset" { s.start_sandbox() } else { s.delete_sandbox() })
                .build(k)
                .key("sb-yes"),
        )
        .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::GhostMuted).on_press(|s: &mut State| s.sandbox_ask = None).build(k).key("sb-no"))
}

/// "How it works", folded.
fn how(s: &State, k: &Kit) -> El {
    let open = s.sandbox_how;
    let mut c = col().key("sb-how").w_full().gap(6.0).pt(6.0).child(
        row()
            .key("sb-how-row")
            .gap(6.0)
            .py(3.0)
            .center_y()
            .child(rune::icon(if open { &rune_kit::glyphs::TRIANGLE_DOWN } else { &rune_kit::glyphs::TRIANGLE_RIGHT }, 9.0).color(k.c.icon_faint))
            .child(text(crate::i18n::t("How it works")).font(k.ty.small_medium).color(k.c.text_muted))
            .on_press(|s: &mut State| s.sandbox_how = !s.sandbox_how),
    );
    if open {
        for line in [
            "The sandbox is a private copy of your local clone in sigit's settings folder. Git shares the commit files with your clone, so it uses little disk.",
            "It has your commits, your branches and your real remote branches (origin/…), so \"pushed\" and \"ahead / behind\" read the same. Uncommitted files are not copied.",
            "Pushing from the sandbox always fails. While it's on, sigit doesn't change GitHub: merge, close, approve, re-run and new pull requests are off.",
            "Every edit still shows the simulation, the risk and Undo, as on your real clone.",
            "The sandbox stays until you delete it. Reset copies your real branch again.",
        ] {
            c = c.child(row().gap(8.0).pl(15.0).child(text("·").font(k.ty.small).color(k.c.text_faint)).child(text(crate::i18n::t(line)).font(k.ty.small).color(k.c.text_muted).wrap().grow(1.0)));
        }
    }
    c
}

/// The strip on every repo page while the sandbox is on.
pub fn banner(s: &State, k: &Kit) -> Option<El> {
    if !s.in_sandbox() {
        return None;
    }
    let branch = s.cur.as_deref().and_then(|c| s.sandbox.get(c)).and_then(|l| l.data.clone().flatten()).and_then(|v| v.info).map(|i| i.branch).unwrap_or_default();
    Some(
        row()
            .key("sb-banner")
            .w_full()
            .gap(8.0)
            .px(12.0)
            .py(6.0)
            .center_y()
            .radius(k.radius.md)
            .fill(k.c.warning.bg)
            .border(1.0, k.c.warning.border)
            .child(rune::icon(&I_SANDBOX, 15.0).color(k.c.warning.icon))
            .child(text(crate::i18n::t(if branch.is_empty() { "Sandbox".to_owned() } else { tf!("Sandbox of {branch}") })).font(k.ty.ui_medium).color(k.c.warning.fg))
            .child(text(crate::i18n::t("Practice copy: your real clone and GitHub don't change.")).font(k.ty.small).color(k.c.warning.fg).wrap().grow(1.0).min_w(0.0))
            .child(Button::new(crate::i18n::t("Sandbox page")).variant(ButtonVariant::GhostMuted).on_press(|s: &mut State| s.show(crate::Page::Sandbox)).build(k).key("sb-banner-page"))
            .child(Button::new(crate::i18n::t("Leave sandbox")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.set_sandbox(false)).build(k).key("sb-banner-leave")),
    )
}
