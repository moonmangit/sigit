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

/// What a repo's comparison dialog saves: an override for each (from, to, on) line that differs
/// from its shared comparison. A blank field means the shared branch.
pub fn edit_overrides(shared: &[Comparison], slug: &str, rows: &[(String, String, bool)]) -> Vec<Override> {
    let changed = |v: &str, base: &str| {
        let v = v.trim();
        (!v.is_empty() && v != base).then(|| v.to_owned())
    };
    shared
        .iter()
        .zip(rows)
        .filter_map(|(c, (from, to, on))| {
            let (from, to) = (changed(from, &c.from), changed(to, &c.to));
            (from.is_some() || to.is_some() || !on).then(|| Override { slug: slug.into(), name: c.name.clone(), from, to, off: !on })
        })
        .collect()
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

// ---- Fetching ------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaitingPr {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub merged_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LooseCommit {
    pub sha: String,
    pub subject: String,
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Cell {
    Off,
    /// The branch names that don't exist in the repo.
    Missing(Vec<String>),
    /// `commits` is exact; `checked` is how many of them were read to find their PRs (at most 500).
    Waiting { commits: u32, prs: Vec<WaitingPr>, loose: Vec<LooseCommit>, checked: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepoCompare {
    pub slug: String,
    pub cells: Vec<Cell>,
    pub error: Option<String>,
}

type Rows = [(String, Vec<Option<Comparison>>)];

const MAX_CHECKED: u32 = 500;
const CHUNK: usize = 10;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn head(slug: &str) -> String {
    let (owner, name) = slug.split_once('/').unwrap_or((slug, ""));
    format!("repository(owner: \"{}\", name: \"{}\")", esc(owner), esc(name))
}

fn commits_sel(after: Option<&str>) -> String {
    let after = after.map(|a| format!(", after: \"{}\"", esc(a))).unwrap_or_default();
    format!(
        "commits(first: 100{after}) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ oid messageHeadline \
         author {{ name user {{ login }} }} associatedPullRequests(first: 5) {{ nodes {{ number title \
         author {{ login }} mergedAt baseRefName state }} }} }} }}"
    )
}

/// False when every comparison in `rows` is off: there is nothing to ask GitHub.
fn needs_query(rows: &Rows) -> bool {
    rows.iter().any(|(_, cmps)| cmps.iter().any(Option::is_some))
}

pub fn query(rows: &Rows) -> String {
    let mut q = String::from("query {");
    for (i, (slug, cmps)) in rows.iter().enumerate() {
        if cmps.iter().all(Option::is_none) {
            continue;
        }
        q += &format!(" r{i}: {} {{", head(slug));
        for (j, c) in cmps.iter().enumerate() {
            let Some(c) = c else { continue };
            q += &format!(
                " c{j}: ref(qualifiedName: \"refs/heads/{}\") {{ compare(headRef: \"refs/heads/{}\") {{ aheadBy {} }} }} \
                 f{j}: ref(qualifiedName: \"refs/heads/{}\") {{ name }}",
                esc(&c.to),
                esc(&c.from),
                commits_sel(None),
                esc(&c.from)
            );
        }
        q += " }";
    }
    q + " }"
}

pub fn page_query(slug: &str, c: &Comparison, after: &str) -> String {
    format!(
        "query {{ {} {{ ref(qualifiedName: \"refs/heads/{}\") {{ compare(headRef: \"refs/heads/{}\") {{ {} }} }} }} }}",
        head(slug),
        esc(&c.to),
        esc(&c.from),
        commits_sel(Some(after))
    )
}

fn s(v: &serde_json::Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// Folds one page of commit nodes into `prs` / `loose`.
fn add_nodes(prs: &mut Vec<WaitingPr>, loose: &mut Vec<LooseCommit>, c: &Comparison, nodes: &[serde_json::Value]) {
    for n in nodes {
        let merged = n["associatedPullRequests"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["state"] == "MERGED")
            .collect::<Vec<_>>();
        let pick = merged
            .iter()
            .find(|p| p["baseRefName"] == c.from.as_str())
            .or_else(|| merged.iter().min_by_key(|p| s(&p["mergedAt"])));
        match pick {
            Some(p) => {
                let number = p["number"].as_u64().unwrap_or(0);
                if !prs.iter().any(|x| x.number == number) {
                    prs.push(WaitingPr {
                        number,
                        title: s(&p["title"]),
                        author: s(&p["author"]["login"]),
                        merged_at: s(&p["mergedAt"]),
                    });
                }
            }
            None => {
                let a = &n["author"];
                let author = a["user"]["login"].as_str().or(a["name"].as_str()).unwrap_or_default().to_owned();
                loose.push(LooseCommit { sha: s(&n["oid"]), subject: s(&n["messageHeadline"]), author });
            }
        }
    }
}

/// `Some(cursor)` when more commits should be read.
fn next_cursor(commits: &serde_json::Value, checked: u32) -> Option<String> {
    let more = commits["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
    let cursor = commits["pageInfo"]["endCursor"].as_str()?;
    (more && checked < MAX_CHECKED).then(|| cursor.to_owned())
}

fn nodes(commits: &serde_json::Value) -> &[serde_json::Value] {
    commits["nodes"].as_array().map(Vec::as_slice).unwrap_or_default()
}

pub fn parse(rows: &Rows, json: &serde_json::Value) -> (Vec<RepoCompare>, Vec<(usize, usize, String)>) {
    let mut more = Vec::new();
    let out = rows
        .iter()
        .enumerate()
        .map(|(i, (slug, cmps))| {
            let mut rc = RepoCompare { slug: slug.clone(), cells: vec![Cell::Off; cmps.len()], error: None };
            if cmps.iter().all(Option::is_none) {
                return rc;
            }
            let repo = &json["data"][format!("r{i}")];
            if repo.is_null() {
                let alias = format!("r{i}");
                let msg = json["errors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|e| e["path"][0] == alias.as_str())
                    .and_then(|e| e["message"].as_str());
                rc.error = Some(msg.map_or_else(|| crate::i18n::t("GitHub returned nothing for this repo"), str::to_owned));
                return rc;
            }
            for (j, c) in cmps.iter().enumerate() {
                let Some(c) = c else { continue };
                let commits = &repo[format!("c{j}")]["compare"]["commits"];
                rc.cells[j] = if commits.is_null() {
                    // `c` is the `to` ref, `f` the `from` ref; each is null when its branch is missing.
                    let mut gone = Vec::new();
                    if repo[format!("f{j}")].is_null() {
                        gone.push(c.from.clone());
                    }
                    if repo[format!("c{j}")].is_null() {
                        gone.push(c.to.clone());
                    }
                    Cell::Missing(gone)
                } else {
                    let (mut prs, mut loose) = (Vec::new(), Vec::new());
                    let nodes = nodes(commits);
                    add_nodes(&mut prs, &mut loose, c, nodes);
                    let checked = nodes.len() as u32;
                    if let Some(cur) = next_cursor(commits, checked) {
                        more.push((i, j, cur));
                    }
                    let ahead = repo[format!("c{j}")]["compare"]["aheadBy"].as_u64().unwrap_or(checked as u64) as u32;
                    Cell::Waiting { commits: ahead, prs, loose, checked }
                };
            }
            rc
        })
        .collect();
    (out, more)
}

/// Adds one follow-up page to `cell`; returns the next cursor, or `None` when done.
pub fn apply_page(cell: &mut Cell, c: &Comparison, json: &serde_json::Value) -> Option<String> {
    let Cell::Waiting { prs, loose, checked, .. } = cell else { return None };
    let commits = &json["data"]["repository"]["ref"]["compare"]["commits"];
    let nodes = nodes(commits);
    add_nodes(prs, loose, c, nodes);
    *checked = (*checked + nodes.len() as u32).min(MAX_CHECKED);
    next_cursor(commits, *checked)
}

/// Runs `gh api graphql`. Unlike `gh::run`, keeps stdout when gh exits non-zero: GitHub answers
/// with `errors` next to a partly filled `data`, and the other repos' rows are still good.
fn graphql(q: &str) -> Result<serde_json::Value, String> {
    let mut cmd = std::process::Command::new("gh");
    cmd.args(["api", "graphql", "-f", &format!("query={q}")]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| crate::tf!("couldn't start `gh`: {e}"))?;
    match serde_json::from_slice::<serde_json::Value>(&out.stdout) {
        Ok(v) if v["data"].is_object() => Ok(v),
        _ => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            Err(if err.is_empty() { crate::i18n::t("`gh api graphql` failed").to_owned() } else { err })
        }
    }
}

pub fn fetch(rows: Vec<(String, Vec<Option<Comparison>>)>) -> Vec<RepoCompare> {
    let mut all = Vec::with_capacity(rows.len());
    for chunk in rows.chunks(CHUNK) {
        if !needs_query(chunk) {
            all.extend(parse(chunk, &serde_json::Value::Null).0);
            continue;
        }
        let json = match graphql(&query(chunk)) {
            Ok(j) => j,
            Err(e) => {
                all.extend(chunk.iter().map(|(slug, cmps)| RepoCompare {
                    slug: slug.clone(),
                    cells: vec![Cell::Off; cmps.len()],
                    error: Some(e.clone()),
                }));
                continue;
            }
        };
        let (mut out, mut pending) = parse(chunk, &json);
        while let Some((i, j, mut cur)) = pending.pop() {
            let Some(c) = chunk[i].1[j].clone() else { continue };
            loop {
                match graphql(&page_query(&chunk[i].0, &c, &cur)) {
                    Ok(page) => match apply_page(&mut out[i].cells[j], &c, &page) {
                        Some(next) => cur = next,
                        None => break,
                    },
                    Err(e) => {
                        out[i].error = Some(e);
                        break;
                    }
                }
            }
        }
        all.extend(out);
    }
    all
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
    fn edit_overrides_keeps_only_changes() {
        let shared = defaults();
        let rows = vec![
            ("dev".to_string(), " staging ".to_string(), true), // same as shared (spaces trimmed)
            ("staging".to_string(), "dev".to_string(), false),  // switched off
            ("".to_string(), "master".to_string(), true),       // blank From = shared
        ];
        let out = edit_overrides(&shared, "a/b", &rows);
        assert_eq!(out, vec![
            Override { slug: "a/b".into(), name: "Back-merge".into(), from: None, to: None, off: true },
            Override { slug: "a/b".into(), name: "To production".into(), from: None, to: Some("master".into()), off: false },
        ]);
    }

    #[test]
    fn old_prefs_get_default_comparisons() {
        let p: Prefs = serde_json::from_str(r#"{"pinned":["a/b"]}"#).unwrap();
        assert_eq!(p.comparisons, defaults());
        assert!(p.comparison_overrides.is_empty() && p.clone_folders.is_empty());
    }

    use serde_json::json;

    fn cmp(from: &str, to: &str) -> Comparison {
        Comparison { name: "x".into(), from: from.into(), to: to.into() }
    }

    fn one(c: Comparison) -> Vec<(String, Vec<Option<Comparison>>)> {
        vec![("o/n".to_string(), vec![Some(c)])]
    }

    fn pr(n: u64, base: &str, state: &str, merged: &str) -> serde_json::Value {
        json!({"number": n, "title": format!("PR {n}"), "author": {"login": "ann"}, "mergedAt": merged, "baseRefName": base, "state": state})
    }

    fn node(oid: &str, prs: Vec<serde_json::Value>) -> serde_json::Value {
        json!({"oid": oid, "messageHeadline": format!("commit {oid}"), "author": {"name": "Ann", "user": {"login": "ann"}},
               "associatedPullRequests": {"nodes": prs}})
    }

    fn data(ahead: u64, nodes: Vec<serde_json::Value>, more: bool, cursor: &str) -> serde_json::Value {
        json!({"data": {"r0": {
            "c0": {"compare": {"aheadBy": ahead, "commits": {"pageInfo": {"hasNextPage": more, "endCursor": cursor}, "nodes": nodes}}},
            "f0": {"name": "x"}
        }}})
    }

    #[test]
    fn query_escapes_branch_names() {
        let rows = one(cmp("feature/it's \"q\"", "main"));
        let q = query(&rows);
        assert!(q.contains(r#"refs/heads/feature/it's \"q\""#));
        assert!(q.contains(r#"repository(owner: "o", name: "n")"#));
    }

    #[test]
    fn parse_counts_commits_and_squash_prs() {
        let nodes = (0..7).map(|k| node(&format!("s{k}"), vec![pr(116 + k, "staging", "MERGED", "2026-01-01T00:00:00Z")])).collect();
        let rows = one(cmp("staging", "main"));
        let (out, more) = parse(&rows, &data(7, nodes, false, "z"));
        assert!(more.is_empty() && out[0].error.is_none());
        let Cell::Waiting { commits, prs, loose, checked } = &out[0].cells[0] else { panic!("{:?}", out[0].cells) };
        assert_eq!((*commits, prs.len(), loose.len(), *checked), (7, 7, 0, 7));
        assert_eq!(prs[0].number, 116);
    }

    #[test]
    fn parse_prefers_pr_into_from_branch() {
        let n = node("a", vec![pr(5, "dev", "MERGED", "2026-02-01T00:00:00Z"), pr(9, "staging", "MERGED", "2026-01-01T00:00:00Z")]);
        let rows = one(cmp("dev", "staging"));
        let (out, _) = parse(&rows, &data(1, vec![n], false, "z"));
        let Cell::Waiting { prs, .. } = &out[0].cells[0] else { panic!() };
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![5]);
    }

    #[test]
    fn parse_falls_back_to_earliest_merged_pr() {
        let n = node("a", vec![pr(5, "x", "MERGED", "2026-02-01T00:00:00Z"), pr(9, "y", "MERGED", "2026-01-01T00:00:00Z")]);
        let rows = one(cmp("dev", "staging"));
        let (out, _) = parse(&rows, &data(1, vec![n], false, "z"));
        let Cell::Waiting { prs, .. } = &out[0].cells[0] else { panic!() };
        assert_eq!(prs.iter().map(|p| p.number).collect::<Vec<_>>(), vec![9]);
    }

    #[test]
    fn parse_ignores_open_prs_and_keeps_loose_commits() {
        let n = node("abc", vec![pr(5, "dev", "OPEN", "")]);
        let rows = one(cmp("dev", "staging"));
        let (out, _) = parse(&rows, &data(1, vec![n], false, "z"));
        let Cell::Waiting { prs, loose, .. } = &out[0].cells[0] else { panic!() };
        assert!(prs.is_empty());
        assert_eq!(loose, &vec![LooseCommit { sha: "abc".into(), subject: "commit abc".into(), author: "ann".into() }]);
    }

    #[test]
    fn parse_reports_missing_branches() {
        let rows = one(cmp("dev", "staging"));
        let both = json!({"data": {"r0": {"c0": null, "f0": null}}});
        assert_eq!(parse(&rows, &both).0[0].cells[0], Cell::Missing(vec!["dev".into(), "staging".into()]));
        let only_to = json!({"data": {"r0": {"c0": null, "f0": {"name": "dev"}}}});
        assert_eq!(parse(&rows, &only_to).0[0].cells[0], Cell::Missing(vec!["staging".into()]));
        let only_from = json!({"data": {"r0": {"c0": {"compare": null}, "f0": null}}});
        assert_eq!(parse(&rows, &only_from).0[0].cells[0], Cell::Missing(vec!["dev".into()]));
    }

    #[test]
    fn all_off_chunks_need_no_query() {
        let off = vec![("o/n".to_string(), vec![None, None])];
        assert!(!needs_query(&off));
        let (out, more) = parse(&off, &serde_json::Value::Null);
        assert!(out[0].error.is_none() && more.is_empty());
        assert_eq!(out[0].cells, vec![Cell::Off, Cell::Off]);
        assert!(needs_query(&one(cmp("a", "b"))));
    }

    #[test]
    fn parse_keeps_other_repos_when_one_fails() {
        let rows = vec![
            ("o/gone".to_string(), vec![Some(cmp("dev", "staging"))]),
            ("o/ok".to_string(), vec![Some(cmp("dev", "staging")), None]),
        ];
        let j = json!({
            "data": {"r0": null, "r1": {"c0": {"compare": {"aheadBy": 0, "commits": {"pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": []}}}, "f0": {"name": "dev"}}},
            "errors": [{"path": ["r0"], "message": "Could not resolve to a Repository"}]
        });
        let (out, _) = parse(&rows, &j);
        assert_eq!(out[0].error.as_deref(), Some("Could not resolve to a Repository"));
        assert!(out[1].error.is_none());
        assert!(matches!(out[1].cells[0], Cell::Waiting { commits: 0, .. }));
        assert_eq!(out[1].cells[1], Cell::Off);
    }

    #[test]
    fn parse_asks_for_more_pages() {
        let rows = one(cmp("dev", "staging"));
        let (_, more) = parse(&rows, &data(250, vec![node("a", vec![])], true, "abc"));
        assert_eq!(more, vec![(0, 0, "abc".to_string())]);
    }

    #[test]
    fn apply_page_stops_at_500() {
        let c = cmp("dev", "staging");
        let mut cell = Cell::Waiting { commits: 900, prs: vec![], loose: vec![], checked: 450 };
        let nodes: Vec<_> = (0..100).map(|k| node(&format!("p{k}"), vec![pr(k, "dev", "MERGED", "2026-01-01T00:00:00Z")])).collect();
        let page = json!({"data": {"repository": {"ref": {"compare": {"commits": {"pageInfo": {"hasNextPage": true, "endCursor": "n"}, "nodes": nodes}}}}}});
        assert_eq!(apply_page(&mut cell, &c, &page), None);
        let Cell::Waiting { checked, prs, .. } = &cell else { panic!() };
        assert_eq!((*checked, prs.len()), (500, 100));
        let mut early = Cell::Waiting { commits: 900, prs: vec![], loose: vec![], checked: 100 };
        assert_eq!(apply_page(&mut early, &c, &page), Some("n".to_string()));
    }

    #[test]
    #[ignore]
    fn live_fetch() {
        let rows = vec![("threadsoftwareth/aom-bet-lottery-admin-fe".to_string(), defaults().into_iter().map(Some).collect())];
        for r in fetch(rows) {
            println!("{} error={:?}", r.slug, r.error);
            for c in &r.cells {
                match c {
                    Cell::Waiting { commits, prs, loose, checked } => {
                        println!("  commits={commits} prs={} loose={} checked={checked}", prs.len(), loose.len())
                    }
                    other => println!("  {other:?}"),
                }
            }
        }
    }
}
