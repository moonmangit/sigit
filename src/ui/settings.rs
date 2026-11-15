//! The Settings window (Ctrl+, or the gear in the title bar): a category list on the left and
//! that category's settings on the right, each a row with what it does and its control, as in
//! GitHub Desktop's or Windows' settings.

use super::cmd::CmdTip;
use rune::{El, TextStyle, col, el, oklch, row, spacer};
use crate::i18n::text;
use rune_kit::{Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Kit, SegmentedControl, Spinner, Switch, TextArea, TextField, glyphs};

use super::{chip, muted, scroll};
use crate::State;
use crate::config::Theme;
use crate::model::{Outcome, ago, now};
use crate::risk::SENSITIVE;
use crate::settings::SECTIONS;

macro_rules! nav_icon {
    ($id:ident, $name:literal, $body:literal) => {
        static $id: rune::Icon = rune::Icon { name: $name, view: 16.0, body: $body };
    };
}
nav_icon!(N_LOOK, "s-look", r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 2.5A5.5 5.5 0 0 1 8 13.5Z" fill="currentColor"/>"#);
nav_icon!(N_USER, "s-user", r#"<circle cx="8" cy="5.5" r="2.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M3 13.5C3.6 11 5.6 9.7 8 9.7S12.4 11 13 13.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#);
nav_icon!(N_SHIELD, "s-shield", r#"<path d="M8 2L13 4V7.5C13 10.6 10.9 12.9 8 14C5.1 12.9 3 10.6 3 7.5V4Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
nav_icon!(N_BELL, "s-bell", r#"<path d="M4 11V7.5C4 5.3 5.8 3.5 8 3.5S12 5.3 12 7.5V11L13 12H3Z M6.5 13.5C6.8 14.1 7.4 14.5 8 14.5S9.2 14.1 9.5 13.5" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
nav_icon!(N_DATA, "s-data", r#"<ellipse cx="8" cy="4" rx="5" ry="1.8" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M3 4V12C3 13 5.2 13.8 8 13.8S13 13 13 12V4M3 8C3 9 5.2 9.8 8 9.8S13 9 13 8" stroke="currentColor" stroke-width="1.2" fill="none"/>"#);
nav_icon!(N_LOG, "s-log", r#"<path d="M5.5 4H13M5.5 8H13M5.5 12H13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><circle cx="3" cy="4" r="0.9" fill="currentColor"/><circle cx="3" cy="8" r="0.9" fill="currentColor"/><circle cx="3" cy="12" r="0.9" fill="currentColor"/>"#);
nav_icon!(N_KEYS, "s-keys", r#"<rect x="1.5" y="4" width="13" height="8.5" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4 7H4.5M7.5 7H8.5M11.5 7H12M5 10H11" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#);
nav_icon!(N_INFO, "s-info", r#"<circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 7.2V11.2" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="8" cy="5" r="0.8" fill="currentColor"/>"#);

/// One icon per entry of `SECTIONS`.
static NAV_ICONS: [&rune::Icon; 9] = [&N_LOOK, &N_USER, &super::I_LOGO, &N_SHIELD, &N_BELL, &N_DATA, &N_LOG, &N_KEYS, &N_INFO];

pub fn dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    if !s.settings_open {
        return;
    }
    let mut nav = col().key("set-nav").w(190.0).h_full().gap(2.0).py(4.0);
    for (i, name) in SECTIONS.iter().enumerate() {
        let on = i == s.settings_sec;
        nav = nav.child(
            row()
                .key(tf!("set-{i}"))
                .w_full()
                .px(10.0)
                .py(7.0)
                .gap(10.0)
                .center_y()
                .radius(k.radius.md)
                .fill(if on { k.c.surface_hover } else { rune::Color::TRANSPARENT })
                .hover(k.c.hover)
                .role("tab")
                .label(crate::i18n::t(*name))
                .on_press(move |s: &mut State| s.settings_sec = i)
                .child(rune::icon(NAV_ICONS[i], 15.0).color(if on { k.c.icon_base } else { k.c.icon_faint }))
                .child(text(*name).font(if on { k.ty.ui_medium } else { k.ty.ui }).color(if on { k.c.text_strong } else { k.c.text_base })),
        );
    }
    let content = match s.settings_sec {
        0 => appearance(s, k),
        1 => accounts(s, k),
        2 => git_identity(s, k),
        3 => safety(s, k),
        4 => notifications(s, k),
        5 => data(s, k),
        6 => activity_log(s, k),
        7 => shortcuts(k),
        _ => about(s, k),
    };
    let title = SECTIONS[s.settings_sec];
    let body = row()
        .key("settings")
        .w_full()
        .h(560.0)
        .pb(12.0)
        .child(nav)
        .child(el().w(1.0).h_full().fill(k.c.border_weak))
        .child(
            col().key(tf!("set-body-{}", s.settings_sec)).w(0.0).grow(1.0).h_full().child(scroll(
                "set-scroll",
                col().w_full().px(24.0).pb(24.0).gap(18.0).child(text(title).font(TextStyle { size: 20.0, ..k.ty.ui_medium }).color(k.c.text_strong)).child(content),
            )),
        );
    Dialog::new("settings-dialog", crate::i18n::t("Settings")).open(true).fit().width(920.0).on_close(|s: &mut State| s.close_settings()).child(body).build(k, layer);
}

// ---------------------------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------------------------

/// A titled block of setting rows.
fn group(k: &Kit, title: &str, rows: Vec<El>) -> El {
    let mut c = col().w_full().gap(1.0).radius(k.radius.lg).border(1.0, k.c.border_weak).clip_rounded(k.radius.lg);
    for r in rows {
        c = c.child(r);
    }
    col().w_full().gap(8.0).child(text(title.to_owned()).font(k.ty.small_medium).color(k.c.text_muted)).child(c)
}

/// One setting: its name and what it does on the left, its control on the right.
fn item(k: &Kit, key: &str, title: &str, desc: &str, control: El) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .gap(16.0)
        .px(16.0)
        .py(12.0)
        .center_y()
        .fill(k.c.layer_01)
        .child(
            col()
                .w(0.0)
                .grow(1.0)
                .gap(3.0)
                .child(text(title.to_owned()).font(k.ty.ui_medium).color(k.c.text_base))
                .children((!desc.is_empty()).then(|| muted(k, desc.to_owned()).wrap().w_full())),
        )
        .child(control)
}

/// A full-width block inside a group (a list, a text area).
fn block(k: &Kit, key: &str, content: El) -> El {
    col().key(key.to_owned()).w_full().px(16.0).py(12.0).gap(8.0).fill(k.c.layer_01).child(content)
}

fn switch(k: &Kit, label: &str, on: bool, flip: fn(&mut State)) -> El {
    Switch::new(on)
        .label(crate::i18n::t(label))
        .hide_label()
        .on_toggle(move |s: &mut State| {
            flip(s);
            s.save_prefs();
        })
        .build(k)
}

// ---------------------------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------------------------

fn appearance(s: &State, k: &Kit) -> El {
    let tiles = Theme::ALL.iter().fold(row().gap(14.0), |r, t| r.child(theme_tile(k, *t, s.prefs.theme == *t)));
    let zooms = [0.9f32, 1.0, 1.1, 1.25];
    let zi = zooms.iter().position(|z| (z - s.prefs.zoom).abs() < 0.01);
    let limits = [5usize, 7, 10, 0];
    let li = limits.iter().position(|l| *l == s.prefs.org_limit);
    col()
        .w_full()
        .gap(18.0)
        .child(group(k, "Theme", vec![block(k, "theme", col().gap(10.0).child(tiles).child(muted(k, "System follows Windows' light or dark setting.")))]))
        .child(group(
            k,
            "Layout",
            vec![
                item(
                    k,
                    "language",
                    "Language",
                    "The interface language. System follows your computer's preferred languages. GitHub's own text (titles, comments, names) stays as written.",
                    language_picker(s, k),
                ),
                item(
                    k,
                    "zoom",
                    "Text size",
                    "Scales text and controls everywhere in the app.",
                    SegmentedControl::new(&crate::i18n::all(&["90%", "100%", "110%", "125%"]), zi)
                        .label(crate::i18n::t("Text size"))
                        .fit()
                        .on_pick(move |s: &mut State, i, _| {
                            s.prefs.zoom = zooms[i];
                            s.save_prefs();
                        })
                        .build(k),
                ),
                item(
                    k,
                    "org-limit",
                    "Repositories per organization",
                    "How many of each organization's repos the sidebar lists before \"Show more\". The ones you opened recently come first; search always shows every match.",
                    SegmentedControl::new(&crate::i18n::all(&["5", "7", "10", "All"]), li)
                        .label(crate::i18n::t("Repositories per organization"))
                        .fit()
                        .on_pick(move |s: &mut State, i, _| {
                            s.prefs.org_limit = limits[i];
                            s.save_prefs();
                        })
                        .build(k),
                ),
                item(
                    k,
                    "reset-splits",
                    "Section sizes",
                    "Side-by-side sections you resized go back to their default widths.",
                    Button::new(crate::i18n::t("Reset")).variant(ButtonVariant::Outline).disabled(s.prefs.splits.is_empty()).on_press(|s: &mut State| s.reset_layout()).build(k),
                ),
            ],
        ))
}

/// A small picture of the app in that theme.
/// System / English / ไทย. English and Thai name themselves, so either can be found whatever the
/// interface is showing.
pub fn language_picker(s: &State, k: &Kit) -> El {
    use crate::config::Language;
    let i = Language::ALL.iter().position(|l| *l == s.prefs.language);
    let labels = Language::ALL.map(|l| if l == Language::System { crate::i18n::t(l.label()) } else { l.label().to_owned() });
    SegmentedControl::new(&labels, i)
        .label(crate::i18n::t("Language"))
        .fit()
        .on_pick(|s: &mut State, i, _| s.set_language(Language::ALL[i]))
        .build(k)
        .key("language")
}

fn theme_tile(k: &Kit, t: Theme, selected: bool) -> El {
    let shade = |dark: bool| {
        let (bg, side, bar) = if dark { (oklch(0.2, 0.0, 0.0), oklch(0.15, 0.0, 0.0), oklch(0.4, 0.0, 0.0)) } else { (oklch(0.99, 0.0, 0.0), oklch(0.94, 0.0, 0.0), oklch(0.82, 0.0, 0.0)) };
        row()
            .w(0.0)
            .grow(1.0)
            .h_full()
            .fill(bg)
            .child(col().w(26.0).h_full().fill(side).padding(5.0).gap(4.0).child(el().w_full().h(4.0).radius(2.0).fill(bar)).child(el().w_full().h(4.0).radius(2.0).fill(bar)))
            .child(col().w(0.0).grow(1.0).padding(7.0).gap(5.0).child(el().w_frac(0.7).h(6.0).radius(3.0).fill(bar)).child(el().w_full().h(18.0).radius(3.0).fill(side)))
    };
    let picture = match t {
        Theme::System => row().w_full().h_full().child(shade(false)).child(shade(true)),
        Theme::Light => row().w_full().h_full().child(shade(false)),
        Theme::Dark => row().w_full().h_full().child(shade(true)),
    };
    col()
        .key(tf!("theme-{}", t.label()))
        .gap(8.0)
        .role("radio")
        .label(crate::i18n::t(t.label()))
        .on_press(move |s: &mut State| s.set_theme(t))
        .child(
            // The ring is a padded frame around the picture (a border would be painted over by
            // the picture's own fill).
            col()
                .w(156.0)
                .h(96.0)
                .padding(3.0)
                .radius(11.0)
                .fill(if selected { k.c.text_accent } else { rune::Color::TRANSPARENT })
                .child(col().w_full().h_full().radius(8.0).clip_rounded(8.0).border(1.0, k.c.border_weak).hover(k.c.hover).child(picture)),
        )
        .child(text(if selected { tf!("✓ {}", crate::i18n::t(t.label())) } else { t.label().to_owned() }).font(if selected { k.ty.ui_medium } else { k.ty.ui }).color(if selected { k.c.text_accent } else { k.c.text_base }))
}

fn accounts(s: &State, k: &Kit) -> El {
    let mut rows = Vec::new();
    match (&s.accounts.data, &s.accounts.error) {
        (None, None) => rows.push(block(k, "acc-loading", row().gap(8.0).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, "Asking gh who you're signed in as…")))),
        (None, Some(e)) => rows.push(block(k, "acc-err", muted(k, tf!("gh couldn't say: {e}\nSign in with `gh auth login` in a terminal.")).wrap().w_full())),
        (Some(list), _) if list.is_empty() => rows.push(block(k, "acc-none", muted(k, "Not signed in. Run `gh auth login` in a terminal, then Refresh."))),
        (Some(list), _) => {
            for (i, a) in list.iter().enumerate() {
                let initial = a.login.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
                let mut right = row().gap(8.0).center_y();
                if a.active {
                    right = right.child(chip(k, "Active", Some(Outcome::Success)));
                } else {
                    let (host, login) = (a.host.clone(), a.login.clone());
                    right = right.child({ let tip = s.with_commands(crate::i18n::t("Make it the account gh (and so sigit) uses"), &[crate::gh::switch_account_cmd(&host, &login).line()]); Button::new(crate::i18n::t("Switch to this account")).variant(ButtonVariant::Outline).on_press(move |s: &mut State| s.switch_account(host.clone(), login.clone())).build(k).key(tf!("switch-{i}")).cmd_tip(s, tip) });
                }
                if !a.ok {
                    right = right.child(chip(k, "Sign-in expired", Some(Outcome::Failure)).help(crate::i18n::t("Run `gh auth refresh` (or `gh auth login`) in a terminal")));
                }
                let avatar = row().size(34.0, 34.0).radius(17.0).center().fill(super::hue(a.login.bytes().map(usize::from).sum())).child(text(initial).font(k.ty.ui_medium).color(rune::Color::WHITE));
                rows.push(
                    row()
                        .key(tf!("acc-{i}"))
                        .w_full()
                        .gap(12.0)
                        .px(16.0)
                        .py(12.0)
                        .center_y()
                        .fill(k.c.layer_01)
                        .child(avatar)
                        .child(
                            col()
                                .w(0.0)
                                .grow(1.0)
                                .gap(2.0)
                                .child(text(tf!("{} · {}", a.login, a.host)).font(k.ty.ui_medium).color(k.c.text_base))
                                .child(muted(k, tf!("{} · token in {} · scopes: {}", a.protocol.to_uppercase(), a.source, a.scopes)).wrap().w_full()),
                        )
                        .child(right),
                );
            }
        }
    }
    let actions = row()
        .gap(8.0)
        .child(Button::new(crate::i18n::t("Refresh")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.load_accounts()).build(k).key("acc-refresh"))
        .child(Button::new(crate::i18n::t("GitHub account settings")).variant(ButtonVariant::GhostMuted).icon(&super::I_EXTERNAL).on_press_tx(|_: &mut State, tx| crate::open_url(tx, "https://github.com/settings/profile")).build(k).key("acc-web"));
    col()
        .w_full()
        .gap(14.0)
        .child(muted(k, "sigit works through the GitHub CLI (gh), so it acts as the account gh uses. To add an account, run `gh auth login` in a terminal.").wrap().w_full())
        .child(group(k, "Signed in", rows))
        .child(actions)
}

fn git_identity(s: &State, k: &Kit) -> El {
    let changed = s.git_identity_changed();
    let fields = col()
        .w_full()
        .gap(12.0)
        .child(TextField::new("Name", &s.git_name).placeholder(crate::i18n::t("Your Name")).on_edit(|s: &mut State| &mut s.git_name).build(k).key("git-name"))
        .child(TextField::new("Email", &s.git_email).placeholder(crate::i18n::t("you@example.com")).on_edit(|s: &mut State| &mut s.git_email).build(k).key("git-email"))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(muted(k, if changed { "Not saved yet" } else { "Saved in your global git config" }))
                .child(spacer())
                .child(
                    Button::new(crate::i18n::t("Revert"))
                        .variant(ButtonVariant::Outline)
                        .disabled(!changed)
                        .on_press(|s: &mut State| {
                            s.git_name = rune::TextEdit::new(s.git_saved.0.clone());
                            s.git_email = rune::TextEdit::new(s.git_saved.1.clone());
                        })
                        .build(k)
                        .key("git-revert"),
                )
                .child(
                    { let tip = s.with_commands(
                            crate::i18n::t("Write them to your global git config"),
                            &[crate::git::set_global_cmd("user.name", s.git_name.text.trim()).line(), crate::git::set_global_cmd("user.email", s.git_email.text.trim()).line()],
                        ); Button::new(crate::i18n::t("Save"))
                        .variant(ButtonVariant::Submit)
                        .disabled(!changed)
                        
                        .on_press(|s: &mut State| s.save_git_identity())
                        .build(k)
                        .key("git-save").cmd_tip(s, tip) },
                ),
        );
    col()
        .w_full()
        .gap(14.0)
        .child(group(k, "Author identity", vec![block(k, "git-id", col().w_full().gap(10.0).child(muted(k, "The name and email git puts on commits you make on this computer (git config --global user.name / user.email). A repo can still override them with its own config.").wrap().w_full()).child(fields))]))
}

fn safety(s: &State, k: &Kit) -> El {
    let builtin = SENSITIVE.iter().map(|(p, what)| tf!("{p}  ({what})")).collect::<Vec<_>>().join("\n");
    let graces = [0u32, 3, 5, 10];
    let gi = graces.iter().position(|g| *g == s.prefs.grace_secs);
    let rules = col()
        .w_full()
        .gap(12.0)
        .child(
            TextField::new("Production environment names (comma separated)", &s.settings_prod)
                .placeholder(crate::i18n::t("prod, production, live, release"))
                .on_edit(|s: &mut State| &mut s.settings_prod)
                .build(k)
                .key("prod-names")
                .help(crate::i18n::t("Re-running or cancelling a workflow that deployed to one of these needs the name typed. Environments with required reviewers or a wait timer on GitHub count too.")),
        )
        .child(text("Your sensitive path patterns (one per line)").font(k.ty.small_medium).color(k.c.text_base))
        .child(
            TextArea::new(&s.settings_sensitive)
                .placeholder(crate::i18n::t("config/payment/**\n**/*.sql"))
                .rows(4)
                .width(None)
                .on_edit(|s: &mut State| &mut s.settings_sensitive)
                .build(k)
                .key("sensitive")
                .help(crate::i18n::t("PRs touching these get a ⚠ chip, and merging them is High risk. * matches within a folder, ** across folders.")),
        )
        .child(row().w_full().child(spacer()).child(Button::new(crate::i18n::t("Save safety rules")).variant(ButtonVariant::Submit).on_press(|s: &mut State| s.save_safety_settings()).build(k).key("save-safety")));
    col()
        .w_full()
        .gap(18.0)
        .child(group(
            k,
            "Merges",
            vec![item(
                k,
                "grace",
                "Merge countdown",
                "After you confirm a merge it waits this long with a Cancel button. Nothing is sent until it runs out.",
                SegmentedControl::new(&crate::i18n::all(&["Off", "3 s", "5 s", "10 s"]), gi)
                    .label(crate::i18n::t("Merge countdown"))
                    .fit()
                    .on_pick(move |s: &mut State, i, _| {
                        s.prefs.grace_secs = graces[i];
                        s.save_prefs();
                    })
                    .build(k),
            )],
        ))
        .child(group(
            k,
            "Commands",
            vec![item(
                k,
                "show-commands",
                "Show gh / git commands",
                "Everything sigit does runs the GitHub CLI (gh) or git. With this on, a button's tooltip shows the command it runs, the confirm dialog lists exactly what will run, and the result and the activity log show what ran. Handy to learn the tools, or to run it yourself when something goes wrong.",
                switch(k, "Show gh / git commands", s.prefs.show_commands, |s| s.prefs.show_commands = !s.prefs.show_commands),
            )],
        ))
        .child(group(k, "What counts as risky", vec![block(k, "rules", rules)]))
        .child(group(k, "Built in, always on", vec![block(k, "builtin", text(builtin).font(TextStyle { size: 11.0, ..k.ty.mono }).color(k.c.text_muted).wrap().w_full())]))
}

fn notifications(s: &State, k: &Kit) -> El {
    let mut watching = col().w_full().gap(6.0);
    if s.watches.is_empty() {
        watching = watching.child(muted(k, "Nothing. Use the eye button on a run or a pull request."));
    }
    for (i, w) in s.watches.iter().enumerate() {
        let target = w.target.clone();
        let label = w.label.clone();
        watching = watching.child(
            row()
                .key(tf!("w-{i}"))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(w.label.clone()).font(k.ty.ui).color(k.c.text_base).grow(1.0).truncate().shrink())
                .child(muted(k, if w.last.is_empty() { "checking…".to_owned() } else { w.last.clone() }))
                .child(IconButton::new(&glyphs::CLOSE).variant(IconButtonVariant::GhostMuted).label(crate::i18n::t("Stop watching")).help(crate::i18n::t("Stop watching")).on_press(move |s: &mut State| s.toggle_watch(target.clone(), label.clone())).build(k)),
        );
    }
    col()
        .w_full()
        .gap(18.0)
        .child(group(
            k,
            "Desktop",
            vec![
                item(k, "notify", "Desktop notifications", "A Windows notification when a watched run finishes or a watched PR changes. Shown while the app is open, even minimized.", switch(k, "Desktop notifications", s.prefs.desktop_notify, |s| s.prefs.desktop_notify ^= true)),
                item(k, "autowatch", "Watch branches you check out", "Checking out a branch here watches its CI for the rest of the day.", switch(k, "Watch branches you check out", s.prefs.auto_watch, |s| s.prefs.auto_watch ^= true)),
            ],
        ))
        .child(group(k, &tf!("Watching now ({})", s.watches.len()), vec![block(k, "watches", watching)]))
}

fn data(s: &State, k: &Kit) -> El {
    let file = crate::cache::file();
    let (size, at) = file.as_ref().and_then(|f| std::fs::metadata(f).ok()).map_or((None, None), |m| {
        (Some(m.len()), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64))
    });
    let desc = match (size, at) {
        (Some(b), Some(t)) => tf!("{:.1} MB, saved {}. Last session's PRs, runs, Activity and more, so the window opens full while fresh data loads.", b as f64 / 1_048_576.0, ago(t, now())),
        _ => "Empty. It fills as pages load and is saved every few minutes.".to_owned(),
    };
    let folder = file.as_ref().and_then(|f| f.parent()).map(|p| p.display().to_string()).unwrap_or_default();
    let url = tf!("file:///{}", folder.replace('\\', "/"));
    col()
        .w_full()
        .gap(18.0)
        .child(group(
            k,
            "Cache",
            vec![
                item(k, "cache", "Saved data", &desc, Button::new(crate::i18n::t("Clear cache")).variant(ButtonVariant::Outline).disabled(size.is_none()).on_press(|s: &mut State| s.clear_cache()).build(k)),
                item(
                    k,
                    "fresh",
                    "Keeping pages fresh",
                    "Opening a page or coming back to the window shows what's cached and checks it again in the background. While a page stays open: PRs every 2 min, runs 1 min (15 s while running), local git 20 s, Activity 5 min. Ctrl+R reloads everything now.",
                    el(),
                ),
            ],
        ))
        .child(group(
            k,
            "Files",
            vec![item(
                k,
                "folder",
                "Settings folder",
                &tf!("{folder}\nprefs.json (these settings, pins, groups), cache.json, activity.jsonl"),
                row()
                    .gap(4.0)
                    .center_y()
                    .child(super::copy_button(k, "folder-copy", folder.clone(), "folder path"))
                    .child(Button::new(crate::i18n::t("Open folder")).variant(ButtonVariant::Outline).on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url)).build(k)),
            )],
        ))
        .child(muted(k, if s.prefs.splits.is_empty() { "" } else { "Resized sections: Appearance → Section sizes resets them." }))
}

