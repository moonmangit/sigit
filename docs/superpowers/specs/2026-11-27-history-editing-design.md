# History editing with a simulated outcome and a preflight

*Design. This feature changes a product rule: until now sigit never made a commit. With this
feature sigit can make local commits and move local branch refs. It still never pushes.*

## Intent

The user wants to edit a local branch's history from sigit: reset, revert, and the other basic
history operations. Before anything changes, sigit must show the result of the operation (a
simulation). Before sigit moves the branch, it must check again that the repository did not
change since the simulation (a preflight). If the branch, HEAD or the upstream changed, sigit
stops and the user simulates again.

## Rule change

- Old rule: "No commit and no push, ever."
- New rule: "No push, ever. Local commits and local ref moves only through a simulated,
  preflighted transaction, with Undo."
- Change the README, `docs/design-notes.md` and the `src/git.rs` header to the new rule.

## Goals

The operations, all on the **current branch** of a local clone:

| Operation | What it does | Engine |
|---|---|---|
| Reset soft / mixed / hard | Move the branch to a commit. Soft keeps the index and the files, mixed keeps the files, hard replaces the files. | ref move only |
| Revert | A new commit on top that undoes a commit. For a merge commit, revert against parent 1 (mainline). | merge-tree |
| Cherry-pick | A copy of a commit from another branch, on top of the current branch. | merge-tree |
| Reword | Change a commit's message. The commits after it are made again. | replay |
| Drop | Remove a commit. The commits after it are made again. | replay |
| Squash / fixup into parent | Join a commit with the commit before it. Squash keeps both messages, fixup keeps the older message. | replay |
| Move up / move down | Change the order of two adjacent commits. | replay |
| Amend with staged changes | Put the staged changes into the last commit. | write-tree |
| Undo | Move the branch back to the tip it had before the last edit (10 s toast, as other Undo). | ref move only |

## Non-goals

- No push and no force-push. When an edit rewrites commits that are already on the upstream,
  sigit tells the user and shows the command (`git push --force-with-lease`) to run themself.
- No edits to a branch that is not checked out, to a detached HEAD, or to the root commit.
- No replay through merge commits (the range must be linear). Revert of a merge is allowed.
- No interactive conflict resolution. A conflict in the simulation blocks the operation and
  lists the files.
- No commit hooks. Plumbing commands (`commit-tree`) do not run hooks, as `git rebase` with
  `--no-verify`. The spec records this so the user knows.

## How it works: one transaction in four steps

1. **Snapshot.** Read the state: branch, the branch's ref, tip SHA, upstream name and the
   upstream SHA we have locally, uncommitted files, and an operation in progress.
2. **Simulate.** Make the result as real commit objects with plumbing commands. No ref moves,
   and the index and the working copy do not change. The result is a new tip SHA.
   - `git merge-tree --write-tree --merge-base=<base> <ours> <theirs>` gives the tree of a
     three-way merge, or the conflicted files (exit status 1).
     - cherry-pick C onto T: base `C^`, ours `T`, theirs `C`.
     - revert C onto T: base `C`, ours `T`, theirs `C^` (`C^m` for a merge, mainline m = 1).
   - `git commit-tree <tree> -p <parent> -F -` makes the commit. The author name, email and
     date come from the original commit (except revert, which is a new commit by you).
   - Replay: walk the commits from the oldest changed commit to the tip. A commit whose new
     parent is its old parent is kept as it is (same SHA). The others are made again with
     merge-tree and commit-tree.
   - New objects that no ref uses are deleted later by `git gc`. They do not hurt.
3. **Preview.** The confirm dialog shows:
   - the old and new tip;
   - the commits that leave the branch, the commits that come in, and the commits made again;
   - `git diff --stat <old> <new>` (what the files will look like after);
   - the uncommitted files that a hard reset would replace;
   - the number of rewritten commits that are already on the upstream;
   - the risk reasons (see Risk).
