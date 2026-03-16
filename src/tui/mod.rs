use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::io::stdout;

use crate::context::ProjectContext;
use crate::services::ServiceManager;

enum Action {
    None,
    Switch(String),
}

struct App {
    contexts: Vec<ProjectContext>,
    list_state: ListState,
    active_ports: Vec<(u16, String)>,
    service_health: Vec<(String, Vec<(String, u32, bool)>)>,
    should_quit: bool,
    action: Action,
    confirm_delete: Option<usize>,
    status_message: Option<String>,
}

impl App {
    fn new() -> Result<Self> {
        let contexts = ProjectContext::list_all().unwrap_or_default();
        let service_mgr = ServiceManager::new();
        let active_ports = service_mgr.get_listening_ports();

        let service_health: Vec<_> = contexts
            .iter()
            .map(|ctx| {
                let health = service_mgr.service_health(&ctx.name);
                (ctx.name.clone(), health)
            })
            .collect();

        let mut list_state = ListState::default();
        if !contexts.is_empty() {
            list_state.select(Some(0));
        }

        Ok(Self {
            contexts,
            list_state,
            active_ports,
            service_health,
            should_quit: false,
            action: Action::None,
            confirm_delete: None,
            status_message: None,
        })
    }

    fn selected_context(&self) -> Option<&ProjectContext> {
        self.list_state
            .selected()
            .and_then(|i| self.contexts.get(i))
    }

    fn next(&mut self) {
        if self.contexts.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => (i + 1) % self.contexts.len(),
            None => 0,
        };
        self.list_state.select(Some(i));
        self.confirm_delete = None;
    }

    fn previous(&mut self) {
        if self.contexts.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.contexts.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.confirm_delete = None;
    }

    fn get_health(&self, name: &str) -> &[(String, u32, bool)] {
        self.service_health
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, h)| h.as_slice())
            .unwrap_or(&[])
    }
}

