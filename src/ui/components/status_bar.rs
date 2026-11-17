use crate::state::{AppState, Focus, GatewayStatus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthChar;

/// Strips HTML bodies and newlines, and extracts Stoat JSON `"type"` error codes
/// so multi-line or verbose server responses never overflow the 1-row StatusBar.
pub fn sanitize_status_text(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let html_pos = lower
        .find("<!doctype")
        .into_iter()
        .chain(lower.find("<html"))
        .min();
    let replaced = if let Some(idx) = html_pos {
        let prefix = raw[..idx].trim_end();
        if prefix.is_empty() {
            "Unexpected HTML response from server (check debug.log)".to_string()
        } else {
            format!("{} Unexpected HTML response from server (check debug.log)", prefix)
        }
    } else if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) {
        if start < end {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw[start..=end]) {
                if let Some(err_type) = val.get("type").and_then(|v| v.as_str()) {
                    let prefix = raw[..start].trim_end();
                    let suffix = raw[end + 1..].trim_start();
                    match (prefix.is_empty(), suffix.is_empty()) {
                        (true, true) => err_type.to_string(),
                        (false, true) => format!("{} {}", prefix, err_type),
                        (true, false) => format!("{} {}", err_type, suffix),
                        (false, false) => format!("{} {} {}", prefix, err_type, suffix),
                    }
                } else {
                    raw.to_string()
                }
            } else {
                raw.to_string()
            }
        } else {
            raw.to_string()
        }
    } else {
        raw.to_string()
    };

    replaced
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct StatusBar<'a> {
    state: &'a AppState,
}

