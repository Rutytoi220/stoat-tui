use crate::state::{AppState, LoginStep, LoginTab};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

pub struct LoginModal<'a> {
    state: &'a mut AppState,
}

impl<'a> LoginModal<'a> {
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for LoginModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.show_login_modal || area.width == 0 || area.height == 0 {
            return;
        }

        let theme = &self.state.theme;
        let borderless = area.width < 36 || area.height < 14;

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

        let inner = if borderless {
            area
        } else {
            let title = if is_totp {
                " Two-Factor Authentication "
            } else {
                " Welcome to Stoat TUI "
            };

            let bottom_title = if is_totp {
                " Enter: Verify Code • Esc: Back to Login "
            } else {
                " Alt+1/2/3 or Click: Switch Tab • Tab: Next Field • Enter: Submit • Esc: Close "
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
                .title(title)
                .title_bottom(bottom_title);

            let inner = block.inner(area);
            block.render(area, buf);
            inner
        };

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        // Compact borderless login modal for tiny terminals (e.g. 10x10)
        if borderless {
            self.state.rect_login_tabs.clear();
            self.state.rect_login_fields.clear();
            self.state.rect_login_submit_btn = None;

            let mut cy = inner.top();
            if is_totp {
                crate::ui::write_str(
                    buf,
                    inner,
                    inner.left(),
                    cy,
                    "2FA Code:",
                    Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
                );
                cy += 1;
                if cy < inner.bottom() {
                    let next_x = crate::ui::write_str(
                        buf,
                        inner,
                        inner.left(),
                        cy,
                        &self.state.login_totp_input,
                        Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 45)),
                    );
                    crate::ui::put_cell(buf, inner, next_x, cy, ' ', Style::default().bg(Color::White).fg(Color::Black));
                    cy += 1;
                }
            } else {
                let title_line = match self.state.login_tab {
                    LoginTab::EmailPassword => "Login:1-Em",
                    LoginTab::Token => "Login:2-Tk",
                    LoginTab::Demo => "Login:3-Dm",
                };
                crate::ui::write_str(
                    buf,
                    inner,
                    inner.left(),
                    cy,
                    title_line,
                    Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
                );
                cy += 1;

                match self.state.login_tab {
                    LoginTab::EmailPassword => {
                        if cy < inner.bottom() {
                            let lbl = if self.state.login_field_index == 0 { ">Email:" } else { "Email:" };
                            crate::ui::write_str(buf, inner, inner.left(), cy, lbl, Style::default().fg(Color::White));
                            cy += 1;
                        }
                        if cy < inner.bottom() {
                            let val = &self.state.login_email_input;
                            let nx = crate::ui::write_str(buf, inner, inner.left(), cy, val, Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 40)));
                            if self.state.login_field_index == 0 {
                                crate::ui::put_cell(buf, inner, nx, cy, ' ', Style::default().bg(Color::White).fg(Color::Black));
                            }
                            cy += 1;
                        }
                        if cy < inner.bottom() {
                            let lbl = if self.state.login_field_index == 1 { ">Pass:" } else { "Pass:" };
                            crate::ui::write_str(buf, inner, inner.left(), cy, lbl, Style::default().fg(Color::White));
                            cy += 1;
                        }
                        if cy < inner.bottom() {
                            let masked: String = self.state.login_password_input.chars().map(|_| '•').collect();
                            let nx = crate::ui::write_str(buf, inner, inner.left(), cy, &masked, Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 40)));
                            if self.state.login_field_index == 1 {
                                crate::ui::put_cell(buf, inner, nx, cy, ' ', Style::default().bg(Color::White).fg(Color::Black));
                            }
                            cy += 1;
                        }
                    }
                    LoginTab::Token => {
                        if cy < inner.bottom() {
                            crate::ui::write_str(buf, inner, inner.left(), cy, "Token:", Style::default().fg(Color::White));
                            cy += 1;
                        }
                        if cy < inner.bottom() {
                            let nx = crate::ui::write_str(
                                buf,
                                inner,
                                inner.left(),
                                cy,
                                &self.state.login_token_input,
                                Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 40)),
                            );
                            crate::ui::put_cell(buf, inner, nx, cy, ' ', Style::default().bg(Color::White).fg(Color::Black));
                            cy += 1;
                        }
                    }
                    LoginTab::Demo => {
                        if cy < inner.bottom() {
                            crate::ui::write_str(buf, inner, inner.left(), cy, "Demo Mode", Style::default().fg(theme.status_online).add_modifier(Modifier::BOLD));
                            cy += 1;
                        }
                    }
                }
            }

            if cy < inner.bottom() {
                let act = if self.state.login_loading { "Wait..." } else { "[Enter]Go" };
                crate::ui::write_str(buf, inner, inner.left(), cy, act, Style::default().fg(theme.primary).add_modifier(Modifier::BOLD));
                cy += 1;
            }
            if cy < inner.bottom() {
                if let Some(ref err) = self.state.login_error {
                    crate::ui::write_str(buf, inner, inner.left(), cy, err, Style::default().fg(Color::Rgb(255, 80, 80)));
                }
            }
            return;
        }

        if inner.height < 8 || inner.width < 20 {
            return;
        }

        let start_x = inner.left() + 2;
        let mut cur_y = inner.top();

        if is_totp {
            // TOTP 2FA Challenge Screen
            crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                "Two-Factor Authentication",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            );
            cur_y += 2;

            crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                "Enter the 6-digit code from your authenticator app:",
                Style::default().fg(theme.muted),
            );
            cur_y += 2;

            let box_w = 24.min(inner.width.saturating_sub(4));
            let has_error = self.state.login_error.is_some();
            let box_bg = if has_error {
                Color::Rgb(45, 20, 25)
            } else {
                Color::Rgb(30, 30, 45)
            };

            for bx in start_x..(start_x + box_w) {
                crate::ui::put_cell(buf, inner, bx, cur_y, ' ', Style::default().bg(box_bg));
            }

            let code = &self.state.login_totp_input;
            let mut cx = start_x + 2;

            for (idx, ch) in code.chars().enumerate() {
                if idx == 3 && cx + 1 < start_x + box_w {
                    cx += 2;
                }
                if cx < start_x + box_w {
                    let fg = if has_error { Color::Rgb(255, 120, 120) } else { Color::White };
                    crate::ui::put_cell(buf, inner, cx, cur_y, ch, Style::default().fg(fg).bg(box_bg).add_modifier(Modifier::BOLD));
                    cx += 2;
                }
            }

            if code.len() < 8 && cx < start_x + box_w {
                if code.len() == 3 {
                    cx += 2;
                }
                let bg = if has_error { Color::Rgb(255, 100, 100) } else { Color::White };
                crate::ui::put_cell(buf, inner, cx, cur_y, ' ', Style::default().bg(bg).fg(Color::Black));
            }

            cur_y += 3;

            let verify_btn = if self.state.login_loading {
                "[ Verifying code... Please wait ]"
            } else {
                "[ Verify Code (Enter) ]"
            };

            let bx = crate::ui::write_str(
                buf,
                inner,
                start_x,
                cur_y,
                verify_btn,
                Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
            );
            crate::ui::write_str(buf, inner, bx, cur_y, "  [ Back to Login (Esc) ]", Style::default().fg(theme.muted));
        } else {
            // 1. Tab Bar
            let tabs = [
                (LoginTab::EmailPassword, "[ 1. Email & Pass ]"),
                (LoginTab::Token, "[ 2. Token ]"),
                (LoginTab::Demo, "[ 3. Demo Mode ]"),
            ];

            self.state.rect_login_tabs.clear();
            self.state.rect_login_fields.clear();
            self.state.rect_login_submit_btn = None;

            let mut tx = start_x;
            for (tab, label) in tabs {
                let is_cur = self.state.login_tab == tab;
                let tab_style = if is_cur {
                    Style::default().fg(Color::White).bg(theme.primary).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.muted)
                };

                let tab_len = label.chars().count() as u16;
                self.state.rect_login_tabs.push((
                    tab,
                    Rect {
                        x: tx,
                        y: cur_y,
                        width: tab_len.min(inner.right().saturating_sub(tx)),
                        height: 1,
                    },
                ));

                tx = crate::ui::write_str(buf, inner, tx, cur_y, label, tab_style);
                tx += 2;
            }

            cur_y += 1;

            // Divider
            for dx in inner.left()..inner.right() {
                crate::ui::put_cell(buf, inner, dx, cur_y, '─', Style::default().fg(Color::Rgb(50, 50, 60)));
            }

            cur_y += 2;

            let input_w = inner.width.saturating_sub(6);

            match self.state.login_tab {
                LoginTab::EmailPassword => {
                    // Field 0: Email
                    let is_email_focused = self.state.login_field_index == 0;
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        "Email Address:",
                        Style::default()
                            .fg(if is_email_focused { Color::White } else { theme.muted })
                            .add_modifier(Modifier::BOLD),
                    );
                    cur_y += 1;

                    self.state.rect_login_fields.push((
                        0,
                        Rect {
                            x: start_x,
                            y: cur_y,
                            width: input_w,
                            height: 1,
                        },
                    ));

                    let email_bg = if is_email_focused { Color::Rgb(30, 30, 40) } else { Color::Rgb(20, 20, 25) };
                    for bx in start_x..(start_x + input_w) {
                        crate::ui::put_cell(buf, inner, bx, cur_y, ' ', Style::default().bg(email_bg));
                    }

                    let field_limit = Rect {
                        x: start_x,
                        y: cur_y,
                        width: input_w,
                        height: 1,
                    };
                    let ex = crate::ui::write_str(buf, field_limit, start_x + 1, cur_y, &self.state.login_email_input, Style::default().fg(Color::White).bg(email_bg));
                    if is_email_focused {
                        crate::ui::put_cell(buf, field_limit, ex, cur_y, ' ', Style::default().bg(Color::White).fg(Color::Black));
                    }

                    cur_y += 2;

                    // Field 1: Password
                    let is_pwd_focused = self.state.login_field_index == 1;
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        "Password:",
                        Style::default()
                            .fg(if is_pwd_focused { Color::White } else { theme.muted })
                            .add_modifier(Modifier::BOLD),
                    );
                    cur_y += 1;

                    self.state.rect_login_fields.push((
                        1,
                        Rect {
                            x: start_x,
                            y: cur_y,
                            width: input_w,
                            height: 1,
                        },
                    ));

                    let pwd_bg = if is_pwd_focused { Color::Rgb(30, 30, 40) } else { Color::Rgb(20, 20, 25) };
                    for bx in start_x..(start_x + input_w) {
                        crate::ui::put_cell(buf, inner, bx, cur_y, ' ', Style::default().bg(pwd_bg));
                    }

                    let pwd_limit = Rect {
                        x: start_x,
                        y: cur_y,
                        width: input_w,
                        height: 1,
                    };
                    let masked: String = self.state.login_password_input.chars().map(|_| '•').collect();
                    let px = crate::ui::write_str(buf, pwd_limit, start_x + 1, cur_y, &masked, Style::default().fg(Color::White).bg(pwd_bg));
                    if is_pwd_focused {
                        crate::ui::put_cell(buf, pwd_limit, px, cur_y, ' ', Style::default().bg(Color::White).fg(Color::Black));
                    }

                    cur_y += 2;

                    // Field 2: Server API URL
                    let is_api_focused = self.state.login_field_index == 2;
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        "Server API URL:",
                        Style::default()
                            .fg(if is_api_focused { Color::Cyan } else { theme.muted })
                            .add_modifier(Modifier::BOLD),
                    );
                    cur_y += 1;

                    self.state.rect_login_fields.push((
                        2,
                        Rect {
                            x: start_x,
                            y: cur_y,
                            width: input_w,
                            height: 1,
                        },
                    ));

                    let api_bg = if is_api_focused { Color::Rgb(30, 35, 48) } else { Color::Rgb(20, 20, 25) };
                    for bx in start_x..(start_x + input_w) {
                        crate::ui::put_cell(buf, inner, bx, cur_y, ' ', Style::default().bg(api_bg));
                    }
                    let api_limit = Rect {
                        x: start_x,
                        y: cur_y,
                        width: input_w,
                        height: 1,
                    };
                    let ax = crate::ui::write_str(buf, api_limit, start_x + 1, cur_y, &self.state.login_api_input, Style::default().fg(Color::Cyan).bg(api_bg));
                    if is_api_focused {
                        crate::ui::put_cell(buf, api_limit, ax, cur_y, ' ', Style::default().bg(Color::White).fg(Color::Black));
                    }

                    cur_y += 2;

                    // Field 3: Mode Selector
                    let is_mode_focused = self.state.login_field_index == 3;
                    let mode_label = if self.state.login_add_account {
                        "[ Add Account (Keep Existing) ]"
                    } else {
                        "[ Replace Primary ]"
                    };
                    self.state.rect_login_fields.push((
                        3,
                        Rect {
                            x: start_x,
                            y: cur_y,
                            width: mode_label.chars().count() as u16,
                            height: 1,
                        },
                    ));
                    let mode_style = if is_mode_focused {
                        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Cyan)
                    };
                    let mx_end = crate::ui::write_str(buf, inner, start_x, cur_y, mode_label, mode_style);
                    crate::ui::write_str(buf, inner, mx_end + 1, cur_y, "(Space/←/→)", Style::default().fg(theme.muted));

                    cur_y += 2;

                    let hint = if self.state.login_loading {
                        "Logging in... Please wait."
                    } else {
                        "[ Press Enter to Log In ]"
                    };

                    self.state.rect_login_submit_btn = Some(Rect {
                        x: start_x,
                        y: cur_y,
                        width: hint.chars().count() as u16,
                        height: 1,
                    });

                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        hint,
                        Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
                    );
                }
                LoginTab::Token => {
                    // Field 0: Session / Bot Token
                    let is_tok_focused = self.state.login_field_index == 0;
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        "Session or Bot Token:",
                        Style::default().fg(if is_tok_focused { Color::White } else { theme.muted }).add_modifier(Modifier::BOLD),
                    );
                    cur_y += 1;

                    self.state.rect_login_fields.push((
                        0,
                        Rect {
                            x: start_x,
                            y: cur_y,
                            width: input_w,
                            height: 1,
                        },
                    ));

                    let tok_bg = if is_tok_focused { Color::Rgb(30, 30, 40) } else { Color::Rgb(20, 20, 25) };
                    for bx in start_x..(start_x + input_w) {
                        crate::ui::put_cell(buf, inner, bx, cur_y, ' ', Style::default().bg(tok_bg));
                    }

                    let tok_limit = Rect {
                        x: start_x,
                        y: cur_y,
                        width: input_w,
                        height: 1,
                    };
                    if self.state.login_token_input.is_empty() {
                        crate::ui::write_str(
                            buf,
                            tok_limit,
                            start_x + 1,
                            cur_y,
                            "Paste x-session-token or bot token here...",
                            Style::default().fg(Color::Rgb(90, 90, 105)).bg(tok_bg),
                        );
                        if is_tok_focused {
                            crate::ui::put_cell(buf, tok_limit, start_x + 1, cur_y, ' ', Style::default().bg(Color::White).fg(Color::Black));
                        }
                    } else {
                        let tx = crate::ui::write_str(
                            buf,
                            tok_limit,
                            start_x + 1,
                            cur_y,
                            &self.state.login_token_input,
                            Style::default().fg(Color::White).bg(tok_bg),
                        );
                        if is_tok_focused {
                            crate::ui::put_cell(buf, tok_limit, tx, cur_y, ' ', Style::default().bg(Color::White).fg(Color::Black));
                        }
                    }

                    cur_y += 2;

                    // Field 1: Server API URL
                    let is_api_focused = self.state.login_field_index == 1;
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        "Server API URL:",
                        Style::default()
                            .fg(if is_api_focused { Color::Cyan } else { theme.muted })
                            .add_modifier(Modifier::BOLD),
                    );
                    cur_y += 1;

                    self.state.rect_login_fields.push((
                        1,
                        Rect {
                            x: start_x,
                            y: cur_y,
                            width: input_w,
                            height: 1,
                        },
                    ));

                    let api_bg = if is_api_focused { Color::Rgb(30, 35, 48) } else { Color::Rgb(20, 20, 25) };
                    for bx in start_x..(start_x + input_w) {
                        crate::ui::put_cell(buf, inner, bx, cur_y, ' ', Style::default().bg(api_bg));
                    }
                    let api_limit = Rect {
                        x: start_x,
                        y: cur_y,
                        width: input_w,
                        height: 1,
                    };
                    let ax = crate::ui::write_str(buf, api_limit, start_x + 1, cur_y, &self.state.login_api_input, Style::default().fg(Color::Cyan).bg(api_bg));
                    if is_api_focused {
                        crate::ui::put_cell(buf, api_limit, ax, cur_y, ' ', Style::default().bg(Color::White).fg(Color::Black));
                    }

                    cur_y += 2;

                    // Field 2: Mode Selector
                    let is_mode_focused = self.state.login_field_index == 2;
                    let mode_label = if self.state.login_add_account {
                        "[ Add Account (Keep Existing) ]"
                    } else {
                        "[ Replace Primary ]"
                    };
                    self.state.rect_login_fields.push((
                        2,
                        Rect {
                            x: start_x,
                            y: cur_y,
                            width: mode_label.chars().count() as u16,
                            height: 1,
                        },
                    ));
                    let mode_style = if is_mode_focused {
                        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Cyan)
                    };
                    let mx_end = crate::ui::write_str(buf, inner, start_x, cur_y, mode_label, mode_style);
                    crate::ui::write_str(buf, inner, mx_end + 1, cur_y, "(Space/←/→)", Style::default().fg(theme.muted));

                    cur_y += 2;

                    let hint = if self.state.login_loading {
                        "Connecting with token... Please wait."
                    } else {
                        "[ Press Enter to Connect ]"
                    };

                    self.state.rect_login_submit_btn = Some(Rect {
                        x: start_x,
                        y: cur_y,
                        width: hint.chars().count() as u16,
                        height: 1,
                    });

                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        hint,
                        Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
                    );
                }
                LoginTab::Demo => {
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        "Instant Offline Demo Mode",
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    );
                    cur_y += 2;

                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        "Experience Stoat TUI with preloaded community servers, channels,",
                        Style::default().fg(theme.muted),
                    );
                    cur_y += 1;
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        "full markdown chat, reactions, online members, and quick switcher.",
                        Style::default().fg(theme.muted),
                    );
                    cur_y += 3;

                    let hint = "[ Press Enter to Launch Demo Mode ]";

                    self.state.rect_login_submit_btn = Some(Rect {
                        x: start_x,
                        y: cur_y,
                        width: hint.chars().count() as u16,
                        height: 1,
                    });

                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        hint,
                        Style::default().fg(theme.status_online).add_modifier(Modifier::BOLD),
                    );
                }
            }
        }

        // Error message if any
        if let Some(ref err) = self.state.login_error {
            cur_y += 2;
            let err_label = if err.contains("<html") || err.contains("<!DOCTYPE") || err.contains("<HTML") {
                "⚠ Server returned HTML instead of JSON — check Server API URL (e.g. /api)".to_string()
            } else if err.starts_with('⚠') {
                err.clone()
            } else {
                format!("⚠ {}", err)
            };

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
                    crate::ui::write_str(
                        buf,
                        inner,
                        start_x,
                        cur_y,
                        err_line,
                        Style::default().fg(Color::Rgb(255, 80, 80)).add_modifier(Modifier::BOLD),
                    );
                    cur_y += 1;
                }
            }
        }
    }
}
