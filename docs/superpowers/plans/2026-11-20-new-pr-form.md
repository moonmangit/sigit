# New PR Form Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the PR-routes feature with a plain "New pull request" form (any branch → any branch) plus helpers.

**Architecture:** First strip the route model (settings, steps, keep-branch rule) and keep the reusable pieces: `PrSpec`, `create_pr_cmd`, `Action::CreatePr`, `branch_prs`, `pushed_state`, and the draft-survives logic. Then add the form's state and pure helper logic on `State`, then the dialog UI and its two entry points.

**Tech Stack:** Rust, the in-repo `rune` / `rune_kit` UI crates, the `gh` CLI, serde.

**Spec:** `docs/superpowers/specs/2026-11-20-new-pr-form-design.md`

## Global Constraints

- Every UI string goes through `crate::i18n::t` / `tf!`. `python scripts/i18n-check.py` must print `0 without Thai`. Add Thai rows to `assets/i18n/th.tsv` (tab-separated, CRLF) and remove the rows for deleted strings.
- What's shown equals what runs: commands are built only as `gh::Cmd`.
- Every PR is opened through `State::request(Action::CreatePr …)` and the guard's confirm dialog. The form never runs anything itself.
- Old config files load: `Prefs` keeps `#[serde(default)]`, and the removed `pr_route` / `pr_route_overrides` keys must not break loading.
- Tests are network-free. Jobs run inline in tests, and fetches must return an offline error under `cfg!(test)`, the same as `reload_route_prs` does now.
- Use `cargo check` / `cargo test`, not `cargo build`. Don't launch the app.
- Remote branches are `origin/<name>`. Show and compare by the name without `origin/`. Names may contain slashes.

## Review Focus

1. **Old prefs with `pr_route` keys.** A file written by the PR-routes build must load, with the removed fields ignored. Test in Task 1 (`prefs_with_removed_route_fields_load`).
2. **From == To, or 0 commits ahead.** Create PR must be disabled with the stated reason. Test in Task 2 (`create_blocked_reasons`).
3. **Comparison or open-PR check still loading or failed.** It must not block forever: while loading, only a blank title, a known-unpushed branch, or From == To block. Test in Task 2 (`create_blocked_reasons`).
4. **Changing From or To while a comparison is in flight.** A result for the old pair must not land on the new pair. Results are keyed by `(slug, from, to)`. Test in Task 2 (`comparison_keyed_by_pair`).
5. **Remembered values for a repo whose remembered base no longer exists.** Fall back to the default branch. Test in Task 2 (`start_values_fall_back`).

---

### Task 1: Remove the route model

**Files:**
- Modify: `src/route.rs` (keep `PrSpec`, `BranchPr` with `merged_at`/`null_as_empty`, `split_list`, `head_name`; delete `RouteTarget`, `RepoRoute`, `route_for`, `RouteRow`, `rows_of`, `targets_of`, `validate_route`, `fill`, `title_for`, `kept_for`, `merge_deletes_branch`, `StepState`, `Step`, `steps`, `can_open`, `blocked_by`, and their tests)
- Modify: `src/config.rs` (drop `pr_route`, `pr_route_overrides`), `src/guard.rs` (`Action::CreatePr { spec }` without `after_unmerged`; `create_pr_risk(base, prod_names)`), `src/ui/prs.rs` and the `main.rs` fixtures (`delete_branch: true` again; drop `fixture_deletes`), `src/ui/overlays.rs` (drop the "comes after … on this route" warning), `src/ui/settings.rs` and `src/settings.rs` (drop the PR route section, `route_rows`, `save_route`, `open_route_edit`, `save_route_edit`, `drop_route_edit`), `src/ui/route.rs` (drop `route_section`, `step_row`, `route_editor`, `route_edit_dialog`; keep `pr_dialog` and `field` for Task 3), `src/ui/compare.rs` (`branch_pane` no longer shows a route section), `src/ui/mod.rs` (drop the `route_edit_dialog` call), `src/main.rs` (drop `route_edit`, `route_rows`, `shows_route`, the route fixtures and route tests; keep `route_prs` + `reload_route_prs` only if Task 2 reuses them, otherwise delete), `README.md` (drop the PR routes paragraph), `assets/i18n/th.tsv`.

