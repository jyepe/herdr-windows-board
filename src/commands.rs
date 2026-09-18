//! Implementations of the CLI subcommand behaviors.

use std::process::Command as ProcCommand;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::herdr_cli;
use crate::state::{self, BoardState, Card};
use crate::CardCommand;

/// Pane title used to recognize the board overlay pane.
const BOARD_TITLE: &str = "Windows Board";

/// Plugin / entrypoint identifiers used to open the board overlay pane.
const PLUGIN_ID: &str = "local.windows-board";
const PLUGIN_ENTRYPOINT: &str = "board";

/// `open`: an idempotent overlay opener/focuser for the board, or open an
/// explicit target in the Windows shell.
pub fn open(target: Option<String>) -> Result<()> {
    match target {
        Some(t) => open_in_shell(&t),
        None => open_board(),
    }
}

/// Idempotently summon the board as an overlay pane:
///
/// * if a pane titled [`BOARD_TITLE`] exists and is focused, close it;
/// * if it exists but is not focused, focus it;
/// * otherwise open it as a focused overlay pane.
///
/// When Herdr is not reachable (e.g. standalone `cargo run` without a server),
/// fall back to launching the TUI in-process.
fn open_board() -> Result<()> {
    match herdr_cli::list_panes(None) {
        Ok(panes) => {
            match panes.iter().find(|p| board_title_matches(p)) {
                Some(pane) if pane.focused => {
                    herdr_cli::close_pane(&pane.pane_id)
                        .with_context(|| format!("failed to close board pane {}", pane.pane_id))?;
                    println!("closed board pane {}", pane.pane_id);
                }
                Some(pane) => {
                                    herdr_cli::focus_plugin_pane(&pane.pane_id)
                        .with_context(|| format!("failed to focus board pane {}", pane.pane_id))?;
                    println!("focused board pane {}", pane.pane_id);
                }
                None => {
                    herdr_cli::open_plugin_pane(PLUGIN_ID, PLUGIN_ENTRYPOINT, "overlay")?;
                    println!("opened board overlay");
                }
            }
            Ok(())
        }
        Err(_) => crate::tui::run(),
    }
}

/// Whether a pane carries the board's known title.
fn board_title_matches(pane: &herdr_cli::Pane) -> bool {
    pane.terminal_title.as_deref() == Some(BOARD_TITLE)
        || pane.terminal_title_stripped.as_deref() == Some(BOARD_TITLE)
}

fn open_in_shell(target: &str) -> Result<()> {
    let status = ProcCommand::new("cmd")
        .args(["/C", "start", "", target])
        .status()
        .with_context(|| format!("failed to open {target:?}"))?;
    if !status.success() {
        bail!("open command failed with exit status {status}");
    }
    Ok(())
}

