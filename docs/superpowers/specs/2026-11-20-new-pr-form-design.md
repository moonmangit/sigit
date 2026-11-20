# New pull request form (replaces PR routes)

*Short design. Supersedes `2026-11-19-pr-routes-design.md`. The user tried the route flow and didn't
like it.*

## Intent

The user wants to open PRs from sigit the normal way: any branch into any branch, like GitHub's
"New pull request". They open `feat-a → staging`, merge and test it, and later open `feat-a → main`
by going through the same form again. They asked for helper tools on top, and left the choice of
helpers to me.

## Goals

- A **New pull request** form for a repo. It has From and To branches, Ready or Draft, title,
  description, reviewers, assignees and labels.
- It opens from a **New pull request** button on the repo's Pull requests page, and from an
  **Open PR** button for the selected branch on the Branches page.
- It has the helpers listed below.
- Every PR still goes through the guard's confirm dialog, which shows the exact `gh pr create` command.

## Non-goals and removals

- No routes, steps, or ordering. Remove:
  - the route in Settings → Git
  - "Route for this repo…"
  - the route section in the Branches pane
  - step states
  - "after X is merged"
  - `Prefs.pr_route` and `Prefs.pr_route_overrides`
  - the keep-branch-on-merge rule, so Merge's "Delete branch" goes back to starting on
- Old config files that contain the removed fields must still load. serde ignores unknown fields
  unless `deny_unknown_fields` is set; check that it isn't.
- No pushing branches, no editing existing PRs, no milestone or project fields.

## Approaches

The user picked "normal, like git, plus helpers" over the route model (approach 1 of the earlier
spec). The other two earlier approaches, reusing Comparisons and an unordered target menu, were
also route-shaped, so they are dropped too.

## Helpers (my calls)

1. **Starting values.** From is the branch picked on the Branches page, or else the current branch
   of the clone, or else blank. To is the last target used in this repo, or else the repo's default
   branch. Both are dropdowns over the repo's branches. Remote names are shown without `origin/`.
2. **Live comparison.** It shows "N commits ahead" for From → To, using the existing
   `compare::fetch_branch`. It updates when either side changes, and shows a spinner while loading.
   With 0 commits, it says "Nothing to merge: {from} has no commits that {to} doesn't" and disables
   Create PR. If From and To are the same branch, Create PR is disabled too.
   *Ruling:* no "files changed" count. The comparison data has commits but no file count, and a file
   count would cost another call.
3. **Fill buttons.** "Use last commit" sets the title to From's latest commit subject. "List
   commits" sets the body to one `- subject` line per commit in the comparison (PR titles for
   commits that came in through PRs), replacing what's there.
4. **Already-open check.** It uses the existing `gh::branch_prs(slug, from)`. If an OPEN PR from
   From into To exists, it shows "A PR from {from} into {to} is already open: #n" with a click
   that opens it, and disables Create PR.
5. **Push check.** This is the existing three-way `pushed_state`. When the branch list has loaded
   and has no `origin/{from}`, it says "Push {from} to GitHub first" and disables Create PR. When
   the branch list hasn't loaded, it doesn't block.
6. **Remembered per repo.** After a PR is created, the repo remembers To, draft, reviewers,
   assignees and labels, saved in `Prefs.pr_last: Vec<PrLast { slug, base, draft, reviewers,
   assignees, labels }>`. The next form for that repo starts from them. There is nothing to set up.
7. **Edits survive.** Edits survive a cancelled confirm or a failed `gh pr create`. This keeps the
   existing `last_pr_draft` behaviour.
8. **Blank title.** A blank title disables Create PR. This is kept from the existing behaviour.

## Reused from the PR-routes branch

These stay:

- `route::PrSpec`
- `gh::create_pr_cmd`
- `Action::CreatePr` and its risk. It is Medium for a production branch name. The "previous step
  isn't merged" reason goes away with `after_unmerged`.
- `gh::branch_prs` and `BranchPr`
- `split_list`
- `pushed_state` and `is_pushed`
- the dialog's `TextEdit` and `TextArea` fields
- the `pr_number_from_url` toast
- `action_finished`

The rest of `src/route.rs` and `src/ui/route.rs` is deleted, or renamed into a `pr_new` module if
that reads better. That's the builder's call.

## Left to the builder

- Exact layout, following the existing dialogs.
- The module name.
- How the comparison and the open-PR check are keyed and cached, following the existing `Load`
  pattern.
- Tests must be network-free. Cover the starting values, fill buttons, the already-open check, the
  push check, remembered values, the removal not breaking old prefs, and Merge "Delete branch"
  defaulting to on again.
- Every string goes through `t()`/`tf!()` with a Thai row (`python scripts/i18n-check.py` gives
  `0 without Thai`). Remove the Thai rows for deleted strings.
