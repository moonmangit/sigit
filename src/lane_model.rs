//! What the lane pane shows about a lane's changed code, worked out once when its PR details or
//! module layout arrive (never per frame): the Modules summary or folder groups, one colour per
//! main-bar row, and per-PR / per-author module maps in those colours.

use std::collections::{HashMap, HashSet};

use crate::compare::PrDetail;
use crate::lane::{self, FolderShare, Folders};
use crate::modules::{self, Layout};

/// The colour of segments outside the main bar's rows (draw it neutral, not with `hue`).
pub const OTHER: usize = usize::MAX;
/// The key of the segment for files outside the main bar's rows (no repo path looks like it).
pub const OTHER_KEY: &str = "\u{0}other";

/// One slice of an item's module map.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// Module path, folder path, or `OTHER_KEY`.
    pub key: String,
    pub label: String,
    /// Index into the main bar's colours (`hue(color)`), or `OTHER`.
    pub color: usize,
    /// This slice's lines over the item's lines (0.0 when nothing changed).
    pub share: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct LaneModel {
    /// PR details by number.
    pub details: HashMap<u64, PrDetail>,
    /// The folder diagram (used when the repo has no learned modules).
    pub folders: Folders,
    /// The Modules section, when the repo's layout has modules.
    pub summary: Option<modules::Summary>,
    /// Main-bar row key (module or folder path) → colour index.
    pub colors: HashMap<String, usize>,
    pub per_pr: HashMap<u64, Vec<Segment>>,
    /// By author login: everything their PRs in the lane changed.
    pub per_author: HashMap<String, Vec<Segment>>,
    /// Main-bar row (path, root) → the PRs that changed a file in it (what picking it filters to).
    pub touched: HashMap<(String, bool), HashSet<u64>>,
}

/// Work out everything the pane shows for a lane. `layout` with no modules counts as none.
pub fn build(details: &[PrDetail], layout: Option<&Layout>) -> LaneModel {
    let layout = layout.filter(|l| !l.containers.is_empty());
    let summary = layout.map(|l| modules::summarize(l, details));
    let mut model = LaneModel {
        details: details.iter().map(|d| (d.number, d.clone())).collect(),
        folders: lane::folders(details),
        summary,
        ..LaneModel::default()
    };
    // The main bar's rows in order: modules then shared rows, or the folder rows, as (path, root).
    let rows: Vec<(String, bool)> = match &model.summary {
        Some(sm) => sm.modules.iter().map(|m| (m.path.clone(), false)).chain(shared_rows(sm).iter().map(|f| (f.path.clone(), f.root))).collect(),
        None => model.folders.top.iter().map(|f| (f.path.clone(), f.root)).collect(),
    };
    for (i, row) in rows.into_iter().enumerate() {
        let prs = details.iter().filter(|d| d.files.iter().any(|(p, _, _)| lane::in_folder(&row.0, row.1, p))).map(|d| d.number).collect();
        model.colors.entry(row.0.clone()).or_insert(i);
        model.touched.entry(row).or_insert(prs);
    }
    let mut by_author: HashMap<String, Vec<(String, u64, u64)>> = HashMap::new();
    for d in details {
        by_author.entry(d.author.login.clone()).or_default().extend(d.files.iter().cloned());
    }
    model.per_pr = details.iter().map(|d| (d.number, segments(&model, layout, &d.files))).collect();
    model.per_author = by_author.into_iter().map(|(login, files)| (login, segments(&model, layout, &files))).collect();
    model
}

/// The shared-code rows the Modules bar shows.
fn shared_rows(sm: &modules::Summary) -> &[FolderShare] {
    &sm.shared.top[..sm.shared.top.len().min(modules::MAX_SHARED_ROWS)]
}

