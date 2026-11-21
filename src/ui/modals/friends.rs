use crate::api::models::RelationshipStatus;
use crate::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub struct FriendsModal<'a> {
    state: &'a AppState,
}

impl<'a> FriendsModal<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for FriendsModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_friends_modal || area.width == 0 || area.height == 0 {
            return;
        }

        let theme = &self.state.theme;
        let borderless = area.width < 36 || area.height < 14;

        Clear.render(area, buf);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_char(' ').set_bg(theme.sidebar_bg);
            }
        }

        let footer_hint = if self.state.friends_input_focused {
            " Type username#0000 • Enter: Send Request • Esc: List "
        } else {
            " a: Add • g: New Group • Enter/y: Accept/DM • x: Decline/Remove • Tab: Account • Esc: Close "
        };

        let inner = if borderless {
            area
        } else {
            let incoming = self.state.incoming_friend_request_count();
            let title = if incoming > 0 {
                format!(" Friends & Requests ({} pending) ", incoming)
            } else {
                " Friends & Requests (F) ".to_string()
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
                .title(title)
                .title_bottom(footer_hint);
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        };

        if inner.width < 4 || inner.height < 2 {
            return;
        }

        let pad: u16 = if borderless { 0 } else { 1 };
        let start_x = inner.left() + pad;
        let mut cur_y = inner.top();

        // 1. Target account bar + Add Friend input box
        let target_acc = if !self.state.friends_account_id.is_empty() {
            self.state.friends_account_id.as_str()
        } else {
            self.state.active_account_id.as_str()
        };
        let acc_tag = self
            .state
            .instance_tag_for_account(target_acc)
            .unwrap_or("stoat.chat");
        let add_prefix = if self.state.friends_input_focused {
            format!("Add ({}) > ", acc_tag)
        } else {
            format!("[a] Add ({}): ", acc_tag)
        };
        let input_bg = if self.state.friends_input_focused {
            Color::Rgb(42, 46, 58)
        } else {
            Color::Rgb(30, 32, 38)
        };
        for rx in inner.left()..inner.right() {
            buf[(rx, cur_y)].set_bg(input_bg);
        }
        let next_x = crate::ui::write_str(
            buf,
            inner,
            start_x,
            cur_y,
            &add_prefix,
            Style::default()
                .fg(if self.state.friends_input_focused {
                    theme.primary
                } else {
                    theme.accent
                })
                .bg(input_bg)
                .add_modifier(Modifier::BOLD),
        );
        let input_text = if self.state.friends_input_buffer.is_empty() && !self.state.friends_input_focused {
            "username#1234 (press 'a' to type)".to_string()
        } else if self.state.friends_input_focused {
            format!("{}_", self.state.friends_input_buffer)
        } else {
            self.state.friends_input_buffer.clone()
        };
        let input_fg = if self.state.friends_input_buffer.is_empty() && !self.state.friends_input_focused {
            theme.muted
        } else {
            Color::White
        };
        crate::ui::write_str(
            buf,
            inner,
            next_x,
            cur_y,
            &input_text,
            Style::default().fg(input_fg).bg(input_bg),
        );
        cur_y += 1;

        // 2. Status feedback line if present
        if let Some(ref msg) = self.state.friends_status {
            if cur_y < inner.bottom() {
                let fg = if msg.starts_with('⚠') {
                    Color::Rgb(240, 90, 90)
                } else {
                    Color::Rgb(35, 165, 90)
                };
                crate::ui::write_str(
                    buf,
                    inner,
                    start_x,
                    cur_y,
                    msg,
                    Style::default().fg(fg).add_modifier(Modifier::BOLD),
                );
                cur_y += 1;
            }
        } else if !borderless && cur_y < inner.bottom() {
            cur_y += 1;
        }

        // 3. Friends & Requests list
        let entries = self.state.sorted_friend_entries();
        if entries.is_empty() {
            if cur_y < inner.bottom() {
                crate::ui::write_str(
                    buf,
                    inner,
                    start_x,
                    cur_y,
                    "No pending requests or friends yet. Press 'a' to add someone!",
                    Style::default().fg(theme.muted),
                );
            }
            return;
        }

        let avail = inner.bottom().saturating_sub(cur_y) as usize;
        if avail == 0 {
            return;
        }

        let sel = self.state.friends_selected_index.min(entries.len() - 1);
        let scroll = if sel >= avail { sel + 1 - avail } else { 0 };
        let show_inst = self.state.config.accounts.len() > 1;

        for (idx, (acc_id, uid, status)) in entries.iter().enumerate().skip(scroll).take(avail) {
            if cur_y >= inner.bottom() {
                break;
            }
            let is_sel = !self.state.friends_input_focused && idx == sel;
            let row_bg = if is_sel {
                theme.item_selected_bg
            } else {
                theme.sidebar_bg
            };
            for rx in inner.left()..inner.right() {
                buf[(rx, cur_y)].set_bg(row_bg);
            }

            let user_opt = self.state.users.get(uid);
            let display = if let Some(u) = user_opt {
                if let Some(ref disc) = u.discriminator {
                    if !disc.is_empty() {
                        format!("{}#{}", u.username, disc)
                    } else {
                        u.name().to_string()
                    }
                } else {
                    u.name().to_string()
                }
            } else {
                uid.chars().take(10).collect()
            };
            let inst_suffix = if show_inst {
                self.state
                    .instance_tag_for_account(acc_id)
                    .map(|t| format!(" ({})", t))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let (badge, badge_color, action_hint) = match status {
                RelationshipStatus::Incoming => (
                    "[INCOMING]",
                    Color::Rgb(250, 190, 60),
                    "[y/Enter: Accept | x: Decline]",
                ),
                RelationshipStatus::Outgoing => (
                    "[PENDING]",
                    Color::Rgb(130, 160, 240),
                    "[x: Cancel]",
                ),
                RelationshipStatus::Friend => {
                    let online = user_opt.and_then(|u| u.online).unwrap_or(false);
                    (
                        if online { "● Friend" } else { "○ Friend" },
                        if online { theme.status_online } else { theme.muted },
                        "[Enter: DM | x: Remove]",
                    )
                }
                _ => ("", theme.muted, ""),
            };

            let prefix = if is_sel { "▸ " } else { "  " };
            let mut x = crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                prefix,
                Style::default().fg(theme.primary).bg(row_bg),
            );
            x = crate::ui::write_str(
                buf,
                inner,
                x,
                cur_y,
                badge,
                Style::default().fg(badge_color).bg(row_bg).add_modifier(Modifier::BOLD),
            );
            x = crate::ui::write_str(
                buf,
                inner,
                x,
                cur_y,
                &format!(" {}{}", display, inst_suffix),
                Style::default()
                    .fg(if is_sel { Color::White } else { Color::Rgb(215, 215, 230) })
                    .bg(row_bg)
                    .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
            );

            if !borderless && is_sel {
                let hint_w = action_hint.chars().count() as u16;
                let hint_x = inner.right().saturating_sub(pad + hint_w);
                if hint_x > x + 1 {
                    crate::ui::write_str(
                        buf,
                        inner,
                        hint_x,
                        cur_y,
                        action_hint,
                        Style::default().fg(theme.muted).bg(row_bg),
                    );
                }
            }

            cur_y += 1;
        }
    }
}
