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

pub fn highlight_code_block(code: &str, lang: &str) -> Vec<Line<'static>> {
    let ss = syntax_set();
    let ts = theme_set();

    let syntax = ss
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| ss.find_syntax_plain_text());

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
            Line::from(spans)
        })
        .collect()
}

fn looks_like_jsx(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() { return false; }
    if trimmed.starts_with('<') && !trimmed.starts_with("<!--") { return true; }
    if trimmed.starts_with('{') && trimmed.contains('}') { return true; }
    if trimmed.contains("className=") || trimmed.contains("onClick=") { return true; }
    if trimmed.contains("viewBox=") || trimmed.contains("fill=\"") { return true; }
    if trimmed.contains("import ") && trimmed.contains("from ") { return true; }
    if trimmed.contains("export ") && (trimmed.contains("function ") || trimmed.contains("default ")) { return true; }
    if trimmed.starts_with("const ") || trimmed.starts_with("let ") || trimmed.starts_with("function ") { return true; }
    if trimmed.starts_with("return (") || trimmed.starts_with("return(") { return true; }
    if trimmed.contains("useState") || trimmed.contains("useEffect") { return true; }
    false
}

pub fn render_message_with_highlights(content: &str) -> Vec<Line<'static>> {
    let mut result = Vec::new();
    let mut in_code_block = false;
    let mut in_auto_code = false;
    let mut lang = String::new();
    let mut code_lines = Vec::new();

    // Normalize escaped newlines from LLM output
    let normalized = content.replace("\\n", "\n");

    for line in normalized.lines() {
        if line.starts_with("```") {
            if in_code_block {
                let highlighted = highlight_code_block(&code_lines.join("\n"), &lang);
                result.extend(highlighted);
                code_lines.clear();
                in_code_block = false;
            } else {
                lang = line.trim_start_matches('`').trim().to_string();
                in_code_block = true;
            }
        } else if in_code_block {
            code_lines.push(line);
        } else if looks_like_jsx(line) {
            if !in_auto_code {
                in_auto_code = true;
                code_lines.clear();
            }
            code_lines.push(line);
        } else {
            if in_auto_code && !code_lines.is_empty() {
                let highlighted = highlight_code_block(&code_lines.join("\n"), "jsx");
                result.extend(highlighted);
                code_lines.clear();
                in_auto_code = false;
            }
            result.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Rgb(200, 200, 200)),
            )));
        }
    }

    if in_code_block && !code_lines.is_empty() {
        let highlighted = highlight_code_block(&code_lines.join("\n"), &lang);
        result.extend(highlighted);
    }
    if in_auto_code && !code_lines.is_empty() {
        let highlighted = highlight_code_block(&code_lines.join("\n"), "jsx");
        result.extend(highlighted);
    }

    result
}
