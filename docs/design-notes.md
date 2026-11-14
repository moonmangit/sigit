# sigit: daily-use and safety features (design notes)

Status: **all features below are built.** Kept as the design notes behind them: the problem
each one solves, how it works, and the rules it follows.

Goal: something developers open first thing every morning and keep open all day, because it
answers "what needs me?" faster than github.com and **stops them before they break something
important**.

Ground rules, same as today:
- No commit and no push, ever.
- Everything goes through `gh` / `git`.
- Nothing blocks the window.
- Local settings only.

---

## What I checked before writing this

Each feature below uses data that is actually readable with your current `gh` login
(scopes `repo`, `read:org`, `workflow`, `gist`):

| Data | Readable? | Notes |
|---|---|---|
| Is a branch protected (`GET /repos/{r}/branches/{b}` → `protected`) | Yes | true for `anomalyco/rune` `dev` |
| Rulesets on a branch (`GET /repos/{r}/rules/branches/{b}`) | Yes | `dev` has `non_fast_forward`, `deletion` |
| Full protection settings (`…/branches/{b}/protection`) | **Only for repo admins** | 404 elsewhere, so the app must fall back to the two rows above |
| Environment protection rules (`GET /repos/{r}/environments`) | Yes | e.g. `dev` has `branch_policy` |
| Notifications (`GET /notifications`) | Yes | includes review requests the Inbox search misses (e.g. from an organization you're not watching) |
| Failed job logs (`gh run view --log-failed`) | Yes | |
| Your permission per repo (`.permissions`) | Yes | **if you are an admin on a repo**, GitHub lets you bypass protections there. The app's guard is the only thing that stops a mistake |

---

## Part A: Guard rails (prevent touching something important)

### A1. Risk levels on every write action ⭐ foundation for A2–A6

**Problem:** today Merge, Close and Cancel each have one plain confirm dialog. A merge into
`main` of a production repo gets the same "Merge? / Keep it" as a typo fix in a sandbox repo,
so people click through it.

**Behaviour:** every write action gets a risk level before it runs:

| Level | When | What the user sees |
|---|---|---|
| **Low** | e.g. comment, approve, fetch, checkout of a clean tree | runs immediately, toast with result |
| **Medium** | merge into a normal branch, close PR, cancel a CI run, re-run | today's confirm dialog, plus a **"why this is risky" list** |
| **High** | anything in A2–A5 matches: protected/default branch, production environment, sensitive files, you bypassing a rule as admin | red dialog, the reasons listed, and **type the repo name (or branch name) to confirm**. Merge button disabled until it matches |

The reasons come from A2–A5 and are listed as bullet points, e.g.:
- "`main` is protected: rulesets *deletion, non_fast_forward*"
- "2 of 5 required checks are failing"
- "This PR changes `.github/workflows/deploy.yml`"
- "You are an admin, so GitHub would let this through"

**Done when:**
- Every existing write action goes through one risk-check function.
- High risk can't be confirmed without typing the name.
- The checks run before the dialog opens, are cached for 60 s, and show a spinner if slower.

Size: **M**

---

### A2. Protected-branch awareness

**Problem:** you can't tell from the list which branches matter.

**Behaviour:**
- **Shield badges:** a 🛡 badge on protected branches (and the repo's default branch) in Branches, Commits ref chips, PR base chips, and the repo header.
- **Before merging a PR,** read the base branch's rulesets and the PR's required checks and review state. Not approved, checks failing, or base protected → raise the risk level (A1).
- **Admin bypass warning:** "GitHub would allow this because you're an admin. Your team's rules say it needs X."

**Data:** `branches/{b}` (`protected`), `rules/branches/{b}`, PR `reviewDecision` + `statusCheckRollup`
(already loaded), repo `.permissions`.

**Done when:**
- Badges show on all four places.
- Merging a PR into a protected base with a failing required check is High risk.
- A missing permission (404) falls back silently to `protected`.

Size: **M**

---

### A3. Sensitive-change detector for PRs

**Problem:** the dangerous PR is the 3-line one that edits a deploy workflow or a DB migration.

**Behaviour:**
- **Flag PRs by changed paths and diff content:**
  - **Paths (default patterns):** `.github/workflows/**`, `CODEOWNERS`, `**/*.tf`, `**/migrations/**`, `Dockerfile*`, `**/k8s/**`, `**/helm/**`, `.env*`, lock files with >500 changed lines.
  - **Diff content:** added lines that look like secrets: AWS keys, `-----BEGIN … PRIVATE KEY`, `ghp_…`, `xox[bp]-…`, high-entropy strings next to `password|secret|token`.
- **Where flags show:**
  - ⚠ chips in the PR list ("workflows", "migration", "possible secret").
  - A "Why this needs care" box at the top of the PR detail.
  - Raised merge risk (A1).
- **Your own patterns:** add more per repo or per group (Settings, A6).

**Data:** `gh pr view --json files` (already loaded) + `gh pr diff` (already loaded for the
Files tab). The list view needs files per PR: one extra `files` field on the list query, or lazy
per visible row.

**Done when:**
- A PR touching `.github/workflows/x.yml` shows the chip and raises merge to High.
- A diff adding `AKIA…` shows "possible secret" with the line highlighted.
- No false flag on the test repo's last 50 merged PRs except real workflow changes.

Size: **M**

---

### A4. Production environment guard

**Problem:** re-running or cancelling the wrong workflow can deploy, or half-deploy, to production.

**Behaviour:**
- **Mark an environment as production** if any of these is true:
  - its name matches `prod|production|live|release`, or one you list
  - it has protection rules (required reviewers, wait timer)
- **Mark a workflow as a deploy workflow** if any of these is true:
  - it targets that environment (its jobs' `environment`, seen in run data)
  - its name matches `deploy|release|publish`
- **What changes:**
  - Re-run / Cancel on a deploy workflow becomes **High** (type the environment name).
  - The Environments page shows a red "production" tag.
  - The Actions run detail shows "This run deploys to *production*".

**Data:** `environments` (protection_rules), run jobs (`gh run view --json jobs`), workflow name.

**Done when:**
- Re-running a run whose workflow deploys to a protected environment requires typing its name.
- Ordinary CI re-runs stay one-click.

Size: **S–M**

---

### A5. Safe local git (dirty tree / in-progress state)

**Problem:** Checkout and Pull fail, or worse, leave a mess, when the working tree has changes or
a rebase/merge is half done. Today the app just shows git's error afterwards.

**Behaviour:** before Checkout / Pull / PR checkout, read `git status --porcelain=v2 --branch`
plus `.git/MERGE_HEAD`, `rebase-merge/`, `rebase-apply/`, `CHERRY_PICK_HEAD`:
- **In-progress merge/rebase/cherry-pick:** block, explain, offer "Open folder" only.
- **Uncommitted changes:** dialog lists the changed files (first 20) and offers:
  - **Stash and continue** (`git stash push -u -m "sigit: before checkout X"`). The toast tells you how to get it back, with a "Pop stash" button on the branch page.
  - **Cancel.**
- **Detached HEAD with commits not on any branch:** warn that those commits become hard to find.
- **Pull** is already `--ff-only`. If it would fail because the branches diverged, say so in plain words instead of git's message.

**Done when:**
- Checkout with a dirty tree never runs `git switch` directly.
- Stash shows up in a "Stashes" list on Branches, with apply/drop, where drop is Medium risk.

Size: **M**

---

### A6. Activity log + undo window

**Problem:** "Did I just close the wrong PR?" There's no history and no undo.

**Behaviour:**
- **Activity panel:** every write action is recorded locally (time, repo, action, target, result, command run). Filterable, last 30 days.
- **Undo for reversible actions:** a toast with **Undo** for 10 s:
  - Close PR → `gh pr reopen`
  - Stash → `git stash pop`
  - Checkout → back to the previous branch
  - Pin/group edits
- **Merge gets a 5 s grace period instead** ("Merging #943 in 5… Cancel"), because a merge can't be undone.
- **Settings page:** the risk patterns (A3, A4), "always ask" list, grace duration.

**Data:** local `%APPDATA%\sigit\activity.jsonl` (append-only, rotated at 5 MB).

**Done when:**
- Each action appears in the log with its outcome.
- Undo restores a closed PR.
- Cancelling during the grace period runs nothing.

Size: **M**

---

## Part B: Daily flow (make people want to open it)

### B1. Notifications that match GitHub's, with Done / Snooze

**Problem:** the Inbox uses search, which misses things GitHub notifies you about: mentions,
CI failures on your PRs, comments, team review requests. Your account has those right now.

**Behaviour:**
- **New Inbox section "Notifications"** grouped by reason (review requested, mentioned, comment, CI, assign), unread first.
- **Per item:**
  - **Open** jumps into the app view (PR, run).
  - **Done** marks it read (`PATCH /notifications/threads/{id}`).
  - **Snooze 1h / tomorrow** is local only.
- **Unread count** on the Inbox row; refreshes every 60 s (GitHub sends `X-Poll-Interval`).

**Data:** `gh api notifications`, `gh api -X PATCH notifications/threads/{id}`.

**Done when:** a review request the search missed shows up in the Inbox, and Done
removes it on GitHub too.

Size: **M**

---

### B2. Review queue with "waiting time"

**Problem:** reviews are what block other people. They need to be visible and prioritised.

**Behaviour:**
- **A focused list of PRs waiting for your review:**
  - Sorted by **how long they've waited**, then size.
  - Each row shows age ("waiting 2d"), size, CI state, and A3 flags.
  - Colours: green under 4h, amber under 24h, red after that (thresholds configurable).
- **Keyboard:**
  - **J/K** move.
  - **Enter** opens the PR.
  - **A** opens Approve with a summary of what you're approving (checks, flags).
  - **C** focuses the comment box.
- **Your own PRs waiting on others** show the same way ("waiting on @x for 3d").

**Data:** search (already loaded) + `pr view` for timeline/age on demand.

**Done when:** you can clear your queue without the mouse, and age matches GitHub's
"review requested" time.

Size: **M**

---

### B3. Command palette (Ctrl+K)

**Problem:** with 241 repos, clicking through the sidebar is slow.

**Behaviour:**
- **Fuzzy jump to:**
  - any repo
  - `#123` in the current repo
  - a branch
  - a page ("actions rune")
  - an action ("fetch", "pull", "theme dark")
- **Ranking:** recent and pinned first.
- **Command actions** run through A1 risk checks like everything else.

**Data:** what's already loaded. Unknown `owner/repo` falls back to the live repo search.

**Done when:**
- Opening and typing feels instant (under 16 ms per keystroke on 241 repos).
- Enter on a result navigates.

Size: **S–M**

---

### B4. CI failure triage

**Problem:** "Checks failed" → open GitHub → find the job → scroll a 10,000-line log. Slow.

**Behaviour:**
- **"Why it failed" panel** in the run detail of a failed run:
  - The **failed steps' last ~80 lines** (`gh run view --log-failed`).
  - Error lines highlighted (`error`, `FAILED`, `panicked`, `Error:`).
  - Copy button.
- **Flaky hint:** if the same workflow on the same commit has both failed and passed, show "**Probably flaky**: passed on re-run".
- **One click from a PR:** the PR's Checks tab links straight to this panel.

**Data:** `gh run view {id} --log-failed` (lazy, capped at 200 KB), run list by `headSha`.

**Done when:** a failed run shows the failing step's error without leaving the app, in under
3 s on a normal run.

Size: **M**

---

### B5. Group dashboard (uses your groups)

**Problem:** your groups (e.g. "Payments" across two organizations) are a sidebar
convenience only. The real question is "is everything in this product OK?"

**Behaviour:** click a group header to open a **dashboard for that group**:
- **CI health matrix:** one row per repo; the default branch's latest run per workflow as coloured dots.
- **Open PRs** across the group in one table (repo column), with A3 flags.
- **Deployments matrix:** repos × environments, cells show the deployed ref/sha, age and status. This is the "what's on production for each service" view.
- **Release drift:** commits on the default branch since the last deployment to production ("12 commits not deployed").

**Data:** per repo in parallel: `run list --branch <default> --limit 10`, `pr list`, envs GraphQL
(already written), `compare/{deployed_sha}...{default}` for drift.

**Done when:** a 5-repo group's dashboard loads in parallel in about 1 slow call's time and
refreshes every 60 s while open.

Size: **L**

---

### B6. Watch & desktop notifications

**Problem:** you push, then refresh GitHub every minute waiting for CI.

**Behaviour:**
- **Watch** a run, a PR, or "my PRs' checks". The app keeps polling in the background, even with the window closed to the tray.
- **Desktop notification** when it finishes, fails, gets approved, or gets a change request. Clicking it opens that view.
- **Auto-watch** runs on branches you checked out today (toggle).

**Data:** existing run/PR calls on a slower timer; Rune's `rune-notify` crate for OS
notifications; Rune's `keep_running()` for the tray.

**Done when:** a watched run's failure raises a Windows notification within 30 s of GitHub
reporting it, and clicking the notification opens the run.

Size: **M**

---

### B7. Branch hygiene (local only)

**Problem:** dozens of dead local branches, and fear of deleting the wrong one.

**Behaviour:**
- **"Clean up" view on Branches** lists **local** branches that are any of:
  - (a) fully merged into the default branch
  - (b) gone upstream (`[gone]`)
  - (c) untouched for 90 days
- **What happens to each:**
  - **Fully merged:** deletes safely with `git branch -d`.
  - **Not merged:** needs `-D`, is **High** risk (A1), and lists the commits you'd lose.
  - **Current branch, default branch, and protected names:** never offered.
- **Remote branches are never deleted** (that's a push).

**Data:** `git for-each-ref` (already used), `git branch --merged <default>`, `git log
<branch> --not --remotes`.

**Done when:** after a squash-merge workflow, "gone upstream" branches are listed, and deleting
one warns if it has commits not on any remote.

Size: **S–M**

---

### B8. Instant start (offline cache)

**Problem:** every launch starts empty and waits ~1–2 s for `gh`.

**Behaviour:**
- **Cache:** save the last Inbox, repo list, and each repo's last PR/run lists to `%APPDATA%\sigit\cache\` on exit and every few minutes.
- **On launch** show them immediately, marked "updated 3h ago", then refresh in the background.
- **Never act on cached data:** every write re-reads the state first (ties into A1).

**Done when:** the second launch shows a full Inbox in the first frame, and stale data is
visibly marked.

Size: **S**

---

## Summary

| # | Feature | Part | Size | My suggested order |
|---|---|---|---|---|
| A1 | Risk levels + typed confirm | Guard | M | **1** (others plug into it) |
| A2 | Protected-branch awareness | Guard | M | 2 |
| A3 | Sensitive-change detector | Guard | M | 3 |
| A5 | Safe local git (dirty tree, stash) | Guard | M | 4 |
| A4 | Production environment guard | Guard | S–M | 5 |
| A6 | Activity log + undo window + settings | Guard | M | 6 |
| B1 | Notifications with Done/Snooze | Daily | M | 7 |
| B4 | CI failure triage | Daily | M | 8 |
| B3 | Command palette | Daily | S–M | 9 |
| B2 | Review queue with waiting time | Daily | M | 10 |
| B8 | Instant start cache | Daily | S | 11 |
| B7 | Branch hygiene (local) | Daily | S–M | 12 |
| B6 | Watch + desktop notifications | Daily | M | 13 |
| B5 | Group dashboard | Daily | L | 14 |

## Left out on purpose

- Commit, push, force-push, remote branch deletion, rebase/merge locally: out of scope: the app never writes history.
- Editing files, resolving conflicts: that's the editor's job.
- Storing a GitHub token in the app: keep using `gh`'s login.
