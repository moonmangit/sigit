//! "Check for updates" (Settings → About): ask the project's public GitHub releases for the
//! latest version and compare it with this build's. Nothing is downloaded or installed here;
//! a newer version offers its release page.

use crate::{State, gh, jobs};

/// The repository releases come from: `repository` in Cargo.toml.
pub fn repo() -> &'static str {
    env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/").trim_end_matches('/')
}

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Where the last check got to.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Update {
    #[default]
    NotChecked,
    Checking,
    /// This build is the latest release (or newer).
    UpToDate,
    /// A newer release: its version and page.
    Available { version: String, url: String },
    Failed(String),
}

/// "v1.2.3" or "1.2.3" → (1, 2, 3); anything after a '-' or '+' is ignored.
pub fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?))
}

/// The latest published release (not a draft or pre-release): (tag, page).
fn latest() -> Result<(String, String), String> {
    let out = gh::run("gh", &["api", &format!("repos/{}/releases/latest", repo())], None).map_err(|e| if e.contains("404") || e.contains("Not Found") { "No releases published yet".to_owned() } else { e })?;
    let v: serde_json::Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("unexpected answer from GitHub")?.to_owned();
    let url = v["html_url"].as_str().unwrap_or_default().to_owned();
    Ok((tag, url))
}

/// Compare a release tag with this build.
pub fn verdict(tag: &str, url: String) -> Update {
    match (parse(tag), parse(current())) {
        (Some(latest), Some(mine)) if latest > mine => Update::Available { version: tag.trim_start_matches('v').to_owned(), url },
        (Some(_), Some(_)) => Update::UpToDate,
        _ => Update::Failed(format!("Couldn't read the version \"{tag}\"")),
    }
}

impl State {
    pub fn check_for_updates(&mut self) {
        if self.update == Update::Checking {
            return;
        }
        self.update = Update::Checking;
        jobs::spawn(self, latest, |s, r| {
            s.update = match r {
                Ok((tag, url)) => verdict(&tag, url),
                Err(e) => Update::Failed(e),
            };
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse("1.2"), Some((1, 2, 0)));
        assert_eq!(parse("v2.0.1-beta.1"), Some((2, 0, 1)));
        assert_eq!(parse("latest"), None);
    }

    #[test]
    fn newer_only_when_greater() {
        let url = String::from("https://example.test/r");
        assert_eq!(verdict(&format!("v{}", current()), url.clone()), Update::UpToDate);
        assert!(matches!(verdict("v999.0.0", url.clone()), Update::Available { ref version, .. } if version == "999.0.0"));
        assert_eq!(verdict("v0.0.1", url.clone()), Update::UpToDate);
        assert!(matches!(verdict("nightly", url), Update::Failed(_)));
        assert!(!repo().is_empty() && !repo().starts_with("https://"));
    }
}