**Interfaces:**
- Produces: `Action::CreatePr { spec: route::PrSpec }`; `pub(crate) fn create_pr_risk(base: &str, prod_names: &[String]) -> Risk` (Low, or Medium for a production branch name). The `restore_pr_draft` / `last_pr_draft` behaviour on failure and cancel stays.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn prefs_with_removed_route_fields_load() // serde_json::from_str::<Prefs>(r#"{"pr_route":[{"base":"staging"}],"pr_route_overrides":[]}"#) is Ok and equals Prefs::default() on every other field
#[test] fn merge_defaults_to_deleting_branch() // the prs.rs Merge builder (extract a tiny fn if needed) yields delete_branch == true
#[test] fn create_pr_risk_levels() // updated: ("staging", default) -> Low; ("production", default) -> Medium
```

- [ ] **Step 2: Run each with `cargo test <name>` and see the new ones fail** (or fail to compile).
- [ ] **Step 3: Delete and adjust the code listed above.** `cargo check` guides the rest. First check that `Prefs` has no `deny_unknown_fields`.
- [ ] **Step 4: Run `cargo test` (all pass), `cargo check` (clean), and the i18n check (`0 without Thai`).**
- [ ] **Step 5: Commit** `refactor: drop PR routes, keep the PR plumbing`

---

### Task 2: Form state and helpers

**Files:**
- Modify: `src/main.rs` (`PrDraft` → the form state; State methods), `src/config.rs` (`Prefs.pr_last`), `src/route.rs` (pure helpers, if they read better there; renaming the module to `pr_new` is allowed)
- Test: `src/main.rs` test module and the `src/route.rs` tests

**Interfaces:**
- Consumes: `compare::fetch_branch(slug, from, to) -> RepoCompare` (its first cell `Cell::Waiting { commits, prs: Vec<WaitingPr{number,title,..}>, loose: Vec<LooseCommit{subject,..}>, .. }` or `Cell::Missing`), `gh::branch_prs`, `pushed_state(slug, head) -> Option<bool>`, `Action::CreatePr`.
- Produces:
  - `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)] #[serde(default)] pub struct PrLast { pub slug: String, pub base: String, pub draft: bool, pub reviewers: String, pub assignees: String, pub labels: String }` and `Prefs.pr_last: Vec<PrLast>`.
  - `PrDraft { slug, from: String, to: String, draft, title, body, reviewers, assignees, labels: TextEdit … }`. `from` and `to` are plain branch names without `origin/`.
  - `pub fn open_new_pr(&mut self, slug: &str, from: Option<&str>)` sets the starting values. From is the `from` argument, else the clone's current branch (`Branch.current`), else "". To is `pr_last.base` if that branch exists in the repo's branch list, else the repo's default branch. The other fields come from `pr_last`, else blank (draft false). It starts the comparison and the open-PR check.
  - `pub fn set_pr_from(&mut self, b: &str)` / `set_pr_to(&mut self, b: &str)` update the pair and start its loads.
  - `State.pr_compare: HashMap<(String, String, String), Load<RepoCompare>>` keyed by (slug, from, to). It reuses `route_prs` (or a renamed one) keyed by (slug, from) for the open-PR check. Both fetches return an offline Err under `cfg!(test)`.
  - `pub fn pr_ahead(&self) -> Option<u32>`: the current pair's commits ahead, None while loading or on error.
  - `pub fn pr_already_open(&self) -> Option<u64>`: an OPEN `BranchPr` from `from` into `to`.
  - `pub fn fill_pr_title(&mut self)` sets the title to From's `Branch.subject` (local, else `origin/<from>`). `pub fn fill_pr_body(&mut self)` sets the body to `- <title>` for each WaitingPr, then `- <subject>` for each LooseCommit, joined with `\n`.
  - `pub fn pr_blocked(&self) -> Option<String>`: the first reason, in this order. "Pick a branch to merge from" when From is empty. "From and To are the same branch". "Push {from} to GitHub first" when `pushed_state == Some(false)`. "Nothing to merge: {from} has no commits that {to} doesn't" when `pr_ahead == Some(0)`. "A PR from {from} into {to} is already open: #{n}". "Add a title" when the trimmed title is empty. Otherwise None. A loading or failed comparison or open-PR check doesn't block.
  - `submit_pr_draft` refuses when `pr_blocked()` is Some. On success, `after(CreatePr)` upserts `PrLast` for the slug and saves prefs.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn start_values_from_selection_and_default() // open_new_pr(slug, Some("feat-a")) with default branch "main", no pr_last -> from "feat-a", to "main", draft false
