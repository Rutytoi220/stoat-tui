use crate::state::{AppState, Focus};
use crate::ui::markdown::MarkdownParser;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

pub struct ChatFeed<'a> {
    state: &'a AppState,
}

impl<'a> ChatFeed<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for ChatFeed<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let is_focused = self.state.focus == Focus::Chat;
        let theme = &self.state.theme;
        let parser = MarkdownParser::default();

        // Fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.bg);
            }
        }

        // Draw border if focused
        if is_focused {
            let block = Block::default()
                .borders(Borders::NONE)
                .border_style(theme.focused_border_style(is_focused));
            block.render(area, buf);
        }

        if area.width < 10 || area.height < 2 {
            return;
        }

        let channel_id = match &self.state.active_channel_id {
            Some(cid) => cid,
            None => {
                let empty_line = Line::from(vec![
                    Span::styled("No channel selected. Select a channel from the left sidebar.", Style::default().fg(theme.muted)),
                ]);
                buf.set_line(area.left() + 2, area.top() + 2, &empty_line, area.width - 4);
                return;
            }
        };

        let messages = self.state.messages.get(channel_id);
        let empty_vec = Vec::new();
        let msgs = messages.unwrap_or(&empty_vec);

        if msgs.is_empty() {
            let ch_name = self.state.active_channel().map(|c| c.display_name(&self.state.users, self.state.current_user.as_ref().map(|u| u.id.as_str()))).unwrap_or_else(|| "this channel".to_string());
            let line1 = Line::from(vec![
                Span::styled(format!("Welcome to #{}!", ch_name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]);
            let line2 = Line::from(vec![
                Span::styled("This is the start of the channel history. Send a message below to begin chatting!", Style::default().fg(theme.muted)),
            ]);
            buf.set_line(area.left() + 2, area.top() + 2, &line1, area.width - 4);
            buf.set_line(area.left() + 2, area.top() + 4, &line2, area.width - 4);
            return;
        }

        // Pre-render all messages into formatted lines
        let mut rendered_lines: Vec<Line<'static>> = Vec::new();

        for msg in msgs {
            // 1. Author header line
            let user = self.state.users.get(&msg.author);
            let author_name = if let Some(ref m) = msg.masquerade {
                m.name.clone().unwrap_or_else(|| "Anonymous".to_string())
            } else if let Some(u) = user {
                u.name().to_string()
            } else {
                format!("User {}", &msg.author[..msg.author.len().min(6)])
            };

            let author_color = theme.role_color(&msg.author);

            // Simple timestamp from message or dummy
            let time_str = "12:30"; // Formatted timestamp

            let mut header_spans = vec![
                Span::styled(format!("[{}] ", time_str), Style::default().fg(Color::Rgb(90, 90, 100))),
                Span::styled(author_name, Style::default().fg(author_color).add_modifier(Modifier::BOLD)),
            ];

            // Bot badge
            if let Some(u) = user {
                if u.bot.is_some() {
                    header_spans.push(Span::styled(" BOT", Style::default().fg(Color::White).bg(theme.accent).add_modifier(Modifier::BOLD)));
                }
            }

            rendered_lines.push(Line::from(header_spans));

            // 2. Content lines
            let content = msg.content.as_deref().unwrap_or("");
            let content_lines = parser.parse_message(content);
            for mut cl in content_lines {
                let mut indented = vec![Span::styled("  ", Style::default())];
                indented.append(&mut cl.spans);
                rendered_lines.push(Line::from(indented));
            }

            // 3. Reactions
            if let Some(ref reactions) = msg.reactions {
                if !reactions.is_empty() {
                    let mut rx_spans = vec![Span::styled("  ", Style::default())];
                    for (emoji, uids) in reactions {
                        let rx_str = format!(" [ {} {} ] ", emoji, uids.len());
                        rx_spans.push(Span::styled(
                            rx_str,
                            Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 50)),
                        ));
                    }
                    rendered_lines.push(Line::from(rx_spans));
                }
            }

            // Message separator spacing
            rendered_lines.push(Line::from(String::new()));
        }

        let total_lines = rendered_lines.len();
        let visible_height = area.height as usize;

        // Calculate scroll slice
        // offset = 0: bottom of rendered_lines is at the bottom of the viewport
        let offset = self.state.chat_scroll_offset.min(total_lines.saturating_sub(visible_height));

        let end_idx = total_lines.saturating_sub(offset);
        let start_idx = end_idx.saturating_sub(visible_height);

        let lines_to_draw = &rendered_lines[start_idx..end_idx];

        for (i, line) in lines_to_draw.iter().enumerate() {
            let y = area.top() + i as u16;
            if y < area.bottom() {
                buf.set_line(area.left() + 1, y, line, area.width.saturating_sub(2));
            }
        }

        // Draw scrollbar indicator on the right edge if backlog is scrollable
        if total_lines > visible_height {
            let scrollbar_x = area.right().saturating_sub(1);
            let track_height = area.height as usize;
            let thumb_height = ((visible_height as f32 / total_lines as f32) * track_height as f32).max(1.0) as usize;
            let max_scroll = total_lines.saturating_sub(visible_height);
            let thumb_pos = if max_scroll == 0 {
                0
            } else {
                ((max_scroll.saturating_sub(offset) as f32 / max_scroll as f32) * (track_height.saturating_sub(thumb_height)) as f32) as usize
            };

            for y_off in 0..track_height {
                let y = area.top() + y_off as u16;
                let is_thumb = y_off >= thumb_pos && y_off < thumb_pos + thumb_height;
                let ch = if is_thumb { '█' } else { '│' };
                let color = if is_thumb { theme.primary } else { Color::Rgb(40, 40, 45) };
                buf[(scrollbar_x, y)].set_char(ch).set_fg(color);
            }
        }
    }
}
