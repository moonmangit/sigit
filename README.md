# sigit

A fast desktop app for everyday GitHub and git work: pull requests, Actions, environments and
history, across all your repositories and organizations. Built in Rust on
[Rune](https://github.com/anomalyco/rune), a GPU-rendered native UI.

**It reads and reviews; it never commits or pushes.**

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
| **Inbox → Activity** (default) | Your tracked repos over a range (this week by default; today, 7 or 30 days, this month, custom): open PRs and which are **ready for review (all checks passed)**, Actions health (exact pass rate, what's red on the default branch now), commits, lines, deployments, releases, contributors, and a section per repo | Scope to a group, manage what's tracked |
| **Inbox → Needs you** | Review queue (ready PRs first, oldest wait first), your open PRs, assigned issues, notifications | Done / Snooze, J/K/Enter/A/C |
| **Pull requests** | Open / merged / closed with checks, review state, diff size; detail with description, reviews, files and diff, checks | Approve, request changes, comment, merge (squash / merge / rebase), close, check out |
| **Actions** | Runs with duration and result, jobs and steps on one timeline, failing log lines | Re-run failed or all, cancel, watch |
| **Environments** | What's deployed where, a deployment timeline | Open the deployed commit |
| **Commits** (local clone) | Commit graph with lanes, branch and tag chips | Commit detail with diff |
| **Branches** (local clone) | Local and remote, ahead/behind, stashes, clean-up of merged or gone branches | Checkout, fetch, fast-forward pull |
| **Groups** | A dashboard for repos you group yourself, across owners | |

The sidebar lists every repo you can reach, grouped by owner, with your **pins** and **groups**
on top. Drag to reorder, fold sections, and give organizations your own colour and icon.

Side-by-side sections have a handle between them: drag to resize, double-click to reset.

## Safety

- Every write action is checked against fresh GitHub and git state first: protected branches,
  failing checks, missing approvals, conflicts, sensitive files, possible secrets in the diff,
  production deployments, uncommitted changes. Risky actions ask first; the riskiest need the
  repo, branch or environment name typed.
- Merges wait a few seconds with a Cancel button. Close, checkout, branch delete, pins and
  groups can be undone. Everything the app runs is in the activity log.

## Settings

The gear at the top right (or **Ctrl+,**): Appearance (theme, text size, repos per
organization), Accounts (the GitHub accounts `gh` is signed in to, and switching), Git (your
commit name and email), Safety (production environment names, sensitive paths, merge
countdown), Notifications and watches, Data & storage (the cache), the activity log, shortcuts.

Everything is stored on your computer only:

| | Where |
|---|---|
| Windows | `%APPDATA%\sigit\` |
| macOS / Linux | `$XDG_CONFIG_HOME/sigit/` or `~/.config/sigit/` |

`prefs.json` (settings, pins, groups), `repos.json` (local clones you added), `cache.json`
(last session's data, for an instant start), `activity.jsonl` (the activity log).

## Keys

**Ctrl+K** jump to anything · **Ctrl+,** settings · **F1** guide · **Ctrl+R** reload ·
**Alt+← / Alt+→** back / forward · **Ctrl+1…6** Inbox, PRs, Actions, Environments, Commits,
Branches · **Esc** closes a window or cancels a merge countdown. Hover anything for a tooltip.

## Why it's fast

- Every `gh` and `git` call runs off the UI thread (`src/jobs.rs`); a click never waits on the
  network.
- Pages keep what they loaded and show it at once; opening a page reloads only what's gone stale,
  with an "updated 2m ago" mark beside Back / Forward. Last session's data is on screen in the
  first frame.
- Long lists build only the rows on screen. Counts come from GitHub's totals, batched into a few
  GraphQL queries.

## Development

```bash
cargo test
cargo run -- --snap shot.png --scene prs --size 1400x860    # headless still of a scene
```

Scenes (in `src/main.rs`): `inbox`, `activity`, `needs`, `prs`, `actions`, `envs`, `commits`,
`branches`, `merge`, `palette`, `settings`, `guide`, `group`, `groups`, `orgs`, and more. They
use public repositories as sample data and never write anything.

| File | What |
|---|---|
| `src/main.rs` | `State`, loading and caching rules, frame hooks, scenes |
| `src/gh.rs` / `src/git.rs` | every `gh` and `git` call |
| `src/pulse.rs` | the Activity page's data |
| `src/guard.rs` / `src/risk.rs` | risk checks, confirmations, undo |
| `src/daily.rs` | notifications, review queue, triage, groups, watches, palette |
| `src/cache.rs`, `src/config.rs`, `src/settings.rs` | the on-disk cache, preferences, the Settings window's actions |
| `src/ui/*.rs` | the shell (custom title bar), and one file per view |

Design notes for the safety and daily features: `docs/design-notes.md`.

## License

MIT. See `LICENSE`.
