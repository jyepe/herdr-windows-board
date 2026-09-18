# herdr-windows-board

A Windows-first kanban **board plugin for [Herdr](https://herdr.dev/)** (plugin id
`local.windows-board`). It gives you a small, persistent board of cards — organized
into columns — that lives inside a Herdr overlay pane, and it ties cards back to
Herdr's panes and coding agents so moving a card can *do* work, not just track it.

What it does:

- Persists cards, columns, and their ordering to a single JSON file in the local
  user state directory (atomic writes, optimistic concurrency).
- Renders an interactive terminal UI (built with `ratatui` + `crossterm`) in a
  Herdr overlay pane.
- Adds an idempotent `open` action that summons or focuses the board overlay and,
  with a target argument, opens a file/URL in the Windows default handler.
- Manages cards from the CLI (`card add`, `add-from-pane`, `list`, `move`,
  `remove`, `focus`, `refresh`).
- Supports **column-triggered dispatch**: moving a card into a
  dispatch-enabled column splits (or reuses) a pane, starts a configured agent,
  and fires the card's title/description (or a `{title}`/`{description}` template)
  at it.
- Ships a `doctor` subcommand that checks the environment prerequisites.

This plugin is **Windows-only** (`platforms = ["windows"]`).

## Prerequisites

- **Windows** (the plugin declares `platforms = ["windows"]`; it uses Windows
  `cmd /C start` for the `open` action and targets `windows-board.exe`).
