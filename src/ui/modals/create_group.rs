use crate::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub struct CreateGroupModal<'a> {
    state: &'a AppState,
}

impl<'a> CreateGroupModal<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for CreateGroupModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_create_group_modal || area.width == 0 || area.height == 0 {
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

        let footer_hint = if self.state.group_focus_section == 2 {
            " Space: Toggle Friend • Enter: Create Group • Tab: Fields • Left/Right: Account • Esc: Close "
        } else {
            " Type text • Tab: Next Field • Enter: Next/Create • Ctrl+Enter: Create • Esc: Close "
        };

        let inner = if borderless {
            area
        } else {
            let selected_n = self.state.group_selected_users.len();
            let title = if selected_n > 0 {
                format!(" Create Group ({} selected) ", selected_n)
            } else {
                " Create Group (G) ".to_string()
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD),
                )
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

        let target_acc = if !self.state.group_account_id.is_empty() {
            self.state.group_account_id.as_str()
        } else {
            self.state.active_account_id.as_str()
        };
        let acc_tag = self
            .state
            .instance_tag_for_account(target_acc)
            .unwrap_or("stoat.chat");

        // Row 0: Target account line
        if cur_y < inner.bottom() {
            let acc_line = format!("Account: {}  (Left/Right on list to switch)", acc_tag);
            crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                &acc_line,
                Style::default().fg(theme.muted),
            );
            cur_y += 1;
        }

        // Row 1: Group Name input (section 0)
        if cur_y < inner.bottom() {
            let active = self.state.group_focus_section == 0;
            let row_bg = if active {
                Color::Rgb(42, 46, 58)
            } else {
                Color::Rgb(30, 32, 38)
            };
            for rx in inner.left()..inner.right() {
                buf[(rx, cur_y)].set_bg(row_bg);
            }
            let prefix = if active { "Name > " } else { "Name:  " };
            let nx = crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                prefix,
                Style::default()
                    .fg(if active { theme.primary } else { theme.accent })
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            );
            let val = if self.state.group_name_input.is_empty() && !active {
                "Enter group name (required)".to_string()
            } else if active {
                format!("{}_", self.state.group_name_input)
            } else {
                self.state.group_name_input.clone()
            };
            let val_fg = if self.state.group_name_input.is_empty() && !active {
                theme.muted
            } else {
                Color::White
            };
            crate::ui::write_str(
                buf,
                inner,
                nx,
                cur_y,
                &val,
                Style::default().fg(val_fg).bg(row_bg),
            );
            cur_y += 1;
        }

        // Row 2: Description input (section 1)
        if cur_y < inner.bottom() {
            let active = self.state.group_focus_section == 1;
            let row_bg = if active {
                Color::Rgb(42, 46, 58)
            } else {
                Color::Rgb(30, 32, 38)
            };
            for rx in inner.left()..inner.right() {
                buf[(rx, cur_y)].set_bg(row_bg);
            }
            let prefix = if active { "Desc > " } else { "Desc:  " };
            let nx = crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                prefix,
                Style::default()
                    .fg(if active { theme.primary } else { theme.accent })
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            );
            let val = if self.state.group_desc_input.is_empty() && !active {
                "Optional description".to_string()
            } else if active {
                format!("{}_", self.state.group_desc_input)
            } else {
                self.state.group_desc_input.clone()
            };
            let val_fg = if self.state.group_desc_input.is_empty() && !active {
                theme.muted
            } else {
                Color::White
            };
            crate::ui::write_str(
                buf,
                inner,
                nx,
                cur_y,
                &val,
                Style::default().fg(val_fg).bg(row_bg),
            );
            cur_y += 1;
        }

        // Optional status / error line
        if let Some(ref status) = self.state.group_status {
            if cur_y < inner.bottom() {
                let is_err = status.starts_with('⚠') || status.starts_with('❌');
                let fg = if is_err {
                    Color::Rgb(245, 110, 110)
                } else {
                    Color::Rgb(110, 230, 145)
                };
                crate::ui::write_str(
                    buf,
                    inner,
                    start_x,
                    cur_y,
                    status,
                    Style::default().fg(fg).add_modifier(Modifier::BOLD),
                );
                cur_y += 1;
            }
        }

        // Section header for friends list
        if cur_y < inner.bottom() {
            let list_active = self.state.group_focus_section == 2;
            let header = format!(
                "Friends on {} ({} selected):",
                acc_tag,
                self.state.group_selected_users.len()
            );
            crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                &header,
                Style::default()
                    .fg(if list_active {
                        theme.primary
                    } else {
                        theme.accent
                    })
                    .add_modifier(Modifier::BOLD),
            );
            cur_y += 1;
        }

        let friends = self.state.friend_ids_for_account(target_acc);
        if friends.is_empty() {
            if cur_y < inner.bottom() {
                crate::ui::write_str(
                    buf,
                    inner,
                    start_x,
                    cur_y,
                    "No friends on this account yet — press Shift+F to add friends.",
                    Style::default().fg(theme.muted),
                );
            }
            return;
        }

        let avail_rows = inner.bottom().saturating_sub(cur_y) as usize;
        if avail_rows == 0 {
            return;
        }

        let list_active = self.state.group_focus_section == 2;
        let cursor = self.state.group_member_cursor.min(friends.len() - 1);
        let scroll = if cursor >= avail_rows {
            cursor + 1 - avail_rows
        } else {
            0
        };

        for (idx, uid) in friends.iter().enumerate().skip(scroll) {
            if cur_y >= inner.bottom() {
                break;
            }
            let is_cursor = list_active && idx == cursor;
            let is_checked = self.state.group_selected_users.contains(uid);
            let row_bg = if is_cursor {
                Color::Rgb(46, 50, 66)
            } else {
                theme.sidebar_bg
            };
            for rx in inner.left()..inner.right() {
                buf[(rx, cur_y)].set_bg(row_bg);
            }

            let checkbox = if is_checked { "[✓] " } else { "[ ] " };
            let cb_fg = if is_checked {
                Color::Rgb(95, 225, 130)
            } else {
                theme.muted
            };
            let nx = crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                checkbox,
                Style::default()
                    .fg(cb_fg)
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD),
            );

            let (label, online) = if let Some(u) = self.state.users.get(uid) {
                let tag = if let Some(ref d) = u.discriminator {
                    format!("@{}#{}", u.username, d)
                } else {
                    format!("@{}", u.username)
                };
                let label = if let Some(ref dn) = u.display_name {
                    if !dn.trim().is_empty() && dn != &u.username {
                        format!("{} ({})", dn, tag)
                    } else {
                        tag
                    }
                } else {
                    tag
                };
                (label, u.online.unwrap_or(false))
            } else {
                (uid.clone(), false)
            };

            let dot = if online { "● " } else { "○ " };
            let dot_fg = if online {
                Color::Rgb(85, 220, 120)
            } else {
                theme.muted
            };
            let nx2 = crate::ui::write_str(
                buf,
                inner,
                nx,
                cur_y,
                dot,
                Style::default().fg(dot_fg).bg(row_bg),
            );
            crate::ui::write_str(
                buf,
                inner,
                nx2,
                cur_y,
                &label,
                Style::default()
                    .fg(if is_cursor || is_checked {
                        Color::White
                    } else {
                        theme.text
                    })
                    .bg(row_bg),
            );
            cur_y += 1;
        }
    }
}
