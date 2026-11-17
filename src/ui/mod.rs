pub mod components;
pub mod images;
pub mod markdown;
pub mod modals;
pub mod theme;

use crate::state::{AppState, Focus};
use components::call::CallStage;
use components::channels::ChannelsSidebar;
use components::chat::ChatFeed;
use components::header::ContentHeader;
use components::input::MessageInput;
use components::members::MemberRail;
use components::servers::ServersRail;
use components::status_bar::StatusBar;
use modals::command_palette::CommandPaletteModal;
use modals::login::LoginModal;
use modals::screen_picker::ScreenPickerModal;
use modals::settings::SettingsModal;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::Frame;

/// Safely sets a single cell inside `area` (and `buf.area()`) without ever panicking out of bounds.
pub fn put_cell(buf: &mut Buffer, area: Rect, x: u16, y: u16, ch: char, style: Style) {
    if x >= area.left()
        && x < area.right()
        && y >= area.top()
        && y < area.bottom()
        && x >= buf.area().left()
        && x < buf.area().right()
        && y >= buf.area().top()
        && y < buf.area().bottom()
    {
        buf[(x, y)].set_char(ch).set_style(style);
    }
}

/// Safely writes clipped text inside `area` starting at `(x, y)` and returns the next x column.
pub fn write_str(buf: &mut Buffer, area: Rect, mut x: u16, y: u16, text: &str, style: Style) -> u16 {
    use unicode_width::UnicodeWidthChar;

    if y < area.top() || y >= area.bottom() || y < buf.area().top() || y >= buf.area().bottom() {
        return x;
    }
    let limit = area.right().min(buf.area().right());
    for ch in text.chars() {
        let w = ch.width().unwrap_or(1).max(1) as u16;
        if x.saturating_add(w) > limit {
            break;
        }
        if x >= area.left() && x >= buf.area().left() {
            buf[(x, y)].set_char(ch).set_style(style);
            for off in 1..w {
                let nx = x + off;
                if nx < limit && nx >= area.left() && nx >= buf.area().left() {
                    buf[(nx, y)]
                        .set_char(' ')
                        .set_style(style)
                        .set_diff_option(ratatui::buffer::CellDiffOption::Skip);
                }
            }
        }
        x = x.saturating_add(w);
    }
    x
}

fn render_deferred_images(f: &mut Frame, state: &mut AppState) {
    // Clear background fills first so Kitty graphics render cleanly instead of dark rectangles.
    let deferred = std::mem::take(&mut state.deferred_image_renders);
    if let Some(ref mut cache) = state.image_cache {
        for (img_rect, key) in deferred {
            if img_rect.width == 0 || img_rect.height == 0 {
                continue;
            }
            if let Some(protocol) = cache.get_mut(&key) {
                f.render_widget(ratatui::widgets::Clear, img_rect);
                f.render_stateful_widget(
                    ratatui_image::StatefulImage::default(),
                    img_rect,
                    protocol,
                );
                if let Some(Err(err)) = protocol.last_encoding_result() {
                    crate::log_debug!("Image encoding error for {}: {}", key, err);
                }
            }
        }
    }
}