- **Rust toolchain** — needed to build the binary. Install via
  [rustup](https://rustup.rs/) (`cargo`, `rustc`).
- **Herdr** on `PATH` (or `HERDR_BIN_PATH` pointing at the Herdr binary) for
  pane/agent integration. The TUI and board state work standalone, but dispatch,
  focus, and `open` in overlay mode require a reachable Herdr server.
- A terminal with mouse support (Windows Terminal recommended).

Run `cargo run -- doctor` to confirm `herdr`, `cargo`, and the state directory
are detected.

## Build

```powershell
cargo build --release
```

Produces `target/release/windows-board.exe`. The manifest's build command
(`cargo build --release`) is idempotent — `cargo` no-ops when nothing changed —
so it is safe for Herdr to run at install time.

## Install / link into Herdr

While developing locally, link the repo so Herdr registers the plugin from its
working directory (note: `herdr plugin link` does **not** run build commands —
build first):

```powershell
cargo build --release
herdr plugin link .
```

Confirm with:

```powershell
herdr plugin list
herdr plugin action list --plugin local.windows-board
```

The equivalent install path for a published Git repo is described in
[PACKAGING.md](PACKAGING.md).

## Connecting Herdr to the board

`herdr-plugin.toml` declares two entrypoints:

- One **pane** (`id = "board"`): an overlay pane running
  `target/release/windows-board.exe tui`.
- Two **actions**:
  - `open-board` — runs `windows-board open`, which open-or-focuses the board
    overlay (idempotent — it closes, focuses, or opens as appropriate).
  - `refresh-board` — runs `windows-board card refresh`, which clears stale
    `pane_id`/`agent_name` associations for panes/agents that no longer exist.

Open the pane directly:

```powershell
herdr plugin pane open --plugin local.windows-board --entrypoint board
```

Invoke an action directly:

```powershell
herdr plugin action invoke local.windows-board.open-board
```

Bind a key to the `open-board` action in your Herdr config
(`~/.config/herdr/config.toml`):

```toml
[[keys.command]]
key = "prefix+b"
type = "plugin_action"
command = "local.windows-board.open-board"
description = "open windows board"
```

## Board state storage

Cards, columns, and ordering are stored in a single file named `board-state.json`.

The state **directory** resolves as follows:

1. If the `HERDR_PLUGIN_STATE_DIR` environment variable is set and non-empty,
   that directory is used (Herdr injects it per-plugin when the plugin runs under
   Herdr).
2. Otherwise, the platform local-data directory is used via
   `directories::ProjectDirs::from("local", "windows", "herdr-windows-board")`
   — on Windows this is typically under
   `%LOCALAPPDATA%\windows\herdr-windows-board\data`.

To find the exact path in your environment, run `cargo run -- doctor`.

To override the location (for a separate board, or for isolated testing):

```powershell
$env:HERDR_PLUGIN_STATE_DIR = "C:\path\to\alternate\state"
cargo run -- card list
```

Persistence guarantees:

- **Atomic** — the file is written to a uniquely-named temp file in the same
  directory and renamed over the target, so an interrupted write never leaves a
  partial `board-state.json`.
- **Optimistically concurrent** — `BoardState.revision` is a monotonic counter.
  A `save` is rejected when the on-disk revision no longer matches the loaded
  revision, detecting a lost update between two concurrent invocations.

No user-editable config is currently required, so config lives alongside state in
the same file rather than a separate `HERDR_PLUGIN_CONFIG_DIR` file.

## CLI

Subcommands are parsed with `clap`:

| Command | Purpose |
|---|---|
| `tui` | Launch the interactive terminal UI (press `q` to quit). |
| `open [target]` | Summon/focus the board overlay; with `target`, open it in the Windows default handler. |
| `card add <title> [--column todo] [--target-agent <agent>]` | Add a card. `--target-agent` overrides the column's dispatch agent, and `--pre-command <cmd>` runs a command in the pane before the agent starts. |
| `card add-from-pane [title] [--column todo]` | Add a card from the invoking pane, defaulting the title to the pane title (or cwd) and storing its `pane_id`/agent. |
| `card focus <id>` | Focus the pane linked to a card (errors if unlinked or the pane is gone). |
| `card refresh` | Re-query Herdr and clear stale `pane_id`/`agent_name` associations. |
| `card list` | List all cards. |
| `card move <id> <column>` | Move a card; dispatch-enabled columns also start/reuse an agent (see below). |
| `card remove <id>` | Remove a card. |
| `doctor` | Check environment prerequisites (`herdr`, `cargo`, state dir). |

## Column-triggered dispatch

Each column carries a `dispatch_enabled` flag and an optional `dispatch` config
(the `DispatchConfig` struct: an `agent` kind such as `codex`/`claude`/`copilot`,
plus an optional `prompt` template). The default board ships four columns:

| Column | `dispatch_enabled` | Configured agent |
|---|---|---|
| To Do (`todo`) | true | `copilot` |
| In Progress (`in-progress`) | true | `codex` |
| Code Review (`code-review`) | true | `codex` |
| Done (`done`) | false | — |

Moving a card into a dispatch-enabled column with a non-empty configured `agent`:

1. moves the card (state is persisted);
2. reuses the card's existing live `pane_id`, or splits a new pane (`down`) in the
   current tab;
3. starts the configured agent kind in that pane (`herdr agent start`);
4. sends the card's title and description as a fire-and-forget prompt
   (`herdr agent prompt`) — or the `prompt` template with `{title}` and
   `{description}` substituted — unless the agent is still initializing, in which
   case the prompt is deferred; and
5. persists the resulting `pane_id` and `agent_name` back onto the card.

A column with `dispatch_enabled = true` but no config (or an empty `agent`)
dispatches nothing — the card is just moved.

Dispatch runs only on an explicit move (CLI `card move`, or TUI `h`/`l` and the
move buttons); there is no background polling. It is best-effort: a failed pane
list/split, agent start, or prompt send is recorded in the status output/toast
rather than aborting (or rolling back) the move.

Example of a dispatch-enabled column with a custom prompt template, as it would
appear in `board-state.json`:

```json
{
  "slug": "doing",
  "title": "Doing",
  "dispatch_enabled": true,
  "dispatch": {
    "agent": "codex",
    "prompt": "Work on the card titled `{title}`. Context: {description}"
  },
  "order": 1
}
```

## TUI keybindings & mouse

### Normal mode

| Key | Action |
|---|---|
| `q` / `Esc` | Quit the board. |
| `j` / `↓` | Select the next card. |
| `k` / `↑` | Select the previous card. |
| `h` / `←` | Move the selected card to the previous column. |
| `l` / `→` | Move the selected card to the next column. |
| `Tab` | Focus the next column. |
| `Shift+Tab` | Focus the previous column. |
| `n` | New card (opens the input modal). |
| `e` | Edit the selected card's title (opens the input modal). |
| `d` / `Delete` | Archive (remove) the selected card. |
| `f` / `Enter` | Focus the pane linked to the selected card. |

### Input modal (new/edit card)

| Key | Action |
|---|---|
| Printable characters | Append to the title. |
| `Enter` | Save (empty input cancels). |
| `Esc` | Cancel. |
| `Backspace` / `Ctrl+H` | Delete the last character. |

### Mouse (left button)

- **Buttons row** — `New`, `←` (move left), `→` (move right), `Edit`, `Archive`,
  `Focus` are clickable and mirror their keyboard equivalents.
- **A card** — click a card to select it.
- **A column header area** — click to focus that column (and select its first
  card).

## Troubleshooting

### Agent does not start when I move a card

- Check the column: it must have `dispatch_enabled = true` **and** a non-empty
  `dispatch.agent`. The default `done` column is not dispatch-enabled.
- Confirm Herdr is reachable: `herdr agent list` (or run `cargo run -- doctor`).
  Dispatch requires `HERDR_BIN_PATH` (injected by Herdr) or `herdr` on `PATH`.
- Dispatch is best-effort. Read the status toast/line — it names the specific
  failure (`pane list failed`, `pane split failed`, `agent start failed`, or
  `prompt failed`). The card still moves even when dispatch is skipped.
- If the agent is still initializing, the prompt is deferred but the card is
  linked; run `card refresh` later if the association looks stale.

### Board state issues (cards missing, "concurrent modification detected")

- Make sure every invocation of the binary sees the **same** state directory. If
  you run `cargo run` standalone while Herdr runs the plugin from the linked
  directory, different `HERDR_PLUGIN_STATE_DIR` values can point at different
  files. Set `HERDR_PLUGIN_STATE_DIR` explicitly to unify them.
- `concurrent modification detected` means another process advanced the revision
  between your load and save (optimistic-concurrency guard). Re-run the command;
  don't hold stale state across long-running operations.
- A corrupt/unparseable `board-state.json` (hand-edited) will fail on load with a
  `failed to parse` error. Delete or repair the file to reset to a default board.
- Saves are atomic — there should be no leftover `.*.tmp.*` files in the state
  directory; if you see them after a crash they are safe to delete.

### Terminal rendering issues (garbled screen, missing borders, no colors)

- Prefer Windows Terminal (enables mouse reporting and color). If cards/buttons
  don't respond to clicks, confirm mouse reporting is on and the terminal
  supports it.
- The `open` action and dispatch prompter shell out via `cmd`/`cargo`; if the
  screen looks broken after quitting, the TUI's scope guard restores the terminal
  on all exit paths (including panics) — relaunching a broken terminal is
  otherwise the fastest fix.
- If the window is too narrow, columns may render at minimum width and cards may
  be truncated; widen the terminal.

## Dependencies

- `clap` — argument parsing
- `serde` / `serde_json` — serialization
- `directories` — local state data directory resolution
- `ratatui` + `crossterm` — terminal UI
- `std::process` — process execution (shell `open`, prerequisite checks)
- `anyhow` — error handling

## Manifest

`herdr-plugin.toml` declares `platforms = ["windows"]`, a release build command,
an overlay pane running `windows-board tui`, and two actions (`open-board`,
`refresh-board`). See [Packaging](PACKAGING.md) for publication notes.