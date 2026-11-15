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
        let theme = &self.state.theme;

        // Render header background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.bg);
            }
        }

        let block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::Rgb(45, 45, 50)));
        block.render(area, buf);

        if area.height < 1 || area.width < 10 {
            return;
        }

        let cur_y = area.top();
        let mut cur_x = area.left() + 1;

        // Channel name & prefix
        let channel = self.state.active_channel();
        let (prefix, name, topic, dm_user) = if let Some(ch) = channel {
            let pfx = if ch.channel_type == "VoiceChannel" {
                "🔊 "
            } else if ch.channel_type == "DirectMessage" {
                "@ "
            } else {
                "# "
            };
            let n = ch.display_name(&self.state.users, self.state.current_user.as_ref().map(|u| u.id.as_str()));
            let t = ch.description.clone().unwrap_or_default();
            let other_user = if ch.channel_type == "DirectMessage" {
                ch.recipients.as_ref().and_then(|r| {
                    let cur_id = self.state.current_user.as_ref().map(|u| u.id.as_str());
                    let uid = r.iter().find(|&id| Some(id.as_str()) != cur_id).or_else(|| r.first());
                    uid.and_then(|id| self.state.users.get(id)).cloned()
                })
            } else {
                None
            };
            (pfx, n, t, other_user)
        } else {
            ("", "No Channel Selected".to_string(), "".to_string(), None)
        };

        // If in DM and user avatar is available, render 2x1 avatar
        if let Some(ref u) = dm_user {
            let key = format!("avatar_{}", u.id);
            let url = self.state.avatar_url_for_user(u);
            let pfp_rect = Rect {
                x: cur_x,
                y: cur_y,
                width: 2,
                height: 1,
            };
            if let Some(ref mut cache) = self.state.image_cache {
                if let Some(proto) = cache.get_mut(&key) {
                    ratatui_image::ResizeEncodeRender::render(proto, pfp_rect, buf);
                } else if !cache.contains(&key) && !cache.is_pending(&key) {
                    cache.mark_pending(key.clone());
                    if let Some(ref tx) = self.state.image_fetch_tx {
                        let _ = tx.send((key, url));
                    }
                }
            }
            cur_x += 3;
        }

        // Render prefix
        for ch in prefix.chars() {
            buf[(cur_x, cur_y)]
                .set_char(ch)
                .set_fg(theme.primary)
                .set_style(Modifier::BOLD);
            cur_x += 1;
        }

        // Render channel name
        for ch in name.chars() {
            if cur_x >= area.right().saturating_sub(40) {
                break;
            }
            buf[(cur_x, cur_y)]
                .set_char(ch)
                .set_fg(Color::White)
                .set_style(Modifier::BOLD);
            cur_x += 1;
        }

        // Separator and topic
        if !topic.is_empty() && cur_x + 6 < area.right().saturating_sub(40) {
            let sep = "  │  ";
            for ch in sep.chars() {
                buf[(cur_x, cur_y)]
                    .set_char(ch)
                    .set_fg(Color::Rgb(60, 60, 70));
                cur_x += 1;
            }

            for ch in topic.chars() {
                if cur_x >= area.right().saturating_sub(38) {
                    break;
                }
                buf[(cur_x, cur_y)]
                    .set_char(ch)
                    .set_fg(theme.muted);
                cur_x += 1;
            }
        }

        // Right side: Member toggle button and connection status
        let mut rx = area.right().saturating_sub(2);

        // Member button: `[ 👥 Members ]`
        let member_btn = if self.state.show_member_rail {
            "[ 👥 Members ▾ ]"
        } else {
            "[ 👥 Members ▸ ]"
        };
        rx = rx.saturating_sub(member_btn.len() as u16);
        let btn_style = if self.state.show_member_rail {
            Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 50))
        } else {
            Style::default().fg(theme.muted).bg(Color::Rgb(25, 25, 30))
        };

        let mut bx = rx;
        for ch in member_btn.chars() {
            if bx < area.right() {
                buf[(bx, cur_y)]
                    .set_char(ch)
                    .set_style(btn_style);
                bx += 1;
            }
        }

        // Connection status pill
        rx = rx.saturating_sub(2);
        let (dot, dot_color, status_text) = match &self.state.gateway_status {
            GatewayStatus::Connected { latency_ms } => ("●", theme.status_online, format!("{}ms", latency_ms)),
            GatewayStatus::Connecting => ("◐", theme.status_idle, "Connecting...".to_string()),
            GatewayStatus::Disconnected(_) => ("○", theme.status_offline, "Offline".to_string()),
        };

        let status_len = (status_text.len() + 3) as u16;
        rx = rx.saturating_sub(status_len);

        if rx > cur_x + 2 {
            buf[(rx, cur_y)]
                .set_char(dot.chars().next().unwrap_or('●'))
                .set_fg(dot_color);

            let mut sx = rx + 2;
            for ch in status_text.chars() {
                buf[(sx, cur_y)]
                    .set_char(ch)
                    .set_fg(theme.muted);
                sx += 1;
            }
        }
    }
}
