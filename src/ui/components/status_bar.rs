use crate::state::{AppState, Focus, GatewayStatus};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::Widget;

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
        let theme = &self.state.theme;

        // Render bar background
        for x in area.left()..area.right() {
            buf[(x, area.top())]
                .set_char(' ')
                .set_bg(theme.rail_bg);
        }

        if area.width < 10 {
            return;
        }

        let y = area.top();
        let mut x = area.left() + 1;

        // 1. Current user & presence
        if let Some(user) = &self.state.current_user {
            buf[(x, y)]
                .set_char('●')
                .set_fg(theme.status_online);
            x += 2;

            let handle = format!("@{}", user.username);
            for ch in handle.chars() {
                buf[(x, y)]
                    .set_char(ch)
                    .set_fg(Color::White)
                    .set_style(Modifier::BOLD);
                x += 1;
            }

            let status_badge = format!(" [{}]", user.presence_str());
            for ch in status_badge.chars() {
                buf[(x, y)]
                    .set_char(ch)
                    .set_fg(theme.muted);
                x += 1;
            }
        } else {
            let guest = "Guest";
            for ch in guest.chars() {
                buf[(x, y)].set_char(ch).set_fg(theme.muted);
                x += 1;
            }
        }

        // Separator
        x += 2;
        buf[(x, y)].set_char('│').set_fg(Color::Rgb(50, 50, 60));
        x += 2;

        // 2. Dynamic contextual hotkeys for the current pane
        let pane_hints = match self.state.focus {
            Focus::Servers => "Servers [ ↑/↓: Select • Enter: Switch ]",
            Focus::Channels => "Channels [ ↑/↓: Select • Enter: Switch ]",
            Focus::Chat => "Chat [ ↑/↓: Scroll • i: Type ]",
            Focus::Input => "Input [ Enter: Send • Esc: Unfocus ]",
            Focus::Members => "Members [ ↑/↓: Browse • F4: Toggle ]",
        };

        for ch in pane_hints.chars() {
            if x >= area.right().saturating_sub(76) {
                break;
            }
            buf[(x, y)]
                .set_char(ch)
                .set_fg(Color::Rgb(200, 200, 220));
            x += 1;
        }

        // F2 Login / Switch Account hint
        let login_hint = if self.state.current_user.is_some() {
            " • F2: Switch Account"
        } else {
            " • F2: Login"
        };
        for ch in login_hint.chars() {
            if x >= area.right().saturating_sub(55) {
                break;
            }
            buf[(x, y)]
                .set_char(ch)
                .set_fg(theme.accent)
                .set_style(Modifier::BOLD);
            x += 1;
        }

        // Fast navigation hints
        let nav_hint = " • Alt+J/K: Channel | Ctrl+Alt+J/K: Server | Alt+Shift+J/K: Unread";
        for ch in nav_hint.chars() {
            if x >= area.right().saturating_sub(22) {
                break;
            }
            buf[(x, y)]
                .set_char(ch)
                .set_fg(theme.primary);
            x += 1;
        }

        // 3. Right side: WebSocket Ping / Latency Indicator
        let (ping_str, ping_color) = match &self.state.gateway_status {
            GatewayStatus::Connected { latency_ms } => (format!("● {}ms", latency_ms), theme.status_online),
            GatewayStatus::Connecting => ("◐ Connecting".to_string(), theme.status_idle),
            GatewayStatus::Disconnected(_) => ("○ Offline".to_string(), theme.status_offline),
        };

        let right_start = area.right().saturating_sub(ping_str.len() as u16 + 2);
        if right_start > x + 2 {
            let mut rx = right_start;
            for (idx, ch) in ping_str.chars().enumerate() {
                let cell = &mut buf[(rx, y)];
                cell.set_char(ch);
                if idx == 0 {
                    cell.set_fg(ping_color);
                } else {
                    cell.set_fg(theme.muted);
                }
                rx += 1;
            }
        }
    }
}
