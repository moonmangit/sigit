# Lane Details Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a comparison lane is opened, show each PR's feature info, a By PR / By contributor switch, and a folder diagram of the code the lane changes, while keeping today's pane.

**Architecture:** An on-demand GraphQL fetch per lane (`compare::fetch_details`, batches of 20 PRs, parallel) feeds a session-only `State.lane_details` map. Pure grouping logic (folders, authors, filter) lives in a new `src/lane.rs` so it's unit-tested without UI. The pane in `src/ui/compare.rs` renders both views and the diagram from those.

**Tech Stack:** Rust 2024, Rune / rune-kit UI, `gh api graphql`, serde_json, `crate::jobs`, `crate::i18n`.

**Spec:** `docs/superpowers/specs/2026-11-19-lane-details-design.md`

## Global Constraints

- Nothing is fetched until a lane is opened; nothing new on the Comparisons tab's own load.
- Requests: at most **20 PRs** per GraphQL request, batches run in parallel; read-only `gh api graphql` via `jobs::spawn`.
- Today's pane content always shows (PR list, commits without a PR), also while details load or after they fail.
- View code never indexes fetched lists (`.get()` only); no new panic paths.
- `summary` = first non-empty line of `bodyText`, trimmed, at most **140** characters.
- Files per PR: first **100** (`files(first: 100)`); PR additions/deletions are exact.
- Diagram: top **8** folders by lines changed (added + removed), rest as "other N folders"; descend up to **4** levels.
- Details kept for the session only (not in cache.json).
- Copy (exact, through i18n, Thai in `assets/i18n/th.tsv`): `By PR`, `By contributor`, `{n} PRs · +{a} −{d}`, `Profile`, `(root)`, `other {n} folders`, `Clear`, `Changed code`, `Couldn't load PR details: {error}`, `Retry`, `Showing PRs that touched {folder}`. `python scripts/i18n-check.py` → `0 without Thai`.
- Tests: `cargo test` (all pass, no warnings). Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Empty profile fields and bots** (`company`/`name` null, `author` a Bot or null for a deleted user) must show the login (or "ghost") and no blank labels → Task 1 test `parse_details_handles_missing_profile`.
2. **A PR GitHub can't return** (deleted, no access) must drop just that PR, the rest still load → Task 1 test `parse_details_skips_null_pr`.
3. **Files at the repo root and lanes whose files all share one deep folder** must still give a useful diagram (not one 100% segment named after the shared prefix) → Task 2 tests `folders_descend_when_single_group`, `folders_root_files`.
4. **Switching lanes while details are loading** must not show one lane's details in another lane's pane → Task 3 test `lane_details_are_keyed_by_lane`.
5. **A folder filter left on when switching lanes** must not hide everything in the new lane → Task 3 test `switching_lane_clears_folder_filter`.

---

### Task 1: Fetch and parse PR details

**Files:**
- Modify: `src/compare.rs`, `assets/i18n/th.tsv`

**Interfaces:**
- Consumes: `graphql(q: &str) -> Result<serde_json::Value, String>` and `esc()` already in `src/compare.rs`.
- Produces (all `Clone, Debug, PartialEq, Serialize, Deserialize, Default`):
  - `pub struct Person { pub login: String, pub name: String, pub company: String }`
  - `pub struct PrDetail { pub number: u64, pub labels: Vec<String>, pub summary: String, pub additions: u64, pub deletions: u64, pub author: Person, pub files: Vec<(String, u64, u64)> }`
  - `pub fn details_query(slug: &str, numbers: &[u64]) -> String` — one `repository(owner: "o", name: "n")`, alias `p{i}` per PR.
  - `pub fn parse_details(numbers: &[u64], json: &serde_json::Value) -> Vec<PrDetail>`
  - `pub fn fetch_details(slug: &str, numbers: Vec<u64>) -> Result<Vec<PrDetail>, String>` — `Err` only when every batch failed.

- [ ] **Step 1: Write the failing tests** in `src/compare.rs` tests:

```rust
#[test] fn details_query_aliases_and_escapes() {
    let q = details_query("o/n\"x", &[5, 9]);
    assert!(q.contains(r#"repository(owner: "o", name: "n\"x")"#));
    assert!(q.contains("p0: pullRequest(number: 5)") && q.contains("p1: pullRequest(number: 9)"));
    assert!(q.contains("files(first: 100)"));
}
#[test] fn parse_details_reads_fields() { /* p0 with labels ["ui"], bodyText "\n\n  Adds the win-loss page.\nMore", additions 421, deletions 3, author {login "pluem", name "Pluem", company "Thread"}, 2 files → PrDetail { summary: "Adds the win-loss page.", files: [("src/a.ts",400,3),("src/b.ts",21,0)], .. } */ }
#[test] fn parse_details_caps_summary_at_140() { /* body of 200 chars on one line → summary.chars().count() == 140 */ }
#[test] fn parse_details_handles_missing_profile() { /* name null, company null → Person { login: "x", name: "", company: "" }; author null → login "ghost" */ }
#[test] fn parse_details_skips_null_pr() { /* numbers [1,2], p0 null, p1 present → one PrDetail with number 2 */ }
```

