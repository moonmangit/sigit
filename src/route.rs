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

/// One step of a route while it's being edited: the same fields as a `RouteTarget`, as text
/// fields.
pub struct RouteRow {
    pub base: rune::TextEdit,
    pub draft: bool,
    pub title: rune::TextEdit,
    pub body: rune::TextEdit,
    pub reviewers: rune::TextEdit,
    pub assignees: rune::TextEdit,
    pub labels: rune::TextEdit,
}

impl RouteRow {
    pub fn blank() -> Self {
        rows_of(&[RouteTarget::default()]).remove(0)
    }
}

pub fn rows_of(targets: &[RouteTarget]) -> Vec<RouteRow> {
    targets
        .iter()
        .map(|t| RouteRow {
            base: rune::TextEdit::new(&t.base),
            draft: t.draft,
            title: rune::TextEdit::new(&t.title),
            body: rune::TextEdit::new(&t.body),
            reviewers: rune::TextEdit::new(&t.reviewers),
            assignees: rune::TextEdit::new(&t.assignees),
            labels: rune::TextEdit::new(&t.labels),
        })
        .collect()
}

/// The rows as targets, every field trimmed.
pub fn targets_of(rows: &[RouteRow]) -> Vec<RouteTarget> {
    rows.iter()
        .map(|r| RouteTarget {
            base: r.base.text.trim().to_owned(),
            draft: r.draft,
            title: r.title.text.trim().to_owned(),
            body: r.body.text.trim().to_owned(),
            reviewers: r.reviewers.text.trim().to_owned(),
            assignees: r.assignees.text.trim().to_owned(),
            labels: r.labels.text.trim().to_owned(),
        })
        .collect()
}

/// The rows as a route, or why they can't be one: every step needs a branch, none twice.
pub fn validate_route(rows: &[RouteRow]) -> Result<Vec<RouteTarget>, String> {
    let targets = targets_of(rows);
    for (i, t) in targets.iter().enumerate() {
        if t.base.is_empty() {
            return Err(crate::i18n::t("Every route step needs a target branch").to_string());
        }
        if targets[..i].iter().any(|u| u.base == t.base) {
            return Err(crate::tf!("{branch} is on the route twice", branch = t.base));
        }
    }
    Ok(targets)
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

/// Whether merging `head` into `base` should delete `head` by default.
pub fn merge_deletes_branch(prefs: &Prefs, slug: &str, head: &str, base: &str) -> bool {
    kept_for(prefs, slug, head, base).is_none()
}

/// Where one route step stands, from its newest PR alone.
#[derive(Clone, Debug, PartialEq)]
pub enum StepState {
    None,
    Open { draft: bool },
    Merged,
    Closed,
}

/// One route target with its newest PR, how many PRs the branch has into it, and the state.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub base: String,
    pub latest: Option<BranchPr>,
    pub count: usize,
    pub state: StepState,
}

/// One `Step` per target in route order, plus the open PRs into bases that aren't on the route.
pub fn steps(targets: &[RouteTarget], prs: &[BranchPr]) -> (Vec<Step>, Vec<BranchPr>) {
    let steps = targets
        .iter()
        .map(|t| {
            let mine: Vec<&BranchPr> = prs.iter().filter(|p| p.base_ref_name == t.base).collect();
            let latest = mine.iter().max_by(|a, b| a.created_at.cmp(&b.created_at)).map(|p| (*p).clone());
            let state = match latest.as_ref().map(|p| (p.state.as_str(), p.is_draft)) {
                Some(("OPEN", draft)) => StepState::Open { draft },
                Some(("MERGED", _)) => StepState::Merged,
                Some(_) => StepState::Closed,
                None => StepState::None,
            };
            Step { base: t.base.clone(), latest, count: mine.len(), state }
        })
        .collect();
    let off = prs
        .iter()
        .filter(|p| p.state == "OPEN" && !targets.iter().any(|t| t.base == p.base_ref_name))
        .cloned()
        .collect();
    (steps, off)
}

/// A PR can be opened for step `i` when it has none (or a closed one) and the step before is merged.
pub fn can_open(steps: &[Step], i: usize) -> bool {
    let free = matches!(steps.get(i).map(|s| &s.state), Some(StepState::None | StepState::Closed));
    free && blocked_by(steps, i).is_none()
}

/// The earlier step that must merge first: the one before `i` when it isn't merged.
pub fn blocked_by(steps: &[Step], i: usize) -> Option<String> {
    let prev = steps.get(i.checked_sub(1)?)?;
    (prev.state != StepState::Merged).then(|| prev.base.clone())
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

    fn pr(n: u64, state: &str, draft: bool, base: &str, at: &str) -> BranchPr {
        BranchPr {
            number: n,
            state: state.into(),
            is_draft: draft,
            base_ref_name: base.into(),
            created_at: at.into(),
            url: String::new(),
        }
    }

    fn route() -> Vec<RouteTarget> {
        vec![t("staging"), t("main")]
    }

    #[test]
    fn steps_follow_newest_pr() {
        let prs = [
            pr(160, "OPEN", false, "staging", "2026-11-19T10:00:00Z"),
            pr(142, "MERGED", false, "staging", "2026-11-17T10:00:00Z"),
        ];
        let (s, _) = steps(&route(), &prs);
        assert_eq!(s[0].state, StepState::Open { draft: false });
        assert_eq!(s[0].count, 2);
        assert_eq!(s[0].latest.as_ref().unwrap().number, 160);
        assert_eq!(s[1].state, StepState::None);
        assert!(!can_open(&s, 1));
        assert_eq!(blocked_by(&s, 1), Some("staging".to_string()));
    }

    #[test]
    fn steps_merged_unlocks_next() {
        let prs = [pr(142, "MERGED", false, "staging", "2026-11-17T10:00:00Z")];
        let (s, _) = steps(&route(), &prs);
        assert!(can_open(&s, 1));
        assert_eq!(blocked_by(&s, 1), None);
        assert!(!can_open(&s, 0));
    }

    #[test]
    fn closed_step_can_reopen() {
        let prs = [pr(150, "CLOSED", false, "staging", "2026-11-17T10:00:00Z")];
        let (s, _) = steps(&route(), &prs);
        assert_eq!(s[0].state, StepState::Closed);
        assert!(can_open(&s, 0));
    }

    #[test]
    fn draft_state_kept() {
        let prs = [pr(170, "OPEN", true, "main", "2026-11-19T10:00:00Z")];
        let (s, _) = steps(&route(), &prs);
        assert_eq!(s[1].state, StepState::Open { draft: true });
    }

    #[test]
    fn steps_off_route_open_prs_listed() {
        let prs = [
            pr(180, "OPEN", false, "hotfix", "2026-11-19T10:00:00Z"),
            pr(181, "MERGED", false, "hotfix", "2026-11-18T10:00:00Z"),
        ];
        let (_, off) = steps(&route(), &prs);
        assert_eq!(off.iter().map(|p| p.number).collect::<Vec<_>>(), vec![180]);
    }

    #[test]
    fn merge_keeps_branch_before_last_step() {
        let p = prefs();
        assert!(!merge_deletes_branch(&p, "b/y", "feat-a", "staging"));
        assert!(merge_deletes_branch(&p, "b/y", "feat-a", "main"));
        assert!(merge_deletes_branch(&Prefs::default(), "b/y", "feat-a", "staging"));
    }
}
