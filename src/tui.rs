//! ratatui + crossterm terminal UI for the board.
//!
//! Renders the board's columns and cards, exposes a keyboard- and mouse-driven
//! set of editing actions, and persists every mutation atomically through
//! [`crate::state`]. Terminal raw mode and the alternate screen are entered on
//! start and restored on every exit path (including panics) via [`TermGuard`].
//!
//! Every action is reachable by keyboard alone; on-screen buttons are a
//! mouse-only convenience layered on top of the same actions.

use std::{io, time::Duration};

use anyhow::{Context, Result};
use crossterm::{
    cursor::Show,
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
            KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        },
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::state::{self, BoardState, Card};

/// Key/button-triggered actions shared by keyboard and mouse input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    NewCard,
    MoveLeft,
    MoveRight,
    Edit,
    Archive,
    FocusPane,
}

/// The label shown on each on-screen button for `action`.
impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::NewCard => " n New ",
            Action::MoveLeft => " h \u{2190} ",
            Action::MoveRight => " l \u{2192} ",
            Action::Edit => " e Edit ",
            Action::Archive => " d Archive ",
            Action::FocusPane => " f Focus ",
        }
    }
}

/// Regions collected by [`App::draw`] so mouse clicks can be resolved.
#[derive(Default)]
struct Hitmap {
    buttons: Vec<(Action, Rect)>,
    cards: Vec<(String, Rect)>,
    columns: Vec<(String, Rect)>,
}

/// Which input mode the UI is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    NewCard,
    EditCard,
}

/// Application state for the interactive loop.
struct App {
    board: BoardState,
    /// Id of the selected card, when one is selected.
    selected_card: Option<String>,
    /// Slug of the focused column (target for new cards).
    focused_column: String,
    mode: Mode,
    input: String,
    status: Option<String>,
    quit: bool,
}

impl App {
    fn new(board: BoardState) -> Self {
        let columns = board.ordered_columns();
        let focused_column = columns
            .first()
            .map(|c| c.slug.clone())
            .unwrap_or_default();
        let selected_card = board
            .cards_in_column(&focused_column)
            .first()
            .map(|c| c.id.clone());
        Self {
            board,
            selected_card,
            focused_column,
            mode: Mode::Normal,
            input: String::new(),
            status: None,
            quit: false,
        }
    }

    /// Cards in `slug` sorted by order then id.
    fn cards_in_column(&self, slug: &str) -> Vec<&Card> {
        let mut cards: Vec<&Card> = self
            .board
            .cards
            .iter()
            .filter(|c| c.column == slug)
            .collect();
        cards.sort_by_key(|c| (c.order, c.id.as_str()));
        cards
    }

    /// Persist the board, capturing any error as a non-fatal status message.
    fn persist(&mut self) {
        if let Err(err) = state::save(&mut self.board) {
            self.status = Some(format!("save failed: {err:#}"));
        }
    }

    /// Move focus to an adjacent column, re-selecting its first card.
    fn move_focus(&mut self, step: i64) {
        let Some(next) = self.board.adjacent_column(&self.focused_column, step) else {
            return;
        };
        self.focused_column = next;
        self.selected_card = self
            .board
            .cards_in_column(&self.focused_column)
            .first()
            .map(|c| c.id.clone());
        self.status = None;
    }

    /// Move the selected card to an adjacent column (left/right), dispatching
        /// an agent when the target column is dispatch-enabled.
        fn move_selected(&mut self, step: i64) {
            let Some(id) = self.selected_card.clone() else {
                self.move_focus(step);
                return;
            };
            let Some(current) = self.board.card(&id).map(|c| c.column.clone()) else {
                self.selected_card = None;
                return;
            };
            let Some(next) = self.board.adjacent_column(&current, step) else {
                self.status = Some("card is already at the edge".to_string());
                return;
            };
            match crate::commands::move_card_dispatch(&mut self.board, &id, &next) {
                Ok(status) => {
                    self.focused_column = next;
                    self.selected_card = Some(id);
                    self.persist();
                    self.status = Some(status);
                }
                Err(err) => self.status = Some(format!("move failed: {err:#}")),
            }
        }

    /// Cycle the selected card within the focused column.
    fn select_delta(&mut self, delta: i64) {
        let cards = self.board.cards_in_column(&self.focused_column);
        if cards.is_empty() {
            self.selected_card = None;
            return;
        }
        let len = cards.len() as i64;
        let current = self
            .selected_card
            .as_ref()
            .and_then(|id| cards.iter().position(|c| c.id == *id));
        let next = match current {
            Some(pos) => (pos as i64 + delta).rem_euclid(len),
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
        };
        self.selected_card = Some(cards[next as usize].id.clone());
    }