/// Split `files` by the main bar's rows: each file goes to its module (with modules), else the
/// first folder row holding it, else "other"; shares by lines changed, in main-bar order.
pub fn segments(model: &LaneModel, layout: Option<&Layout>, files: &[(String, u64, u64)]) -> Vec<Segment> {
    let layout = layout.filter(|l| model.summary.is_some() && !l.containers.is_empty());
    let rows: &[FolderShare] = match &model.summary {
        Some(sm) => shared_rows(sm),
        None => &model.folders.top,
    };
    // (key, label, lines), in first-seen order.
    let mut buckets: Vec<(String, String, u64)> = Vec::new();
    for (path, a, r) in files {
        let (key, label) = if let Some(p) = layout.and_then(|l| modules::place(l, path)) {
            let key = if p.container.is_empty() { p.module.clone() } else { format!("{}/{}", p.container, p.module) };
            (key, p.module)
        } else if let Some(f) = rows.iter().find(|f| lane::holds(f, path)) {
            let label = match (&model.summary, f.root) {
                (Some(_), _) if f.path.is_empty() => crate::i18n::t("(root)"),
                (Some(_), _) => f.path.clone(),
                (None, true) => crate::i18n::t("(root)"),
                (None, false) => f.label.clone(),
            };
            (f.path.clone(), label)
        } else {
            (OTHER_KEY.to_owned(), crate::i18n::t("other"))
        };
        match buckets.iter_mut().find(|b| b.0 == key) {
            Some(b) => b.2 += a + r,
            None => buckets.push((key, label, a + r)),
        }
    }
    let total: u64 = buckets.iter().map(|b| b.2).sum();
    let mut out: Vec<Segment> = buckets
        .into_iter()
        .map(|(key, label, lines)| Segment {
            color: model.colors.get(&key).copied().unwrap_or(OTHER),
            share: if total == 0 { 0.0 } else { lines as f32 / total as f32 },
            key,
            label,
        })
        .collect();
    out.sort_by(|a, b| a.color.cmp(&b.color).then_with(|| a.key.cmp(&b.key)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn layout() -> Layout {
        let mut t = Vec::new();
        for m in ["agent", "report", "history", "auth"] {
            for f in ["view/Main.vue", "service/index.ts", "route.ts"] {
                t.push(format!("src/feature/{m}/{f}"));
            }
        }
        t.push("docs/limits.md".into());
        modules::learn(&t)
    }

    fn pr(number: u64, login: &str, files: &[(&str, u64, u64)]) -> PrDetail {
        let mut d = PrDetail { number, files: files.iter().map(|(p, a, r)| (p.to_string(), *a, *r)).collect(), ..PrDetail::default() };
        d.author.login = login.into();
        d
    }

    fn lane() -> Vec<PrDetail> {
        vec![
            pr(139, "nok", &[("src/feature/history/view/History.vue", 210, 0), ("src/feature/history/route.ts", 12, 0)]),
            pr(141, "ploy", &[("src/feature/agent/service/limit.ts", 120, 14), ("docs/limits.md", 18, 0)]),
            pr(137, "ploy", &[("src/feature/agent/view/Payout.vue", 12, 9)]),
        ]
    }

    #[test]
    fn colours_follow_the_main_bar() {
        let l = layout();
        let m = build(&lane(), Some(&l));
        assert_eq!(m.colors.get("src/feature/history"), Some(&0));
        assert_eq!(m.colors.get("src/feature/agent"), Some(&1));
        assert_eq!(m.colors.get("docs"), Some(&2));
        let colours: Vec<usize> = m.per_pr[&141].iter().map(|s| s.color).collect();
        assert_eq!(colours, [1, 2]);
        assert_eq!(m.per_pr[&141][0].label, "agent");
        assert!(m.summary.is_some());
        assert_eq!(m.details[&139].number, 139);
        // The same colours for a list of files that isn't a PR (a commit), and one "other".
        let files = vec![("src/feature/agent/x.ts".to_string(), 3, 0), ("lib/elsewhere.rs".to_string(), 1, 0)];
        let s = segments(&m, Some(&l), &files);
        assert_eq!(s.iter().map(|x| (x.key.as_str(), x.color)).collect::<Vec<_>>(), [("src/feature/agent", 1), (OTHER_KEY, OTHER)]);
        assert_eq!(s[0].share, 0.75);
    }

    #[test]
    fn per_pr_and_per_author_shares() {
        let l = layout();
        let m = build(&lane(), Some(&l));
        let ploy = &m.per_author["ploy"];
        let sum: f32 = ploy.iter().map(|s| s.share).sum();
        assert!((sum - 1.0).abs() < 1e-5, "{ploy:?}");
        // agent: 134 + 21 lines, docs: 18 → of 173.
        let agent = ploy.iter().find(|s| s.key == "src/feature/agent").unwrap();
        assert!((agent.share - 155.0 / 173.0).abs() < 1e-5);
        let docs = ploy.iter().find(|s| s.key == "docs").unwrap();
        assert!((docs.share - 18.0 / 173.0).abs() < 1e-5);
        assert_eq!(m.per_author["nok"].len(), 1);
        assert_eq!(m.per_pr[&139][0].share, 1.0);
    }

    #[test]
    fn folder_fallback_without_layout() {
        let m = build(&lane(), None);
        assert!(m.summary.is_none());
        // Folder rows in the diagram's order get 0, 1, 2…; the PR maps use their paths.
        for (i, f) in m.folders.top.iter().enumerate() {
            assert_eq!(m.colors.get(&f.path), Some(&i));
        }
        let keys: Vec<&str> = m.per_pr[&141].iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().all(|k| m.folders.top.iter().any(|f| f.path == *k)), "{keys:?} vs {:?}", m.folders.top);
        assert!(m.per_pr[&141].iter().all(|s| m.colors.get(&s.key) == Some(&s.color)));
    }

    #[test]
    fn zero_line_files_give_no_nan() {
        let details = vec![pr(1, "a", &[("src/feature/agent/x.ts", 0, 0), ("docs/y.md", 0, 0)])];
        let m = build(&details, Some(&layout()));
        for s in m.per_pr[&1].iter().chain(&m.per_author["a"]) {
            assert_eq!(s.share, 0.0);
        }
        assert!(segments(&m, None, &[]).is_empty());
    }

    /// Every main-bar row's PR set, against `lane::touches` on the same details.
    fn assert_touched_matches(m: &LaneModel, details: &[PrDetail], rows: &[FolderShare]) {
        assert!(!rows.is_empty());
        assert_eq!(m.touched.len(), rows.len(), "{:?}", m.touched.keys().collect::<Vec<_>>());
        for f in rows {
            let want: HashSet<u64> = details.iter().filter(|d| lane::touches(d, f)).map(|d| d.number).collect();
            assert_eq!(m.touched.get(&(f.path.clone(), f.root)), Some(&want), "{}/{}", f.path, f.root);
        }
    }

    #[test]
    fn touched_sets_match_touches() {
        // Modules, a shared folder (docs), a shared folder's own files (src/) and the repo root.
        let mut details = lane();
        details.push(pr(150, "nok", &[("README.md", 3, 0), ("src/main.ts", 4, 1), ("docs/more.md", 1, 0)]));
        let m = build(&details, Some(&layout()));
        let sm = m.summary.clone().unwrap();
        let rows: Vec<FolderShare> = sm
            .modules
            .iter()
            .map(|x| FolderShare { label: x.name.clone(), path: x.path.clone(), root: false, added: 0, removed: 0, share: 0.0 })
            .chain(shared_rows(&sm).iter().cloned())
            .collect();
        assert!(rows.iter().any(|f| f.root && f.path.is_empty()), "{rows:?}");
        assert_touched_matches(&m, &details, &rows);
        // Without modules: the folder rows.
        let m = build(&details, None);
        assert_touched_matches(&m, &details, &m.folders.top.clone());
    }

    #[test]
    fn empty_lane_builds_an_empty_model() {
        let m = build(&[], Some(&layout()));
        assert!(m.per_pr.is_empty() && m.per_author.is_empty() && m.colors.is_empty());
    }
}
