//! The rules behind the guard rails (A1, A3, A4): how risky an action is and why, which changed
//! files deserve care, which added lines look like secrets, and which environments are
//! production. Pure functions, no IO, so they're unit-tested below.

/// How careful the app is before running an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Runs at once; a toast says how it went.
    Low,
    /// A confirm dialog listing what makes it risky.
    Medium,
    /// A red dialog: the reasons, and you type a name to confirm.
    High,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Low => "Low risk",
            Level::Medium => "Needs a look",
            Level::High => "High risk",
        }
    }
}

/// An action's risk: its level and the reasons for it, in plain words.
#[derive(Clone, Debug)]
pub struct Risk {
    pub level: Level,
    pub reasons: Vec<String>,
}

impl Risk {
    pub fn new(level: Level) -> Risk {
        Risk { level, reasons: Vec::new() }
    }
    /// Add a reason, raising the level to at least `at_least`.
    pub fn because(&mut self, at_least: Level, reason: impl Into<String>) {
        self.level = self.level.max(at_least);
        self.reasons.push(reason.into());
    }
}

// ---------------------------------------------------------------------------------------------
// A3: files that deserve care
// ---------------------------------------------------------------------------------------------

/// Built-in patterns and what they are. Yours (Settings) add to them.
pub const SENSITIVE: [(&str, &str); 12] = [
    (".github/workflows/**", "CI/CD workflow"),
    (".github/actions/**", "CI action"),
    ("**/CODEOWNERS", "code owners"),
    ("**/*.tf", "infrastructure (Terraform)"),
    ("**/*.tfvars", "infrastructure (Terraform)"),
    ("**/migrations/**", "database migration"),
    ("**/migrate/**", "database migration"),
    ("**/Dockerfile*", "container image"),
    ("**/k8s/**", "Kubernetes"),
    ("**/helm/**", "Kubernetes (Helm)"),
    ("**/.env*", "environment / secrets file"),
    ("**/*.pem", "certificate or key"),
];

/// Lock files only count when a lot of them changed.
const LOCKFILES: [&str; 6] = ["package-lock.json", "yarn.lock", "pnpm-lock.yaml", "Cargo.lock", "go.sum", "poetry.lock"];

/// `*` matches within one path segment, `**` any number of segments. Case-sensitive, `/` paths.
pub fn glob(pattern: &str, path: &str) -> bool {
    fn seg(p: &str, s: &str) -> bool {
        // `*` within a segment: a simple backtracking match.
        let (p, s) = (p.as_bytes(), s.as_bytes());
        let (mut pi, mut si, mut star, mut mark) = (0, 0, None, 0);
        while si < s.len() {
            if pi < p.len() && (p[pi] == s[si] || p[pi] == b'?') {
                pi += 1;
                si += 1;
            } else if pi < p.len() && p[pi] == b'*' {
                star = Some(pi);
                mark = si;
                pi += 1;
            } else if let Some(st) = star {
                pi = st + 1;
                mark += 1;
                si = mark;
            } else {
                return false;
            }
        }
        while pi < p.len() && p[pi] == b'*' {
            pi += 1;
        }
        pi == p.len()
    }
    fn go(p: &[&str], s: &[&str]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some(&"**") => (0..=s.len()).any(|i| go(&p[1..], &s[i..])),
            Some(first) => !s.is_empty() && seg(first, s[0]) && go(&p[1..], &s[1..]),
        }
    }
    let p: Vec<&str> = pattern.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    go(&p, &s)
}

/// The changed files that deserve care, with what they are: built-in patterns, yours, and lock
/// files with more than 500 changed lines. `files`: (path, lines changed).
pub fn sensitive_files<'a>(files: impl IntoIterator<Item = (&'a str, u64)>, extra: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (path, changed) in files {
        let name = path.rsplit('/').next().unwrap_or(path);
        if LOCKFILES.contains(&name) {
            if changed > 500 {
                out.push((path.to_owned(), tf!("lock file, {changed} lines")));
            }
            continue;
        }
        if let Some((_, what)) = SENSITIVE.iter().find(|(p, _)| glob(p, path)) {
            out.push((path.to_owned(), (*what).to_owned()));
        } else if extra.iter().any(|p| !p.trim().is_empty() && glob(p.trim(), path)) {
            out.push((path.to_owned(), "your sensitive pattern".to_owned()));
        }
    }
    out
}

/// Short labels for a PR list chip: "workflows", "migration"...
pub fn chip_labels(found: &[(String, String)]) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (_, what) in found {
        let label = match what.as_str() {
            w if w.starts_with("CI") => "workflows",
            w if w.contains("migration") => "migration",
            w if w.contains("infrastructure") || w.contains("Kubernetes") || w.contains("container") => "infra",
            w if w.contains("secrets") || w.contains("key") => "secrets file",
            w if w.contains("owners") => "owners",
            w if w.starts_with("lock") => "big lockfile",
            _ => "sensitive",
        };
        if !out.contains(&label) {
            out.push(label);
        }
    }
    out
}

/// Added lines in a unified diff that look like secrets: (file, line text, what).
pub fn secret_lines(diff: &str) -> Vec<(String, String, &'static str)> {
    let mut out = Vec::new();
    let mut file = String::new();
    for line in diff.lines() {
        if let Some(f) = line.strip_prefix("+++ b/") {
            file = f.to_owned();
            continue;
        }
        let Some(added) = line.strip_prefix('+') else { continue };
        if added.starts_with("++") {
            continue;
        }
        if let Some(what) = looks_secret(added) {
            let shown: String = added.trim().chars().take(120).collect();
            out.push((file.clone(), shown, what));
        }
    }
    out
}

