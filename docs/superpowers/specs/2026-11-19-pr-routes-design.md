# PR routes: open a branch's PRs, target by target

## Intent

The user ships a feature branch in steps. They open `feat-a → staging`, merge it, and test it by
hand on staging. Once it passes, they open `feat-a → main` from the same branch. Staging is never
merged into main.

Today sigit can't create a PR at all: it lists PRs and merges, approves, closes or comments on
them. So the user leaves the app for every PR, re-types the same settings (draft, reviewers,
labels…) each time, and has to remember which step each branch has reached.

This adds:

- **A route**: the ordered list of target branches a feature goes through (`staging`, then `main`).
- **A preset per target**: what a PR into that target usually looks like.
- **On the Branches page, for the picked branch**: where it is on the route, and a button to open
  the next PR in a dialog filled in from the preset.

## Goals

- From the Branches page, open a PR from the picked branch into one target, with the user choosing
  every setting in a dialog filled in from that target's preset.
- Show, for the picked branch, every route step: no PR yet, open (draft or ready), merged, or
  closed, with a link to the PR.
- Set up the route and its presets once, in general, and let one repo replace them with its own.
- Protect the branch: merging a PR into a step that isn't the last must not delete the feature
  branch by default, since the next step still needs it.

## Non-goals

- Opening several PRs at once. The user opens each step when they're ready.
- Tracking "tested on staging". Testing is human. The next step is offered once the previous
  step's PR is merged, and opening it means the user decided it passed. No CI signal is shown as a
  stand-in.
- Pushing branches. A branch that isn't on GitHub can't get a PR, and the dialog says so.
- Editing an existing PR's settings (title, reviewers…) after it's opened.
- Promoting `staging → main` as a branch (that's what Comparisons shows).

## Anti-goals (a failure even if it works)

- The app opens a PR the user didn't see in full first. Every PR goes through the existing guard
  confirmation, showing the exact `gh pr create …` command.
- A preset silently wins over the user. The dialog always shows every field, already filled in,
  and editable.
- The feature branch gets deleted after the staging merge while main is still ahead.

## Approaches considered

1. **A target list in Settings (picked).** An ordered route (general, with per-repo replacement),
   each target with its preset. The pane shows steps and suggests the next one. Costs one more
   Settings section; matches the user's ordered flow and is what makes branch protection possible.
2. **Reuse Comparisons.** Use the `to` side of the existing branch comparisons as the targets. No
   new setup, but Comparisons mean "branch promoted into branch" (dev → staging), not "where
   features go". The two would get tangled, and editing one would change the other.
3. **No order.** An "Open PR" button with a target menu, presets per target, and a list of this
   branch's PRs. The least work, but it can't point to the next step or protect the branch.

The user picked 1.

## What the user decided

- The route is ordered and walked one step at a time (staging first; main only after staging is
  merged and tested by hand).
- The user decides every PR's settings themselves, in the dialog. Presets only fill the dialog in.
- Presets exist in general, and per repo.
- Presets are kept per target branch.

## Design

### Settings: the route

In Settings → Git, next to Branch comparisons, a new **PR route** section:

- An ordered list of targets. Each row: the target branch name and its preset fields (below), with
  buttons to move it up or down and remove it. "Add target" appends one.
- This is the general route, used by every repo.
- A repo can have **its own route** that replaces the general one completely (targets and
  presets). It's edited from the Branches pane ("Route for this repo…", same idea as the
  "Comparisons for owner/repo" dialog), with "Use the general route" to drop it.

*Builder's call (mine):* "replaces completely" rather than per-field merging. It's easier to reason
about, and Comparisons' per-field overrides have already shown how much UI that takes. If one repo
only differs in reviewers, its route is a copy with that change.

Saved in `Prefs` (`src/config.rs`) as `pr_route: Vec<RouteTarget>` and
`pr_route_overrides: Vec<RepoRoute { slug, targets: Vec<RouteTarget> }>`, with serde defaults so
existing config files load unchanged. The route that applies is the repo's own if there is one,
else the general one.

A default general route is empty. The pane then shows how to set one up (see below), so nothing
appears that the user didn't configure.

### A target's preset

Every field is a starting value for the dialog:

| Field | Meaning | `gh pr create` flag |
|---|---|---|
| `base` | The target branch (`staging`, `main`) | `--base` |
| `draft` | Open as draft | `--draft` |
| `title` | Title template | `--title` |
| `body` | Description template | `--body` |
| `reviewers` | Logins or `org/team`, comma-separated | `--reviewer` |
| `assignees` | Logins, comma-separated; `@me` allowed | `--assignee` |
| `labels` | Comma-separated | `--label` |

