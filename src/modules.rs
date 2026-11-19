//! Learned modules: a repo's module layout, worked out from its own file tree (no folder names
//! in the app), and a lane's changes summarized by module.
//!
//! A folder is a *module container* when its subfolders look alike: at least `MIN_MODULES`
//! subfolders; names (subfolders, and files without extension) present in at least half of them
//! are its *parts*; at least `MIN_PARTS` parts, one of them a subfolder; and at least
//! `MODULE_SHARE` of the subfolders hold `MIN_PARTS` or more parts. Each subfolder of a container
//! is a module. Everything outside a module is shared code.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::compare::PrDetail;
use crate::lane::{FolderShare, Folders};

const MIN_MODULES: usize = 3;
const MIN_PARTS: usize = 2;
const MODULE_SHARE: f32 = 0.6;
/// Rows shown before the rest collapse into "N more".
pub const MAX_MODULE_ROWS: usize = 8;
pub const MAX_SHARED_ROWS: usize = 4;

/// A folder whose subfolders are modules, and the parts they share (in learned order: most
/// common first, then A–Z).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Container {
    pub path: String,
    pub parts: Vec<String>,
}

/// What was learned about one repo (at one branch).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Layout {
    pub containers: Vec<Container>,
}

/// Where a file sits: its module (container path + module name) and the part it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub container: String,
    pub module: String,
    /// The learned part (first segment inside the module, without extension), or None ("other").
    pub part: Option<String>,
}

fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// Learn the containers from every file path in the tree.
pub fn learn(files: &[String]) -> Layout {
    // Direct entries of every folder: name (files without extension) → is it a folder?
    let mut entries: HashMap<String, HashMap<String, bool>> = HashMap::new();
    let mut subdirs: HashMap<String, BTreeSet<String>> = HashMap::new();
    for f in files {
        let segs: Vec<&str> = f.split('/').filter(|s| !s.is_empty()).collect();
        for i in 0..segs.len() {
            let dir = segs[..i].join("/");
            let is_dir = i + 1 < segs.len();
            let name = if is_dir { segs[i].to_owned() } else { stem(segs[i]).to_owned() };
            let e = entries.entry(dir.clone()).or_default().entry(name).or_insert(false);
            *e |= is_dir;
            if is_dir {
                subdirs.entry(dir).or_default().insert(segs[..=i].join("/"));
            }
        }
    }
    let mut containers = Vec::new();
    let mut dirs: Vec<&String> = subdirs.keys().collect();
    dirs.sort();
    for dir in dirs {
        let subs = &subdirs[dir];
        if subs.len() < MIN_MODULES {
            continue;
        }
        let mut count: BTreeMap<&str, (usize, bool)> = BTreeMap::new();
        for s in subs {
            for (name, is_dir) in entries.get(s).into_iter().flatten() {
                let c = count.entry(name.as_str()).or_default();
                c.0 += 1;
                c.1 |= *is_dir;
            }
        }
        let mut parts: Vec<(&str, usize, bool)> = count.into_iter().filter(|(_, (n, _))| n * 2 >= subs.len()).map(|(k, (n, d))| (k, n, d)).collect();
        if parts.len() < MIN_PARTS || !parts.iter().any(|p| p.2) {
            continue;
        }
        let names: BTreeSet<&str> = parts.iter().map(|p| p.0).collect();
        let holding = subs.iter().filter(|s| entries.get(*s).is_some_and(|e| e.keys().filter(|k| names.contains(k.as_str())).count() >= MIN_PARTS)).count();
        if (holding as f32) < MODULE_SHARE * subs.len() as f32 {
            continue;
        }
        parts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        containers.push(Container { path: dir.clone(), parts: parts.into_iter().map(|p| p.0.to_owned()).collect() });
    }
    Layout { containers }
}

