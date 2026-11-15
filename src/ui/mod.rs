pub mod components;
pub mod markdown;
pub mod modals;
pub mod theme;

use crate::state::AppState;
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
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

pub fn render(f: &mut Frame, state: &mut AppState) {
    let size = f.area();

    // If call fullscreen mode is active, render CallStage across the entire terminal area
    if state.call_fullscreen && state.active_call.is_some() {
        f.render_widget(CallStage::new(state), size);
        return;
    }

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
    let sidebar_width: u16 = 22;
    let member_width: u16 = if state.show_member_rail { 22 } else { 0 };

    let mut constraints = vec![
        Constraint::Length(rail_width),
        Constraint::Length(sidebar_width),
        Constraint::Min(30), // Main content
    ];

    if state.show_member_rail {
        constraints.push(Constraint::Length(member_width));
    }

    let horiz_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(workspace_area);

    let rail_rect = horiz_chunks[0];
    let channels_rect = horiz_chunks[1];
    let center_rect = horiz_chunks[2];
    let members_rect = if state.show_member_rail {
        horiz_chunks[3]
    } else {
        Rect::default()
    };

    // Check if current channel has an active incoming/room call in progress that the user has NOT joined
    let current_cid_opt = state.active_channel_id.as_deref();
    let has_unjoined_active_call = if state.active_call.is_none() {
        if let Some(cid) = current_cid_opt {
            state.voice_channel_participants.get(cid).map(|v| !v.is_empty()).unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    // 3. Center Area Split:
    // When state.active_call is Some:
    //   1. Header (Constraint::Length(3))
    //   2. CallStage (Constraint::Percentage(45) / Constraint::Min(10))
    //   3. ChatFeed (Constraint::Min(6))
    //   4. Input (Constraint::Length(3))
    // When an unjoined active call banner is visible:
    //   1. Header (Constraint::Length(3))
    //   2. CallBanner (Constraint::Length(1))
    //   3. ChatFeed (Constraint::Min(6))
    //   4. Input (Constraint::Length(3))
    // Otherwise:
    //   1. Header (Constraint::Length(3))
    //   2. ChatFeed (Constraint::Min(6))
    //   3. Input (Constraint::Length(3))
    let has_call = state.active_call.is_some();
    let center_constraints = if has_call {
        vec![
            Constraint::Length(3),         // 1. Header
            Constraint::Percentage(45),    // 2. CallStage (at least 10)
            Constraint::Min(6),            // 3. ChatFeed
            Constraint::Length(3),         // 4. Input
        ]
    } else if has_unjoined_active_call {
        vec![
            Constraint::Length(3),         // 1. Header
            Constraint::Length(1),         // 2. CallBanner
            Constraint::Min(6),            // 3. ChatFeed
            Constraint::Length(3),         // 4. Input
        ]
    } else {
        vec![
            Constraint::Length(3),         // 1. Header
            Constraint::Min(6),            // 2. ChatFeed
            Constraint::Length(3),         // 3. Input
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

    // Member button toggle hit-test area (top right of header)
    state.rect_member_toggle = Rect {
        x: header_rect.right().saturating_sub(18),
        y: header_rect.top(),
        width: 17,
        height: 1,
    };

    // Record server item positions in rail
    state.rect_server_items.clear();
    // DM button: y = rail_rect.top() + 1
    state.rect_server_items.push((
        None,
        Rect {
            x: rail_rect.left(),
            y: rail_rect.top() + 1,
            width: rail_rect.width,
            height: 1,
        },
    ));
    // Server icons: y = rail_rect.top() + 4 + i * 2
    let mut sy = rail_rect.top() + 4;
    for server in &state.servers {
        state.rect_server_items.push((
            Some(server.id.clone()),
            Rect {
                x: rail_rect.left(),
                y: sy,
                width: rail_rect.width,
                height: 1,
            },
        ));
        sy += 2;
    }

    // Record channel item positions
    state.rect_channel_items.clear();
    let sidebar_items = ChannelsSidebar::new(state).compute_items();
    let mut cy = channels_rect.top() + 2;
    for item in sidebar_items {
        match item {
            components::channels::SidebarItem::Category(_) => {
                cy += 2;
            }
            components::channels::SidebarItem::Channel { id, .. } => {
                state.rect_channel_items.push((
                    id,
                    Rect {
                        x: channels_rect.left(),
                        y: cy,
                        width: channels_rect.width,
                        height: 1,
                    },
                ));
                cy += 1;
            }
        }
    }

    // 4. Render components
    f.render_widget(ServersRail::new(state), rail_rect);
    f.render_widget(ChannelsSidebar::new(state), channels_rect);
    f.render_widget(ContentHeader::new(state), header_rect);
    if let Some(br) = banner_rect {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        let banner_line = Line::from(vec![
            Span::styled(" 📞 Active call in progress — ", Style::default().fg(Color::White).bg(Color::Rgb(35, 120, 65)).add_modifier(Modifier::BOLD)),
            Span::styled("Press [c] to join (or click here)", Style::default().fg(Color::Yellow).bg(Color::Rgb(35, 120, 65)).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
            Span::styled(" ", Style::default().bg(Color::Rgb(35, 120, 65))),
        ]);
        f.render_widget(ratatui::widgets::Paragraph::new(banner_line).style(Style::default().bg(Color::Rgb(35, 120, 65))), br);
    }
    if let Some(cr) = call_rect {
        f.render_widget(CallStage::new(state), cr);
    }
    f.render_widget(ChatFeed::new(state), chat_rect);
    f.render_widget(MessageInput::new(state), input_rect);

    if state.show_member_rail {
        f.render_widget(MemberRail::new(state), members_rect);
    }

    f.render_widget(StatusBar::new(state), status_bar_area);

    // 5. Render Modals
    if state.show_command_palette {
        let modal_w = 64.min(size.width.saturating_sub(4));
        let modal_h = 18.min(size.height.saturating_sub(4));
        let modal_rect = Rect {
            x: size.width.saturating_sub(modal_w) / 2,
            y: size.height.saturating_sub(modal_h) / 3,
            width: modal_w,
            height: modal_h,
        };
        state.rect_command_palette = modal_rect;
        f.render_widget(CommandPaletteModal::new(state), modal_rect);
    }

    if state.show_login_modal {
        let modal_w = 64.min(size.width.saturating_sub(4));
        let modal_h = 22.min(size.height.saturating_sub(4));
        let modal_rect = Rect {
            x: size.width.saturating_sub(modal_w) / 2,
            y: size.height.saturating_sub(modal_h) / 2,
            width: modal_w,
            height: modal_h,
        };
        f.render_widget(LoginModal::new(state), modal_rect);
    }
}