pub fn run_dashboard() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut app = App::new()?;

    while !app.should_quit {
        terminal.draw(|f| draw(f, &mut app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Handle delete confirmation mode
                if let Some(idx) = app.confirm_delete {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let name = app.contexts[idx].name.clone();
                            if let Err(e) = ProjectContext::remove(&name) {
                                app.status_message = Some(format!("Error: {}", e));
                            } else {
                                app.status_message = Some(format!("Removed '{}'", name));
                                app.contexts.remove(idx);
                                if !app.contexts.is_empty() {
                                    let new_idx = if idx >= app.contexts.len() {
                                        app.contexts.len() - 1
                                    } else {
                                        idx
                                    };
                                    app.list_state.select(Some(new_idx));
                                } else {
                                    app.list_state.select(None);
                                }
                            }
                            app.confirm_delete = None;
                        }
                        _ => {
                            app.confirm_delete = None;
                            app.status_message = None;
                        }
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                    KeyCode::Down | KeyCode::Char('j') => app.next(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous(),
                    KeyCode::Enter => {
                        if let Some(ctx) = app.selected_context() {
                            app.action = Action::Switch(ctx.name.clone());
                            app.should_quit = true;
                        }
                    }
                    KeyCode::Char('d') | KeyCode::Delete => {
                        if let Some(idx) = app.list_state.selected() {
                            app.confirm_delete = Some(idx);
                            app.status_message = Some(format!(
                                "Delete '{}'? Press y to confirm, any other key to cancel",
                                app.contexts[idx].name
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    // After TUI exits, perform the switch action if requested
    if let Action::Switch(name) = app.action {
        if let Ok(ctx) = ProjectContext::load(&name) {
            // Check dirty state
            let cwd = std::env::current_dir()?;
            if ProjectContext::has_dirty_git_state(&cwd) {
                if let Some(summary) = ProjectContext::dirty_summary(&cwd) {
                    eprintln!("Warning: Current directory has uncommitted changes ({}).", summary);
                }
            }

            let service_mgr = ServiceManager::new();

            // Start services
            if !ctx.services.is_empty() {
                eprintln!("Starting services...");
                service_mgr.start_services(&ctx)?;
            }

            eprintln!("Switched to '{}'", name);
            ctx.print_shell_commands();
        }
    }

    Ok(())
}

fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Header
    let header = Paragraph::new(" pylot — Project Context Switcher")
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(header, chunks[0]);

    // Main content
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    // Context list
    let items: Vec<ListItem> = app
        .contexts
        .iter()
        .map(|ctx| {
            let branch = ctx.git_branch.as_deref().unwrap_or("-");
            let health = app.get_health(&ctx.name);
            let svc_indicator = if health.is_empty() {
                String::new()
            } else {
                let alive = health.iter().filter(|(_, _, a)| *a).count();
                if alive == health.len() {
                    " [ok]".to_string()
                } else if alive == 0 {
                    " [stopped]".to_string()
                } else {
                    format!(" [{}/{}]", alive, health.len())
                }
            };
            ListItem::new(format!("  {} ({}){}", ctx.name, branch, svc_indicator))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Contexts ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .bold(),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, main_chunks[0], &mut app.list_state);

    // Detail panel
    let detail_text = if let Some(ctx) = app.selected_context() {
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Name:     ", Style::default().fg(Color::Yellow)),
                Span::raw(&ctx.name),
            ]),
            Line::from(vec![
                Span::styled("Path:     ", Style::default().fg(Color::Yellow)),
                Span::raw(ctx.path.display().to_string()),
            ]),
            Line::from(vec![
                Span::styled("Branch:   ", Style::default().fg(Color::Yellow)),
                Span::raw(ctx.git_branch.as_deref().unwrap_or("n/a")),
            ]),
            Line::from(vec![
                Span::styled("Env file: ", Style::default().fg(Color::Yellow)),
                Span::raw(ctx.env_file.as_deref().unwrap_or("none")),
            ]),
            Line::from(vec![
                Span::styled("Env vars: ", Style::default().fg(Color::Yellow)),
                Span::raw(format!("{}", ctx.env_vars.len())),
            ]),
            Line::from(""),
        ];

        // Service health
        let health = app.get_health(&ctx.name);
        if !health.is_empty() {
            lines.push(Line::from(Span::styled(
                "Services:",
                Style::default().fg(Color::Green).bold(),
            )));
            for (name, pid, alive) in health {
                let (status, color) = if *alive {
                    ("running", Color::Green)
                } else {
                    ("stopped", Color::Red)
                };
                lines.push(Line::from(vec![
                    Span::raw(format!("  {} ", name)),
                    Span::styled(format!("(PID {}) ", pid), Style::default().fg(Color::DarkGray)),
                    Span::styled(status, Style::default().fg(color)),
                ]));
            }
            lines.push(Line::from(""));
        } else if !ctx.services.is_empty() {
            lines.push(Line::from(Span::styled(
                "Services (not started):",
                Style::default().fg(Color::DarkGray),
            )));
            for (name, cmd) in &ctx.services {
                lines.push(Line::from(format!("  {} → {}", name, cmd)));
            }
            lines.push(Line::from(""));
        }

        // Required ports
        if !ctx.ports_required.is_empty() {
            lines.push(Line::from(Span::styled(
                "Required ports:",
                Style::default().fg(Color::Magenta).bold(),
            )));
            for port in &ctx.ports_required {
                let (status, color) = if app.active_ports.iter().any(|(p, _)| p == port) {
                    ("IN USE", Color::Red)
                } else {
                    ("free", Color::Green)
                };
                lines.push(Line::from(vec![
                    Span::raw(format!("  :{} ", port)),
                    Span::styled(status, Style::default().fg(color)),
                ]));
            }
            lines.push(Line::from(""));
        }

        if let Some(ref last) = ctx.last_accessed {
            lines.push(Line::from(vec![
                Span::styled("Last used: ", Style::default().fg(Color::Yellow)),
                Span::raw(last.format("%Y-%m-%d %H:%M").to_string()),
            ]));
        }

        lines
    } else {
        vec![Line::from("No context selected")]
    };

    let detail = Paragraph::new(detail_text)
        .block(
            Block::default()
                .title(" Details ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(detail, main_chunks[1]);

    // Footer
    let footer_text = if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        " ↑/↓ navigate  |  Enter switch  |  d delete  |  q quit".to_string()
    };
    let footer_style = if app.confirm_delete.is_some() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let footer = Paragraph::new(format!(" {}", footer_text))
        .style(footer_style)
        .block(Block::default().borders(Borders::TOP));
    f.render_widget(footer, chunks[2]);
}
