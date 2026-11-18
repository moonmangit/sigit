# Dashboard, branch comparisons and clone-folder scanning

Date: 2026-11-19 · Status: draft for approval

Three separate changes, to be built in this order:

1. Rename the **Inbox** page to **Dashboard**.
2. **Clone folders:** sigit finds your local clones by itself.
3. **Branch comparisons:** for every tracked repo, how much is waiting between branches you name
   (for example `dev → staging`), in commits and PRs.

## Why

- Commits and Branches need a local clone, and today each clone is located by hand, one at a
  time ("Locate local clone…"). Only 4 of ~240 repos are set up. The user wants sigit to find
  them: "instead of me finding them myself".
- The user's team works `dev` → `staging` → `main` (production), with GitHub environments
  `staging` and `production` deploying from those branches. They want one place that shows, for
  every repo they follow, what's sitting on one branch and not yet on the next, and what never
  came back (hotfixes on staging missing from dev). The comparisons are the user's own: named,
  with a direction, and adjustable per repo.
- "Dashboard" describes the page better than "Inbox" now that it opens on Activity.

## Goals

- Rename: every place the page's name shows says Dashboard (English) / แดชบอร์ด (Thai).
- Scan: pick a folder once; its clones are added now, and new ones at every launch and on
  Rescan, with no further clicks.
- Comparisons: one table on the Dashboard, tracked repos × your comparisons, each cell showing
  commits and PRs waiting; click a cell to see which PRs and commits.
- Fast: comparisons come from one GitHub request for all repos; scanning never blocks the
  window.

## Non-goals

- No promoting from the table (no "create PR dev → staging", no merge). Read-only for now.
- Comparisons don't use GitHub deployments/environments; branches only. (The group dashboard's
  existing "commits not on production" from deployments stays as it is.)
- A repo can't have comparisons of its own (decided: "change shared only").
- No cloning: the scan only finds clones that already exist.
- No local `git` for comparisons (it would need every tracked repo cloned and fetched).

## Anti-goals (a failure even if it works)

- The scan or comparisons writing anything into the user's clones (no `fetch`, no config
  changes). sigit's rule stays: no commit, no push, nothing written to a repo.
- A Dashboard that waits on the network or freezes while scanning.
- Counting squash-merged PRs as zero (see "PRs, not merge commits").

## Constraints

- Everything goes through `gh` / `git`, as the rest of the app (`gh api graphql`).
- Local settings only: new settings live in `prefs.json` (`config::Prefs`, `#[serde(default)]`,
  so old files load unchanged).
- Every visible string goes through `crate::i18n` and gets a Thai line in
  `assets/i18n/th.tsv`; `python scripts/i18n-check.py` must pass.
- Background work uses `jobs::spawn` / `jobs::spawn_each`; data uses `Load<T>` with a `ttl`
  constant, like every other page.

## Approaches considered (comparison data)

| | How | Trade-off | |
|---|---|---|---|
| **A** | One batched GraphQL query: per tracked repo an alias, per comparison `ref(qualifiedName: "refs/heads/<target>") { compare(headRef: "<source>") { aheadBy commits {…} } }` | One request for everything; tested on the user's repos: cost 1 point of the rate limit; a missing branch comes back `null`. Needs paging when >100 commits are waiting | **Chosen** |
| B | One REST `compare` per repo and comparison (like `gh::ahead_by` for deploy drift) | ~30 requests for 10 repos × 3 comparisons; slower, simpler per call | |
| C | Local `git rev-list --count` | Needs every tracked repo cloned and fetched; fetching writes into clones | Dropped |

Folder scan and rename have one sensible way each (below).

---

## 1. Rename Inbox → Dashboard

- User-facing: sidebar row, toolbar title, command palette entry, `Ctrl+1` shortcut label
  (Settings → Shortcuts and the shortcuts line in the guide), the guide's paragraphs that say
  "Inbox", README, `docs/design-notes.md`, the error screen's "Go to Inbox" button → "Go to
  Dashboard".
