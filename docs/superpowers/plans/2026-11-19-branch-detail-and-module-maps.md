# Branch Detail and Module Maps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A detail pane for a selected branch on the Branches page (compared with the default branch, or a picked one), sharing the Comparisons lane pane; per-PR, per-contributor and per-commit module bars in both; summaries computed once, not per frame.

**Architecture:** The pane stops reading `compare_sel` directly and works from an "open lane" (`State::open_lane()`), which is either a Comparisons lane or a branch lane (`BranchLane`, fetched with the existing single-repo compare query). Everything the pane draws from fetched data — modules summary, folders, colour map, per-PR and per-author segments, details by number — is built once into a `LaneModel` when inputs arrive and stored by key; view code only looks things up. Commit files are fetched on click and cached by sha.

**Tech Stack:** Rust 2024, Rune / rune-kit, `gh api` (GraphQL + REST), `crate::jobs`, `crate::i18n`.

**Spec:** `docs/superpowers/specs/2026-11-19-branch-detail-and-module-maps-design.md`

## Global Constraints

- Branch lane base = repo default branch (`RepoMeta.default_branch`), changeable with a picker (default first, then other branches A–Z).
- Every fetch via `jobs::spawn`, once per session per key: branch lane per (slug, from, to); trees per (slug, branch); avatars per URL; commit files per (slug, sha); PR details per lane key. Never fetch for unselected branches.
- No per-frame recomputation: `lane::folders`, `modules::summarize`, segment maps and the details-by-number map are built in `State::rebuild_lane_model` only, triggered when details or a layout land; view code must not call `folders`, `summarize`, or `item_segments`.
- Item bars: 4 px high; a module's colour is the same on every bar in a pane (index assigned from the main Modules/folder bar order).
- View code: no indexing into fetched lists (`.get()` only), never starts a fetch except from click handlers / Retry.
- Copy (exact, Thai in th.tsv): `compared with`, `This branch isn't on GitHub yet`, `{n} commits not pushed`, `Loading files…`, `Couldn't load this commit's files`.
- `cargo test` all pass, `cargo build --release` no warnings, `python scripts/i18n-check.py` → `0 without Thai`. Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Switching between Comparisons and Branches** with a lane open on each must show each page's own lane, never the other's → Task 1 test `open_lane_follows_the_page`.
2. **Changing the base branch** while its fetch is in flight must not show the old base's data under the new one → Task 1 test `branch_lane_keyed_by_base`.
3. **A branch with zero commits ahead** (already merged) shows "up to date", not an empty pane → Task 4 snapshot + Task 1 test `branch_lane_up_to_date`.
4. **Inputs arriving in any order** (details before the layout, or after) must end with the same model, built once per change → Task 2 test `model_rebuilds_on_each_input_once`.
5. **A commit whose files fail to load** shows the error line and can be clicked again to retry → Task 3 test `commit_files_retry_after_error`.

---

### Task 1: Open lane: Comparisons lane or branch lane

**Files:** Modify `src/main.rs` (State, lane methods), `src/compare.rs` (`fetch_branch`), `src/ui/compare.rs` (`detail` reads `open_lane`).

