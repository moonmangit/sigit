//! The "New pull request" form: From and To, Ready or Draft, title, body, people and labels,
//! with the helpers (comparison, fill buttons, blocked reason). Create PR hands it to the guard.

use rune::{Color, El, TextEdit, col, el, row, spacer};
use rune_kit::{Button, ButtonVariant, Combobox, ComboboxOption, ComboboxState, Dialog, IconButton, IconButtonVariant, Kit, SegmentedControl, Select, Spinner, TextArea, TextField, glyphs};

use super::muted;
use crate::i18n::text;
use crate::route::{self, Person, RepoLabel};
use crate::{PrList, State};

/// A label's colour from GitHub's 6 hex digits, or `fallback` if they don't parse.
fn label_color(hex: &str, fallback: Color) -> Color {
    if hex.len() == 6 { rune::color::hex(&format!("#{hex}")).unwrap_or(fallback) } else { fallback }
}

/// The last option of a people picker: add what was typed, if it's not already an option or chosen.
fn typed_option(options: &mut Vec<ComboboxOption>, chosen: &[String], search: &str) {
    let text = search.trim();
    if !text.is_empty() && !options.iter().any(|o| o.id == text) && !chosen.iter().any(|c| c == text) {
        options.push(ComboboxOption::new(text, tf!("Add \"{text}\" as typed")));
    }
}

/// Reviewers and Assignees options: `users` not chosen yet (`@me` first for Assignees), then
/// what's typed as one more option when no option is it. The combobox filters them by what's typed.
fn people_options(users: &[Person], chosen: &[String], me: bool, search: &str) -> Vec<ComboboxOption> {
    let mut options: Vec<ComboboxOption> = Vec::new();
    if me && !chosen.iter().any(|c| c == "@me") {
        options.push(ComboboxOption::new("@me", "@me"));
    }
    for u in users.iter().filter(|u| !chosen.contains(&u.login)) {
        let mut o = ComboboxOption::new(u.login.clone(), u.login.clone());
        o.detail = u.name.clone();
        options.push(o);
    }
    typed_option(&mut options, chosen, search);
    options
}

/// Label options: the repo's labels not chosen yet. Never a typed entry: GitHub rejects a PR
/// with a label the repo doesn't have.
fn label_options(labels: &[RepoLabel], chosen: &[String]) -> Vec<ComboboxOption> {
    labels
        .iter()
        .filter(|l| !chosen.contains(&l.name))
        .map(|l| {
            let mut o = ComboboxOption::new(l.name.clone(), l.name.clone());
            o.detail = l.description.clone();
            o
        })
        .collect()
}

/// One field of the open form. Edits only reach an open form; one landing after it closed
/// goes nowhere.
fn field(s: &mut State, f: fn(&mut crate::PrDraft) -> &mut TextEdit) -> &mut TextEdit {
    match s.pr_draft.as_mut() {
        Some(d) => f(d),
        None => Box::leak(Box::new(TextEdit::new(""))),
    }
}

/// The repo's branch names, from GitHub and the clone, each once, without `origin/`, A-Z.
fn branch_names(s: &State, slug: &str) -> Vec<String> {
    s.pr_branch_names(slug).unwrap_or_default()
}

/// "3 commits ahead", a spinner while the comparison loads, nothing if it failed.
fn comparison_line(s: &State, k: &Kit, d: &crate::PrDraft) -> Option<El> {
    if d.from.is_empty() || d.to.is_empty() || d.from == d.to {
        return None;
    }
    if let Some(n) = s.pr_ahead() {
        let line = if n == 1 { crate::i18n::t("1 commit ahead") } else { tf!("{n} commits ahead") };
        return Some(muted(k, line).key("prd-ahead"));
    }
    let loading = s.pr_compare.get(&(d.slug.clone(), d.from.clone(), d.to.clone())).is_some_and(|l| l.loading);
    loading.then(|| row().key("prd-loading").gap(8.0).center_y().child(Spinner::new().size(12.0).build(k)).child(muted(k, crate::i18n::t("Comparing…"))))
}