    fn new_card(&mut self, title: String) {
        let column = self.focused_column.clone();
        let id = self.board.add_card(title, &column);
        self.selected_card = Some(id);
        self.persist();
    }

    fn edit_card(&mut self, title: String) {
        let Some(id) = self.selected_card.clone() else {
            return;
        };
        if self.board.set_card_title(&id, title) {
            self.persist();
        }
    }

    fn archive(&mut self) {
        let Some(id) = self.selected_card.clone() else {
            self.status = Some("no card selected".to_string());
            return;
        };
        if self.board.remove_card(&id) {
            self.selected_card = None;
            self.persist();
        }
    }

    /// Note that the selected card's linked pane should be focused by looking
    /// it up through the herdr CLI's pane list (dispatch is out of scope).
    fn focus_pane(&mut self) {
        let Some(id) = self.selected_card.clone() else {
            self.status = Some("no card selected".to_string());
            return;
        };
        let Some(card) = self.board.card(&id) else {
            self.selected_card = None;
            return;
        };
        let Some(pane_id) = card.pane_id.clone() else {
            self.status = Some(format!("card {:?} has no linked pane", card.title));
            return;
        };

        let result = crate::herdr_cli::list_panes(None).and_then(|panes| {
            panes
                .into_iter()
                .find(|p| p.pane_id == pane_id)
                .map(|p| p.pane_id)
                .context("pane not present in herdr pane list")
        });

        self.status = Some(match result {
                    Ok(found) => match crate::herdr_cli::focus_pane(&found) {
                        Ok(()) => format!("focused pane {found}"),
                        Err(err) => format!("focus failed: {err:#}"),
                    },
                    Err(err) => format!("pane lookup failed: {err:#}"),
                });
    }

    fn start_input(&mut self, mode: Mode, initial: String) {
        self.mode = mode;
        self.input = initial;
        self.status = None;
    }

    fn cancel_input(&mut self) {
        self.mode = Mode::Normal;
        self.input.clear();
    }

    fn submit_input(&mut self) {
        let title = self.input.trim().to_string();
        let mode = self.mode;
        self.mode = Mode::Normal;
        self.input.clear();
        if title.is_empty() {
            return;
        }
        match mode {
            Mode::NewCard => self.new_card(title),
            Mode::EditCard => self.edit_card(title),
            Mode::Normal => {}
        }
    }

