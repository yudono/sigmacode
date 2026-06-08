use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, AppState, MessageRole, Tab};
use crate::highlight;

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
        Tab::Chat => {
            if app.state == AppState::Setup {
                render_setup(f, app, content_chunks[0]);
            } else {
                render_chat(f, app, content_chunks[0]);
            }
        }
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
                let highlighted = highlight::render_message_with_highlights(&msg.content);
                lines.extend(highlighted);
            }
            MessageRole::Tool => {
                let content = msg.content.clone();
                if msg.tool_output {
                    // Tool output (bash output, file content, etc.) - add highlighting
                    let highlighted = highlight::render_message_with_highlights(&content);
                    for mut line in highlighted {
                        // Indent tool output
                    let indent = vec![Span::styled("   ", Style::default())];
                    let spans: Vec<Span<'static>> = indent.into_iter().chain(line.spans.into_iter()).collect();
                    lines.push(Line::from(spans));
                    }
                } else {
                    // Tool call display
                    let (icon, text) = if content.starts_with("  $ ") {
                        ("◆ ", format!("$ {}", &content[4..]))
                    } else if content.starts_with("  read ") {
                        ("◆ ", format!("read {}", &content[7..]))
                    } else if content.starts_with("  write ") {
                        ("◆ ", format!("write {}", &content[8..]))
                    } else if content.starts_with("  edit ") {
                        ("◆ ", format!("edit {}", &content[7..]))
                    } else if content.starts_with("  glob ") {
                        ("◆ ", format!("glob {}", &content[7..]))
                    } else if content.starts_with("  grep ") {
                        ("◆ ", format!("grep {}", &content[7..]))
                    } else {
                        ("◆ ", content)
                    };
                    lines.push(Line::from(vec![
                        Span::styled("   ", Style::default()),
                        Span::styled(icon, Style::default().fg(ACCENT)),
                        Span::styled(text, Style::default().fg(DIM)),
                    ]));
                }
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

    let total_lines = lines.len() as u16;
    let area_height = area.height;
    let effective_scroll = if app.follow {
        total_lines.saturating_sub(area_height)
    } else {
        app.scroll_offset as u16
    };
    let paragraph = Paragraph::new(lines).scroll((effective_scroll, 0));
    f.render_widget(paragraph, area);
}

