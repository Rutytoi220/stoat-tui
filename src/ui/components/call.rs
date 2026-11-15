use crate::state::AppState;
use image::RgbImage;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

pub struct CallStage<'a> {
    state: &'a AppState,
}

impl<'a> CallStage<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

/// Renders an RGB image into a ratatui buffer using Unicode half-blocks ('▀').
/// Each character represents two vertical pixels:
/// Upper pixel is the foreground color, lower pixel is the background color.
pub fn render_video_frame(img: &RgbImage, area: Rect, buf: &mut Buffer) {
    let img_w = img.width();
    let img_h = img.height();
    if img_w == 0 || img_h == 0 || area.width == 0 || area.height == 0 {
        return;
    }

    for dy in 0..area.height {
        let top_y = area.top() + dy;
        // Two image rows per character row
        let img_y_top = (dy as f32 / area.height as f32 * (img_h.saturating_sub(1) as f32)) as u32;
        let img_y_bot = ((dy as f32 + 0.5) / area.height as f32 * (img_h.saturating_sub(1) as f32)).min((img_h - 1) as f32) as u32;

        for dx in 0..area.width {
            let top_x = area.left() + dx;
            let img_x = (dx as f32 / area.width as f32 * (img_w.saturating_sub(1) as f32)) as u32;

            let p_top = img.get_pixel(img_x.min(img_w - 1), img_y_top.min(img_h - 1));
            let p_bot = img.get_pixel(img_x.min(img_w - 1), img_y_bot.min(img_h - 1));

            let fg = Color::Rgb(p_top[0], p_top[1], p_top[2]);
            let bg = Color::Rgb(p_bot[0], p_bot[1], p_bot[2]);

            let cell = &mut buf[(top_x, top_y)];
            cell.set_char('▀');
            cell.set_fg(fg);
            cell.set_bg(bg);
        }
    }
}

impl<'a> Widget for CallStage<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let active_call = match &self.state.active_call {
            Some(call) => call,
            None => return,
        };

        let _theme = &self.state.theme;

        // Draw Stage Background Block
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(80, 100, 140)))
            .title(Span::styled(" 📞 Live Voice & Video Stage ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
        block.render(area, buf);

        let inner = Rect {
            x: area.left() + 1,
            y: area.top() + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };

        if inner.width < 10 || inner.height < 2 {
            return;
        }

        // Top line: Duration and Control status badges
        let duration = active_call.formatted_duration();
        let is_muted = active_call.is_muted();
        let is_deafened = active_call.is_deafened();
        let is_screensharing = active_call.is_screensharing();

        let mut status_spans = vec![
            Span::styled(format!(" [⏱️ {}] ", duration), Style::default().fg(Color::White).bg(Color::Rgb(40, 45, 60))),
            Span::raw(" "),
        ];

        if is_muted {
            status_spans.push(Span::styled(" [🎙️ MUTED] ", Style::default().fg(Color::White).bg(Color::Rgb(180, 40, 40))));
        } else {
            status_spans.push(Span::styled(" [🎙️ MIC ON] ", Style::default().fg(Color::White).bg(Color::Rgb(40, 140, 60))));
        }
        status_spans.push(Span::raw(" "));

        if is_deafened {
            status_spans.push(Span::styled(" [🎧 DEAFENED] ", Style::default().fg(Color::White).bg(Color::Rgb(180, 40, 40))));
        } else {
            status_spans.push(Span::styled(" [🎧 AUDIO ON] ", Style::default().fg(Color::White).bg(Color::Rgb(40, 140, 60))));
        }
        status_spans.push(Span::raw(" "));

        if is_screensharing {
            status_spans.push(Span::styled(" [🖥️ LIVE STREAMING] ", Style::default().fg(Color::Black).bg(Color::Rgb(255, 180, 40)).add_modifier(Modifier::BOLD)));
        } else {
            status_spans.push(Span::styled(" [🖥️ SCREEN OFF] ", Style::default().fg(Color::Rgb(150, 150, 160)).bg(Color::Rgb(40, 40, 50))));
        }

        let line = Line::from(status_spans);
        buf.set_line(inner.left(), inner.top(), &line, inner.width);

        // Stage body: If screensharing or video frame exists, render video canvas; else render participant tiles
        let body_area = Rect {
            x: inner.left(),
            y: inner.top() + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };

        if body_area.height == 0 {
            return;
        }

        if let Ok(frame_guard) = active_call.current_frame.try_lock() {
            if let Some(ref img) = *frame_guard {
                render_video_frame(img, body_area, buf);
                return;
            }
        }

        // Render participant tiles in stage
        let mut py = body_area.top() + 1;
        let self_name = self.state.current_user.as_ref().map(|u| u.name()).unwrap_or("You");
        let self_spans = vec![
            Span::styled(if is_muted { " [🔇] " } else { " [🎙️] " }, Style::default().fg(if is_muted { Color::Red } else { Color::Green })),
            Span::styled(format!("{} (You)", self_name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(if is_screensharing { " (Streaming)" } else { "" }, Style::default().fg(Color::Yellow)),
        ];
        buf.set_line(body_area.left() + 2, py, &Line::from(self_spans), body_area.width.saturating_sub(4));
        py += 1;

        if let Some(participants) = self.state.voice_channel_participants.get(&active_call.channel_id) {
            for p in participants {
                if py >= body_area.bottom() {
                    break;
                }
                let uname = if let Some(u) = self.state.resolve_user(&p.user_id) {
                    u.name().to_string()
                } else {
                    format!("User {}", &p.user_id[..p.user_id.len().min(6)])
                };

                let mic_icon = if p.is_speaking {
                    " [🗣️ SPEAKING] "
                } else if p.is_muted {
                    " [🔇 MUTED] "
                } else {
                    " [🎙️ IDLE] "
                };
                let icon_color = if p.is_speaking {
                    Color::Green
                } else if p.is_muted {
                    Color::Red
                } else {
                    Color::Gray
                };

                let mut p_spans = vec![
                    Span::styled(mic_icon, Style::default().fg(icon_color).add_modifier(if p.is_speaking { Modifier::BOLD } else { Modifier::empty() })),
                    Span::styled(uname, Style::default().fg(Color::White)),
                ];
                if p.is_screensharing {
                    p_spans.push(Span::styled(" [🖥️ LIVE]", Style::default().fg(Color::Yellow)));
                }

                buf.set_line(body_area.left() + 2, py, &Line::from(p_spans), body_area.width.saturating_sub(4));
                py += 1;
            }
        }
    }
}