    /// Handle a keyboard event; returns when the UI should keep running (the
    /// `quit` flag on `self` signals the exit condition).
    fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        // Modal text input first.
        if self.mode != Mode::Normal {
            match key.code {
                KeyCode::Esc => self.cancel_input(),
                KeyCode::Enter => self.submit_input(),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                    self.input.push(c);
                }
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.input.pop();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.select_delta(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_delta(-1),
            KeyCode::Char('h') | KeyCode::Left => self.move_selected(-1),
            KeyCode::Char('l') | KeyCode::Right => self.move_selected(1),
            KeyCode::Tab => {
                let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    -1
                } else {
                    1
                };
                self.move_focus(step);
            }
            KeyCode::Char('n') => self.start_input(Mode::NewCard, String::new()),
            KeyCode::Char('e') => {
                let title = self
                    .selected_card
                    .as_ref()
                    .and_then(|id| self.board.card(id))
                    .map(|c| c.title.clone())
                    .unwrap_or_default();
                self.start_input(Mode::EditCard, title);
            }
            KeyCode::Char('d') | KeyCode::Delete => self.archive(),
            KeyCode::Char('f') | KeyCode::Enter => self.focus_pane(),
            _ => {}
        }
    }

    /// Resolve a mouse click against the rendered regions.
    fn on_mouse(&mut self, mouse: MouseEvent, hitmap: &Hitmap) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let (x, y) = (mouse.column, mouse.row);

        for (action, rect) in &hitmap.buttons {
            if rect_contains(*rect, x, y) {
                self.run_action(*action);
                return;
            }
        }
        for (id, rect) in &hitmap.cards {
            if rect_contains(*rect, x, y) {
                self.selected_card = Some(id.clone());
                if let Some(card) = self.board.card(id) {
                    self.focused_column = card.column.clone();
                }
                return;
            }
        }
        for (slug, rect) in &hitmap.columns {
            if rect_contains(*rect, x, y) {
                self.focused_column = slug.clone();
                self.selected_card = self
                    .board
                    .cards_in_column(slug)
                    .first()
                    .map(|c| c.id.clone());
                return;
            }
        }
    }

    /// Run a button action (also reachable via keyboard).
    fn run_action(&mut self, action: Action) {
        match action {
            Action::NewCard => self.start_input(Mode::NewCard, String::new()),
            Action::MoveLeft => self.move_selected(-1),
            Action::MoveRight => self.move_selected(1),
            Action::Edit => {
                let title = self
                    .selected_card
                    .as_ref()
                    .and_then(|id| self.board.card(id))
                    .map(|c| c.title.clone())
                    .unwrap_or_default();
                self.start_input(Mode::EditCard, title);
            }
            Action::Archive => self.archive(),
            Action::FocusPane => self.focus_pane(),
        }
    }

    /// Render the full screen and collect the clickable regions.
    fn draw(&self, f: &mut Frame) -> Hitmap {
        let mut hitmap = Hitmap::default();
        let area = f.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // title
                Constraint::Length(3), // action buttons
                Constraint::Min(0),    // columns
                Constraint::Length(3), // help / status
            ])
            .split(area);

        self.draw_title(f, chunks[0]);
        self.draw_buttons(f, chunks[1], &mut hitmap);
        self.draw_columns(f, chunks[2], &mut hitmap);
        self.draw_help(f, chunks[3]);

        if self.mode != Mode::Normal {
            self.draw_modal(f, f.area());
        }

        hitmap
    }

    fn draw_title(&self, f: &mut Frame, area: Rect) {
        let focused = self
            .board
            .ordered_columns()
            .iter()
            .find(|c| c.slug == self.focused_column)
            .map(|c| c.title.as_str())
            .unwrap_or("-");
        let selected = self
            .selected_card
            .as_ref()
            .and_then(|id| self.board.card(id))
            .map(|c| c.title.as_str())
            .unwrap_or("-");

        let text = Line::from(vec![
            Span::styled("Windows Board", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!(
                "    focused: {}    selected: {}    revision: {}",
                focused, selected, self.board.revision
            )),
        ]);
        let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan));
        f.render_widget(Paragraph::new(text).block(block), area);
    }

    fn draw_buttons(&self, f: &mut Frame, area: Rect, hitmap: &mut Hitmap) {
        let actions = [
            Action::NewCard,
            Action::MoveLeft,
            Action::MoveRight,
            Action::Edit,
            Action::Archive,
            Action::FocusPane,
        ];
        let n = actions.len() as u32;
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, n); actions.len()])
            .split(area);

        for (action, cell) in actions.into_iter().zip(cells.iter()) {
            let rect = cell.inner(Margin { vertical: 0, horizontal: 1 });
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray));
            f.render_widget(
                Paragraph::new(Span::styled(action.label(), Style::default()))
                    .block(block)
                    .wrap(Wrap { trim: true }),
                *cell,
            );
            hitmap.buttons.push((action, rect));
        }
    }

    fn draw_columns(&self, f: &mut Frame, area: Rect, hitmap: &mut Hitmap) {
        let columns = self.board.ordered_columns();
        let n = columns.len();
        if n == 0 {
            f.render_widget(
                Paragraph::new("No columns defined.").block(Block::default().borders(Borders::ALL)),
                area,
            );
            return;
        }

        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, n as u32); n])
            .split(area);

        for (column, cell) in columns.iter().zip(cells.iter()) {
            let is_focused = column.slug == self.focused_column;
            let border = if is_focused { Color::Cyan } else { Color::DarkGray };
            let title_style = if is_focused {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .title(Span::styled(
                    format!(" {} ", column.title),
                    title_style,
                ));
            f.render_widget(block, *cell);
            hitmap.columns.push((column.slug.clone(), *cell));

            let inner = cell.inner(Margin { vertical: 1, horizontal: 1 });
            if inner.width == 0 || inner.height == 0 {
                continue;
            }
            self.draw_cards(f, &column.slug, inner, hitmap);
        }
    }

    fn draw_cards(&self, f: &mut Frame, slug: &str, inner: Rect, hitmap: &mut Hitmap) {
        let cards = self.cards_in_column(slug);
        if cards.is_empty() {
            let hint = Paragraph::new(Span::styled(
                "(empty)",
                Style::default().fg(Color::DarkGray),
            ))
            .block(Block::default());
            f.render_widget(hint, Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 });
            return;
        }

        let mut y = inner.y;
        let mut shown = 0;
        for card in &cards {
            let has_sub = card.pane_id.is_some() || card.agent_name.is_some();
            let height = if has_sub { 4 } else { 3 };
            if y + height as u16 > inner.y.saturating_add(inner.height) {
                break;
            }
            let selected = self.selected_card.as_deref() == Some(card.id.as_str());

            let prefix = if selected { "\u{25B8} " } else { "  " };
            let mut lines: Vec<Line> = vec![Line::from(Span::styled(
                format!("{prefix}{}", card.title),
                if selected {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ))];
            if let Some(pane) = &card.pane_id {
                lines.push(Line::from(Span::styled(
                    format!("  pane {pane}"),
                    Style::default().fg(Color::DarkGray),
                )));
            } else if let Some(agent) = &card.agent_name {
                lines.push(Line::from(Span::styled(
                    format!("  agent {agent}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            let border = if selected { Color::Cyan } else { Color::DarkGray };
            let rect = Rect { x: inner.x, y, width: inner.width, height };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border));
            f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), rect);
            hitmap.cards.push((card.id.clone(), rect));

            y += height;
            shown += 1;
        }

        if shown < cards.len() {
            let remaining = cards.len() - shown;
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("... {remaining} more"),
                    Style::default().fg(Color::DarkGray),
                )),
                Rect { x: inner.x, y, width: inner.width, height: 1 },
            );
        }
    }

    fn draw_help(&self, f: &mut Frame, area: Rect) {
        let mut spans = vec![Span::styled(
            " q quit  j/k select  h/l move card  Tab column  n new  e edit  d archive  f focus pane ",
            Style::default().fg(Color::DarkGray),
        )];
        if let Some(status) = &self.status {
            spans.push(Span::styled(
                format!(" -- {status}"),
                Style::default().fg(Color::Yellow),
            ));
        }
        let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray));
        f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
    }

    fn draw_modal(&self, f: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(4).min(60).max(10);
        let height = 3;
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let rect = Rect { x, y, width, height };

        let prompt = match self.mode {
            Mode::NewCard => " New card title (Enter to save, Esc to cancel) ",
            Mode::EditCard => " Edit card title (Enter to save, Esc to cancel) ",
            Mode::Normal => " ",
        };
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(prompt);
        f.render_widget(
            Paragraph::new(self.input.as_str()).block(block).wrap(Wrap { trim: true }),
            rect,
        );
    }
}