/// A labelled branch dropdown; `which` is 1 for From, 2 for To.
fn picker(s: &State, k: &Kit, which: u8, label: &str, current: &str, key: &str) -> El {
    col()
        .w(0.0)
        .grow(1.0)
        .gap(4.0)
        .child(text(crate::i18n::t(label)).font(k.ty.small_medium).color(k.c.text_base))
        .child(
            Select::new(key.to_owned(), &[] as &[&str])
                .label(crate::i18n::t(label))
                .current(Some(current).filter(|c| !c.is_empty()))
                .placeholder(crate::i18n::t("Pick a branch"))
                .open(s.pr_pick_open == which)
                .on_toggle(move |s: &mut State| s.pr_pick_open = if s.pr_pick_open == which { 0 } else { which })
                .trigger(k),
        )
}

/// One chosen value: its colour dot when it has one, the name, and a × that takes it off.
fn chip(k: &Kit, list: PrList, key: &str, value: &str, dot: Option<Color>) -> El {
    let mut c = row().key(format!("{key}-chip-{value}")).gap(6.0).center_y().pl(8.0).pr(2.0).py(1.0).radius(999.0).fill(k.c.surface_base).border(1.0, k.c.border_weak);
    if let Some(color) = dot {
        c = c.child(el().w(8.0).h(8.0).radius(4.0).fill(color));
    }
    let target = value.to_owned();
    c.child(text(value.to_owned()).font(k.ty.small).color(k.c.text_base)).child(
        IconButton::new(&glyphs::CLOSE)
            .variant(IconButtonVariant::GhostMuted)
            .label(tf!("Remove {value}"))
            .on_press(move |s: &mut State| s.pr_remove(list, &target))
            .build(k)
            .key(format!("{key}-x-{value}")),
    )
}

/// A picker: its label, the chosen values as chips, and "+ Add …" opening a searchable list
/// (built into `pop`, which goes above the dialog). While the list loads a spinner shows, and if
/// it failed a muted line says so; people can still be added as typed then, labels can't.
fn pick_block(s: &State, k: &Kit, pop: &mut Vec<El>, list: PrList) -> El {
    let Some(d) = &s.pr_draft else { return col() };
    let (label, add, search, key, text_now, state, access): (&str, &str, &str, &str, &str, &ComboboxState, fn(&mut State) -> &mut ComboboxState) = match list {
        PrList::Reviewers => ("Reviewers", "+ Add reviewer", "Username or org/team", "prd-reviewers", &d.reviewers.text, &s.pr_reviewers_pick, |s| &mut s.pr_reviewers_pick),
        PrList::Assignees => ("Assignees", "+ Add assignee", "@me or a username", "prd-assignees", &d.assignees.text, &s.pr_assignees_pick, |s| &mut s.pr_assignees_pick),
        PrList::Labels => ("Labels", "+ Add label", "Search labels", "prd-labels", &d.labels.text, &s.pr_labels_pick, |s| &mut s.pr_labels_pick),
    };
    let chosen = route::split_list(text_now);
    let (options, loading, failed, error, colors) = if list == PrList::Labels {
        let l = s.pr_labels.get(&d.slug);
        let labels = l.and_then(|l| l.data.clone()).unwrap_or_default();
        let colors: Vec<(String, Color)> = labels.iter().map(|x| (x.name.clone(), label_color(&x.color, k.c.icon_faint))).collect();
        (label_options(&labels, &chosen), l.is_some_and(|l| l.loading), l.is_some_and(|l| l.data.is_none() && l.error.is_some()), "Couldn't load labels", colors)
    } else {
        let l = s.pr_people.get(&d.slug);
        let users = l.and_then(|l| l.data.clone()).unwrap_or_default();
        (people_options(&users, &chosen, list == PrList::Assignees, &state.search().text), l.is_some_and(|l| l.loading), l.is_some_and(|l| l.data.is_none() && l.error.is_some()), "Couldn't load people", Vec::new())
    };
    let mut chips = row().key(format!("{key}-chips")).w_full().wrap().gap(6.0);
    for v in &chosen {
        // A label chip wears the label's colour when the repo's labels are known.
        let dot = colors.iter().find(|(n, _)| n == v).map(|(_, c)| *c);
        chips = chips.child(chip(k, list, key, v, dot));
    }
    let trigger = Button::new(crate::i18n::t(add)).variant(ButtonVariant::Outline).build(k);
    let picker = Combobox::new(key, state, &options, access)
        .label(crate::i18n::t(label))
        .search_placeholder(crate::i18n::t(search))
        .empty_message(crate::i18n::t(if list == PrList::Labels { "No labels match" } else { "No people match" }))
        .trigger(trigger)
        .on_choose(move |s: &mut State, id: &str, _| s.pr_add(list, id))
        .build(k, pop);
    let mut tail = row().gap(8.0).center_y().child(picker);
    if loading {
        tail = tail.child(Spinner::new().size(12.0).build(k).key(format!("{key}-loading")));
    } else if failed {
        tail = tail.child(muted(k, crate::i18n::t(error)).key(format!("{key}-failed")));
    }
    let mut block = col().w(0.0).grow(1.0).gap(6.0).child(text(crate::i18n::t(label)).font(k.ty.small_medium).color(k.c.text_base));
    if !chosen.is_empty() {
        block = block.child(chips);
    }
    block.child(tail)
}