fn looks_secret(line: &str) -> Option<&'static str> {
    let l = line.trim();
    if l.contains("-----BEGIN") && l.contains("PRIVATE KEY") {
        return Some("private key");
    }
    for (prefix, what, min) in [
        ("AKIA", "AWS access key", 16),
        ("ghp_", "GitHub token", 30),
        ("gho_", "GitHub token", 30),
        ("github_pat_", "GitHub token", 30),
        ("xoxb-", "Slack token", 20),
        ("xoxp-", "Slack token", 20),
        ("sk_live_", "Stripe live key", 20),
        ("AIza", "Google API key", 30),
    ] {
        if let Some(i) = l.find(prefix) {
            let tail = &l[i + prefix.len()..];
            let run = tail.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').count();
            if run >= min {
                return Some(what);
            }
        }
    }
    // `password = "…"`, `apiKey: '…'`: a secret-ish name assigned a long, mixed literal.
    let lower = l.to_lowercase();
    let named = ["password", "passwd", "secret", "api_key", "apikey", "token", "private_key", "access_key"].iter().any(|n| lower.contains(n));
    if named && (l.contains('=') || l.contains(':')) {
        for q in ['"', '\''] {
            let mut parts = l.split(q);
            parts.next();
            if let Some(value) = parts.next()
                && value.len() >= 12
                && !value.contains(' ')
                && !value.starts_with("${")
                && !value.starts_with('<')
                && mixed(value)
            {
                return Some("hard-coded credential");
            }
        }
    }
    None
}

/// Letters and digits both: a password, not a word or a placeholder.
fn mixed(v: &str) -> bool {
    v.chars().any(|c| c.is_ascii_digit()) && v.chars().any(|c| c.is_ascii_alphabetic())
}

// ---------------------------------------------------------------------------------------------
// A4: production
// ---------------------------------------------------------------------------------------------

/// An environment counts as production when its name is one of yours (`prod`, `production`...)
/// or it has deployment protection (required reviewers or a wait timer).
pub fn is_production(name: &str, protected: bool, names: &[String]) -> bool {
    let n = name.to_lowercase();
    protected || names.iter().any(|p| !p.trim().is_empty() && n == p.trim().to_lowercase())
}

/// A workflow that deploys, by its name.
pub fn looks_like_deploy(workflow: &str) -> bool {
    let w = workflow.to_lowercase();
    ["deploy", "release", "publish", "prod", "rollout", "ship"].iter().any(|k| w.contains(k))
}

/// Branch names never offered for cleanup and treated as important.
pub fn important_branch(name: &str, default: &str, protected: &[String]) -> bool {
    // Only a remote's prefix comes off: "release/1.2" is a branch name, not remote + branch.
    let short = name.strip_prefix("origin/").or_else(|| name.strip_prefix("upstream/")).unwrap_or(name);
    short == default
        || protected.iter().any(|p| p == short)
        || ["main", "master", "develop", "dev", "production", "prod", "staging"].contains(&short)
        || short.starts_with("release")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob(".github/workflows/**", ".github/workflows/deploy.yml"));
        assert!(glob("**/migrations/**", "api/db/migrations/0001_init.sql"));
        assert!(glob("**/*.tf", "main.tf"));
        assert!(glob("**/*.tf", "infra/prod/main.tf"));
        assert!(glob("**/Dockerfile*", "Dockerfile.prod"));
        assert!(!glob("**/*.tf", "main.tfstate.txt"));
        assert!(!glob(".github/workflows/**", "src/.github.rs"));
    }

    #[test]
    fn sensitive_paths_and_lockfiles() {
        let found = sensitive_files([(".github/workflows/x.yml", 3), ("src/main.rs", 40), ("package-lock.json", 900), ("Cargo.lock", 20)], &[]);
        let paths: Vec<&str> = found.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, [".github/workflows/x.yml", "package-lock.json"]);
        assert_eq!(chip_labels(&found), ["workflows", "big lockfile"]);
        let mine = sensitive_files([("config/payment/keys.yaml", 1)], &["config/payment/**".to_owned()]);
        assert_eq!(mine.len(), 1);
    }

    #[test]
    fn secrets_in_added_lines_only() {
        let diff = "+++ b/app.py\n+aws = 'AKIAABCDEFGHIJKLMNOP'\n-old = 'ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\n+password = \"hunter2hunter2x9\"\n+name = \"just some words here\"\n+token = \"${{ secrets.TOKEN }}\"\n";
        let found = secret_lines(diff);
        let kinds: Vec<&str> = found.iter().map(|f| f.2).collect();
        assert_eq!(kinds, ["AWS access key", "hard-coded credential"]);
        assert_eq!(found[0].0, "app.py");
    }

    #[test]
    fn production_and_branches() {
        let names = vec!["prod".to_owned(), "production".to_owned()];
        assert!(is_production("Production", false, &names));
        assert!(is_production("eu-west", true, &names));
        assert!(!is_production("staging", false, &names));
        assert!(looks_like_deploy("Deploy to DigitalOcean"));
        assert!(!looks_like_deploy("Tests"));
        assert!(important_branch("origin/main", "main", &[]));
        assert!(important_branch("release/1.2", "main", &[]));
        assert!(!important_branch("feature/login", "main", &[]));
    }

    #[test]
    fn risk_only_goes_up() {
        let mut r = Risk::new(Level::Medium);
        r.because(Level::Low, "minor");
        assert_eq!(r.level, Level::Medium);
        r.because(Level::High, "protected");
        assert_eq!(r.level, Level::High);
        assert_eq!(r.reasons.len(), 2);
    }
}