**Interfaces:**
- Produces in `src/compare.rs`: `pub fn fetch_branch(slug: &str, from: &str, to: &str) -> RepoCompare` — `fetch(vec![(slug, vec![Some(Comparison { name: String::new(), from, to })])])` first row, or a `RepoCompare` with `error` when empty.
- Produces in `src/main.rs`:
  - `pub struct OpenLane<'a> { pub slug: &'a str, pub name: String, pub from: String, pub to: String, pub cell: &'a compare::Cell }` (`name` = comparison name, or the branch name for a branch lane).
  - `pub fn open_lane(&self) -> Option<OpenLane<'_>>` — on `Page::Inbox` with `inbox_tab == COMPARE_TAB`: from `compare_sel` (today's logic); on `Page::Branches` (views 0/1): from `branch_lane_sel` and `branch_lanes`.
  - `pub branch_lanes: HashMap<(String, String, String), Load<compare::RepoCompare>>` (slug, from, to), `pub branch_lane_sel: Option<(String, String)>` (slug, branch), `pub branch_base: Option<String>` (None = default branch).
  - `pub fn branch_base_for(&self, slug: &str) -> Option<String>` — `branch_base` or the repo meta default branch.
  - `pub fn select_branch_lane(&mut self, branch: &str)` — sets `branch_lane_sel` (a remote branch `origin/x` → `x`), clears `lane_view.folder`, fetches the lane if absent (`jobs::spawn(fetch_branch)`), then `load_lane_details(false)`.
  - `pub fn set_branch_base(&mut self, base: Option<String>)` — sets it, fetches the new key if absent, `load_lane_details(false)`.
  - `lane_key()` and `lane_branch()` now derive from `open_lane()` (same return types as today).
- The Branches table's `on_select` calls `s.select_branch_lane(name)` (keeps setting `branch_sel` for Check out).

- [ ] **Step 1: Failing tests** in `src/main.rs` tests (state via `State::new(Prefs::default())`, hand-made `compare` and `branch_lanes` entries; never reach `jobs::spawn`: pre-insert `Load::start()` / loaded entries):
```rust
#[test] fn open_lane_follows_the_page() { /* compare_sel on a lane, branch_lane_sel on another; page Inbox+COMPARE_TAB → open_lane().name == comparison name; page Branches → == branch */ }
#[test] fn branch_lane_keyed_by_base() { /* loaded entry for (slug,"feat","main"); set_branch_base(Some("dev")) with (slug,"feat","dev") pre-inserted as Load::start() → open_lane() is None-cell-or-loading for dev, never main's cell */ }
#[test] fn branch_lane_up_to_date() { /* entry with Cell::Waiting{commits:0,..} → open_lane().cell is that cell (pane shows up to date) */ }
#[test] fn remote_branch_name_drops_origin() { /* select_branch_lane("origin/feat/x") with a pre-inserted key → branch_lane_sel.1 == "feat/x" */ }
```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement; `detail()` in `ui/compare.rs` takes its slug/name/from/to/cell from `open_lane()` instead of `compare_sel` + `rows` (Comparisons behaviour unchanged). **Step 4:** `cargo test` → PASS.
- [ ] **Step 5: Commit** `Lane pane: open lane from Comparisons or a branch`.

---

### Task 2: LaneModel, built once

**Files:** Create `src/lane_model.rs` (register `mod lane_model;`); modify `src/main.rs` (store + rebuild calls), `src/ui/compare.rs` (read the model).

**Interfaces:**
- Consumes: `modules::{Layout, summarize, place, Summary}`, `lane::{folders, touches, Folders, FolderShare}`, `compare::PrDetail`.
- Produces in `src/lane_model.rs`:
  - `pub struct Segment { pub key: String /* module path or folder path */, pub label: String, pub color: usize, pub share: f32 }`
  - `pub struct LaneModel { pub details: HashMap<u64, PrDetail>, pub folders: Folders, pub summary: Option<modules::Summary>, pub colors: HashMap<String, usize>, pub per_pr: HashMap<u64, Vec<Segment>>, pub per_author: HashMap<String /* login */, Vec<Segment>> }`
  - `pub fn build(details: &[PrDetail], layout: Option<&Layout>) -> LaneModel` — colours: index by the main bar's order (summary modules then shared rows; or folder rows when no layout).
  - `pub fn segments(model: &LaneModel, layout: Option<&Layout>, files: &[(String, u64, u64)]) -> Vec<Segment>` — bucket each file (module path via `place`, else the first main-bar folder row `touches` would match, else "other"), shares by lines, colour from `model.colors` (unknown keys share one "other" colour); used for PRs, authors (their PRs' files), and commits (Task 3).
- Produces in `src/main.rs`: `pub lane_models: HashMap<(String, Vec<u64>), LaneModel>`; `pub fn rebuild_lane_model(&mut self, key: &(String, Vec<u64>))` called from the details completion and the layout completion (for every lane key of that repo/branch currently loaded); view reads `lane_models.get(&key)`.
- `ui/compare.rs` `detail()` and `modules_view`/`changed_code` read the stored `summary`/`folders`/details map; no calls to `folders`, `summarize` or `segments` remain in view code.

- [ ] **Step 1: Failing tests** in `src/lane_model.rs`:
```rust
#[test] fn colours_follow_the_main_bar() { /* modules history(1st), agent(2nd), shared docs → colors["src/feature/history"]==0, agent==1, docs==2; a PR touching agent+docs → segments colours [1,2] */ }
#[test] fn per_pr_and_per_author_shares() { /* author with two PRs → per_author[login] shares sum to 1.0 and reflect summed lines */ }
#[test] fn folder_fallback_without_layout() { /* layout None → per_pr keys are folder paths, colours from folder order */ }
#[test] fn zero_line_files_give_no_nan() { /* all files 0/0 → shares 0.0, no NaN */ }
```
and in `src/main.rs`:
```rust
#[test] fn model_rebuilds_on_each_input_once() { /* insert details then layout (via the same code paths' completion helpers or by calling rebuild after each) → model.summary is Some after the layout, None before; calling rebuild with unchanged inputs yields an equal model */ }
```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** `cargo test` → PASS; grep `src/ui/compare.rs` shows no `lane::folders(`, `modules::summarize(` or `segments(` calls.
- [ ] **Step 5: Commit** `Lane pane: build summaries once, not every frame`.

---

### Task 3: Commit files on click

**Files:** Modify `src/compare.rs` (fetch), `src/main.rs` (cache + click).

**Interfaces:**
- Produces in `src/compare.rs`: `pub fn fetch_commit_files(slug: &str, sha: &str) -> Result<Vec<(String, u64, u64)>, String>` (`gh api repos/{slug}/commits/{sha} --jq '.files[] | [.filename, .additions, .deletions] | @tsv'`), and `pub fn parse_commit_files(tsv: &str) -> Vec<(String, u64, u64)>`.
- Produces in `src/main.rs`: `pub commit_files: HashMap<(String, String), Load<Vec<(String, u64, u64)>>>`, `pub open_commits: HashSet<String>` (sha), `pub fn wants_commit_fetch(entry: Option<&Load<Vec<(String, u64, u64)>>>) -> bool` (true when absent, or errored and not loading), `pub fn toggle_commit(&mut self, slug: &str, sha: &str)` — opens/closes; on open fetches when `wants_commit_fetch` says so.

- [ ] **Step 1: Failing tests:**
```rust
#[test] fn parse_commit_files_tsv() { /* "src/a.ts\t10\t2\nREADME.md\t1\t0\n" → [("src/a.ts",10,2),("README.md",1,0)]; blank/garbled lines skipped */ }
#[test] fn commit_files_retry_after_error() { /* wants_commit_fetch(None) && wants_commit_fetch(Some(&errored)) && !wants_commit_fetch(Some(&loading)) && !wants_commit_fetch(Some(&loaded)) */ }
#[test] fn toggle_commit_closes_without_fetch() { /* open then toggle again → removed from open_commits, entry unchanged */ }
```
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** `cargo test` → PASS.
- [ ] **Step 5: Commit** `Commits without a PR: load their files on click`.

---

### Task 4: The panes: branch pane, item bars

**Files:** Modify `src/ui/branches.rs` (side pane), `src/ui/compare.rs` (header variant, item bars, commit rows), `src/main.rs` (scenes), `assets/i18n/th.tsv`.

**Interfaces:**
- Consumes everything above. `pub fn item_bar(k: &Kit, segs: &[Segment]) -> El` in `ui/compare.rs` (4 px, `hue(color)` per segment, hover help `label · NN%`).
- Branches page (views 0/1): table and pane side by side via `super::split::split` like the Comparisons tab (pane only while `branch_lane_sel` is set); pane header shows branch name, `compared with` + a picker (`SegmentedControl` or the kit's select — default branch first, then local/remote branch names without `origin/`, A–Z, deduplicated) → `set_branch_base`; close button clears `branch_lane_sel`. Lane `Missing([from])` → `This branch isn't on GitHub yet` + `{n} commits not pushed` (from the local `Branch.ahead`).
- Item bars: under each PR row (`per_pr`), under each author heading (`per_author`), under an opened loose commit (`segments` over its files, `Loading files…` / `Couldn't load this commit's files` + click to retry). Loose commit rows become clickable (`toggle_commit`).
- Scenes: `branches-detail` (Branches page, a selected branch with a fixture branch lane, details and layout reused from `compare_fixture`), `light-branches-detail`; extend `comparisons-by-contributor` with one opened loose commit (fixture `commit_files` entry).

- [ ] **Step 1:** Add the scenes; run `cargo run -- --snap <png> --scene branches-detail --size 1400x860` → fails (no pane).
- [ ] **Step 2:** Implement.
- [ ] **Step 3: Verify:** snapshots `branches-detail`, `light-branches-detail`, `comparisons-by-contributor`, `comparisons` — bars visible and colour-consistent with the main bar; nothing overlaps at the pane's minimum width. `cargo test`, release build, i18n-check clean.
- [ ] **Step 4: Commit** `Branch detail pane and per-item module bars`.

---

## Final check

- [ ] Live: on a real tracked repo, open a lane and a branch; both load; closing and reopening each shows its data at once with no spinner.
- [ ] README: a line under Branches about the detail pane; mention the per-item bars under Comparisons.
