use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph, Wrap},
};
use std::io::stdout;

use crate::context::ProjectContext;
use crate::services::ServiceManager;

// ── Color palette ────────────────────────────────────────
const BRAND: Color = Color::Rgb(80, 160, 255);   // Primary blue
const ACCENT: Color = Color::Rgb(130, 200, 255);  // Light blue
const SURFACE: Color = Color::Rgb(30, 30, 46);    // Dark background
const SURFACE_ALT: Color = Color::Rgb(40, 40, 56); // Slightly lighter
const TEXT: Color = Color::Rgb(205, 214, 244);     // Main text
const TEXT_DIM: Color = Color::Rgb(108, 112, 134); // Dimmed text
const GREEN_OK: Color = Color::Rgb(166, 227, 161); // Success green
const RED_ERR: Color = Color::Rgb(243, 139, 168);  // Error red
const YELLOW_WARN: Color = Color::Rgb(249, 226, 175); // Warning
const MAUVE: Color = Color::Rgb(203, 166, 247);   // Purple accent

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
    status_message: Option<(String, Color)>,
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
        self.status_message = None;
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
        self.status_message = None;
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

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if let Some(idx) = app.confirm_delete {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let name = app.contexts[idx].name.clone();
                            if let Err(e) = ProjectContext::remove(&name) {
                                app.status_message = Some((format!("Error: {}", e), RED_ERR));
                            } else {
                                app.status_message = Some((format!("Removed '{}'", name), GREEN_OK));
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
                            app.status_message = Some((
                                format!("Delete '{}'?  y confirm  esc cancel", app.contexts[idx].name),
                                YELLOW_WARN,
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

    if let Action::Switch(name) = app.action {
        if let Ok(ctx) = ProjectContext::load(&name) {
            let cwd = std::env::current_dir()?;
            if ProjectContext::has_dirty_git_state(&cwd) {
                if let Some(summary) = ProjectContext::dirty_summary(&cwd) {
                    eprintln!(
                        "  {}! Uncommitted changes: {}{}",
                        crate::style::YELLOW, summary, crate::style::RESET,
                    );
                }
            }

            let service_mgr = ServiceManager::new();
            if !ctx.services.is_empty() {
                crate::style::section("Services");
                service_mgr.start_services(&ctx)?;
            }

            crate::style::blank();
            crate::style::success(&format!(
                "Switched to {}{}{}",
                crate::style::BOLD, name, crate::style::RESET,
            ));
            crate::style::blank();
            ctx.print_shell_commands();
        }
    }

    Ok(())
}

fn draw(f: &mut Frame, app: &mut App) {
    // Fill entire background
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(SURFACE)), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),  // Header
            Constraint::Min(0),     // Main
            Constraint::Length(3),  // Footer
        ])
        .split(area);

    draw_header(f, chunks[0]);
    draw_main(f, chunks[1], app);
    draw_footer(f, chunks[2], app);
}

fn draw_header(f: &mut Frame, area: Rect) {
    let header_block = Block::default()
        .style(Style::default().bg(SURFACE_ALT))
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Rgb(50, 50, 70)));

    let inner = header_block.inner(area);
    f.render_widget(header_block, area);

    let title = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("  pylot", Style::default().fg(BRAND).bold()),
            Span::styled(
                format!("  v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(TEXT_DIM),
            ),
        ]),
        Line::from(Span::styled(
            "  Project Context Switcher",
            Style::default().fg(TEXT_DIM).italic(),
        )),
    ]);
    f.render_widget(title, inner);
}