impl<'a> StatusBar<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let theme = &self.state.theme;

        // Render bar background
        for x in area.left()..area.right() {
            buf[(x, area.top())]
                .set_char(' ')
                .set_bg(theme.rail_bg);
        }

        let y = area.top();

        // Compact tiny bar when width < 36
        if area.width < 36 {
            let recent_status = self.state.status_message.as_ref().and_then(|(msg, ts)| {
                if chrono::Utc::now().signed_duration_since(*ts).num_seconds() < 8 {
                    Some(msg.as_str())
                } else {
                    None
                }
            });

            if let Some(status_msg) = recent_status {
                let clean_msg = sanitize_status_text(status_msg);
                let lower = clean_msg.to_lowercase();
                let is_error = clean_msg.starts_with('❌')
                    || lower.contains("failed")
                    || lower.contains("error")
                    || lower.contains("expired");
                let style = if is_error {
                    ratatui::style::Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                } else {
                    ratatui::style::Style::default().fg(Color::Rgb(200, 220, 255))
                };
                crate::ui::write_str(buf, area, area.left(), y, &clean_msg, style);
                return;
            }

            let compact_text = if !self.state.input_buffer.is_empty() {
                format!("> {}", self.state.input_buffer)
            } else {
                let pane = match self.state.focus {
                    Focus::Servers => "Srv",
                    Focus::Channels => "Ch",
                    Focus::Chat => "Chat",
                    Focus::Input => "Msg",
                    Focus::Members => "Mem",
                };
                if area.width < 16 {
                    format!("Tab:{}", pane)
                } else {
                    let handle = self
                        .state
                        .current_user
                        .as_ref()
                        .map(|u| u.username.as_str())
                        .unwrap_or("Guest");
                    format!("{} • @{}", pane, handle)
                }
            };
            crate::ui::write_str(buf, area, area.left(), y, &compact_text, ratatui::style::Style::default().fg(Color::Rgb(200, 200, 220)));

            let dot = match &self.state.gateway_status {
                GatewayStatus::Connected { .. } => ('●', theme.status_online),
                GatewayStatus::Connecting => ('◐', theme.status_idle),
                GatewayStatus::Disconnected(_) => ('○', theme.status_offline),
            };
            if area.width >= 8 {
                crate::ui::put_cell(buf, area, area.right() - 1, y, dot.0, ratatui::style::Style::default().fg(dot.1));
            }
            return;
        }

        let mut x = area.left() + 1;

        // 1. Current user & presence
        if let Some(user) = &self.state.current_user {
            crate::ui::put_cell(buf, area, x, y, '●', ratatui::style::Style::default().fg(theme.status_online));
            x += 2;

            let handle = format!("@{}", user.username);
            x = crate::ui::write_str(buf, area, x, y, &handle, ratatui::style::Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

            let status_badge = format!(" [{}]", user.presence_str());
            x = crate::ui::write_str(buf, area, x, y, &status_badge, ratatui::style::Style::default().fg(theme.muted));
        } else {
            x = crate::ui::write_str(buf, area, x, y, "Guest", ratatui::style::Style::default().fg(theme.muted));
        }

        // Separator
        x += 2;
        crate::ui::put_cell(buf, area, x, y, '│', ratatui::style::Style::default().fg(Color::Rgb(50, 50, 60)));
        x += 2;

        // 2. Recent status message OR dynamic contextual hotkeys for the current pane
        let recent_status = self.state.status_message.as_ref().and_then(|(msg, ts)| {
            if chrono::Utc::now().signed_duration_since(*ts).num_seconds() < 8 {
                Some(msg.as_str())
            } else {
                None
            }
        });

        // 3. Right side: WebSocket Ping / Latency Indicator (computed early so middle hints never collide)
        let (ping_str, ping_color) = match &self.state.gateway_status {
            GatewayStatus::Connected { latency_ms } => (format!("● {}ms", latency_ms), theme.status_online),
            GatewayStatus::Connecting => ("◐ Connecting".to_string(), theme.status_idle),
            GatewayStatus::Disconnected(_) => ("○ Offline".to_string(), theme.status_offline),
        };
        let ping_w = ping_str.chars().count() as u16;
        let right_start = area.right().saturating_sub(ping_w + 1);
        let hints_max_x = right_start.saturating_sub(2);

        if let Some(status_msg) = recent_status {
            let clean_msg = sanitize_status_text(status_msg);
            let lower = clean_msg.to_lowercase();
            let is_error = clean_msg.starts_with('❌')
                || lower.contains("failed")
                || lower.contains("error")
                || lower.contains("expired");
            let fg = if is_error { Color::Red } else { Color::Rgb(200, 220, 255) };
            for ch in clean_msg.chars() {
                let w = ch.width().unwrap_or(1).max(1) as u16;
                if x + w > hints_max_x {
                    break;
                }
                crate::ui::put_cell(
                    buf,
                    area,
                    x,
                    y,
                    ch,
                    ratatui::style::Style::default()
                        .fg(fg)
                        .add_modifier(if is_error { Modifier::BOLD } else { Modifier::empty() }),
                );
                x += w;
            }
        } else {
            let mut hints: Vec<(&str, &str, Color)> = vec![
                ("F2", if self.state.current_user.is_some() { "Account" } else { "Login" }, theme.accent),
                ("S", "Settings", theme.accent),
            ];
            if self.state.active_call.is_some() {
                hints.push(("c", "Leave", theme.primary));
                hints.push(("m", "Mute", Color::Rgb(200, 200, 220)));
                hints.push(("d", "Deafen", Color::Rgb(200, 200, 220)));
                hints.push(("s", "Share", Color::Rgb(200, 200, 220)));
                hints.push(("f", "Full", Color::Rgb(200, 200, 220)));
            } else {
                hints.push(("c", "Call", theme.primary));
                hints.push(("u", "Upload", Color::Rgb(200, 200, 220)));
                hints.push(("Ctrl+P", "Jump", Color::Rgb(200, 200, 220)));
                hints.push(("Alt+J/K", "Ch", theme.muted));
            }

            let hints_rect = Rect {
                x: area.left(),
                y: area.top(),
                width: hints_max_x.saturating_sub(area.left()),
                height: area.height,
            };

            for (idx, (keycap, label, label_fg)) in hints.into_iter().enumerate() {
                let sep_w: u16 = if idx > 0 { 3 } else { 0 };
                let item_w = sep_w + (keycap.chars().count() as u16) + 1 + (label.chars().count() as u16);
                if x + item_w > hints_max_x {
                    break;
                }
                if idx > 0 {
                    x = crate::ui::write_str(
                        buf,
                        hints_rect,
                        x,
                        y,
                        " · ",
                        ratatui::style::Style::default().fg(Color::Rgb(75, 78, 90)),
                    );
                }
                x = crate::ui::write_str(
                    buf,
                    hints_rect,
                    x,
                    y,
                    keycap,
                    ratatui::style::Style::default()
                        .fg(Color::White)
                        .bg(Color::Rgb(42, 44, 52))
                        .add_modifier(Modifier::BOLD),
                );
                x = crate::ui::write_str(
                    buf,
                    hints_rect,
                    x,
                    y,
                    &format!(" {}", label),
                    ratatui::style::Style::default().fg(label_fg),
                );
            }
        }

        if right_start > x + 1 {
            let mut rx = right_start;
            for (idx, ch) in ping_str.chars().enumerate() {
                let fg = if idx == 0 { ping_color } else { theme.muted };
                crate::ui::put_cell(buf, area, rx, y, ch, ratatui::style::Style::default().fg(fg));
                rx += 1;
            }
        }
    }
}