/// "New pull request acme/api": the fields, editable; Create PR hands the PR to the guard.
pub fn pr_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(d) = &s.pr_draft else { return };
    let blocked = s.pr_blocked();
    // A visible label above each box, and a hint inside it while it's empty.
    let input = |e: &TextEdit, label: &str, hint: &str, key: &str, f: fn(&mut crate::PrDraft) -> &mut TextEdit| {
        TextField::new(crate::i18n::t(label), e).placeholder(crate::i18n::t(hint)).on_edit(move |s: &mut State| field(s, f)).build(k).key(key.to_owned())
    };
    // The pickers' open lists, built here and pushed after the dialog so they sit on top of it.
    let mut pop: Vec<El> = Vec::new();
    let mut body = col().w_full().px(16.0).pb(16.0).gap(12.0).child(muted(k, d.slug.clone())).child(
        row()
            .w_full()
            .gap(8.0)
            .child(picker(s, k, 1, "From", &d.from, "prd-from"))
            .child(text("→").font(k.ty.small).color(k.c.text_faint))
            .child(picker(s, k, 2, "To", &d.to, "prd-to")),
    );
    if let Some(line) = comparison_line(s, k, d) {
        body = body.child(line);
    }
    let ready = s.pr_ahead().is_some();
    body = body
        .child(
            SegmentedControl::new(&crate::i18n::all(&["Ready for review", "Draft"]), Some(usize::from(d.draft)))
                .fit()
                .on_pick(|s: &mut State, i, _| {
                    if let Some(d) = s.pr_draft.as_mut() {
                        d.draft = i == 1;
                    }
                })
                .build(k)
                .key("prd-draft"),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_end()
                .child(row().w(0.0).grow(1.0).child(input(&d.title, "Title", "What this pull request does", "prd-title", |d| &mut d.title)))
                .child(Button::new(crate::i18n::t("Use last commit")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.fill_pr_title()).build(k).key("prd-fill-title")),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(crate::i18n::t("Body")).font(k.ty.small_medium).color(k.c.text_base))
                .child(spacer())
                .child(
                    Button::new(crate::i18n::t("List commits"))
                        .variant(ButtonVariant::GhostMuted)
                        .disabled(!ready)
                        .on_press(|s: &mut State| s.fill_pr_body())
                        .build(k)
                        .key("prd-fill-body"),
                ),
        )
        .child(TextArea::new(&d.body).rows(5).width(None).on_edit(|s: &mut State| field(s, |d| &mut d.body)).build(k).key("prd-body"))
        .child(row().w_full().gap(8.0).items_start().child(pick_block(s, k, &mut pop, PrList::Reviewers)).child(pick_block(s, k, &mut pop, PrList::Assignees)))
        .child(row().w_full().child(pick_block(s, k, &mut pop, PrList::Labels)));
    if let Some(why) = &blocked {
        let line = match s.pr_already_open() {
            Some(n) => {
                let slug = d.slug.clone();
                Button::new(why.clone())
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |s: &mut State| {
                        s.pr_draft = None;
                        s.open_pr(&slug, n);
                    })
                    .build(k)
                    .key("prd-blocked")
            }
            None => text(why.clone()).font(k.ty.small).color(k.c.warning.fg).wrap().key("prd-blocked"),
        };
        body = body.child(line);
    }
    body = body.child(
        row()
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(Button::new(crate::i18n::t("Cancel")).variant(ButtonVariant::Outline).on_press(|s: &mut State| s.pr_draft = None).build(k).key("prd-cancel"))
            .child(Button::new(crate::i18n::t("Create PR")).variant(ButtonVariant::Submit).disabled(blocked.is_some()).on_press(|s: &mut State| s.submit_pr_draft()).build(k).key("prd-open")),
    );
    Dialog::new("pr-form", crate::i18n::t("New pull request")).open(true).fit().width(620.0).on_close(|s: &mut State| s.pr_draft = None).child(body).build(k, layer);
    layer.extend(pop);
    // The open branch list, anchored to its trigger (built after the dialog so it sits on top).
    let which = s.pr_pick_open;
    if which == 1 || which == 2 {
        let (key, current) = if which == 1 { ("prd-from", &d.from) } else { ("prd-to", &d.to) };
        let names = branch_names(s, &d.slug);
        let _trigger = Select::new(key.to_owned(), &names)
            .label(crate::i18n::t(if which == 1 { "From" } else { "To" }))
            .current(Some(current.as_str()))
            .open(true)
            .on_close(|s: &mut State| s.pr_pick_open = 0)
            .on_choose(move |s: &mut State, v: &str, _| {
                s.pr_pick_open = 0;
                if which == 1 { s.set_pr_from(v) } else { s.set_pr_to(v) }
            })
            .build(k, layer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(login: &str, name: &str) -> Person {
        Person { login: login.into(), name: name.into() }
    }

    fn ids(o: &[ComboboxOption]) -> Vec<&str> {
        o.iter().map(|o| o.id.as_str()).collect()
    }

    #[test]
    fn picker_options_skip_chosen_and_offer_typed() {
        let users = [person("lead", "Lena"), person("sam", "Sam R"), person("sarah", "")];
        let chosen = vec!["lead".to_owned()];
        assert_eq!(ids(&people_options(&users, &chosen, false, "sa")), ["sam", "sarah", "sa"], "typed `sa` is no option's id");
        let o = people_options(&users, &chosen, false, " sa ");
        assert_eq!(o.last().unwrap().label, "Add \"sa\" as typed");
        assert_eq!(o[0].detail, "Sam R", "the detail is the name");
        assert_eq!(ids(&people_options(&users, &chosen, false, "sam")), ["sam", "sarah"], "`sam` is an option already");
        assert_eq!(ids(&people_options(&users, &chosen, false, "")), ["sam", "sarah"]);
        assert_eq!(ids(&people_options(&[], &[], false, "org/team")), ["org/team"], "typed entry still works with no list");
        assert_eq!(ids(&people_options(&users, &chosen, false, "lead")), ["sam", "sarah"], "already chosen: nothing to add");
        let labels = [RepoLabel { name: "bug".into(), color: "d73a4a".into(), description: "Broken".into() }, RepoLabel { name: "docs".into(), color: String::new(), description: String::new() }];
        assert_eq!(ids(&label_options(&labels, &["docs".to_owned()])), ["bug"], "labels skip chosen and never offer typed");
        assert_eq!(label_options(&labels, &[])[0].detail, "Broken");
    }

    #[test]
    fn assignees_offer_me_first() {
        let users = [person("lead", ""), person("sam", "")];
        assert_eq!(ids(&people_options(&users, &[], true, "")), ["@me", "lead", "sam"]);
        assert_eq!(ids(&people_options(&users, &["@me".to_owned()], true, "")), ["lead", "sam"]);
        assert_eq!(ids(&people_options(&users, &[], false, "")), ["lead", "sam"], "Reviewers has no @me");
    }

    #[test]
    fn label_color_parses_or_falls_back() {
        let fallback = Color::WHITE;
        assert_eq!(label_color("d73a4a", fallback), rune::color::hex("#d73a4a").unwrap());
        assert_eq!(label_color("zz", fallback), fallback);
        assert_eq!(label_color("", fallback), fallback);
        assert_eq!(label_color("gggggg", fallback), fallback);
    }
}
