<p align="center"><img src="assets/icon.png" width="112" alt="sigit icon"></p>

# sigit

A fast desktop app for everyday GitHub and git work: pull requests, Actions, environments and
history, across all your repositories and organizations. Built in Rust on
[Rune](https://github.com/anomalyco/rune), a GPU-rendered native UI.

**It reads and reviews, and edits local history only after it shows you the simulated outcome; it never pushes.** In English or Thai (ไทย).

## Download

Get the latest build from this repository's **Releases** page:

| System | Installer (recommended) | Portable |
|---|---|---|
| Windows 10/11 (x64) | `sigit-setup-windows-x64.exe`: installs for your user (no admin), adds a Start menu entry, uninstall from Settings → Apps. | `sigit-windows-x64.zip`: unzip, run `sigit.exe`. |
| macOS, Apple Silicon | `sigit-macos-arm64.dmg`: open it, drag sigit into Applications. | `sigit-macos-arm64.zip` |
| macOS, Intel | `sigit-macos-x64.dmg` | `sigit-macos-x64.zip` |

The builds aren't code-signed yet. On Windows, SmartScreen may warn the first time: **More info
→ Run anyway**. On macOS (not notarized by Apple), open it the first time with **right-click →
Open**. Uninstalling keeps your settings (`%APPDATA%\sigit` or `~/.config/sigit`).

`SHA256SUMS.txt` lists each file's checksum. On Linux, build from source (below).

## Requirements

- The [GitHub CLI](https://cli.github.com), signed in (`gh auth login`, check with `gh auth status`).
- `git` on your `PATH`.
- To build: Rust (the version in `rust-toolchain.toml` installs itself through rustup), and on
  Windows the Visual Studio Build Tools with the C++ workload.

sigit calls `gh` and `git` for you, so it never sees a password or token.

Missing something? You don't need to set it up first: on launch sigit checks for git, gh
and a gh sign-in, and if one is missing it shows a setup screen with the install command
(`winget` on Windows, `brew` on macOS), a download link, and `gh auth login` (Copy, or Run in
terminal). It re-checks every few seconds and carries on by itself once everything's there.
On a first run it then shows a short welcome.

## Build from source

```bash
cargo run --release                    # opens the repo you're in, plus your GitHub repos
cargo run --release -- path/to/repo    # start on a specific local clone
```

The first build downloads Rune (pinned in `Cargo.toml`) and takes a few minutes. On Linux,
install `pkg-config libxkbcommon-dev libwayland-dev libdbus-1-dev` first.

To build a binary to share, use `scripts/release.ps1` (Windows) or `scripts/release.sh`: they
keep your local folder paths out of the binary. Official releases are built by
`.github/workflows/release.yml` when a `v*` tag is pushed.

## What's in it

| View | Shows | Actions |
|---|---|---|
| **Dashboard → Activity** (default; formerly Inbox) | Your tracked repos over a range (this week by default; today, 7 or 30 days, this month, custom): open PRs and which are **ready for review (all checks passed)**, Actions health (exact pass rate, what's red on the default branch now), commits, lines, deployments, releases, contributors, and a section per repo | Scope to a group, manage what's tracked |
| **Dashboard → Needs you** | Review queue (ready PRs first, oldest wait first), your open PRs, assigned issues, notifications | Done / Snooze, J/K/Enter/A/C |
| **Dashboard → Comparisons** | Per tracked repo, commits and PRs waiting between your branch pairs (dev → staging…), one GitHub query | Edit per repo; click a cell for its PRs and commits, each with a thin bar of the modules it touched (click a commit without a PR to load its bar) |
| **Pull requests** | Open / merged / closed with checks, review state, diff size; detail with description, reviews, files and diff, checks | Approve, request changes, comment, merge (squash / merge / rebase), close, check out |
| **Actions** | Runs with duration and result, jobs and steps on one timeline, failing log lines | Re-run failed or all, cancel, watch |
| **Environments** | What's deployed where, a deployment timeline | Open the deployed commit |
| **Commits** (local clone) | Commit graph with lanes, branch and tag chips | Commit detail with diff; **edit history** (below) |
| **Branches** (local clone) | Local and remote, ahead/behind, stashes, clean-up of merged or gone branches; select a branch for a side pane of what it brings to the default branch (or one you pick): PRs, contributors, modules | Checkout, fetch, fast-forward pull |
| **Groups** | A dashboard for repos you group yourself, across owners | |

The sidebar starts with the **Dashboard**, then your **pins**, your **groups** and, under
**Repositories**, every repo you can reach, grouped by owner. In each group and owner the repo you
opened last comes first, marked *recent*. Drag groups and organizations by their headers, and
pinned repos and the repos inside a group, into your order; fold sections, and give
organizations your own colour and icon.

Pick a repo or group and a second sidebar lists its sections, each with its count (Pull
requests, Actions, Environments, Commits, Branches; for a group, Overview and one panel at a
time, then its repos). Drag its edge to resize it. Its **filters** (a date range, and a
contributor you type or pick from a list with profile pictures) stay as you move around, and
narrow pull requests, runs and commits in the query itself, so they reach past the first page.

Side-by-side sections have a handle between them: drag to resize, double-click to reset.

Around every page:

- **Links open the exact thing.** A red workflow on Activity opens that failing run, selected, with
  its details open and its row scrolled into view; a CI notification opens the latest failed run; a
  PR anywhere opens that PR. One older than the loaded list is fetched on its own.
- **Copy what you paste elsewhere.** SHAs, branch and file names, links, clone URLs and error
  messages have a copy button or copy on click.
- **Ctrl+K** jumps to any repo, #PR, branch, group or command, typed in English or Thai.

## See the gh and git commands

Everything sigit does runs the GitHub CLI or git, and it shows you exactly what:

- **Hover** a button that runs something (Approve, Merge, Re-run, Fetch, Pop stash, Undo, Done…):
  a card shows what it does and the command, highlighted, e.g.
  `gh pr merge 12 -R owner/repo --squash --delete-branch`.
- **Before it runs**, the confirm dialog lists the commands under **Runs**, with the
  `git stash push` first when you choose Stash and continue.
- **After it runs**, the result message and the activity log show the commands that actually
  ran (a remote checkout shows whether the plain `git switch` worked or the `--track` one was
  needed), each with a copy button: run it yourself in a terminal to learn or debug.

Turn it off in **Settings → Safety → Show gh / git commands**.

## Language

**Settings → Appearance → Language**: System (follows your computer), English or ไทย, also on
the first-run welcome. The whole interface switches at once: labels, tooltips, messages, the
guide, dates (จ. 9, พ.ย.) and relative times. Git and GitHub terms (PR, commit, branch, merge,
workflow) stay in English, as Thai developers use them, and GitHub's own text (titles,
comments, names) is shown as written.

Thai is set in **Noto Sans Thai**, bundled with the app so it looks the same everywhere
(SIL Open Font License, `assets/fonts/NotoSansThai-OFL.txt`), with taller lines for Thai's
stacked vowels and tone marks.

## Safety

- Every write action is checked against fresh GitHub and git state first: protected branches,
  failing checks, missing approvals, conflicts, sensitive files, possible secrets in the diff,
  production deployments, uncommitted changes. Risky actions ask first; the riskiest need the
  repo, branch or environment name typed.
- Merges wait a few seconds with a Cancel button. Close, checkout, branch delete, pins and
  groups can be undone. Everything the app runs is in the activity log, with its commands.

## Settings

The gear at the top right (or **Ctrl+,**): Appearance (theme, language, text size, repos per
organization, section sizes), Accounts (the GitHub accounts `gh` is signed in to, and
switching), Git (your commit name and email), Safety (merge countdown, showing gh / git
commands, production environment names, sensitive paths), Notifications and watches, Data &
storage (the cache), the activity log, shortcuts, and About.

## Updates

**Settings → About → Check for updates** asks GitHub for the newest release. When there is
one, **Update now** downloads this system's build, checks it against the release's
`SHA256SUMS.txt`, swaps it in for the running copy and offers **Restart now**; your settings
stay. If sigit is installed for all users (Program Files), Windows asks for administrator
permission for the swap (macOS asks for your password). A build run from `target/` isn't
updated this way: update it from source.

Everything is stored on your computer only:

| | Where |
|---|---|
| Windows | `%APPDATA%\sigit\` |
| macOS / Linux | `$XDG_CONFIG_HOME/sigit/` or `~/.config/sigit/` |

`prefs.json` (settings, pins, groups), `repos.json` (local clones you added), `cache.json`
(last session's data, for an instant start), `activity.jsonl` (the activity log).

## Keys

**Ctrl+K** jump to anything · **Ctrl+,** settings · **F1** guide · **Ctrl+R** reload ·
**Alt+← / Alt+→** back / forward · **Ctrl+1…6** Dashboard, PRs, Actions, Environments, Commits,
Branches · **Esc** closes a window or cancels a merge countdown. Hover anything for a tooltip.

## Why it's fast

- Every `gh` and `git` call runs off the UI thread (`src/jobs.rs`); a click never waits on the
  network.
- Pages show what they loaded at once and check GitHub again in the background whenever you
  open one or come back to the window, swapping in only what changed, so you rarely press
  Refresh. An "updated 2m ago" mark beside Back / Forward says how fresh the page is. Last
  session's data is on screen in the first frame.
- Long lists build only the rows on screen. Counts come from GitHub's totals, batched into a few
  GraphQL queries.

## Development

```bash
cargo test
cargo run -- --snap shot.png --scene prs --size 1400x860    # headless still of a scene
```

Scenes (in `src/main.rs`): `inbox`, `activity`, `needs`, `prs`, `actions`, `envs`, `commits`,
`branches`, `merge`, `history-edit` (a simulated reword: it makes commit objects only, no branch or file change), `palette`, `settings`, `settings-git`, `comparisons`, `comparisons-by-contributor`, `comparisons-edit`, `branches-detail`, `guide`, `group`, `groups`, `orgs`; `light-*` for
the light theme (`light-comparisons`, `light-comparisons-by-contributor`, `light-branches-detail`); `cmd-*` for a button's command card (with `--script` moving the pointer onto
it); `setup`, `welcome`, `update-available`, and more. They use public repositories and sample
data and never write anything, except that the `history-*` scenes simulate an edit, which adds
unreferenced commit objects (no branch or file changes; `git gc` removes them). Add `SIGIT_LANG=th` to render in Thai.

**Comparisons.** Edited in Settings → Git (name, From, To; a pair like dev → staging), and changed
per repo from the gear on that repo's row on the Dashboard. Each cell counts the commits and PRs
on From that To doesn't have yet. The second number counts merged PRs, so squash merges count;
PRs are counted from up to 500 commits per comparison. Click a lane to list its PRs with labels,
a one-line summary and size; switch to **By contributor** to see who wrote them (name, company,
profile), and click a folder in **Changed code** to show only the PRs that touched it.

**New pull request.** Open one from the **New pull request** button on a repo's Pull requests page, or **Open PR** for the selected branch on Branches. Pick any branch as From and any as To; the form shows how many commits ahead the pair is, starts with the title from the last commit (**Use last commit**), and **List commits** fills the body. It blocks with a reason when there's nothing to merge, the branch isn't pushed, a PR between those two is already open (click to open it), or the title is blank. The target, draft/ready, reviewers, assignees and labels are remembered per repo, and every PR still goes through a confirm step showing the exact `gh pr create` command; edits survive a cancel or a failure.

**Edit history.** Select a commit on the Commits page. The Edit history card groups the actions by
what you want: **Go back to here** (Reset soft, mixed, hard), **Undo this commit** (Revert, Drop),
**Change this commit** (Reword…, Squash, Fixup, Move up / down) and, for a commit on another
branch, **Copy to** your branch (Cherry-pick); **Amend last commit** in the toolbar puts your
staged changes into the last commit. Each one is first **simulated**: sigit makes the new commits
as objects only (`git merge-tree`, `git commit-tree`), no branch or file changes, and the confirm
dialog shows the outcome as a graphic: a small commit graph (− leaves, + new, ~ made again,
↳ joined), a bar per changed file, and risk chips; **Why?**, **+N more** and the ⓘ button open the
details, and the commands are listed under Runs. Every action explains itself: hover a button for
what it does and its common use, or click ⓘ (in the Edit history section, or in the preview) for
a before / after picture of the commits (A–B–C–D → A–B′–C′–D′) with the same words. Theory stays
folded: the guide lists the actions by name, one opens at a time, and "How it works" folds too.
**Play** (in the preview's Commits header) animates the edit on your real commits, with the
commits under the edit as neighbours: 1 Before → 2 What changes (red leave, amber are made again,
green are new) → 3 After. Click a step to stop on it; Replay starts again. Rewriting commits already on the upstream, or on an important branch, is high
risk (type the branch name). When you confirm, a **preflight** checks again that the branch, its
upstream and the remote (`git ls-remote`, read-only) didn't change; if one did, nothing changes and
you simulate again. Then `git update-ref` moves the branch only if it is still where the simulation
saw it, and your files follow (or the branch goes back). Uncommitted changes are stashed first when
the files change. Undo (10 s) puts the branch back. sigit never pushes: after rewriting pushed
commits it shows the `git push --force-with-lease` command for you to run. Conflicts, merge commits
inside the edited range and the first commit of a history are refused. Needs git 2.40 or newer
(Reset and Amend work with any git).

**Clone folders.** Add folders in Settings → Git; they're scanned at launch and on Rescan now,
5 levels deep, skipping hidden folders, `node_modules`, `target`, `vendor`, `dist` and `build`.
Clones found are added to your repos. Scanning never writes to a clone.

**Translations.** English is written in the code and is its own key: `assets/i18n/th.tsv`
maps each English string to Thai (`English<TAB>Thai`, `\n` for a newline). Use `t("…")` (or
the views' `text`) for fixed text and `tf!` in place of `format!` for a sentence with values,
so Thai can put them where its grammar wants. `python scripts/i18n-check.py` lists strings
with no Thai yet; a debug build run with `SIGIT_LANG=th SIGIT_I18N_MISSES=misses.txt` lists
every English string it actually shows that the catalog lacks.

| File | What |
|---|---|
| `src/main.rs` | `State`, loading and caching rules, frame hooks, scenes |
| `src/gh.rs` / `src/git.rs` | every `gh` and `git` call, and the `Cmd`s actions show and run |
| `src/pulse.rs` | the Activity page's data |
| `src/compare.rs` | branch-comparison settings and the batched GitHub query |
| `src/lane.rs` | grouping a lane's PRs by folder and by author |
| `src/scan.rs` | finding clones inside a folder |
| `src/ui/compare.rs` | the Comparisons tab and the per-repo dialog |
| `src/guard.rs` / `src/risk.rs` | risk checks, confirmations, undo |
| `src/history.rs`, `src/ui/history.rs` | history editing: simulate, preflight, apply, undo; its buttons and preview |
| `src/daily.rs` | notifications, review queue, triage, groups, watches, palette |
| `src/cache.rs`, `src/config.rs`, `src/settings.rs` | the on-disk cache, preferences, the Settings window's actions |
| `src/focus.rs` | links that open the exact run or PR |
| `src/onboarding.rs` | the startup check for git, gh and a sign-in |
| `src/update.rs` | checking for, downloading and installing a new release |
| `src/i18n.rs`, `assets/i18n/th.tsv` | the interface language and the Thai catalog |
| `src/ui/cmd.rs` | highlighted command lines and the hover cards |
| `src/ui/*.rs` | the shell (custom title bar), and one file per view |
| `scripts/i18n-check.py`, `scripts/release.*` | the translation check; local release builds |

Design notes (why the safety, daily, refresh, command and translation features work the way
they do, and what was left out on purpose): `docs/design-notes.md`.

## License

MIT. See `LICENSE`.
