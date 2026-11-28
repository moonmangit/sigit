# Sandbox: practise history edits on a copy of the clone

*Design. Version 1: the sandbox only. Later: practice scenarios with goals, and "Apply to real repo"
(replay the sandbox result on the real branch with the same simulation, preflight and Undo).*

## Intent

A new user can be afraid to press Reset (hard) on a real branch, even with the simulation and
Undo. The sandbox gives them a copy of the real branch to practise every history tool on, with
no effect on the real clone or on GitHub.

## Decisions

- **One sandbox per repo**, made from the branch the real clone has checked out. Only commits are
  copied (a clean checkout), not uncommitted files.
- **A `git clone --local`** into `<settings folder>/sandboxes/owner__name`
  (`SIGIT_SANDBOX_HOME` overrides it for tests). Objects are hard links: cheap, and safe because
  git objects never change. Not `git worktree` (shares branch refs), not `--shared` (breaks when
  the real clone runs `git gc`).
- **The real remote again:** `origin` is removed (it pointed at the real folder) and added back with
  the real clone's origin URL; the real `refs/remotes/origin/*`, local branches and tags are
  fetched from the real folder (no new objects move). The branch's upstream, `user.name` and
  `user.email` are copied; `commit.gpgsign=false` (sandbox commits never leave it).
- **No push:** `remote.origin.pushurl` is a URL git can't use.
- **Mark:** `.git/sigit-sandbox` (JSON: slug, real folder, branch, base SHA, time). sigit deletes a
  folder only when it has the mark and is directly under the sandbox folder. The folder scan
  skips marked folders. A detached HEAD in the real clone blocks Start.

## In the app

- `Page::Sandbox` in sidebar 2 (needs a local clone). It shows a picture (your clone → copy →
  sandbox), what works and what doesn't, Start / Enter / Leave / Reset… / Delete… (Reset and
  Delete ask first), the +new / −gone commit counts against the base, the extra disk, and a note
  when the real branch moved. "How it works" is folded.
- `State::sandbox_on` maps a slug to its sandbox folder while it's on. `State::repo_dir` (and so
  `local_path`) returns that folder: every local page and action uses it. `Repo.path` (saved to
  disk) never changes.
- Switching (`local_switched`) drops the local loads, cached commit details, the selection, the
  reword editor and an open dialog for a local folder, then loads again. Every local job checks
  `same_dir` before it applies its answer, so a late answer from the other folder is dropped.
- `State::sandbox_block`, in `request()`: while the current repo's sandbox is on, a local action
  must be for the sandbox folder, and a GitHub action on this repo is refused with a toast.
- A banner on every repo page: "Sandbox of {branch}", Sandbox page, Leave sandbox. Sidebar 2
  marks the Sandbox row "On".

## Tests

- `sandbox.rs`: the copy has the real upstream and one unpushed commit as the real clone; other
  branches come along; push fails; edits leave the real clone's refs, HEAD, status and stashes
  unchanged; the real branch moving shows; Reset starts over; only marked folders are deleted;
  a detached HEAD leaves nothing behind.
- End to end through `State`: start, revert and hard reset through the dialog with Undo, a GitHub
  action and a real-folder action refused, leave, delete — the real clone ends as it began.
