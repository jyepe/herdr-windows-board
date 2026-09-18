# Changelog

All notable changes to this project are documented in this file.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — MVP

Initial Windows-first board plugin for Herdr (plugin id `local.windows-board`).

### Added

- **Project scaffold** — Cargo binary crate `windows-board` with a `[profile.release]`
  `strip = true` setting and a minimal `.gitignore`.
- **Herdr CLI adapter** (`src/herdr_cli.rs`) — thin, typed wrappers over the
  `herdr` CLI (pane list/current/split/focus/close, tab/workspace/agent list,
  agent start/prompt, `open plugin pane`), resolving the binary via
  `HERDR_BIN_PATH` with a `herdr`-on-`PATH` fallback.
- **Board state** (`src/state.rs`) — cards, columns (`dispatch_enabled` /
  `DispatchConfig`), and a monotonic `revision` counter serialized to a single
  JSON file (`board-state.json`) with atomic writes and optimistic-concurrency
  detection. State directory resolution honors `HERDR_PLUGIN_STATE_DIR`, falling
  back to the platform local-data directory.
- **TUI** (`src/tui.rs`) — a `ratatui`/`crossterm` terminal UI over the board:
  column/card rendering, keyboard navigation and actions, mouse click hitmaps,
  a new/edit title input modal, status toasts, and a scope guard that restores
  the terminal on every exit path.
- **Herdr-aware actions** — an idempotent `open` command (open-or-focus the board
  overlay; with a target, open it via the Windows default handler), `card focus`
  (focus a card's linked pane), `card refresh` (clear stale pane/agent links),
  and `card add-from-pane` (capture the invoking pane's title/pane/agent).
- **Column-triggered dispatch** — moving a card into a dispatch-enabled column
  splits (or reuses) a pane, starts a configured agent, and fires the card's
  title/description (or a `{title}`/`{description}` prompt template) at it,
  best-effort with per-step failure reporting.
- **Tests** — unit tests for state persistence (atomic writes, revision
  concurrency, migration of legacy documents) and dispatch behavior (pane reuse,
  agent start, prompt templating, failure tolerance), using a `FakeHerdr` test
  double and isolated environment guards.