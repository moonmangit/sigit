//! The interface in English or Thai.
//!
//! English is written in the code and is its own key (as gettext does): the Thai catalog,
//! `assets/i18n/th.tsv`, maps each English string to its Thai. Two ways in:
//!
//! - [`t`] for a fixed string (`t("Pull requests")`), and [`text`], the `rune::text` the views
//!   use, which runs it for them.
//! - [`tf!`](crate::tf) in place of `format!` for a sentence with values in it
//!   (`tf!("{n} open now")`): the catalog holds the whole template, so Thai can put the values
//!   where its grammar wants them. Values go in by name (`{n}`), by position (`{0}`), or in order
//!   (`{}`).
//!
//! A string with no Thai yet shows in English, and in a debug build it's noted (see
//! [`misses`]) so the catalog can be completed; the `catalog_covers_the_ui` test checks every
//! string the views name.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::config::Language;

static THAI: AtomicBool = AtomicBool::new(false);

/// Use `pref` from now on (System: the computer's first preferred language we have). Cheap to
/// call every frame: only a change does any work. `SIGIT_LANG=th` (or `en`) overrides it, for
/// screenshots.
pub fn apply(pref: Language) {
    static LAST: Mutex<Option<Language>> = Mutex::new(None);
    {
        let Ok(mut last) = LAST.lock() else { return };
        if *last == Some(pref) {
            return;
        }
        *last = Some(pref);
    }
    let thai = match std::env::var("SIGIT_LANG").ok().as_deref() {
        Some("th") => true,
        Some("en") => false,
        _ => match pref {
            Language::English => false,
            Language::Thai => true,
            Language::System => system_prefers_thai(),
        },
    };
    THAI.store(thai, Ordering::Relaxed);
    rune::labels::set_locale(if thai { "th" } else { "en" });
    rune::labels::set_labels(|l| if is_thai() { rune_label(l) } else { None });
}

/// The interface is in Thai right now.
pub fn is_thai() -> bool {
    THAI.load(Ordering::Relaxed)
}

fn system_prefers_thai() -> bool {
    let langs = rune_i18n::system_languages();
    rune_i18n::negotiate(langs.iter().map(String::as_str), &["en", "th"]) == Some(1)
}

/// The Thai catalog: English → Thai, read once from the embedded TSV. A line is
/// `english<TAB>thai`; `\n` and `\t` inside either side stand for a newline and a tab; lines
/// starting with `#` are comments.
fn catalog() -> &'static HashMap<String, String> {
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| parse(include_str!("../assets/i18n/th.tsv")))
}

fn parse(tsv: &str) -> HashMap<String, String> {
    let unescape = |s: &str| s.replace("\\n", "\n").replace("\\t", "\t");
    tsv.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(en, th)| (unescape(en), unescape(th)))
        .collect()
}

/// `s` in the interface language.
pub fn t(s: impl Into<String>) -> String {
    let s = s.into();
    if !is_thai() {
        return s;
    }
    match catalog().get(&s) {
        Some(th) => th.clone(),
        None => {
            note_miss(&s);
            s
        }
    }
}

/// Every label in `labels` in the interface language (a segmented control's, say).
pub fn all<S: AsRef<str>>(labels: &[S]) -> Vec<String> {
    labels.iter().map(|l| t(l.as_ref())).collect()
}

/// A date written with English day and month abbreviations ("Mon 9", "01 Nov – 15 Nov 2026"),
/// with Thai ones in Thai ("จ. 9", "01 พ.ย. – 15 พ.ย. 2026").
pub fn dates(s: &str) -> String {
    if !is_thai() {
        return s.to_owned();
    }
    const WORDS: [(&str, &str); 19] = [
        ("Mon", "จ."), ("Tue", "อ."), ("Wed", "พ."), ("Thu", "พฤ."), ("Fri", "ศ."), ("Sat", "ส."), ("Sun", "อา."),
        ("Jan", "ม.ค."), ("Feb", "ก.พ."), ("Mar", "มี.ค."), ("Apr", "เม.ย."), ("May", "พ.ค."), ("Jun", "มิ.ย."),
        ("Jul", "ก.ค."), ("Aug", "ส.ค."), ("Sep", "ก.ย."), ("Oct", "ต.ค."), ("Nov", "พ.ย."), ("Dec", "ธ.ค."),
    ];
    s.split(' ').map(|w| WORDS.iter().find(|(en, _)| *en == w).map_or(w, |(_, th)| th)).collect::<Vec<_>>().join(" ")
}

