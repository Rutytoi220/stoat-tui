use crate::state::{AppState, GatewayStatus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

pub struct ContentHeader<'a> {
    state: &'a mut AppState,
}

impl<'a> ContentHeader<'a> {
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for ContentHeader<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let theme = self.state.theme.clone();

        // Render header background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.bg);
            }
        }

        if area.height >= 2 {
            let block = Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::Rgb(45, 45, 50)));
            block.render(area, buf);
        }

        if area.width < 2 {
            return;
        }

        let cur_y = area.top();
        let is_compact = area.width < 32 || area.height == 1;
        let mut cur_x = if area.width < 12 { area.left() } else { area.left() + 1 };

        // Channel name & prefix
        let channel = self.state.active_channel().cloned();
        let (prefix, name, topic, dm_user, instance_tag) = if let Some(ch) = channel {
            let acc_id = self.state.account_for_channel(&ch.id).to_string();
            let cur_uid = self.state.current_user_for_account(&acc_id).map(|u| u.id.as_str());
            let inst_tag = self.state.instance_tag_for_account(&acc_id).map(str::to_string);
            let pfx = if ch.channel_type == "VoiceChannel" {
                if is_compact { "🔊" } else { "🔊 " }
            } else if ch.channel_type == "DirectMessage" {
                if is_compact { "@" } else { "@ " }
            } else if is_compact {
                "#"
            } else {
                "# "
            };
            let n = ch.display_name(&self.state.users, cur_uid);
            let t = ch.description.clone().unwrap_or_default();
            let other_user = if ch.channel_type == "DirectMessage" {
                ch.recipients.as_ref().and_then(|r| {
                    let uid = r.iter().find(|&id| Some(id.as_str()) != cur_uid).or_else(|| r.first());
                    uid.and_then(|id| self.state.users.get(id)).cloned()
                })
            } else {
                None
            };
            (pfx, n, t, other_user, inst_tag)
        } else if is_compact {
            ("", "Stoat".to_string(), String::new(), None, None)
        } else {
            ("", "No Channel Selected".to_string(), "".to_string(), None, None)
        };

        // Compact tiny mode (e.g. 10x10): render crisp title & pane indicator cleanly without clipping
        if area.width < 40 {
            let title_text = format!("{}{}", prefix, name);
            cur_x = crate::ui::write_str(
                buf,
                area,
                cur_x,
                cur_y,
                &title_text,
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            );
            if area.width >= 16 && cur_x + 5 < area.right() {
                let pane_tag = match self.state.focus {
                    crate::state::Focus::Servers => "[1:Srv]",
                    crate::state::Focus::Channels => "[2:Ch]",
                    crate::state::Focus::Chat => "[3:Msg]",
                    crate::state::Focus::Input => "[4:In]",
                    crate::state::Focus::Members => "[5:Mem]",
                };
                let tag_x = area.right().saturating_sub(pane_tag.chars().count() as u16);
                if tag_x > cur_x {
                    crate::ui::write_str(buf, area, tag_x, cur_y, pane_tag, Style::default().fg(theme.primary));
                }
            }
            return;
        }

        // If in DM and user avatar is available, render 2x1 avatar badge + deferred image
        if let Some(ref u) = dm_user {
            if cur_x + 2 < area.right() {
                let pfp_rect = Rect {
                    x: cur_x,
                    y: cur_y,
                    width: 2,
                    height: 1,
                };

                let initials: String = u.name().chars().take(2).collect::<String>().to_uppercase();
                let (badge_bg, badge_fg) = theme.user_initials_color(&u.id);
                let mut init_chars = initials.chars();
                let c1 = init_chars.next().unwrap_or('?');
                let c2 = init_chars.next().unwrap_or(' ');
                crate::ui::put_cell(buf, area, cur_x, cur_y, c1, Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));
                crate::ui::put_cell(buf, area, cur_x + 1, cur_y, c2, Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD));

                let acc_id = self.state.account_for_user(&u.id).to_string();
                let key = self.state.avatar_cache_key(&acc_id, &u.id, "avatar");
                self.state.deferred_image_renders.push((pfp_rect, key.clone()));
                self.state.request_user_avatar_with_key_in_account(&acc_id, &u.id, &key);

                cur_x += 3;
            }
        }

        // Compute right-side controls first so the channel title gets all remaining space
        let member_btn = if self.state.show_member_rail {
            "[ 👥 Members ▾ ]"
        } else {
            "[ 👥 Members ▸ ]"
        };
        let member_btn_w = member_btn.chars().count() as u16;
        let member_rx = area.right().saturating_sub(1 + member_btn_w);

        let (dot, dot_color, status_text) = match &self.state.gateway_status {
            GatewayStatus::Connected { latency_ms } => ('●', theme.status_online, format!("{}ms", latency_ms)),
            GatewayStatus::Connecting => ('◐', theme.status_idle, "Connecting...".to_string()),
            GatewayStatus::Disconnected(_) => ('○', theme.status_offline, "Offline".to_string()),
        };
        let status_w = (status_text.chars().count() as u16) + 2; // dot + space + text
        let status_rx = member_rx.saturating_sub(2 + status_w);
        let title_right_limit = status_rx.saturating_sub(2);

        let name_limit = Rect {
            x: area.left(),
            y: area.top(),
            width: title_right_limit.saturating_sub(area.left()),
            height: area.height,
        };

        // Render prefix
        cur_x = crate::ui::write_str(
            buf,
            name_limit,
            cur_x,
            cur_y,
            prefix,
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
        );
        cur_x = crate::ui::write_str(
            buf,
            name_limit,
            cur_x,
            cur_y,
            &name,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        );
        if let Some(ref tag) = instance_tag {
            cur_x = crate::ui::write_str(
                buf,
                name_limit,
                cur_x,
                cur_y,
                &format!(" ({})", tag),
                Style::default().fg(theme.muted),
            );
        }

        // Separator and topic if room remains
        if !topic.is_empty() && cur_x + 8 < title_right_limit {
            cur_x = crate::ui::write_str(buf, name_limit, cur_x, cur_y, "  │  ", Style::default().fg(Color::Rgb(60, 62, 70)));
            let _ = crate::ui::write_str(buf, name_limit, cur_x, cur_y, &topic, Style::default().fg(theme.muted));
        }

        // Render right-aligned connection status pill + member toggle pill
        if status_rx > area.left() + 4 {
            crate::ui::put_cell(buf, area, status_rx, cur_y, dot, Style::default().fg(dot_color));
            crate::ui::write_str(buf, area, status_rx + 2, cur_y, &status_text, Style::default().fg(theme.muted));
        }

        let btn_style = if self.state.show_member_rail {
            Style::default().fg(Color::White).bg(Color::Rgb(40, 42, 52))
        } else {
            Style::default().fg(theme.muted).bg(Color::Rgb(28, 30, 36))
        };
        crate::ui::write_str(buf, area, member_rx, cur_y, member_btn, btn_style);
    }
}
