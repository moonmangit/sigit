# Dashboard, Branch Comparisons and Clone-Folder Scan Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename Inbox to Dashboard, add a Dashboard tab comparing named branch pairs across tracked repos (commits and PRs waiting), and let sigit find local clones in remembered folders.

**Architecture:** Two new pure-logic modules, `src/compare.rs` (comparison settings, per-repo overrides, the batched GraphQL query and its parsing) and `src/scan.rs` (the folder walk), each with inline unit tests like the rest of the crate. UI lives in a new `src/ui/compare.rs` plus additions to `src/ui/settings.rs`. State gets a `Load<Vec<RepoCompare>>` loaded through the existing `load_stale` / `refresh_page` / cache machinery.

**Tech Stack:** Rust 2024, Rune / rune-kit UI, `gh api graphql`, serde, `crate::jobs`, `crate::i18n` (Thai catalog `assets/i18n/th.tsv`).

**Spec:** `docs/superpowers/specs/2026-11-19-dashboard-comparisons-scan-design.md`

## Global Constraints

- Nothing writes into a user's clone: no `git fetch`, no config writes. The scan reads the filesystem and runs only `git remote get-url origin` (via `git::slug_of`) on folders that have `.git`.
- All network and scanning work runs through `jobs::spawn` / `jobs::spawn_each`; the window never waits.
- New prefs fields go on `config::Prefs` (already `#[serde(default)]`); old `prefs.json` files must load unchanged.
- Every visible string goes through `crate::i18n` (`t`, `text`, `tf!`) and gets a Thai line in `assets/i18n/th.tsv`; `python scripts/i18n-check.py` exits 0 at the end of every task.
- Page name: **Dashboard** / Thai **แดชบอร์ด**.
- Defaults: comparisons `To staging: dev → staging`, `Back-merge: staging → dev` (shown `dev ← staging`), `To production: staging → main`. Branch names are literal.
- Scan: depth **5**, skip hidden folders and `node_modules`, `target`, `vendor`, `dist`, `build`; stop after **20,000** directories per folder; don't descend into a found repo.
- Comparisons: one batched query, at most **10 repos per query**; commits read for PR counting up to **500** per comparison; refresh `ttl::GH` (120 s), revalidated on tab open and window focus.
- Run `cargo test` and `python scripts/i18n-check.py` before each commit. Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Branch names with `/` or quotes** (`feature/x`, `it's`) in comparisons must reach GitHub intact, not break the query → Task 3 test `query_escapes_branch_names`.
2. **A tracked repo GitHub won't let you read** (renamed, no access) must show its error on its own row while the other rows load → Task 3 test `parse_keeps_other_repos_when_one_fails`.
3. **No tracked repos / no comparisons** must show a helpful empty state, not a blank tab or a request → Task 4 test `refresh_compare_skips_when_nothing_to_compare`.
4. **Picking a huge folder (`C:\`) or one with unreadable subfolders** must end quickly and say so, not hang or error → Task 7 test `walk_stops_at_dir_cap` and `walk_skips_unreadable`.
5. **Renaming or deleting a shared comparison** must carry or drop its per-repo overrides, never leave an orphan that silently turns a column off → Task 2 tests `rename_carries_overrides`, `delete_drops_overrides`.

---

### Task 1: Rename Inbox → Dashboard

**Files:**
- Modify: `src/main.rs:201` (`Page::label`), `src/ui/mod.rs:366` (toolbar title), `src/ui/sidebar.rs:412`, `src/daily.rs:435,464` (palette command), `src/ui/settings.rs:526` (shortcuts list), `src/ui/mod.rs` (`crash_screen` button "Go to Inbox"), guide strings in `src/ui/overlays.rs` (`GUIDE`) that say "Inbox"
- Modify: `assets/i18n/th.tsv`, `README.md`, `docs/design-notes.md`
- Test: `src/daily.rs` tests module

**Interfaces:**
- Produces: `pub fn State::new(prefs: config::Prefs) -> State` in `src/main.rs`: the `State { … }` literal now in `init()` (`src/main.rs:426`), with no disk reads, no jobs and no preflight; `init()` becomes load-from-disk + `State::new(prefs)` + its current startup work. Every later task's tests build state with `State::new(config::Prefs::default())`.
- Produces: palette command name `"Dashboard"` (used by `palette_pick`). Internal names `Page::Inbox`, `inbox_tab`, `ui/inbox.rs`, `I_INBOX` stay as they are (decided: keep, less churn).

- [ ] **Step 1: Split `init()`** into `State::new(prefs)` and the disk/startup part as described above. Run `cargo test` → existing tests PASS (pure move).
- [ ] **Step 1b: Write the failing tests** in `src/daily.rs` `mod tests`:

```rust
#[test]
fn palette_finds_dashboard_by_old_name() {
    let mut s = crate::State::new(crate::config::Prefs::default());
    s.palette_q = rune::TextEdit::new("inbox");
    assert!(s.palette_items().iter().any(|(item, title, _)| matches!(item, PaletteItem::Command("Dashboard")) && title == "Dashboard"));
}

#[test]
fn page_is_called_dashboard() {
    assert_eq!(crate::Page::Inbox.label(), "Dashboard");
}
```

- [ ] **Step 2: Run** `cargo test palette_finds_dashboard_by_old_name page_is_called_dashboard` → both FAIL.
- [ ] **Step 3: Rename every user-facing "Inbox" to "Dashboard"** in the files above (labels, title, sidebar row, palette command and its match arm, Shortcuts list, `crash_screen` button → "Go to Dashboard", guide text). In `palette_items`, score the `"Dashboard"` command also against `"inbox"` (e.g. `max` with `fuzzy(&q, "inbox")`).
- [ ] **Step 4: Thai catalog:** add `Dashboard\tแดชบอร์ด`, `Go to Dashboard\tไปที่แดชบอร์ด`, and English→Thai lines for every rewritten guide/shortcut string, using แดชบอร์ด where the old Thai said กล่องงาน; delete catalog lines whose English no longer appears in the code (`Inbox`, `Go to Inbox`, `Inbox view`, the old shortcut and guide lines). Keep `Inbox & notifications` only if that string still exists.
- [ ] **Step 5: Docs:** README and `docs/design-notes.md` say Dashboard for the page (keep "Inbox" only where it names the history, e.g. "formerly Inbox" once in the README).
- [ ] **Step 6: Run** `cargo test` → all PASS; `python scripts/i18n-check.py` → `0 without Thai`; `grep -rn '"Inbox"' src` → only internal/test hits, no user-facing string.
- [ ] **Step 7: Commit** `git commit -am "Rename the Inbox page to Dashboard"`.

---

### Task 2: Comparison settings and per-repo overrides

**Files:**
- Create: `src/compare.rs` (register `mod compare;` in `src/main.rs`)
- Modify: `src/config.rs` (`Prefs` fields + defaults)

**Interfaces:**
- Produces (in `src/compare.rs`):
  - `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)] pub struct Comparison { pub name: String, pub from: String, pub to: String }` — counts commits on `from` that `to` lacks.
  - `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)] pub struct Override { pub slug: String, pub name: String, pub from: Option<String>, pub to: Option<String>, pub off: bool }`
  - `pub fn defaults() -> Vec<Comparison>` — the three Global Constraints defaults, in that order.
  - `pub fn effective(shared: &[Comparison], overrides: &[Override], slug: &str) -> Vec<Option<Comparison>>` — one entry per shared comparison, same order; `None` when off for this repo; `from`/`to` replaced by the override's when set; `name` always the shared name.
  - `pub fn is_changed(overrides: &[Override], slug: &str, name: &str) -> bool`
  - `pub fn rename(prefs: &mut Prefs, old: &str, new: &str)` and `pub fn delete(prefs: &mut Prefs, name: &str)` — update `prefs.comparisons` and `prefs.comparison_overrides` together.
- Produces (in `config::Prefs`): `pub comparisons: Vec<compare::Comparison>` (default `compare::defaults()`), `pub comparison_overrides: Vec<compare::Override>` (default empty), `pub clone_folders: Vec<PathBuf>` (default empty; used by Task 8).

- [ ] **Step 1: Write the failing tests** in `src/compare.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn defaults_are_the_users_three() {
    let d = defaults();
    assert_eq!(d.iter().map(|c| (c.name.as_str(), c.from.as_str(), c.to.as_str())).collect::<Vec<_>>(),
        vec![("To staging", "dev", "staging"), ("Back-merge", "staging", "dev"), ("To production", "staging", "main")]);
}

