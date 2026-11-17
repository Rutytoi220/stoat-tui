use crate::state::{AppState, Focus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::{Block, BorderType, Borders, Widget};

pub struct MessageInput<'a> {
    state: &'a AppState,
}

impl<'a> MessageInput<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for MessageInput<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let is_focused = self.state.focus == Focus::Input;
        let theme = &self.state.theme;
        let borderless = area.width < 24 || area.height < 5;

        let ch_name = self
            .state
            .active_channel()
            .map(|c| c.display_name(&self.state.users, self.state.current_user.as_ref().map(|u| u.id.as_str())))
            .unwrap_or_else(|| "chat".to_string());
        let placeholder = if area.width < 24 {
            format!("> #{}", ch_name)
        } else {
            format!("Message #{}... (Enter to send)", ch_name)
        };

        let surface_bg = Color::Rgb(32, 34, 37);
        let border_style = if is_focused {
            ratatui::style::Style::default().fg(theme.accent)
        } else {
            ratatui::style::Style::default().fg(theme.border_unfocused)
        };

        let inner = if borderless {
            area
        } else {
            let char_count = self.state.input_buffer.chars().count();
            let counter_text = format!(" {}/2000 ", char_count);
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .style(ratatui::style::Style::default().bg(surface_bg))
                .title_top(if is_focused { " ✎ Message " } else { "" })
                .title_bottom(ratatui::text::Line::from(counter_text).right_aligned());
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        };

        if inner.height < 1 || inner.width < 1 {
            return;
        }

        // Fill inner background with subtle dark surface
        for y in inner.top()..inner.bottom() {
            for x in inner.left()..inner.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(surface_bg);
            }
        }

        let has_attachments = !self.state.pending_attachments.is_empty();
        let mut input_y = inner.top();

        if has_attachments && inner.height >= 2 {
            let mut chip_str = String::from("📎 ");
            for (idx, att) in self.state.pending_attachments.iter().enumerate() {
                if idx > 0 {
                    chip_str.push_str(", ");
                }
                chip_str.push_str(&att.filename);
            }
            crate::ui::write_str(
                buf,
                inner,
                inner.left(),
                input_y,
                &chip_str,
                ratatui::style::Style::default().fg(Color::Rgb(100, 200, 255)),
            );
            input_y += 1;
        }

        if input_y >= inner.bottom() {
            return;
        }

        let start_x = inner.left();
        let max_w = inner.width as usize;

        if self.state.input_buffer.is_empty() {
            if !is_focused {
                crate::ui::write_str(
                    buf,
                    inner,
                    start_x,
                    input_y,
                    &placeholder,
                    ratatui::style::Style::default().fg(theme.muted),
                );
            } else {
                crate::ui::put_cell(
                    buf,
                    inner,
                    start_x,
                    input_y,
                    ' ',
                    ratatui::style::Style::default().bg(Color::White).fg(Color::Black),
                );
                if borderless && inner.width >= 5 {
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x + 1,
                        input_y,
                        &format!(" #{}", ch_name),
                        ratatui::style::Style::default().fg(theme.muted),
                    );
                }
            }
        } else {
            let buffer = &self.state.input_buffer;
            let cursor_pos = self.state.input_cursor;

            let offset = if cursor_pos >= max_w {
                cursor_pos.saturating_sub(max_w) + 1
            } else {
                0
            };

            let chars: Vec<char> = buffer.chars().collect();
            let mut cur_col = start_x;

            for (idx, &ch) in chars.iter().enumerate().skip(offset) {
                if cur_col >= inner.right() {
                    break;
                }
                let is_cursor = is_focused && idx == cursor_pos;
                let style = if is_cursor {
                    ratatui::style::Style::default().bg(Color::White).fg(Color::Black)
                } else {
                    ratatui::style::Style::default().fg(Color::White)
                };
                crate::ui::put_cell(buf, inner, cur_col, input_y, ch, style);
                cur_col += 1;
            }

            if is_focused && cursor_pos == chars.len() && cur_col < inner.right() {
                crate::ui::put_cell(
                    buf,
                    inner,
                    cur_col,
                    input_y,
                    ' ',
                    ratatui::style::Style::default().bg(Color::White).fg(Color::Black),
                );
            }
        }
    }
}