/// `card`: add/list/move/remove board cards persisted to local state.
pub fn card(cmd: CardCommand) -> Result<()> {
    match cmd {
        CardCommand::Add { title, column, target_agent, pre_command } => {
            let mut board = state::load()?;
            let order = board.next_order_in_column(&column);
            let ts = now()?;
            board.cards.push(Card {
                id: state::next_id(),
                title,
                column: column.clone(),
                order,
                pane_id: None,
                agent_name: None,
                target_agent,
                        pre_command,
                description: String::new(),
                created_at: ts,
                updated_at: ts,
            });
            state::save(&mut board)?;
            println!("added card to '{column}'");
        }
                CardCommand::AddFromPane { title, column } => {
                    let mut board = state::load()?;
                    let (resolved_title, pane_id, agent_name) = match invoking_pane() {
                        Some(pane) => {
                            let ptitle = pane
                                .terminal_title
                                .clone()
                                .or_else(|| pane.terminal_title_stripped.clone())
                                .or_else(|| pane.cwd.clone())
                                .unwrap_or_else(|| pane.pane_id.clone());
                            (title.unwrap_or(ptitle), Some(pane.pane_id.clone()), pane.agent.clone())
                        }
                        None => match title {
                            Some(t) => (t, None, None),
                            None => bail!(
                                "no title given and no focused/invoking pane detected \
                                 (run from a Herdr pane or pass an explicit title)"
                            ),
                        },
                    };
                    let order = board.next_order_in_column(&column);
                    let ts = now()?;
                    board.cards.push(Card {
                        id: state::next_id(),
                        title: resolved_title,
                        column: column.clone(),
                        order,
                        pane_id,
                        agent_name,
                        target_agent: None,
                        pre_command: None,
                        description: String::new(),
                        created_at: ts,
                        updated_at: ts,
                    });
                    state::save(&mut board)?;
                    println!("added card to '{column}'");
                }
                CardCommand::Focus { id } => {
                    let board = state::load()?;
                    let card = board
                        .cards
                        .iter()
                        .find(|c| c.id == id)
                        .with_context(|| format!("card {id:?} not found"))?;
                    let pane_id = card
                        .pane_id
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("card {id:?} has no linked pane"))?;
                    let panes = herdr_cli::list_panes(None)
                        .with_context(|| "failed to query herdr panes")?;
                    if !panes.iter().any(|p| p.pane_id == pane_id) {
                        bail!("linked pane {pane_id:?} for card {id:?} no longer exists");
                    }
                    herdr_cli::focus_pane(pane_id)
                        .with_context(|| format!("failed to focus pane {pane_id:?}"))?;
                    println!("focused pane {pane_id}");
                }
                CardCommand::Refresh => {
                    let mut board = state::load()?;
                    let panes = herdr_cli::list_panes(None)
                        .with_context(|| "failed to query herdr panes")?;
                    let agents = herdr_cli::list_agents().unwrap_or_default();
                    let live_panes: std::collections::HashSet<&str> =
                        panes.iter().map(|p| p.pane_id.as_str()).collect();
                    let mut changed = Vec::new();
                    for card in board.cards.iter_mut() {
                        let linked = card
                            .pane_id
                            .as_deref()
                            .map(|id| live_panes.contains(id))
                            .unwrap_or(false);
                        if !linked {
                            if card.pane_id.is_some() || card.agent_name.is_some() {
                                changed.push(card.id.clone());
                            }
                            card.pane_id = None;
                            card.agent_name = None;
                        } else if let Some(name) = card.agent_name.as_deref() {
                            // Keep agent_name only if it still corresponds to an agent.
                            if !agents.iter().any(|a| a.agent == name) {
                                card.agent_name = None;
                                changed.push(card.id.clone());
                            }
                        }
                    }
                    if changed.is_empty() {
                        println!("nothing to refresh");
                    } else {
                        state::save(&mut board)?;
                        for id in &changed {
                            println!("cleared association on card {id}");
                        }
                    }
                }
        CardCommand::List => {
                    let mut board = state::load()?;
                    board.sort_cards();
                    if board.cards.is_empty() {
                        println!("no cards");
                    }
                    for card in &board.cards {
                        println!("[{}] #{} {} ({})", card.column, card.order, card.id, card.title);
                    }
                }
        CardCommand::Move { id, column } => {
                    let mut board = state::load()?;
                    let status = move_card_dispatch(&mut board, &id, &column)?;
                    state::save(&mut board)?;
                    println!("{status}");
                }
        CardCommand::Remove { id } => {
            let mut board = state::load()?;
            let before = board.cards.len();
            board.cards.retain(|c| c.id != id);
            if board.cards.len() == before {
                bail!("card {id:?} not found");
            }
                        state::save(&mut board)?;
            println!("removed {id}");
        }
    }
    Ok(())
}

/// `doctor`: verify the environment needed to build and run the plugin.
pub fn doctor() -> Result<()> {
    let mut ok = true;

    check_cmd("herdr", "--version", &mut ok);

    // `cargo` may not be on PATH until a freshly-installed rustup shell is
    // restarted; fall back to the standard rustup install location.
    let cargo_found = match ProcCommand::new("cargo").arg("--version").output() {
        Ok(out) if out.status.success() => {
            println!("ok   cargo    {}", String::from_utf8_lossy(&out.stdout).trim());
            true
        }
        Ok(_) | Err(_) => {
            if let Ok(user) = std::env::var("USERPROFILE") {
                let fallback = std::path::Path::new(&user).join(".cargo").join("bin").join("cargo.exe");
                if fallback.exists() {
                    println!("ok   cargo    {}", fallback.display());
                    true
                } else {
                    println!("fail cargo    not found on PATH or in ~/.cargo/bin");
                    false
                }
            } else {
                println!("fail cargo    not found on PATH");
                false
            }
        }
    };
    if !cargo_found {
        ok = false;
    }

    match state::state_path() {
        Ok(p) => println!("ok   state     {}", p.display()),
        Err(e) => {
            ok = false;
            println!("fail state     {e:#}");
        }
    }

    if !ok {
        bail!("one or more checks failed");
    }
    Ok(())
}

