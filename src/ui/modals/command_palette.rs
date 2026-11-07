use crate::state::{AppState, PaletteItemType};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub struct CommandPaletteModal<'a> {
    state: &'a AppState,
}

impl<'a> CommandPaletteModal<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for CommandPaletteModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_command_palette {
            return;
        }

        let theme = &self.state.theme;

        // Clear the popup area
        Clear.render(area, buf);

        // Fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.sidebar_bg);
            }
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
            .title(" Quick Switcher (Ctrl+P) ")
            .title_bottom(" ↑/↓: Navigate • Enter: Jump • Esc: Close ");

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        let start_x = inner.left() + 1;
        let max_w = inner.width.saturating_sub(2) as usize;
        let mut cur_y = inner.top();

        // 1. Search Query Box
        let prompt = "🔍 ";
        let mut qx = start_x;
        for ch in prompt.chars() {
            buf[(qx, cur_y)].set_char(ch).set_fg(theme.primary);
            qx += 1;
        }

        let query = &self.state.command_palette_query;
        if query.is_empty() {
            let placeholder = "Type a channel, server, or user name...";
            for ch in placeholder.chars().take(max_w.saturating_sub(4)) {
                buf[(qx, cur_y)].set_char(ch).set_fg(theme.muted);
                qx += 1;
            }
        } else {
            for ch in query.chars().take(max_w.saturating_sub(4)) {
                buf[(qx, cur_y)]
                    .set_char(ch)
                    .set_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
                qx += 1;
            }
        }

        // Render cursor in query box
        buf[(qx, cur_y)]
            .set_char(' ')
            .set_bg(Color::White)
            .set_fg(Color::Black);

        cur_y += 1;

        // Divider
        for dx in inner.left()..inner.right() {
            buf[(dx, cur_y)]
                .set_char('─')
                .set_fg(Color::Rgb(50, 50, 60));
        }

        cur_y += 1;

        // 2. Results List
        let items = self.state.get_palette_items();
        let selected_idx = self.state.command_palette_selected;

        let visible_count = inner.bottom().saturating_sub(cur_y) as usize;
        let offset = if selected_idx >= visible_count {
            selected_idx.saturating_sub(visible_count) + 1
        } else {
            0
        };

        if items.is_empty() {
            let no_res = "No matching channels or servers found.";
            let mut nx = start_x;
            for ch in no_res.chars() {
                buf[(nx, cur_y + 1)].set_char(ch).set_fg(theme.muted);
                nx += 1;
            }
            return;
        }

        for (idx, item) in items.iter().enumerate().skip(offset) {
            if cur_y >= inner.bottom() {
                break;
            }

            let is_selected = idx == selected_idx;
            let row_bg = if is_selected {
                theme.item_selected_bg
            } else {
                theme.sidebar_bg
            };

            // Highlight full line
            for rx in inner.left()..inner.right() {
                buf[(rx, cur_y)].set_bg(row_bg);
            }

            // Tag badge: [Server] or [# Channel] or [@ DM]
            let (tag, tag_color) = match item.item_type {
                PaletteItemType::Server => ("[SERVER]", Color::Rgb(150, 150, 250)),
                PaletteItemType::Channel => ("[CHANNEL]", Color::Cyan),
                PaletteItemType::DirectMessage => ("[DM]", theme.primary),
            };

            let mut tx = start_x;
            for ch in tag.chars() {
                buf[(tx, cur_y)]
                    .set_char(ch)
                    .set_fg(tag_color)
                    .set_style(Modifier::BOLD);
                tx += 1;
            }

            tx += 2;

            // Title
            let title_style = if is_selected {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(220, 220, 230))
            };

            for ch in item.title.chars().take(30) {
                buf[(tx, cur_y)]
                    .set_char(ch)
                    .set_style(title_style);
                tx += 1;
            }

            tx += 2;

            // Subtitle
            for ch in item.subtitle.chars().take(max_w.saturating_sub(tx as usize - start_x as usize)) {
                buf[(tx, cur_y)]
                    .set_char(ch)
                    .set_fg(theme.muted);
                tx += 1;
            }

            cur_y += 1;
        }
    }
}
