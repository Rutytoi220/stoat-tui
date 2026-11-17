use crate::state::{AppState, PaletteItemType};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub struct CommandPaletteModal<'a> {
    state: &'a AppState,
}

impl<'a> CommandPaletteModal<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for CommandPaletteModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_command_palette || area.width == 0 || area.height == 0 {
            return;
        }

        let theme = &self.state.theme;
        let borderless = area.width < 36 || area.height < 14;

        // Clear the popup area
        Clear.render(area, buf);

        // Fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.sidebar_bg);
            }
        }

        let inner = if borderless {
            area
        } else {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
                .title(" Quick Switcher (Ctrl+P) ")
                .title_bottom(" ↑/↓: Navigate • Enter: Jump • Esc: Close ");

            let inner = block.inner(area);
            block.render(area, buf);
            inner
        };

        if inner.height < 1 || inner.width < 2 {
            return;
        }

        let pad: u16 = if borderless { 0 } else { 1 };
        let start_x = inner.left() + pad;
        let max_w = inner.width.saturating_sub(pad.saturating_mul(2)) as usize;
        let mut cur_y = inner.top();

        // 1. Search Query Box
        let prompt = if borderless { "/" } else { "🔍 " };
        let mut qx = crate::ui::write_str(buf, inner, start_x, cur_y, prompt, ratatui::style::Style::default().fg(theme.primary));

        let query = &self.state.command_palette_query;
        if query.is_empty() {
            let placeholder = if borderless { "Search..." } else { "Type a channel, server, or user name..." };
            let clipped: String = placeholder.chars().take(max_w.saturating_sub(2)).collect();
            qx = crate::ui::write_str(buf, inner, qx, cur_y, &clipped, ratatui::style::Style::default().fg(theme.muted));
        } else {
            let clipped: String = query.chars().take(max_w.saturating_sub(2)).collect();
            qx = crate::ui::write_str(
                buf,
                inner,
                qx,
                cur_y,
                &clipped,
                ratatui::style::Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            );
        }

        crate::ui::put_cell(
            buf,
            inner,
            qx,
            cur_y,
            ' ',
            ratatui::style::Style::default().bg(Color::White).fg(Color::Black),
        );

        cur_y += 1;

        // Divider if tall enough
        if !borderless && cur_y < inner.bottom() {
            for dx in inner.left()..inner.right() {
                crate::ui::put_cell(buf, inner, dx, cur_y, '─', ratatui::style::Style::default().fg(Color::Rgb(50, 50, 60)));
            }
            cur_y += 1;
        }

        // 2. Results List
        let items = self.state.get_palette_items();
        let selected_idx = self.state.command_palette_selected;

        let visible_count = inner.bottom().saturating_sub(cur_y) as usize;
        if visible_count == 0 {
            return;
        }
        let offset = if selected_idx >= visible_count {
            selected_idx.saturating_sub(visible_count) + 1
        } else {
            0
        };

        if items.is_empty() {
            let no_res = if borderless { "None" } else { "No matching channels or servers found." };
            let msg_y = if cur_y + 1 < inner.bottom() { cur_y + 1 } else { cur_y };
            crate::ui::write_str(buf, inner, start_x, msg_y, no_res, ratatui::style::Style::default().fg(theme.muted));
            return;
        }

        for (idx, item) in items.iter().enumerate().skip(offset) {
            if cur_y >= inner.bottom() {
                break;
            }

            let is_selected = idx == selected_idx;
            let row_bg = if is_selected {
                theme.item_selected_bg
            } else {
                theme.sidebar_bg
            };

            // Highlight full line
            for rx in inner.left()..inner.right() {
                buf[(rx, cur_y)].set_bg(row_bg);
            }

            if borderless {
                let pfx = match item.item_type {
                    PaletteItemType::Server => "S:",
                    PaletteItemType::Channel => "#",
                    PaletteItemType::DirectMessage => "@",
                };
                let style = if is_selected {
                    ratatui::style::Style::default().fg(Color::White).bg(row_bg).add_modifier(Modifier::BOLD)
                } else {
                    ratatui::style::Style::default().fg(Color::Rgb(220, 220, 230)).bg(row_bg)
                };
                crate::ui::write_str(buf, inner, start_x, cur_y, &format!("{}{}", pfx, item.title), style);
                cur_y += 1;
                continue;
            }

            // Tag badge: [Server] or [# Channel] or [@ DM]
            let (tag, tag_color) = match item.item_type {
                PaletteItemType::Server => ("[SERVER]", Color::Rgb(150, 150, 250)),
                PaletteItemType::Channel => ("[CHANNEL]", Color::Cyan),
                PaletteItemType::DirectMessage => ("[DM]", theme.primary),
            };

            let mut tx = crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                tag,
                ratatui::style::Style::default().fg(tag_color).bg(row_bg).add_modifier(Modifier::BOLD),
            );

            tx += 2;

            // Title
            let title_style = if is_selected {
                ratatui::style::Style::default().fg(Color::White).bg(row_bg).add_modifier(Modifier::BOLD)
            } else {
                ratatui::style::Style::default().fg(Color::Rgb(220, 220, 230)).bg(row_bg)
            };

            let title_clip: String = item.title.chars().take(30).collect();
            tx = crate::ui::write_str(buf, inner, tx, cur_y, &title_clip, title_style);

            tx += 2;

            // Subtitle
            let sub_clip: String = item.subtitle.chars().take(max_w.saturating_sub((tx.saturating_sub(start_x)) as usize)).collect();
            crate::ui::write_str(buf, inner, tx, cur_y, &sub_clip, ratatui::style::Style::default().fg(theme.muted).bg(row_bg));

            cur_y += 1;
        }
    }
}