fn render_setup(f: &mut Frame, app: &App, area: Rect) {
    use crate::app::SetupStep;

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "  sigmaCode Setup",
            Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    match app.setup.step {
        SetupStep::Welcome => {
            lines.push(Line::from(Span::styled(
                "  Welcome to sigmaCode!",
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(Span::styled(
                "  Let's set up your AI coding assistant.",
                Style::default().fg(DIM),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Press Enter to continue...",
                Style::default().fg(DIM),
            )));
        }
        SetupStep::Provider => {
            lines.push(Line::from(Span::styled(
                "  Choose your AI provider:",
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  1. ", Style::default().fg(BRAND)),
                Span::styled("OpenAI", Style::default().fg(Color::White)),
                Span::styled(" (gpt-4o, gpt-4.1)", Style::default().fg(DIM)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  2. ", Style::default().fg(BRAND)),
                Span::styled("Anthropic", Style::default().fg(Color::White)),
                Span::styled(" (claude-sonnet-4)", Style::default().fg(DIM)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  3. ", Style::default().fg(BRAND)),
                Span::styled("Ollama", Style::default().fg(Color::White)),
                Span::styled(" (local models)", Style::default().fg(DIM)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  4. ", Style::default().fg(BRAND)),
                Span::styled("Gemini", Style::default().fg(Color::White)),
                Span::styled(" (gemini-2.0-flash)", Style::default().fg(DIM)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  5. ", Style::default().fg(BRAND)),
                Span::styled("MiMo", Style::default().fg(Color::White)),
                Span::styled(" (mimo-v2.5)", Style::default().fg(DIM)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Type 1-5 or provider name, then Enter",
                Style::default().fg(DIM),
            )));
        }
        SetupStep::ApiKey => {
            let provider_display = match app.setup.provider_choice.as_str() {
                "openai" => "OpenAI".to_string(),
                "anthropic" => "Anthropic".to_string(),
                "gemini" => "Gemini".to_string(),
                other => other.to_string(),
            };
            lines.push(Line::from(vec![
                Span::styled("  Provider: ", Style::default().fg(DIM)),
                Span::styled(
                    provider_display,
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Enter your API key:",
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(""));
            // Show masked input
            let masked = if app.input.is_empty() {
                "  (waiting for input...)".to_string()
            } else {
                format!("  {}{}", "*".repeat(app.input.len().min(20)), if app.input.len() > 20 { "..." } else { "" })
            };
            lines.push(Line::from(Span::styled(
                masked,
                Style::default().fg(DIM),
            )));
        }
        SetupStep::BaseUrl => {
            let default_url = match app.setup.provider_choice.as_str() {
                "ollama" => "http://localhost:11434",
                "openai" => "https://api.openai.com/v1",
                "anthropic" => "https://api.anthropic.com",
                "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai",
                _ => "",
            };
            let provider_display = match app.setup.provider_choice.as_str() {
                "openai" => "OpenAI".to_string(),
                "anthropic" => "Anthropic".to_string(),
                "ollama" => "Ollama".to_string(),
                "gemini" => "Gemini".to_string(),
                other => other.to_string(),
            };
            lines.push(Line::from(vec![
                Span::styled("  Provider: ", Style::default().fg(DIM)),
                Span::styled(
                    provider_display,
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Enter base URL (or press Enter for default):",
                Style::default().fg(Color::White),
            )));
            if !default_url.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  Default: {}", default_url),
                    Style::default().fg(DIM),
                )));
            }
            lines.push(Line::from(Span::styled(
                "  Use this for custom/proxy endpoints",
                Style::default().fg(DIM),
            )));
            lines.push(Line::from(""));
            let input_display = if app.input.is_empty() {
                "  (press Enter to skip)".to_string()
            } else {
                format!("  {}", app.input)
            };
            lines.push(Line::from(Span::styled(
                input_display,
                Style::default().fg(Color::Rgb(200, 200, 200)),
            )));
        }
        SetupStep::Workspace => {
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "~".into());
            lines.push(Line::from(vec![
                Span::styled("  Base URL: ", Style::default().fg(DIM)),
                Span::styled(
                    app.setup.base_url.clone(),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Enter workspace path (or press Enter for current dir):",
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(Span::styled(
                format!("  Default: {}", cwd),
                Style::default().fg(DIM),
            )));
            lines.push(Line::from(Span::styled(
                "  The agent will read/write files here",
                Style::default().fg(DIM),
            )));
            lines.push(Line::from(""));
            let input_display = if app.input.is_empty() {
                "  (press Enter to skip)".to_string()
            } else {
                format!("  {}", app.input)
            };
            lines.push(Line::from(Span::styled(
                input_display,
                Style::default().fg(Color::Rgb(200, 200, 200)),
            )));
        }
        SetupStep::Done => {}
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
    let dropdown_visible = !app.cmd_choices.is_empty();
    let dropdown_height = if dropdown_visible {
        (8.min(app.cmd_choices.len()) as u16) + 2 // +2 for borders
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(dropdown_height), // autocomplete dropdown
            Constraint::Length(3),               // Input box
            Constraint::Length(1),               // Model info
        ])
        .split(area);

    // Command autocomplete dropdown
    if dropdown_visible {
        let max_visible = 8.min(app.cmd_choices.len());
        let items: Vec<Line<'static>> = app.cmd_choices.iter().enumerate().take(max_visible).map(|(i, cmd)| {
            let style = if i == app.cmd_selected {
                Style::default().fg(Color::Black).bg(BRAND).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(vec![
                Span::styled(format!("  {:12}", cmd.name), style),
                Span::styled(
                    cmd.desc.to_string(),
                    if i == app.cmd_selected {
                        Style::default().fg(Color::Black).bg(BRAND)
                    } else {
                        Style::default().fg(DIM)
                    },
                ),
            ])
        }).collect();

        let dropdown = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BRAND))
            .bg(Color::Rgb(30, 30, 35))
            .title(" Commands ");

        let list = Paragraph::new(items).block(dropdown);
        f.render_widget(list, chunks[0]);
    }

    // Input box
    let border_color = match app.state {
        AppState::Input => BRAND,
        AppState::Permission => RED,
        AppState::Setup => ACCENT,
        _ => DIM,
    };

    let input_content = match app.state {
        AppState::Input => {
            let cursor = if app.tick_count % 20 < 10 { "▏" } else { " " };
            format!("{}{}", app.input, cursor)
        }
        AppState::Setup => {
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
        AppState::Running => {
            let cursor = if app.tick_count % 20 < 10 { "▏" } else { " " };
            if !app.input.is_empty() {
                let q = if !app.queue.is_empty() {
                    format!(" [{} queued]", app.queue.len())
                } else {
                    String::new()
                };
                format!("{}{}{}", app.input, cursor, q)
            } else if !app.queue.is_empty() {
                let cursor = if app.tick_count % 20 < 10 { "▏" } else { " " };
                format!("{}[{} queued]", cursor, app.queue.len())
            } else {
                "...".to_string()
            }
        }
        _ => String::new(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .bg(Color::Rgb(25, 25, 30));

    let input = Paragraph::new(input_content)
        .style(Style::default().fg(Color::White))
        .block(block);
    f.render_widget(input, chunks[1]);

    // Model info line
    let model_info = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            "sigmaCode",
            Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(DIM)),
        Span::styled(&app.sigma_config.model, Style::default().fg(DIM)),
        Span::styled(" · ", Style::default().fg(DIM)),
        Span::styled(&app.token_display, Style::default().fg(DIM)),
        Span::styled(
            format!(" ({}%)", app.context_usage_pct),
            Style::default().fg(DIM),
        ),
    ]);
    f.render_widget(Paragraph::new(model_info), chunks[2]);
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
        AppState::Setup => " enter confirm · esc back",
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
    let status_text = if !app.queue.is_empty() {
        format!("  {} queued", app.queue.len())
    } else {
        "  No active task".to_string()
    };
    let title = Paragraph::new(vec![
        Line::from(Span::styled(
            " Context",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(status_text, Style::default().fg(DIM))),
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
    let cwd = app.sigma_config.workspace.as_ref()
        .map(|w| {
            let home = dirs::home_dir()
                .map(|h| w.replace(&h.display().to_string(), "~"))
                .unwrap_or_else(|| w.clone());
            home
        })
        .or_else(|| {
            std::env::current_dir().ok().map(|p| {
                let home = dirs::home_dir()
                    .map(|h| p.display().to_string().replace(&h.display().to_string(), "~"))
                    .unwrap_or_else(|| p.display().to_string());
                home
            })
        })
        .unwrap_or_else(|| "~".into());

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
