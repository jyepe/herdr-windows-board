//! Local board state persistence.
//!
//! Cards, columns, and their ordering are stored in a single JSON file inside
//! the directory named by the `HERDR_PLUGIN_STATE_DIR` environment variable.
//! When that variable is unset (as in standalone development or testing), the
//! platform local-data directory is used instead
//! ([`directories::ProjectDirs::data_local_dir`]).
//!
//! Persistence is:
//! * **atomic** — the file is first written to a uniquely-named temporary file
//!   in the same directory and then renamed over the target, so an interrupted
//!   write can never leave a partially-written state file behind; and
//! * **optimistically concurrent** — [`BoardState::revision`] is a monotonic
//!   counter; [`save`] rejects a write when the on-disk revision no longer
//!   matches the revision the caller loaded, detecting a lost update between
//!   two concurrent invocations.
//!
//! No user-editable config is currently required, so config lives alongside
//! state rather than in a separate `HERDR_PLUGIN_CONFIG_DIR` file.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Environment variable naming the board's state directory.
pub const STATE_DIR_ENV: &str = "HERDR_PLUGIN_STATE_DIR";
/// Name of the state file within the state directory.
pub const STATE_FILE: &str = "board-state.json";

/// A single card on the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub title: String,
    /// Slug of the column the card belongs to.
    pub column: String,
    /// Position of the card within its column (lower sorts first).
    #[serde(default)]
    pub order: i64,
    /// Optional associated Herdr pane id.
    #[serde(default)]
    pub pane_id: Option<String>,
    /// Optional associated agent name.
    #[serde(default)]
    pub agent_name: Option<String>,
        /// Optional long-form description used as dispatch prompt content.
        #[serde(default)]
        pub description: String,
        /// Unix epoch seconds.
        pub created_at: u64,
    /// Unix epoch seconds.
    #[serde(default)]
    pub updated_at: u64,
}

/// Per-column agent dispatch configuration.
///
/// When a column has [`Column::dispatch_enabled`] set and carries a config,
/// moving a card into it spins up (or reuses) the configured agent and prompts
/// it with the card's title/description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchConfig {
    /// Agent kind to start (e.g. `codex`, `claude`, `copilot`).
    pub agent: String,
    /// Optional fixed prompt template. `{title}` and `{description}` are
    /// substituted from the card; when omitted, the card's title and
    /// description are submitted verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// A column on the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub slug: String,
    pub title: String,
    pub dispatch_enabled: bool,
    /// Dispatch config applied when `dispatch_enabled` is true. A column with
    /// `dispatch_enabled` but no config (or an empty `agent`) dispatches
    /// nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<DispatchConfig>,
    /// Position of the column on the board (lower sorts first).
    pub order: i64,
}

/// Root persistence model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardState {
    /// Monotonic revision counter used for optimistic concurrency.
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
        pub cards: Vec<Card>,
        #[serde(default = "default_columns")]
        pub columns: Vec<Column>,
    }

impl Default for BoardState {
    fn default() -> Self {
        Self {
            revision: 0,
            cards: Vec::new(),
            columns: default_columns(),
        }
    }
}

impl BoardState {
    /// Sort cards by column then ordering position (id as a stable tiebreak).
    pub fn sort_cards(&mut self) {
        self.cards
            .sort_by_key(|c| (c.column.clone(), c.order, c.id.clone()));
    }

    /// Sort columns by ordering position.
        ///
        /// Currently exercised by tests only; callers that render columns in
        /// order should invoke this after loading/editing state.
        #[allow(dead_code)]
        pub fn sort_columns(&mut self) {
        self.columns.sort_by_key(|c| c.order);
    }

    /// Return the ordering position to use when appending a card to `column`.
    pub fn next_order_in_column(&self, column: &str) -> i64 {
        self.cards
            .iter()
            .filter(|c| c.column == column)
            .map(|c| c.order)
            .max()
            .unwrap_or(-1)
            + 1
    }

        /// Find the card with the given id.
        pub fn card(&self, id: &str) -> Option<&Card> {
            self.cards.iter().find(|c| c.id == id)
        }

        /// Mutable access to the card with the given id.
        pub fn card_mut(&mut self, id: &str) -> Option<&mut Card> {
            self.cards.iter_mut().find(|c| c.id == id)
        }

