use crate::state::AppState;
use crate::ui::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub struct ScreenPickerModal<'a> {
    pub state: &'a AppState,
}

impl<'a> ScreenPickerModal<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for ScreenPickerModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_screen_picker_modal || area.width == 0 || area.height == 0 {
            return;
        }

        let theme = Theme::from_config(&self.state.config.theme);
        Clear.render(area, buf);

        let block = Block::default()
            .title(" 🖥 Select Screen Share Source ")
            .title_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .style(Style::default().bg(theme.sidebar_bg));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 4 || inner.height < 2 {
            return;
        }

        let start_x = inner.left() + 1;
        let mut cur_y = inner.top();
        let footer_y = inner.bottom().saturating_sub(1);

        if self.state.screen_picker_targets.is_empty() {
            crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                "No capture sources available",
                Style::default().fg(theme.muted),
            );
        } else {
            let max_rows = footer_y.saturating_sub(cur_y) as usize;
            let total = self.state.screen_picker_targets.len();
            let sel = self.state.screen_picker_selected.min(total - 1);
            let scroll = if max_rows > 0 && sel >= max_rows {
                sel + 1 - max_rows
            } else {
                0
            };

            for (idx, target) in self
                .state
                .screen_picker_targets
                .iter()
                .enumerate()
                .skip(scroll)
                .take(max_rows)
            {
                let is_sel = idx == sel;
                let row_style = if is_sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text).bg(theme.sidebar_bg)
                };

                for x in inner.left()..inner.right() {
                    crate::ui::put_cell(buf, inner, x, cur_y, ' ', row_style);
                }
                let prefix = if is_sel { "▸ " } else { "  " };
                let line = format!("{}{}", prefix, target.label);
                crate::ui::write_str(buf, inner, start_x, cur_y, &line, row_style);
                cur_y += 1;
                if cur_y >= footer_y {
                    break;
                }
            }
        }

        if footer_y >= inner.top() && footer_y < inner.bottom() {
            crate::ui::write_str(
                buf,
                inner,
                start_x,
                footer_y,
                "[↑/↓ or j/k] Select   [Enter] Share   [Esc] Cancel",
                Style::default().fg(theme.muted),
            );
        }
    }
}
