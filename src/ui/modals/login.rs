use crate::state::{AppState, LoginStep, LoginTab};
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

        let is_totp = matches!(self.state.login_step, LoginStep::TotpChallenge { .. });

        let title = if is_totp {
            " Two-Factor Authentication "
        } else {
            " Welcome to Stoat TUI "
        };

        let bottom_title = if is_totp {
            " Enter: Verify Code • Esc: Back to Login "
        } else if self.state.login_tab == LoginTab::Token {
            " Enter: Connect with Token • Tab: Switch Tab • Esc: Close "
        } else {
            " Tab: Switch Field/Tab • Enter: Submit • Esc: Close "
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
            .title(title)
            .title_bottom(bottom_title);

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 8 || inner.width < 20 {
            return;
        }

        let start_x = inner.left() + 2;
        let mut cur_y = inner.top();

        if is_totp {
            // TOTP 2FA Challenge Screen
            let header = "Two-Factor Authentication";
            for (i, ch) in header.chars().enumerate() {
                buf[(start_x + i as u16, cur_y)]
                    .set_char(ch)
                    .set_fg(Color::White)
                    .set_style(Modifier::BOLD);
            }
            cur_y += 2;

            let subtitle = "Enter the 6-digit code from your authenticator app:";
            for (i, ch) in subtitle.chars().enumerate() {
                if (start_x + i as u16) < inner.right().saturating_sub(2) {
                    buf[(start_x + i as u16, cur_y)]
                        .set_char(ch)
                        .set_fg(theme.muted);
                }
            }
            cur_y += 2;

            // Box for TOTP code (styled with space separation for clarity)
            let box_w = 24.min(inner.width.saturating_sub(4));
            let has_error = self.state.login_error.is_some();
            let box_bg = if has_error {
                Color::Rgb(45, 20, 25)
            } else {
                Color::Rgb(30, 30, 45)
            };

            for bx in start_x..(start_x + box_w) {
                buf[(bx, cur_y)]
                    .set_char(' ')
                    .set_bg(box_bg);
            }

            let code = &self.state.login_totp_input;
            let mut cx = start_x + 2;

            // Render digits formatted nicely (e.g. 1 2 3   4 5 6)
            for (idx, ch) in code.chars().enumerate() {
                if idx == 3 && cx + 1 < start_x + box_w {
                    cx += 2; // Spacing between 3-digit groups
                }
                if cx < start_x + box_w {
                    buf[(cx, cur_y)]
                        .set_char(ch)
                        .set_fg(if has_error { Color::Rgb(255, 120, 120) } else { Color::White })
                        .set_style(Modifier::BOLD);
                    cx += 2;
                }
            }

            // Blinking/solid cursor if space allows
            if code.len() < 8 && cx < start_x + box_w {
                if code.len() == 3 {
                    cx += 2;
                }
                buf[(cx, cur_y)]
                    .set_char(' ')
                    .set_bg(if has_error { Color::Rgb(255, 100, 100) } else { Color::White })
                    .set_fg(Color::Black);
            }

            cur_y += 3;

            // Action hints / Buttons
            let verify_btn = if self.state.login_loading {
                "[ Verifying code... Please wait ]"
            } else {
                "[ Verify Code (Enter) ]"
            };

            for (i, ch) in verify_btn.chars().enumerate() {
                buf[(start_x + i as u16, cur_y)]
                    .set_char(ch)
                    .set_fg(theme.primary)
                    .set_style(Modifier::BOLD);
            }

            let back_btn = "  [ Back to Login (Esc) ]";
            let mut bx = start_x + verify_btn.len() as u16;
            for ch in back_btn.chars() {
                if bx < inner.right().saturating_sub(2) {
                    buf[(bx, cur_y)]
                        .set_char(ch)
                        .set_fg(theme.muted);
                    bx += 1;
                }
            }
        } else {
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
                    if self.state.login_token_input.is_empty() {
                        let placeholder = "Paste x-session-token or bot token here...";
                        for ch in placeholder.chars() {
                            if tx < start_x + input_w {
                                buf[(tx, cur_y)].set_char(ch).set_fg(Color::Rgb(90, 90, 105));
                                tx += 1;
                            }
                        }
                    } else {
                        for ch in self.state.login_token_input.chars() {
                            if tx < start_x + input_w {
                                buf[(tx, cur_y)].set_char(ch).set_fg(Color::White);
                                tx += 1;
                            }
                        }
                        if tx < start_x + input_w {
                            buf[(tx, cur_y)].set_char(' ').set_bg(Color::White).set_fg(Color::Black);
                        }
                    }

                    cur_y += 3;

                    let hint = if self.state.login_loading {
                        "Connecting with token... Please wait."
                    } else {
                        "[ Press Enter to Connect ]"
                    };
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
        }

        // Error message if any
        if let Some(ref err) = self.state.login_error {
            cur_y += 2;
            let err_label = if err.contains("<html") || err.contains("<!DOCTYPE") || err.contains("<HTML") {
                "⚠ Server error: HTTP 401 Unauthorized (endpoint rejected request or invalid session)".to_string()
            } else if err.starts_with('⚠') {
                err.clone()
            } else {
                format!("⚠ {}", err)
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

            for err_line in err_lines.iter().take(4) {
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