- [ ] **Step 2: Run** `cargo test compare::` → new tests FAIL (not defined).
- [ ] **Step 3: Implement** the types and functions. `fetch_details`: chunks of 20, `std::thread::scope` like `fetch`, parse each batch with its own numbers; collect PRs in the order of `numbers`.
- [ ] **Step 4: Run** `cargo test` → PASS. Add `#[test] #[ignore] fn live_fetch_details()` for `threadsoftwareth/aom-bet-lottery-admin-fe` PRs `[116,117,118,119,120,121,122]`; run with `-- --ignored --nocapture`; expect 7 details, #121 with 7 files.
- [ ] **Step 5: Commit** `git commit -m "Fetch PR details for a comparison lane"`.

---

### Task 2: Folder shares, author groups, folder filter

**Files:**
- Create: `src/lane.rs` (register `mod lane;` in `src/main.rs`)

**Interfaces:**
- Consumes: `compare::{PrDetail, Person, WaitingPr}`.
- Produces:
  - `pub struct FolderShare { pub label: String, pub path: String, pub root: bool, pub added: u64, pub removed: u64, pub share: f32 }` — `label` is what the legend shows (the path with the shared prefix dropped, or `"(root)"`); `path` is the full repo path of the folder (for `root`, the shared-prefix folder itself, `""` at the repo root); `root` = files directly in `path`, not in a subfolder.
  - `pub struct Folders { pub top: Vec<FolderShare>, pub others: usize, pub others_lines: u64 }`.
  - `pub fn folders(details: &[PrDetail]) -> Folders` — algorithm in spec "Folder diagram": drop the folder prefix shared by all files, take the next segment; if exactly one group, go one level deeper, up to 4 levels; top 8 by added + removed, descending; `share` = lines / total lines.
  - `pub fn touches(d: &PrDetail, f: &FolderShare) -> bool` — `root`: some file's parent folder is exactly `f.path`; otherwise some file starts with `f.path + "/"`.
  - `LaneView.folder` (Task 3) stores the selected `FolderShare.path` plus `root` as `Option<(String, bool)>`.
  - `pub struct AuthorGroup { pub person: Person, pub prs: Vec<u64>, pub added: u64, pub removed: u64 }`
  - `pub fn by_author(prs: &[WaitingPr], details: &[PrDetail]) -> Vec<AuthorGroup>` — PR numbers newest merge first within a group; groups by PR count desc, then lines desc, then login. A PR with no detail yet groups under its `WaitingPr.author` login with empty name/company and 0 lines.

- [ ] **Step 1: Write the failing tests** in `src/lane.rs`:

```rust
#[test] fn folders_drop_shared_prefix() { /* src/feature/agent/a.ts (10), src/feature/report/b.ts (30) → labels ["report","agent"], paths ["src/feature/report","src/feature/agent"], shares 0.75/0.25 */ }
#[test] fn folders_descend_when_single_group() { /* src/feature/report/locale/en.ts, src/feature/report/view/x.vue → labels ["locale","view"] (any order by size), not one group */ }
#[test] fn folders_root_files() { /* "README.md" (5) and "src/a.ts" (5) → labels contain "(root)" (root: true, path "") and "src" */ }
#[test] fn folders_top_eight_then_others() { /* 10 folders → top.len()==8, others==2, others_lines == sum of the two smallest */ }
#[test] fn touches_matches_folder() { /* detail with "src/feature/report/x.ts" touches FolderShare{path:"src/feature/report",root:false,..}, not path "src/feature/agent"; a root FolderShare{path:"",root:true} touches "README.md" only */ }
#[test] fn by_author_groups_and_orders() { /* 3 PRs by a (2) and b (1) → [a, b]; a.prs newest merge first; totals summed */ }
#[test] fn by_author_without_details_uses_pr_author() { /* details empty → groups by WaitingPr.author with 0 lines */ }
```

