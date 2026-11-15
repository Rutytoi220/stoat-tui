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
        if !self.state.show_member_rail || area.width < 5 {
            return;
        }

        let is_focused = self.state.focus == Focus::Members;
        let theme = &self.state.theme;

        // Render member rail background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.sidebar_bg);
            }
        }

        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(theme.focused_border_style(is_focused));
        block.render(area, buf);

        if area.width < 6 || area.height < 4 {
            return;
        }

        let start_x = area.left() + 1;
        let max_w = area.width.saturating_sub(2) as usize;
        let mut cur_y = area.top() + 1;

        // Gather members for active server
        let empty_members = Vec::new();
        let server_id = self.state.active_server_id.as_deref().unwrap_or("");
        let members = self.state.members.get(server_id).unwrap_or(&empty_members);

        // Split into Online and Offline
        let mut online_members = Vec::new();
        let mut offline_members = Vec::new();

        for m in members {
            let user = self.state.users.get(&m.id.user);
            let presence = user
                .and_then(|u| u.status.as_ref())
                .and_then(|s| s.presence.as_ref())
                .unwrap_or(&Presence::Offline);

            if *presence != Presence::Offline {
                online_members.push((m, user, presence));
            } else {
                offline_members.push((m, user, presence));
            }
        }

        let mut item_index = 0;

        // 1. Online Section
        if !online_members.is_empty() {
            let header = format!("ONLINE — {}", online_members.len());
            let mut hx = start_x;
            for ch in header.chars().take(max_w) {
                buf[(hx, cur_y)]
                    .set_char(ch)
                    .set_fg(theme.muted)
                    .set_style(Modifier::BOLD);
                hx += 1;
            }
            cur_y += 2;

            for (m, user, presence) in online_members {
                if cur_y >= area.bottom() {
                    break;
                }

                let is_selected = is_focused && self.state.selected_member_index == item_index;

                let name = m.nickname.as_deref().unwrap_or_else(|| {
                    user.map(|u| u.name()).unwrap_or("Member")
                });

                let (dot, dot_color) = match presence {
                    Presence::Online => ("●", theme.status_online),
                    Presence::Idle => ("●", theme.status_idle),
                    Presence::Busy => ("●", theme.status_dnd),
                    _ => ("○", theme.status_offline),
                };

                // Draw row highlight if selected
                if is_selected {
                    for bx in start_x..(start_x + area.width.saturating_sub(2)) {
                        buf[(bx, cur_y)].set_bg(theme.item_selected_bg);
                    }
                }

                // Render status dot
                buf[(start_x, cur_y)]
                    .set_char(dot.chars().next().unwrap_or('●'))
                    .set_fg(dot_color);

                // Render 2x1 Avatar icon if available
                let pfp_rect = Rect {
                    x: start_x + 2,
                    y: cur_y,
                    width: 2,
                    height: 1,
                };
                if let Some(u) = user {
                    let key = format!("avatar_{}", u.id);
                    let url = self.state.avatar_url_for_user(u);
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
                }

                // Render name
                let name_color = theme.role_color(&m.id.user);
                let mut nx = start_x + 5;
                for ch in name.chars().take(max_w.saturating_sub(6)) {
                    buf[(nx, cur_y)]
                        .set_char(ch)
                        .set_fg(name_color);
                    nx += 1;
                }

                item_index += 1;
                cur_y += 1;
            }
        }

        // 2. Offline Section
        if !offline_members.is_empty() && cur_y + 2 < area.bottom() {
            cur_y += 1;
            let header = format!("OFFLINE — {}", offline_members.len());
            let mut hx = start_x;
            for ch in header.chars().take(max_w) {
                buf[(hx, cur_y)]
                    .set_char(ch)
                    .set_fg(theme.muted)
                    .set_style(Modifier::BOLD);
                hx += 1;
            }
            cur_y += 2;

            for (m, user, _) in offline_members {
                if cur_y >= area.bottom() {
                    break;
                }

                let is_selected = is_focused && self.state.selected_member_index == item_index;
                let name = m.nickname.as_deref().unwrap_or_else(|| {
                    user.map(|u| u.name()).unwrap_or("Member")
                });

                if is_selected {
                    for bx in start_x..(start_x + area.width.saturating_sub(2)) {
                        buf[(bx, cur_y)].set_bg(theme.item_selected_bg);
                    }
                }

                buf[(start_x, cur_y)]
                    .set_char('○')
                    .set_fg(theme.status_offline);

                let mut nx = start_x + 2;
                for ch in name.chars().take(max_w.saturating_sub(3)) {
                    buf[(nx, cur_y)]
                        .set_char(ch)
                        .set_fg(theme.muted);
                    nx += 1;
                }

                item_index += 1;
                cur_y += 1;
            }
        }
    }
}
