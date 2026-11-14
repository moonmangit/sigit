//! The Settings window: what it loads when it opens (signed-in accounts, your git identity, tool
//! versions, the activity log) and what its buttons do. The view is `ui::settings`.

use rune::TextEdit;

use crate::{State, activity, cache, gh, git, jobs, model};

/// The window's sections, in the order its sidebar lists them.
pub const SECTIONS: [&str; 9] = ["Appearance", "Accounts", "Git", "Safety", "Notifications", "Data & storage", "Activity log", "Shortcuts", "About"];
pub const NOTIFICATIONS: usize = 4;

impl State {
    /// Open the Settings window at `section` (an index into `SECTIONS`).
    pub fn open_settings(&mut self, section: usize) {
        self.settings_sec = section.min(SECTIONS.len() - 1);
        if self.settings_open {
            return;
        }
        self.settings_open = true;
        self.palette = false;
        let t = model::now();
        jobs::spawn(self, move || activity::recent(t), |s, v| s.activity = v);
        self.load_accounts();
        jobs::spawn(self, || Ok::<_, String>((git::global_identity(), gh::versions())), |s, r| {
            if let Ok(((name, email), versions)) = r {
                s.git_name = TextEdit::new(name.clone());
                s.git_email = TextEdit::new(email.clone());
                s.git_saved = (name, email);
                s.versions = Some(versions);
            }
        });
    }

    pub fn close_settings(&mut self) {
        self.settings_open = false;
    }

    pub fn load_accounts(&mut self) {
        self.accounts.loading = true;
        jobs::spawn(self, gh::accounts, |s, r| s.accounts.finish(r));
    }

    /// Make another signed-in account the one gh (and so this app) uses, then reload.
    pub fn switch_account(&mut self, host: String, login: String) {
        self.act(format!("Switching to {login}"), move || gh::switch_account(&host, &login), |s| {
            s.load_accounts();
            s.refresh_page();
        });
    }

    /// Write user.name / user.email to your global git config.
    pub fn save_git_identity(&mut self) {
        let (name, email) = (self.git_name.text.trim().to_owned(), self.git_email.text.trim().to_owned());
        if !email.is_empty() && !email.contains('@') {
            self.toast("That email doesn't look right".into(), true);
            return;
        }
        let saved = (name.clone(), email.clone());
        self.act(
            "Saving git identity".into(),
            move || {
                git::set_global("user.name", &name)?;
                git::set_global("user.email", &email)?;
                Ok("Git identity saved".into())
            },
            move |s| s.git_saved = saved,
        );
    }

    pub fn git_identity_changed(&self) -> bool {
        (self.git_name.text.trim(), self.git_email.text.trim()) != (self.git_saved.0.as_str(), self.git_saved.1.as_str())
    }

    /// Delete the on-disk cache; what's on screen stays until the next launch.
    pub fn clear_cache(&mut self) {
        match cache::clear() {
            Ok(()) => self.toast("Cache cleared. The next launch starts empty and loads everything fresh.".into(), false),
            Err(e) => self.toast(format!("Couldn't clear the cache: {e}"), true),
        }
    }

    /// Back to the default widths of every resizable section.
    pub fn reset_layout(&mut self) {
        self.prefs.splits.clear();
        self.save_prefs();
        self.toast("Section sizes reset".into(), false);
    }
}