4. **Preflight and apply.** Preflight runs twice: when the dialog opens (on the worker) and
   again just before the apply. It compares fresh state with the snapshot:
   - the branch is the same, and HEAD is attached to it;
   - the tip SHA is the same;
   - no merge, rebase, cherry-pick or revert is in progress;
   - the local upstream SHA is the same;
   - the remote's SHA (`git ls-remote <remote> refs/heads/<branch>`, read only) is the same as
     the local upstream SHA. If the remote has new commits, the user must fetch first. If the
     remote cannot be reached, this is a warning only, unless the edit rewrites pushed commits:
     then it blocks.

   If a check fails, nothing changes and the message tells the user to simulate again.

   The apply uses the exact new SHA from the simulation. It does not run the operation again.
   - `git update-ref -m "sigit: <op>" refs/heads/<branch> <new> <old>`: git refuses if the tip
     is not `<old>` (compare-and-swap). This is the commit point of the transaction.
   - Then the index and the working copy:
     - soft: nothing;
     - mixed: `git read-tree <new>` (the index only);
     - hard: `git reset --hard <new>`;
     - all other edits: `git read-tree -m -u <old> <new>` (a two-tree merge: it changes only
       the files that differ between old and new, and refuses if a local change is in the way).
       If it refuses, sigit moves the ref back with `git update-ref <ref> <old> <new>`.
     - amend with staged changes: nothing (the index is already the new tree).

## Uncommitted changes

- **Hard reset:** the dialog asks to **Stash and continue** first. That button needs the same
  checks as Confirm, including the typed branch name for High risk. sigit records the stash it
  made by its SHA (git makes none when there is nothing to stash), and:
  - if the apply fails, it pops that stash back at once and says so;
  - Undo moves the branch back and pops that stash (found by SHA, not `stash@{0}`), so an older
    stash of the user's is never popped.
- **All other edits** keep uncommitted changes in place: `read-tree -m -u` changes only the files
  that differ between the old and the new tip. The preflight blocks, with the file names, when an
  uncommitted file is one of those files. If git still refuses, the branch goes back (rollback).
- Soft and mixed reset and amend do not touch the files. Amend needs staged changes.

## Pushed commits, signing, the remote

- A rewritten commit counts as pushed when **any** remote branch has it.
- With `commit.gpgsign=true`, `commit-tree` gets `-S`, so rewritten commits stay signed.
- The remote check uses no prompts (`GIT_TERMINAL_PROMPT=0`) and stops after 20 s; then the remote
  counts as unreachable (a warning, or a block when pushed commits are rewritten).
- Reflog notes are English (`sigit: reword abc1234`), whatever the interface language.

## Risk

- Low: a new commit on top (revert, cherry-pick), amend of an unpushed commit.
- Medium: any reset or rewrite of commits that are not on the upstream; the branch is the
  default branch or a production branch name.
- High (type the branch name): the edit removes or rewrites commits that are on the upstream,
  or on a protected or default branch; a hard reset that removes commits.
- Blocked: conflict in the simulation, detached HEAD, operation in progress, root commit in the
  range, a merge commit in a replay range, no `user.name` / `user.email`, the preflight failed.

## Where in the UI

- The Commits page's side pane for a selected commit has an **Edit history** card (header with
  the branch and ⓘ). Its buttons are grouped by what you want: **Go back to here** (Reset soft,
  mixed, hard), **Undo this commit** (Revert, Drop), **Change this commit** (Reword…, Squash,
  Fixup, Move up, Move down) and **Copy to {branch}** (Cherry-pick). Whether the commit is on the
  current branch, and whether it is HEAD, comes from git when its detail loads (so Commits filters
  can't hide buttons); after any action that moves a branch, the cached details are dropped.
- ⓘ opens a folded guide: action names; one opens to a before/after picture, what it does and
  its common use. "How it works" folds too. Every button's hover help has the same text.
- The toolbar has **Amend last commit**.
- The confirm dialog is a graphic preview: a commit graph (− leaves, + new, ~ made again,
  ↳ joined), a bar per changed file, risk chips, **Why?** for the full reasons, and **Runs**.
  **Play** animates the edit on the real commits with their neighbours (Before → What changes →
  After). Long dialogs scroll; the buttons stay in view.

## Tests

- `src/history.rs` tests make temporary repositories with `git init` and test each operation:
  the new tip's tree and messages, the kept SHAs, conflicts, the preflight refusing after a
  tip moves, the compare-and-swap, the rollback, and Undo.
- The command lines (`Cmd::line`) for the dialog are tested as in `gh.rs`.
- Thai strings for every new text; `scripts/i18n-check.py` passes.
