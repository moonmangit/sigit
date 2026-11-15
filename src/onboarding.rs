//! First run and preflight: before anything loads, check that git and the GitHub CLI are
//! installed and that gh is signed in. Missing anything, the window shows a setup screen that
//! says how to fix each step and re-checks by itself; all set on a first run, a short welcome.
//! The view is `ui::onboarding`.

use std::time::Instant;

use crate::{State, gh, jobs};

/// What the preflight found. Each step is `Ok(what's there)` or `Err(why not)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Preflight {
    pub git: Result<String, String>,
    pub gh: Result<String, String>,
    /// The signed-in account ("octocat on github.com").
    pub auth: Result<String, String>,
}

impl Preflight {
    pub fn ready(&self) -> bool {
        self.git.is_ok() && self.gh.is_ok() && self.auth.is_ok()
    }
}

/// Where the app is in getting set up.
#[derive(Clone, Debug, PartialEq)]
pub enum Setup {
    /// The first check is running: the window shows last session's data meanwhile.
    Checking,
    /// Something's missing: the setup screen.
    Missing(Preflight),
    /// All set on a first run: the welcome screen.
    Welcome(Preflight),
    Ready,
}

impl Setup {
    /// The setup or welcome screen takes the window.
    pub fn covers_app(&self) -> bool {
        matches!(self, Setup::Missing(_) | Setup::Welcome(_))
    }
}

/// Check git, gh and gh's sign-in (blocking; run it on a worker).
pub fn preflight() -> Preflight {
    let version = |p: &str| match gh::run(p, &["--version"], None) {
        Ok(out) => Ok(out.lines().next().unwrap_or(p).trim().to_owned()),
        Err(_) => Err(tf!("`{p}` isn't installed, or isn't on your PATH")),
    };
    let git = version("git");
    let gh = version("gh");
    let auth = match &gh {
        Err(_) => Err("Install the GitHub CLI first".to_owned()),
        Ok(_) => signed_in(),
    };
    Preflight { git, gh, auth }
}

fn signed_in() -> Result<String, String> {
    match gh::accounts() {
        Ok(list) => match list.iter().find(|a| a.active) {
            Some(a) if a.ok => Ok(tf!("{} on {}", a.login, a.host)),
            Some(a) => Err(tf!("The sign-in for {} on {} has expired", a.login, a.host)),
            None => Err("Not signed in to GitHub".to_owned()),
        },
        // Older gh has no `auth status --json`: fall back to its exit status.
        Err(e) if e.contains("unknown flag") => gh::run("gh", &["auth", "status"], None).map(|_| "signed in".to_owned()).map_err(|_| "Not signed in to GitHub".to_owned()),
        Err(_) => Err("Not signed in to GitHub".to_owned()),
    }
}

/// How to install a tool here: (command, where to download it).
pub fn install_hint(tool: &str) -> (Option<&'static str>, &'static str) {
    let windows = cfg!(windows);
    let mac = cfg!(target_os = "macos");
    match tool {
        "git" => (
            if windows {
                Some("winget install --id Git.Git -e")
            } else if mac {
                Some("brew install git")
            } else {
                None
            },
            "https://git-scm.com/downloads",
        ),
        _ => (
            if windows {
                Some("winget install --id GitHub.cli -e")
            } else if mac {
                Some("brew install gh")
            } else {
                None
            },
            "https://cli.github.com",
        ),
    }
}

/// The sign-in command; gh walks you through it in a terminal (browser or token).
pub const LOGIN: &str = "gh auth login";

/// Open a terminal window running `command` (Windows and macOS; elsewhere, copy it instead).
pub fn open_terminal(command: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd").args(["/c", "start", "sigit setup", "cmd", "/k", command]).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!("tell application \"Terminal\" to do script \"{command}\"");
        std::process::Command::new("osascript").args(["-e", &script, "-e", "tell application \"Terminal\" to activate"]).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = command;
        Err("Open a terminal and run it there".to_owned())
    }
}

/// Seconds between re-checks while the setup screen is up (you're installing or signing in).
pub const RECHECK_SECS: u64 = 4;

impl State {
    /// Run the preflight in the background.
    pub fn run_preflight(&mut self) {
        if self.setup_busy {
            return;
        }
        self.setup_busy = true;
        self.setup_at = Instant::now();
        jobs::spawn(self, || Ok::<_, String>(preflight()), |s, r| {
            if let Ok(p) = r {
                s.preflight_done(p);
            }
        });
    }

    fn preflight_done(&mut self, p: Preflight) {
        self.setup_busy = false;
        if !p.ready() {
            self.setup = Setup::Missing(p);
            return;
        }
        // Data loads once everything's there (it would only fail before).
        if matches!(self.setup, Setup::Checking | Setup::Missing(_)) {
            self.refresh_repo_list();
        }
        let first_run = !self.prefs.seen_onboarding && !self.prefs.seen_guide;
        self.setup = if first_run && !matches!(self.setup, Setup::Ready) { Setup::Welcome(p) } else { Setup::Ready };
    }

    /// Leave the welcome screen (and never show it again).
    pub fn finish_onboarding(&mut self, open_guide: bool) {
        self.setup = Setup::Ready;
        self.prefs.seen_onboarding = true;
        self.prefs.seen_guide = true;
        self.save_prefs();
        self.guide_open = open_guide;
    }

    /// Re-check every few seconds while the setup screen waits on you.
    pub fn poll_setup(&mut self) -> bool {
        if matches!(self.setup, Setup::Missing(_)) && !self.setup_busy && self.setup_at.elapsed().as_secs() >= RECHECK_SECS {
            self.run_preflight();
            return true;
        }
        false
    }

    pub fn run_in_terminal(&mut self, command: &'static str) {
        match open_terminal(command) {
            Ok(()) => self.toast(crate::i18n::t(tf!("Opened a terminal running `{command}`. This screen moves on by itself when it's done.")), false),
            Err(e) => self.toast(crate::i18n::t(tf!("{e}: `{command}`")), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_needs_all_three() {
        let ok = |s: &str| Ok(s.to_owned());
        let all = Preflight { git: ok("git"), gh: ok("gh"), auth: ok("me") };
        assert!(all.ready());
        assert!(!Preflight { auth: Err("no".into()), ..all.clone() }.ready());
        assert!(!Preflight { git: Err("no".into()), ..all }.ready());
        assert!(Setup::Missing(Preflight { git: Err(String::new()), gh: Err(String::new()), auth: Err(String::new()) }).covers_app());
        assert!(!Setup::Checking.covers_app() && !Setup::Ready.covers_app());
    }

    #[test]
    fn install_hints_point_somewhere() {
        for tool in ["git", "gh"] {
            let (_, url) = install_hint(tool);
            assert!(url.starts_with("https://"));
        }
    }
}