- Thai: แดชบอร์ด, replacing กล่องงาน in every catalog line that names the page. Remove the
  catalog lines for English strings that no longer exist.
- The palette still finds the page when you type "inbox" (search alias), so muscle memory
  works. *(builder's call how, e.g. a keyword on the palette item)*
- Internal names (`Page::Inbox`, `inbox_tab`, `ui/inbox.rs`, `I_INBOX`) may be renamed or kept;
  builder's call. The "Needs you" tab keeps its name and contents.
- Sidebar icon: a dashboard-style glyph if it reads better than the tray; builder's call.

## 2. Clone folders (scan)

**Settings:** Settings → Git gets a "Clone folders" list: each folder with Remove, plus "Add
folder…" (OS folder picker, `tx.open_folder` as `ask_local_repo` does) and "Rescan now". Stored
as `Prefs.clone_folders: Vec<PathBuf>`.

**When it scans:** right after a folder is added; at launch, after `repos.json` is loaded; on
Rescan now. Always in the background (`jobs::spawn`).

**What it does:**
- Walks each folder up to **5 levels** deep. Skips hidden folders (starting with `.`) and
  `node_modules`, `target`, `vendor`, `dist`, `build`. Doesn't look inside a git repo once found
  (a repo's subfolders aren't scanned), but a folder holding several repos is.
- A folder is a clone when it contains `.git` (a folder, or a file for worktrees/submodules).
- Its repo comes from the `origin` remote via the existing `git::slug_of`. No `origin`, or not a
  GitHub URL → skipped.
- Each found clone goes through the same path as "Locate local clone…" (`add_repo` with a path,
  saved to `repos.json`), so Commits and Branches work right away. A repo already saved keeps
  its saved path. If one repo is found twice, the shallower folder wins.
- Repos on GitHub that aren't in your sidebar list are still added (Commits/Branches only need
  the clone). *(my call)*
- Saved clones whose folder is gone are already dropped at launch (`config::load` checks
  `path.exists()`); nothing new needed.

**Feedback:** a toast after a scan you started (add folder / Rescan now): "Found 14 clones · 12
new · 2 already known · 3 skipped". The launch scan is silent unless it adds something
("Added 2 new clones from your folders"). Folders that can't be read show in the toast as
skipped, never as an error dialog.