        /// Columns ordered by `order` (slug as a stable tiebreak).
        pub fn ordered_columns(&self) -> Vec<&Column> {
            let mut columns: Vec<&Column> = self.columns.iter().collect();
            columns.sort_by_key(|c| (c.order, c.slug.as_str()));
            columns
        }

                /// Cards in `column` ordered by `order` (id as a stable tiebreak).
                pub fn cards_in_column(&self, column: &str) -> Vec<&Card> {
                    let mut cards: Vec<&Card> = self
                        .cards
                        .iter()
                        .filter(|c| c.column == column)
                        .collect();
                    cards.sort_by_key(|c| (c.order, c.id.as_str()));
                    cards
                }

        /// Append a new card to `column` and return its id.
        pub fn add_card(&mut self, title: String, column: &str) -> String {
            let id = next_id();
            let order = self.next_order_in_column(column);
            let ts = now_epoch();
            self.cards.push(Card {
                id: id.clone(),
                title,
                column: column.to_string(),
                order,
                pane_id: None,
                agent_name: None,
                                description: String::new(),
                                created_at: ts,
                                updated_at: ts,
                            });
            id
        }

        /// Move a card into another column, appending after its existing cards.
        /// (State-level primitive; the dispatch-aware path in `commands` drives it
        /// through its own logic, but tests still exercise this directly.)
        #[allow(dead_code)]
        pub fn move_card(&mut self, id: &str, column: &str) -> Result<()> {
            let order = self.next_order_in_column(column);
            let card = self
                .card_mut(id)
                .with_context(|| format!("card {id:?} not found"))?;
            card.column = column.to_string();
            card.order = order;
            card.updated_at = now_epoch();
            Ok(())
        }

        /// Remove (archive) the card with `id`; returns whether it existed.
        pub fn remove_card(&mut self, id: &str) -> bool {
            let before = self.cards.len();
            self.cards.retain(|c| c.id != id);
            self.cards.len() != before
        }

        /// Rename a card; returns whether the card existed.
        pub fn set_card_title(&mut self, id: &str, title: String) -> bool {
            match self.card_mut(id) {
                Some(card) => {
                    card.title = title;
                    card.updated_at = now_epoch();
                    true
                }
                None => false,
            }
        }

        /// Slug of the column adjacent to `slug` (`step` is -1 for left, +1 right).
        pub fn adjacent_column(&self, slug: &str, step: i64) -> Option<String> {
            let columns = self.ordered_columns();
            let pos = columns.iter().position(|c| c.slug == slug)?;
            let target = pos as i64 + step;
            if target < 0 || target >= columns.len() as i64 {
                return None;
            }
            Some(columns[target as usize].slug.clone())
        }
    }

/// Built-in columns for a brand-new board.
pub fn default_columns() -> Vec<Column> {
    vec![
        Column {
            slug: "todo".to_string(),
            title: "To Do".to_string(),
            dispatch_enabled: true,
            dispatch: Some(DispatchConfig {
                agent: "copilot".to_string(),
                prompt: None,
            }),
            order: 0,
        },
        Column {
            slug: "doing".to_string(),
            title: "Doing".to_string(),
            dispatch_enabled: true,
            dispatch: Some(DispatchConfig {
                agent: "codex".to_string(),
                prompt: None,
            }),
            order: 1,
        },
        Column {
            slug: "done".to_string(),
            title: "Done".to_string(),
            dispatch_enabled: false,
            dispatch: None,
            order: 2,
        },
    ]
}

/// Resolve the state directory, honoring `HERDR_PLUGIN_STATE_DIR` when set and
/// falling back to the platform local-data directory otherwise.
pub fn state_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var(STATE_DIR_ENV) {
        let dir = dir.trim();
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let dirs = directories::ProjectDirs::from("local", "windows", "herdr-windows-board")
        .context("failed to resolve project directories")?;
    Ok(dirs.data_local_dir().to_path_buf())
}

/// Resolve the state file path, creating its parent directory when necessary.
pub fn state_path() -> Result<PathBuf> {
    let dir = state_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create state directory {}", dir.display()))?;
    Ok(dir.join(STATE_FILE))
}

