//! Adapter for the `herdr` CLI.
//!
//! All interaction with Herdr goes through the `herdr` binary (resolved from
//! `HERDR_BIN_PATH`, falling back to `herdr` on `PATH`). Commands are executed
//! as subprocesses and JSON stdout is parsed into typed structs. No raw
//! sockets or named pipes are used.

// This module is a library-style adapter exposed for use by the board UI and
// command handlers; not every item is consumed yet.
#![allow(dead_code)]

use std::process::{Command, Output};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Resolve the Herdr binary path.
///
/// `HERDR_BIN_PATH` wins when set and non-empty; otherwise we rely on a
/// `herdr` executable discoverable via `PATH`.
pub fn herdr_bin() -> String {
    match std::env::var("HERDR_BIN_PATH") {
        Ok(path) if !path.trim().is_empty() => path.trim().to_string(),
        _ => "herdr".to_string(),
    }
}

/// Run `herdr` with the given arguments, capturing stdout and stderr.
///
/// Non-zero exit statuses are converted into an error that includes both the
/// exit code and the sanitized stderr output for actionable diagnostics.
fn run(args: &[&str]) -> Result<Output> {
    let bin = herdr_bin();
    let output = Command::new(&bin)
        .args(args)
        .output()
        .with_context(|| {
            if std::env::var("HERDR_BIN_PATH").is_ok() {
                format!("failed to execute herdr binary at {bin:?}")
            } else {
                format!("failed to execute `herdr` (is it installed and on PATH?)")
            }
        })?;

    if !output.status.success() {
        let code = match output.status.code() {
            Some(c) => c.to_string(),
            None => "terminated by signal".to_string(),
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let detail = if stderr.is_empty() {
            "no stderr output".to_string()
        } else {
            format!("stderr: {stderr}")
        };
        bail!("herdr {} exited with code {code} ({detail})", args.join(" "));
    }

    Ok(output)
}

/// Run `herdr` and parse its JSON stdout into `T`.
fn run_json<T: for<'de> Deserialize<'de>>(args: &[&str]) -> Result<T> {
    let output = run(args)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .with_context(|| format!("failed to parse herdr JSON output for `herdr {}`", args.join(" ")))
}

/// The outer `{"id": ..., "result": ...}` envelope returned by herdr queries.
#[derive(Deserialize)]
struct Envelope<T> {
    result: T,
}

// ---------------------------------------------------------------------------
// Typed result payloads (mirrors the `result` object of each list command).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WorkspaceResult {
    workspaces: Vec<Workspace>,
}

#[derive(Deserialize)]
struct PaneResult {
    panes: Vec<Pane>,
}

#[derive(Deserialize)]
struct TabResult {
    tabs: Vec<Tab>,
}

#[derive(Deserialize)]
struct AgentResult {
    agents: Vec<Agent>,
}

#[derive(Deserialize)]
struct CurrentPaneResult {
    pane: Pane,
}

#[derive(Deserialize)]
struct PaneSplitResult {
    pane: Pane,
}

// ---------------------------------------------------------------------------
// Public domain structs.
// ---------------------------------------------------------------------------

/// A Herdr workspace.
#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    pub label: String,
    pub number: u64,
    pub active_tab_id: Option<String>,
    pub agent_status: Option<String>,
    pub focused: bool,
    pub pane_count: u64,
    pub tab_count: u64,
}

/// A Herdr tab.
#[derive(Debug, Clone, Deserialize)]
pub struct Tab {
    pub tab_id: String,
    pub workspace_id: String,
    pub label: String,
    pub number: u64,
    pub pane_count: u64,
    pub agent_status: Option<String>,
    pub focused: bool,
}

/// A Herdr pane.
#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub terminal_id: Option<String>,
    pub terminal_title: Option<String>,
    pub terminal_title_stripped: Option<String>,
    pub cwd: Option<String>,
    pub agent_status: Option<String>,
    pub focused: bool,
    pub revision: u64,
    pub scroll: Option<Scroll>,
    pub agent: Option<String>,
    pub agent_session: Option<AgentSession>,
}

/// A Herdr agent (an agent terminal attached to a pane).
#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub agent: String,
    pub agent_session: Option<AgentSession>,
    pub agent_status: Option<String>,
    pub cwd: Option<String>,
    pub focused: bool,
    pub pane_id: Option<String>,
    pub revision: u64,
    pub state_change_seq: Option<u64>,
    pub tab_id: Option<String>,
    pub terminal_id: Option<String>,
    pub terminal_title: Option<String>,
    pub terminal_title_stripped: Option<String>,
    pub workspace_id: String,
}

/// Identity of a running agent session.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentSession {
    pub agent: String,
    pub kind: String,
    pub source: String,
    pub value: String,
}

