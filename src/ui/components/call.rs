use crate::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

pub struct CallStage<'a> {
    state: &'a mut AppState,
}

impl<'a> CallStage<'a> {
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }

    pub fn desired_height(area_width: u16, call_state: &crate::voice::ActiveCallState) -> u16 {
        let mut seen = std::collections::HashSet::new();
        for p in &call_state.participants {
            let nid = crate::voice::normalize_livekit_identity(&p.user_id);
            if !nid.is_empty() && nid != "caller" {
                seen.insert(nid);
            }
        }
        let participant_count = 1 + seen.len();
        let has_video = call_state.is_screensharing()
            || call_state
                .latest_video_frame
                .try_lock()
                .ok()
                .and_then(|g| {
                    g.as_ref()
                        .map(|s| s.is_remote || call_state.show_local_preview)
                })
                .unwrap_or(false);
        Self::desired_height_for_count(area_width, participant_count, has_video)
    }

    pub fn desired_height_for_state(area_width: u16, state: &AppState) -> u16 {
        let Some(ref call) = state.active_call else {
            return 0;
        };
        let mut seen = std::collections::HashSet::new();
        let acc_id = if !call.account_id.is_empty() {
            call.account_id.as_str()
        } else {
            state.account_for_channel(&call.channel_id)
        };
        let cur_uid = if !call.local_user_id.is_empty() {
            Some(call.local_user_id.as_str())
        } else {
            state.current_user_for_account(acc_id).map(|u| u.id.as_str())
        };
        for p in &call.participants {
            if p.is_local {
                continue;
            }
            let nid = crate::voice::normalize_livekit_identity(&p.user_id);
            if !nid.is_empty() && nid != "caller" && Some(nid.as_str()) != cur_uid {
                seen.insert(nid);
            }
        }
        if let Some(gw) = state.voice_channel_participants.get(&call.channel_id) {
            for p in gw {
                if p.is_local {
                    continue;
                }
                let nid = crate::voice::normalize_livekit_identity(&p.user_id);
                if !nid.is_empty() && nid != "caller" && Some(nid.as_str()) != cur_uid {
                    seen.insert(nid);
                }
            }
        }
        let has_video = call.is_screensharing()
            || call
                .latest_video_frame
                .try_lock()
                .ok()
                .and_then(|g| g.as_ref().map(|s| s.is_remote || call.show_local_preview))
                .unwrap_or(false);
        Self::desired_height_for_count(area_width, 1 + seen.len(), has_video)
    }

    fn desired_height_for_count(area_width: u16, participant_count: usize, has_active_video: bool) -> u16 {
        if has_active_video {
            return 16;
        }
        let cols = ((area_width.saturating_sub(2)) / 14).max(1) as usize;
        let rows = participant_count.max(1).div_ceil(cols) as u16;
        2 + 1 + (rows * 4)
    }
}

