# Manual Testing Checklist

End-to-end validation that requires a live Herdr server and a real terminal.
Run these in order; each step builds on the previous one.

## 0. Prerequisites

- A running Herdr server (`herdr` on `PATH`, or `HERDR_BIN_PATH` pointing at it).
- Rust toolchain (`cargo`) on `PATH`.
- A terminal with mouse support (Windows Terminal recommended).
- The plugin linked into Herdr (see step 1).

> Tip: use `cargo run -- doctor` to confirm `herdr`, `cargo`, and the state
> directory are detected before starting.

## 1. Link the plugin

```powershell
herdr plugin link .
```

Confirm Herdr reports the plugin `local.windows-board` linked successfully.

## 2. Build

```powershell
cargo build --release
```

The build must complete with no errors. Re-link if the binary path changed.

## 3. Open the board

Trigger the configured `open-board` action (see `herdr-plugin.toml`), or run
`cargo run -- open`. The board TUI (title "Windows Board") should appear with
the default columns **To Do / Doing / Done**.

- Press `q` to quit and return to a usable terminal (no garbled screen, cursor
  visible, alternate screen left).

## 4. Create a card (keyboard)

- Press the **New** key/button (`n`). Enter a title and accept.
- Confirm a new card appears in the target column (default "To Do").
- Quit and inspect state:

  ```powershell
  cargo run -- card list
  ```

  The new card should be listed with a `card-*` id.

## 5. Create a card (mouse)

- Re-open the board and click the **n New** button in a column.
- Enter a title and confirm. The card should appear in that column.

## 6. Move a card (keyboard)

- Select a card and press `h`/`l` (or the left/right move button) to move it
  between adjacent columns.
- Confirm the card moves and the board re-renders in the new column.

## 7. Move a card (mouse)

- Click a card's move-right/move-left button (or the button in the header).
- Confirm the move matches the keyboard behavior.

## 8. Remove a card

- Archive a card via its archive button or the archive key.
- Confirm it disappears and `cargo run -- card list` no longer lists it.

## 9. Persistence across restarts

- Create several cards and move them to different columns.
- Quit the TUI, then re-open it.
- Confirm card titles, columns, and ordering are unchanged.
- Optionally, inspect the on-disk file (shown by `cargo run -- doctor`) to see
  `cards`, `columns`, and the `revision` counter. Since saves are atomic, there
  should be no leftover `*.tmp.*` files in the state directory.

## 10. Dispatch-enabled columns start an agent

Default board: **To Do** and **Doing** are dispatch-enabled (`copilot` and
`codex` respectively); **Done** is not.

- Create a card in **Done** (non-dispatch), then move it to **To Do** (or **Doing**).
- Confirm:
  - the card lands in the target column;
  - a new pane is split and the configured agent starts in it;
  - the card's TUI/status shows a "started `<agent>` agent in pane `<id>`" toast;
  - `cargo run -- card list` (after `card refresh` if needed) reflects the
    linked `pane_id`/`agent_name`.
- Move a card into **Done**: no agent should start (plain move, no new pane).
- Move a card back to a dispatch column that already has a live linked pane:
  the existing pane should be reused rather than a new split.

## 11. Focus a linked pane

- With a dispatched card selected, trigger **Focus Pane** (button/key).
- Herdr should focus the pane hosting the card's agent.

## 12. Failure tolerance (best-effort dispatch)

- Temporarily point `HERDR_BIN_PATH` at a script that exits non-zero on `pane
  split` (or stop the Herdr server), then move a card into a dispatch column.
- The card must still move; the status indicates `dispatch skipped` rather than
  aborting the move.

## 13. Cleanup

- Remove test cards, close the board, and quit cleanly.
- Confirm the cursor is visible and the terminal is usable afterward (no raw
  mode left enabled).
- Optionally delete the state file to reset to a fresh board.