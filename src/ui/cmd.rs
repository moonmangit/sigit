//! The gh / git commands behind actions, as the app shows them: highlighted terminal lines, and
//! the hover card on a button that runs one (Settings → Safety → Show gh / git commands).

use rune::{Color, El, Span, TextStyle, col, el, rich, row, text};
use rune_kit::{Kit, Placement, Tooltip, TooltipStyle};

use super::hue;
use crate::State;

/// What a button's hover card says: its description and the commands it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Tip {
    pub help: String,
    pub lines: Vec<String>,
}

impl Tip {
    pub fn new(help: impl Into<String>, lines: Vec<String>) -> Tip {
        Tip { help: help.into(), lines }
    }

    /// A key for the button's anchor, the same for the same tip (so a list of Done buttons gets
    /// one each).
    fn key(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.help.hash(&mut h);
        self.lines.hash(&mut h);
        format!("tip-{:x}", h.finish())
    }
}

/// The colour of each part of a command line.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Program,
    Command,
    Flag,
    Quoted,
    Number,
    /// A ref, path or URL-ish argument (`origin/main`, `o/r`, `stash@{0}`).
    Path,
    Word,
}

/// Splits a shell line (as `gh::Cmd::line` writes it) into coloured parts, by byte range.
fn parts(line: &str) -> Vec<(usize, usize, Part)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut word = 0;
    while i < b.len() {
        if b[i] == b' ' {
            i += 1;
            continue;
        }
        let start = i;
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(b.len());
            out.push((start, i, Part::Quoted));
        } else {
            while i < b.len() && b[i] != b' ' {
                i += 1;
            }
            let w = &line[start..i];
            let part = if word == 0 {
                Part::Program
            } else if w.starts_with('-') {
                Part::Flag
            } else if w.chars().all(|c| c.is_ascii_digit()) {
                Part::Number
            } else if w.contains(['/', '@', '.', '{']) {
                Part::Path
            } else if word <= 2 && out.last().is_some_and(|(_, _, p)| matches!(p, Part::Program | Part::Command)) {
                // `gh pr merge`, `git stash pop`: the subcommands.
                Part::Command
            } else {
                Part::Word
            };
            out.push((start, i, part));
        }
        word += 1;
    }
    out
}

fn color(k: &Kit, p: Part) -> Color {
    match p {
        Part::Program => k.c.text_accent,
        Part::Command => k.c.text_strong,
        Part::Flag => hue(3),
        Part::Quoted => k.c.success.fg,
        Part::Number => k.c.warning.fg,
        Part::Path => hue(4),
        Part::Word => k.c.text_base,
    }
}

/// One command, highlighted, after a faint `$`.
pub fn line(k: &Kit, cmd: &str, size: f32) -> El {
    let font = TextStyle { size, ..k.ty.mono };
    let mut spans: Vec<Span> = parts(cmd).into_iter().map(|(a, b, p)| Span::new(a, b).color(color(k, p))).collect();
    // The program in a heavier weight, so the line reads from its start.
    if let Some(first) = spans.first_mut() {
        first.weight = Some(600.0);
    }
    row()
        .gap(7.0)
        .min_w(0.0)
        .child(rune::text("$").font(font).color(k.c.text_faint))
        .child(rich(cmd.to_owned(), spans).font(font).color(k.c.text_base).wrap().shrink().min_w(0.0).selectable(true))
}

/// Several commands, one per line.
pub fn lines(k: &Kit, cmds: &[String], size: f32) -> El {
    cmds.iter().fold(col().gap(3.0).min_w(0.0), |c, l| c.child(line(k, l, size)))
}

/// Every tooltip's look: a raised card with a hairline border, a soft shadow and room inside,
/// fading and scaling in a little as it opens and closes.
pub fn tip_style(k: &Kit) -> TooltipStyle {
    TooltipStyle::from_kit(k, rune_kit::TooltipAppearance::Compact)
        .fill(k.c.surface_raised)
        .border(1.0, k.c.border_weak)
        .radius(8.0)
        .padding(10.0, 7.0)
        .max_width(420.0)
        .font(k.ty.small)
        .ink(k.c.text_base)
        .shadows(k.shadow_layers(&k.elevation.floating).collect::<Vec<_>>())
        .motion(k.motion.overlay)
}

/// The runtime's own tooltips (every plain `.help`) in that look, with a short fade.
pub fn help_look(k: &Kit) -> rune::HelpLook {
    tip_style(k).help_look().fade(0.12, 0.08, 0.97)
}

/// A button (or anything) that runs commands: its tooltip is a card with the description and
/// the highlighted commands once the pointer rests on it, or just the description when
/// Settings hides commands.
pub trait CmdTip {
    fn cmd_tip(self, s: &State, tip: Tip) -> El;
}

impl CmdTip for El {
    fn cmd_tip(self, s: &State, tip: Tip) -> El {
        if !s.prefs.show_commands || tip.lines.is_empty() {
            return self.help(tip.help);
        }
        let key = tip.key();
        let mine = key.clone();
        row()
            .key(format!("{key}-trigger"))
            .child(self)
            .hover_delays(std::time::Duration::from_millis(450), std::time::Duration::from_millis(60))
            .on_hover_change(move |s: &mut State, on, _| {
                if on {
                    s.cmd_tip = Some(tip.clone());
                } else if s.cmd_tip.as_ref().is_some_and(|t| t.key() == mine) {
                    s.cmd_tip = None;
                }
            })
    }
}

/// The open hover card, anchored under its button.
pub fn tip_layer(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(tip) = &s.cmd_tip else { return };
    let body = col()
        .gap(8.0)
        .max_w(400.0)
        .child(text(tip.help.clone()).font(k.ty.small).color(k.c.text_base).wrap())
        .child(el().w_full().h(1.0).fill(k.c.border_weak))
        .child(lines(k, &tip.lines, 11.5));
    let _ = Tooltip::new(tip.key()).child(body).open(true).placement(Placement::BottomEnd).render(&tip_style(k).padding(12.0, 10.0), layer);
}

#[cfg(test)]
mod tests {
    use super::{Part, parts};

    #[test]
    fn colours_each_part() {
        let l = "gh pr merge 12 -R o/r --squash -b \"ship it\"";
        let got: Vec<(&str, Part)> = parts(l).into_iter().map(|(a, b, p)| (&l[a..b], p)).collect();
        assert_eq!(
            got,
            [
                ("gh", Part::Program),
                ("pr", Part::Command),
                ("merge", Part::Command),
                ("12", Part::Number),
                ("-R", Part::Flag),
                ("o/r", Part::Path),
                ("--squash", Part::Flag),
                ("-b", Part::Flag),
                ("\"ship it\"", Part::Quoted),
            ]
        );
        let g: Vec<Part> = parts("git stash pop stash@{0}").into_iter().map(|(_, _, p)| p).collect();
        assert_eq!(g, [Part::Program, Part::Command, Part::Command, Part::Path]);
    }
}
