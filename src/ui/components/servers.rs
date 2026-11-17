use crate::state::{ActiveRail, AppState, Focus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

pub struct ServersRail<'a> {
    state: &'a mut AppState,
}

impl<'a> ServersRail<'a> {
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for ServersRail<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let is_focused = self.state.focus == Focus::Servers;
        let theme = self.state.theme.clone();
        let borderless = area.width < 24 || area.height < 10;

        // Render rail background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.rail_bg);
            }
        }

        // Draw border only when space permits (non-compact rail)
        let inner_width = if borderless && area.width > 7 {
            area.width
        } else if area.width >= 3 {
            let block = Block::default()
                .borders(Borders::RIGHT)
                .border_style(theme.focused_border_style(is_focused));
            block.render(area, buf);
            area.width.saturating_sub(1)
        } else {
            area.width
        };

        if area.width < 2 || area.height < 1 {
            return;
        }

        let start_x = area.left();
        let mut cur_y = if borderless { area.top() } else { area.top() + 1 };
        let y_step: u16 = if area.height < 10 { 1 } else { 2 };

        if cur_y >= area.bottom() {
            return;
        }

        // 1. Direct Messages Icon button (@)
        let is_dm_active = matches!(self.state.active_rail, ActiveRail::DirectMessages);
        let dm_unread: usize = self.state.dms.iter().map(|d| self.state.unread_counts.get(&d.id).copied().unwrap_or(0)).sum();

        let dm_style = if is_dm_active {
            Style::default().fg(Color::White).bg(theme.item_selected_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.muted).bg(theme.rail_bg)
        };

        if is_dm_active {
            crate::ui::put_cell(buf, area, start_x, cur_y, '▌', Style::default().fg(theme.primary).add_modifier(Modifier::BOLD));
        }

        let dm_text = if borderless && inner_width >= 10 {
            " @ DMs"
        } else if inner_width >= 6 {
            "  @   "
        } else {
            "@"
        };
        let text_start_x = if inner_width >= 3 { start_x + 1 } else { start_x };
        crate::ui::write_str(
            buf,
            Rect { x: area.left(), y: area.top(), width: inner_width, height: area.height },
            text_start_x,
            cur_y,
            dm_text,
            dm_style,
        );

        // DM Unread pill
        if dm_unread > 0 && inner_width > 4 {
            crate::ui::put_cell(buf, area, start_x + inner_width - 1, cur_y, '●', Style::default().fg(theme.unread_badge));
        }

        cur_y += y_step;

        // 2. Horizontal separator line (when tall enough)
        if !borderless && cur_y < area.bottom() {
            for x in (start_x + 1)..(start_x + inner_width) {
                crate::ui::put_cell(buf, area, x, cur_y, '─', Style::default().fg(Color::Rgb(40, 40, 45)));
            }
            cur_y += 2;
        }

        // 3. Server list
        let servers = self.state.servers.clone();
        let bottom_limit = if borderless { area.bottom() } else { area.bottom().saturating_sub(1) };
        for (idx, server) in servers.iter().enumerate() {
            if cur_y >= bottom_limit {
                break;
            }

            let is_server_active = match &self.state.active_rail {
                ActiveRail::Server(id) => id == &server.id,
                _ => false,
            };

            let is_selected = is_focused && self.state.selected_server_index == idx + 1;

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

            if is_server_active {
                crate::ui::put_cell(buf, area, start_x, cur_y, '▌', Style::default().fg(theme.primary).add_modifier(Modifier::BOLD));
            }

            let server_box = if borderless && inner_width >= 10 {
                format!(" [{}] {}", label, server.name)
            } else if inner_width >= 6 {
                format!(" [{}] ", label)
            } else {
                format!("[{}]", label)
            };

            let col_start = if inner_width >= 3 { start_x + 1 } else { start_x };
            crate::ui::write_str(
                buf,
                Rect { x: area.left(), y: area.top(), width: inner_width, height: area.height },
                col_start,
                cur_y,
                &server_box,
                item_style,
            );

            // Enqueue server icon image over the 2-char badge area if icon is set and width allows
            if inner_width >= 5 {
                if let Some(icon_url) = self.state.server_icon_url(server) {
                    let icon_x = if borderless && inner_width >= 10 {
                        start_x + 3
                    } else if inner_width >= 6 {
                        start_x + 3
                    } else {
                        start_x + 2
                    };
                    let icon_w = 2.min((start_x + inner_width).saturating_sub(icon_x));
                    if icon_w > 0 {
                        let icon_rect = Rect {
                            x: icon_x,
                            y: cur_y,
                            width: icon_w,
                            height: 1,
                        };
                        let key = format!("server_icon_{}", server.id);
                        self.state.deferred_image_renders.push((icon_rect, key.clone()));
                        self.state.request_image(key, icon_url);
                    }
                }
            }

            // Unread indicator for server channels
            let mut srv_unreads = 0;
            if let Some(cids) = self.state.server_channels.get(&server.id) {
                for cid in cids {
                    srv_unreads += self.state.unread_counts.get(cid).copied().unwrap_or(0);
                }
            }
            if srv_unreads > 0 && inner_width > 4 {
                crate::ui::put_cell(buf, area, start_x + inner_width - 1, cur_y, '●', Style::default().fg(theme.unread_badge));
            }

            cur_y += y_step;
        }
    }
}
