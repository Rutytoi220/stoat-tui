use crate::api::models::Presence;
use crate::state::{AppState, Focus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Widget};

pub struct MemberRail<'a> {
    state: &'a mut AppState,
}

impl<'a> MemberRail<'a> {
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for MemberRail<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_member_rail || area.width == 0 || area.height == 0 {
            return;
        }

        let is_focused = self.state.focus == Focus::Members;
        let theme = self.state.theme.clone();
        let borderless = area.width < 24 || area.height < 10;

        // Render member rail background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.sidebar_bg);
            }
        }

        let content_rect = if !borderless && area.width >= 4 {
            let block = Block::default()
                .borders(Borders::LEFT)
                .border_style(theme.focused_border_style(is_focused));
            block.render(area, buf);
            Rect {
                x: area.left() + 1,
                y: area.top(),
                width: area.width.saturating_sub(1),
                height: area.height,
            }
        } else {
            area
        };

        if content_rect.width < 2 || content_rect.height < 1 {
            return;
        }

        let start_x = content_rect.left() + if content_rect.width <= 12 { 0 } else { 1 };
        let max_w = content_rect.right().saturating_sub(start_x) as usize;
        let mut cur_y = if borderless { content_rect.top() } else { content_rect.top() + 1 };

        if cur_y >= content_rect.bottom() {
            return;
        }

        // Check if we are in Direct Messages or Server
        if self.state.active_server_id.is_none() {
            // Direct Messages mode: inspect active DM channel
            let dm_channel = self.state.active_channel_id.as_ref().and_then(|cid| self.state.channels.get(cid)).cloned();

            if let Some(ch) = dm_channel {
                if ch.channel_type == "Group" {
                    if content_rect.height >= 4 {
                        crate::ui::write_str(
                            buf,
                            content_rect,
                            start_x,
                            cur_y,
                            "PARTICIPANTS",
                            ratatui::style::Style::default().fg(theme.muted).add_modifier(Modifier::BOLD),
                        );
                        cur_y += 2;
                    }

                    let recipients = ch.recipients.clone().unwrap_or_default();
                    for uid in recipients {
                        if cur_y >= content_rect.bottom() {
                            break;
                        }
                        let user = self.state.users.get(&uid).cloned();
                        let name = user.as_ref().map(|u| u.name()).unwrap_or(uid.as_str());
                        let presence = user
                            .as_ref()
                            .and_then(|u| u.status.as_ref())
                            .and_then(|s| s.presence.as_ref())
                            .unwrap_or(&Presence::Offline);

                        let (dot, dot_color) = match presence {
                            Presence::Online => ('●', theme.status_online),
                            Presence::Idle => ('●', theme.status_idle),
                            Presence::Busy => ('●', theme.status_dnd),
                            _ => ('○', theme.status_offline),
                        };

                        crate::ui::put_cell(buf, content_rect, start_x, cur_y, dot, ratatui::style::Style::default().fg(dot_color));

                        if content_rect.width >= 14 {
                            let pfp_rect = Rect {
                                x: start_x + 2,
                                y: cur_y,
                                width: 2,
                                height: 1,
                            };

                            let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
                            let (badge_bg, badge_fg) = theme.user_initials_color(&uid);
                            let mut init_chars = initials.chars();
                            let c1 = init_chars.next().unwrap_or('?');
                            let c2 = init_chars.next().unwrap_or(' ');
                            crate::ui::put_cell(buf, content_rect, start_x + 2, cur_y, c1, ratatui::style::Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));
                            crate::ui::put_cell(buf, content_rect, start_x + 3, cur_y, c2, ratatui::style::Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));

                            let acc_id = self.state.account_for_user(&uid).to_string();
                            let key = self.state.avatar_cache_key(&acc_id, &uid, "avatar");
                            self.state.deferred_image_renders.push((pfp_rect, key.clone()));
                            self.state.request_user_avatar_with_key_in_account(&acc_id, &uid, &key);

                            crate::ui::write_str(buf, content_rect, start_x + 5, cur_y, name, ratatui::style::Style::default().fg(theme.text));
                        } else {
                            crate::ui::write_str(buf, content_rect, start_x + 2, cur_y, name, ratatui::style::Style::default().fg(theme.text));
                        }

                        cur_y += 1;
                    }
                    return;
                } else if ch.channel_type == "DirectMessage" {
                    // 1-on-1 DM: Render User Profile Card
                    let cur_id = self.state.current_user.as_ref().map(|u| u.id.as_str());
                    let other_id = ch.recipients.as_ref().and_then(|r| {
                        r.iter().find(|&id| Some(id.as_str()) != cur_id).or_else(|| r.first()).cloned()
                    });

                    if let Some(uid) = other_id {
                        let user = self.state.users.get(&uid).cloned();
                        let profile = self.state.user_profiles.get(&uid).cloned();

                        let display_name = user.as_ref().map(|u| u.name()).unwrap_or(uid.as_str());
                        let username = user.as_ref().map(|u| u.username.as_str()).unwrap_or("unknown");

                        let presence = user
                            .as_ref()
                            .and_then(|u| u.status.as_ref())
                            .and_then(|s| s.presence.as_ref())
                            .unwrap_or(&Presence::Offline);

                        let status_text = user
                            .as_ref()
                            .and_then(|u| u.status.as_ref())
                            .and_then(|s| s.text.as_deref())
                            .unwrap_or("");

                        let (dot, dot_color, presence_label) = match presence {
                            Presence::Online => ('●', theme.status_online, "Online"),
                            Presence::Idle => ('●', theme.status_idle, "Idle"),
                            Presence::Busy => ('●', theme.status_dnd, "Do Not Disturb"),
                            _ => ('○', theme.status_offline, "Offline"),
                        };

                        if content_rect.height >= 6 && content_rect.width >= 14 {
                            crate::ui::write_str(
                                buf,
                                content_rect,
                                start_x,
                                cur_y,
                                "USER PROFILE",
                                ratatui::style::Style::default().fg(theme.muted).add_modifier(Modifier::BOLD),
                            );
                            cur_y += 2;
                        }

                        // Large PFP Area (Width: up to 10, Height: 5) when space permits
                        let pfp_w = 10.min(content_rect.width.saturating_sub(4));
                        let pfp_h = 5;
                        if content_rect.width >= 14 && cur_y + pfp_h < content_rect.bottom() && pfp_w >= 4 {
                            let pfp_rect = Rect {
                                x: start_x + (max_w.saturating_sub(pfp_w as usize) / 2) as u16,
                                y: cur_y,
                                width: pfp_w,
                                height: pfp_h,
                            };

                            let (badge_bg, badge_fg) = theme.user_initials_color(&uid);
                            for py in pfp_rect.top()..pfp_rect.bottom() {
                                for px in pfp_rect.left()..pfp_rect.right() {
                                    crate::ui::put_cell(buf, content_rect, px, py, ' ', ratatui::style::Style::default().bg(badge_bg));
                                }
                            }
                            let initials: String = display_name.chars().take(2).collect::<String>().to_uppercase();
                            let mut init_chars = initials.chars();
                            let c1 = init_chars.next().unwrap_or('?');
                            let c2 = init_chars.next().unwrap_or(' ');
                            let mid_x = pfp_rect.left() + (pfp_rect.width / 2).saturating_sub(1);
                            let mid_y = pfp_rect.top() + (pfp_rect.height / 2);
                            crate::ui::put_cell(buf, content_rect, mid_x, mid_y, c1, ratatui::style::Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));
                            crate::ui::put_cell(buf, content_rect, mid_x + 1, mid_y, c2, ratatui::style::Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));

                            let acc_id = self.state.account_for_user(&uid).to_string();
                            let key = self.state.avatar_cache_key(&acc_id, &uid, "avatar_large");
                            self.state.deferred_image_renders.push((pfp_rect, key.clone()));
                            self.state.request_user_avatar_with_key_in_account(&acc_id, &uid, &key);

                            cur_y += pfp_h + 1;
                        }

                        if cur_y < content_rect.bottom() {
                            crate::ui::write_str(buf, content_rect, start_x, cur_y, display_name, ratatui::style::Style::default().fg(ratatui::style::Color::White).add_modifier(Modifier::BOLD));
                            cur_y += 1;
                        }

                        if cur_y < content_rect.bottom() {
                            crate::ui::write_str(buf, content_rect, start_x, cur_y, &format!("@{}", username), ratatui::style::Style::default().fg(theme.muted));
                            cur_y += if borderless { 1 } else { 2 };
                        }

                        if cur_y < content_rect.bottom() {
                            crate::ui::put_cell(buf, content_rect, start_x, cur_y, dot, ratatui::style::Style::default().fg(dot_color));
                            let status_line = if !status_text.is_empty() {
                                format!(" {} — \"{}\"", presence_label, status_text)
                            } else {
                                format!(" {}", presence_label)
                            };
                            crate::ui::write_str(buf, content_rect, start_x + 1, cur_y, &status_line, ratatui::style::Style::default().fg(theme.text));
                            cur_y += if borderless { 1 } else { 2 };
                        }

                        if let Some(prof) = profile {
                            if let Some(ref bio) = prof.content {
                                if !bio.trim().is_empty() && cur_y + 1 < content_rect.bottom() {
                                    crate::ui::write_str(buf, content_rect, start_x, cur_y, "ABOUT ME", ratatui::style::Style::default().fg(theme.muted).add_modifier(Modifier::BOLD));
                                    cur_y += 1;
                                    for line in bio.lines() {
                                        if cur_y >= content_rect.bottom() {
                                            break;
                                        }
                                        crate::ui::write_str(buf, content_rect, start_x, cur_y, line, ratatui::style::Style::default().fg(theme.text));
                                        cur_y += 1;
                                    }
                                }
                            }
                        }
                    }
                    return;
                }
            }

            let empty_notice = if content_rect.width < 18 { "No DM" } else { "Select a DM to view profile" };
            crate::ui::write_str(buf, content_rect, start_x, cur_y, empty_notice, ratatui::style::Style::default().fg(theme.muted));
            return;
        }

        // Server Mode: Gather members for active server
        let server_id = self.state.active_server_id.as_deref().unwrap_or("");
        let members = self.state.members.get(server_id).cloned().unwrap_or_default();

        if members.is_empty() {
            let loading_text = if content_rect.width < 14 { "Loading" } else { "Loading members..." };
            crate::ui::write_str(buf, content_rect, start_x, cur_y, loading_text, ratatui::style::Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC));
            return;
        }

        // Split into Online and Offline
        let mut online_members = Vec::new();
        let mut offline_members = Vec::new();

        for m in members {
            let user = self.state.users.get(&m.id.user).cloned();
            let presence = user
                .as_ref()
                .and_then(|u| u.status.as_ref())
                .and_then(|s| s.presence.as_ref())
                .cloned()
                .unwrap_or(Presence::Offline);

            if presence != Presence::Offline {
                online_members.push((m, user, presence));
            } else {
                offline_members.push((m, user, presence));
            }
        }

        let mut item_index = 0;

        // 1. Online Section
        if !online_members.is_empty() && cur_y < content_rect.bottom() {
            if !borderless {
                let header = format!("ONLINE — {}", online_members.len());
                crate::ui::write_str(buf, content_rect, start_x, cur_y, &header, ratatui::style::Style::default().fg(theme.muted).add_modifier(Modifier::BOLD));
                cur_y += 2;
            }

            for (m, user, presence) in online_members {
                if cur_y >= content_rect.bottom() {
                    break;
                }

                let is_selected = is_focused && self.state.selected_member_index == item_index;
                let name = m.nickname.clone().unwrap_or_else(|| {
                    user.as_ref().map(|u| u.name().to_string()).unwrap_or_else(|| "Member".to_string())
                });

                let (dot, dot_color) = match presence {
                    Presence::Online => ('●', theme.status_online),
                    Presence::Idle => ('●', theme.status_idle),
                    Presence::Busy => ('●', theme.status_dnd),
                    _ => ('○', theme.status_offline),
                };

                if is_selected {
                    for bx in content_rect.left()..content_rect.right() {
                        buf[(bx, cur_y)].set_bg(theme.item_selected_bg);
                    }
                }

                crate::ui::put_cell(buf, content_rect, start_x, cur_y, dot, ratatui::style::Style::default().fg(dot_color));

                let name_color = theme.role_color(&m.id.user);
                if content_rect.width >= 14 {
                    let pfp_rect = Rect {
                        x: start_x + 2,
                        y: cur_y,
                        width: 2,
                        height: 1,
                    };

                    let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
                    let (badge_bg, badge_fg) = theme.user_initials_color(&m.id.user);
                    let mut init_chars = initials.chars();
                    let c1 = init_chars.next().unwrap_or('?');
                    let c2 = init_chars.next().unwrap_or(' ');
                    crate::ui::put_cell(buf, content_rect, start_x + 2, cur_y, c1, ratatui::style::Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));
                    crate::ui::put_cell(buf, content_rect, start_x + 3, cur_y, c2, ratatui::style::Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));

                    let acc_id = self.state.account_for_user(&m.id.user).to_string();
                    let key = self.state.avatar_cache_key(&acc_id, &m.id.user, "avatar");
                    self.state.deferred_image_renders.push((pfp_rect, key.clone()));
                    self.state.request_user_avatar_with_key_in_account(&acc_id, &m.id.user, &key);

                    crate::ui::write_str(buf, content_rect, start_x + 5, cur_y, &name, ratatui::style::Style::default().fg(name_color));
                } else {
                    crate::ui::write_str(buf, content_rect, start_x + 2, cur_y, &name, ratatui::style::Style::default().fg(name_color));
                }

                item_index += 1;
                cur_y += 1;
            }
        }

        // 2. Offline Section
        if !offline_members.is_empty() && cur_y < content_rect.bottom() {
            if !borderless && cur_y + 2 < content_rect.bottom() {
                cur_y += 1;
                let header = format!("OFFLINE — {}", offline_members.len());
                crate::ui::write_str(buf, content_rect, start_x, cur_y, &header, ratatui::style::Style::default().fg(theme.muted).add_modifier(Modifier::BOLD));
                cur_y += 2;
            }

            for (m, user, _) in offline_members {
                if cur_y >= content_rect.bottom() {
                    break;
                }

                let is_selected = is_focused && self.state.selected_member_index == item_index;
                let name = m.nickname.clone().unwrap_or_else(|| {
                    user.as_ref().map(|u| u.name().to_string()).unwrap_or_else(|| "Member".to_string())
                });

                if is_selected {
                    for bx in content_rect.left()..content_rect.right() {
                        buf[(bx, cur_y)].set_bg(theme.item_selected_bg);
                    }
                }

                crate::ui::put_cell(buf, content_rect, start_x, cur_y, '○', ratatui::style::Style::default().fg(theme.status_offline));

                if content_rect.width >= 14 {
                    let pfp_rect = Rect {
                        x: start_x + 2,
                        y: cur_y,
                        width: 2,
                        height: 1,
                    };

                    let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
                    let (badge_bg, badge_fg) = theme.user_initials_color(&m.id.user);
                    let mut init_chars = initials.chars();
                    let c1 = init_chars.next().unwrap_or('?');
                    let c2 = init_chars.next().unwrap_or(' ');
                    crate::ui::put_cell(buf, content_rect, start_x + 2, cur_y, c1, ratatui::style::Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));
                    crate::ui::put_cell(buf, content_rect, start_x + 3, cur_y, c2, ratatui::style::Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));

                    let acc_id = self.state.account_for_user(&m.id.user).to_string();
                    let key = self.state.avatar_cache_key(&acc_id, &m.id.user, "avatar");
                    self.state.deferred_image_renders.push((pfp_rect, key.clone()));
                    self.state.request_user_avatar_with_key_in_account(&acc_id, &m.id.user, &key);

                    crate::ui::write_str(buf, content_rect, start_x + 5, cur_y, &name, ratatui::style::Style::default().fg(theme.muted));
                } else {
                    crate::ui::write_str(buf, content_rect, start_x + 2, cur_y, &name, ratatui::style::Style::default().fg(theme.muted));
                }

                item_index += 1;
                cur_y += 1;
            }
        }
    }
}