/// Scrollbar position reported for a pane.
#[derive(Debug, Clone, Deserialize)]
pub struct Scroll {
    pub max_offset_from_bottom: i64,
    pub offset_from_bottom: i64,
    pub viewport_rows: u64,
}

// ---------------------------------------------------------------------------
// Public query functions.
// ---------------------------------------------------------------------------

/// List all workspaces.
pub fn list_workspaces() -> Result<Vec<Workspace>> {
    let envelope = run_json::<Envelope<WorkspaceResult>>(&["workspace", "list"])?;
    Ok(envelope.result.workspaces)
}

/// List panes, optionally scoped to a single workspace.
pub fn list_panes(workspace: Option<&str>) -> Result<Vec<Pane>> {
    let mut args: Vec<&str> = vec!["pane", "list"];
    if let Some(id) = workspace {
        args.push("--workspace");
        args.push(id);
    }
    let envelope = run_json::<Envelope<PaneResult>>(&args)?;
    Ok(envelope.result.panes)
}

/// List tabs, optionally scoped to a single workspace.
pub fn list_tabs(workspace: Option<&str>) -> Result<Vec<Tab>> {
    let mut args: Vec<&str> = vec!["tab", "list"];
    if let Some(id) = workspace {
        args.push("--workspace");
        args.push(id);
    }
    let envelope = run_json::<Envelope<TabResult>>(&args)?;
    Ok(envelope.result.tabs)
}

/// List all agents.
pub fn list_agents() -> Result<Vec<Agent>> {
    let envelope = run_json::<Envelope<AgentResult>>(&["agent", "list"])?;
    Ok(envelope.result.agents)
}

/// Return the currently focused pane.
pub fn current_pane() -> Result<Pane> {
    let envelope = run_json::<Envelope<CurrentPaneResult>>(&["pane", "current"])?;
    Ok(envelope.result.pane)
}

/// Focus an arbitrary pane by id.
///
/// Herdr 0.9 exposes no generic `pane focus <pane_id>`; the id-targeted focus
/// surfaces are the agent surface (which also moves pane focus and requires a
/// live agent in the pane) and the plugin-pane surface. Try the agent surface
/// first — the common case for pane-linked cards — then fall back to the
/// plugin-pane surface (as used by the board overlay pane).
pub fn focus_pane(pane_id: &str) -> Result<()> {
    let agent_err = match run(&["agent", "focus", pane_id]) {
        Ok(_) => return Ok(()),
        Err(e) => e,
    };
    match run(&["plugin", "pane", "focus", pane_id]) {
        Ok(_) => Ok(()),
        Err(plugin_err) => bail!(
            "could not focus pane {pane_id:?} \
             (agent focus: {agent_err:#}; plugin pane focus: {plugin_err:#})"
        ),
    }
}

/// Focus a plugin-owned pane (e.g. the board overlay pane) by id.
pub fn focus_plugin_pane(pane_id: &str) -> Result<()> {
    run(&["plugin", "pane", "focus", pane_id]).map(|_| ())
}

/// Close a pane by id.
pub fn close_pane(pane_id: &str) -> Result<()> {
    run(&["pane", "close", pane_id]).map(|_| ())
}

/// Result of starting an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStart {
    /// The agent is ready and can accept prompts.
    Ready,
    /// The agent was created but is still blocked during startup (Herdr's
    /// `agent_not_ready`); it cannot accept prompts yet.
    Initializing,
}

/// Open a plugin pane with the given plugin/entrypoint/placement, focused.
pub fn open_plugin_pane(plugin: &str, entrypoint: &str, placement: &str) -> Result<()> {
    run(&[
        "plugin",
        "pane",
        "open",
        "--plugin",
        plugin,
        "--entrypoint",
        entrypoint,
        "--placement",
        placement,
        "--focus",
    ])
    .map(|_| ())
}

/// Split the panes of the current workspace/tab, returning the new pane.
///
/// When `source` is given, the split is rooted at that pane's tab; otherwise
/// Herdr uses the currently focused pane/tab. `--no-focus` keeps user focus on
/// the source pane so the spawned agent starts in the background.
pub fn split_pane(source: Option<&str>, direction: &str) -> Result<Pane> {
    let mut args: Vec<&str> = vec!["pane", "split"];
    if let Some(id) = source {
        args.push(id);
    }
    args.push("--direction");
    args.push(direction);
    args.push("--no-focus");
    let envelope = run_json::<Envelope<PaneSplitResult>>(&args)?;
    Ok(envelope.result.pane)
}

