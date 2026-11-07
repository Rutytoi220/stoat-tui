use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub struct MarkdownParser {
    pub code_bg: Color,
    pub code_fg: Color,
    pub quote_bar: Color,
    pub link_fg: Color,
    pub mention_fg: Color,
    pub bold_fg: Color,
    pub text_fg: Color,
}

impl Default for MarkdownParser {
    fn default() -> Self {
        Self {
            code_bg: Color::Rgb(35, 35, 40),
            code_fg: Color::Rgb(220, 220, 150),
            quote_bar: Color::Rgb(89, 112, 253),
            link_fg: Color::Rgb(100, 180, 255),
            mention_fg: Color::Rgb(253, 102, 113),
            bold_fg: Color::White,
            text_fg: Color::Rgb(240, 240, 240),
        }
    }
}

impl MarkdownParser {
    pub fn parse_message(&self, content: &str) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let raw_lines: Vec<&str> = content.lines().collect();
        let mut in_code_block = false;

        for raw_line in raw_lines {
            let trimmed = raw_line.trim_start();
            if trimmed.starts_with("```") {
                if in_code_block {
                    in_code_block = false;
                    lines.push(Line::from(vec![
                        Span::styled("  └───", Style::default().fg(Color::DarkGray)),
                    ]));
                    continue;
                } else {
                    in_code_block = true;
                    let code_lang = trimmed.trim_start_matches("```").trim();
                    let lang_tag = if code_lang.is_empty() {
                        "code".to_string()
                    } else {
                        code_lang.to_string()
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  ┌───[ {} ]", lang_tag), Style::default().fg(Color::DarkGray)),
                    ]));
                    continue;
                }
            }

            if in_code_block {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        raw_line.to_string(),
                        Style::default().fg(self.code_fg).bg(self.code_bg),
                    ),
                ]));
                continue;
            }

            // Blockquote
            if trimmed.starts_with('>') {
                let quote_content = trimmed.strip_prefix('>').unwrap_or("").trim_start();
                let mut spans = vec![
                    Span::styled("▎ ", Style::default().fg(self.quote_bar).add_modifier(Modifier::BOLD)),
                ];
                spans.extend(self.parse_inline_spans(quote_content, true));
                lines.push(Line::from(spans));
                continue;
            }

            // Normal line with inline markdown
            let spans = self.parse_inline_spans(raw_line, false);
            lines.push(Line::from(spans));
        }

        if lines.is_empty() {
            lines.push(Line::from(String::new()));
        }

        lines
    }

    fn parse_inline_spans(&self, text: &str, in_quote: bool) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = 0;
        let mut cur_text = String::new();

        let base_style = if in_quote {
            Style::default().fg(Color::Rgb(190, 190, 200)).add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(self.text_fg)
        };

        while i < len {
            // Inline code `...`
            if chars[i] == '`' {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(cur_text.clone(), base_style));
                    cur_text.clear();
                }
                i += 1;
                let mut code = String::new();
                while i < len && chars[i] != '`' {
                    code.push(chars[i]);
                    i += 1;
                }
                if i < len && chars[i] == '`' {
                    i += 1;
                }
                spans.push(Span::styled(
                    format!(" {} ", code),
                    Style::default().fg(self.code_fg).bg(self.code_bg),
                ));
                continue;
            }

            // Bold **...**
            if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(cur_text.clone(), base_style));
                    cur_text.clear();
                }
                i += 2;
                let mut bold_content = String::new();
                while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                    bold_content.push(chars[i]);
                    i += 1;
                }
                if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
                    i += 2;
                }
                spans.push(Span::styled(
                    bold_content,
                    base_style.fg(self.bold_fg).add_modifier(Modifier::BOLD),
                ));
                continue;
            }

            // Italic *...*
            if chars[i] == '*' {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(cur_text.clone(), base_style));
                    cur_text.clear();
                }
                i += 1;
                let mut italic_content = String::new();
                while i < len && chars[i] != '*' {
                    italic_content.push(chars[i]);
                    i += 1;
                }
                if i < len && chars[i] == '*' {
                    i += 1;
                }
                spans.push(Span::styled(
                    italic_content,
                    base_style.add_modifier(Modifier::ITALIC),
                ));
                continue;
            }

            // Strikethrough ~~...~~
            if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(cur_text.clone(), base_style));
                    cur_text.clear();
                }
                i += 2;
                let mut strike_content = String::new();
                while i + 1 < len && !(chars[i] == '~' && chars[i + 1] == '~') {
                    strike_content.push(chars[i]);
                    i += 1;
                }
                if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
                    i += 2;
                }
                spans.push(Span::styled(
                    strike_content,
                    base_style.add_modifier(Modifier::CROSSED_OUT),
                ));
                continue;
            }

            // URLs (http:// or https://)
            let remaining: String = chars[i..].iter().collect();
            if remaining.starts_with("http://") || remaining.starts_with("https://") {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(cur_text.clone(), base_style));
                    cur_text.clear();
                }
                let mut url = String::new();
                while i < len && !chars[i].is_whitespace() && chars[i] != '>' && chars[i] != ')' {
                    url.push(chars[i]);
                    i += 1;
                }
                spans.push(Span::styled(
                    url,
                    Style::default().fg(self.link_fg).add_modifier(Modifier::UNDERLINED),
                ));
                continue;
            }

            // Mentions: @something or <@ID>
            if chars[i] == '@' {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(cur_text.clone(), base_style));
                    cur_text.clear();
                }
                let mut mention = String::new();
                while i < len && (chars[i].is_alphanumeric() || chars[i] == '@' || chars[i] == '_' || chars[i] == '-') {
                    mention.push(chars[i]);
                    i += 1;
                }
                spans.push(Span::styled(
                    mention,
                    Style::default().fg(self.mention_fg).add_modifier(Modifier::BOLD),
                ));
                continue;
            }

            // Channels: #something or <#ID>
            if chars[i] == '#' {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(cur_text.clone(), base_style));
                    cur_text.clear();
                }
                let mut ch = String::new();
                while i < len && (chars[i].is_alphanumeric() || chars[i] == '#' || chars[i] == '-' || chars[i] == '_') {
                    ch.push(chars[i]);
                    i += 1;
                }
                spans.push(Span::styled(
                    ch,
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ));
                continue;
            }

            cur_text.push(chars[i]);
            i += 1;
        }

        if !cur_text.is_empty() {
            spans.push(Span::styled(cur_text, base_style));
        }

        spans
    }
}
