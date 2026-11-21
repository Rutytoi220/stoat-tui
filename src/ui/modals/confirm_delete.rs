use crate::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub struct ConfirmDeleteModal<'a> {
    state: &'a AppState,
}

impl<'a> ConfirmDeleteModal<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for ConfirmDeleteModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Some((_, _, ref name, ref action)) = self.state.pending_delete_channel else {
            return;
        };
        if area.width == 0 || area.height == 0 {
            return;
        }

        let theme = &self.state.theme;
        let borderless = area.width < 32 || area.height < 7;

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
                .border_style(
                    Style::default()
                        .fg(Color::Rgb(245, 100, 100))
                        .add_modifier(Modifier::BOLD),
                )
                .title(" Confirm Delete / Leave ")
                .title_bottom(" Enter / y / Ctrl+X: Confirm • Esc / n: Cancel ");
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

        let prompt = format!("Are you sure you want to {} '{}'?", action, name);
        crate::ui::write_str(
            buf,
            inner,
            start_x,
            cur_y,
            &prompt,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        cur_y += 2;

        if cur_y < inner.bottom() {
            let nx = crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                "[ Enter / y: Confirm ]",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(190, 45, 55))
                    .add_modifier(Modifier::BOLD),
            );
            crate::ui::write_str(
                buf,
                inner,
                nx + 2,
                cur_y,
                "[ Esc / n: Cancel ]",
                Style::default()
                    .fg(theme.text)
                    .bg(Color::Rgb(50, 52, 64)),
            );
        }
    }
}
