# Packaging notes

This plugin is a plain Rust crate with a `herdr-plugin.toml` manifest at the
repo root. It is intended to be distributed as a Git repository that users
install with `herdr plugin install owner/repo`.

## Windows-only

`herdr-plugin.toml` declares `platforms = ["windows"]`. The plugin shells out to
Windows `cmd /C start` for the `open` action and targets
`target/release/windows-board.exe`, so it is not portable to Linux or macOS as
currently written. This is the main difference from the general-purpose
`herdr-board` plugin referenced below.

## Publishing once pushed to GitHub

1. Push this directory as a GitHub repository (the manifest is at the root).
2. Add the GitHub topic `herdr-plugin` so the plugin is picked up by the
   [Herdr marketplace](https://herdr.dev/plugins/) index. The plugin id
   (`local.windows-board`) must remain unique among plugins you publish.
3. Users install it with:

   ```powershell
   herdr plugin install owner/repo
   ```

   `plugin install` clones the repo, shows a preview of the manifest and the
   commands it will run, runs the manifest's build command (`cargo build
   --release`) in the checkout, and registers the plugin. Because the build
   command runs `cargo`, users need the Rust toolchain installed — document that
   prerequisite (it is already called out in the README).
4. Reinstalling from GitHub replaces the managed checkout; local development
   still uses `herdr plugin link .` (see the README).

## Model: the existing `herdr-board` plugin

The established reference for a board plugin is `nelsonPires5/herdr-board`
(Herdr's `herdr-board`), installed via:

```powershell
herdr plugin install nelsonPires5/herdr-board
```

Its manifest (`id = "herdr-board"`, `version = "0.17.0"`) declares
`platforms = ["linux", "macos"]`, a `[[build]]` step, the `board` overlay pane
running `target/release/board tui`, and a keybindable `open-board` action with a
description.

This repo follows that model — same overlay pane, same `open-board` action, same
idempotent open-or-focus launcher — but is deliberately scoped to **Windows only**:
`platforms = ["windows"]`, a Windows executable path (`windows-board.exe`), the
Windows `cmd /C start` opener, and a single `cargo build --release` build command
rather than the shell-script build steps the Unix plugin uses.

## Local development vs. install

| Command | When to use | Runs build command? |
|---|---|---|
| `herdr plugin link .` | Active development on a local checkout | No — build first with `cargo build --release` |
| `herdr plugin install owner/repo` | Installing a published repo | Yes — runs `cargo build --release` in the clone |

`herdr plugin link` registers the working directory as-is (so the release binary
must already exist); `herdr plugin install` clones and builds before registering.

## Manifest checklist (for a distributable repo)

The manifest `herdr-plugin.toml` should keep at minimum:

- `id`, `name`, `version`, `min_herdr_version` — required metadata; set
  `min_herdr_version` to the oldest Herdr that supports the features used
  (`0.9.0` currently).
- `description` — optional but recommended for discoverability.
- `platforms = ["windows"]` — required for this plugin.
- `[[build]]` — `cargo build --release`, run at install time.
- `[[panes]]` — the `board` overlay entrypoint.
- `[[actions]]` — `open-board` and `refresh-board`.

Do not invent metadata that does not apply: there is currently no explicit
author/repository field in Herdr's plugin manifest schema, so repository and
author information is conveyed by the GitHub repo itself (and its marketplace
topic) rather than by extra manifest fields.