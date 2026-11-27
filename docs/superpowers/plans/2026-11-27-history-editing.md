# Plan: history editing

Spec: `docs/superpowers/specs/2026-11-27-history-editing-design.md`.

1. **Engine** `src/history.rs` (no UI): `Op`, `Snapshot`, `Plan`, `simulate`, `preflight`,
   `apply`, `undo`. `gh::run_full` gives the exit status and stdout together, for
   `merge-tree`'s "conflict" exit status 1, and takes stdin for `commit-tree -F -`.
   Tests with temporary repositories, one per operation, plus preflight, CAS and rollback.
2. **Guard** `src/guard.rs`: `Action::History { dir, slug, plan }`, its commands, its risk
   (pushed / protected / default branch), the preflight in `assess`, the preflight again in
   `execute`, `Undo::History`. Reload the graph and the status after.
3. **State** `src/main.rs`: `simulate_history(op)` runs `simulate` on a worker and then calls
   `request(Action::History)`. The reword dialog's state.
4. **UI**: the Edit history section in `src/ui/commits.rs`, the preview in the guard dialog
   (`src/ui/overlays.rs`), the reword dialog, and Amend in the toolbar.
5. **Text**: Thai in `assets/i18n/th.tsv`; README, design notes and the `git.rs` header get the
   new rule; scenes for the preview.
