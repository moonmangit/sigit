//! Branch comparisons: shared "from → to" pairs shown on every repo's Dashboard, and the
//! per-repo overrides that retarget or switch one off.

use serde::{Deserialize, Serialize};

use crate::config::Prefs;

/// Counts the commits on `from` that `to` lacks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Comparison {
    pub name: String,
    pub from: String,
    pub to: String,
}

/// One repo's change to a shared comparison, matched by `name`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Override {
    pub slug: String,
    pub name: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub off: bool,
}

pub fn defaults() -> Vec<Comparison> {
    [("To staging", "dev", "staging"), ("Back-merge", "staging", "dev"), ("To production", "staging", "main")]
        .into_iter()
        .map(|(n, f, t)| Comparison { name: n.into(), from: f.into(), to: t.into() })
        .collect()
}

/// One entry per shared comparison, in order: `None` when it is off for this repo.
pub fn effective(shared: &[Comparison], overrides: &[Override], slug: &str) -> Vec<Option<Comparison>> {
    shared
        .iter()
        .map(|c| {
            let o = overrides.iter().find(|o| o.slug == slug && o.name == c.name);
            match o {
                Some(o) if o.off => None,
                Some(o) => Some(Comparison {
                    name: c.name.clone(),
                    from: o.from.clone().unwrap_or_else(|| c.from.clone()),
                    to: o.to.clone().unwrap_or_else(|| c.to.clone()),
                }),
                None => Some(c.clone()),
            }
        })
        .collect()
}

pub fn is_changed(overrides: &[Override], slug: &str, name: &str) -> bool {
    overrides.iter().any(|o| o.slug == slug && o.name == name)
}

pub fn rename(prefs: &mut Prefs, old: &str, new: &str) {
    for c in prefs.comparisons.iter_mut().filter(|c| c.name == old) {
        c.name = new.into();
    }
    for o in prefs.comparison_overrides.iter_mut().filter(|o| o.name == old) {
        o.name = new.into();
    }
}

pub fn delete(prefs: &mut Prefs, name: &str) {
    prefs.comparisons.retain(|c| c.name != name);
    prefs.comparison_overrides.retain(|o| o.name != name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides() -> Vec<Override> {
        vec![
            Override { slug: "a/b".into(), name: "To production".into(), from: None, to: Some("master".into()), off: false },
            Override { slug: "a/b".into(), name: "Back-merge".into(), off: true, ..Default::default() },
            Override { slug: "x/y".into(), name: "To staging".into(), off: true, ..Default::default() },
        ]
    }

    #[test]
    fn defaults_are_the_users_three() {
        let d = defaults();
        assert_eq!(d.iter().map(|c| (c.name.as_str(), c.from.as_str(), c.to.as_str())).collect::<Vec<_>>(),
            vec![("To staging", "dev", "staging"), ("Back-merge", "staging", "dev"), ("To production", "staging", "main")]);
    }

    #[test]
    fn effective_applies_overrides() {
        let shared = defaults();
        let ov = overrides();
        let e = effective(&shared, &ov, "a/b");
        assert_eq!(e[0].as_ref().unwrap().from, "dev");
        assert!(e[1].is_none());
        assert_eq!((e[2].as_ref().unwrap().from.as_str(), e[2].as_ref().unwrap().to.as_str()), ("staging", "master"));
        assert!(is_changed(&ov, "a/b", "To production") && !is_changed(&ov, "a/b", "To staging"));
    }

    #[test]
    fn rename_carries_overrides() {
        let mut p = Prefs { comparison_overrides: overrides(), ..Prefs::default() };
        rename(&mut p, "Back-merge", "Hotfix back");
        assert_eq!(p.comparisons[1].name, "Hotfix back");
        assert!(p.comparison_overrides.iter().any(|o| o.slug == "a/b" && o.name == "Hotfix back" && o.off));
        assert!(!p.comparison_overrides.iter().any(|o| o.name == "Back-merge"));
        assert!(effective(&p.comparisons, &p.comparison_overrides, "a/b")[1].is_none());
    }

    #[test]
    fn delete_drops_overrides() {
        let mut p = Prefs { comparison_overrides: overrides(), ..Prefs::default() };
        delete(&mut p, "To production");
        assert_eq!(p.comparisons.len(), 2);
        assert!(!p.comparison_overrides.iter().any(|o| o.name == "To production"));
        assert_eq!(p.comparison_overrides.len(), 2);
    }

    #[test]
    fn old_prefs_get_default_comparisons() {
        let p: Prefs = serde_json::from_str(r#"{"pinned":["a/b"]}"#).unwrap();
        assert_eq!(p.comparisons, defaults());
        assert!(p.comparison_overrides.is_empty() && p.clone_folders.is_empty());
    }
}