fn activity_log(s: &State, k: &Kit) -> El {
    let t = now();
    let mut log = col().w_full().gap(2.0);
    if s.activity.is_empty() {
        log = log.child(muted(k, "Nothing yet. Every merge, close, re-run, checkout… the app runs is listed here, with its result."));
    }
    for (i, e) in s.activity.iter().take(200).enumerate() {
        let head = row()
            .w_full()
            .gap(10.0)
            .center_y()
            .child(chip(k, if e.ok { "ok" } else { "failed" }, Some(if e.ok { Outcome::Success } else { Outcome::Failure })))
            .child(text(e.action.clone()).font(k.ty.ui_medium).color(k.c.text_base).w(170.0).truncate())
            .child(text(e.repo.clone()).font(k.ty.small).color(k.c.text_muted).w(160.0).truncate())
            .child(text(e.detail.clone()).font(k.ty.small).color(k.c.text_faint).grow(1.0).truncate().shrink().help(crate::i18n::t(e.detail.clone())))
            .child(muted(k, ago(e.at, t)));
        let mut entry = col().key(tf!("a-{i}")).w_full().gap(4.0).py(4.0).child(head);
        // What it ran, as you'd type it.
        if s.prefs.show_commands && !e.commands.is_empty() {
            let lines = super::cmd::lines(k, &e.commands, 11.0).w(0.0).grow(1.0);
            entry = entry.child(row().w_full().pl(4.0).gap(6.0).center_y().child(lines).child(super::copy_button(k, &tf!("a-copy-{i}"), e.commands.join("\n"), "commands")));
        }
        log = log.child(entry);
    }
    col().w_full().gap(10.0).child(muted(k, "Everything the app did in the last 30 days (activity.jsonl in the settings folder).")).child(group(k, "Recent", vec![block(k, "log", log)]))
}

