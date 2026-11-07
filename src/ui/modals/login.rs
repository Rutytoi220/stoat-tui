use crate::state::{AppState, LoginTab};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub struct LoginModal<'a> {
    state: &'a AppState,
}

impl<'a> LoginModal<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for LoginModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_login_modal {
            return;
        }

        let theme = &self.state.theme;

        // Clear modal area
        Clear.render(area, buf);

        // Fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_bg(theme.sidebar_bg);
            }
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
            .title(" Welcome to Stoat TUI ")
            .title_bottom(" Tab: Switch Field/Tab • Enter: Submit • Esc: Close ");

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 8 || inner.width < 20 {
            return;
        }

        let start_x = inner.left() + 2;
        let mut cur_y = inner.top();

        // 0. API Endpoint indicator
        let api_pfx = "Server API: ";
        let mut ax = start_x;
        for ch in api_pfx.chars() {
            buf[(ax, cur_y)].set_char(ch).set_fg(theme.muted);
            ax += 1;
        }
        for ch in self.state.config.api_url.chars() {
            if ax < inner.right().saturating_sub(2) {
                buf[(ax, cur_y)].set_char(ch).set_fg(Color::Cyan);
                ax += 1;
            }
        }
        cur_y += 2;

        // 1. Tab Bar
        let tabs = [
            (LoginTab::EmailPassword, "[ 1. Email & Pass ]"),
            (LoginTab::Token, "[ 2. Token ]"),
            (LoginTab::Demo, "[ 3. Demo Mode ]"),
        ];

        let mut tx = start_x;
        for (tab, label) in tabs {
            let is_cur = self.state.login_tab == tab;
            let tab_style = if is_cur {
                Style::default().fg(Color::White).bg(theme.primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.muted)
            };

            for ch in label.chars() {
                buf[(tx, cur_y)].set_char(ch).set_style(tab_style);
                tx += 1;
            }
            tx += 2;
        }

        cur_y += 2;

        // Divider
        for dx in inner.left()..inner.right() {
            buf[(dx, cur_y)]
                .set_char('─')
                .set_fg(Color::Rgb(50, 50, 60));
        }

        cur_y += 2;

        match self.state.login_tab {
            LoginTab::EmailPassword => {
                // Email field
                let is_email_focused = self.state.login_field_index == 0;
                let email_lbl = "Email Address:";
                for (i, ch) in email_lbl.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(if is_email_focused { Color::White } else { theme.muted })
                        .set_style(Modifier::BOLD);
                }
                cur_y += 1;

                // Box for email
                let input_w = inner.width.saturating_sub(6);
                for bx in start_x..(start_x + input_w) {
                    buf[(bx, cur_y)]
                        .set_char(' ')
                        .set_bg(if is_email_focused { Color::Rgb(30, 30, 40) } else { Color::Rgb(20, 20, 25) });
                }

                let mut ex = start_x + 1;
                for ch in self.state.login_email_input.chars() {
                    buf[(ex, cur_y)].set_char(ch).set_fg(Color::White);
                    ex += 1;
                }
                if is_email_focused {
                    buf[(ex, cur_y)].set_char(' ').set_bg(Color::White).set_fg(Color::Black);
                }

                cur_y += 2;

                // Password field
                let is_pwd_focused = self.state.login_field_index == 1;
                let pwd_lbl = "Password:";
                for (i, ch) in pwd_lbl.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(if is_pwd_focused { Color::White } else { theme.muted })
                        .set_style(Modifier::BOLD);
                }
                cur_y += 1;

                for bx in start_x..(start_x + input_w) {
                    buf[(bx, cur_y)]
                        .set_char(' ')
                        .set_bg(if is_pwd_focused { Color::Rgb(30, 30, 40) } else { Color::Rgb(20, 20, 25) });
                }

                let mut px = start_x + 1;
                for _ in self.state.login_password_input.chars() {
                    buf[(px, cur_y)].set_char('•').set_fg(Color::White);
                    px += 1;
                }
                if is_pwd_focused {
                    buf[(px, cur_y)].set_char(' ').set_bg(Color::White).set_fg(Color::Black);
                }

                cur_y += 3;

                // Action hint
                let hint = if self.state.login_loading {
                    "Logging in... Please wait."
                } else {
                    "[ Press Enter to Log In ]"
                };
                for (i, ch) in hint.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(theme.primary)
                        .set_style(Modifier::BOLD);
                }
            }
            LoginTab::Token => {
                let token_lbl = "Session or Bot Token:";
                for (i, ch) in token_lbl.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(Color::White)
                        .set_style(Modifier::BOLD);
                }
                cur_y += 1;

                let input_w = inner.width.saturating_sub(6);
                for bx in start_x..(start_x + input_w) {
                    buf[(bx, cur_y)]
                        .set_char(' ')
                        .set_bg(Color::Rgb(30, 30, 40));
                }

                let mut tx = start_x + 1;
                for ch in self.state.login_token_input.chars() {
                    buf[(tx, cur_y)].set_char(ch).set_fg(Color::White);
                    tx += 1;
                }
                buf[(tx, cur_y)].set_char(' ').set_bg(Color::White).set_fg(Color::Black);

                cur_y += 3;

                let hint = "[ Press Enter to Connect with Token ]";
                for (i, ch) in hint.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(theme.primary)
                        .set_style(Modifier::BOLD);
                }
            }
            LoginTab::Demo => {
                let text1 = "Instant Offline Demo Mode";
                for (i, ch) in text1.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(Color::White)
                        .set_style(Modifier::BOLD);
                }
                cur_y += 2;

                let text2 = "Experience Stoat TUI with preloaded community servers, channels,";
                let text3 = "full markdown chat, reactions, online members, and quick switcher.";
                for (i, ch) in text2.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(theme.muted);
                }
                cur_y += 1;
                for (i, ch) in text3.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(theme.muted);
                }
                cur_y += 3;

                let hint = "[ Press Enter to Launch Demo Mode ]";
                for (i, ch) in hint.chars().enumerate() {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(theme.status_online)
                        .set_style(Modifier::BOLD);
                }
            }
        }

        // Error message if any
        if let Some(ref err) = self.state.login_error {
            cur_y += 2;
            let err_label = if err.starts_with("Error:") || err.starts_with("HTTP") {
                format!("⚠ {}", err)
            } else {
                format!("⚠ Error: {}", err)
            };

            // Word wrap error across available width
            let max_err_w = inner.width.saturating_sub(4) as usize;
            let words: Vec<&str> = err_label.split_whitespace().collect();
            let mut line = String::new();
            let mut err_lines = Vec::new();

            for word in words {
                if !line.is_empty() && line.len() + word.len() + 1 > max_err_w {
                    err_lines.push(line);
                    line = word.to_string();
                } else {
                    if !line.is_empty() {
                        line.push(' ');
                    }
                    line.push_str(word);
                }
            }
            if !line.is_empty() {
                err_lines.push(line);
            }

            for err_line in err_lines.iter().take(3) {
                if cur_y < inner.bottom() {
                    let mut ex = start_x;
                    for ch in err_line.chars() {
                        if ex < inner.right() {
                            buf[(ex, cur_y)]
                                .set_char(ch)
                                .set_fg(Color::Rgb(255, 80, 80))
                                .set_style(Modifier::BOLD);
                            ex += 1;
                        }
                    }
                    cur_y += 1;
                }
            }
        }
    }
}
