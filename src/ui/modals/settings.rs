use crate::config::ACCENT_PRESETS;
use crate::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub const SETTINGS_ITEM_COUNT: usize = 6;

pub struct SettingsModal<'a> {
    state: &'a AppState,
}

impl<'a> SettingsModal<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for SettingsModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_settings_modal || area.width == 0 || area.height == 0 {
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

        let inner = if borderless {
            area
        } else {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
                .title(" Settings (S) ")
                .title_bottom(" ↑/↓: Select • ←/→/Enter: Adjust • Esc/S: Close ");
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        };

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let preset_idx = self.state.current_accent_preset_index();
        let (preset_name, preset_hex) = ACCENT_PRESETS[preset_idx];

        let mut rows: Vec<(String, String, Color)> = vec![
            (
                if borderless { "SFX".into() } else { "Call Sound Effects".into() },
                if self.state.config.call_sounds_enabled { "[ON]".into() } else { "[OFF]".into() },
                if self.state.config.call_sounds_enabled { Color::Rgb(35, 165, 90) } else { theme.muted },
            ),
            (
                if borderless { "SideW".into() } else { "Left Sidebar Width".into() },
                if borderless {
                    format!("{}", self.state.left_sidebar_width)
                } else {
                    format!("< {} cols >", self.state.left_sidebar_width)
                },
                Color::Cyan,
            ),
            (
                if borderless { "Dense".into() } else { "Compact / Dense Chat".into() },
                if self.state.config.compact_mode { "[ON]".into() } else { "[OFF]".into() },
                if self.state.config.compact_mode { Color::Rgb(35, 165, 90) } else { theme.muted },
            ),
            (
                if borderless { "Color".into() } else { "Accent Color Preset".into() },
                if borderless {
                    preset_name.split_whitespace().next().unwrap_or(preset_name).to_string()
                } else {
                    format!("< ● {} ({}) >", preset_name, preset_hex)
                },
                theme.primary,
            ),
            (
                if borderless { "AddAcc".into() } else { "Add / Switch Account (F2)".into() },
                if borderless { "[F2]".into() } else { "[ Enter ]".into() },
                Color::White,
            ),
            (
                if borderless { "Logout".into() } else { "Log Out (All)".into() },
                if borderless { "[Out]".into() } else { "[ Clear All ]".into() },
                Color::Rgb(240, 80, 80),
            ),
        ];

        for acc in &self.state.config.accounts {
            let host_lbl = crate::config::short_instance_label(&acc.api_url)
                .unwrap_or_else(|| "stoat.chat".to_string());
            let uname = acc
                .username
                .as_deref()
                .map(|u| format!("@{}", u))
                .unwrap_or_else(|| acc.id.clone());
            rows.push((
                if borderless {
                    format!("{}({})", uname, host_lbl)
                } else {
                    format!("Account: {} ({})", uname, host_lbl)
                },
                if borderless { "[Del]".into() } else { "[ Del/Enter: Remove ]".into() },
                Color::Rgb(230, 110, 110),
            ));
        }

        let pad: u16 = if borderless { 0 } else { 2 };
        let start_x = inner.left() + pad;
        let mut cur_y = inner.top() + if borderless { 0 } else { 1 };

        if borderless {
            crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                "Settings",
                Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
            );
            cur_y += 1;
        }

        let avail_rows = inner.bottom().saturating_sub(cur_y) as usize;
        if avail_rows == 0 {
            return;
        }

        let step: u16 = if borderless { 1 } else { 2 };
        let visible_items = ((avail_rows + (step as usize - 1)) / step as usize).max(1);
        let sel = self.state.settings_selected_index.min(rows.len().saturating_sub(1));
        let scroll_offset = if sel >= visible_items {
            sel.saturating_sub(visible_items) + 1
        } else {
            0
        };

        for (idx, (label, val, val_color)) in rows.iter().enumerate().skip(scroll_offset) {
            if cur_y >= inner.bottom() {
                break;
            }
            let is_selected = idx == sel;
            let row_bg = if is_selected {
                theme.item_selected_bg
            } else {
                theme.sidebar_bg
            };

            for rx in inner.left()..inner.right() {
                buf[(rx, cur_y)].set_bg(row_bg);
            }

            if borderless {
                let prefix = if is_selected { ">" } else { " " };
                let line = format!("{}{}:{}", prefix, label, val);
                let style = if is_selected {
                    Style::default().fg(Color::White).bg(row_bg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.muted).bg(row_bg)
                };
                crate::ui::write_str(buf, inner, start_x, cur_y, &line, style);
            } else {
                let prefix = if is_selected { "▸ " } else { "  " };
                let label_style = if is_selected {
                    Style::default().fg(Color::White).bg(row_bg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Rgb(210, 210, 225)).bg(row_bg)
                };
                crate::ui::write_str(buf, inner, start_x, cur_y, &format!("{}{}", prefix, label), label_style);

                let val_w = val.chars().count() as u16;
                let val_x = inner.right().saturating_sub(pad + val_w);
                if val_x > start_x + label.chars().count() as u16 + 2 {
                    crate::ui::write_str(
                        buf,
                        inner,
                        val_x,
                        cur_y,
                        val,
                        Style::default().fg(*val_color).bg(row_bg).add_modifier(Modifier::BOLD),
                    );
                }
            }

            cur_y += step;
        }
    }
}