#[test] fn start_values_use_remembered() // pr_last {base "staging", draft true, labels "testing"} and branch list containing staging -> to "staging", draft true, labels "testing"
#[test] fn start_values_fall_back() // pr_last base "gone" not in the branch list -> to = default branch
#[test] fn start_values_current_branch() // open_new_pr(slug, None) with the current branch "wip/x" -> from "wip/x"
#[test] fn comparison_keyed_by_pair() // insert a finished Load for (slug,"feat-a","main") with commits 3; set_pr_to("staging") -> pr_ahead() is None (not 3)
#[test] fn create_blocked_reasons() // each reason in order: empty from; from == to; known unpushed; ahead Some(0); an open PR into `to` (-> contains "#142"); blank title; all fine -> None; comparison loading (no entry) -> None
#[test] fn fill_title_and_body() // From subject "Add login" -> title "Add login"; Waiting{prs:[#5 "Login form"], loose:[{subject:"Fix typo"}]} -> body "- Login form\n- Fix typo"
#[test] fn create_remembers_repo_values() // after a successful CreatePr, prefs.pr_last for the slug has base "staging", draft true, reviewers "lead"
```

- [ ] **Step 2: Run each with `cargo test <name>` and see them fail.**
- [ ] **Step 3: Implement.** Follow `load_branch_lane` / `reload_route_prs` for the loads.
- [ ] **Step 4: `cargo test` passes and `cargo check` is clean.**
- [ ] **Step 5: Commit** `feat(prs): new-PR form state and helpers`

---

### Task 3: The form UI and its entry points

**Files:**
- Modify: `src/ui/route.rs` (or `src/ui/pr_new.rs`): `pr_dialog` becomes the form. `src/ui/prs.rs`: a "New pull request" button in the toolbar (`pr-tools`, ~line 69). `src/ui/branches.rs`: an "Open PR" button next to "Check out" for the selected branch (~line 55). `src/main.rs`: fixtures. `assets/i18n/th.tsv`.

**Interfaces:**
- Consumes: everything Task 2 produces.
- Produces, per the spec sketch:
  - The title is `tf!("New pull request")` with the slug beside it.
  - From and To are `Select` dropdowns (see how `rune_kit::Select` is used in `src/ui/compare.rs`) over the repo's branch names, local and remote, de-duplicated, without `origin/`. They call `set_pr_from` / `set_pr_to`.
  - The comparison line shows `tf!("{n} commits ahead")`, a spinner while loading, or nothing on error.
  - Ready/Draft control.
  - The Title input has a "Use last commit" button (`fill_pr_title`).
  - The Body `TextArea` has a "List commits" button (`fill_pr_body`). It is disabled until the comparison has loaded.
  - Reviewers, Assignees and Labels inputs.
  - The `pr_blocked()` reason is shown above the buttons in the warning tone. When it's the already-open reason, clicking it calls `s.open_pr(slug, n)`.
  - Cancel / Create PR. Create PR is disabled when `pr_blocked()` is Some.
  - The PRs-page button calls `open_new_pr(cur_slug, None)`. The Branches button calls `open_new_pr(slug, Some(selected name without origin/))`.
  - Fixtures: `"new-pr"` (ready to create, 3 commits ahead), `"new-pr-blocked"` (already open), `"new-pr-unpushed"`.

- [ ] **Step 1: Write a failing test**

```rust
#[test] fn entry_points_open_the_form() // the PRs-page action opens pr_draft for the current repo with from = current branch; the Branches action with a selected "origin/feat/x" opens it with from == "feat/x"
```

- [ ] **Step 2: Run it and see it fail.**
- [ ] **Step 3: Implement the UI, buttons, fixtures, and Thai rows.**
- [ ] **Step 4: `cargo test` passes, `cargo check` is clean, and the i18n check prints `0 without Thai`.**
- [ ] **Step 5: Commit** `feat(prs): New pull request form with helpers`

---

### Task 4: Docs

**Files:**
- Modify: `README.md` (where the PR routes paragraph was, next to Comparisons)

- [ ] **Step 1: Write one short paragraph.** It names the New pull request button on the Pull requests page and Open PR on the Branches page, any branch into any branch, the helpers (commits ahead, fill buttons, already-open and push checks, remembered per repo), and the confirm step.
- [ ] **Step 2: Commit** `docs: New pull request form`