#[test]
fn effective_applies_overrides() {
    let shared = defaults();
    let ov = vec![
        Override { slug: "a/b".into(), name: "To production".into(), from: None, to: Some("master".into()), off: false },
        Override { slug: "a/b".into(), name: "Back-merge".into(), off: true, ..Default::default() },
        Override { slug: "x/y".into(), name: "To staging".into(), off: true, ..Default::default() },
    ];
    let e = effective(&shared, &ov, "a/b");
    assert_eq!(e[0].as_ref().unwrap().from, "dev");
    assert!(e[1].is_none());
    assert_eq!((e[2].as_ref().unwrap().from.as_str(), e[2].as_ref().unwrap().to.as_str()), ("staging", "master"));
    assert!(is_changed(&ov, "a/b", "To production") && !is_changed(&ov, "a/b", "To staging"));
}

#[test]
fn rename_carries_overrides() { /* rename "Back-merge" → "Hotfix back"; the a/b override now has name "Hotfix back" and effective(..)[1] is still None */ }

#[test]
fn delete_drops_overrides() { /* delete "To production"; prefs.comparisons has 2 entries and no override names "To production" */ }

#[test]
fn old_prefs_get_default_comparisons() {
    let p: crate::config::Prefs = serde_json::from_str(r#"{"pinned":["a/b"]}"#).unwrap();
    assert_eq!(p.comparisons, defaults());
    assert!(p.comparison_overrides.is_empty() && p.clone_folders.is_empty());
}
```

- [ ] **Step 2: Run** `cargo test compare::` → FAIL (module/types missing).
- [ ] **Step 3: Implement** the types and functions above and the three `Prefs` fields.
- [ ] **Step 4: Run** `cargo test` → PASS.
- [ ] **Step 5: Commit** `git commit -m "Branch comparison settings with per-repo overrides"`.

---

### Task 3: The batched query and its parsing

**Files:**
- Modify: `src/compare.rs`

**Interfaces:**
- Consumes: `Comparison`, `effective` (Task 2); `gh::run(program, args, cwd)` (`src/gh.rs:120`).
- Produces (all `Clone, Debug, PartialEq, Serialize, Deserialize`):
  - `pub struct WaitingPr { pub number: u64, pub title: String, pub author: String, pub merged_at: String }`
  - `pub struct LooseCommit { pub sha: String, pub subject: String, pub author: String }`
  - `pub enum Cell { Off, Missing(Vec<String>), Waiting { commits: u32, prs: Vec<WaitingPr>, loose: Vec<LooseCommit>, checked: u32 } }` — `Missing` holds the missing branch names; `checked` = commits read for PR counting (≤ 500).
  - `pub struct RepoCompare { pub slug: String, pub cells: Vec<Cell>, pub error: Option<String> }` — `cells` aligned with the shared comparisons.
  - `pub fn query(rows: &[(String, Vec<Option<Comparison>>)]) -> String` — one GraphQL document; aliases `r{i}` per repo and `c{j}` per comparison; each comparison is `c{j}: ref(qualifiedName: "refs/heads/<to>") { compare(headRef: "refs/heads/<from>") { aheadBy commits(first: 100) { pageInfo { hasNextPage endCursor } nodes { oid messageHeadline author { name user { login } } associatedPullRequests(first: 5) { nodes { number title author { login } mergedAt baseRefName state } } } } } }`, plus `f{j}: ref(qualifiedName: "refs/heads/<from>") { name }` to tell which branch is missing. Off comparisons produce nothing.
  - `pub fn parse(rows: &[(String, Vec<Option<Comparison>>)], json: &serde_json::Value) -> (Vec<RepoCompare>, Vec<(usize, usize, String)>)` — the rows, plus `(row, comparison, endCursor)` for comparisons with more commits to read.
  - `pub fn page_query(slug: &str, c: &Comparison, after: &str) -> String` and `pub fn apply_page(cell: &mut Cell, c: &Comparison, json: &serde_json::Value) -> Option<String>` (next cursor, `None` when done or `checked` reached 500).
  - `pub fn fetch(rows: Vec<(String, Vec<Option<Comparison>>)>) -> Vec<RepoCompare>` — splits into chunks of 10 repos, runs `gh api graphql -f query=…` per chunk, follows pages, never returns `Err`: a failed chunk or repo sets `error` on its rows.

PR rule (the one algorithm tests don't fully pin): for each commit, take its `associatedPullRequests` with `state == "MERGED"`; prefer the one whose `baseRefName == c.from`, else the earliest `mergedAt`; collect distinct PR numbers into `prs`; commits with no merged PR go to `loose`. `commits` is `aheadBy` (exact, independent of `checked`).

- [ ] **Step 1: Write the failing tests** (build the JSON fixtures inline with `serde_json::json!`, in the shape GitHub returns for the query above):

```rust
#[test]
fn query_escapes_branch_names() {
    let rows = vec![("o/n".to_string(), vec![Some(Comparison { name: "x".into(), from: "feature/it's \"q\"".into(), to: "main".into() })])];
    let q = query(&rows);
    assert!(q.contains(r#"refs/heads/feature/it's \"q\""#)); // quotes escaped, slash kept
    assert!(q.contains(r#"repository(owner: "o", name: "n")"#));
}

#[test]
fn parse_counts_commits_and_squash_prs() { /* aheadBy 7, 7 commits each with one MERGED PR into "staging" (#116..#122) → Waiting { commits: 7, prs.len() == 7, loose empty, checked 7 } */ }

#[test]
fn parse_prefers_pr_into_from_branch() { /* one commit with PRs #5 (base "dev", merged later) and #9 (base "staging", merged earlier), comparison from "dev" → prs == [#5] */ }

#[test]
fn parse_ignores_open_prs_and_keeps_loose_commits() { /* commit with only an OPEN PR → in `loose`, prs empty */ }

#[test]
fn parse_reports_missing_branches() { /* c0 null and f0 null → Missing(vec!["dev","staging"]) ordered from, to; c0 null and f0 present → Missing(vec!["staging"]) */ }

#[test]
fn parse_keeps_other_repos_when_one_fails() { /* r0 null with an "errors" entry whose path starts with "r0" → rows[0].error is Some(message), rows[1] parsed normally */ }

#[test]
fn parse_asks_for_more_pages() { /* hasNextPage true, endCursor "abc" → second return value == vec![(0, 0, "abc".into())] */ }

#[test]
fn apply_page_stops_at_500() { /* cell with checked 450, page of 100 nodes with hasNextPage true → checked 500 and apply_page returns None */ }
```

- [ ] **Step 2: Run** `cargo test compare::` → new tests FAIL.
- [ ] **Step 3: Implement** `query`, `parse`, `page_query`, `apply_page`, `fetch` as specified; GraphQL string escaping for `\` and `"`.
- [ ] **Step 4: Run** `cargo test` → PASS. Then by hand: `cargo run -- --tree` is not needed; instead add a `#[test] #[ignore] fn live_fetch()` that calls `fetch` for `threadsoftwareth/aom-bet-lottery-admin-fe` with the defaults and prints the cells; run `cargo test live_fetch -- --ignored --nocapture` → `To production` shows 7 commits / 7 PRs (or today's real numbers, matching github.com's compare page).
- [ ] **Step 5: Commit** `git commit -m "Batched GitHub query for branch comparisons"`.

---

### Task 4: Loading, refreshing and caching comparisons

**Files:**
- Modify: `src/main.rs` (State field, `load_stale` / `refresh_page` for `Page::Inbox`, cache restore/save), `src/cache.rs` (`Snapshot` fields)

**Interfaces:**
- Consumes: `compare::{effective, fetch, RepoCompare}`; `State::tracked()` (`src/main.rs:903`).
- Produces:
  - `pub compare: Load<Vec<compare::RepoCompare>>` and `pub compare_key: String` on `State` (key = tracked repos + comparisons + overrides, serialized; a changed key means reload).
  - `pub fn refresh_compare(&mut self)` — rows = `tracked()` sorted A–Z, each with `effective(...)`; does nothing (no request, `compare` left empty) when there are no rows or no comparisons; otherwise `jobs::spawn(fetch, finish)`.
  - `pub fn compare_key_now(&self) -> String`
  - `Snapshot.compare: Option<(String, Vec<compare::RepoCompare>)>`, `Snapshot.compare_at: i64`; restored with `Load::from_cache` only when the key matches.
  - Dashboard tab index constant `pub const COMPARE_TAB: usize = 2;`

- [ ] **Step 1: Write the failing tests** in `src/main.rs` `mod cache_tests`:

```rust
#[test]
fn refresh_compare_skips_when_nothing_to_compare() {
    let mut s = State::new(config::Prefs::default());
    s.prefs.tracked.clear(); s.prefs.pinned.clear(); s.prefs.groups.clear();
    s.refresh_compare();
    assert!(!s.compare.loading && s.compare.data.is_none());
}

#[test]
fn snapshot_round_trips_compare() { /* Snapshot { compare: Some(("k".into(), vec![RepoCompare{..one Waiting cell..}])), compare_at: 9, .. } survives to_string/from_str */ }
```

- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.** In `load_stale`'s `Page::Inbox` arm: when `inbox_tab == COMPARE_TAB` and (`compare.stale(ttl::GH.min(cap))` or key changed) → `refresh_compare()`. `refresh_page` on `Page::Inbox` also calls it when on that tab. Save/restore in the existing snapshot code next to `pulse`.
- [ ] **Step 4: Run** `cargo test` → PASS.
- [ ] **Step 5: Commit** `git commit -m "Load and cache branch comparisons"`.

---

### Task 5: Comparisons tab, detail pane and per-repo dialog

**Files:**
- Create: `src/ui/compare.rs` (register in `src/ui/mod.rs`)
- Modify: `src/ui/mod.rs:517-528` (`inbox_page`: third tab), `src/ui/mod.rs` view (`compare::repo_dialog` layer), `src/main.rs` (State fields + scene)

**Interfaces:**
- Consumes: `State.compare`, `compare::{Cell, RepoCompare, effective, is_changed, Override}`, `State::open_pr(slug, n)`, existing helpers `panel`, `pending`, `empty`, `muted`, `chip`, `side_pane` pattern from `ui/actions.rs`, `overlays::fresh_chip`.
- Produces:
  - `pub fn view(s: &State, k: &Kit) -> El` and `pub fn repo_dialog(s: &State, k: &Kit, layer: &mut Vec<El>)` in `ui/compare.rs`.
  - State: `pub compare_sel: Option<(String, usize)>` (slug, comparison index) for the pane; `pub compare_edit: Option<CompareEdit>` where `pub struct CompareEdit { pub slug: String, pub rows: Vec<(TextEdit, TextEdit, bool)> }` (from, to, on) for the dialog; `pub fn save_compare_edit(&mut self)` writes overrides (omit rows equal to shared and on), saves prefs, reloads; `pub fn reset_compare_edit(&mut self)` removes that repo's overrides.
  - Scene `("comparisons", …)` in `main.rs` scenes: Dashboard on `COMPARE_TAB` with a fixed `compare` fixture (one Waiting, one Missing, one 0, one error row, one changed cell) so `--snap` shows every cell state without network.

Copy (exact, all through i18n): tab `Comparisons`; cell `{commits} commits · {prs} PRs` (singular forms `1 commit`, `1 PR`); zero `up to date`; missing `— no {branch}` / `— no {a} or {b}`; changed-cell tooltip `Changed for this repo`; row menu item `Edit comparisons for this repo…`; dialog title `Comparisons for {slug}`, buttons `Reset to shared`, `Cancel`, `Save`; empty states `Track some repos to compare their branches.` (with a `Manage tracked` button opening the existing tracked dialog) and `Add a comparison in Settings → Git.`; pane sections `Merged PRs waiting ({n})`, `Commits without a PR ({n})`; truncation tooltip `PRs counted from the first {checked} commits`.

- [ ] **Step 1: Write the failing test:** add the `comparisons` scene, then run `cargo run -- --snap <scratch>/cmp.png --scene comparisons --size 1400x860` → FAIL (no tab / panic).
- [ ] **Step 2: Implement** the tab (third `SegmentedControl` entry), table (rows A–Z, header = name + shared `from → to`), cells per copy above, cell click → `compare_sel`, side pane with PRs (click → `open_pr`) then loose commits, row menu → dialog, dialog save/reset.
- [ ] **Step 3: Verify:** the snapshot shows all five cell states readably in light and dark (`--theme dark`); `cargo test` PASS; i18n-check `0 without Thai` (add every new string to `th.tsv`).
- [ ] **Step 4: Commit** `git commit -m "Comparisons tab on the Dashboard"`.

---

### Task 6: Comparisons in Settings → Git

**Files:**
- Modify: `src/ui/settings.rs` (Git section, `src/ui/settings.rs:62` `2 => git_identity(s, k)` becomes the identity block plus the new block), `src/main.rs` (State edit buffers)

**Interfaces:**
- Consumes: `Prefs.comparisons`, `compare::{rename, delete}`, `State::save_prefs()`.
- Produces: State `pub cmp_rows: Vec<(TextEdit, TextEdit, TextEdit)>` (name, from, to) loaded when Settings opens; `pub fn save_comparisons(&mut self) -> Result<(), String>` — validates (name and both branches non-empty, names unique, from ≠ to), applies renames by position via `compare::rename`, removed rows via `compare::delete`, saves prefs, toasts `Comparisons saved`, and marks `compare` stale.

Copy: heading `Branch comparisons`; help `Each counts the commits and PRs on From that To doesn't have yet. Change one per repo from the Dashboard.`; buttons `Add comparison`, `Swap`, `Remove`, `Move up`, `Move down`, `Save`; errors `Every comparison needs a name and two branches`, `Two comparisons are called {name}`, `{name} compares {branch} with itself`.

- [ ] **Step 1: Write the failing test** (in `main.rs` tests):

```rust
#[test]
fn save_comparisons_rejects_duplicates_and_keeps_overrides_on_rename() {
    // rows: ("A","dev","staging"),("A","staging","main") → Err("Two comparisons are called A"), prefs unchanged
    // rows renamed "Back-merge"→"Hotfix back" with an override on it → Ok, override follows the new name
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** the block and `save_comparisons`. **Step 4: Run** `cargo test` → PASS; snapshot Settings → Git (`--scene` that opens settings at section 2, add one if missing) shows the list. i18n-check passes.
- [ ] **Step 5: Commit** `git commit -m "Edit branch comparisons in Settings"`.

---

### Task 7: The folder walk

**Files:**
- Create: `src/scan.rs` (register `mod scan;`)

**Interfaces:**
- Consumes: `git::slug_of(&Path) -> Option<String>` (`src/git.rs:16`).
- Produces:
  - `pub struct Found { pub slug: String, pub path: PathBuf, pub depth: usize }`
  - `pub struct Scan { pub found: Vec<Found>, pub skipped: usize, pub stopped_early: bool }` — `skipped` = clones with no GitHub `origin` + unreadable folders.
  - `pub fn walk(root: &Path, max_depth: usize, max_dirs: usize, slug: &dyn Fn(&Path) -> Option<String>) -> Scan` — depth 0 is `root` itself; a dir with a `.git` entry (dir or file) is a clone: record it and don't descend; skip names starting with `.` and `node_modules`, `target`, `vendor`, `dist`, `build`; unreadable dir → `skipped += 1`, carry on; stop when `max_dirs` dirs were visited (`stopped_early = true`). The same slug found twice keeps the smaller `depth`.
  - `pub const MAX_DEPTH: usize = 5; pub const MAX_DIRS: usize = 20_000;`
  - `pub fn scan(root: &Path) -> Scan` = `walk(root, MAX_DEPTH, MAX_DIRS, &|p| git::slug_of(p))`.

- [ ] **Step 1: Write the failing tests** (temp dirs under `std::env::temp_dir()`, fake `slug` closure reading a `slug.txt` written next to `.git`, so no real git is needed):

```rust
#[test] fn walk_finds_nested_clones_and_stops_inside_them() { /* root/a/.git, root/b/c/.git, root/a/sub/.git → found a and b/c only */ }
#[test] fn walk_skips_hidden_and_build_folders() { /* clones under .cache/, node_modules/, target/ → none found */ }
#[test] fn walk_treats_git_file_as_clone() { /* root/wt/.git is a file → found */ }
#[test] fn walk_respects_depth() { /* clone at depth 6 → not found; at depth 5 → found */ }
#[test] fn walk_keeps_shallower_duplicate() { /* same slug at depth 1 and 3 → one Found with depth 1 */ }
#[test] fn walk_counts_non_github_as_skipped() { /* .git with no slug.txt → skipped == 1 */ }
#[test] fn walk_stops_at_dir_cap() { /* 30 empty dirs, max_dirs 10 → stopped_early, returns */ }
#[test] fn walk_skips_unreadable() { /* root passed as a file path, or a missing subpath → skipped >= 1, no panic */ }
```

- [ ] **Step 2: Run** `cargo test scan::` → FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test` → PASS.
- [ ] **Step 5: Commit** `git commit -m "Find git clones in a folder"`.

---

### Task 8: Clone folders: settings, launch rescan, adding clones

**Files:**
- Modify: `src/main.rs` (`add_local_repo` at `src/main.rs:1592`: extract saving; `init` at the `config::load()` loop; new methods), `src/ui/settings.rs` (Git section block)

**Interfaces:**
- Consumes: `scan::{scan, Scan, Found}`, `Prefs.clone_folders` (Task 2), `add_repo` (`src/main.rs:608`), `tx.open_folder` as in `ask_local_repo`.
- Produces:
  - `fn save_repo_list(&mut self)` (extracted from `add_local_repo`, used by both).
  - `pub fn add_clones(&mut self, scans: Vec<Scan>) -> (usize, usize)` — (new, already known); a repo already with a path keeps it; saves the repo list once.
  - `pub fn rescan_folders(&mut self, announce: bool)` — `jobs::spawn` scanning every `prefs.clone_folders` entry, then `add_clones`; toast rules below.
  - `pub fn ask_clone_folder(s: &mut State, tx: &mut Tx)` — folder picker; adds to `prefs.clone_folders` (no duplicates), saves prefs, `rescan_folders(true)`.
  - `pub fn remove_clone_folder(&mut self, i: usize)` — removes the folder only (clones already added stay).

Toasts (exact copy): announced scan `Found {n} clones · {new} new · {known} already known · {skipped} skipped`, plus ` · stopped early in {folder}` when `stopped_early`; launch scan (`announce = false`) only when `new > 0`: `Added {new} new clones from your folders`. Settings copy: heading `Clone folders`; help `sigit looks for git clones in these folders at launch, so Commits and Branches work without locating each one.`; buttons `Add folder…`, `Rescan now`, `Remove`; empty `No folders yet.`

- [ ] **Step 1: Write the failing test** in `main.rs` tests:

```rust
#[test]
fn add_clones_keeps_known_paths_and_counts() {
    // state with repo "a/b" at path P1; scans contain a/b at P2 and c/d at P3
    // → returns (1, 1); a/b still at P1; c/d at P3 and listed first (local clones sort first)
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement**; call `rescan_folders(false)` in `init` after the saved repos load (skip in headless runs: `jobs::is_inline_run()`).
- [ ] **Step 4: Run** `cargo test` → PASS; i18n-check passes. By hand: add `C:\Users\<you>\Desktop\Project` in Settings → Git, see the toast, the 4 known clones keep their paths, others appear with Commits working; relaunch → no toast unless a new clone was added.
- [ ] **Step 5: Commit** `git commit -m "Clone folders: find local clones by themselves"`.

---

## Final check

- [ ] `cargo test` all pass; `cargo build --release` no warnings; `python scripts/i18n-check.py` → `0 without Thai`.
- [ ] Snapshots of the Dashboard (`--scene inbox`), `comparisons` scene and Settings → Git look right in light and dark.
- [ ] README: a short "Comparisons" and "Clone folders" section, and the file table lists `compare.rs`, `scan.rs`, `ui/compare.rs`.
