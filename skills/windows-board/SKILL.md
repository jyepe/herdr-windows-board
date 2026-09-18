---
name: windows-board
description: Use the Windows Board plugin for Herdr to manage a kanban board of cards, columns, and pane/agent dispatch from the CLI or terminal UI. Use when the user asks about the board, cards, columns, moving/dispatching work, or wiring a coding agent to a Herdr pane.
---

# Windows Board

A Windows-first kanban **board plugin for Herdr** (plugin id `local.windows-board`).
It persists cards and columns to a single `board-state.json` file and links cards
back to Herdr panes and coding agents, so moving a card can *do* work (spawn an
agent in a split pane and prompt it), not just track it.

Use the `herdr` skill for the multiplexer itself (panes/tabs/workspaces). This
skill covers the board plugin's own CLI and behavior.

## Binary and environment

- Binary: `windows-board` (built as `target/release/windows-board.exe`).
- State directory resolution (highest precedence first):
  1. `HERDR_PLUGIN_STATE_DIR` (Herdr injects this per-plugin when run under Herdr).
  2. Herdr's per-plugin directory, derived even when the env var is absent:
     `%LOCALAPPDATA%\herdr\plugins\local.windows-board`.
  - The state file is always named `board-state.json`.
  - A legacy copy at `%LOCALAPPDATA%\windows\herdr-windows-board\data` is
    migrated into the canonical directory automatically if it is missing there.
- The TUI live-reloads from disk, so cards added/moved by an agent in another
  pane appear without restarting.
- Herdr binary resolution: `HERDR_BIN_PATH` if set, else `herdr` on `PATH`.
- Invoking-pane detection reads `HERDR_PANE_ID`, then `herdr pane current`, then
  `HERDR_PLUGIN_CONTEXT_JSON`.

Find the exact state path with `windows-board doctor`.

## Default layout

A brand-new board (no pre-existing `board-state.json`) has four columns:

| Column | slug | dispatch_enabled | agent |
|---|---|---|---|
| To Do | `todo` | true | `copilot` |
| In Progress | `in-progress` | true | `codex` |
| Code Review | `code-review` | true | `codex` |
| Done | `done` | false | — |

Columns carry an optional `dispatch` config (`agent`, `prompt` template,
`split_direction`, `pre_command`). A card may also carry a `target_agent`
(overrides the column's agent) and a `pre_command` (command run in the pane
before the agent starts).

## CLI

```text
windows-board tui                                    # interactive terminal UI (q to quit)
windows-board open [target]                          # open-or-focus the board overlay; with target, open in Windows default handler
windows-board card list                              # list all cards
windows-board card add <title> [--column todo] [--target-agent <agent>] [--pre-command <cmd>]
windows-board card add-from-pane [title] [--column todo]
windows-board card move <id> <column>                # move card; dispatch-enabled columns may spawn an agent
windows-board card remove <id>
windows-board card focus <id>                        # focus the pane linked to a card
windows-board card refresh                           # clear stale pane/agent links
windows-board doctor                                 # check environment prerequisites
```

## Dispatch semantics

Moving a card into a `dispatch_enabled` column whose configured `agent` is
non-empty (or whose card has a `target_agent`):

1. Moves the card and persists state.
2. Reuses the card's existing live `pane_id`, otherwise splits a new pane
   (`down` by default, or `tab`, or the column's `split_direction`).
3. Runs the card/column `pre_command` in the pane (before the agent starts), if set.
4. Starts the agent (`herdr agent start --kind <agent> --pane <pane>`).
5. Prompts the agent with the card title + description, or the `prompt` template
   with `{title}` / `{description}` substituted. If the agent is still
   initializing, the prompt is deferred but the link is recorded.
6. Persists `pane_id` / `agent_name` back onto the card.

Dispatch is **best-effort**: a failure at any step (pane list, split, pre-command,
agent start, prompt) leaves the card moved and reports the problem in the status
output rather than failing the whole command. A column with `dispatch_enabled`
but no config/agent is a plain move (no agent).

## State file

`board-state.json` contains `revision` (monotonic, for optimistic concurrency),
`cards[]`, and `columns[]`. Writes are atomic (temp file + rename).

Reset the board to the default layout by deleting `board-state.json` (find it via
`windows-board doctor`). Legacy state files without `columns` are backfilled with
the default layout on load.