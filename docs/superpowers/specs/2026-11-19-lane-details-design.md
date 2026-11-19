# Lane details: features, contributors and the code they touch

Date: 2026-11-19 · Status: draft for approval

## Why

On Dashboard → Comparisons, clicking a lane (say `staging → main` on one repo) opens a pane that
lists the merged PRs waiting and the commits that came from no PR. The user wants to know **which
features** are in a lane and **who wrote them**, because their work spans several teams and
organisations and they need to know whom to contact. They want to group by contributor when
they choose to, and to see which parts of the code the lane changes as a diagram. Everything the
pane does today stays.

## Goals

- Each PR in the pane shows what the feature is: title, labels, the first line of its
  description, lines added/removed, author, merged ago (today's fields stay).
- A switch at the top of the pane: **By PR** (today's list, default) / **By contributor**.
- By contributor: one heading per PR author with name, @login, company (from their GitHub
  profile), PR count and lines added/removed in this lane, and a link to their GitHub profile;
  their PRs listed under it, newest merge first. Authors ordered by PR count, then lines.
- A folder diagram of the code the lane changes, sized by lines changed (added + removed).
  Clicking a folder filters the list (either view) to PRs that touched it; clicking it again
  clears the filter.
- Opening a lane shows today's list at once; details fill in when they arrive and are kept for
  the session, so reopening is instant.

## Non-goals

- Reviewers/approvers as participants (decided: PR authors only).
- File data for commits that came from no PR: they stay in their own list, without files, and
  aren't in the diagram.
- Showing individual files or diffs (folders only; the PR itself is one click away).
- Loading details for lanes that aren't opened.

## Anti-goals

- The Comparisons tab getting slower: nothing new is fetched until a lane is opened.
- Hitting GitHub's request time limit again (the lesson from the 502s): requests stay small.
- A pane that's empty or broken while details load or if they fail: today's list always shows.

## Constraints

- Read-only `gh api graphql`, run with `jobs::spawn`; never blocks the window.
- Strings through `crate::i18n` with Thai in `assets/i18n/th.tsv`; `scripts/i18n-check.py` clean.
- View code must not panic on missing data (use `.get()`, no indexing on fetched lists).

## Approaches

| | How | Trade-off | |
|---|---|---|---|
| **A** | When a lane opens, one GraphQL query for its PRs by number (batches of 20): labels, body, author name/login/company, additions/deletions, files (paths + lines) | Works for any lane size (per PR, no compare-API caps); has everything the contributor view needs. Loose commits get no file data | **Chosen** |
| B | GitHub's REST compare for the lane | One request with files and commit authors, but capped at 250 commits / 300 files (`aombet-ag-fe staging → dev` has 1,097 commits) and no labels, descriptions or company | |
| C | Fetch details with the comparisons themselves | Makes the whole tab wait for data only one lane needs | Dropped |

## Design

### Data (src/compare.rs)

- `pub struct PrDetail { number: u64, labels: Vec<String>, summary: String, additions: u64,
  deletions: u64, author: Person, files: Vec<(String, u64, u64)> /* path, added, removed */ }`
- `pub struct Person { login: String, name: String, company: String }` (empty strings when the
  profile has none; bots and deleted users: login as given, `name` empty).
- `pub fn details_query(slug: &str, numbers: &[u64]) -> String`: one `repository(owner, name)`
  with an alias `p{i}: pullRequest(number: N) { labels(first: 10){nodes{name}} bodyText additions
  deletions author { login ... on User { name company } } files(first: 100) { nodes { path
  additions deletions } } }` per PR.
- `pub fn parse_details(json) -> Vec<PrDetail>`; a PR GitHub returns null for is skipped, others kept.
- `pub fn fetch_details(slug, numbers: Vec<u64>) -> Result<Vec<PrDetail>, String>`: batches of
  **20** PRs per request, run in parallel like `fetch`; a failed batch is an error for the whole
  call only if every batch failed, otherwise the PRs that loaded are returned.
- `summary` = the first non-empty line of `bodyText`, trimmed, at most 140 characters.
- PRs with more than 100 files: files beyond 100 aren't counted in the diagram; the PR's own
  additions/deletions are still exact. *(my call)*

### State (src/main.rs)

- `lane_details: HashMap<(String, Vec<u64>), Load<Vec<PrDetail>>>` keyed by repo slug and the
  lane's PR numbers (so a lane whose PRs change refetches). Kept for the session; not saved to
  the cache file. *(my call)*
- `lane_view: LaneView { by_contributor: bool, folder: Option<String> }`; switching lanes clears
  `folder`, keeps `by_contributor`. *(my call: the grouping choice sticks while you browse)*
- Selecting a lane (`compare_sel`) starts the fetch if that key isn't loaded or loading.

### Folder diagram

- Group every PR file by folder: drop the folder prefix shared by all files (segment by segment),
  then take the next path segment. Files directly in that shared folder group as "(root)". (A
  single group can only happen when every file sits in one folder, so no deeper step is needed.)
- A group holding 75% or more of the lines is replaced by its subfolders, repeatedly, so a lane
  that touches `src/` and `docs/` shows `src/feature/agent`, `src/feature/report`, … and `docs`
  instead of "src 96%". Files directly in a split folder show as `src/`.
- Show the 8 biggest groups by lines changed, the rest as "other N folders".
- Drawn as one horizontal stacked bar (each folder a segment, width = its share of lines
  changed) with a legend below: colour swatch, folder path, share %, +added −removed. Clicking a
  segment or legend row sets the filter; the active one is outlined and shows a "Clear" link.
  *(bar + legend is my call: it fits the pane's width; a treemap was considered and needs more
  room)*

### Pane layout (src/ui/compare.rs `detail`)

Top to bottom: today's header (name, repo, from → to, commit count, the "PRs counted from…"
note) → **By PR | By contributor** switch → folder diagram (when details are loaded) → list.

- By PR: today's rows, plus labels as small chips, the summary line in muted text, and
  `+added −removed`.
- By contributor: author headings as described in Goals, then the same PR rows without the
  author line.
- While details load: today's rows, a small spinner next to the switch, no diagram.
- Details failed: today's rows and a one-line muted error with a Retry link.
- "Commits without a PR" stays last, unchanged, in both views and unaffected by the folder
  filter.

### Copy (exact; Thai lines added)

`By PR`, `By contributor`, `{n} PRs · +{a} −{d}`, `Profile`, `(root)`, `other {n} folders`,
`Clear`, `Changed code`, `Couldn't load PR details: {error}`, `Retry`, `Showing PRs that touched {folder}`.

## Testing

- Unit: `details_query` (aliases, escaping of owner/name), `parse_details` (labels, summary first
  line + 140 cap, missing company/name, null PR skipped, bot author), folder grouping (shared
  prefix dropped, single-group descends, "(root)", top 8 + other), contributor grouping order
  and totals, folder filter on both views.
- Snapshot: extend the `comparisons` scene fixture with details for one lane, and snapshot both
  views in dark and light.
- Live (ignored test): `fetch_details` for the 7 PRs on `aom-bet-lottery-admin-fe staging → main`.

## Left to the builder

Exact widths, colours of folder segments (use `hue(i)`), the profile link's look, batch-request
parallelism mechanics.