/// Start an agent of `kind` in `pane_id`, naming it `name`.
///
/// Herdr reports `agent_not_ready` (exit code 1) when the agent is created but
/// still blocked during startup; that is surfaced as `Initializing` rather than
/// an error so callers can persist the pane/agent link and skip the prompt.
pub fn agent_start(name: &str, kind: &str, pane_id: &str) -> Result<AgentStart> {
    let bin = herdr_bin();
    let output = Command::new(&bin)
        .args(["agent", "start", name, "--kind", kind, "--pane", pane_id])
        .output()
        .with_context(|| {
            if std::env::var("HERDR_BIN_PATH").is_ok() {
                format!("failed to execute herdr binary at {bin:?}")
            } else {
                format!("failed to execute `herdr` (is it installed and on PATH?)")
            }
        })?;

    if output.status.success() {
        return Ok(AgentStart::Ready);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("agent_not_ready") {
        return Ok(AgentStart::Initializing);
    }

    let code = match output.status.code() {
        Some(c) => c.to_string(),
        None => "terminated by signal".to_string(),
    };
    let stderr = stderr.trim();
    let detail = if stderr.is_empty() {
        "no stderr output".to_string()
    } else {
        format!("stderr: {stderr}")
    };
    bail!(
        "herdr agent start {name} --kind {kind} --pane {pane_id} exited with code {code} ({detail})"
    )
}

/// Send a fire-and-forget prompt to `target` (a live agent name or the pane id
/// hosting it). No `--wait`, so this returns as soon as the prompt is queued.
pub fn agent_prompt(target: &str, text: &str) -> Result<()> {
    run(&["agent", "prompt", target, text]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workspace_list() {
        let raw = r#"{"id":"cli:workspace:list","result":{"type":"workspace_list","workspaces":[{"active_tab_id":"w8:t1","agent_status":"unknown","focused":false,"label":"ecare360","number":1,"pane_count":2,"tab_count":2,"workspace_id":"w8"}]}}"#;
        let envelope: Envelope<WorkspaceResult> = serde_json::from_str(raw).unwrap();
        let workspaces = envelope.result.workspaces;
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].workspace_id, "w8");
        assert_eq!(workspaces[0].pane_count, 2);
        assert_eq!(workspaces[0].active_tab_id.as_deref(), Some("w8:t1"));
    }

    #[test]
    fn parses_agent_list() {
        let raw = r#"{"id":"cli:agent:list","result":{"type":"agent_list","agents":[{"agent":"copilot","agent_session":{"agent":"copilot","kind":"id","source":"herdr:copilot","value":"abc"},"agent_status":"idle","cwd":"C:\\tmp","focused":false,"pane_id":"w9:p1","revision":10,"state_change_seq":121,"tab_id":"w9:t1","terminal_id":"term_x","terminal_title":"t","terminal_title_stripped":"t","workspace_id":"w9"}]}}"#;
        let envelope: Envelope<AgentResult> = serde_json::from_str(raw).unwrap();
        let agents = envelope.result.agents;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent, "copilot");
        assert_eq!(agents[0].agent_session.as_ref().unwrap().value, "abc");
        assert_eq!(agents[0].state_change_seq, Some(121));
    }

            /// Live smoke test against a reachable `herdr`; ignored by default since
            /// it depends on a running Herdr server.
            #[test]
            #[ignore]
            fn live_queries_against_herdr() {
                let workspaces = list_workspaces().expect("workspace list");
                for w in &workspaces {
                    let panes = list_panes(Some(&w.workspace_id)).expect("pane list");
                    let tabs = list_tabs(Some(&w.workspace_id)).expect("tab list");
                    assert_eq!(panes.iter().all(|p| p.workspace_id == w.workspace_id), true);
                    assert_eq!(tabs.iter().all(|t| t.workspace_id == w.workspace_id), true);
                }
                let _agents = list_agents().expect("agent list");
            }

                        // -----------------------------------------------------------------
                        // Integration-style tests against a `.cmd` fake `herdr` binary,
                        // pointed to via `HERDR_BIN_PATH`. No live server is touched.
                        // -----------------------------------------------------------------

                        use crate::test_support;

                        #[test]
                        fn list_workspaces_via_fake_binary() {
                                                    let fake = test_support::FakeHerdr::new();
                            fake.write(
                                "workspace-list.json",
                                test_support::workspaces_envelope(&["w1", "w2"]),
                            );

                            let workspaces = list_workspaces().unwrap();
                            assert_eq!(workspaces.len(), 2);
                            assert_eq!(workspaces[0].workspace_id, "w1");
                            assert_eq!(workspaces[1].workspace_id, "w2");
                            assert_eq!(workspaces[1].label, "ws-w2");
                        }

                        #[test]
                        fn list_panes_via_fake_binary() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("pane-list.json", test_support::panes_envelope(&["p1", "p2"]));

                            let panes = list_panes(None).unwrap();
                            assert_eq!(panes.len(), 2);
                            assert_eq!(panes[0].pane_id, "p1");
                            assert_eq!(panes[1].pane_id, "p2");
                            assert_eq!(panes[0].workspace_id, "w1");
                        }

                        #[test]
                        fn list_panes_scoped_to_workspace() {
                            let fake = test_support::FakeHerdr::new();
                            // The fake routes `pane list --workspace <id>` to this fixture.
                            fake.write(
                                "pane-list-workspace.json",
                                test_support::panes_envelope(&["w1:p1"]),
                            );

                            let panes = list_panes(Some("w1")).unwrap();
                            assert_eq!(panes.len(), 1);
                            assert_eq!(panes[0].pane_id, "w1:p1");
                        }

                        #[test]
                        fn list_tabs_via_fake_binary() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("tab-list.json", test_support::tabs_envelope(&["t1", "t2"]));

                            let tabs = list_tabs(Some("w1")).unwrap();
                            assert_eq!(tabs.len(), 2);
                            assert_eq!(tabs[0].tab_id, "t1");
                            assert_eq!(tabs[1].tab_id, "t2");
                        }

                        #[test]
                        fn list_agents_via_fake_binary() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("agent-list.json", test_support::agents_envelope(&["copilot", "codex"]));

                            let agents = list_agents().unwrap();
                            assert_eq!(agents.len(), 2);
                            assert_eq!(agents[0].agent, "copilot");
                            assert_eq!(agents[1].agent, "codex");
                        }

                        #[test]
                        fn current_pane_via_fake_binary() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("pane-current.json", test_support::pane_envelope("p9"));

                            let pane = current_pane().unwrap();
                            assert_eq!(pane.pane_id, "p9");
                        }

                        #[test]
                        fn split_pane_via_fake_binary() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("pane-split.json", test_support::pane_envelope("p-new"));

                            let pane = split_pane(None, "down").unwrap();
                            assert_eq!(pane.pane_id, "p-new");
                        }

                        #[test]
                        fn agent_start_ready_via_fake_binary() {
                                                    let _fake = test_support::FakeHerdr::new();
                            // No fixture: fake `agent start` exits 0, signaling `Ready`.

                            assert_eq!(agent_start("a", "copilot", "p1").unwrap(), AgentStart::Ready);
                        }

                        #[test]
                        fn agent_start_initializing_when_agent_not_ready() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("agent-start.stderr", "agent_not_ready: still booting\n");

                            assert_eq!(
                                agent_start("a", "copilot", "p1").unwrap(),
                                AgentStart::Initializing
                            );
                        }

                        #[test]
                        fn agent_start_error_surfaces_stderr() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("agent-start.stderr", "boom: unknown kind\n");

                            let err = agent_start("a", "bogus", "p1").unwrap_err();
                            let msg = format!("{err:#}");
                            assert!(msg.contains("boom"), "unexpected error: {msg}");
                            assert!(msg.contains("exited with code 1"), "unexpected error: {msg}");
                        }

                        #[test]
                        fn agent_prompt_forwards_target_and_text() {
                            let fake = test_support::FakeHerdr::new();

                            agent_prompt("p1", "do the thing").unwrap();

                            let args = std::fs::read_to_string(fake.path("agent-prompt-args.txt")).unwrap();
                            assert!(args.contains("agent prompt"), "args: {args}");
                            assert!(args.contains("p1"), "args: {args}");
                            assert!(args.contains("do the thing"), "args: {args}");
                        }

                        #[test]
                        fn focus_pane_prefers_agent_focus() {
                            let _fake = test_support::FakeHerdr::new();
                            // Default fake: `agent focus` exits 0.
                            focus_pane("p1").unwrap();
                        }

                        #[test]
                        fn focus_pane_falls_back_to_plugin_pane() {
                            let _fake = test_support::FakeHerdr::new();
                            _fake.write("agent-focus.fail", "");
                            // `plugin pane focus` succeeds (no plugin.fail).
                            focus_pane("p1").unwrap();
                        }

                        #[test]
                        fn focus_pane_errors_when_both_surfaces_fail() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("agent-focus.fail", "");
                            fake.write("agent-focus.stderr", "no agent in pane\n");
                            fake.write("plugin.fail", "");

                            let err = focus_pane("p1").unwrap_err();
                            let msg = format!("{err:#}");
                            assert!(msg.contains("could not focus pane"), "unexpected error: {msg}");
                        }

                        #[test]
                        fn list_panes_error_surfaces_failure() {
                            let fake = test_support::FakeHerdr::new();
                            fake.write("pane-list.fail", "");
                            fake.write("pane-list.stderr", "server unreachable\n");

                            let err = list_panes(None).unwrap_err();
                            let msg = format!("{err:#}");
                            assert!(msg.contains("server unreachable"), "unexpected error: {msg}");
                        }
                    }