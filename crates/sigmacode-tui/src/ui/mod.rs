use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::{App, AppState, MessageRole, Tab};

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    render_header(f, app, chunks[0]);

    match app.current_tab {
        Tab::Chat => render_chat(f, app, chunks[1]),
        Tab::Logs => render_logs(f, app, chunks[1]),
    }

    render_footer(f, app, chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let header = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(30)])
        .split(area);

    let title = Paragraph::new(" SigmaCode v0.1.0")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(title, header[0]);

    let model_info = Paragraph::new(format!(" Model: {}", app.config.model))
        .style(Style::default().fg(Color::Gray));
    f.render_widget(model_info, header[1]);

    let border = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(border, area);
}

fn render_chat(f: &mut Frame, app: &App, area: Rect) {
    let mut messages = Vec::new();

    for msg in &app.messages {
        let (prefix, style) = match msg.role {
            MessageRole::User => ("> ", Style::default().fg(Color::Green)),
            MessageRole::Assistant => ("", Style::default().fg(Color::White)),
            MessageRole::System => ("[sys] ", Style::default().fg(Color::Yellow)),
            MessageRole::Tool => ("[tool] ", Style::default().fg(Color::Blue)),
        };

        let content = format!("{}{}", prefix, msg.content);
        let lines: Vec<Line<'static>> = content
            .lines()
            .map(|l| Line::from(l.to_owned()).style(style))
            .collect();

        messages.push(ListItem::new(lines));
    }

    let messages_list = List::new(messages)
        .block(Block::default().borders(Borders::ALL).title("Chat"));

    f.render_widget(messages_list, area);
}

fn render_logs(f: &mut Frame, app: &App, area: Rect) {
    let logs: Vec<ListItem<'static>> = app
        .logs
        .iter()
        .map(|log| {
            ListItem::new(Line::from(log.clone()).style(Style::default().fg(Color::Gray)))
        })
        .collect();

    let logs_list = List::new(logs)
        .block(Block::default().borders(Borders::ALL).title("Logs"));

    f.render_widget(logs_list, area);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let footer_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0)])
        .split(area);

    match app.state {
        AppState::Idle => {
            let help = Paragraph::new(" Press [i] to type, [l] logs, [q] quit")
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(help, footer_layout[0]);
        }
        AppState::Input => {
            let input = Paragraph::new(app.input.as_str())
                .style(Style::default().fg(Color::White))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Task")
                        .border_style(Style::default().fg(Color::Green)),
                );
            f.render_widget(input, footer_layout[0]);
        }
        AppState::Running => {
            let status = Paragraph::new(" Agent running... (Ctrl+C to cancel)")
                .style(Style::default().fg(Color::Yellow));
            f.render_widget(status, footer_layout[0]);
        }
        AppState::Done => {
            let status = Paragraph::new(" Done. Press [i] for new task, [q] quit")
                .style(Style::default().fg(Color::Green));
            f.render_widget(status, footer_layout[0]);
        }
    }
}