fn draw_main(f: &mut Frame, area: Rect, app: &mut App) {
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);

    draw_list(f, main_chunks[0], app);
    draw_details(f, main_chunks[1], app);
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
    if app.contexts.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled("  No contexts saved", Style::default().fg(TEXT_DIM))),
            Line::from(""),
            Line::from(Span::styled("  Run pylot save <name>", Style::default().fg(TEXT_DIM).italic())),
            Line::from(Span::styled("  in a project directory", Style::default().fg(TEXT_DIM).italic())),
        ])
        .block(
            Block::default()
                .title(Span::styled(" Contexts ", Style::default().fg(BRAND).bold()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 70)))
                .style(Style::default().bg(SURFACE))
                .padding(Padding::horizontal(1)),
        );
        f.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = app
        .contexts
        .iter()
        .enumerate()
        .map(|(i, ctx)| {
            let health = app.get_health(&ctx.name);
            let svc_indicator = if health.is_empty() {
                String::new()
            } else {
                let alive = health.iter().filter(|(_, _, a)| *a).count();
                if alive == health.len() {
                    " ●".to_string()
                } else if alive == 0 {
                    " ○".to_string()
                } else {
                    " ◐".to_string()
                }
            };

            let branch = ctx.git_branch.as_deref().unwrap_or("");
            let is_selected = app.list_state.selected() == Some(i);

            let name_style = if is_selected {
                Style::default().fg(TEXT).bold()
            } else {
                Style::default().fg(TEXT)
            };

            ListItem::new(Line::from(vec![
                Span::styled(&ctx.name, name_style),
                Span::styled(
                    if branch.is_empty() { String::new() } else { format!("  {}", branch) },
                    Style::default().fg(MAUVE),
                ),
                Span::styled(
                    svc_indicator,
                    Style::default().fg(if health.iter().all(|(_, _, a)| *a) { GREEN_OK } else { YELLOW_WARN }),
                ),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(
                    format!(" Contexts ({}) ", app.contexts.len()),
                    Style::default().fg(BRAND).bold(),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 70)))
                .style(Style::default().bg(SURFACE))
                .padding(Padding::horizontal(1)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(55, 55, 80))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌ ");

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_details(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .title(Span::styled(" Details ", Style::default().fg(BRAND).bold()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(50, 50, 70)))
        .style(Style::default().bg(SURFACE))
        .padding(Padding::new(2, 2, 1, 1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Clone data out of app to avoid lifetime issues
    let ctx_snapshot = app.selected_context().cloned();

    let detail_text: Vec<Line<'static>> = if let Some(ctx) = ctx_snapshot.as_ref() {
        let health: Vec<(String, u32, bool)> = app.get_health(&ctx.name).to_vec();
        let active_ports = app.active_ports.clone();

        let mut lines: Vec<Line<'static>> = vec![
            // Context name as header
            Line::from(Span::styled(
                ctx.name.clone(),
                Style::default().fg(TEXT).bold(),
            )),
            Line::from(Span::styled(
                "─".repeat(32),
                Style::default().fg(Color::Rgb(50, 50, 70)),
            )),
            Line::from(""),

            detail_line("Path", &ctx.path.display().to_string(), TEXT),
            detail_line("Branch", ctx.git_branch.as_deref().unwrap_or("n/a"), MAUVE),
            detail_line("Env file", ctx.env_file.as_deref().unwrap_or("none"), TEXT),
            detail_line("Env vars", &ctx.env_vars.len().to_string(), TEXT),

            Line::from(""),
        ];

        // Service health
        if !health.is_empty() {
            lines.push(Line::from(Span::styled(
                "Services".to_string(),
                Style::default().fg(ACCENT).bold(),
            )));
            lines.push(Line::from(""));
            for (name, pid, alive) in &health {
                let (symbol, status, color) = if *alive {
                    ("●", "running", GREEN_OK)
                } else {
                    ("○", "stopped", RED_ERR)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", symbol), Style::default().fg(color)),
                    Span::styled(name.clone(), Style::default().fg(TEXT)),
                    Span::styled(format!("  {}", status), Style::default().fg(color)),
                    Span::styled(format!("  pid {}", pid), Style::default().fg(TEXT_DIM)),
                ]));
            }
            lines.push(Line::from(""));
        } else if !ctx.services.is_empty() {
            lines.push(Line::from(Span::styled(
                "Services (not running)".to_string(),
                Style::default().fg(TEXT_DIM),
            )));
            lines.push(Line::from(""));
            for (name, cmd) in &ctx.services {
                lines.push(Line::from(vec![
                    Span::styled(format!("  ○ {}", name), Style::default().fg(TEXT_DIM)),
                    Span::styled(format!("  {}", cmd), Style::default().fg(Color::Rgb(70, 70, 90))),
                ]));
            }
            lines.push(Line::from(""));
        }

        // Required ports
        if !ctx.ports_required.is_empty() {
            lines.push(Line::from(Span::styled(
                "Required Ports".to_string(),
                Style::default().fg(ACCENT).bold(),
            )));
            lines.push(Line::from(""));
            for port in &ctx.ports_required {
                let in_use = active_ports.iter().any(|(p, _)| p == port);
                let (symbol, status, color) = if in_use {
                    ("●", "in use", RED_ERR)
                } else {
                    ("○", "free", GREEN_OK)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", symbol), Style::default().fg(color)),
                    Span::styled(format!(":{}", port), Style::default().fg(TEXT)),
                    Span::styled(format!("  {}", status), Style::default().fg(color)),
                ]));
            }
            lines.push(Line::from(""));
        }

        // Last used
        if let Some(ref last) = ctx.last_accessed {
            lines.push(Line::from(vec![
                Span::styled("Last used  ".to_string(), Style::default().fg(TEXT_DIM)),
                Span::styled(
                    last.format("%b %d, %Y at %H:%M").to_string(),
                    Style::default().fg(TEXT_DIM),
                ),
            ]));
        }

        lines
    } else {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "Select a context to view details".to_string(),
                Style::default().fg(TEXT_DIM).italic(),
            )),
        ]
    };

    let detail = Paragraph::new(detail_text).wrap(Wrap { trim: false });
    f.render_widget(detail, inner);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let footer_block = Block::default()
        .style(Style::default().bg(SURFACE_ALT))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::Rgb(50, 50, 70)));

    let inner = footer_block.inner(area);
    f.render_widget(footer_block, area);

    if let Some((ref msg, color)) = app.status_message {
        let status = Paragraph::new(Line::from(vec![
            Span::styled(format!("  {}", msg), Style::default().fg(color)),
        ]));
        f.render_widget(status, inner);
        return;
    }

    let keys = vec![
        ("↑↓", "navigate"),
        ("enter", "switch"),
        ("d", "delete"),
        ("q", "quit"),
    ];

    let spans: Vec<Span> = keys
        .iter()
        .enumerate()
        .flat_map(|(i, (key, desc))| {
            let mut s = vec![
                Span::styled(format!(" {} ", key), Style::default().fg(TEXT).bold().bg(Color::Rgb(55, 55, 80))),
                Span::styled(format!(" {} ", desc), Style::default().fg(TEXT_DIM)),
            ];
            if i < keys.len() - 1 {
                s.push(Span::styled("  ", Style::default()));
            }
            s
        })
        .collect();

    let footer = Paragraph::new(Line::from(spans));
    f.render_widget(footer, inner);
}

fn detail_line(label: &str, value: &str, value_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<11}", label), Style::default().fg(TEXT_DIM)),
        Span::styled(value.to_string(), Style::default().fg(value_color)),
    ])
}
