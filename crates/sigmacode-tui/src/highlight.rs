use ratatui::prelude::*;
use ratatui::style::Color;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use std::sync::OnceLock;

fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(|| SyntaxSet::load_defaults_newlines())
}

fn theme_set() -> &'static ThemeSet {
    static TS: OnceLock<ThemeSet> = OnceLock::new();
    TS.get_or_init(|| ThemeSet::load_defaults())
}

fn syntect_to_color(c: syntect::highlighting::Color) -> ratatui::style::Color {
    Color::Rgb(c.r, c.g, c.b)
}

fn highlight_code_block(code: &str, lang: &str) -> Vec<Line<'static>> {
    let ss = syntax_set();
    let ts = theme_set();

    let syntax = if lang.is_empty() {
        ss.find_syntax_plain_text()
    } else {
        ss.find_syntax_by_token(lang)
            .unwrap_or_else(|| ss.find_syntax_plain_text())
    };

    let theme = &ts.themes["base16-ocean.dark"];
    let mut h = HighlightLines::new(syntax, theme);

    code.lines()
        .map(|line| {
            let ranges = h.highlight_line(line, ss).unwrap_or_default();
            let spans: Vec<Span<'static>> = ranges
                .into_iter()
                .map(|(style, text)| {
                    Span::styled(
                        text.to_string(),
                        Style::default().fg(syntect_to_color(style.foreground)),
                    )
                })
                .collect();
            if spans.is_empty() {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Rgb(200, 200, 200)),
                ))
            } else {
                Line::from(spans)
            }
        })
        .collect()
}

pub fn render_message_with_highlights(content: &str) -> Vec<Line<'static>> {
    let mut result = Vec::new();
    let mut in_code_block = false;
    let mut lang = String::new();
    let mut code_lines = Vec::new();

    // Normalize escaped newlines from LLM output
    let normalized = content.replace("\\n", "\n");

    for line in normalized.lines() {
        if line.starts_with("```") {
            if in_code_block {
                // End of fenced code block — highlight it
                let highlighted = highlight_code_block(&code_lines.join("\n"), &lang);
                result.extend(highlighted);
                code_lines.clear();
                in_code_block = false;
            } else {
                // Start of fenced code block — extract language token
                lang = line[3..].trim().to_string();
                in_code_block = true;
            }
        } else if in_code_block {
            code_lines.push(line);
        } else {
            // Regular text — render as-is with default color
            result.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Rgb(200, 200, 200)),
            )));
        }
    }

    // Flush remaining code block
    if in_code_block && !code_lines.is_empty() {
        let highlighted = highlight_code_block(&code_lines.join("\n"), &lang);
        result.extend(highlighted);
    }

    result
}