Templates fill in `{branch}` (the head branch name), `{target}` (the base), and `{subject}` (the
head branch's latest commit subject). An unknown `{…}` is left as typed. Empty title defaults to
`{subject}`.

*Builder's call (mine):* no milestone or project fields yet; they need extra lookups and the user
didn't mention them. They can be added the same way later.

### The Branches pane: route steps

When a branch is picked on the Branches page, the existing branch pane (`compare::branch_pane`)
gains a **PR route** section above the comparison, for the route that applies to this repo:

```
PR route                                   Route for this repo…
 ① staging   #142  merged 2d ago            ↗
 ② main      no PR yet                      [Open PR → main]
```

Each step is one of:

- **No PR yet.** If it's the first step, or the step before it is merged, an **Open PR → target**
  button. Otherwise muted text: "after staging is merged". The user can still open it early from a
  small "Open anyway" link, since it's their call. That PR's confirm dialog then says the step before
  isn't merged yet.
- **Open** (draft or ready), **merged**, or **closed**: the PR number, state and age; a click opens
  the PR in the app's PR view (`open_pr`). A closed (unmerged) step offers **Open PR** again.
- If several PRs exist for one step, the newest is shown and the count noted.

**A step's state is its newest PR's state** (the user's decision). A step counts as merged only
when its newest PR is merged. So if #142 into staging is merged and the user then opens #160 into
staging with a fix, staging is "open" again. Main goes back to "after staging is merged" until #160
is merged, so a fix reaches staging before main. This one rule drives the step shown, the Open PR
button, the "Open anyway" link, and the Medium-risk rule in `CreatePr`.

Where it comes from: one call per picked branch,
`gh pr list --repo SLUG --head BRANCH --state all --json number,state,isDraft,baseRefName,mergedAt,createdAt,url`,
matched to steps by `baseRefName`. It loads with the pane, reuses the app's `Load<T>` pattern
(spinner, error, cache), and reloads after a PR is opened or merged from the app. PRs into a base
that isn't on the route are listed under the steps as "Also open: #150 → hotfix".

Branches that are route targets themselves (`staging`, `main`), and the default branch, show no
route section.

With no route set up: one muted line, "Set up a PR route in Settings → Git to open PRs from here",
with a button that goes there.

### The open-PR dialog

**Open PR → target** opens a dialog (a `Dialog` like the "Comparisons for…" one):

```
Open PR  feat-a → main                 preset: main (general)
 ( ) Ready for review   (•) Draft
 Title      [ feat-a: add login                        ]
 Body       [ …                                        ]
 Reviewers  [ lead ]     Assignees [ @me ]
 Labels     [ release ]
                                   [Cancel]  [Open PR]
```

- Fields start from the preset with templates filled in. The user edits any of them.
- "preset: main (general)" or "(this repo)" says where the values came from.
- **Open PR** hands a new guard action to the existing confirm flow. Nothing is sent from the dialog
  itself.
- If the head branch has no remote branch (`origin/feat-a` missing), the dialog says "Push feat-a
  to GitHub first" and Open PR is disabled.

### The guard action

A new `Action::CreatePr { slug, head, base, draft, title, body, reviewers, assignees, labels }` in
`src/guard.rs`, built by a new `gh::create_pr_cmd(...)` in `src/gh.rs` so what's shown equals what
runs (the `Cmd` rule there). Its assessment:

- Low risk by default ("Opens a PR; you can close it").
- Medium if the base matches the production names in Settings → Safety
  (`risk::is_production` with `prefs.prod_names`), or the previous route step isn't merged yet.
- After it succeeds: a toast with the new PR number, the route statuses reload, and the repo's PR
  list refreshes (as the other PR actions do in `guard.rs`).

### Protecting the feature branch when merging

When the Merge confirm dialog (`overlays.rs`, `Action::Merge`) is opened for a PR whose base is a
route step that **isn't the last**, and whose head is a feature branch:

- "Delete branch" starts **off** (today every Merge starts with `delete_branch: true`).
- If the user turns it on, a warning line appears: "main comes after staging on this route".

*Builder's call (mine):* this looks only at the route's **order**, not at the later steps' PR
status. It's a plain config lookup that needs nothing loaded first, so the Merge dialog never waits.
In the rare case where main is already merged too (via "Open anyway"), the switch still starts off.
The user can turn it on, and the warning is still true.

To know this, the Merge request needs the PR's head and base. They're already on the PR model
(`head_ref_name`, `base_ref_name`). The check lives in one function
(`route_next_step(slug, base) -> Option<String>`, the next target after `base`) used by every place
that builds an `Action::Merge` (`prs.rs` and the test fixtures in `main.rs`).

### Text

All new UI strings go through `crate::i18n::t`/`tf!` and get Thai entries in
`assets/i18n/th.tsv` (`scripts/i18n-check.py` must report 0 missing).

## Left to the builder

- Exact layout and spacing of the Settings section, the pane section and the dialog, following the
  existing Comparisons UI.
- How route statuses are cached and keyed, following the existing `Load`/cache patterns.
- Whether the repo-route dialog reuses code from the Comparisons-for-repo dialog.
- Test fixtures for the new pane states (no route, each step state, branch not pushed), following
  the existing fixture list in `main.rs`.
- Unit tests for: template filling, choosing the route that applies (repo vs general), step
  matching from `gh pr list` JSON, `create_pr_cmd` arguments, and the delete-branch default.