fn render_modals(f: &mut Frame, state: &mut AppState, size: Rect) {
    if size.width == 0 || size.height == 0 {
        return;
    }

    if state.show_command_palette {
        let modal_rect = if size.width < 36 || size.height < 14 {
            size
        } else {
            let modal_w = 64.min(size.width.saturating_sub(4));
            let modal_h = 18.min(size.height.saturating_sub(4));
            Rect {
                x: size.width.saturating_sub(modal_w) / 2,
                y: size.height.saturating_sub(modal_h) / 3,
                width: modal_w,
                height: modal_h,
            }
        };
        if modal_rect.width > 0 && modal_rect.height > 0 {
            state.rect_command_palette = modal_rect;
            f.render_widget(CommandPaletteModal::new(state), modal_rect);
        }
    }

    if state.show_login_modal {
        let modal_rect = if size.width < 36 || size.height < 14 {
            size
        } else {
            let modal_w = 64.min(size.width.saturating_sub(4));
            let modal_h = 22.min(size.height.saturating_sub(4));
            Rect {
                x: size.width.saturating_sub(modal_w) / 2,
                y: size.height.saturating_sub(modal_h) / 2,
                width: modal_w,
                height: modal_h,
            }
        };
        if modal_rect.width > 0 && modal_rect.height > 0 {
            f.render_widget(LoginModal::new(state), modal_rect);
        }
    }

    if state.show_settings_modal {
        let modal_rect = if size.width < 36 || size.height < 14 {
            size
        } else {
            let modal_w = 68.min(size.width.saturating_sub(4));
            let modal_h = (22 + (state.config.accounts.len() as u16) * 2).min(size.height.saturating_sub(2));
            Rect {
                x: size.width.saturating_sub(modal_w) / 2,
                y: size.height.saturating_sub(modal_h) / 2,
                width: modal_w,
                height: modal_h,
            }
        };
        if modal_rect.width > 0 && modal_rect.height > 0 {
            f.render_widget(SettingsModal::new(state), modal_rect);
        }
    }

    if state.show_screen_picker_modal {
        let modal_rect = if size.width < 36 || size.height < 12 {
            size
        } else {
            let modal_w = 66.min(size.width.saturating_sub(4));
            let modal_h = (5 + state.screen_picker_targets.len() as u16)
                .clamp(8, 18)
                .min(size.height.saturating_sub(2));
            Rect {
                x: size.width.saturating_sub(modal_w) / 2,
                y: size.height.saturating_sub(modal_h) / 2,
                width: modal_w,
                height: modal_h,
            }
        };
        if modal_rect.width > 0 && modal_rect.height > 0 {
            f.render_widget(ScreenPickerModal::new(state), modal_rect);
        }
    }
}

fn populate_server_rects(state: &mut AppState, rail_rect: Rect, borderless: bool) {
    state.rect_server_items.clear();
    let y_start = if borderless { rail_rect.top() } else { rail_rect.top() + 1 };
    let y_step = if rail_rect.height < 10 { 1 } else { 2 };
    if y_start < rail_rect.bottom() {
        state.rect_server_items.push((
            None,
            Rect {
                x: rail_rect.left(),
                y: y_start,
                width: rail_rect.width,
                height: 1,
            },
        ));
    }
    let mut sy = if borderless {
        y_start + if rail_rect.height < 10 { 1 } else { 2 }
    } else {
        rail_rect.top() + 4
    };
    for server in &state.servers {
        if sy >= rail_rect.bottom() {
            break;
        }
        state.rect_server_items.push((
            Some(server.id.clone()),
            Rect {
                x: rail_rect.left(),
                y: sy,
                width: rail_rect.width,
                height: 1,
            },
        ));
        sy += y_step;
    }
}

fn populate_channel_rects(state: &mut AppState, channels_rect: Rect) {
    state.rect_channel_items.clear();
    let sidebar_items = ChannelsSidebar::new(state).compute_items();
    let header_lines: u16 = if channels_rect.height < 4 { 0 } else { 2 };
    let mut cy = channels_rect.top() + header_lines;
    for item in sidebar_items {
        match item {
            components::channels::SidebarItem::Category(_) => {
                if channels_rect.height >= 4 {
                    cy += 2;
                }
            }
            components::channels::SidebarItem::Channel { id, .. } => {
                if cy < channels_rect.bottom() {
                    state.rect_channel_items.push((
                        id,
                        Rect {
                            x: channels_rect.left(),
                            y: cy,
                            width: channels_rect.width,
                            height: 1,
                        },
                    ));
                }
                cy += 1;
            }
        }
    }
}

fn render_call_banner(f: &mut Frame, br: Rect) {
    use ratatui::style::{Color, Modifier};
    use ratatui::text::{Line, Span};
    if br.width == 0 || br.height == 0 {
        return;
    }
    let banner_line = Line::from(vec![
        Span::styled(
            " 📞 Active call — ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(35, 120, 65))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Press [c] to join",
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Rgb(35, 120, 65))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        Span::styled(" ", Style::default().bg(Color::Rgb(35, 120, 65))),
    ]);
    f.render_widget(
        ratatui::widgets::Paragraph::new(banner_line)
            .style(Style::default().bg(Color::Rgb(35, 120, 65))),
        br,
    );
}

