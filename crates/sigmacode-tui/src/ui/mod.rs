use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, AppState, MessageRole, Tab};

const BRAND: Color = Color::Rgb(200, 160, 80);
const DIM: Color = Color::Rgb(100, 100, 100);
const ACCENT: Color = Color::Rgb(130, 180, 255);
const SIDEBAR_BG: Color = Color::Rgb(25, 25, 30);
const INPUT_BG: Color = Color::Rgb(30, 30, 38);

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    // Main layout: content + sidebar
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),      // Content area
            Constraint::Length(28),  // Sidebar
        ])
        .split(area);

    // Content area: messages + input + status
    let content_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),   // Messages
            Constraint::Length(3), // Input
            Constraint::Length(1), // Status bar
        ])
        .split(main_chunks[0]);

    // Render content
    match app.current_tab {
        Tab::Chat => render_chat(f, app, content_chunks[0]),
        Tab::Logs => render_logs(f, app, content_chunks[0]),
    }

    render_input(f, app, content_chunks[1]);
    render_status_bar(f, app, content_chunks[2]);

    // Render sidebar
    render_sidebar(f, app, main_chunks[1]);
}

fn render_chat(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for msg in &app.messages {
        match msg.role {
            MessageRole::User => {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled(
                        " > ",
                        Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        msg.content.clone(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            MessageRole::Assistant => {
                for line in msg.content.lines() {
                    lines.push(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Rgb(200, 200, 200)),
                    )));
                }
            }
            MessageRole::System => {
                let preview: String = msg.content.chars().take(100).collect();
                lines.push(Line::from(vec![
                    Span::styled(
                        "   ",
                        Style::default(),
                    ),
                    Span::styled(
                        preview,
                        Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            MessageRole::Tool => {
                let icon = "◆";
                lines.push(Line::from(vec![
                    Span::styled(
                        "   ",
                        Style::default(),
                    ),
                    Span::styled(
                        format!("{} ", icon),
                        Style::default().fg(ACCENT),
                    ),
                    Span::styled(
                        msg.content.clone(),
                        Style::default().fg(DIM),
                    ),
                ]));
            }
        }
    }

    // Running indicator
    if app.state == AppState::Running {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = spinner[app.tick_count % spinner.len()];
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {} ", frame),
                Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "thinking...".to_string(),
                Style::default().fg(DIM),
            ),
        ]));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn render_logs(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for log in app.logs.iter().rev().take(area.height as usize) {
        lines.push(Line::from(Span::styled(
            log.clone(),
            Style::default().fg(DIM),
        )));
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.state == AppState::Input {
            BRAND
        } else {
            DIM
        }))
        .style(Style::default().bg(INPUT_BG));

    let input_content = match app.state {
        AppState::Input => {
            let cursor = if app.tick_count % 20 < 10 { "▏" } else { " " };
            format!("{}{}", app.input, cursor)
        }
        AppState::Running => "...".to_string(),
        _ => String::new(),
    };

    let input_style = if app.state == AppState::Input {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(DIM)
    };

    let paragraph = Paragraph::new(input_content).style(input_style).block(block);
    f.render_widget(paragraph, area);
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(30),
        ])
        .split(area);

    // Left: status
    let status = match app.state {
        AppState::Idle => "esc interrupt",
        AppState::Input => "enter send · esc cancel",
        AppState::Running => "esc interrupt",
        AppState::Done => "i new task · q quit",
    };

    f.render_widget(
        Paragraph::new(format!(" {}", status)).style(Style::default().fg(DIM)),
        chunks[0],
    );

    // Right: token count
    let token_text = format!("{} ", app.token_usage);
    f.render_widget(
        Paragraph::new(token_text).style(Style::default().fg(DIM)),
        chunks[1],
    );
}

fn render_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let bg = Block::default().bg(SIDEBAR_BG);
    let inner = bg.inner(area);
    f.render_widget(bg, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Length(5),  // Context info
            Constraint::Length(2),  // LSP
            Constraint::Min(0),    // Getting started
            Constraint::Length(3), // Path
        ])
        .split(inner);

    // Title
    let title = Paragraph::new(vec![
        Line::from(Span::styled(
            " Context",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  No active task",
            Style::default().fg(DIM),
        )),
    ]);
    f.render_widget(title, chunks[0]);

    // Context info
    let ctx_info = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "Context",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                &app.token_display,
                Style::default().fg(DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{}% used", app.context_usage_pct),
                Style::default().fg(DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("${:.2} spent", app.cost),
                Style::default().fg(DIM),
            ),
        ]),
    ]);
    f.render_widget(ctx_info, chunks[1]);

    // LSP
    let lsp = Paragraph::new(vec![
        Line::from(Span::styled(
            "  LSP",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  LSPs are disabled",
            Style::default().fg(DIM),
        )),
    ]);
    f.render_widget(lsp, chunks[2]);

    // Getting started
    let getting_started = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "Getting started",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "SigmaCode includes free models",
                Style::default().fg(DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "so you can start immediately.",
                Style::default().fg(DIM),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "Connect from 75+ providers to",
                Style::default().fg(DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "use other models, including",
                Style::default().fg(DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "Claude, GPT, Gemini etc",
                Style::default().fg(DIM),
            ),
        ]),
    ]);
    f.render_widget(getting_started, chunks[3]);

    // Path
    let cwd = std::env::current_dir()
        .map(|p| {
            let home = dirs::home_dir()
                .map(|h| p.display().to_string().replace(&h.display().to_string(), "~"))
                .unwrap_or_else(|| p.display().to_string());
            home
        })
        .unwrap_or_else(|_| "~".into());

    let short_path: String = if cwd.len() > 24 {
        format!("...{}", &cwd[cwd.len() - 21..])
    } else {
        cwd
    };

    let path = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", short_path),
            Style::default().fg(DIM),
        )),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("sigmaCode {}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(DIM),
            ),
        ]),
    ]);
    f.render_widget(path, chunks[4]);
}