/// `rune::text` in the interface language.
pub fn text(s: impl Into<String>) -> rune::El {
    rune::text(t(s))
}

/// What [`tf!`](crate::tf) runs: `english` is `template` already filled in; in Thai the
/// template's translation is filled with the same values instead.
pub fn tf(template: &str, english: String) -> String {
    if !is_thai() {
        return english;
    }
    let Some(thai) = catalog().get(template) else {
        // A template with no Thai might still be a whole fixed string the catalog knows.
        return match catalog().get(&english) {
            Some(th) => th.clone(),
            None => {
                if template.contains(char::is_alphabetic) && template.contains(' ') {
                    note_miss(template);
                }
                english
            }
        };
    };
    match values(template, &english) {
        Some(vals) => fill(thai, &vals),
        None => english,
    }
}

/// `format!` in the interface language: same arguments, the template looked up in the catalog.
#[macro_export]
macro_rules! tf {
    ($lit:literal $($rest:tt)*) => {
        $crate::i18n::tf($lit, format!($lit $($rest)*))
    };
}

/// A template split into literal text and placeholders (`{}`, `{n}`, `{0}`, `{x:.1}`).
#[derive(Debug, PartialEq)]
enum Piece<'a> {
    Lit(String),
    Hole(&'a str),
}

fn pieces(template: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let b = template.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' if b.get(i + 1) == Some(&b'{') => {
                lit.push('{');
                i += 2;
            }
            b'}' if b.get(i + 1) == Some(&b'}') => {
                lit.push('}');
                i += 2;
            }
            b'{' => {
                let end = template[i..].find('}').map_or(template.len(), |e| i + e);
                out.push(Piece::Lit(std::mem::take(&mut lit)));
                out.push(Piece::Hole(&template[i + 1..end]));
                i = end + 1;
            }
            _ => {
                let ch = template[i..].chars().next().unwrap_or(' ');
                lit.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out.push(Piece::Lit(lit));
    out
}

/// The values the English template was filled with, read back out of `english`: each
/// placeholder's text, with its name (`n`, or `""` for `{}`).
fn values<'a>(template: &'a str, english: &str) -> Option<Vec<(&'a str, String)>> {
    let ps = pieces(template);
    let mut rest = english;
    let mut out = Vec::new();
    let mut i = 0;
    // The previous piece was a placeholder (an empty literal between two doesn't count).
    let mut after_hole = false;
    while i < ps.len() {
        match &ps[i] {
            Piece::Lit(l) => {
                rest = rest.strip_prefix(l.as_str())?;
                after_hole &= l.is_empty();
            }
            Piece::Hole(name) => {
                // Runs to the next non-empty literal (or the end). Of placeholders side by side
                // (`{shown}{more}`) the first takes it all and the rest are empty: the same text.
                let next = ps[i + 1..].iter().enumerate().find_map(|(j, p)| match p {
                    Piece::Lit(l) if !l.is_empty() => Some((i + 1 + j, l.as_str())),
                    _ => None,
                });
                let end = match next {
                    None => rest.len(),
                    // The last occurrence for the final literal, the first otherwise.
                    Some((j, l)) if j + 1 >= ps.len() => rest.rfind(l)?,
                    Some((_, l)) => rest.find(l)?,
                };
                let name = name.split(':').next().unwrap_or("");
                if after_hole {
                    out.push((name, String::new()));
                } else {
                    out.push((name, rest[..end].to_owned()));
                    rest = &rest[end..];
                }
                after_hole = true;
            }
        }
        i += 1;
    }
    rest.is_empty().then_some(out)
}

/// `thai` with its placeholders replaced: `{name}` by the value of that name, `{N}` by the
/// Nth value, `{}` by the next unnamed value not yet used.
fn fill(thai: &str, vals: &[(&str, String)]) -> String {
    let mut out = String::new();
    let mut next = 0;
    for p in pieces(thai) {
        match p {
            Piece::Lit(l) => out.push_str(&l),
            Piece::Hole(h) => {
                let h = h.split(':').next().unwrap_or("");
                // `{}` takes the next unnamed value: in "{failing} of {} checks" it is the
                // count, not `failing`.
                let v = if h.is_empty() {
                    let v = vals.iter().filter(|(name, _)| name.is_empty()).nth(next);
                    next += 1;
                    v
                } else if let Ok(n) = h.parse::<usize>() {
                    vals.get(n)
                } else {
                    vals.iter().find(|(name, _)| *name == h)
                };
                out.push_str(v.map_or("", |(_, v)| v.as_str()));
            }
        }
    }
    out
}