- [ ] **Step 2: Run** `cargo test lane::` → FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test` → PASS.
- [ ] **Step 5: Commit** `git commit -m "Group a lane's PRs by folder and by author"`.

---

### Task 3: Load details when a lane opens

**Files:**
- Modify: `src/main.rs` (State fields + methods), `src/ui/compare.rs` (lane click calls the new method)

**Interfaces:**
- Consumes: `compare::fetch_details`, `Cell::Waiting { prs, .. }`.
- Produces:
  - `pub lane_details: HashMap<(String, Vec<u64>), Load<Vec<compare::PrDetail>>>` on State (session only; not in `cache::Snapshot`).
  - `pub lane_view: LaneView` with `#[derive(Default, Clone, Debug, PartialEq)] pub struct LaneView { pub by_contributor: bool, pub folder: Option<(String, bool)> }`.
  - `pub fn lane_key(&self) -> Option<(String, Vec<u64>)>` — from `compare_sel` and the selected cell's PR numbers (sorted).
  - `pub fn select_lane(&mut self, slug: &str, j: usize)` — toggles `compare_sel` as today's click does; when the lane changes, clears `lane_view.folder`; then `load_lane_details(false)`.
  - `pub fn load_lane_details(&mut self, retry: bool)` — if a key exists and (`retry` or no entry, or entry has neither data nor loading), start `jobs::spawn(fetch_details)`; the result is stored under the key it was started for, whatever lane is selected by then.

- [ ] **Step 1: Write the failing tests** (in `src/main.rs` tests, `State::new(config::Prefs::default())` plus a hand-built `compare` with one repo and two Waiting cells):

```rust
#[test] fn lane_details_are_keyed_by_lane() { /* lane_key for (slug,0) != for (slug,1); inserting a Load for key 0 leaves key 1 absent */ }
#[test] fn switching_lane_clears_folder_filter() { /* lane_view.folder = Some(("src".into(), false)); select (slug,1) after (slug,0) → folder None, by_contributor kept */ }
#[test] fn loaded_lane_is_not_refetched() { /* entry with data for the key → load_lane_details(false) leaves it untouched (not loading) */ }
```

Note: `jobs::spawn` runs inline in tests and would hit the network; tests set `lane_details` entries by hand and only call paths that don't spawn (an existing loaded/loading entry), or check `lane_key` / `lane_view` directly.

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement**; replace the lane `on_press` body in `ui/compare.rs` with `s.select_lane(&slug, j)`. **Step 4: Run** `cargo test` → PASS.
- [ ] **Step 5: Commit** `git commit -m "Load a lane's PR details when it opens"`.

---

### Task 4: The pane: switch, diagram, contributor view

**Files:**
- Modify: `src/ui/compare.rs` (`detail` and new helpers), `src/main.rs` (`compare_fixture` details for one lane), `assets/i18n/th.tsv`

**Interfaces:**
- Consumes: `State.lane_details`, `State.lane_view`, `lane_key`, `load_lane_details(true)` (Retry), `lane::{folders, touches, by_author}`, `State::open_pr`.
- Produces: no new public interface.

Layout (spec "Pane layout"): header (unchanged) → `SegmentedControl` **By PR | By contributor** (sets `lane_view.by_contributor`; a small `Spinner` beside it while loading) → **Changed code** diagram when details are loaded (one stacked bar, segments coloured `hue(i)`, widths = `share`; legend rows `path · NN% · +a −d`; clicking a segment or row sets/clears `lane_view.folder`; active row outlined; `Showing PRs that touched {folder}` + `Clear` when filtered; `other {n} folders` row is not clickable) → list → `Commits without a PR` (unchanged, not filtered).
- PR row additions: label chips (`chip`), summary in muted small text when non-empty, `+a −d` muted; author line kept in By PR, dropped under author headings in By contributor.
- Author heading: name (or login when name empty) · `@login` · company when non-empty · `{n} PRs · +{a} −{d}` · `Profile` link (`open_url` to `https://github.com/{login}`; none for `ghost`).
- Error: `Couldn't load PR details: {error}` muted + `Retry` → `load_lane_details(true)`.

- [ ] **Step 1: Extend the fixture:** in `compare_fixture` insert a `lane_details` entry for the pre-selected lane (acme/api, To staging) with 3 `PrDetail`s across two authors (one with a company, one without) and files in `src/feature/agent`, `src/feature/report`, `docs`. Add scenes `comparisons-by-contributor` (same, `by_contributor = true`) and keep `comparisons`.
- [ ] **Step 2: Implement** the pane per the layout above.
- [ ] **Step 3: Verify:** `cargo run -- --snap <scratch>/lane-pr.png --scene comparisons --size 1400x860` and `... --scene comparisons-by-contributor`, plus `light-comparisons`; the diagram, chips, summary, author headings and Profile link are readable and nothing overlaps at the pane's minimum width. `cargo test` PASS; i18n-check `0 without Thai`.
- [ ] **Step 4: Commit** `git commit -m "Lane pane: by contributor, changed-code diagram, feature info"`.

---

## Final check

- [ ] `cargo build --release` no warnings; `cargo test` all pass; i18n-check clean.
- [ ] Live: open `aom-bet-lottery-admin-fe` To production in a release build; details load in a few seconds; By contributor shows pluemmmmm and others.
- [ ] README: one line under Comparisons about the lane pane's By contributor view and Changed code diagram.