pub fn render(f: &mut Frame, state: &mut AppState) {
    let size = f.area();
    state.deferred_image_renders.clear();
    if size.width == 0 || size.height == 0 {
        return;
    }

    // If call fullscreen mode is active, render CallStage across the entire terminal area
    if state.call_fullscreen && state.active_call.is_some() {
        f.render_widget(CallStage::new(state), size);
        render_deferred_images(f, state);
        render_modals(f, state, size);
        return;
    }

    // Check if current channel has an active incoming/room call in progress that the user has NOT joined
    let has_unjoined_active_call = if state.active_call.is_none() {
        if let Some(cid) = state.active_channel_id.as_deref() {
            state
                .voice_channel_participants
                .get(cid)
                .map(|v| !v.is_empty())
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    // Breakpoint 1: Tiny / Compact Mode (width < 48 or height < 14) -> Single-pane focused view
    if size.width < 48 || size.height < 14 {
        state.rect_rail = Rect::default();
        state.rect_channels = Rect::default();
        state.rect_chat = Rect::default();
        state.rect_input = Rect::default();
        state.rect_members = Rect::default();
        state.rect_header = Rect::default();
        state.rect_call_banner = None;
        state.rect_member_toggle = Rect::default();
        state.rect_server_items.clear();
        state.rect_channel_items.clear();

        let (header_opt, pane_rect, status_opt) = if size.height < 6 {
            (None, size, None)
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(size);
            (Some(chunks[0]), chunks[1], Some(chunks[2]))
        };

        if let Some(hr) = header_opt {
            state.rect_header = hr;
            f.render_widget(ContentHeader::new(state), hr);
        }

        if pane_rect.width > 0 && pane_rect.height > 0 {
            match state.focus {
                Focus::Servers => {
                    state.rect_rail = pane_rect;
                    populate_server_rects(state, pane_rect, true);
                    f.render_widget(ServersRail::new(state), pane_rect);
                }
                Focus::Channels => {
                    state.rect_channels = pane_rect;
                    populate_channel_rects(state, pane_rect);
                    f.render_widget(ChannelsSidebar::new(state), pane_rect);
                }
                Focus::Chat => {
                    if state.active_call.is_some() {
                        if pane_rect.height >= 4 {
                            let split = Layout::default()
                                .direction(Direction::Vertical)
                                .constraints([
                                    Constraint::Length((pane_rect.height / 2).max(2)),
                                    Constraint::Min(2),
                                ])
                                .split(pane_rect);
                            state.rect_chat = split[1];
                            f.render_widget(CallStage::new(state), split[0]);
                            f.render_widget(ChatFeed::new(state), split[1]);
                        } else {
                            f.render_widget(CallStage::new(state), pane_rect);
                        }
                    } else if has_unjoined_active_call && pane_rect.height >= 4 {
                        let split = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([Constraint::Length(1), Constraint::Min(1)])
                            .split(pane_rect);
                        state.rect_call_banner = Some(split[0]);
                        state.rect_chat = split[1];
                        render_call_banner(f, split[0]);
                        f.render_widget(ChatFeed::new(state), split[1]);
                    } else {
                        state.rect_chat = pane_rect;
                        f.render_widget(ChatFeed::new(state), pane_rect);
                    }
                }
                Focus::Input => {
                    state.rect_input = pane_rect;
                    f.render_widget(MessageInput::new(state), pane_rect);
                }
                Focus::Members => {
                    state.rect_members = pane_rect;
                    f.render_widget(MemberRail::new(state), pane_rect);
                }
            }
        }

        if let Some(sr) = status_opt {
            f.render_widget(StatusBar::new(state), sr);
        }

        render_deferred_images(f, state);
        render_modals(f, state, size);
        return;
    }

    // Breakpoint 2 (48 <= width < 85) & Breakpoint 3 (width >= 85)
    let is_medium = size.width < 85;
    let show_members = if is_medium {
        state.show_member_rail && state.member_rail_explicitly_toggled
    } else {
        state.show_member_rail
    };

    // 1. Root Vertical Split: Main Workspace + Bottom Status Bar
    let root_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(1), // Status Bar
        ])
        .split(size);

    let workspace_area = root_chunks[0];
    let status_bar_area = root_chunks[1];

    // 2. Horizontal Split: Left Rail + Channel Sidebar + Center Chat/Input + Member Rail
    let rail_width: u16 = 7;
    let member_width: u16 = if show_members {
        if is_medium { 16 } else { 22 }
    } else {
        0
    };
    let sidebar_width: u16 = if is_medium {
        // Cap sidebar width to <= width / 3 and keep center >= 60% of workspace width
        let max_non_center = (workspace_area.width * 2 / 5).saturating_sub(rail_width + member_width);
        state
            .left_sidebar_width
            .min(workspace_area.width / 3)
            .min(max_non_center)
            .max(10)
    } else {
        state.left_sidebar_width
    };

    let mut constraints = vec![
        Constraint::Length(rail_width),
        Constraint::Length(sidebar_width),
        Constraint::Min(20), // Main content
    ];

    if show_members {
        constraints.push(Constraint::Length(member_width));
    }

    let horiz_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(workspace_area);

    let rail_rect = horiz_chunks[0];
    let channels_rect = horiz_chunks[1];
    let center_rect = horiz_chunks[2];
    let members_rect = if show_members {
        horiz_chunks[3]
    } else {
        Rect::default()
    };

    // 3. Center Area Split
    let has_call = state.active_call.is_some();
    let input_height = if state.pending_attachments.is_empty() { 3 } else { 4 };
    let center_constraints = if has_call {
        let max_call_h = center_rect.height.saturating_sub(3 + 6 + input_height).max(4);
        let desired_call_h = CallStage::desired_height_for_state(center_rect.width, state).min(max_call_h);
        vec![
            Constraint::Length(3),              // 1. Header
            Constraint::Length(desired_call_h), // 2. Dynamic Auto-Sized CallStage
            Constraint::Min(6),                 // 3. ChatFeed
            Constraint::Length(input_height),   // 4. Input
        ]
    } else if has_unjoined_active_call {
        vec![
            Constraint::Length(3),            // 1. Header
            Constraint::Length(1),            // 2. CallBanner
            Constraint::Min(6),               // 3. ChatFeed
            Constraint::Length(input_height), // 4. Input
        ]
    } else {
        vec![
            Constraint::Length(3),            // 1. Header
            Constraint::Min(6),               // 2. ChatFeed
            Constraint::Length(input_height), // 3. Input
        ]
    };

    let center_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(center_constraints)
        .split(center_rect);

    let (header_rect, call_rect, banner_rect, chat_rect, input_rect) = if has_call {
        (center_chunks[0], Some(center_chunks[1]), None, center_chunks[2], center_chunks[3])
    } else if has_unjoined_active_call {
        (center_chunks[0], None, Some(center_chunks[1]), center_chunks[2], center_chunks[3])
    } else {
        (center_chunks[0], None, None, center_chunks[1], center_chunks[2])
    };

    // Record layout rects in state for mouse hit testing
    state.rect_rail = rail_rect;
    state.rect_channels = channels_rect;
    state.rect_chat = chat_rect;
    state.rect_input = input_rect;
    state.rect_members = members_rect;
    state.rect_header = header_rect;
    state.rect_call_banner = banner_rect;

    state.rect_member_toggle = Rect {
        x: header_rect.right().saturating_sub(18),
        y: header_rect.top(),
        width: 17.min(header_rect.width),
        height: 1.min(header_rect.height),
    };

    populate_server_rects(state, rail_rect, false);
    populate_channel_rects(state, channels_rect);

    // 4. Render components
    f.render_widget(ServersRail::new(state), rail_rect);
    f.render_widget(ChannelsSidebar::new(state), channels_rect);
    f.render_widget(ContentHeader::new(state), header_rect);
    if let Some(br) = banner_rect {
        render_call_banner(f, br);
    }
    if let Some(cr) = call_rect {
        f.render_widget(CallStage::new(state), cr);
    }
    f.render_widget(ChatFeed::new(state), chat_rect);
    f.render_widget(MessageInput::new(state), input_rect);

    if show_members {
        f.render_widget(MemberRail::new(state), members_rect);
    }

    f.render_widget(StatusBar::new(state), status_bar_area);

    // 4b. Deferred Image Pass (Kitty Graphics & Sixel Z-Order Fix)
    render_deferred_images(f, state);

    // 5. Render Modals
    render_modals(f, state, size);
}
