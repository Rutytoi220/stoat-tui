use crate::state::{ActiveRail, AppState, Focus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

pub struct ServersRail<'a> {
    state: &'a AppState,
}

impl<'a> ServersRail<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for ServersRail<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let is_focused = self.state.focus == Focus::Servers;
        let theme = &self.state.theme;

        // Render rail background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.rail_bg);
            }
        }

        // Draw border if focused or subtle separator
        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(theme.focused_border_style(is_focused));
        block.render(area, buf);

        if area.width < 3 || area.height < 4 {
            return;
        }

        let inner_width = area.width.saturating_sub(1); // Exclude right border
        let start_x = area.left();
        let mut cur_y = area.top() + 1;

        // 1. Direct Messages Icon button (@)
        let is_dm_active = matches!(self.state.active_rail, ActiveRail::DirectMessages);
        let dm_unread: usize = self.state.dms.iter().map(|d| self.state.unread_counts.get(&d.id).copied().unwrap_or(0)).sum();

        let dm_style = if is_dm_active {
            Style::default().fg(Color::White).bg(theme.item_selected_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.muted).bg(theme.rail_bg)
        };

        // Left indicator bar for active item
        if is_dm_active {
            buf[(start_x, cur_y)]
                .set_char('▌')
                .set_fg(theme.primary)
                .set_style(Modifier::BOLD);
        }

        let dm_text = if inner_width >= 6 { "  @   " } else { " @ " };
        let text_start_x = start_x + 1;
        let mut col_x = text_start_x;
        for ch in dm_text.chars() {
            if col_x < start_x + inner_width {
                buf[(col_x, cur_y)]
                    .set_char(ch)
                    .set_style(dm_style);
                col_x += 1;
            }
        }

        // DM Unread pill
        if dm_unread > 0 && start_x + inner_width > start_x + 4 {
            buf[(start_x + inner_width - 1, cur_y)]
                .set_char('●')
                .set_fg(theme.unread_badge);
        }

        cur_y += 2;

        // 2. Horizontal separator line
        for x in (start_x + 1)..(start_x + inner_width) {
            buf[(x, cur_y)]
                .set_char('─')
                .set_fg(Color::Rgb(40, 40, 45));
        }
        cur_y += 2;

        // 3. Server list
        for (idx, server) in self.state.servers.iter().enumerate() {
            if cur_y >= area.bottom() - 1 {
                break;
            }

            let is_server_active = match &self.state.active_rail {
                ActiveRail::Server(id) => id == &server.id,
                _ => false,
            };

            let is_selected = is_focused && self.state.selected_server_index == idx;

            // Generate 2-letter abbreviation badge: initials for multi-word (e.g. "Stoat Official" -> "SO"),
            // or first 2 characters for single-word / short names (e.g. "Zero" -> "ZE", "Origin" -> "OR")
            let words: Vec<&str> = server.name.split_whitespace().collect();
            let label = if words.len() >= 2 {
                let first = words[0].chars().next().unwrap_or('?');
                let second = words[1].chars().next().unwrap_or('?');
                format!("{}{}", first, second).to_uppercase()
            } else if let Some(first_word) = words.first() {
                let chars: Vec<char> = first_word.chars().collect();
                if chars.len() >= 2 {
                    format!("{}{}", chars[0], chars[1]).to_uppercase()
                } else if let Some(&c) = chars.first() {
                    format!("{} ", c).to_uppercase()
                } else {
                    "??".to_string()
                }
            } else {
                "??".to_string()
            };

            let item_style = if is_server_active {
                Style::default().fg(theme.primary).bg(theme.item_selected_bg).add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::White).bg(Color::Rgb(35, 35, 42))
            } else {
                Style::default().fg(theme.muted).bg(theme.rail_bg)
            };

            // Left indicator bar
            if is_server_active {
                buf[(start_x, cur_y)]
                    .set_char('▌')
                    .set_fg(theme.primary)
                    .set_style(Modifier::BOLD);
            }

            let server_box = if inner_width >= 6 {
                format!(" [{}] ", label)
            } else {
                format!("[{}]", label)
            };

            let mut col_x = start_x + 1;
            for ch in server_box.chars() {
                if col_x < start_x + inner_width {
                    buf[(col_x, cur_y)]
                        .set_char(ch)
                        .set_style(item_style);
                    col_x += 1;
                }
            }

            // Unread indicator for server channels
            let mut srv_unreads = 0;
            if let Some(cids) = self.state.server_channels.get(&server.id) {
                for cid in cids {
                    srv_unreads += self.state.unread_counts.get(cid).copied().unwrap_or(0);
                }
            }
            if srv_unreads > 0 && start_x + inner_width > start_x + 4 {
                buf[(start_x + inner_width - 1, cur_y)]
                    .set_char('●')
                    .set_fg(theme.unread_badge);
            }

            cur_y += 2;
        }
    }
}
