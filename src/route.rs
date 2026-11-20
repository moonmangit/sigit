//! PR routes: the ordered target branches a feature branch ships through, each with a PR preset.

use serde::{Deserialize, Serialize};

use crate::config::Prefs;

/// One step of a route: the branch a PR goes into, and how that PR is filled in.
/// The lists stay as the comma-separated text the user typed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RouteTarget {
    pub base: String,
    pub draft: bool,
    pub title: String,
    pub body: String,
    pub reviewers: String,
    pub assignees: String,
    pub labels: String,
}

/// A repo's own route, replacing the general one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RepoRoute {
    pub slug: String,
    pub targets: Vec<RouteTarget>,
}

/// The repo's own targets and `true` if it has an override, else the general ones and `false`.
pub fn route_for<'a>(prefs: &'a Prefs, slug: &str) -> (&'a [RouteTarget], bool) {
    match prefs.pr_route_overrides.iter().find(|r| r.slug == slug) {
        Some(r) => (&r.targets, true),
        None => (&prefs.pr_route, false),
    }
}

/// A branch as GitHub knows it: one leading `origin/` removed.
pub fn head_name(branch: &str) -> &str {
    branch.strip_prefix("origin/").unwrap_or(branch)
}

/// Replaces `{branch}`, `{target}` and `{subject}`; other braces stay as typed.
pub fn fill(template: &str, branch: &str, target: &str, subject: &str) -> String {
    template
        .replace("{branch}", branch)
        .replace("{target}", target)
        .replace("{subject}", subject)
}

/// The PR title: the target's title filled in, or the subject when it is blank.
pub fn title_for(t: &RouteTarget, branch: &str, subject: &str) -> String {
    if t.title.trim().is_empty() {
        subject.to_string()
    } else {
        fill(&t.title, branch, &t.base, subject)
    }
}

/// A comma-separated list as typed: trimmed, blanks dropped.
pub fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// Everything `gh pr create` needs for one route step.
#[derive(Clone, Debug, PartialEq)]
pub struct PrSpec {
    pub slug: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
    pub title: String,
    pub body: String,
    pub reviewers: String,
    pub assignees: String,
    pub labels: String,
}

/// A PR of a branch as `gh pr list --json` reports it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BranchPr {
    pub number: u64,
    pub state: String,
    #[serde(default)]
    pub is_draft: bool,
    pub base_ref_name: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub url: String,
}

/// When `base` is a route target that isn't the last, and `head` isn't itself a route target,
/// the next target's name: the feature branch must be kept for the next step.
pub fn kept_for(prefs: &Prefs, slug: &str, head: &str, base: &str) -> Option<String> {
    let (targets, _) = route_for(prefs, slug);
    if targets.iter().any(|t| t.base == head) {
        return None;
    }
    let i = targets.iter().position(|t| t.base == base)?;
    targets.get(i + 1).map(|t| t.base.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Prefs;

    #[test]
    fn branch_pr_parses_gh_json() {
        let j = r#"[{"number":142,"state":"MERGED","isDraft":false,"baseRefName":"staging","createdAt":"2026-11-17T10:00:00Z","url":"u"}]"#;
        let v = serde_json::from_str::<Vec<BranchPr>>(j).unwrap();
        assert_eq!((v[0].number, v[0].state.as_str(), v[0].base_ref_name.as_str()), (142, "MERGED", "staging"));
    }

    fn t(base: &str) -> RouteTarget {
        RouteTarget { base: base.into(), ..Default::default() }
    }

    fn prefs() -> Prefs {
        Prefs {
            pr_route: vec![t("staging"), t("main")],
            pr_route_overrides: vec![RepoRoute { slug: "a/x".into(), targets: vec![t("qa")] }],
            ..Prefs::default()
        }
    }

    #[test]
    fn route_for_prefers_repo_override() {
        let p = prefs();
        assert_eq!(route_for(&p, "a/x"), (&[t("qa")][..], true));
        assert_eq!(route_for(&p, "b/y"), (&[t("staging"), t("main")][..], false));
    }

    #[test]
    fn head_name_strips_one_origin() {
        assert_eq!(head_name("origin/feat/a"), "feat/a");
        assert_eq!(head_name("feat/a"), "feat/a");
        assert_eq!(head_name("origin/origin/x"), "origin/x");
    }

    #[test]
    fn fill_replaces_placeholders() {
        assert_eq!(
            fill("{branch} → {target}: {subject}", "feat-a", "main", "Add login"),
            "feat-a → main: Add login"
        );
    }

    #[test]
    fn fill_keeps_unknown_placeholders() {
        assert_eq!(fill("{ticket} {branch}", "f", "m", "s"), "{ticket} f");
    }

    #[test]
    fn title_defaults_to_subject() {
        let mut x = t("main");
        x.title = "  ".into();
        assert_eq!(title_for(&x, "feat-a", "Add login"), "Add login");
        x.title = "[main] {branch}".into();
        assert_eq!(title_for(&x, "feat-a", "Add login"), "[main] feat-a");
    }

    #[test]
    fn split_list_trims_and_drops_blanks() {
        assert_eq!(split_list(" lead, ,team/x "), vec!["lead", "team/x"]);
    }

    #[test]
    fn kept_for_only_before_last_step() {
        let p = prefs();
        let s = "b/y";
        assert_eq!(kept_for(&p, s, "feat-a", "staging"), Some("main".to_string()));
        assert_eq!(kept_for(&p, s, "feat-a", "main"), None);
        assert_eq!(kept_for(&p, s, "feat-a", "dev"), None);
        assert_eq!(kept_for(&p, s, "staging", "staging"), None);
    }

    #[test]
    fn old_prefs_load_without_route() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert!(p.pr_route.is_empty());
        assert!(p.pr_route_overrides.is_empty());
    }
}