const SHORTCUTS: [(&str, &str); 14] = [
    ("Ctrl+K", "Command palette: jump to a repo, #PR, branch or group"),
    ("Ctrl+,", "Settings"),
    ("F1", "Guide"),
    ("Ctrl+R", "Reload this page"),
    ("Alt+← / Alt+→", "Back / forward"),
    ("Ctrl+1", "Inbox"),
    ("Ctrl+2 … 6", "Pull requests, Actions, Environments, Commits, Branches"),
    ("J / K", "Needs you: next / previous in the review queue"),
    ("Enter", "Needs you: open the selected PR"),
    ("A", "Needs you: approve the selected PR"),
    ("C", "Needs you: check it out"),
    ("Esc", "Close a window, the palette, or cancel a merge countdown"),
    ("Ctrl+Shift+B", "Show or hide the sidebar"),
    ("Double-click a handle", "Reset resized sections"),
];

fn shortcuts(k: &Kit) -> El {
    let rows = SHORTCUTS
        .iter()
        .enumerate()
        .map(|(i, (key, what))| {
            row()
                .key(tf!("sc-{i}"))
                .w_full()
                .gap(16.0)
                .px(16.0)
                .py(9.0)
                .center_y()
                .fill(k.c.layer_01)
                .child(text(*what).font(k.ty.ui).color(k.c.text_base).w(0.0).grow(1.0))
                .child(row().px(8.0).py(2.0).radius(k.radius.sm).fill(k.c.surface_base).border(1.0, k.c.border_weak).child(text(*key).font(k.ty.mono).color(k.c.text_muted)))
        })
        .collect();
    group(k, "Keyboard", rows)
}

