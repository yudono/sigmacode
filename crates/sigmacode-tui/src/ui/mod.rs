use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, AppState, MessageRole, Tab};

const BRAND: Color = Color::Rgb(200, 160, 80);
const DIM: Color = Color::Rgb(100, 100, 100);
const ACCENT: Color = Color::Rgb(130, 180, 255);
const GREEN: Color = Color::Rgb(80, 200, 120);
const RED: Color = Color::Rgb(230, 80, 80);
const SIDEBAR_BG: Color = Color::Rgb(20, 20, 25);

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(26),
        ])
        .split(area);

    let content_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(5), // input + model info
            Constraint::Length(1),
        ])
        .split(main_chunks[0]);

    match app.current_tab {
        Tab::Chat => render_chat(f, app, content_chunks[0]),
        Tab::Logs => render_logs(f, app, content_chunks[0]),
    }

    render_input_area(f, app, content_chunks[1]);
    render_status_bar(f, app, content_chunks[2]);
    render_sidebar(f, app, main_chunks[1]);
}

fn render_chat(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for msg in &app.messages {
        match msg.role {
            MessageRole::User => {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled(" > ", Style::default().fg(BRAND).add_modifier(Modifier::BOLD)),
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
            MessageRole::Tool => {
                lines.push(Line::from(vec![
                    Span::styled("   ", Style::default()),
                    Span::styled("◆ ", Style::default().fg(ACCENT)),
                    Span::styled(msg.content.clone(), Style::default().fg(DIM)),
                ]));
            }
            MessageRole::Thought => {
                lines.push(Line::from(vec![
                    Span::styled("   ", Style::default()),
                    Span::styled(
                        msg.content.clone(),
                        Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            MessageRole::Diff => {
                if let Some(ref diff) = msg.diff {
                    lines.push(Line::from(vec![
                        Span::styled("   ", Style::default()),
                        Span::styled(
                            format!("← Edit {}", diff.file_path),
                            Style::default().fg(ACCENT),
                        ),
                    ]));
                    // Show side-by-side diff
                    let max_lines = diff.old_lines.len().max(diff.new_lines.len());
                    for i in 0..max_lines.min(12) {
                        let old = diff.old_lines.get(i);
                        let new = diff.new_lines.get(i);

                        let left = match old {
                            Some(l) => {
                                if l.is_removed {
                                    Span::styled(
                                        format!("- {}", l.content),
                                        Style::default().fg(RED),
                                    )
                                } else {
                                    Span::styled(
                                        format!("  {}", l.content),
                                        Style::default().fg(DIM),
                                    )
                                }
                            }
                            None => Span::styled("  ", Style::default()),
                        };

                        let right = match new {
                            Some(l) => {
                                if l.is_added {
                                    Span::styled(
                                        format!("+ {}", l.content),
                                        Style::default().fg(GREEN),
                                    )
                                } else {
                                    Span::styled(
                                        format!("  {}", l.content),
                                        Style::default().fg(DIM),
                                    )
                                }
                            }
                            None => Span::styled("  ", Style::default()),
                        };

                        lines.push(Line::from(vec![
                            Span::styled("   ", Style::default()),
                            left,
                            Span::styled(" │ ", Style::default().fg(DIM)),
                            right,
                        ]));
                    }
                    if max_lines > 12 {
                        lines.push(Line::from(vec![
                            Span::styled("   ", Style::default()),
                            Span::styled(
                                format!("   ... {} more lines", max_lines - 12),
                                Style::default().fg(DIM),
                            ),
                        ]));
                    }
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("   ", Style::default()),
                        Span::styled(msg.content.clone(), Style::default().fg(ACCENT)),
                    ]));
                }
            }
            MessageRole::System => {
                let preview: String = msg.content.chars().take(100).collect();
                lines.push(Line::from(vec![
                    Span::styled("   ", Style::default()),
                    Span::styled(
                        preview,
                        Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
        }
    }

    // Running spinner
    if app.state == AppState::Running {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = spinner[app.tick_count % spinner.len()];
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {} ", frame),
                Style::default().fg(BRAND),
            ),
            Span::styled("thinking...", Style::default().fg(DIM)),
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

fn render_input_area(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Input box
            Constraint::Length(1), // Model info
        ])
        .split(area);

    // Input box
    let border_color = match app.state {
        AppState::Input => BRAND,
        AppState::Permission => RED,
        _ => DIM,
    };

    let input_content = match app.state {
        AppState::Input => {
            let cursor = if app.tick_count % 20 < 10 { "▏" } else { " " };
            format!("{}{}", app.input, cursor)
        }
        AppState::Permission => {
            if let Some(ref req) = app.permission_pending {
                format!(
                    "Allow {}? (y=allow once, a=allow always, n=reject)",
                    req.tool_name
                )
            } else {
                String::new()
            }
        }
        AppState::Running => "...".to_string(),
        _ => String::new(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .bg(Color::Rgb(25, 25, 30));

    let input = Paragraph::new(input_content)
        .style(Style::default().fg(Color::White))
        .block(block);
    f.render_widget(input, chunks[0]);

    // Model info line
    let model_info = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            "sigmaCode",
            Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(DIM)),
        Span::styled(&app.config.model, Style::default().fg(DIM)),
        Span::styled(" · ", Style::default().fg(DIM)),
        Span::styled(&app.token_display, Style::default().fg(DIM)),
        Span::styled(
            format!(" ({}%)", app.context_usage_pct),
            Style::default().fg(DIM),
        ),
    ]);
    f.render_widget(Paragraph::new(model_info), chunks[1]);
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(40)])
        .split(area);

    let status = match app.state {
        AppState::Idle => " ctrl+c exit",
        AppState::Input => " enter send · esc cancel",
        AppState::Running => " ctrl+c interrupt",
        AppState::Done => " i new task",
        AppState::Permission => " y allow · a always · n reject",
    };

    f.render_widget(
        Paragraph::new(status).style(Style::default().fg(DIM)),
        chunks[0],
    );

    let right = Line::from(vec![
        Span::styled(
            format!("{} ", app.token_display),
            Style::default().fg(DIM),
        ),
        Span::styled(
            format!("${:.2}", app.cost),
            Style::default().fg(DIM),
        ),
    ]);
    f.render_widget(Paragraph::new(right), chunks[1]);
}

fn render_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let bg = Block::default().bg(SIDEBAR_BG);
    let inner = bg.inner(area);
    f.render_widget(bg, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(4),
        ])
        .split(inner);

    // Context header
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

    // Token info
    let ctx = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("Context", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(&app.token_display, Style::default().fg(DIM)),
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
    f.render_widget(ctx, chunks[1]);

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
    let gs = Paragraph::new(vec![
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
                "Press i to start typing",
                Style::default().fg(DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "a task for SigmaCode",
                Style::default().fg(DIM),
            ),
        ]),
    ]);
    f.render_widget(gs, chunks[3]);

    // Path
    let cwd = std::env::current_dir()
        .map(|p| {
            let home = dirs::home_dir()
                .map(|h| p.display().to_string().replace(&h.display().to_string(), "~"))
                .unwrap_or_else(|| p.display().to_string());
            home
        })
        .unwrap_or_else(|_| "~".into());

    let short_path: String = if cwd.len() > 22 {
        format!("...{}", &cwd[cwd.len() - 19..])
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