impl<'a> Widget for CallStage<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let (
            channel_id,
            call_account_id,
            local_user_id,
            status,
            duration,
            is_muted,
            is_deafened,
            is_screensharing,
            show_local_preview,
            local_speaking,
            participants,
            video_snap,
        ) = match &self.state.active_call {
            Some(call) => {
                let dur = if call.status == crate::voice::CallStatus::Connecting {
                    "Connecting...".to_string()
                } else {
                    call.formatted_duration()
                };
                let snap = call
                    .latest_video_frame
                    .try_lock()
                    .ok()
                    .and_then(|g| g.clone());
                let acc_id = if !call.account_id.is_empty() {
                    call.account_id.clone()
                } else {
                    self.state.account_for_channel(&call.channel_id).to_string()
                };
                (
                    call.channel_id.clone(),
                    acc_id,
                    call.local_user_id.clone(),
                    call.status.clone(),
                    dur,
                    call.is_muted(),
                    call.is_deafened(),
                    call.is_screensharing(),
                    call.show_local_preview,
                    call.local_speaking,
                    call.participants.clone(),
                    snap,
                )
            }
            None => return,
        };
        let _ = status;

        let borderless = area.width < 24 || area.height < 6;

        // Fill subtle call stage background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_char(' ').set_bg(Color::Rgb(20, 24, 34));
            }
        }

        let inner = if borderless {
            area
        } else {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(70, 88, 124)))
                .title(Span::styled(
                    " 📞 Live Voice & Video Stage ",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ));
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        };

        if inner.width < 2 || inner.height < 1 {
            return;
        }

        // Determine if an active video stream (remote or local preview) should be shown
        let active_video = video_snap.filter(|s| s.is_remote || show_local_preview);

        // Compact tiny call stage (e.g. inside 10x10 split)
        if borderless {
            let mic_tag = if is_muted { "🔇" } else { "🎙" };
            let scr_tag = if active_video.is_some() || is_screensharing { " 🖥" } else { "" };
            let short_dur = if duration.starts_with("Conn") { "Conn" } else { &duration };
            let header = format!("📞{}{} {}", short_dur, scr_tag, mic_tag);
            crate::ui::write_str(
                buf,
                inner,
                inner.left(),
                inner.top(),
                &header,
                Style::default().fg(Color::Cyan).bg(Color::Rgb(30, 38, 54)).add_modifier(Modifier::BOLD),
            );
            let body_area = Rect {
                x: inner.left(),
                y: inner.top() + 1,
                width: inner.width,
                height: inner.height.saturating_sub(1),
            };
            if body_area.width == 0 || body_area.height == 0 {
                return;
            }
            if active_video.is_some()
                && body_area.width >= 2
                && self.state.image_cache.as_ref().map(|c| c.contains("active_call_video")).unwrap_or(false)
            {
                self.state
                    .deferred_image_renders
                    .push((body_area, "active_call_video".to_string()));
                return;
            }
            let self_name = self.state.current_user.as_ref().map(|u| u.name()).unwrap_or("You");
            let mut py = body_area.top();
            crate::ui::write_str(
                buf,
                body_area,
                body_area.left(),
                py,
                &format!("{} {}", if local_speaking { "🗣" } else { "•" }, self_name),
                Style::default().fg(Color::White),
            );
            py += 1;
            let mut seen_tiny = std::collections::HashSet::new();
            if let Some(ref cur) = self.state.current_user {
                seen_tiny.insert(cur.id.clone());
            }
            for p in &participants {
                if py >= body_area.bottom() {
                    break;
                }
                let nid = crate::voice::normalize_livekit_identity(&p.user_id);
                if nid.is_empty() || nid == "caller" || !seen_tiny.insert(nid.clone()) {
                    continue;
                }
                let _ = self.state.resolve_user(&nid);
                let uname = self.state.resolve_call_participant_name(&nid);
                crate::ui::write_str(buf, body_area, body_area.left(), py, &format!("• {}", uname), Style::default().fg(Color::Gray));
                py += 1;
            }
            return;
        }

        let mut status_spans = vec![
            Span::styled(
                format!(" [ ⏱ {} ] ", duration),
                Style::default().fg(Color::White).bg(Color::Rgb(38, 42, 56)),
            ),
            Span::raw(" "),
        ];

        if is_muted {
            status_spans.push(Span::styled(
                " [ 🔇 MUTED ] ",
                Style::default().fg(Color::White).bg(Color::Rgb(175, 45, 50)),
            ));
        } else {
            status_spans.push(Span::styled(
                " [ 🎙 MIC ] ",
                Style::default().fg(Color::White).bg(Color::Rgb(38, 136, 68)),
            ));
        }
        status_spans.push(Span::raw(" "));

        if is_deafened {
            status_spans.push(Span::styled(
                " [ 🔇 DEAF ] ",
                Style::default().fg(Color::White).bg(Color::Rgb(175, 45, 50)),
            ));
        } else {
            status_spans.push(Span::styled(
                " [ 🎧 AUDIO ] ",
                Style::default().fg(Color::White).bg(Color::Rgb(38, 136, 68)),
            ));
        }
        status_spans.push(Span::raw(" "));

        let mut screen_spans = Vec::new();
        if let Some(ref snap) = active_video {
            let owner_name = if !snap.is_remote {
                self.state
                    .current_user
                    .as_ref()
                    .map(|u| u.name().to_string())
                    .unwrap_or_else(|| "You".to_string())
            } else {
                let _ = self.state.resolve_user(&snap.owner_id);
                self.state.resolve_call_participant_name(&snap.owner_id)
            };
            screen_spans.push(Span::styled(
                format!(" [ 🖥 STREAMING • {}fps • {} ] ", snap.fps, owner_name),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(255, 180, 40))
                    .add_modifier(Modifier::BOLD),
            ));
            if is_screensharing && !snap.is_remote {
                screen_spans.push(Span::raw(" "));
                screen_spans.push(Span::styled(
                    " [ 👁 PREVIEW ] ",
                    Style::default().fg(Color::White).bg(Color::Rgb(52, 56, 72)),
                ));
            }
        } else if is_screensharing {
            screen_spans.push(Span::styled(
                if show_local_preview {
                    " [ 🖥 STARTING... ] "
                } else {
                    " [ 🖥 SHARING ] "
                },
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(255, 180, 40))
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            screen_spans.push(Span::styled(
                " [ 🖥 SHARE ] ",
                Style::default()
                    .fg(Color::Rgb(165, 170, 185))
                    .bg(Color::Rgb(38, 40, 50)),
            ));
        }
        if self.state.call_fullscreen {
            screen_spans.push(Span::raw(" "));
            screen_spans.push(Span::styled(
                " [ ⛶ FULL ] ",
                Style::default().fg(Color::White).bg(Color::Rgb(60, 74, 110)),
            ));
        }

        let mut combined_spans = status_spans.clone();
        combined_spans.extend(screen_spans.iter().cloned());
        let combined_line = Line::from(combined_spans);

        // When video is active (expanded 16+ row stage), allow 2 header rows so the full streaming badge isn't clipped;
        // when voice-only (7-row compact stage), keep header strictly 1 row and dynamically drop text to icon-only pills when narrow.
        let header_rows: u16 = if area.width >= 68 && combined_line.width() <= inner.width as usize {
            buf.set_line(inner.left(), inner.top(), &combined_line, inner.width);
            1
        } else if active_video.is_some() && inner.height >= 3 {
            let row1 = Line::from(status_spans);
            let row2 = Line::from(screen_spans);
            buf.set_line(inner.left(), inner.top(), &row1, inner.width);
            buf.set_line(inner.left(), inner.top() + 1, &row2, inner.width);
            2
        } else {
            let short_dur = if duration.starts_with("Conn") { "Conn" } else { &duration };
            let mut icon_spans = vec![
                Span::styled(
                    format!(" [ ⏱ {} ]", short_dur),
                    Style::default().fg(Color::White).bg(Color::Rgb(38, 42, 56)),
                ),
                Span::raw(" "),
                Span::styled(
                    if is_muted { "[ 🔇 ]" } else { "[ 🎙 ]" },
                    Style::default().fg(Color::White).bg(if is_muted { Color::Rgb(175, 45, 50) } else { Color::Rgb(38, 136, 68) }),
                ),
                Span::raw(" "),
                Span::styled(
                    if is_deafened { "[ 🔇 ]" } else { "[ 🎧 ]" },
                    Style::default().fg(Color::White).bg(if is_deafened { Color::Rgb(175, 45, 50) } else { Color::Rgb(38, 136, 68) }),
                ),
                Span::raw(" "),
            ];
            if let Some(ref snap) = active_video {
                icon_spans.push(Span::styled(
                    format!("[ 🖥 STREAMING • {}fps • {} ]", snap.fps, self.state.resolve_call_participant_name(&snap.owner_id)),
                    Style::default().fg(Color::Black).bg(Color::Rgb(255, 180, 40)).add_modifier(Modifier::BOLD),
                ));
            } else if is_screensharing {
                icon_spans.push(Span::styled(
                    "[ 🖥 ]",
                    Style::default().fg(Color::Black).bg(Color::Rgb(255, 180, 40)).add_modifier(Modifier::BOLD),
                ));
            } else {
                icon_spans.push(Span::styled(
                    "[ 🖥 ]",
                    Style::default().fg(Color::Rgb(165, 170, 185)).bg(Color::Rgb(38, 40, 50)),
                ));
            }
            if is_screensharing && active_video.is_some() {
                icon_spans.push(Span::raw(" "));
                icon_spans.push(Span::styled(
                    "[ 👁 ]",
                    Style::default().fg(Color::White).bg(Color::Rgb(52, 56, 72)),
                ));
            }
            if self.state.call_fullscreen {
                icon_spans.push(Span::raw(" "));
                icon_spans.push(Span::styled(
                    "[ ⛶ ]",
                    Style::default().fg(Color::White).bg(Color::Rgb(60, 74, 110)),
                ));
            }
            let icon_line = Line::from(icon_spans);
            buf.set_line(inner.left(), inner.top(), &icon_line, inner.width);
            1
        };

        // Stage body: If active video stream is visible, render via ratatui-image deferred pass
        // with 1-row participant strip underneath; otherwise render compact participant cards.
        let body_area = Rect {
            x: inner.left(),
            y: inner.top() + header_rows,
            width: inner.width,
            height: inner.height.saturating_sub(header_rows),
        };

        if body_area.height == 0 {
            return;
        }

        if active_video.is_some() {
            let has_cached = self
                .state
                .image_cache
                .as_ref()
                .map(|c| c.contains("active_call_video"))
                .unwrap_or(false);
            if has_cached {
                let (video_rect, strip_y_opt) = if body_area.height >= 4 {
                    (
                        Rect {
                            x: body_area.left(),
                            y: body_area.top(),
                            width: body_area.width,
                            height: body_area.height.saturating_sub(1),
                        },
                        Some(body_area.bottom().saturating_sub(1)),
                    )
                } else {
                    (body_area, None)
                };
                self.state
                    .deferred_image_renders
                    .push((video_rect, "active_call_video".to_string()));
                if let Some(strip_y) = strip_y_opt {
                    let mut sx = body_area.left();
                    let self_tag = self
                        .state
                        .current_user
                        .as_ref()
                        .map(|u| format!("{} (You)", u.name()))
                        .unwrap_or_else(|| "You".to_string());
                    let self_style = if local_speaking {
                        Style::default().fg(Color::Rgb(35, 165, 90)).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    sx = crate::ui::write_str(buf, body_area, sx, strip_y, &format!("[{}]", self_tag), self_style);
                    let cur_uid = self.state.current_user.as_ref().map(|u| u.id.as_str());
                    for p in &participants {
                        let nid = crate::voice::normalize_livekit_identity(&p.user_id);
                        if nid.is_empty() || nid == "caller" || Some(nid.as_str()) == cur_uid {
                            continue;
                        }
                        let name = self.state.resolve_call_participant_name(&nid);
                        let p_style = if p.is_speaking {
                            Style::default().fg(Color::Rgb(35, 165, 90)).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Rgb(190, 190, 205))
                        };
                        sx = crate::ui::write_str(buf, body_area, sx, strip_y, &format!("  [{}]", name), p_style);
                    }
                }
                return;
            }
        }

        // Render participant avatar cards horizontally (wrapping into rows on narrow terminals)
        struct CardParticipant {
            user_id: String,
            display_name: String,
            is_self: bool,
            is_speaking: bool,
            is_muted: bool,
            is_deafened: bool,
        }

        let mut seen_ids = std::collections::HashSet::new();
        let mut cards: Vec<CardParticipant> = Vec::new();

        let effective_local_uid = if !local_user_id.is_empty() {
            local_user_id.clone()
        } else {
            self.state
                .current_user_for_account(&call_account_id)
                .map(|u| u.id.clone())
                .unwrap_or_default()
        };

        if !effective_local_uid.is_empty() {
            seen_ids.insert(effective_local_uid.clone());
            let _ = self
                .state
                .resolve_user_in_account(&call_account_id, &effective_local_uid);
            let local_name = self
                .state
                .current_user_for_account(&call_account_id)
                .filter(|u| u.id == effective_local_uid)
                .map(|u| u.name().to_string())
                .unwrap_or_else(|| self.state.resolve_call_participant_name(&effective_local_uid));
            cards.push(CardParticipant {
                user_id: effective_local_uid,
                display_name: local_name,
                is_self: true,
                is_speaking: local_speaking,
                is_muted,
                is_deafened,
            });
        } else {
            cards.push(CardParticipant {
                user_id: String::new(),
                display_name: "You".to_string(),
                is_self: true,
                is_speaking: local_speaking,
                is_muted,
                is_deafened,
            });
        }

        let mut merged_remote = participants;
        if let Some(gw_participants) = self.state.voice_channel_participants.get(&channel_id) {
            for gp in gw_participants {
                if !merged_remote.iter().any(|p| p.user_id == gp.user_id) {
                    merged_remote.push(gp.clone());
                }
            }
        }

        for p in merged_remote {
            if p.is_local {
                continue;
            }
            let norm_id = crate::voice::normalize_livekit_identity(&p.user_id);
            if norm_id.is_empty() || norm_id == "caller" || !seen_ids.insert(norm_id.clone()) {
                continue;
            }
            let p_acc = if !p.account_id.is_empty() {
                p.account_id.as_str()
            } else {
                call_account_id.as_str()
            };
            let _ = self.state.resolve_user_in_account(p_acc, &norm_id);
            let name = self.state.resolve_call_participant_name(&norm_id);
            cards.push(CardParticipant {
                user_id: norm_id,
                display_name: name,
                is_self: false,
                is_speaking: p.is_speaking,
                is_muted: p.is_muted,
                is_deafened: p.is_deafened,
            });
        }
        let _ = is_screensharing;

        // Card sizing: 16x5 (4x2 PFP) when height >= 5, 16x4 or 14x4 compact otherwise
        let approx_cols = ((body_area.width) / 14).max(1) as usize;
        let approx_rows = ((cards.len() + approx_cols - 1) / approx_cols).max(1) as u16;

        let (card_w, card_h, pfp_w, pfp_h): (u16, u16, u16, u16) =
            if body_area.height >= 5 * approx_rows && body_area.width >= 16 {
                (16, 5, 6, 2)
            } else if body_area.height >= 4 && body_area.width >= 16 {
                (16, 4, 4, 1)
            } else if body_area.height >= 4 && body_area.width >= 14 {
                (14, 4, 4, 1)
            } else if body_area.height >= 4 && body_area.width >= 12 {
                (12, 4, 4, 1)
            } else {
                // Ultra-tiny fallback (e.g. 10x3)
                let mut py = body_area.top();
                for card in cards {
                    if py >= body_area.bottom() {
                        break;
                    }
                    let prefix = if card.is_speaking { "● " } else { "  " };
                    let suffix = if card.is_deafened {
                        " [🔇🎧]"
                    } else if card.is_muted {
                        " [🔇]"
                    } else {
                        ""
                    };
                    let style = if card.is_speaking {
                        Style::default().fg(Color::Rgb(35, 165, 90)).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    crate::ui::write_str(
                        buf,
                        body_area,
                        body_area.left() + 1,
                        py,
                        &format!("{}{}{}", prefix, card.display_name, suffix),
                        style,
                    );
                    py += 1;
                }
                return;
            };

        let gap_x: u16 = if body_area.width >= card_w * 2 + 4 {
            2
        } else if body_area.width >= card_w * 2 + 2 {
            1
        } else {
            0
        };
        let max_per_row = ((body_area.width + gap_x) / (card_w + gap_x).max(1)).max(1) as usize;
        let row_cards_count = cards.len().min(max_per_row).max(1) as u16;
        let row_total_w = row_cards_count * card_w + row_cards_count.saturating_sub(1) * gap_x;
        let start_x = body_area.left() + body_area.width.saturating_sub(row_total_w) / 2;
        let rows_needed = ((cards.len() + max_per_row - 1) / max_per_row).max(1) as u16;
        let gap_y: u16 = if body_area.height >= rows_needed * card_h + rows_needed.saturating_sub(1) {
            1
        } else {
            0
        };
        let mut cur_x = start_x;
        let mut cur_y = body_area.top();

        for card in cards {
            if cur_x + card_w > body_area.right() && cur_x > start_x {
                cur_x = start_x;
                cur_y = cur_y.saturating_add(card_h + gap_y);
            }
            if cur_y + card_h > body_area.bottom() || cur_x + card_w > body_area.right() {
                break;
            }

            let card_rect = Rect {
                x: cur_x,
                y: cur_y,
                width: card_w,
                height: card_h,
            };

            let card_bg = Color::Rgb(30, 31, 34);
            for cy in card_rect.top()..card_rect.bottom() {
                for cx in card_rect.left()..card_rect.right() {
                    crate::ui::put_cell(buf, card_rect, cx, cy, ' ', Style::default().bg(card_bg));
                }
            }

            // Rounded green border when speaking; subtle dark card fill when silent
            if card.is_speaking {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(
                        Style::default()
                            .fg(Color::Rgb(35, 165, 90))
                            .add_modifier(Modifier::BOLD),
                    )
                    .style(Style::default().bg(card_bg));
                block.render(card_rect, buf);
            }

            let card_inner = Rect {
                x: card_rect.left() + 1,
                y: card_rect.top() + 1,
                width: card_rect.width.saturating_sub(2),
                height: card_rect.height.saturating_sub(2),
            };

            if card_inner.width >= pfp_w && card_inner.height >= pfp_h + 1 {
                let pfp_rect = Rect {
                    x: card_inner.left() + (card_inner.width.saturating_sub(pfp_w)) / 2,
                    y: card_inner.top(),
                    width: pfp_w,
                    height: pfp_h,
                };

                let (badge_bg, badge_fg) = if !card.user_id.is_empty() {
                    self.state.theme.user_initials_color(&card.user_id)
                } else {
                    (Color::Rgb(60, 80, 120), Color::White)
                };
                for py in pfp_rect.top()..pfp_rect.bottom() {
                    for px in pfp_rect.left()..pfp_rect.right() {
                        crate::ui::put_cell(buf, pfp_rect, px, py, ' ', Style::default().bg(badge_bg));
                    }
                }

                let initials: String = card
                    .display_name
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .take(2)
                    .collect::<String>()
                    .to_uppercase();
                let mut init_chars = initials.chars();
                let c1 = init_chars.next().unwrap_or('?');
                let c2 = init_chars.next().unwrap_or(' ');
                let mid_x = pfp_rect.left() + (pfp_rect.width / 2).saturating_sub(1);
                let mid_y = pfp_rect.top() + (pfp_rect.height / 2);
                crate::ui::put_cell(
                    buf,
                    pfp_rect,
                    mid_x,
                    mid_y,
                    c1,
                    Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD),
                );
                crate::ui::put_cell(
                    buf,
                    pfp_rect,
                    mid_x + 1,
                    mid_y,
                    c2,
                    Style::default().fg(badge_fg).bg(badge_bg).add_modifier(Modifier::BOLD),
                );

                if !card.user_id.is_empty() {
                    let key = self.state.avatar_cache_key(&call_account_id, &card.user_id, "avatar_card");
                    self.state.deferred_image_renders.push((pfp_rect, key.clone()));
                    self.state
                        .request_user_avatar_with_key_in_account(&call_account_id, &card.user_id, &key);
                }
            }

            // Bottom label row inside card: truncated username + muted/deafened icon ONLY when muted/deafened
            let label_y = card_inner.bottom().saturating_sub(1);
            let status_badge = if card.is_deafened {
                " 🎧"
            } else if card.is_muted {
                " 🔇"
            } else {
                ""
            };
            let badge_len = unicode_width::UnicodeWidthStr::width(status_badge) as u16;
            let max_name_cols = card_inner.width.saturating_sub(badge_len) as usize;
            let trunc_name: String = card.display_name.chars().take(max_name_cols).collect();
            let total_w = (trunc_name.chars().count() as u16).saturating_add(badge_len);
            let label_x = card_inner.left() + card_inner.width.saturating_sub(total_w) / 2;

            let name_style = Style::default()
                .fg(if card.is_self || card.is_speaking { Color::White } else { Color::Rgb(215, 215, 225) })
                .add_modifier(if card.is_speaking { Modifier::BOLD } else { Modifier::empty() });
            let next_x = crate::ui::write_str(buf, card_inner, label_x, label_y, &trunc_name, name_style);
            if !status_badge.is_empty() {
                crate::ui::write_str(
                    buf,
                    card_inner,
                    next_x,
                    label_y,
                    status_badge,
                    Style::default().fg(Color::Rgb(240, 80, 80)).add_modifier(Modifier::BOLD),
                );
            }

            cur_x = cur_x.saturating_add(card_w + gap_x);
        }
    }
}