/// Updates: check, then install in place and restart (or, where this copy can't update
/// itself, the release page).
fn update_row(s: &State, k: &Kit) -> El {
    use crate::update::{Update, cannot_install, current, repo};
    let releases = tf!("https://github.com/{}/releases", repo());
    let blocked = cannot_install();
    let (desc, chip_label, tone): (String, Option<String>, Option<Outcome>) = match &s.update {
        Update::NotChecked => (tf!("Looks for a newer release on github.com/{}.", repo()), None, None),
        Update::Checking => ("Checking…".into(), None, None),
        Update::UpToDate => (tf!("You have the latest version ({}).", current()), Some("Up to date".into()), Some(Outcome::Success)),
        Update::Available { version, .. } => match &blocked {
            None => (tf!("Version {version} is available. Update downloads it, checks it, and replaces this copy; your settings stay."), Some(tf!("{version} available")), Some(Outcome::Running)),
            Some(why) => (tf!("Version {version} is available. {why}"), Some(tf!("{version} available")), Some(Outcome::Running)),
        },
        Update::Installing { version } => (
            tf!(
                "Downloading and installing {version}… If sigit is installed for all users, {} asks for permission to replace it.",
                if cfg!(target_os = "macos") { "macOS" } else { "Windows" }
            ),
            None,
            None,
        ),
        Update::Installed { version } => (tf!("Version {version} is installed. Restart sigit to start using it."), Some("Ready".into()), Some(Outcome::Success)),
        Update::Failed(e) => (tf!("{e}. Nothing was changed."), Some("Failed".into()), Some(Outcome::Failure)),
    };
    let mut controls = row().gap(8.0).center_y();
    if let (Some(label), Some(t)) = (chip_label, tone) {
        controls = controls.child(chip(k, label, Some(t)));
    }
    let release_page = |url: &str, primary: bool| {
        let url = if url.is_empty() { releases.clone() } else { url.to_owned() };
        Button::new(crate::i18n::t(if primary { "Open release page" } else { "What's new" }))
            .variant(if primary { ButtonVariant::Submit } else { ButtonVariant::Outline })
            .icon(&super::I_EXTERNAL)
            .on_press_tx(move |_: &mut State, tx| crate::open_url(tx, &url))
            .build(k)
    };
    match &s.update {
        Update::Available { url, tag, .. } if blocked.is_none() => {
            controls = controls
                .child(release_page(url, false).key("update-notes"))
                .child(
                    { let tip = s.with_commands(
                            crate::i18n::t("Downloads the new version, checks it against the release's checksums, and replaces this copy"),
                            &[crate::update::download_cmd(tag, crate::update::asset().unwrap_or("<this system's build>"), "<temp folder>").line()],
                        ); Button::new(crate::i18n::t("Update now"))
                        .variant(ButtonVariant::Submit)
                        
                        .on_press(|s: &mut State| s.install_update())
                        .build(k)
                        .key("update-install").cmd_tip(s, tip) },
                );
        }
        Update::Available { url, .. } => controls = controls.child(release_page(url, true).key("update-open")),
        Update::Installing { .. } => controls = controls.child(Spinner::new().size(14.0).build(k)),
        Update::Installed { .. } => {
            controls = controls.child(Button::new(crate::i18n::t("Restart now")).variant(ButtonVariant::Submit).on_press(|s: &mut State| s.restart_after_update()).build(k).key("update-restart"));
        }
        _ => {}
    }
    if !matches!(s.update, Update::Available { .. } | Update::Installing { .. } | Update::Installed { .. }) {
        controls = controls.child(
            { let tip = s.with_commands(crate::i18n::t("Looks for a newer release on GitHub"), &[crate::update::check_cmd().line()]); Button::new(crate::i18n::t(if s.update == Update::Checking { "Checking…" } else { "Check for updates" }))
                .variant(ButtonVariant::Outline)
                .disabled(s.update == Update::Checking)
                
                .on_press(|s: &mut State| s.check_for_updates())
                .build(k)
                .key("update-check").cmd_tip(s, tip) },
        );
    }
    item(k, "update", "Updates", &desc, controls)
}

