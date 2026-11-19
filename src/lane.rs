//! Grouping a comparison lane's PRs: by folder (the "Changed code" diagram) and by author.

use crate::compare::{PrDetail, Person, WaitingPr};
use std::collections::HashMap;

/// Folder groups shown in the diagram; the rest collapse into "other N folders".
const TOP_FOLDERS: usize = 8;
/// A folder with at least this share of the lines is split into its subfolders.
const DOMINANT: f32 = 0.75;

/// One folder's slice of the lane's changed lines.
#[derive(Clone, Debug, PartialEq)]
pub struct FolderShare {
    /// What the legend shows: the path without the shared prefix, or "(root)".
    pub label: String,
    /// Full repo path of the folder (for `root`, the shared-prefix folder; "" at the repo root).
    pub path: String,
    /// Files directly in `path`, not in a subfolder.
    pub root: bool,
    pub added: u64,
    pub removed: u64,
    /// This folder's lines over all lines (0.0 when nothing changed).
    pub share: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Folders {
    pub top: Vec<FolderShare>,
    pub others: usize,
    pub others_lines: u64,
}

fn segments(s: &str) -> Vec<&str> {
    s.split('/').filter(|p| !p.is_empty()).collect()
}

/// Parent-folder segments of a file path.
fn parent_segments(path: &str) -> Vec<&str> {
    let mut s = segments(path);
    s.pop();
    s
}

/// Files under `under` (relative paths), grouped by their folder `depth` segments deep; files
/// directly in `under` are the group `(under, true)`.
fn group_at<'a>(rels: &[(&'a [&'a str], u64, u64)], under: &[&'a str], depth: usize) -> HashMap<(Vec<&'a str>, bool), (u64, u64)> {
    let mut groups: HashMap<(Vec<&'a str>, bool), (u64, u64)> = HashMap::new();
    for (dirs, a, r) in rels {
        let key = if dirs.len() >= depth { (dirs[..depth].to_vec(), false) } else { (under.to_vec(), true) };
        let e = groups.entry(key).or_default();
        e.0 += a;
        e.1 += r;
    }
    groups
}

/// Group every changed file by folder, dropping the prefix all files share.
pub fn folders(details: &[PrDetail]) -> Folders {
    let files: Vec<(Vec<&str>, u64, u64)> = details
        .iter()
        .flat_map(|d| d.files.iter())
        .map(|(p, a, r)| (parent_segments(p), *a, *r))
        .collect();
    let Some(first) = files.first() else {
        return Folders::default();
    };
    let mut prefix = first.0.len();
    for (dirs, _, _) in &files {
        prefix = prefix.min(dirs.len());
        prefix = (0..prefix).take_while(|&i| dirs[i] == first.0[i]).count();
    }
    let prefix_path = first.0[..prefix].join("/");

    // Group by the first folder below the shared prefix. Files directly in a group's folder are
    // their own group, keyed `(folder, true)`.
    let rels: Vec<(&[&str], u64, u64)> = files.iter().map(|(dirs, a, r)| (&dirs[prefix..], *a, *r)).collect();
    let mut groups = group_at(&rels, &[], 1);
    // A folder holding most of the lines says little ("src 96%"): replace it with its
    // subfolders, again and again, until none dominates or the biggest can't split further.
    loop {
        let total: u64 = groups.values().map(|(a, r)| a + r).sum();
        let Some(big) = groups
            .iter()
            .filter(|((_, root), _)| !root)
            .find(|(_, (a, r))| total > 0 && (a + r) as f32 / total as f32 >= DOMINANT)
            .map(|(k, _)| k.0.clone())
        else {
            break;
        };
        let inside: Vec<(&[&str], u64, u64)> = rels.iter().filter(|(d, _, _)| d.starts_with(&big)).copied().collect();
        let split = group_at(&inside, &big, big.len() + 1);
        // One subfolder only (src → feature): the loop takes it, still dominant, next round. Only
        // files sitting directly in it (a single root group) leave nothing deeper to show.
        if split.len() == 1 && split.keys().all(|(_, root)| *root) {
            break;
        }
        groups.remove(&(big, false));
        groups.extend(split);
    }

    let total: u64 = groups.values().map(|(a, r)| a + r).sum();
    let mut all: Vec<FolderShare> = groups
        .into_iter()
        .map(|((key, root), (added, removed))| {
            let rel = key.join("/");
            let path = match (prefix_path.is_empty(), rel.is_empty()) {
                (_, true) => prefix_path.clone(),
                (true, false) => rel.clone(),
                (false, false) => format!("{prefix_path}/{rel}"),
            };
            let lines = added + removed;
            FolderShare {
                // Files directly in a split folder read "src/"; at the shared prefix, "(root)".
                label: match (root, rel.is_empty()) {
                    (true, true) => "(root)".to_string(),
                    (true, false) => format!("{rel}/"),
                    (false, _) => rel,
                },
                path,
                root,
                added,
                removed,
                share: if total == 0 { 0.0 } else { lines as f32 / total as f32 },
            }
        })
        .collect();
    all.sort_by(|a, b| {
        (b.added + b.removed)
            .cmp(&(a.added + a.removed))
            .then_with(|| a.path.cmp(&b.path))
    });
    let rest = all.split_off(all.len().min(TOP_FOLDERS));
    Folders {
        others: rest.len(),
        others_lines: rest.iter().map(|f| f.added + f.removed).sum(),
        top: all,
    }
}

/// Whether the PR changed a file in this folder.
pub fn touches(d: &PrDetail, f: &FolderShare) -> bool {
    d.files.iter().any(|(p, _, _)| {
        if f.root {
            parent_segments(p).join("/") == f.path
        } else {
            p.starts_with(&format!("{}/", f.path))
        }
    })
}

/// One author's PRs in a lane.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorGroup {
    pub person: Person,
    /// Newest merge first.
    pub prs: Vec<u64>,
    pub added: u64,
    pub removed: u64,
}

