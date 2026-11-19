# Learned modules: what a lane changes, by module

Date: 2026-11-19 · Status: draft for approval

## Why

The lane pane's "Changed code" diagram groups files by folder path. The user wants it to describe
the lane by **module** ("report: new page — 2 views, 2 services, route, 2 locales") and to work
that out **from each repo itself**, with no folder names hard-coded in the app.

## Approach (chosen: A, learn from the folder tree)

| | How | |
|---|---|---|
| **A** | Read the repo's file tree; folders whose subfolders look alike are module containers; the names those subfolders share are the module's parts | **Chosen** |
| B | Group files that change together across past PRs | Later, maybe: needs hundreds of PRs per repo, unnamed fuzzy groups |
| C | Follow imports in the code | Out of scope: language-specific, needs the code |

"Learning" = inferring each repo's structure from its own data, redone when the repo changes. No
model, no trained data, no list of names in the app.

## What it learns (per repo, from the lane's From branch)

- **Tree:** one `gh api repos/{slug}/git/trees/{from}?recursive=1` request (files only). Kept for
  the session per repo + branch; refetched when the lane's details refetch. If GitHub marks the
  tree `truncated` (very large repos), use what came back.
- **Module containers:** a folder with at least **3** subfolders where those subfolders look
  alike: the names of their direct entries (subfolder names, and file names without extension)
  are counted across siblings; a name present in at least **half** of the siblings is a
  *shared part*. The folder is a container when at least **2** shared parts exist and at least
  **60%** of its subfolders contain 2 or more of them, **and** at least one shared part is a
  subfolder (modules have inner structure; asset folders of look-alike files, such as
  `public/images/*` holding the same numbered images, don't count). Each subfolder of a container is a
  **module**; the shared parts are its **parts** (e.g. `view`, `service`, `locale`, `route`).
- Containers are found at any depth; a module inside another module's container belongs to the
  deepest container. Files outside every module are **shared code**, grouped by their folder
  (the existing folder grouping, including the dominant-folder split).
- If a repo has no container, the pane shows today's folder diagram unchanged.

Checked on the real trees: in both aom-bet-lottery-admin-fe and aombet-ag-fe the only container
found is `src/feature` (12 modules; parts `view service locale route navigation`, plus
`component lib store` in aombet-ag-fe). Without the subfolder rule `public/images` was a false
positive. Example: `src/feature` has 12 subfolders that mostly contain
`view`, `service`, `locale`, `route`, `navigation` → container; `src/shared/component` children
don't share structure → shared code.

## What it shows

Needs each changed file's kind of change: the PR details query adds `changeType` per file
(ADDED / MODIFIED / DELETED / RENAMED / COPIED / CHANGED).

Per lane, a **Modules** section replaces "Changed code" when modules were learned:

```
Modules
report     NEW   ███████████  2 views · 2 services · route · navigation · 2 locales   #121 #122
monitor    NEW   ████████     1 view · 3 services · route                              #120
agent            ███          1 view · 1 service                                       #117 #118
Shared code      █            src/shared/component (1 file)                            #116
```

- One row per touched module: name, **NEW** when every changed file of the module in this lane
  was added (otherwise no badge; **REMOVED** when every file was deleted), a bar sized by lines
  changed (same scale for all rows), the parts touched with file counts (singular/plural, parts
  in the order learned), files in no learned part counted as "other", and the PR numbers.
- Then one "Shared code" row per shared folder group (same look, no badge, folder path as name).
- Sorted by lines changed, largest first; at most 8 module rows + 4 shared rows, the rest as
  "N more".
- Clicking a row filters the PR list to PRs that touched that module (or shared folder), like
  the folder filter today; Clear removes it.
- While the tree loads: the folder diagram shows; the Modules section replaces it when ready.
  Tree failed: folder diagram stays, no error line (it's an enhancement).

## Non-goals

Import/dependency analysis; history-based learning (B); per-file listings; persisting learned
structure across sessions (cheap to relearn).

## Constraints

Read-only `gh api`, background jobs only, no panics in view code, all strings through i18n with
Thai, tests for the learning on realistic trees (the user's layout, a flat repo, a monorepo with
`packages/*`, nested containers, a truncated tree).

## Calls I made

Thresholds (3 subfolders, half of siblings, 2 shared parts, 60%); learning from the From branch
(so new modules exist in it); file names compared without extension; NEW/REMOVED rules; row
caps; folder diagram as fallback and while loading.
