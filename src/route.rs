//! What a pull request is made of: its spec, and a branch's PRs as `gh` reports them.

use serde::Deserialize;

/// A branch as GitHub knows it: one leading `origin/` removed.
pub fn head_name(branch: &str) -> &str {
    branch.strip_prefix("origin/").unwrap_or(branch)
}

/// A comma-separated list as typed: trimmed, blanks dropped.
pub fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// Everything `gh pr create` needs.
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

/// `gh` sends `"mergedAt": null` for a PR that isn't merged.
fn null_as_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
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
    #[serde(default, deserialize_with = "null_as_empty")]
    pub merged_at: String,
    #[serde(default)]
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_pr_parses_gh_json() {
        let j = r#"[{"number":142,"state":"MERGED","isDraft":false,"baseRefName":"staging","createdAt":"2026-11-17T10:00:00Z","mergedAt":"2026-11-18T09:00:00Z","url":"u"}]"#;
        let v = serde_json::from_str::<Vec<BranchPr>>(j).unwrap();
        assert_eq!((v[0].number, v[0].state.as_str(), v[0].base_ref_name.as_str()), (142, "MERGED", "staging"));
        assert_eq!(v[0].merged_at, "2026-11-18T09:00:00Z");
        // An open PR has no mergedAt (gh sends null or omits it).
        let open = serde_json::from_str::<Vec<BranchPr>>(r#"[{"number":1,"state":"OPEN","baseRefName":"main","mergedAt":null}]"#).unwrap();
        assert!(open[0].merged_at.is_empty());
    }

    #[test]
    fn head_name_strips_one_origin() {
        assert_eq!(head_name("origin/feat/a"), "feat/a");
        assert_eq!(head_name("feat/a"), "feat/a");
        assert_eq!(head_name("origin/origin/x"), "origin/x");
    }

    #[test]
    fn split_list_trims_and_drops_blanks() {
        assert_eq!(split_list(" lead, ,team/x "), vec!["lead", "team/x"]);
    }
}
