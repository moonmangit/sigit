//! Organizations' local look: a badge (base colour + emoji or initials) shown on the owner's
//! sidebar header, on each of its repos (also in Pinned and your groups) and in the repo header,
//! and the "Customize" dialog that sets it.

use rune::{Color, El, TextStyle, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, TextInput};

use super::{hue, muted};
use crate::State;

/// Ready-made icons: tap one, or type any emoji or two letters.
const ICONS: [&str; 16] = ["🏢", "🎮", "🎰", "💳", "🚀", "⚡", "🧪", "🛠", "📦", "🌐", "🛡", "💼", "🐙", "⭐", "🔥", "🌙"];

/// The swatches: the app's series hues plus a neutral grey.
fn swatches() -> Vec<String> {
    let mut v: Vec<String> = (0..8).map(|i| hue(i).hex()[..7].to_owned()).collect();
    v.push("#8a8f98".into());
    v
}

fn parse(hex: &str) -> Option<Color> {
    rune::color::hex(hex).filter(|_| hex.len() == 7)
}

/// The badge itself: a rounded square in the colour with the icon (or the owner's initial).
pub fn badge_for(k: &Kit, owner: &str, color: &str, icon: &str, size: f32) -> El {
    let fill = parse(color).unwrap_or(k.c.surface_base);
    let label = if icon.is_empty() { owner.chars().next().unwrap_or('?').to_uppercase().to_string() } else { icon.to_owned() };
    let ink = if parse(color).is_some_and(|c| c.luminance() < 0.45) { Color::WHITE } else { k.c.text_strong };
    row()
        .size(size, size)
        .radius(size * 0.28)
        .center()
        .fill(fill)
        .child(text(label).font(TextStyle { size: size * 0.6, ..k.ty.small_medium }).color(ink))
}

/// The owner's badge when it has a custom look; None keeps the default icons.
pub fn badge(s: &State, k: &Kit, owner: &str, size: f32) -> Option<El> {
    let st = s.owner_style(owner)?;
    Some(badge_for(k, owner, &st.color, &st.icon, size))
}

pub fn dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some((owner, picked)) = &s.owner_edit else { return };
    let typed = s.owner_hex.text.trim();
    let typed = if typed.is_empty() || typed.starts_with('#') { typed.to_owned() } else { format!("#{typed}") };
    let color = if parse(&typed).is_some() { typed.clone() } else { picked.clone() };
    let icon: String = s.owner_icon.text.trim().chars().take(2).collect();

    // Colour: swatches + custom hex.
    let mut sw = row().gap(8.0).center_y().flex_wrap().child(
        row()
            .key("sw-none")
            .size(28.0, 28.0)
            .radius(8.0)
            .center()
            .border(if color.is_empty() { 2.0 } else { 1.0 }, if color.is_empty() { k.c.border_focus } else { k.c.border_base })
            .hover(k.c.hover)
            .help("No colour (default look)")
            .on_press(|s: &mut State| {
                if let Some(e) = s.owner_edit.as_mut() {
                    e.1.clear();
                }
                s.owner_hex.clear();
            })
            .child(text("∅").font(k.ty.small).color(k.c.text_muted)),
    );
    for (i, hex) in swatches().into_iter().enumerate() {
        let on = color.eq_ignore_ascii_case(&hex);
        let h2 = hex.clone();
        sw = sw.child(
            el().key(format!("sw-{i}"))
                .size(28.0, 28.0)
                .radius(8.0)
                .fill(parse(&hex).unwrap_or(k.c.surface_base))
                .border(if on { 2.5 } else { 0.0 }, k.c.text_strong)
                .hover(k.c.hover)
                .help(hex.clone())
                .on_press(move |s: &mut State| {
                    if let Some(e) = s.owner_edit.as_mut() {
                        e.1 = h2.clone();
                    }
                    s.owner_hex = rune::TextEdit::new(h2.clone());
                }),
        );
    }
    let hex_input = TextInput::new(&s.owner_hex)
        .placeholder("#rrggbb")
        .label("Custom colour")
        .width(Some(120.0))
        .invalid(!typed.is_empty() && parse(&typed).is_none())
        .on_edit(|s: &mut State| &mut s.owner_hex)
        .build(k)
        .key("hex");

    // Icon: presets + your own.
    let mut icons = row().gap(6.0).flex_wrap();
    for (i, ic) in ICONS.iter().enumerate() {
        let on = icon == *ic;
        icons = icons.child(
            row()
                .key(format!("ic-{i}"))
                .size(32.0, 32.0)
                .radius(8.0)
                .center()
                .fill(if on { k.c.surface_hover } else { Color::TRANSPARENT })
                .border(if on { 2.0 } else { 1.0 }, if on { k.c.border_focus } else { k.c.border_weak })
                .hover(k.c.hover)
                .on_press(move |s: &mut State| s.owner_icon = rune::TextEdit::new(*ic))
                .child(text(*ic).font(TextStyle { size: 17.0, ..k.ty.ui })),
        );
    }
    let icon_input = TextInput::new(&s.owner_icon)
        .placeholder("emoji or 2 letters")
        .label("Icon")
        .width(Some(160.0))
        .on_edit(|s: &mut State| &mut s.owner_icon)
        .on_submit(|s: &mut State, _| s.save_owner_edit(false))
        .build(k)
        .key("icon");

    // Preview: the header and a repo row as they'll look.
    let preview = col()
        .w_full()
        .gap(6.0)
        .padding(12.0)
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_weak)
        .child(row().gap(8.0).center_y().child(badge_for(k, owner, &color, &icon, 18.0)).child(text(owner.to_uppercase()).font(k.ty.text_xsmall_medium).color(k.c.text_muted)))
        .child(row().gap(8.0).pl(4.0).center_y().child(badge_for(k, owner, &color, &icon, 16.0)).child(text("some-repo").font(k.ty.ui_medium).color(k.c.text_base)));

    let section = |title: &str| text(title.to_owned()).font(k.ty.small_medium).color(k.c.text_base);
    let body = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .gap(12.0)
        .child(muted(k, "Saved on this computer only. The badge shows on this organization's header, its repos (in Pinned and your groups too) and the repo header.").wrap().w_full())
        .child(section("Colour"))
        .child(sw)
        .child(row().gap(8.0).center_y().child(hex_input).child(muted(k, "or type a hex colour")))
        .child(section("Icon"))
        .child(icons)
        .child(row().gap(8.0).center_y().child(icon_input).child(muted(k, "empty = the first letter")))
        .child(section("Preview"))
        .child(preview)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(Button::new("Reset to default").variant(ButtonVariant::GhostMuted).help("Remove the colour and icon (Undo brings them back)").on_press(|s: &mut State| s.save_owner_edit(true)).build(k).key("o-reset"))
                .child(spacer())
                .child(Button::new("Cancel").variant(ButtonVariant::Outline).on_press(|s: &mut State| s.owner_edit = None).build(k).key("o-cancel"))
                .child(Button::new("Save").variant(ButtonVariant::Submit).on_press(|s: &mut State| s.save_owner_edit(false)).build(k).key("o-save")),
        );
    Dialog::new("owner", format!("Customize {owner}")).open(true).fit().width(520.0).on_close(|s: &mut State| s.owner_edit = None).child(body).build(k, layer);
}