fn check_cmd(bin: &str, arg: &str, ok: &mut bool) {
    match ProcCommand::new(bin).arg(arg).output() {
        Ok(out) if out.status.success() => {
            println!("ok   {bin:<8} {}", String::from_utf8_lossy(&out.stdout).trim());
        }
        Ok(out) => {
            *ok = false;
            println!("fail {bin:<8} exited with {}", out.status);
        }
        Err(e) => {
            *ok = false;
            println!("fail {bin:<8} not found ({e})");
        }
    }
}

/// Resolve the currently focused/invoking pane, if any.
///
/// Preference order:
/// 1. `HERDR_PANE_ID` — if set, fetch that pane from `herdr pane list`.
/// 2. `herdr pane current` — the currently focused pane.
/// 3. `HERDR_PLUGIN_CONTEXT_JSON` — a plugin invocation context (best effort).
///
/// Returns `Ok(None)` when no pane can be determined (e.g. running outside
/// Herdr), and only returns an error for genuine lookup failures.
fn invoking_pane() -> Option<herdr_cli::Pane> {
    if let Ok(id) = std::env::var("HERDR_PANE_ID") {
        let id = id.trim().to_string();
        if !id.is_empty() {
            if let Ok(panes) = herdr_cli::list_panes(None) {
                if let Some(pane) = panes.into_iter().find(|p| p.pane_id == id) {
                    return Some(pane);
                }
            }
        }
    }

    if let Ok(pane) = herdr_cli::current_pane() {
        return Some(pane);
    }

    parse_context_json().and_then(|ctx| ctx.pane_id).and_then(|id| {
        herdr_cli::list_panes(None).ok()?.into_iter().find(|p| p.pane_id == id)
    })
}

/// Minimal parsed shape of `HERDR_PLUGIN_CONTEXT_JSON`.
#[derive(serde::Deserialize)]
struct PluginContext {
    #[serde(default)]
    pane_id: Option<String>,
}

fn parse_context_json() -> Option<PluginContext> {
    let raw = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok()?;
    serde_json::from_str(&raw).ok()
}

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?
        .as_secs())
}

