"""Lists the interface strings with no Thai yet.

    python scripts/i18n-check.py

It reads the string literals in the views (src/ui) and in the modules whose messages reach the
screen, drops what isn't prose (keys, ids, paths, commands, formats, API fields), and prints the
ones `assets/i18n/th.tsv` doesn't have. A string that should stay as it is (a product name, a
command) can go in the catalog mapped to itself. Exit status 1 when something is missing.

Strings built at run time are best caught by running the app in Thai with
`SIGIT_LANG=th SIGIT_I18N_MISSES=misses.txt` (debug build), which lists every English string
shown that the catalog lacks.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
FILES = sorted((ROOT / "src/ui").glob("*.rs")) + [ROOT / "src" / n for n in [
    "main.rs", "daily.rs", "focus.rs", "settings.rs", "onboarding.rs", "update.rs", "guard.rs",
    "config.rs", "pulse.rs", "risk.rs", "model.rs", "activity.rs"]]

LITERAL = re.compile(r'(?<![A-Za-z0-9_#])"((?:[^"\\]|\\.)*)"')
# Literals right after these are code, not words on screen.
CODE_CONTEXT = re.compile(
    r'(\.key\(|\.role\(|\.shortcut\(|is_command|hex\(|Command::new|\.arg\(|\.args\(|env::var|starts_with|'
    r'ends_with|strip_prefix|strip_suffix|contains\(|split|trim_|replace\(|\.get\(|json|serde|== "|!= "|'
    r'=> "|matches!|cfg|#\[|include_str|Path|with_extension|\.join\(|assert|expect\(|\.format\(|'
    r'gh::run|git\(|"--|Icon \{)')


# Shown as written: commands, versions and the screenshot scenes' sample data.
KEEP = re.compile(r"^(gh |git |rm -rf|repo:|scale\(|currentColor|Demo$|Rust tooling$|octocat |, env!|AIza$|Ctrl\+|\{alias\}|\.\./|<[^>]+>$|.*Noto Sans Thai$|✓ Squashed and merged)")


def is_prose(v: str) -> bool:
    words = re.sub(r"\{[^}]*\}", "", v)
    if not re.search(r"[A-Za-z]{2,}", words):
        return False  # only placeholders, symbols or single letters
    if KEEP.search(v) or re.fullmatch(r"[a-z]+[A-Z][A-Za-z]*|[A-Z][a-z]+[A-Z][A-Za-z]*", v):
        return False  # camelCase API fields
    if re.fullmatch(r"(Enter|Esc)", v):
        return False
    if re.fullmatch(r"[a-z0-9_\-./:{}|=?&@\[\]\\ ,]+", v) and " " not in v.strip():
        return False  # keys, ids, paths, urls
    if re.fullmatch(r"[A-Z_]+", v) and len(v) > 1:
        return False  # API enum values
    if re.fullmatch(r"[A-Za-z]+(,[A-Za-z]+)+", v):
        return False  # gh --json field lists
    if v.startswith(("http", "#", "--", "-", "**", ".[", ".github", "M", "%")) and not re.match(r"M[a-z]", v):
        return v.startswith("M") and " " in v and not re.match(r"M[\d.]", v)
    if re.search(r"<path|<svg|Start-Process|Expand-Archive|do shell script|tell application|query\{|is:pr|repos/|winget|brew ", v):
        return False
    if re.fullmatch(r"(Ctrl|Alt|Shift|F\d)[+\w←→/ …]*", v):
        return False  # shortcuts
    return True


def strings():
    out = set()
    for f in FILES:
        in_tests = False
        in_scenes = False  # the screenshot scenes' sample data (main.rs)
        for line in f.read_text(encoding="utf-8").splitlines():
            if "#[cfg(test)]" in line:
                in_tests = True
            if "scenes: &[" in line:
                in_scenes = True
            elif in_scenes and line.strip() == "],":
                in_scenes = False
            if in_tests or in_scenes or line.strip().startswith("//"):
                continue
            for m in LITERAL.finditer(line):
                if CODE_CONTEXT.search(line[:m.start()][-40:]):
                    continue
                v = m.group(1).replace('\\"', '"')
                if is_prose(v):
                    out.add(v)
    return out


def catalog():
    known = set()
    for line in (ROOT / "assets/i18n/th.tsv").read_text(encoding="utf-8").splitlines():
        if line and not line.startswith("#") and "\t" in line:
            known.add(line.split("\t", 1)[0])
    return known


if __name__ == "__main__":
    missing = sorted(strings() - catalog())
    for s in missing:
        print(s)
    print(f"{len(missing)} without Thai", file=sys.stderr)
    sys.exit(1 if missing else 0)