/// Load the board from the resolved state directory, or a default board when no
/// state file exists yet.
pub fn load() -> Result<BoardState> {
    load_from(&state_dir()?)
}

/// Persist `state` atomically, returning the new revision. The write is
/// rejected if the on-disk revision differs from `state.revision`.
pub fn save(state: &mut BoardState) -> Result<u64> {
    save_to(&state_dir()?, state)
}

/// Load the board from an explicit directory (kept separate so tests can point
/// at a temporary directory without touching process-wide environment state).
fn load_from(dir: &Path) -> Result<BoardState> {
    let path = dir.join(STATE_FILE);
    if !path.exists() {
        return Ok(BoardState::default());
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

/// Persist `state` to an explicit directory, returning the new revision.
fn save_to(dir: &Path, state: &mut BoardState) -> Result<u64> {
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create state directory {}", dir.display()))?;
    let path = dir.join(STATE_FILE);

    // Optimistic concurrency: only proceed if nothing else has advanced the
    // on-disk revision since we loaded `state`.
    let on_disk_revision = if path.exists() {
        load_from(dir)?.revision
    } else {
        0
    };
    if on_disk_revision != state.revision {
        bail!(
            "concurrent modification detected (on-disk revision {on_disk_revision}, \
             loaded revision {})",
            state.revision
        );
    }

    state.revision = state.revision.saturating_add(1);
    write_atomic(&path, &serialize(state)?)?;
    Ok(state.revision)
}

fn serialize(state: &BoardState) -> Result<String> {
    serde_json::to_string_pretty(state).context("failed to serialize state")
}

/// Monotonic sequence to disambiguate temporary file names within a process.
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path` atomically: write a uniquely-named temporary file
/// in the same directory, then rename it over the target.
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create directory {}", dir.display()))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(STATE_FILE);
    let seq = TEMP_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(
        ".{file_name}.tmp.{}-{nanos}-{seq}",
        std::process::id()
    ));

    fs::write(&tmp, contents)
        .with_context(|| format!("failed to write temporary file {}", tmp.display()))?;

    // `rename` replaces an existing destination atomically on both Unix and
    // Windows (MoveFileExW with MOVEFILE_REPLACE_EXISTING).
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("failed to replace {}", path.display()));
    }
    Ok(())
}

/// A collision-resistant card id derived from the current time.
pub fn next_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("card-{nanos:x}")
}

/// Current unix epoch seconds.
fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "herdr-board-test-{}-{nanos}-{seq}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn card(id: &str, column: &str, order: i64) -> Card {
        Card {
            id: id.to_string(),
            title: format!("Card {id}"),
            column: column.to_string(),
            order,
            pane_id: Some(format!("pane-{id}")),
            agent_name: Some("agent".to_string()),
                        description: String::new(),
                        created_at: 1000,
                        updated_at: 2000,
                    }
    }

    #[test]
    fn load_missing_returns_default_board() {
        let dir = temp_dir();
        let board = load_from(&dir).unwrap();
        assert_eq!(board.revision, 0);
        assert_eq!(board.columns, default_columns());
        assert!(board.cards.is_empty());
    }

    #[test]
    fn round_trips_cards_columns_and_associations() {
        let dir = temp_dir();
        let mut board = BoardState {
            revision: 0,
            cards: vec![card("1", "todo", 0), card("2", "doing", 0)],
            columns: default_columns(),
        };
        let new_revision = save_to(&dir, &mut board).unwrap();
        assert_eq!(new_revision, 1);

        let loaded = load_from(&dir).unwrap();
        assert_eq!(loaded.revision, 1);
        assert_eq!(loaded.cards, board.cards);
        assert_eq!(loaded.columns, board.columns);
        assert_eq!(loaded.cards[0].pane_id, Some("pane-1".to_string()));
        assert_eq!(loaded.cards[0].agent_name, Some("agent".to_string()));
    }

    #[test]
    fn atomic_write_leaves_no_temporary_files() {
        let dir = temp_dir();
        let mut board = BoardState::default();
        board.cards.push(card("1", "todo", 0));
        save_to(&dir, &mut board).unwrap();

        let mut entries: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        assert_eq!(entries, vec![STATE_FILE.to_string()]);
    }

    #[test]
    fn overwrites_existing_file_via_rename() {
        let dir = temp_dir();
        let mut board = BoardState::default();
        save_to(&dir, &mut board).unwrap(); // revision 0 -> 1
        board.cards.push(card("1", "todo", 0));
        let revision = save_to(&dir, &mut board).unwrap(); // 1 -> 2, overwrites
        assert_eq!(revision, 2);

        let loaded = load_from(&dir).unwrap();
        assert_eq!(loaded.revision, 2);
        assert_eq!(loaded.cards.len(), 1);
    }

    #[test]
    fn rejects_stale_revision() {
        let dir = temp_dir();
        let mut board = BoardState::default();
        save_to(&dir, &mut board).unwrap(); // advances disk to 1

        // Simulate a concurrent writer that still holds the old snapshot.
        let mut stale = BoardState {
            revision: 0,
            cards: vec![card("1", "todo", 0)],
            columns: default_columns(),
        };
        let err = save_to(&dir, &mut stale).unwrap_err();
        assert!(err.to_string().contains("concurrent modification"));
    }

    #[test]
    fn ordering_sorts_cards_and_columns() {
        let mut board = BoardState::default();
        board.cards = vec![
            card("b", "todo", 1),
            card("a", "todo", 0),
            card("c", "doing", 0),
        ];
        board.columns = vec![
            Column {
                slug: "done".into(),
                title: "Done".into(),
                dispatch_enabled: false,
                        dispatch: None,
                        order: 2,
                    },
                    Column {
                        slug: "todo".into(),
                        title: "To Do".into(),
                        dispatch_enabled: true,
                        dispatch: None,
                        order: 0,
                    },
                ];

        board.sort_cards();
        let ids: Vec<&str> = board.cards.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a", "b"]);

        board.sort_columns();
        let slugs: Vec<&str> = board.columns.iter().map(|c| c.slug.as_str()).collect();
        assert_eq!(slugs, vec!["todo", "done"]);
    }

    #[test]
    fn next_order_in_column_appends() {
        let mut board = BoardState::default();
        assert_eq!(board.next_order_in_column("todo"), 0);
        board.cards = vec![
            card("a", "todo", 0),
            card("b", "todo", 4),
            card("c", "doing", 0),
        ];
        assert_eq!(board.next_order_in_column("todo"), 5);
        assert_eq!(board.next_order_in_column("doing"), 1);
    }

    #[test]
    fn deserializes_legacy_state_without_columns() {
        let dir = temp_dir();
        let legacy = r#"{"cards":[{"id":"1","title":"hi","column":"todo","created_at":5}]}"#;
        fs::write(dir.join(STATE_FILE), legacy).unwrap();

        let board = load_from(&dir).unwrap();
        assert_eq!(board.revision, 0);
        assert_eq!(board.columns, default_columns());
        assert_eq!(board.cards.len(), 1);
        assert_eq!(board.cards[0].order, 0);
        assert_eq!(board.cards[0].updated_at, 0);
        assert_eq!(board.cards[0].pane_id, None);
    }

    #[test]
    fn state_dir_honors_env_var() {
        let custom = temp_dir().join("custom-state");
        std::env::set_var(STATE_DIR_ENV, &custom);
        let resolved = state_dir().unwrap();
        std::env::remove_var(STATE_DIR_ENV);
        assert_eq!(resolved, custom);
    }

        #[test]
        fn add_card_appends_to_column_order() {
            let mut board = BoardState::default();
            let first = board.add_card("first".into(), "todo");
            let second = board.add_card("second".into(), "todo");
            assert_ne!(first, second);
            assert_eq!(board.cards.len(), 2);
            assert_eq!(board.card(&first).unwrap().order, 0);
            assert_eq!(board.card(&second).unwrap().order, 1);
            assert_eq!(board.card(&second).unwrap().column, "todo");
        }

        #[test]
        fn move_card_changes_column_and_timestamp() {
            let mut board = BoardState::default();
            let id = board.add_card("one".into(), "todo");
            board.move_card(&id, "doing").unwrap();
            let card = board.card(&id).unwrap();
            assert_eq!(card.column, "doing");
            assert!(card.updated_at >= card.created_at);
        }

        #[test]
        fn move_card_missing_returns_error() {
            let mut board = BoardState::default();
            assert!(board.move_card("nope", "todo").is_err());
        }

        #[test]
        fn remove_card_reports_presence() {
            let mut board = BoardState::default();
            let id = board.add_card("x".into(), "todo");
            assert!(board.remove_card(&id));
            assert!(!board.remove_card(&id));
        }

        #[test]
        fn set_card_title_updates_timestamp() {
            let mut board = BoardState::default();
            let id = board.add_card("old".into(), "todo");
            assert!(board.set_card_title(&id, "new".into()));
            assert_eq!(board.card(&id).unwrap().title, "new");
            assert!(!board.set_card_title("missing", "x".into()));
        }

        #[test]
        fn adjacent_column_matches_board_order() {
            let board = BoardState::default(); // todo, doing, done
            assert_eq!(board.adjacent_column("todo", -1), None);
            assert_eq!(board.adjacent_column("todo", 1).as_deref(), Some("doing"));
            assert_eq!(board.adjacent_column("doing", 1).as_deref(), Some("done"));
            assert_eq!(board.adjacent_column("done", 1), None);
            assert_eq!(board.adjacent_column("done", -1).as_deref(), Some("doing"));
        }

                #[test]
                fn corrupt_state_file_surfaces_error() {
                    let dir = temp_dir();
                    fs::write(dir.join(STATE_FILE), "{ not valid json").unwrap();
                    let err = load_from(&dir).unwrap_err();
                    let msg = format!("{err:#}");
                    assert!(msg.contains("failed to parse"), "unexpected error: {msg}");
                }

                #[test]
                fn revision_is_monotonic_across_saves() {
                    let dir = temp_dir();
                    let mut board = BoardState::default();
                    let mut prev = 0;
                    for expected in 1..=5 {
                        let revision = save_to(&dir, &mut board).unwrap();
                        assert_eq!(revision, expected);
                        assert!(revision > prev);
                        prev = revision;
                    }
                    assert_eq!(load_from(&dir).unwrap().revision, 5);
                }

                #[test]
                fn next_id_is_unique() {
                    let mut seen = std::collections::HashSet::new();
                    for _ in 0..1000 {
                        assert!(seen.insert(next_id()), "next_id produced a duplicate");
                    }
                }

                #[test]
                fn move_card_to_nonexistent_column_places_card_by_slug() {
                    let mut board = BoardState::default();
                    let id = board.add_card("one".into(), "todo");
                    // Moving into a slug with no column still records the slug; the card
                    // is merely re-sorted under that unknown column (dispatch handled at
                    // a higher layer decides whether the column is valid).
                    board.move_card(&id, "does-not-exist").unwrap();
                    let card = board.card(&id).unwrap();
                    assert_eq!(card.column, "does-not-exist");
                    assert_eq!(
                        board.cards_in_column("does-not-exist").iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                        vec![id.as_str()]
                    );
                }

                #[test]
                fn column_ordering_tiebreak_uses_slug() {
                    let mut board = BoardState::default();
                    board.columns = vec![
                        Column {
                            slug: "zeta".into(),
                            title: "Zeta".into(),
                            dispatch_enabled: false,
                            dispatch: None,
                            order: 0,
                        },
                        Column {
                            slug: "alpha".into(),
                            title: "Alpha".into(),
                            dispatch_enabled: false,
                            dispatch: None,
                            order: 0,
                        },
                    ];
                    let slugs: Vec<&str> = board.ordered_columns().iter().map(|c| c.slug.as_str()).collect();
                    assert_eq!(slugs, vec!["alpha", "zeta"]);
                }

                #[test]
                fn card_ordering_tiebreak_uses_id() {
                    let mut board = BoardState::default();
                    board.cards = vec![
                        card("z-card", "todo", 3),
                        card("a-card", "todo", 3),
                    ];
                    let ids: Vec<&str> = board.cards_in_column("todo").iter().map(|c| c.id.as_str()).collect();
                    assert_eq!(ids, vec!["a-card", "z-card"]);

                    board.sort_cards();
                    let ids: Vec<&str> = board.cards.iter().map(|c| c.id.as_str()).collect();
                    assert_eq!(ids, vec!["a-card", "z-card"]);
                }
            }