**Speed:** the walk is filesystem-only; `git remote get-url` runs only on folders that have
`.git`. Cap the walk at ~20,000 directories per folder so a mistaken pick (`C:\`) ends quickly
(toast says it stopped early). *(cap is my call; adjust if real folders are bigger)*

## 3. Branch comparisons

### Settings

Settings → Git gets "Branch comparisons", a list you can add to, edit, reorder and remove:

| Field | Meaning |
|---|---|
| Name | your label, shown as the column title ("To staging") |
| From | the branch whose extra commits are counted |
| To | the branch they're missing from |

Shown as `From → To`, with a swap button to flip the direction. The user's notation maps
directly: `dev → staging` is From dev, To staging; `dev ← staging` is From staging, To dev.

Stored as `Prefs.comparisons: Vec<Comparison { name, from, to }>`. **Defaults** (the user's own
examples): To staging `dev → staging`, Back-merge `dev ← staging`, To production
`staging → main`. *(defaults are my call)*

Branch names are taken literally: `main` means a branch called `main`. A repo whose production
branch is `master` changes it per repo (below). *(my call; no "default branch" placeholder)*

### Per repo (change shared only)

A repo can, for any shared comparison: use different branches (From and/or To), or turn it off.
It can't add comparisons. Stored as `Prefs.comparison_overrides: Vec<Override { slug, name,
from: Option<String>, to: Option<String>, off: bool }>`, matched by comparison name. Renaming a
shared comparison carries its overrides along; deleting it deletes them.

Edited from the Dashboard table: each repo row has a menu → "Edit comparisons for this repo…",
a dialog with one line per shared comparison (From / To fields prefilled with the shared
values, an On switch) and "Reset to shared". A changed cell in the table has a small mark with
a tooltip saying it's changed for this repo.

### The table

A third Dashboard tab, **Comparisons** (after Activity and Needs you). *(tab name and
placement are my call)*

Sketch (numbers illustrative):

```
Comparisons                To staging          Back-merge          To production
                           dev → staging       dev ← staging       staging → main
aom-bet-lottery-admin-fe   0                   149 commits · 39 PRs  7 commits · 7 PRs
aombet-ag-fe               8 commits · 3 PRs   2 commits · 0 PRs   0
sbo99-member-fe            — no dev            — no dev            1 commit · 1 PR
```

- Rows: `State::tracked()` (your tracked list, or pinned + groups when it's empty), A–Z.
  Empty → a line pointing to "Manage tracked".
- Cells: "N commits · M PRs"; 0 shows as a quiet "up to date"; a missing branch shows "—" with
  "no branch dev" (both branches named if both are missing); a repo GitHub won't let you read
  shows its error in the row.
- Column header: name, and the shared `from → to` underneath.
- Click a non-empty cell → a side pane listing the merged PRs waiting (number, title, author,
  merged ago; click opens the PR in sigit) and then the commits that came from no PR (short
  SHA, subject, author). Same pane style as the PR and run detail panes.
- "Updated Xm ago" chip and refresh like other pages: `ttl::GH` (120 s), re-checked on opening
  the tab and on coming back to the window.

### PRs, not merge commits

The user asked for commits and merge commits. Their team **squash-merges**: on
`aom-bet-lottery-admin-fe`, `staging → main` has 7 commits waiting, each one a merged PR
(#116–#122), and none a merge commit (each has one parent). A merge-commit count would say 0.
So the second number is **PRs**: distinct merged pull requests the waiting commits belong to
(`commit.associatedPullRequests`), which counts squash and merge-commit PRs alike. Proposed in
the conversation; the user didn't object. *(flagged for confirmation)*

When a commit belongs to several PRs (a feature PR into dev, then a release PR dev → staging),
count the merged PR whose base is the comparison's **From** branch; failing that, the earliest
merged one. Open PRs never count.

### Fetching

- One `gh api graphql` query for all rows × comparisons (aliases `r0…`, `c0…`), with branch
  names passed safely (GraphQL variables or escaped strings; branch names can contain `/`).
- Per comparison: `aheadBy` (exact count, shown as "commits") and the first 100 commits with
  `associatedPullRequests(first: 5) { number title author mergedAt baseRefName state }`.
- More than 100 waiting: follow-up pages for that pair only, up to **500 commits**. Beyond that
  the PR count shows "500+ commits checked" in its tooltip and counts what was read.
  *(limit is my call)*
- GitHub's query size limits: split into several queries if needed (builder's call where, for
  example 10 repos per query).
- Results cached in the on-disk snapshot like other Load data, so the tab has numbers at launch
  (builder's call; follow how `cache.rs` stores pulse/inbox).

## Testing

- Unit: turning the GraphQL response into counts (aheadBy, PR dedupe, the several-PRs rule,
  missing branch → "—"); applying overrides (changed branches, off, rename/delete of a shared
  comparison); the scan walk on a temp folder tree (nested repos, skipped folders, `.git` file,
  depth limit, a duplicate slug).
- i18n-check passes; existing tests pass.
- By hand: scan `Desktop\Project` and see the 4 known clones kept and others added; the
  comparisons tab on the user's 10 tracked repos matches GitHub's compare page for two pairs.

## Left to the builder

Internal names after the rename; the Dashboard icon; how the palette alias works; query
batching size; caching shape; exact layout and widths of the table and dialogs; where in
Settings → Git the two lists sit.