/// The module a file belongs to: under the deepest container that holds it.
pub fn place(layout: &Layout, path: &str) -> Option<Place> {
    let c = layout
        .containers
        .iter()
        .filter(|c| c.path.is_empty() || path.starts_with(&format!("{}/", c.path)))
        .max_by_key(|c| c.path.len())?;
    let rest = if c.path.is_empty() { path } else { &path[c.path.len() + 1..] };
    let mut segs = rest.split('/').filter(|s| !s.is_empty());
    let module = segs.next()?;
    let inner = segs.next()?; // a file directly in the container isn't in a module
    let part = stem(inner);
    Some(Place {
        container: c.path.clone(),
        module: module.to_owned(),
        part: c.parts.iter().any(|p| p == part).then(|| part.to_owned()),
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    New,
    Changed,
    Removed,
}

/// One module touched by a lane.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleRow {
    pub name: String,
    /// Full repo path of the module folder (for the PR filter).
    pub path: String,
    pub kind: Kind,
    /// Parts touched, in the container's learned order, with how many files each.
    pub parts: Vec<(String, usize)>,
    /// Files in no learned part.
    pub other: usize,
    pub added: u64,
    pub removed: u64,
    /// PRs that touched it, ascending.
    pub prs: Vec<u64>,
}

/// A lane's changes by module, plus everything outside modules as shared folders.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Summary {
    pub modules: Vec<ModuleRow>,
    pub more_modules: usize,
    pub shared: Folders,
}

pub fn summarize(layout: &Layout, details: &[PrDetail]) -> Summary {
    #[derive(Default)]
    struct Acc {
        parts: BTreeMap<String, usize>,
        other: usize,
        added: u64,
        removed: u64,
        prs: BTreeSet<u64>,
        files: usize,
        new_files: usize,
        gone_files: usize,
        container: String,
    }
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    let mut shared: Vec<PrDetail> = Vec::new();
    for d in details {
        let mut outside = PrDetail { number: d.number, author: d.author.clone(), ..PrDetail::default() };
        for (path, a, r) in &d.files {
            match place(layout, path) {
                Some(p) => {
                    let key = if p.container.is_empty() { p.module.clone() } else { format!("{}/{}", p.container, p.module) };
                    let m = acc.entry(key).or_default();
                    m.container = p.container.clone();
                    match p.part {
                        Some(part) => *m.parts.entry(part).or_default() += 1,
                        None => m.other += 1,
                    }
                    m.added += a;
                    m.removed += r;
                    m.prs.insert(d.number);
                    m.files += 1;
                    m.new_files += usize::from(d.added_files.contains(path));
                    m.gone_files += usize::from(d.deleted_files.contains(path));
                }
                None => outside.files.push((path.clone(), *a, *r)),
            }
        }
        if !outside.files.is_empty() {
            shared.push(outside);
        }
    }
    let mut modules: Vec<ModuleRow> = acc
        .into_iter()
        .map(|(path, m)| {
            let order = layout.containers.iter().find(|c| c.path == m.container).map(|c| c.parts.clone()).unwrap_or_default();
            let mut parts: Vec<(String, usize)> = m.parts.into_iter().collect();
            parts.sort_by_key(|(p, _)| order.iter().position(|o| o == p).unwrap_or(usize::MAX));
            let kind = if m.new_files == m.files {
                Kind::New
            } else if m.gone_files == m.files {
                Kind::Removed
            } else {
                Kind::Changed
            };
            ModuleRow {
                name: path.rsplit('/').next().unwrap_or(&path).to_owned(),
                path,
                kind,
                parts,
                other: m.other,
                added: m.added,
                removed: m.removed,
                prs: m.prs.into_iter().collect(),
            }
        })
        .collect();
    modules.sort_by(|a, b| (b.added + b.removed).cmp(&(a.added + a.removed)).then_with(|| a.path.cmp(&b.path)));
    let more_modules = modules.len().saturating_sub(MAX_MODULE_ROWS);
    modules.truncate(MAX_MODULE_ROWS);
    Summary { modules, more_modules, shared: shared_folders(&shared) }
}