fn about(s: &State, k: &Kit) -> El {
    let (gh, git) = s.versions.clone().unwrap_or_else(|| ("…".into(), "…".into()));
    col()
        .w_full()
        .gap(18.0)
        .child(group(
            k,
            "sigit",
            vec![
                item(k, "ver", &tf!("Version {}", env!("CARGO_PKG_VERSION")), "A fast desktop view of your pull requests, Actions, environments and commits, built with Rust and Rune.", el()),
                update_row(s, k),
                item(k, "gh", "GitHub CLI", &gh, el()),
                item(k, "git", "Git", &git, el()),
            ],
        ))
        .child(
            row()
                .gap(8.0)
                .child(Button::new(crate::i18n::t("Open the guide")).variant(ButtonVariant::Outline).shortcut("F1").on_press(|s: &mut State| {
                    s.close_settings();
                    s.guide_open = true;
                }).build(k).key("open-guide"))
                .child(
                    // For bug reports: everything someone helping you would ask first.
                    Button::new(crate::i18n::t("Copy version info"))
                        .variant(ButtonVariant::Outline)
                        .icon(&super::I_COPY)
                        .help(crate::i18n::t("Copy sigit's, gh's and git's versions and your system, for a bug report"))
                        .on_press_tx(move |s: &mut State, tx| {
                            let info = tf!("sigit {} ({} {})
{gh}
{git}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
                            tx.copy(info);
                            s.toast(crate::i18n::t("Copied version info"), false);
                        })
                        .build(k)
                        .key("copy-versions"),
                ),
        )
}
