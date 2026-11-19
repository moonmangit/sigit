# Branch detail and per-item module maps

Date: 2026-11-19 · Status: draft for approval

## Why

The Comparisons lane pane now explains a lane: PRs, contributors with avatars, and the learned
Modules bar. The user wants (1) the same detail when they select a branch on a repo's **Branches**
page, and (2) to see **which modules each PR, each contributor and each commit touched**, not
only the lane as a whole. And it must stay fast ("don't forget to optimize").

## Decided

- Branch detail compares the selected branch with the repo's **default branch** (from the repo's
  metadata), with a picker to compare against another branch instead.
- Built by **treating a selected branch as a one-off comparison** (approach A): it goes through
  the same fetch, PR details, contributors, avatars and learned modules as a Comparisons lane;
  the pane code is shared, so both screens get every feature below.
- Rejected: reading the local clone with `git` (no PRs, profiles or avatars; GitHub-only branches
  would not work); a separate branch pane (duplicates the lane pane, drifts).

## What the user sees

**Branches page (Local / Remote views):** selecting a branch opens a side pane beside the table
(the table keeps its width share as the lane pane does today; Check out stays in the toolbar).
The pane header: branch name, "compared with" + a picker (default branch first, then other
branches of the repo, A–Z), then everything the lane pane shows: commits and PRs waiting, By PR /
By contributor, Modules (collapsed bar, Show details), PR list, commits without a PR. A
local-only branch GitHub doesn't know shows "This branch isn't on GitHub yet" and its ahead count.

**Per-item module maps (lane pane and branch pane):**

```
Modules  ███████████████████████████▓▓▓▓░░  history 59% NEW · agent 35% · docs 4% · shared 2%
● Ploy Srisuk @ploy                       2 PRs · +150 −23
  ██████████████████▓▓                    agent 88% · docs 12%     (contributor map)
  #141 Sample rate limit   ▬▬▬▬▬▬▬▬▬▬▓                            (PR map)
  4be07f1 Sample header logo  ▬▬▬▬▬▬▬                             (commit map, after click)
```

- Every PR row gets a thin bar (4 px) of the modules its files touched, in the **same colour per
  module** as the main Modules bar (colour assigned once per lane from the main bar's order, so
  `agent` is the same colour everywhere). Hover a segment: module name and share.
- Contributor headings (By contributor) get a bar of everything that author changed in the lane.
- Commits without a PR: clicking one fetches its files once (`gh api repos/{slug}/commits/{sha}`,
  cached per sha for the session) and shows its bar under it; a spinner while loading.
- Repos with no learned modules: the bars use the folder groups instead (same colours as the
  folder diagram).

## Optimisation (required)

- **No per-frame recomputation.** Summaries (lane Modules, per-PR, per-author shares, colour map)
  are computed **once** when a lane's details or layout arrive (or change), stored with the lane,
  and read by the view. The view does lookups only. (The lane pane today recomputes `folders` and
  `summarize` every frame.)
- **Lookups by key, not scans:** PR details indexed by number (map) once per lane.
- **Requests only on demand, each once per session:** a branch's comparison is fetched when it's
  selected (never for all branches); repo trees per (repo, branch); avatars per URL; commit files
  per sha; PR details per lane key. Reselecting is instant.
- **Small requests:** the branch comparison is one single-repo query (the lane path already sends
  one request per repo); PR details stay in batches of 20 in parallel.
- **Nothing on the UI thread:** every fetch through `jobs::spawn`; parsing and summarizing happen
  in the job or once on arrival, not in view code.
- Bars are a handful of `el()` segments per row (no canvas, no per-file elements).

## Non-goals

Import analysis; history learning; showing every file; persisting caches across sessions; a
branch detail for the Stashes / Clean up views.

## Constraints

Read-only `gh api`; no panics in view code (`.get()`, no indexing into fetched lists); all new
strings through i18n with Thai; tests for: branch-as-comparison keys and base switching, cached
summaries recomputed only when inputs change, per-PR/per-author shares and colour consistency,
commit-file caching, and the not-on-GitHub case.

## Calls I made

Default branch from metadata with a picker; side pane layout on Branches; 4 px item bars; colour
per module fixed from the main bar; commit files on click only; folder groups as the fallback;
session-only caches.