/// Code outside modules, by the full folder its files sit in ("src/shared/component"), biggest
/// first; a row is a `root` FolderShare so the PR filter matches exactly that folder.
fn shared_folders(details: &[PrDetail]) -> Folders {
    let mut by_dir: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for (path, a, r) in details.iter().flat_map(|d| d.files.iter()) {
        let dir = path.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
        let e = by_dir.entry(dir).or_default();
        e.0 += a;
        e.1 += r;
    }
    let total: u64 = by_dir.values().map(|(a, r)| a + r).sum();
    let mut all: Vec<FolderShare> = by_dir
        .into_iter()
        .map(|(dir, (added, removed))| FolderShare {
            label: if dir.is_empty() { "(root)".to_owned() } else { dir.clone() },
            path: dir,
            root: true,
            added,
            removed,
            share: if total == 0 { 0.0 } else { (added + removed) as f32 / total as f32 },
        })
        .collect();
    all.sort_by(|a, b| (b.added + b.removed).cmp(&(a.added + a.removed)).then_with(|| a.path.cmp(&b.path)));
    let rest = all.split_off(all.len().min(MAX_SHARED_ROWS));
    Folders { others: rest.len(), others_lines: rest.iter().map(|f| f.added + f.removed).sum(), top: all }
}

/// Every file path in the repo at `branch` (one `gh api` request). A tree GitHub cuts short
/// (very large repos) is used as far as it goes.
pub fn fetch_tree(slug: &str, branch: &str) -> Result<Vec<String>, String> {
    let branch: String = branch.chars().map(|c| match c {
        '/' => "%2F".to_owned(),
        '#' => "%23".to_owned(),
        '?' => "%3F".to_owned(),
        c => c.to_string(),
    }).collect();
    let out = crate::gh::run("gh", &["api", &format!("repos/{slug}/git/trees/{branch}?recursive=1"), "--jq", r#".tree[] | select(.type == "blob") | .path"#], None)?;
    Ok(out.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    /// The user's layout: src/feature/<module>/{view,service,locale,route.ts,navigation.ts},
    /// shared code beside it, and an asset folder of look-alike files.
    fn app() -> Vec<String> {
        let mut t = Vec::new();
        for m in ["agent", "report", "auth", "audit-log"] {
            for f in ["view/Main.vue", "service/index.ts", "locale/en.ts", "locale/th.ts", "route.ts", "navigation.ts"] {
                t.push(format!("src/feature/{m}/{f}"));
            }
        }
        t.push("src/feature/report/component/Chart.vue".into()); // a part only one module has
        for c in ["Table.vue", "Button.vue", "Modal.vue"] {
            t.push(format!("src/shared/component/{c}"));
        }
        t.extend(tree(&["src/shared/lib/http.ts", "src/app/config/env.ts", "package.json", "README.md"]));
        for set in ["a", "b", "c"] {
            for n in 1..=5 {
                t.push(format!("public/images/{set}/{n}.png"));
            }
        }
        t
    }

    #[test]
    fn learns_the_feature_container_and_its_parts() {
        let l = learn(&app());
        assert_eq!(l.containers.len(), 1, "{l:?}");
        assert_eq!(l.containers[0].path, "src/feature");
        let parts: BTreeSet<&str> = l.containers[0].parts.iter().map(String::as_str).collect();
        assert_eq!(parts, BTreeSet::from(["locale", "navigation", "route", "service", "view"]));
    }

    #[test]
    fn asset_folders_of_lookalike_files_are_not_containers() {
        // public/images/{a,b,c} all hold 1..5.png, but no shared subfolder → not modules.
        assert!(learn(&app()).containers.iter().all(|c| !c.path.starts_with("public")));
    }

    #[test]
    fn monorepo_packages_are_modules() {
        let mut t = Vec::new();
        for p in ["api", "web", "cli"] {
            t.extend([format!("packages/{p}/src/index.ts"), format!("packages/{p}/package.json"), format!("packages/{p}/README.md")]);
        }
        let l = learn(&t);
        assert_eq!(l.containers.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["packages"]);
    }

    #[test]
    fn flat_repo_learns_nothing() {
        assert!(learn(&tree(&["main.rs", "lib.rs", "README.md", "src/a.rs", "src/b.rs"])).containers.is_empty());
    }

    #[test]
    fn nested_containers_use_the_deepest() {
        let mut t = Vec::new();
        for app in ["admin", "shop", "docs"] {
            t.extend([format!("apps/{app}/src/main.ts"), format!("apps/{app}/package.json")]);
        }
        for m in ["cart", "order", "user"] {
            t.extend([format!("apps/shop/src/modules/{m}/view/x.vue"), format!("apps/shop/src/modules/{m}/store/x.ts")]);
        }
        let l = learn(&t);
        let p = place(&l, "apps/shop/src/modules/cart/view/x.vue").unwrap();
        assert_eq!((p.container.as_str(), p.module.as_str(), p.part.as_deref()), ("apps/shop/src/modules", "cart", Some("view")));
        assert_eq!(place(&l, "apps/admin/src/main.ts").unwrap().module, "admin");
    }

    #[test]
    fn place_finds_module_and_part() {
        let l = learn(&app());
        let p = place(&l, "src/feature/report/route.ts").unwrap();
        assert_eq!((p.module.as_str(), p.part.as_deref()), ("report", Some("route")));
        assert_eq!(place(&l, "src/feature/report/component/Chart.vue").unwrap().part, None);
        assert_eq!(place(&l, "src/shared/component/Table.vue"), None);
        assert_eq!(place(&l, "src/feature/README.md"), None, "a file directly in the container is in no module");
    }

    fn pr(number: u64, files: &[(&str, u64, u64)], added: &[&str], deleted: &[&str]) -> PrDetail {
        PrDetail {
            number,
            files: files.iter().map(|(p, a, r)| (p.to_string(), *a, *r)).collect(),
            added_files: added.iter().map(|p| p.to_string()).collect(),
            deleted_files: deleted.iter().map(|p| p.to_string()).collect(),
            ..PrDetail::default()
        }
    }

    #[test]
    fn summarize_by_module_with_kind_parts_and_shared() {
        let l = learn(&app());
        let new_report = ["src/feature/report/view/Main.vue", "src/feature/report/service/index.ts", "src/feature/report/route.ts"];
        let details = [
            pr(121, &[(new_report[0], 200, 0), (new_report[1], 60, 0), (new_report[2], 16, 0)], &new_report, &[]),
            pr(118, &[("src/feature/agent/view/Main.vue", 10, 4), ("src/feature/agent/component/x.vue", 3, 0)], &[], &[]),
            pr(116, &[("src/shared/component/Table.vue", 5, 5)], &[], &[]),
        ];
        let s = summarize(&l, &details);
        assert_eq!(s.modules.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["report", "agent"], "biggest first");
        let report = &s.modules[0];
        assert_eq!((report.kind, report.path.as_str(), report.prs.clone()), (Kind::New, "src/feature/report", vec![121]));
        assert_eq!(report.parts.iter().map(|(p, n)| (p.as_str(), *n)).collect::<Vec<_>>().len(), 3);
        let agent = &s.modules[1];
        assert_eq!((agent.kind, agent.other, agent.parts.clone()), (Kind::Changed, 1, vec![("view".to_string(), 1)]));
        assert_eq!(s.shared.top.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["src/shared/component"]);
    }

    #[test]
    fn summarize_marks_removed_modules() {
        let l = learn(&app());
        let gone = ["src/feature/auth/view/Main.vue", "src/feature/auth/route.ts"];
        let s = summarize(&l, &[pr(9, &[(gone[0], 0, 80), (gone[1], 0, 10)], &[], &gone)]);
        assert_eq!(s.modules[0].kind, Kind::Removed);
    }

    #[test]
    fn summarize_caps_rows() {
        let mut t = Vec::new();
        let mut files = Vec::new();
        for i in 0..12 {
            t.extend([format!("m/x{i}/view/a.vue"), format!("m/x{i}/store/a.ts")]);
            files.push((format!("m/x{i}/view/a.vue"), 10 + i as u64, 0));
        }
        let l = learn(&t);
        let d = PrDetail { number: 1, files, ..PrDetail::default() };
        let s = summarize(&l, &[d]);
        assert_eq!((s.modules.len(), s.more_modules), (MAX_MODULE_ROWS, 4));
    }
}
