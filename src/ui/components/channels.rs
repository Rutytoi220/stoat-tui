use crate::state::{ActiveRail, AppState, Focus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

pub struct ChannelsSidebar<'a> {
    state: &'a mut AppState,
}

impl<'a> ChannelsSidebar<'a> {
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }

    pub fn compute_items(&self) -> Vec<SidebarItem> {
        let mut items = Vec::new();

        match &self.state.active_rail {
            ActiveRail::DirectMessages => {
                for dm in &self.state.dms {
                    let acc_id = self.state.account_for_channel(&dm.id);
                    let cur_user_id = self.state.current_user_for_account(acc_id).map(|u| u.id.as_str());
                    let instance_tag = self.state.instance_tag_for_account(acc_id).map(str::to_string);

                    if let Some(ref recipients) = dm.recipients {
                        for r in recipients {
                            let _ = self.state.resolve_user_in_account(acc_id, r);
                        }
                    }
                    let name = dm.display_name(&self.state.users, cur_user_id);
                    let recipient_id = if dm.channel_type == "DirectMessage" {
                        dm.recipients.as_ref().and_then(|r| {
                            r.iter()
                                .find(|&id| Some(id.as_str()) != cur_user_id)
                                .or_else(|| r.first())
                                .cloned()
                        })
                    } else {
                        None
                    };
                    items.push(SidebarItem::Channel {
                        id: dm.id.clone(),
                        prefix: "@".to_string(),
                        name,
                        instance_tag,
                        is_voice: false,
                        recipient_id,
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
                                        instance_tag: None,
                                        is_voice,
                                        recipient_id: None,
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
                                    instance_tag: None,
                                    is_voice,
                                    recipient_id: None,
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
        instance_tag: Option<String>,
        is_voice: bool,
        recipient_id: Option<String>,
    },
}

impl<'a> Widget for ChannelsSidebar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let is_focused = self.state.focus == Focus::Channels;
        let theme = self.state.theme.clone();
        let borderless = area.width < 24 || area.height < 10;

        // Render sidebar background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.sidebar_bg);
            }
        }

        let content_width = if !borderless && area.width >= 3 {
            let block = Block::default()
                .borders(Borders::RIGHT)
                .border_style(theme.focused_border_style(is_focused));
            block.render(area, buf);
            area.width.saturating_sub(1)
        } else {
            area.width
        };

        if content_width < 2 || area.height < 1 {
            return;
        }

        let content_rect = Rect {
            x: area.left(),
            y: area.top(),
            width: content_width,
            height: area.height,
        };
        let pad: u16 = if borderless && content_width <= 12 { 0 } else { 1 };
        let start_x = area.left() + pad;
        let max_w = content_width.saturating_sub(pad.saturating_mul(2)) as usize;
        let mut cur_y = area.top();

        // 1. Sidebar Header & divider only when height >= 4
        if area.height >= 4 {
            let title = match &self.state.active_rail {
                ActiveRail::DirectMessages => {
                    if content_width < 16 { "DMs".to_string() } else { "DIRECT MESSAGES".to_string() }
                }
                ActiveRail::Server(sid) => {
                    let base = self.state
                        .servers
                        .iter()
                        .find(|s| &s.id == sid)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| "SERVER".to_string());
                    let acc_id = self.state.account_for_server(sid);
                    if let Some(tag) = self.state.instance_tag_for_account(acc_id) {
                        format!("{} ({})", base, tag)
                    } else {
                        base
                    }
                }
            };

            let title_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
            let truncated_title: String = title.chars().take(max_w).collect();
            crate::ui::write_str(buf, content_rect, start_x, cur_y, &truncated_title, title_style);
            cur_y += 1;

            // Divider
            for dx in start_x..start_x.saturating_add(max_w as u16) {
                crate::ui::put_cell(buf, content_rect, dx, cur_y, '─', Style::default().fg(Color::Rgb(45, 45, 50)));
            }
            cur_y += 1;
        }

        // 2. Channel items
        let items = self.compute_items();
        let mut channel_index = 0;

        for item in items {
            if cur_y >= area.bottom() {
                break;
            }

            match item {
                SidebarItem::Category(cat_title) => {
                    if area.height < 4 {
                        continue;
                    }
                    cur_y += 1; // Spacing before category
                    if cur_y >= area.bottom() {
                        break;
                    }
                    let header_style = Style::default().fg(theme.muted).add_modifier(Modifier::BOLD);
                    let label: String = format!("▾ {}", cat_title.to_uppercase()).chars().take(max_w).collect();
                    crate::ui::write_str(buf, content_rect, start_x, cur_y, &label, header_style);
                    cur_y += 1;
                }
                SidebarItem::Channel { id, prefix, name, instance_tag, is_voice: _, recipient_id } => {
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
                        let bg = if is_active { theme.item_selected_bg } else { Color::Rgb(35, 35, 45) };
                        for bg_x in content_rect.left()..content_rect.right() {
                            buf[(bg_x, cur_y)].set_bg(bg);
                        }
                    }

                    // Render 2x1 PFP in Direct Messages mode only when width permits (>= 14)
                    let mut cx = if pad == 0 { start_x } else { start_x + 1 };
                    let mut avail_name_w = if unread > 0 && content_width >= 12 {
                        max_w.saturating_sub(4)
                    } else {
                        max_w.saturating_sub(if pad == 0 { 0 } else { 1 })
                    };
                    if self.state.active_server_id.is_none() && content_width >= 14 {
                        if let Some(uid) = recipient_id {
                            let pfp_rect = Rect {
                                x: start_x + 1,
                                y: cur_y,
                                width: 2,
                                height: 1,
                            };

                            let display_for_init = self.state.users.get(&uid)
                                .map(|u| u.name().to_string())
                                .unwrap_or_else(|| name.clone());
                            let initials: String = display_for_init.chars().take(2).collect::<String>().to_uppercase();
                            let (badge_bg, badge_fg) = theme.user_initials_color(&uid);
                            let mut init_chars = initials.chars();
                            let c1 = init_chars.next().unwrap_or('?');
                            let c2 = init_chars.next().unwrap_or(' ');
                            crate::ui::put_cell(buf, content_rect, start_x + 1, cur_y, c1, Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));
                            crate::ui::put_cell(buf, content_rect, start_x + 2, cur_y, c2, Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));

                            let acc_id = self.state.account_for_user(&uid).to_string();
                            let key = self.state.avatar_cache_key(&acc_id, &uid, "avatar");
                            self.state.deferred_image_renders.push((pfp_rect, key.clone()));
                            self.state.request_user_avatar_with_key_in_account(&acc_id, &uid, &key);

                            cx = start_x + 4;
                            avail_name_w = if unread > 0 { max_w.saturating_sub(8) } else { max_w.saturating_sub(4) };
                        }
                    }

                    // Render channel icon and name (+ optional muted instance tag on secondary accounts)
                    let line_str = if content_width <= 12 {
                        format!("{}{}", prefix, name)
                    } else {
                        format!("{} {}", prefix, name)
                    };
                    let clipped: String = line_str.chars().take(avail_name_w.max(1)).collect();
                    let next_cx = crate::ui::write_str(buf, content_rect, cx, cur_y, &clipped, item_style);
                    if let Some(ref tag) = instance_tag {
                        let used = clipped.chars().count();
                        let rem = avail_name_w.saturating_sub(used);
                        if rem >= 4 {
                            let tag_str: String = format!(" ({})", tag).chars().take(rem).collect();
                            let row_bg = if is_active {
                                theme.item_selected_bg
                            } else if is_selected {
                                Color::Rgb(35, 35, 45)
                            } else {
                                theme.sidebar_bg
                            };
                            crate::ui::write_str(
                                buf,
                                content_rect,
                                next_cx,
                                cur_y,
                                &tag_str,
                                Style::default().fg(theme.muted).bg(row_bg),
                            );
                        }
                    }

                    // Render unread badge if any and wide enough
                    if unread > 0 && content_width >= 12 {
                        let badge = format!(" {} ", unread);
                        let badge_x = content_rect.right().saturating_sub(badge.chars().count() as u16 + 1);
                        crate::ui::write_str(
                            buf,
                            content_rect,
                            badge_x,
                            cur_y,
                            &badge,
                            Style::default().fg(Color::White).bg(theme.unread_badge).add_modifier(Modifier::BOLD),
                        );
                    }

                    channel_index += 1;
                    cur_y += 1;
                }
            }
        }
    }
}
