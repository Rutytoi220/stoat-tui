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
        let is_focused = self.state.focus == Focus::Input;
        let theme = &self.state.theme;

        // Channel placeholder
        let ch_name = self.state.active_channel().map(|c| c.display_name(&self.state.users, self.state.current_user.as_ref().map(|u| u.id.as_str()))).unwrap_or_else(|| "chat".to_string());
        let placeholder = format!("Message #{}... (Enter to send)", ch_name);

        let char_count = self.state.input_buffer.chars().count();
        let counter_text = format!(" {}/2000 ", char_count);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.focused_border_style(is_focused))
            .title_top(if is_focused { " [INPUT ACTIVE] " } else { "" })
            .title_bottom(counter_text);

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 1 || inner.width < 1 {
            return;
        }

        // Fill inner background
        for y in inner.top()..inner.bottom() {
            for x in inner.left()..inner.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.bg);
            }
        }

        let has_attachments = !self.state.pending_attachments.is_empty();
        let mut input_y = inner.top();

        if has_attachments {
            let mut chip_str = String::from(" [ 📎 ");
            for (idx, att) in self.state.pending_attachments.iter().enumerate() {
                if idx > 0 {
                    chip_str.push_str(", ");
                }
                chip_str.push_str(&att.filename);
            }
            chip_str.push_str(" • Esc/Backspace to clear ]");

            let mut cx = inner.left();
            for ch in chip_str.chars().take(inner.width as usize) {
                buf[(cx, input_y)]
                    .set_char(ch)
                    .set_fg(Color::Rgb(100, 200, 255));
                cx += 1;
            }
            input_y += 1;
        }

        if input_y >= inner.bottom() {
            return;
        }

        let start_x = inner.left();
        let max_w = inner.width as usize;

        if self.state.input_buffer.is_empty() {
            if !is_focused {
                let mut x = start_x;
                for ch in placeholder.chars().take(max_w) {
                    buf[(x, input_y)]
                        .set_char(ch)
                        .set_fg(theme.muted);
                    x += 1;
                }
            } else {
                // Focused but empty: render cursor at beginning
                buf[(start_x, input_y)]
                    .set_char(' ')
                    .set_bg(Color::White)
                    .set_fg(Color::Black);
            }
        } else {
            // Render text
            let buffer = &self.state.input_buffer;
            let cursor_pos = self.state.input_cursor;

            // Simple viewport scrolling if text exceeds width
            let offset = if cursor_pos >= max_w {
                cursor_pos.saturating_sub(max_w) + 5
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
                let cell = &mut buf[(cur_col, input_y)];
                cell.set_char(ch);
                if is_cursor {
                    cell.set_bg(Color::White);
                    cell.set_fg(Color::Black);
                } else {
                    cell.set_fg(Color::White);
                }

                cur_col += 1;
            }

            // If cursor is at the very end of the text
            if is_focused && cursor_pos == chars.len() && cur_col < inner.right() {
                buf[(cur_col, input_y)]
                    .set_char(' ')
                    .set_bg(Color::White)
                    .set_fg(Color::Black);
            }
        }
    }
}