/// Group a lane's PRs by author: most PRs first, then most lines, then login.
pub fn by_author(prs: &[WaitingPr], details: &[PrDetail]) -> Vec<AuthorGroup> {
    let mut sorted: Vec<&WaitingPr> = prs.iter().collect();
    sorted.sort_by(|a, b| b.merged_at.cmp(&a.merged_at));
    let mut groups: Vec<AuthorGroup> = Vec::new();
    for pr in sorted {
        let detail = details.iter().find(|d| d.number == pr.number);
        let person = match detail {
            Some(d) => d.author.clone(),
            None => Person { login: pr.author.clone(), ..Person::default() },
        };
        let (added, removed) = detail.map_or((0, 0), |d| (d.additions, d.deletions));
        match groups.iter_mut().find(|g| g.person.login == person.login) {
            Some(g) => {
                if g.person.name.is_empty() && g.person.company.is_empty() {
                    g.person = person;
                }
                g.prs.push(pr.number);
                g.added += added;
                g.removed += removed;
            }
            None => groups.push(AuthorGroup { person, prs: vec![pr.number], added, removed }),
        }
    }
    groups.sort_by(|a, b| {
        b.prs
            .len()
            .cmp(&a.prs.len())
            .then((b.added + b.removed).cmp(&(a.added + a.removed)))
            .then_with(|| a.person.login.cmp(&b.person.login))
    });
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(number: u64, files: &[(&str, u64, u64)]) -> PrDetail {
        PrDetail {
            number,
            files: files.iter().map(|(p, a, r)| (p.to_string(), *a, *r)).collect(),
            ..PrDetail::default()
        }
    }
    fn labels(f: &Folders) -> Vec<&str> {
        f.top.iter().map(|s| s.label.as_str()).collect()
    }
    #[test]
    fn folders_split_a_dominant_folder() {
        // src holds 94% → split; src/feature then holds 85% → split again; docs stays.
        let d = detail(1, &[("src/feature/agent/a.ts", 50, 0), ("src/feature/report/b.ts", 30, 0), ("src/shared/c.ts", 10, 0), ("docs/x.md", 4, 0)]);
        let f = folders(&[d]);
        let mut got = labels(&f);
        got.sort();
        assert_eq!(got, vec!["docs", "src/feature/agent", "src/feature/report", "src/shared"]);
        let agent = f.top.iter().find(|s| s.label == "src/feature/agent").unwrap();
        assert_eq!((agent.path.as_str(), agent.root), ("src/feature/agent", false));
    }

    #[test]
    fn folders_split_through_a_single_subfolder() {
        // src → only feature → agent / report: walk through "feature" instead of stopping at src.
        let d = detail(1, &[("src/feature/agent/a.ts", 300, 0), ("src/feature/report/b.ts", 140, 0), ("docs/x.md", 18, 0)]);
        let f = folders(&[d]);
        let mut got = labels(&f);
        got.sort();
        assert_eq!(got, vec!["docs", "src/feature/agent", "src/feature/report"]);
    }

    #[test]
    fn folders_split_keeps_files_directly_in_the_folder() {
        let d = detail(1, &[("src/main.ts", 5, 0), ("src/a/x.ts", 90, 0), ("docs/y.md", 5, 0)]);
        let f = folders(&[d.clone()]);
        let mut got = labels(&f);
        got.sort();
        assert_eq!(got, vec!["docs", "src/", "src/a"]);
        let here = f.top.iter().find(|s| s.label == "src/").unwrap();
        assert_eq!((here.path.as_str(), here.root), ("src", true));
        assert!(touches(&detail(2, &[("src/main.ts", 1, 0)]), here));
        assert!(!touches(&detail(3, &[("src/a/x.ts", 1, 0)]), here));
    }

    #[test]
    fn folders_no_split_below_threshold() {
        let d = detail(1, &[("src/a/x.ts", 70, 0), ("docs/y.md", 30, 0)]);
        let f = folders(&[d]);
        let mut got = labels(&f);
        got.sort();
        assert_eq!(got, vec!["docs", "src"]);
    }

    fn pr(number: u64, author: &str, merged_at: &str) -> WaitingPr {
        WaitingPr {
            number,
            title: String::new(),
            author: author.into(),
            merged_at: merged_at.into(),
        }
    }
    fn authored(number: u64, login: &str, add: u64, del: u64) -> PrDetail {
        PrDetail {
            number,
            additions: add,
            deletions: del,
            author: Person { login: login.into(), name: format!("{login} name"), company: String::new(), ..Default::default() },
            ..PrDetail::default()
        }
    }

    #[test]
    fn folders_drop_shared_prefix() {
        let f = folders(&[detail(1, &[("src/feature/agent/a.ts", 10, 0), ("src/feature/report/b.ts", 30, 0)])]);
        assert_eq!(labels(&f), ["report", "agent"]);
        assert_eq!(f.top[0].path, "src/feature/report");
        assert_eq!(f.top[1].path, "src/feature/agent");
        assert_eq!(f.top[0].share, 0.75);
        assert_eq!(f.top[1].share, 0.25);
    }

    #[test]
    fn folders_prefix_is_segment_wise() {
        let f = folders(&[detail(1, &[("src/feat/a.ts", 1, 0), ("src/feature/b.ts", 2, 0)])]);
        assert_eq!(labels(&f), ["feature", "feat"]);
        assert_eq!(f.top[0].path, "src/feature");
    }

    #[test]
    fn folders_descend_when_single_group() {
        let f = folders(&[detail(1, &[("src/feature/report/locale/en.ts", 5, 0), ("src/feature/report/view/x.vue", 7, 0)])]);
        let mut l = labels(&f);
        l.sort();
        assert_eq!(l, ["locale", "view"]);
        // A prefix that is already shared never leaves a single group to descend from.
        let f = folders(&[detail(1, &[("a/x/p/1.ts", 5, 0), ("a/x/q/2.ts", 7, 0), ("a/x/q/3.ts", 1, 0)])]);
        assert_eq!(labels(&f), ["q", "p"]);
        assert_eq!(f.top[0].path, "a/x/q");
    }

    #[test]
    fn folders_root_files() {
        let f = folders(&[detail(1, &[("README.md", 5, 0), ("src/a.ts", 5, 0)])]);
        let root = f.top.iter().find(|s| s.label == "(root)").unwrap();
        assert!(root.root);
        assert_eq!(root.path, "");
        let src = f.top.iter().find(|s| s.label == "src").unwrap();
        assert!(!src.root);
        assert_eq!(src.path, "src");
    }

    #[test]
    fn folders_top_eight_then_others() {
        let files: Vec<(String, u64, u64)> = (1..=10).map(|i| (format!("d{i:02}/f.ts"), i * 10, 0)).collect();
        let refs: Vec<(&str, u64, u64)> = files.iter().map(|(p, a, r)| (p.as_str(), *a, *r)).collect();
        let f = folders(&[detail(1, &refs)]);
        assert_eq!(f.top.len(), 8);
        assert_eq!(f.others, 2);
        assert_eq!(f.others_lines, 10 + 20);
    }

    #[test]
    fn folders_sum_same_path_and_handle_empty() {
        let f = folders(&[detail(1, &[("a/x.ts", 1, 1), ("b/y.ts", 1, 0)]), detail(2, &[("a/x.ts", 2, 0)])]);
        assert_eq!((f.top[0].label.as_str(), f.top[0].added, f.top[0].removed), ("a", 3, 1));
        assert_eq!(folders(&[]), Folders::default());
        let z = folders(&[detail(1, &[("a/x.ts", 0, 0), ("b/y.ts", 0, 0)])]);
        assert_eq!(z.top[0].share, 0.0);
    }

    #[test]
    fn touches_matches_folder() {
        let d = detail(1, &[("src/feature/report/x.ts", 1, 0)]);
        let share = |path: &str, root: bool| FolderShare {
            label: String::new(),
            path: path.into(),
            root,
            added: 0,
            removed: 0,
            share: 0.0,
        };
        assert!(touches(&d, &share("src/feature/report", false)));
        assert!(!touches(&d, &share("src/feature/agent", false)));
        assert!(!touches(&d, &share("src/feature/rep", false)));
        let r = share("", true);
        assert!(!touches(&d, &r));
        assert!(touches(&detail(2, &[("README.md", 1, 0)]), &r));
    }

    #[test]
    fn by_author_groups_and_orders() {
        let prs = [pr(1, "a", "2026-01-01"), pr(2, "b", "2026-01-02"), pr(3, "a", "2026-01-03")];
        let details = [authored(1, "a", 10, 1), authored(2, "b", 100, 100), authored(3, "a", 5, 2)];
        let g = by_author(&prs, &details);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].person.login, "a");
        assert_eq!(g[0].prs, [3, 1]);
        assert_eq!((g[0].added, g[0].removed), (15, 3));
        assert_eq!(g[1].person.login, "b");
        assert_eq!(g[1].prs, [2]);
    }

    #[test]
    fn by_author_without_details_uses_pr_author() {
        let g = by_author(&[pr(1, "zed", "2026-01-01"), pr(2, "amy", "2026-01-02")], &[]);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].person, Person { login: "amy".into(), ..Person::default() });
        assert_eq!((g[0].added, g[0].removed), (0, 0));
        assert_eq!(g[1].person.login, "zed");
    }
}
