//! Finds git clones under a folder, for the Settings "scan a folder" action. Pure: no State, no UI.

// Wired into Settings and launch in a later task.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::git;

/// Deepest level searched. `root` is depth 0, `root/a` is depth 1, so a clone at `root/a/b/c/d/e`
/// (depth 5) is found and one at depth 6 is not.
pub const MAX_DEPTH: usize = 5;
/// Most folders read in one scan, so a huge disk can't hang it.
pub const MAX_DIRS: usize = 20_000;

const IGNORED: [&str; 5] = ["node_modules", "target", "vendor", "dist", "build"];

pub struct Found {
    pub slug: String,
    pub path: PathBuf,
    pub depth: usize,
}

pub struct Scan {
    pub found: Vec<Found>,
    /// Clones with no GitHub `origin`, plus folders that couldn't be read.
    pub skipped: usize,
    /// The folder cap was hit before the walk finished.
    pub stopped_early: bool,
}

pub fn scan(root: &Path) -> Scan {
    walk(root, MAX_DEPTH, MAX_DIRS, &|p| git::slug_of(p))
}

/// Walks down from `root` (depth 0). A folder holding a `.git` entry (folder or file) is a clone:
/// it is recorded and not descended into. Hidden and build folders and symlinks are not entered.
pub fn walk(root: &Path, max_depth: usize, max_dirs: usize, slug: &dyn Fn(&Path) -> Option<String>) -> Scan {
    let mut out = Scan { found: Vec::new(), skipped: 0, stopped_early: false };
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if visited >= max_dirs {
            out.stopped_early = true;
            break;
        }
        visited += 1;
        if dir.join(".git").symlink_metadata().is_ok() {
            match slug(&dir) {
                Some(s) => match index.get(&s) {
                    Some(&i) => {
                        if depth < out.found[i].depth {
                            out.found[i] = Found { slug: s, path: dir, depth };
                        }
                    }
                    None => {
                        index.insert(s.clone(), out.found.len());
                        out.found.push(Found { slug: s, path: dir, depth });
                    }
                },
                None => out.skipped += 1,
            }
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            out.skipped += 1;
            continue;
        };
        if depth >= max_depth {
            continue;
        }
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if !ft.is_dir() || ft.is_symlink() {
                continue;
            }
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || IGNORED.contains(&name.as_ref()) {
                continue;
            }
            stack.push((e.path(), depth + 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Tree(PathBuf);
    impl Tree {
        fn new(name: &str) -> Tree {
            let p = std::env::temp_dir().join(format!("sigit-scan-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tree(p)
        }
        /// A clone at `rel` whose fake slug is `slug` (None: a `.git` with no GitHub origin).
        fn clone(&self, rel: &str, slug: Option<&str>) {
            let d = self.0.join(rel);
            fs::create_dir_all(d.join(".git")).unwrap();
            if let Some(s) = slug {
                fs::write(d.join("slug.txt"), s).unwrap();
            }
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fake(p: &Path) -> Option<String> {
        fs::read_to_string(p.join("slug.txt")).ok().map(|s| s.trim().to_string())
    }
    fn run(t: &Tree) -> Scan {
        walk(&t.0, 5, 1000, &fake)
    }
    fn slugs(s: &Scan) -> Vec<String> {
        let mut v: Vec<String> = s.found.iter().map(|f| f.slug.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn walk_finds_nested_clones_and_stops_inside_them() {
        let t = Tree::new("nested");
        t.clone("a", Some("o/a"));
        t.clone("b/c", Some("o/c"));
        t.clone("a/sub", Some("o/sub"));
        let s = run(&t);
        assert_eq!(slugs(&s), ["o/a", "o/c"]);
        assert_eq!(s.skipped, 0);
        assert!(!s.stopped_early);
    }

    #[test]
    fn walk_skips_hidden_and_build_folders() {
        let t = Tree::new("hidden");
        for d in [".cache", "node_modules", "target", "vendor", "dist", "build"] {
            t.clone(&format!("{d}/x"), Some("o/x"));
        }
        let s = run(&t);
        assert!(s.found.is_empty());
        assert_eq!(s.skipped, 0);
    }

    #[test]
    fn walk_treats_git_file_as_clone() {
        let t = Tree::new("gitfile");
        let d = t.0.join("wt");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(".git"), "gitdir: elsewhere").unwrap();
        fs::write(d.join("slug.txt"), "o/wt").unwrap();
        assert_eq!(slugs(&run(&t)), ["o/wt"]);
    }

    #[test]
    fn walk_respects_depth() {
        let t = Tree::new("depth");
        t.clone("1/2/3/4/5", Some("o/five"));
        t.clone("a/b/c/d/e/f", Some("o/six"));
        assert_eq!(slugs(&run(&t)), ["o/five"]);
    }

    #[test]
    fn walk_keeps_shallower_duplicate() {
        let t = Tree::new("dup");
        t.clone("x/y/z", Some("o/same"));
        t.clone("a", Some("o/same"));
        let s = run(&t);
        assert_eq!(s.found.len(), 1);
        assert_eq!(s.found[0].depth, 1);
        assert!(s.found[0].path.ends_with("a"));
    }

    #[test]
    fn walk_counts_non_github_as_skipped() {
        let t = Tree::new("nongh");
        t.clone("local", None);
        t.clone("gh", Some("o/gh"));
        let s = run(&t);
        assert_eq!(s.skipped, 1);
        assert_eq!(slugs(&s), ["o/gh"]);
    }

    #[test]
    fn walk_stops_at_dir_cap() {
        let t = Tree::new("cap");
        for i in 0..30 {
            fs::create_dir_all(t.0.join(format!("d{i}"))).unwrap();
        }
        let s = walk(&t.0, 5, 10, &fake);
        assert!(s.stopped_early);
    }

    #[test]
    fn walk_skips_unreadable() {
        let t = Tree::new("unread");
        let file = t.0.join("plain.txt");
        fs::write(&file, "x").unwrap();
        assert!(walk(&file, 5, 1000, &fake).skipped >= 1);
        assert!(walk(&t.0.join("missing"), 5, 1000, &fake).skipped >= 1);
    }
}