/// Thai for the words Rune itself shows (a text field's menu, the window menu).
fn rune_label(l: rune::labels::Label<'_>) -> Option<String> {
    use rune::labels::Label;
    Some(
        match l {
            Label::Cut => "ตัด",
            Label::Copy => "คัดลอก",
            Label::Paste => "วาง",
            Label::SelectAll => "เลือกทั้งหมด",
            Label::Undo => "เลิกทำ",
            Label::Redo => "ทำซ้ำ",
            Label::Minimize => "ย่อหน้าต่าง",
            Label::Close => "ปิด",
            _ => return None,
        }
        .to_owned(),
    )
}

// ---------------------------------------------------------------------------------------------
// Strings with no Thai yet (debug builds)
// ---------------------------------------------------------------------------------------------

static MISSES: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn note_miss(s: &str) {
    // Thai already (a string translated twice), or nothing to translate.
    let thai = s.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c));
    if !cfg!(debug_assertions) || thai || !s.chars().any(char::is_alphabetic) {
        return;
    }
    let Ok(mut m) = MISSES.lock() else { return };
    if m.insert(s.to_owned()) {
        // SIGIT_I18N_MISSES=<file>: append each new one, to complete the catalog from.
        if let Some(path) = std::env::var_os("SIGIT_I18N_MISSES") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{}", s.replace('\n', "\\n"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unnamed_holes_skip_named_values() {
        let v = values("{failing} of {} checks are failing", "2 of 5 checks are failing").unwrap();
        assert_eq!(fill("{failing} จาก {} check ล้มเหลว", &v), "2 จาก 5 check ล้มเหลว");
        let v = values("{what} would conflict in: {}", "Reverting abc would conflict in: a.txt").unwrap();
        assert_eq!(fill("{what} จะ conflict ใน: {}", &v), "Reverting abc จะ conflict ใน: a.txt");
    }

    #[test]
    fn reads_values_back_and_reorders_them() {
        let v = values("{n} of {total} open", "3 of 10 open").unwrap();
        assert_eq!(fill("เปิด {n} จาก {total}", &v), "เปิด 3 จาก 10");
        let v = values("{} behind {}", "4 behind main").unwrap();
        assert_eq!(fill("{1} ตามหลัง {0}", &v), "main ตามหลัง 4");
        assert_eq!(fill("ตามหลัง {} อยู่ {}", &v), "ตามหลัง 4 อยู่ main");
        assert!(values("{n} open", "3 closed").is_none());
        let v = values("Copied {what}: {shown}{more}", "Copied SHA: abc: def…").unwrap();
        assert_eq!(v[0].1, "SHA");
    }

    #[test]
    fn escapes_and_formats() {
        let v = values("{{x}} {n:.1}%", "{x} 2.5%").unwrap();
        assert_eq!(fill("{n}% ({{x}})", &v), "2.5% ({x})");
    }

    /// Every Thai entry fills only from values its English has: a named placeholder must exist
    /// in the English, and `{N}` / `{}` must be in range.
    #[test]
    fn thai_placeholders_match_the_english() {
        let holes = |s: &str| pieces(s).into_iter().filter_map(|p| if let Piece::Hole(h) = p { Some(h.split(':').next().unwrap_or("").to_owned()) } else { None }).collect::<Vec<_>>();
        let mut bad = Vec::new();
        for (en, th) in parse(include_str!("../assets/i18n/th.tsv")) {
            let (e, t) = (holes(&en), holes(&th));
            let positional = e.len();
            let named: Vec<&String> = e.iter().filter(|h| !h.is_empty() && h.parse::<usize>().is_err()).collect();
            let sequential = t.iter().filter(|h| h.is_empty()).count();
            for h in &t {
                let ok = if h.is_empty() {
                    sequential <= positional
                } else if let Ok(n) = h.parse::<usize>() {
                    n < positional
                } else {
                    named.contains(&h)
                };
                if !ok {
                    bad.push(format!("{en:?} → {th:?}: {{{h}}}"));
                }
            }
        }
        assert!(bad.is_empty(), "Thai placeholders with no English value:\n{}", bad.join("\n"));
    }

    #[test]
    fn catalog_parses() {
        let c = parse("# comment\nHello\tสวัสดี\nTwo\\nlines\tสอง\\nบรรทัด\n");
        assert_eq!(c["Hello"], "สวัสดี");
        assert_eq!(c["Two\nlines"], "สอง\nบรรทัด");
    }
}
