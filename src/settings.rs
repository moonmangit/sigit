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
        self.open_cmp_rows();
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

    /// Fill the Settings → Git comparison rows from the saved comparisons.
    pub fn open_cmp_rows(&mut self) {
        let cs = &self.prefs.comparisons;
        self.cmp_rows = cs.iter().map(|c| (TextEdit::new(&c.name), TextEdit::new(&c.from), TextEdit::new(&c.to))).collect();
        self.cmp_ids = cs.iter().map(|c| Some(c.name.clone())).collect();
    }

    pub fn cmp_add(&mut self) {
        self.cmp_rows.push((TextEdit::new(""), TextEdit::new(""), TextEdit::new("")));
        self.cmp_ids.push(None);
    }

    pub fn cmp_remove(&mut self, i: usize) {
        if i < self.cmp_rows.len() {
            self.cmp_rows.remove(i);
            self.cmp_ids.remove(i);
        }
    }

    /// Move row `i` one place up (`-1`) or down (`1`), its identity with it.
    pub fn cmp_move(&mut self, i: usize, by: isize) {
        let j = i.checked_add_signed(by).filter(|j| *j < self.cmp_rows.len());
        if let (Some(j), true) = (j, i < self.cmp_rows.len()) {
            self.cmp_rows.swap(i, j);
            self.cmp_ids.swap(i, j);
        }
    }

    /// Exchange row `i`'s From and To.
    pub fn cmp_swap(&mut self, i: usize) {
        if let Some(r) = self.cmp_rows.get_mut(i) {
            std::mem::swap(&mut r.1, &mut r.2);
        }
    }

    /// Check the rows, apply renames and removals to the comparisons and their overrides, then
    /// save. On an error nothing changes.
    pub fn save_comparisons(&mut self) -> Result<(), String> {
        let rows: Vec<(String, String, String)> =
            self.cmp_rows.iter().map(|(n, f, t)| (n.text.trim().to_owned(), f.text.trim().to_owned(), t.text.trim().to_owned())).collect();
        for (i, (n, f, t)) in rows.iter().enumerate() {
            if n.is_empty() || f.is_empty() || t.is_empty() {
                return Err(crate::i18n::t("Every comparison needs a name and two branches").to_string());
            }
            if rows[..i].iter().any(|(m, ..)| m == n) {
                return Err(crate::tf!("Two comparisons are called {name}", name = n));
            }
            if f == t {
                return Err(crate::tf!("{name} compares {branch} with itself", name = n, branch = f));
            }
        }
        for old in self.prefs.comparisons.iter().map(|c| c.name.clone()).collect::<Vec<_>>() {
            if !self.cmp_ids.iter().flatten().any(|id| *id == old) {
                crate::compare::delete(&mut self.prefs, &old);
            }
        }
        // Renames go through a placeholder, so two rows trading names don't merge their overrides.
        let renamed: Vec<(usize, String, String)> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, (n, ..))| self.cmp_ids[i].as_ref().filter(|id| *id != n).map(|id| (i, id.clone(), n.clone())))
            .collect();
        for (i, old, _) in &renamed {
            crate::compare::rename(&mut self.prefs, old, &format!("\u{0}{i}"));
        }
        for (i, _, new) in &renamed {
            crate::compare::rename(&mut self.prefs, &format!("\u{0}{i}"), new);
        }
        self.prefs.comparisons = rows.into_iter().map(|(name, from, to)| crate::compare::Comparison { name, from, to }).collect();
        self.open_cmp_rows();
        self.save_prefs();
        self.toast(crate::i18n::t("Comparisons saved"), false);
        self.refresh_compare();
        Ok(())
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
        self.act(crate::i18n::t(tf!("Switching to {login}")), move || gh::switch_account(&host, &login), |s| {
            s.load_accounts();
            s.refresh_page();
        });
    }

    /// Write user.name / user.email to your global git config.
    pub fn save_git_identity(&mut self) {
        let (name, email) = (self.git_name.text.trim().to_owned(), self.git_email.text.trim().to_owned());
        if !email.is_empty() && !email.contains('@') {
            self.toast(crate::i18n::t("That email doesn't look right"), true);
            return;
        }
        let saved = (name.clone(), email.clone());
        self.act(crate::i18n::t(
            "Saving git identity"),
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
            Ok(()) => self.toast(crate::i18n::t("Cache cleared. The next launch starts empty and loads everything fresh."), false),
            Err(e) => self.toast(crate::i18n::t(tf!("Couldn't clear the cache: {e}")), true),
        }
    }

    /// Back to the default widths of every resizable section.
    pub fn reset_layout(&mut self) {
        self.prefs.splits.clear();
        self.save_prefs();
        self.toast(crate::i18n::t("Section sizes reset"), false);
    }
}
