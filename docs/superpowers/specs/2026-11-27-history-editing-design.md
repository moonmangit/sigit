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

The guard's dialog already offers **Stash and continue**. A hard reset and every edit that makes
the working copy change (revert, cherry-pick, replay) use it: with uncommitted changes, the user
must stash first, and Undo pops the stash. Soft and mixed reset and amend do not touch the files,
so they need no stash. Amend needs staged changes and refuses without them.

## Risk

- Low: a new commit on top (revert, cherry-pick), amend of an unpushed commit.
- Medium: any reset or rewrite of commits that are not on the upstream; the branch is the
  default branch or a production branch name.
- High (type the branch name): the edit removes or rewrites commits that are on the upstream,
  or on a protected or default branch; a hard reset that removes commits.
- Blocked: conflict in the simulation, detached HEAD, operation in progress, root commit in the
  range, a merge commit in a replay range, no `user.name` / `user.email`, the preflight failed.

## Where in the UI

The Commits page's side pane for a selected commit gets an **Edit history** section:
- For a commit on the current branch: Reset here (soft / mixed / hard), Revert, Reword…, Drop,
  Squash into parent, Fixup into parent, Move up, Move down.
- For a commit not on the current branch: Cherry-pick onto {branch}, Reset here (hard and soft).
- The toolbar: **Amend last commit** (with staged changes).

Each button simulates first (spinner on the button) and then opens the guard's dialog with the
preview. Reword opens a small dialog with the message first.

## Tests

- `src/history.rs` tests make temporary repositories with `git init` and test each operation:
  the new tip's tree and messages, the kept SHAs, conflicts, the preflight refusing after a
  tip moves, the compare-and-swap, the rollback, and Undo.
- The command lines (`Cmd::line`) for the dialog are tested as in `gh.rs`.
- Thai strings for every new text; `scripts/i18n-check.py` passes.
