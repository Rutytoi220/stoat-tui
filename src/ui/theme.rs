use crate::config::{parse_hex_color, ThemeConfig};
use ratatui::style::{Color, Modifier, Style};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Theme {
    pub primary: Color,
    pub accent: Color,
    pub bg: Color,
    pub sidebar_bg: Color,
    pub rail_bg: Color,
    pub text: Color,
    pub muted: Color,

    pub border_focused: Color,
    pub border_unfocused: Color,
    pub item_selected_bg: Color,
    pub unread_badge: Color,

    // Status colors
    pub status_online: Color,
    pub status_idle: Color,
    pub status_dnd: Color,
    pub status_offline: Color,
}

impl Theme {
    pub fn from_config(cfg: &ThemeConfig) -> Self {
        let primary = parse_hex_color(&cfg.primary);
        let accent = parse_hex_color(&cfg.accent);
        let bg = parse_hex_color(&cfg.background);
        let sidebar_bg = parse_hex_color(&cfg.sidebar_bg);
        let rail_bg = parse_hex_color(&cfg.rail_bg);
        let text = parse_hex_color(&cfg.text);
        let muted = parse_hex_color(&cfg.muted);

        Self {
            primary,
            accent,
            bg,
            sidebar_bg,
            rail_bg,
            text,
            muted,
            border_focused: primary,
            border_unfocused: Color::Rgb(50, 50, 55),
            item_selected_bg: Color::Rgb(45, 45, 55),
            unread_badge: primary,
            status_online: Color::Rgb(46, 204, 113),
            status_idle: Color::Rgb(241, 196, 15),
            status_dnd: Color::Rgb(231, 76, 60),
            status_offline: Color::Rgb(127, 140, 141),
        }
    }

    pub fn role_color(&self, user_id: &str) -> Color {
        // Deterministic hash to generate pleasant role colors
        let mut hash: u32 = 5381;
        for b in user_id.bytes() {
            hash = ((hash << 5).wrapping_add(hash)).wrapping_add(b as u32);
        }

        const PALETTE: [Color; 8] = [
            Color::Rgb(253, 102, 113), // Coral
            Color::Rgb(89, 112, 253),  // Stoat Blue
            Color::Rgb(52, 152, 219),  // Sky Blue
            Color::Rgb(155, 89, 182),  // Purple
            Color::Rgb(230, 126, 34),  // Orange
            Color::Rgb(26, 188, 156),  // Turquoise
            Color::Rgb(241, 196, 15),  // Sunflower
            Color::Rgb(231, 76, 60),   // Crimson
        ];

        PALETTE[(hash as usize) % PALETTE.len()]
    }

    pub fn focused_border_style(&self, is_focused: bool) -> Style {
        if is_focused {
            Style::default().fg(self.border_focused).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.border_unfocused)
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_config(&ThemeConfig::default())
    }
}