/// Move a card into a column and, when that column is dispatch-enabled with a
/// configured agent, spin up (or reuse) the card's linked pane, start the agent
/// there, and fire off the card prompt. Shared by the CLI `card move` command
/// and the TUI's move actions.
///
/// The card is always moved in-memory; callers persist the board. Dispatch is
/// best-effort: pane split, agent start, and prompt send are each individually
/// tolerated (a failure is recorded in the returned status rather than aborting
/// the move). Returns a human-readable status naming the moved card and any
/// started pane/agent, or the dispatch problem encountered.
pub fn move_card_dispatch(board: &mut BoardState, id: &str, column: &str) -> Result<String> {
        // Resolve dispatch config and the card's current links up front, before
        // mutating the board, to avoid overlapping borrows.
        enum Action {
            Plain,
            Dispatch {
                kind: String,
                prompt_template: Option<String>,
                split_direction: Option<String>,
                pre_command: Option<String>,
            },
        }

        let action = match board.columns.iter().find(|c| c.slug == column) {
            Some(col) if col.dispatch_enabled => match col.dispatch.as_ref() {
                Some(d) if !d.agent.is_empty() => Action::Dispatch {
                    kind: d.agent.clone(),
                    prompt_template: d.prompt.clone(),
                    split_direction: d.split_direction.clone(),
                    pre_command: d.pre_command.clone(),
                },
                _ => Action::Plain,
            },
            _ => Action::Plain,
        };

        // Capture the card links (title/description/pane) before mutation.
        let (title, description, existing_pane, target_agent) = match board.card(id) {
            Some(c) => (c.title.clone(), c.description.clone(), c.pane_id.clone(), c.target_agent.clone()),
            None => bail!("card {id:?} not found"),
        };

        // Move the card.
        let order = board.next_order_in_column(column);
        let card = board
            .cards
            .iter_mut()
            .find(|c| c.id == id)
            .with_context(|| format!("card {id:?} not found"))?;
        card.column = column.to_string();
        card.order = order;
        card.updated_at = now()?;

        let Action::Dispatch {
            mut kind,
            prompt_template,
            split_direction,
            pre_command,
        } = action else {
            return Ok(format!("moved {id} to '{column}'"));
        };

        if let Some(ta) = target_agent {
            if !ta.trim().is_empty() {
                kind = ta.trim().to_string();
            }
        }

        // Reuse the card's existing pane if it is still live; otherwise split a new
        // pane rooted at the current tab.
        let panes = match herdr_cli::list_panes(None) {
            Ok(p) => p,
            Err(err) => {
                return Ok(format!(
                    "moved {id} to '{column}'; dispatch skipped (pane list failed: {err:#})"
                ))
            }
        };
        let live_pane = existing_pane
            .as_deref()
            .filter(|pid| panes.iter().any(|p| p.pane_id == *pid));

        let pane_id = match live_pane {
            Some(pid) => pid.to_string(),
            None => {
                let is_tab = split_direction.as_deref() == Some("tab");
                if is_tab {
                    match herdr_cli::create_tab() {
                        Ok(pane) => pane.pane_id,
                        Err(err) => {
                            return Ok(format!(
                                "moved {id} to '{column}'; dispatch skipped (tab create failed: {err:#})"
                            ))
                        }
                    }
                } else {
                    let direction = split_direction.as_deref().unwrap_or("down");
                    match herdr_cli::split_pane(None, direction) {
                        Ok(pane) => pane.pane_id,
                        Err(err) => {
                            return Ok(format!(
                                "moved {id} to '{column}'; dispatch skipped (pane split failed: {err:#})"
                            ))
                        }
                    }
                }
            }
        };

        if let Some(cmd) = pre_command {
            if !cmd.trim().is_empty() {
                if let Err(err) = herdr_cli::run_in_pane(&pane_id, &cmd) {
                    return Ok(format!(
                        "moved {id} to '{column}'; dispatch skipped (pre-command failed: {err:#})"
                    ));
                }
            }
        }

        match herdr_cli::agent_start(&kind, &kind, &pane_id) {
            Ok(herdr_cli::AgentStart::Ready) => {
                let prompt = match prompt_template.as_deref().filter(|p| !p.is_empty()) {
                    Some(template) => template
                        .replace("{title}", &title)
                        .replace("{description}", &description),
                    None => {
                        let mut parts = vec![title.trim().to_string()];
                        if !description.trim().is_empty() {
                            parts.push(description.trim().to_string());
                        }
                        parts.join("\n")
                    }
                };
                if !prompt.is_empty() {
                    if let Err(err) = herdr_cli::agent_prompt(&pane_id, &prompt) {
                        return Ok(format!(
                            "moved {id} to '{column}'; started {kind} agent in pane {pane_id} \
                             but prompt failed ({err:#})"
                        ));
                    }
                }
                apply_link(board, id, &pane_id, &kind)?;
                Ok(format!(
                    "moved {id} to '{column}'; started {kind} agent in pane {pane_id}"
                ))
            }
            Ok(herdr_cli::AgentStart::Initializing) => {
                apply_link(board, id, &pane_id, &kind)?;
                Ok(format!(
                    "moved {id} to '{column}'; started {kind} agent in pane {pane_id} \
                     (still initializing, prompt deferred)"
                ))
            }
            Err(err) => Ok(format!(
                "moved {id} to '{column}'; dispatch skipped (agent start failed: {err:#})"
            )),
        }
}

