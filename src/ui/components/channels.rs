use crate::state::{ActiveRail, AppState, Focus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

pub struct ChannelsSidebar<'a> {
    state: &'a AppState,
}

impl<'a> ChannelsSidebar<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    pub fn compute_items(&self) -> Vec<SidebarItem> {
        let mut items = Vec::new();

        match &self.state.active_rail {
            ActiveRail::DirectMessages => {
                for dm in &self.state.dms {
                    if let Some(ref recipients) = dm.recipients {
                        for r in recipients {
                            let _ = self.state.resolve_user(r);
                        }
                    }
                    let name = dm.display_name(&self.state.users, self.state.current_user.as_ref().map(|u| u.id.as_str()));
                    items.push(SidebarItem::Channel {
                        id: dm.id.clone(),
                        prefix: "@".to_string(),
                        name,
                        is_voice: false,
                    });
                }
            }
            ActiveRail::Server(server_id) => {
                if let Some(server) = self.state.servers.iter().find(|s| &s.id == server_id) {
                    if let Some(categories) = &server.categories {
                        for cat in categories {
                            items.push(SidebarItem::Category(cat.title.clone()));
                            for cid in &cat.channels {
                                if let Some(ch) = self.state.channels.get(cid) {
                                    let is_voice = ch.channel_type == "VoiceChannel";
                                    let prefix = if is_voice { "🔊" } else { "#" };
                                    let name = ch.name.clone().unwrap_or_else(|| "channel".to_string());
                                    items.push(SidebarItem::Channel {
                                        id: ch.id.clone(),
                                        prefix: prefix.to_string(),
                                        name,
                                        is_voice,
                                    });
                                }
                            }
                        }
                    } else if let Some(cids) = self.state.server_channels.get(server_id) {
                        for cid in cids {
                            if let Some(ch) = self.state.channels.get(cid) {
                                let is_voice = ch.channel_type == "VoiceChannel";
                                let prefix = if is_voice { "🔊" } else { "#" };
                                let name = ch.name.clone().unwrap_or_else(|| "channel".to_string());
                                items.push(SidebarItem::Channel {
                                    id: ch.id.clone(),
                                    prefix: prefix.to_string(),
                                    name,
                                    is_voice,
                                });
                            }
                        }
                    }
                }
            }
        }

        items
    }
}

#[allow(dead_code)]
pub enum SidebarItem {
    Category(String),
    Channel {
        id: String,
        prefix: String,
        name: String,
        is_voice: bool,
    },
}

impl<'a> Widget for ChannelsSidebar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let is_focused = self.state.focus == Focus::Channels;
        let theme = &self.state.theme;

        // Render sidebar background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.sidebar_bg);
            }
        }

        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(theme.focused_border_style(is_focused));
        block.render(area, buf);

        if area.width < 5 || area.height < 4 {
            return;
        }

        let start_x = area.left() + 1;
        let max_w = area.width.saturating_sub(2) as usize;
        let mut cur_y = area.top();

        // 1. Sidebar Header (Server Name or "Direct Messages")
        let title = match &self.state.active_rail {
            ActiveRail::DirectMessages => "DIRECT MESSAGES".to_string(),
            ActiveRail::Server(sid) => {
                self.state
                    .servers
                    .iter()
                    .find(|s| &s.id == sid)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "SERVER".to_string())
            }
        };

        let title_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
        let truncated_title: String = title.chars().take(max_w).collect();
        let mut x = start_x;
        for ch in truncated_title.chars() {
            buf[(x, cur_y)]
                .set_char(ch)
                .set_style(title_style);
            x += 1;
        }

        cur_y += 1;

        // Divider
        for dx in start_x..(start_x + area.width.saturating_sub(2)) {
            buf[(dx, cur_y)]
                .set_char('─')
                .set_fg(Color::Rgb(45, 45, 50));
        }

        cur_y += 1;

        // 2. Channel items
        let items = self.compute_items();
        let mut channel_index = 0;

        for item in items {
            if cur_y >= area.bottom() {
                break;
            }

            match item {
                SidebarItem::Category(cat_title) => {
                    cur_y += 1; // Spacing before category
                    if cur_y >= area.bottom() {
                        break;
                    }
                    let header_style = Style::default().fg(theme.muted).add_modifier(Modifier::BOLD);
                    let label = format!("▾ {}", cat_title.to_uppercase());
                    let mut cx = start_x;
                    for ch in label.chars().take(max_w) {
                        buf[(cx, cur_y)]
                            .set_char(ch)
                            .set_style(header_style);
                        cx += 1;
                    }
                    cur_y += 1;
                }
                SidebarItem::Channel { id, prefix, name, is_voice: _ } => {
                    let is_active = self.state.active_channel_id.as_deref() == Some(&id);
                    let is_selected = is_focused && self.state.selected_channel_index == channel_index;
                    let unread = self.state.unread_counts.get(&id).copied().unwrap_or(0);

                    let item_style = if is_active {
                        Style::default().fg(Color::White).bg(theme.item_selected_bg).add_modifier(Modifier::BOLD)
                    } else if is_selected {
                        Style::default().fg(Color::White).bg(Color::Rgb(35, 35, 45))
                    } else if unread > 0 {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.muted)
                    };

                    // Draw background for active / selected row
                    if is_active || is_selected {
                        for bg_x in start_x..(start_x + area.width.saturating_sub(2)) {
                            buf[(bg_x, cur_y)].set_bg(if is_active {
                                theme.item_selected_bg
                            } else {
                                Color::Rgb(35, 35, 45)
                            });
                        }
                    }

                    // Render channel icon and name
                    let line_str = format!("{} {}", prefix, name);
                    let mut cx = start_x + 1;
                    let avail_name_w = if unread > 0 { max_w.saturating_sub(4) } else { max_w.saturating_sub(1) };
                    for ch in line_str.chars().take(avail_name_w) {
                        buf[(cx, cur_y)]
                            .set_char(ch)
                            .set_style(item_style);
                        cx += 1;
                    }

                    // Render unread badge if any
                    if unread > 0 {
                        let badge = format!(" {} ", unread);
                        let badge_x = start_x + area.width.saturating_sub(2).saturating_sub(badge.len() as u16 + 1);
                        let mut bx = badge_x;
                        for bch in badge.chars() {
                            buf[(bx, cur_y)]
                                .set_char(bch)
                                .set_fg(Color::White)
                                .set_bg(theme.unread_badge)
                                .set_style(Modifier::BOLD);
                            bx += 1;
                        }
                    }

                    channel_index += 1;
                    cur_y += 1;
                }
            }
        }
    }
}