/// Whether `rect` contains terminal coordinate `(x, y)`.
fn rect_contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

/// Restore the terminal (idempotent): leave raw mode and the alternate screen,
/// disable mouse capture, and re-show the cursor. Errors are ignored so callers
/// can be unconditional.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture, Show);
}

/// Scope guard that restores the terminal on any exit path (early return,
/// `?` propagation, or panic unwind).
struct TermGuard;

impl Drop for TermGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Configure a panic hook that restores the terminal before the default panic
/// handler writes its message, so a crash never leaves the console garbled.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
}

pub fn run() -> Result<()> {
    let board = state::load()?;
    let mut app = App::new(board);

    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let _guard = TermGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let _ = execute!(io::stdout(), Show);

    loop {
        let mut hitmap = Hitmap::default();
        terminal.draw(|f| {
            hitmap = app.draw(f);
        })?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => app.on_key(key),
                Event::Mouse(mouse) => app.on_mouse(mouse, &hitmap),
                _ => {}
            }
        }

        if app.quit {
            break;
        }
    }

    restore_terminal();
    Ok(())
}

    #[cfg(test)]
    mod tests {
        use super::*;
        use ratatui::layout::Rect;

        fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
            Rect { x, y, width: w, height: h }
        }

        #[test]
        fn rect_contains_covers_edges_and_bounds() {
            let r = rect(2, 3, 4, 5); // x in [2,6), y in [3,8)
            assert!(rect_contains(r, 2, 3));
            assert!(rect_contains(r, 5, 7));
            assert!(!rect_contains(r, 6, 3)); // right edge exclusive
            assert!(!rect_contains(r, 2, 8)); // bottom edge exclusive
            assert!(!rect_contains(r, 1, 3));
            assert!(!rect_contains(r, 2, 2));
        }
    }