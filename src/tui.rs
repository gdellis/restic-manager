use crate::errors::AppError;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::time::Duration;

const SIDEBAR_ITEMS: [&str; 3] = ["Jobs", "Repositories", "Logs"];
const TICK_MS: u64 = 100;

pub fn run() -> Result<(), AppError> {
    let mut terminal = setup_terminal()?;
    let result = event_loop(&mut terminal);
    restore_terminal(&mut terminal).ok();
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, AppError> {
    crossterm::terminal::enable_raw_mode()
        .map_err(|e| AppError::Other(format!("enable_raw_mode: {e}")))?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)
        .map_err(|e| AppError::Other(format!("EnterAlternateScreen: {e}")))?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(|e| AppError::Other(format!("Terminal::new: {e}")))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), AppError> {
    let mut sidebar_state = ListState::default();
    sidebar_state.select(Some(0));

    loop {
        terminal
            .draw(|f| ui(f, &sidebar_state))
            .map_err(|e| AppError::Other(format!("draw: {e}")))?;

        if event::poll(Duration::from_millis(TICK_MS))
            .map_err(|e| AppError::Other(format!("event::poll: {e}")))?
        {
            if let Event::Key(key) =
                event::read().map_err(|e| AppError::Other(format!("event::read: {e}")))?
            {
                if handle_key(key, &mut sidebar_state)? {
                    return Ok(());
                }
            }
        }
    }
}

/// Returns `Ok(true)` when the user requested quit.
fn handle_key(key: KeyEvent, sidebar_state: &mut ListState) -> Result<bool, AppError> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
        KeyCode::Down | KeyCode::Char('j') => move_selection(sidebar_state, 1),
        KeyCode::Up | KeyCode::Char('k') => move_selection(sidebar_state, -1),
        _ => {}
    }
    Ok(false)
}

fn move_selection(state: &mut ListState, delta: i32) {
    let len = SIDEBAR_ITEMS.len() as i32;
    let cur = state.selected().unwrap_or(0) as i32;
    let next = (cur + delta).rem_euclid(len) as usize;
    state.select(Some(next));
}

fn ui(f: &mut ratatui::Frame, sidebar_state: &ListState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(1),    // body
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new(Line::from("restic-manager — tui (skeleton)"))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(header, chunks[0]);

    // Body: sidebar + detail
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(20), Constraint::Min(1)])
        .split(chunks[1]);

    let items: Vec<ListItem> = SIDEBAR_ITEMS.iter().map(|s| ListItem::new(*s)).collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::RIGHT).title("Views"))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    f.render_stateful_widget(list, body[0], &mut sidebar_state.clone());

    let selected = sidebar_state.selected().unwrap_or(0);
    let detail_text = match SIDEBAR_ITEMS.get(selected).copied().unwrap_or("Jobs") {
        "Jobs" => "Jobs view — Wave 3 will populate job list here.",
        "Repositories" => "Repositories view — Wave 3 will populate repo list here.",
        _ => "Logs view — Wave 3 will stream command output here.",
    };
    let detail =
        Paragraph::new(detail_text).block(Block::default().borders(Borders::ALL).title("Detail"));
    f.render_widget(detail, body[1]);

    // Status bar
    let status = Paragraph::new("?:help  q:quit  ↑/↓:select")
        .style(Style::default().bg(Color::Reset).fg(Color::Gray));
    f.render_widget(status, chunks[2]);
}
