//! Shared helpers for tests: environment isolation and a fake `herdr` binary.
//!
//! * [`EnvGuard`] serializes (and restores) process-wide environment-variable
//!   mutation so tests that set `HERDR_BIN_PATH`, `HERDR_PLUGIN_STATE_DIR`, etc.
//!   cannot race each other when the test harness runs them on parallel threads.
//! * [`FakeHerdr`] points `HERDR_BIN_PATH` at a generated `.cmd` batch script
//!   that serves canned JSON fixtures (and canned failures) without ever
//!   reaching a real Herdr server.

#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// Global lock serializing all environment-variable mutation across tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that restores a set of environment variables when dropped.
pub struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<String>)>,
}

impl EnvGuard {
    pub fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        EnvGuard { _lock: lock, saved: Vec::new() }
    }

    /// Set `key=value`, remembering the previous value so it can be restored.
    pub fn set(&mut self, key: &str, value: &str) {
        self.saved.push((key.to_string(), std::env::var(key).ok()));
        std::env::set_var(key, value);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.saved.iter().rev() {
            match prev {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// A unique temporary directory, removed on drop.
pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "herdr-board-{label}-{}-{nanos}-{seq}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    pub fn write(&self, name: &str, contents: &str) {
        fs::write(self.path.join(name), contents).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A fake `herdr` executable (a `.cmd` batch script) that prints canned JSON
/// fixture files. Fixtures live in the script's own directory; each test writes
/// only the fixtures its scenario needs.
pub struct FakeHerdr {
    pub dir: TempDir,
    guard: EnvGuard,
}

impl FakeHerdr {
    pub fn new() -> Self {
        let dir = TempDir::new("fake-herdr");
        let script = FAKE_SCRIPT.replace('\n', "\r\n");
        dir.write("fake-herdr.cmd", &script);

        let bin = dir.path.join("fake-herdr.cmd");
        let mut guard = EnvGuard::new();
        guard.set("HERDR_BIN_PATH", bin.to_str().unwrap());
        FakeHerdr { dir, guard }
    }

    /// Write a fixture file into the fake's directory.
    pub fn write(&self, name: &str, contents: impl AsRef<str>) {
        self.dir.write(name, contents.as_ref());
    }

    /// Full path to a (possibly not-yet-created) file in the fake's directory.
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path.join(name)
    }

    /// Set an additional environment variable using the already-held lock.
    pub fn set_env(&mut self, key: &str, value: &str) {
        self.guard.set(key, value);
    }
}

/// Route `herdr` arguments to fixture files. Exit codes and stderr are driven
/// by the presence of `.fail` / `.stderr` marker files.
const FAKE_SCRIPT: &str = r#"@echo off
setlocal
set "DIR=%~dp0"

if "%1"=="workspace" goto workspace
if "%1"=="pane" goto pane
if "%1"=="tab" goto tab
if "%1"=="agent" goto agent
if "%1"=="plugin" goto plugin
exit /b 0

:workspace
if exist "%DIR%workspace.fail" goto workspace_fail
if exist "%DIR%workspace-list.json" type "%DIR%workspace-list.json"
exit /b 0
:workspace_fail
if exist "%DIR%workspace.stderr" type "%DIR%workspace.stderr" 1>&2
exit /b 1

:pane
if "%2"=="list" goto pane_list
if "%2"=="current" goto pane_current
if "%2"=="split" goto pane_split
if "%2"=="run" goto pane_run
if "%2"=="close" exit /b 0
exit /b 0

:pane_run
> "%DIR%pane-run-args.txt" echo %*
exit /b 0

:pane_list
if exist "%DIR%pane-list.fail" goto pane_list_fail
if "%3"=="--workspace" goto pane_list_ws
if exist "%DIR%pane-list.json" type "%DIR%pane-list.json"
exit /b 0
:pane_list_ws
if exist "%DIR%pane-list-workspace.json" type "%DIR%pane-list-workspace.json"
exit /b 0
:pane_list_fail
if exist "%DIR%pane-list.stderr" type "%DIR%pane-list.stderr" 1>&2
exit /b 1

:pane_current
if exist "%DIR%pane-current.json" type "%DIR%pane-current.json"
exit /b 0

:pane_split
if exist "%DIR%pane-split.fail" goto pane_split_fail
if exist "%DIR%pane-split.json" type "%DIR%pane-split.json"
exit /b 0
:pane_split_fail
if exist "%DIR%pane-split.stderr" type "%DIR%pane-split.stderr" 1>&2
exit /b 1

:tab
if exist "%DIR%tab-list.json" type "%DIR%tab-list.json"
exit /b 0

:agent
if "%2"=="list" goto agent_list
if "%2"=="start" goto agent_start
if "%2"=="focus" goto agent_focus
if "%2"=="prompt" goto agent_prompt
exit /b 0

:agent_list
if exist "%DIR%agent-list.json" type "%DIR%agent-list.json"
exit /b 0

:agent_start
if exist "%DIR%agent-start.stderr" goto agent_start_fail
exit /b 0
:agent_start_fail
type "%DIR%agent-start.stderr" 1>&2
exit /b 1

:agent_focus
if exist "%DIR%agent-focus.fail" goto agent_focus_fail
exit /b 0
:agent_focus_fail
if exist "%DIR%agent-focus.stderr" type "%DIR%agent-focus.stderr" 1>&2
exit /b 1

:agent_prompt
if exist "%DIR%agent-prompt.fail" exit /b 1
> "%DIR%agent-prompt-args.txt" echo %*
exit /b 0

:plugin
if exist "%DIR%plugin.fail" exit /b 1
exit /b 0
"#;

// ---------------------------------------------------------------------------
// JSON fixture builders shared by herdr_cli and commands tests.
// ---------------------------------------------------------------------------

/// Wrap a `result` payload in the `{"id": ..., "result": ...}` envelope herdr
/// returns. `result` must already be a serialized object/array.
pub fn envelope(result: &str) -> String {
    format!(r#"{{"id":"fake:test","result":{result}}}"#)
}

/// A minimal, valid Herdr `Pane` object.
pub fn pane_obj(pane_id: &str) -> String {
    format!(
        r#"{{"pane_id":"{pane_id}","workspace_id":"w1","tab_id":"t1","terminal_id":null,"terminal_title":null,"terminal_title_stripped":null,"cwd":null,"agent_status":null,"focused":false,"revision":1,"scroll":null,"agent":null,"agent_session":null}}"#
    )
}

/// Envelope for `pane list` (and `tab list` of panes): `{"panes":[...]}`.
pub fn panes_envelope(ids: &[&str]) -> String {
    let objs = ids.iter().map(|id| pane_obj(id)).collect::<Vec<_>>().join(",");
    envelope(&format!(r#"{{"panes":[{objs}]}}"#))
}

/// Envelope for `pane current` / `pane split`: `{"pane":{...}}`.
pub fn pane_envelope(pane_id: &str) -> String {
    envelope(&format!(r#"{{"pane":{}}}"#, pane_obj(pane_id)))
}

/// A minimal, valid Herdr `Workspace` object.
pub fn workspace_obj(workspace_id: &str) -> String {
    format!(
        r#"{{"workspace_id":"{workspace_id}","label":"ws-{workspace_id}","number":1,"active_tab_id":null,"agent_status":null,"focused":false,"pane_count":0,"tab_count":0}}"#
    )
}

/// Envelope for `workspace list`: `{"workspaces":[...]}`.
pub fn workspaces_envelope(ids: &[&str]) -> String {
    let objs = ids.iter().map(|id| workspace_obj(id)).collect::<Vec<_>>().join(",");
    envelope(&format!(r#"{{"workspaces":[{objs}]}}"#))
}

/// A minimal, valid Herdr `Tab` object.
pub fn tab_obj(tab_id: &str) -> String {
    format!(
        r#"{{"tab_id":"{tab_id}","workspace_id":"w1","label":"tab-{tab_id}","number":1,"pane_count":0,"agent_status":null,"focused":false}}"#
    )
}

/// Envelope for `tab list`: `{"tabs":[...]}`.
pub fn tabs_envelope(ids: &[&str]) -> String {
    let objs = ids.iter().map(|id| tab_obj(id)).collect::<Vec<_>>().join(",");
    envelope(&format!(r#"{{"tabs":[{objs}]}}"#))
}

/// A minimal, valid Herdr `Agent` object.
pub fn agent_obj(agent: &str) -> String {
    format!(
        r#"{{"agent":"{agent}","agent_session":null,"agent_status":null,"cwd":null,"focused":false,"pane_id":null,"revision":1,"state_change_seq":null,"tab_id":null,"terminal_id":null,"terminal_title":null,"terminal_title_stripped":null,"workspace_id":"w1"}}"#
    )
}

/// Envelope for `agent list`: `{"agents":[...]}`.
pub fn agents_envelope(agents: &[&str]) -> String {
    let objs = agents.iter().map(|a| agent_obj(a)).collect::<Vec<_>>().join(",");
    envelope(&format!(r#"{{"agents":[{objs}]}}"#))
}