/// Persist the dispatched pane/agent link back onto the moved card.
fn apply_link(board: &mut BoardState, id: &str, pane_id: &str, kind: &str) -> Result<()> {
        match board.card_mut(id) {
            Some(c) => {
                c.pane_id = Some(pane_id.to_string());
                c.agent_name = Some(kind.to_string());
                c.updated_at = now()?;
            }
            None => bail!("card {id:?} vanished during dispatch"),
        }
        Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Column, DispatchConfig};
    use crate::test_support::{self, FakeHerdr};

    /// A board whose columns: `dispatch` is dispatch-enabled with `copilot`,
    /// `plain` is not dispatch-enabled.
    fn board_with_columns() -> BoardState {
        let mut board = BoardState::default();
        board.columns = vec![
            Column {
                slug: "dispatch".into(),
                title: "Dispatch".into(),
                dispatch_enabled: true,
                dispatch: Some(DispatchConfig {
                    agent: "copilot".into(),
                    prompt: Some("Act on: {title} / {description}".into()),
                                    split_direction: None,
                                    pre_command: None,
                                }),
                order: 0,
            },
            Column {
                slug: "plain".into(),
                title: "Plain".into(),
                dispatch_enabled: false,
                dispatch: None,
                order: 1,
            },
        ];
        board
    }

    #[test]
    fn move_to_plain_column_is_verbatim_no_herdr() {
        let mut board = board_with_columns();
        let id = board.add_card("task one".into(), "dispatch");
        let status = move_card_dispatch(&mut board, &id, "plain").unwrap();

        assert_eq!(status, format!("moved {id} to 'plain'"));
        assert_eq!(board.card(&id).unwrap().column, "plain");
        assert_eq!(board.card(&id).unwrap().pane_id, None);
        assert_eq!(board.card(&id).unwrap().agent_name, None);
    }

    #[test]
    fn move_to_dispatch_column_starts_agent_and_links_pane() {
        let _fake = FakeHerdr::new();
        _fake.write("pane-list.json", test_support::panes_envelope(&[]));
        _fake.write("pane-split.json", test_support::pane_envelope("p-new"));

        let mut board = board_with_columns();
        let id = board.add_card("deploy service".into(), "plain");
        let status = move_card_dispatch(&mut board, &id, "dispatch").unwrap();

        assert!(status.contains("started copilot agent in pane p-new"), "status: {status}");
        assert_eq!(board.card(&id).unwrap().column, "dispatch");
        assert_eq!(board.card(&id).unwrap().pane_id.as_deref(), Some("p-new"));
        assert_eq!(board.card(&id).unwrap().agent_name.as_deref(), Some("copilot"));
    }

    #[test]
    fn dispatch_reuses_already_linked_live_pane() {
        let _fake = FakeHerdr::new();
        _fake.write("pane-list.json", test_support::panes_envelope(&["p-linked"]));

        let mut board = board_with_columns();
        let id = board.add_card("reuse me".into(), "plain");
        board.card_mut(&id).unwrap().pane_id = Some("p-linked".into());

        let status = move_card_dispatch(&mut board, &id, "dispatch").unwrap();
        assert!(status.contains("in pane p-linked"), "status: {status}");
        assert_eq!(board.card(&id).unwrap().pane_id.as_deref(), Some("p-linked"));
    }

    #[test]
    fn pane_list_failure_skips_dispatch_but_still_moves() {
        let _fake = FakeHerdr::new();
        _fake.write("pane-list.fail", "");
        _fake.write("pane-list.stderr", "bogus server\n");

        let mut board = board_with_columns();
        let id = board.add_card("x".into(), "plain");
        let status = move_card_dispatch(&mut board, &id, "dispatch").unwrap();

        assert!(status.contains("moved"), "status: {status}");
        assert!(status.contains("dispatch skipped (pane list failed"), "status: {status}");
        assert_eq!(board.card(&id).unwrap().column, "dispatch");
        assert_eq!(board.card(&id).unwrap().pane_id, None);
    }

    #[test]
    fn pane_split_failure_skips_dispatch_but_still_moves() {
        let _fake = FakeHerdr::new();
        _fake.write("pane-list.json", test_support::panes_envelope(&[]));
        _fake.write("pane-split.fail", "");
        _fake.write("pane-split.stderr", "no room\n");

        let mut board = board_with_columns();
        let id = board.add_card("x".into(), "plain");
        let status = move_card_dispatch(&mut board, &id, "dispatch").unwrap();

        assert!(status.contains("dispatch skipped (pane split failed"), "status: {status}");
        assert_eq!(board.card(&id).unwrap().column, "dispatch");
        assert_eq!(board.card(&id).unwrap().pane_id, None);
    }

    #[test]
    fn agent_start_failure_skips_dispatch_but_still_moves() {
        let _fake = FakeHerdr::new();
        _fake.write("pane-list.json", test_support::panes_envelope(&[]));
        _fake.write("pane-split.json", test_support::pane_envelope("p-new"));
        _fake.write("agent-start.stderr", "unknown kind bacon\n");

        let mut board = board_with_columns();
        let id = board.add_card("x".into(), "plain");
        let status = move_card_dispatch(&mut board, &id, "dispatch").unwrap();

        assert!(status.contains("dispatch skipped (agent start failed"), "status: {status}");
        assert_eq!(board.card(&id).unwrap().column, "dispatch");
        assert_eq!(board.card(&id).unwrap().pane_id, None);
    }

    #[test]
    fn agent_initializing_defers_prompt_but_links_pane() {
        let _fake = FakeHerdr::new();
        _fake.write("pane-list.json", test_support::panes_envelope(&[]));
        _fake.write("pane-split.json", test_support::pane_envelope("p-new"));
        _fake.write("agent-start.stderr", "agent_not_ready still booting\n");

        let mut board = board_with_columns();
        let id = board.add_card("boot".into(), "plain");
        let status = move_card_dispatch(&mut board, &id, "dispatch").unwrap();

        assert!(status.contains("still initializing, prompt deferred"), "status: {status}");
        assert_eq!(board.card(&id).unwrap().pane_id.as_deref(), Some("p-new"));
        assert_eq!(board.card(&id).unwrap().agent_name.as_deref(), Some("copilot"));
    }

    #[test]
    fn dispatch_prompt_substitutes_title_and_description() {
        let _fake = FakeHerdr::new();
        _fake.write("pane-list.json", test_support::panes_envelope(&[]));
        _fake.write("pane-split.json", test_support::pane_envelope("p-new"));

        let mut board = board_with_columns();
        let id = board.add_card("Build the thing".into(), "plain");
        board.card_mut(&id).unwrap().description = "carefully".into();

        let status = move_card_dispatch(&mut board, &id, "dispatch").unwrap();
        assert!(status.contains("started copilot agent"), "status: {status}");

        let args = std::fs::read_to_string(_fake.path("agent-prompt-args.txt")).unwrap();
        assert!(
            args.contains("Act on: Build the thing / carefully"),
            "prompt args: {args}"
        );
    }

    #[test]
    fn dispatch_without_template_falls_back_to_title_plus_description() {
        let mut board = board_with_columns();
        board.columns[0].dispatch.as_mut().unwrap().prompt = None;

        let _fake = FakeHerdr::new();
        _fake.write("pane-list.json", test_support::panes_envelope(&[]));
        _fake.write("pane-split.json", test_support::pane_envelope("p-new"));

            // Empty description keeps the fallback prompt a single line, which the
            // fake `.cmd` captures faithfully (embedded newlines would not).
            let id = board.add_card("Just a title".into(), "plain");
        move_card_dispatch(&mut board, &id, "dispatch").unwrap();

            let args = std::fs::read_to_string(_fake.path("agent-prompt-args.txt")).unwrap();
            assert!(args.contains("Just a title"), "prompt args: {args}");
    }

    #[test]
    fn move_to_nonexistent_column_is_plain_and_still_moves() {
        let mut board = board_with_columns();
        let id = board.add_card("orphan".into(), "plain");
        let status = move_card_dispatch(&mut board, &id, "nope").unwrap();

        assert_eq!(status, format!("moved {id} to 'nope'"));
        assert_eq!(board.card(&id).unwrap().column, "nope");
    }

    #[test]
    fn dispatch_column_without_config_is_plain() {
        let mut board = board_with_columns();
        board.columns[0].dispatch = None; // enabled but no config

        let id = board.add_card("cfg".into(), "plain");
        let status = move_card_dispatch(&mut board, &id, "dispatch").unwrap();
        assert_eq!(status, format!("moved {id} to 'dispatch'"));
        assert_eq!(board.card(&id).unwrap().pane_id, None);
    }

    #[test]
    fn move_nonexistent_card_is_an_error() {
        let mut board = board_with_columns();
        let err = move_card_dispatch(&mut board, "no-such-card", "plain").unwrap_err();
        assert!(format!("{err:#}").contains("not found"), "err: {err:#}");
    }
}