use crate::api::models::Message;
use crate::api::DeltaClient;
use crate::config::Config;
use crate::event::AppEvent;
use crate::gateway::messages::ServerMessage;
use crate::gateway::{BonfireClient, GatewayEvent};
use crate::state::{AppState, Focus, GatewayStatus, LoginTab, PaletteItemType};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub struct App {
    pub state: AppState,
    pub api_client: Arc<Mutex<DeltaClient>>,
    pub event_tx: mpsc::Sender<AppEvent>,
    pub gateway_handle: Option<tokio::task::JoinHandle<()>>,
}

impl App {
    pub fn new(config: Config, event_tx: mpsc::Sender<AppEvent>) -> Self {
        let api_client = Arc::new(Mutex::new(DeltaClient::new(
            config.api_url.clone(),
            config.token.clone(),
            config.is_bot,
        )));

        let state = AppState::new(config);

        Self {
            state,
            api_client,
            event_tx,
            gateway_handle: None,
        }
    }

    pub fn start_gateway(&mut self, token: String, gateway_url: Option<String>) {
        if let Some(h) = self.gateway_handle.take() {
            h.abort();
        }

        let event_tx = self.event_tx.clone();
        let api_client = self.api_client.clone();

        let gw_url = gateway_url.unwrap_or_else(|| "wss://ws.stoat.chat".to_string());

        // Spawn a gateway runner that first checks metadata if needed
        let handle = tokio::spawn(async move {
            let (gw_tx, mut gw_rx) = mpsc::channel(128);

            // Fetch gateway URL from root metadata if not explicitly configured
            let effective_gw_url = {
                let client = api_client.lock().await;
                if let Ok(meta) = client.fetch_metadata().await {
                    meta.ws.unwrap_or(gw_url)
                } else {
                    gw_url
                }
            };

            let client = BonfireClient::new(effective_gw_url, token, gw_tx);
            let client_handle = client.start();

            while let Some(gw_event) = gw_rx.recv().await {
                if event_tx.send(AppEvent::Gateway(gw_event)).await.is_err() {
                    break;
                }
            }

            client_handle.abort();
        });

        self.gateway_handle = Some(handle);
    }

    pub async fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.handle_key(key).await,
            AppEvent::Mouse(mouse) => self.handle_mouse(mouse).await,
            AppEvent::Paste(text) => self.handle_paste(text).await,
            AppEvent::Resize(_, _) => {}
            AppEvent::Tick => {}
            AppEvent::Gateway(gw) => self.handle_gateway(gw).await,
            AppEvent::ApiLoginSuccess { token, is_bot } => {
                self.state.login_loading = false;
                self.state.login_error = None;
                self.state.show_login_modal = false;
                self.state.config.token = Some(token.clone());
                self.state.config.is_bot = is_bot;
                let _ = self.state.config.save();

                {
                    let mut api = self.api_client.lock().await;
                    api.set_token(token.clone(), is_bot);
                }

                self.start_gateway(token, self.state.config.gateway_url.clone());
                self.fetch_initial_data().await;
            }
            AppEvent::ApiLoginFailure(err) => {
                self.state.login_loading = false;
                self.state.login_error = Some(err);
            }
            AppEvent::ApiMessageSent(res) => match res {
                Ok(msg) => {
                    self.state.messages.entry(msg.channel.clone()).or_default().push(msg);
                }
                Err(err) => {
                    self.state.set_status(format!("Error sending message: {}", err));
                }
            },
            AppEvent::ApiMessagesFetched { channel_id, messages } => {
                self.state.messages.insert(channel_id, messages);
            }
            AppEvent::ApiCurrentUser(user) => {
                self.state.current_user = Some(user);
            }
            AppEvent::ApiDmsFetched(dms) => {
                self.state.dms = dms;
            }
        }
    }

    async fn fetch_initial_data(&mut self) {
        let api = self.api_client.clone();
        let tx = self.event_tx.clone();

        tokio::spawn(async move {
            let client = api.lock().await;
            if let Ok(user) = client.get_current_user().await {
                let _ = tx.send(AppEvent::ApiCurrentUser(user)).await;
            }
            if let Ok(dms) = client.fetch_dms().await {
                let _ = tx.send(AppEvent::ApiDmsFetched(dms)).await;
            }
        });
    }

    async fn handle_paste(&mut self, text: String) {
        if self.state.show_command_palette {
            self.state.command_palette_query.push_str(&text);
            self.state.command_palette_selected = 0;
            return;
        }

        if self.state.show_login_modal {
            match self.state.login_tab {
                LoginTab::EmailPassword => {
                    if self.state.login_field_index == 0 {
                        self.state.login_email_input.push_str(&text);
                    } else {
                        self.state.login_password_input.push_str(&text);
                    }
                }
                LoginTab::Token => {
                    self.state.login_token_input.push_str(&text);
                }
                LoginTab::Demo => {}
            }
            return;
        }

        if self.state.focus == Focus::Input {
            self.state.input_insert_str(&text);
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        // Global quit
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.state.should_quit = true;
            return;
        }

        // 1. If Command Palette Modal is open
        if self.state.show_command_palette {
            match key.code {
                KeyCode::Esc => {
                    self.state.show_command_palette = false;
                }
                KeyCode::Up => {
                    if self.state.command_palette_selected > 0 {
                        self.state.command_palette_selected -= 1;
                    }
                }
                KeyCode::Down => {
                    let count = self.state.get_palette_items().len();
                    if self.state.command_palette_selected + 1 < count {
                        self.state.command_palette_selected += 1;
                    }
                }
                KeyCode::Enter => {
                    let items = self.state.get_palette_items();
                    if let Some(item) = items.get(self.state.command_palette_selected) {
                        match item.item_type {
                            PaletteItemType::Server => {
                                self.state.select_server(Some(item.id.clone()));
                            }
                            PaletteItemType::Channel => {
                                if let Some(ref sid) = item.server_id {
                                    self.state.select_server(Some(sid.clone()));
                                }
                                self.state.select_channel(item.id.clone());
                            }
                            PaletteItemType::DirectMessage => {
                                self.state.select_server(None);
                                self.state.select_channel(item.id.clone());
                            }
                        }
                    }
                    self.state.show_command_palette = false;
                }
                KeyCode::Backspace => {
                    self.state.command_palette_query.pop();
                    self.state.command_palette_selected = 0;
                }
                KeyCode::Char(c) => {
                    self.state.command_palette_query.push(c);
                    self.state.command_palette_selected = 0;
                }
                _ => {}
            }
            return;
        }

        // 2. If Login Modal is open
        if self.state.show_login_modal {
            match key.code {
                KeyCode::Esc => {
                    self.state.show_login_modal = false;
                }
                KeyCode::Tab => {
                    if self.state.login_tab == LoginTab::EmailPassword {
                        self.state.login_field_index = (self.state.login_field_index + 1) % 2;
                    } else {
                        self.state.login_tab = match self.state.login_tab {
                            LoginTab::EmailPassword => LoginTab::Token,
                            LoginTab::Token => LoginTab::Demo,
                            LoginTab::Demo => LoginTab::EmailPassword,
                        };
                    }
                }
                KeyCode::BackTab => {
                    if self.state.login_tab == LoginTab::EmailPassword {
                        self.state.login_field_index = if self.state.login_field_index == 0 { 1 } else { 0 };
                    }
                }
                KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.state.login_tab = LoginTab::EmailPassword;
                }
                KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.state.login_tab = LoginTab::Token;
                }
                KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.state.login_tab = LoginTab::Demo;
                }
                KeyCode::Enter => {
                    match self.state.login_tab {
                        LoginTab::EmailPassword => {
                            let email = self.state.login_email_input.trim().to_string();
                            let password = self.state.login_password_input.clone();
                            if email.is_empty() || password.is_empty() {
                                self.state.login_error = Some("Email and password are required".to_string());
                                return;
                            }
                            self.state.login_loading = true;
                            self.state.login_error = None;

                            let api = self.api_client.clone();
                            let tx = self.event_tx.clone();

                            tokio::spawn(async move {
                                let client = api.lock().await;
                                match client.login(&email, &password).await {
                                    Ok(resp) => {
                                        let _ = tx.send(AppEvent::ApiLoginSuccess {
                                            token: resp.token,
                                            is_bot: false,
                                        }).await;
                                    }
                                    Err(e) => {
                                        let _ = tx.send(AppEvent::ApiLoginFailure(e.to_string())).await;
                                    }
                                }
                            });
                        }
                        LoginTab::Token => {
                            let token = self.state.login_token_input.trim().to_string();
                            if !token.is_empty() {
                                let _ = self.event_tx.send(AppEvent::ApiLoginSuccess {
                                    token,
                                    is_bot: false,
                                }).await;
                            }
                        }
                        LoginTab::Demo => {
                            self.state.load_demo_data();
                            self.state.show_login_modal = false;
                        }
                    }
                }
                KeyCode::Backspace => match self.state.login_tab {
                    LoginTab::EmailPassword => {
                        if self.state.login_field_index == 0 {
                            self.state.login_email_input.pop();
                        } else {
                            self.state.login_password_input.pop();
                        }
                    }
                    LoginTab::Token => {
                        self.state.login_token_input.pop();
                    }
                    LoginTab::Demo => {}
                },
                KeyCode::Char(c) => match self.state.login_tab {
                    LoginTab::EmailPassword => {
                        if self.state.login_field_index == 0 {
                            self.state.login_email_input.push(c);
                        } else {
                            self.state.login_password_input.push(c);
                        }
                    }
                    LoginTab::Token => {
                        self.state.login_token_input.push(c);
                    }
                    LoginTab::Demo => {}
                },
                _ => {}
            }
            return;
        }

        // Global Shortcuts:
        // Command Palette
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            self.state.show_command_palette = true;
            self.state.command_palette_query.clear();
            self.state.command_palette_selected = 0;
            return;
        }

        // Member Rail toggle (F4)
        if key.code == KeyCode::F(4) {
            self.state.show_member_rail = !self.state.show_member_rail;
            if !self.state.show_member_rail && self.state.focus == Focus::Members {
                self.state.focus = Focus::Chat;
            }
            return;
        }

        // Direct Pane Navigation (Alt+1..4)
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('1') => {
                    self.state.focus = Focus::Servers;
                    return;
                }
                KeyCode::Char('2') => {
                    self.state.focus = Focus::Channels;
                    return;
                }
                KeyCode::Char('3') => {
                    self.state.focus = Focus::Chat;
                    return;
                }
                KeyCode::Char('4') => {
                    self.state.focus = Focus::Input;
                    return;
                }
                KeyCode::Char('5') if self.state.show_member_rail => {
                    self.state.focus = Focus::Members;
                    return;
                }
                _ => {}
            }
        }

        // Pane sequential cycling with Tab / Shift+Tab
        if key.code == KeyCode::Tab {
            self.state.focus = self.state.focus.next(self.state.show_member_rail);
            return;
        }
        if key.code == KeyCode::BackTab {
            self.state.focus = self.state.focus.prev(self.state.show_member_rail);
            return;
        }

        // Contextual Pane Handling
        match self.state.focus {
            Focus::Servers => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.state.selected_server_index > 0 {
                        self.state.selected_server_index -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.state.selected_server_index + 1 <= self.state.servers.len() {
                        self.state.selected_server_index += 1;
                    }
                }
                KeyCode::Enter => {
                    if self.state.selected_server_index == 0 {
                        self.state.select_server(None);
                    } else if let Some(server) = self.state.servers.get(self.state.selected_server_index - 1) {
                        self.state.select_server(Some(server.id.clone()));
                    }
                }
                _ => {}
            },
            Focus::Channels => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.state.selected_channel_index > 0 {
                        self.state.selected_channel_index -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let count = self.state.rect_channel_items.len();
                    if self.state.selected_channel_index + 1 < count {
                        self.state.selected_channel_index += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some((cid, _)) = self.state.rect_channel_items.get(self.state.selected_channel_index) {
                        self.state.select_channel(cid.clone());
                    }
                }
                _ => {}
            },
            Focus::Chat => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.state.chat_scroll_offset += 2;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.state.chat_scroll_offset = self.state.chat_scroll_offset.saturating_sub(2);
                }
                KeyCode::PageUp => {
                    self.state.chat_scroll_offset += 15;
                }
                KeyCode::PageDown => {
                    self.state.chat_scroll_offset = self.state.chat_scroll_offset.saturating_sub(15);
                }
                KeyCode::Home => {
                    self.state.chat_scroll_offset = 10000;
                }
                KeyCode::End => {
                    self.state.chat_scroll_offset = 0;
                }
                KeyCode::Char('i') | KeyCode::Enter => {
                    self.state.focus = Focus::Input;
                }
                _ => {}
            },
            Focus::Input => match key.code {
                KeyCode::Esc => {
                    self.state.focus = Focus::Chat;
                }
                KeyCode::Enter => {
                    let content = self.state.input_clear();
                    if !content.trim().is_empty() {
                        self.send_message(content).await;
                    }
                }
                KeyCode::Backspace => {
                    self.state.input_backspace();
                }
                KeyCode::Delete => {
                    self.state.input_delete();
                }
                KeyCode::Left => {
                    self.state.input_cursor_left();
                }
                KeyCode::Right => {
                    self.state.input_cursor_right();
                }
                KeyCode::Home => {
                    self.state.input_cursor_home();
                }
                KeyCode::End => {
                    self.state.input_cursor_end();
                }
                KeyCode::Char(c) => {
                    self.state.input_insert_char(c);
                }
                _ => {}
            },
            Focus::Members => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.state.selected_member_index > 0 {
                        self.state.selected_member_index -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.state.selected_member_index += 1;
                }
                KeyCode::Esc => {
                    self.state.focus = Focus::Chat;
                }
                _ => {}
            },
        }
    }

    async fn handle_mouse(&mut self, mouse: MouseEvent) {
        let (mx, my) = (mouse.column, mouse.row);

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // If login modal is open, ignore outside clicks or handle
                if self.state.show_login_modal {
                    return;
                }

                // If command palette is open
                if self.state.show_command_palette {
                    let rect = self.state.rect_command_palette;
                    if mx >= rect.left() && mx < rect.right() && my >= rect.top() && my < rect.bottom() {
                        // Clicked inside command palette
                        let inner_y = my.saturating_sub(rect.top() + 3) as usize;
                        let items = self.state.get_palette_items();
                        if let Some(item) = items.get(inner_y) {
                            match item.item_type {
                                PaletteItemType::Server => self.state.select_server(Some(item.id.clone())),
                                PaletteItemType::Channel => {
                                    if let Some(ref sid) = item.server_id {
                                        self.state.select_server(Some(sid.clone()));
                                    }
                                    self.state.select_channel(item.id.clone());
                                }
                                PaletteItemType::DirectMessage => {
                                    self.state.select_server(None);
                                    self.state.select_channel(item.id.clone());
                                }
                            }
                            self.state.show_command_palette = false;
                        }
                    } else {
                        // Clicked outside: close palette
                        self.state.show_command_palette = false;
                    }
                    return;
                }

                // Member rail toggle button clicked in header
                let toggle_rect = self.state.rect_member_toggle;
                if mx >= toggle_rect.left() && mx < toggle_rect.right() && my == toggle_rect.top() {
                    self.state.show_member_rail = !self.state.show_member_rail;
                    return;
                }

                // 1. Click Left Rail (Servers & DMs)
                let rail = self.state.rect_rail;
                if mx >= rail.left() && mx < rail.right() && my >= rail.top() && my < rail.bottom() {
                    self.state.focus = Focus::Servers;

                    for (srv_opt, rect) in &self.state.rect_server_items {
                        if my >= rect.top() && my <= rect.bottom() {
                            self.state.select_server(srv_opt.clone());
                            break;
                        }
                    }
                    return;
                }

                // 2. Click Channel Sidebar
                let chan = self.state.rect_channels;
                if mx >= chan.left() && mx < chan.right() && my >= chan.top() && my < chan.bottom() {
                    self.state.focus = Focus::Channels;

                    for (cid, rect) in &self.state.rect_channel_items {
                        if my == rect.top() {
                            self.state.select_channel(cid.clone());
                            break;
                        }
                    }
                    return;
                }

                // 3. Click Input area
                let inp = self.state.rect_input;
                if mx >= inp.left() && mx < inp.right() && my >= inp.top() && my < inp.bottom() {
                    self.state.focus = Focus::Input;
                    return;
                }

                // 4. Click Chat area
                let chat = self.state.rect_chat;
                if mx >= chat.left() && mx < chat.right() && my >= chat.top() && my < chat.bottom() {
                    self.state.focus = Focus::Chat;
                    return;
                }

                // 5. Click Member rail
                let mem = self.state.rect_members;
                if self.state.show_member_rail && mx >= mem.left() && mx < mem.right() && my >= mem.top() && my < mem.bottom() {
                    self.state.focus = Focus::Members;
                    return;
                }
            }

            MouseEventKind::ScrollUp => {
                let chat = self.state.rect_chat;
                if mx >= chat.left() && mx < chat.right() && my >= chat.top() && my < chat.bottom() {
                    self.state.chat_scroll_offset += 3;
                }
            }

            MouseEventKind::ScrollDown => {
                let chat = self.state.rect_chat;
                if mx >= chat.left() && mx < chat.right() && my >= chat.top() && my < chat.bottom() {
                    self.state.chat_scroll_offset = self.state.chat_scroll_offset.saturating_sub(3);
                }
            }

            _ => {}
        }
    }

    async fn handle_gateway(&mut self, event: GatewayEvent) {
        match event {
            GatewayEvent::Connected => {
                self.state.gateway_status = GatewayStatus::Connecting;
            }
            GatewayEvent::Authenticated => {
                self.state.gateway_status = GatewayStatus::Connected { latency_ms: 20 };
                self.state.set_status("Connected to Bonfire Gateway");
            }
            GatewayEvent::Latency(ms) => {
                self.state.gateway_status = GatewayStatus::Connected { latency_ms: ms };
            }
            GatewayEvent::Disconnected(reason) => {
                self.state.gateway_status = GatewayStatus::Disconnected(reason);
            }
            GatewayEvent::MessageReceived(msg) => match msg {
                ServerMessage::Ready { users, servers, channels, members } => {
                    for u in users {
                        self.state.users.insert(u.id.clone(), u);
                    }
                    for s in servers {
                        self.state.servers.push(s);
                    }
                    for ch in channels {
                        if let Some(ref sid) = ch.server {
                            self.state.server_channels.entry(sid.clone()).or_default().push(ch.id.clone());
                        } else if ch.channel_type == "DirectMessage" {
                            self.state.dms.push(ch.clone());
                        }
                        self.state.channels.insert(ch.id.clone(), ch);
                    }
                    for m in members {
                        self.state.members.entry(m.id.server.clone()).or_default().push(m);
                    }

                    // Auto-select first server and channel if none selected
                    if self.state.active_server_id.is_none() {
                        if let Some(first_srv) = self.state.servers.first() {
                            self.state.select_server(Some(first_srv.id.clone()));
                        }
                    }
                }
                ServerMessage::Message(m) => {
                    let channel_id = m.channel.clone();
                    let is_active_channel = self.state.active_channel_id.as_deref() == Some(&channel_id);

                    self.state.messages.entry(channel_id.clone()).or_default().push(m);

                    if !is_active_channel {
                        *self.state.unread_counts.entry(channel_id).or_insert(0) += 1;
                    } else if self.state.chat_scroll_offset == 0 {
                        // Keep at bottom
                    }
                }
                ServerMessage::MessageUpdate { id, data } => {
                    for msgs in self.state.messages.values_mut() {
                        if let Some(m) = msgs.iter_mut().find(|msg| msg.id == id) {
                            if let Some(content) = data.get("content").and_then(|c| c.as_str()) {
                                m.content = Some(content.to_string());
                            }
                        }
                    }
                }
                ServerMessage::MessageDelete { id, channel } => {
                    if let Some(msgs) = self.state.messages.get_mut(&channel) {
                        msgs.retain(|m| m.id != id);
                    }
                }
                ServerMessage::ChannelCreate(ch) => {
                    if let Some(ref sid) = ch.server {
                        self.state.server_channels.entry(sid.clone()).or_default().push(ch.id.clone());
                    }
                    self.state.channels.insert(ch.id.clone(), ch);
                }
                ServerMessage::ChannelDelete { id } => {
                    self.state.channels.remove(&id);
                }
                _ => {}
            },
        }
    }

    async fn send_message(&mut self, content: String) {
        let channel_id = match &self.state.active_channel_id {
            Some(cid) => cid.clone(),
            None => return,
        };

        // If authenticated with real token, send via REST API
        if let Some(ref _token) = self.state.config.token {
            let api = self.api_client.clone();
            let tx = self.event_tx.clone();
            let cid = channel_id.clone();
            let cnt = content.clone();

            tokio::spawn(async move {
                let client = api.lock().await;
                match client.send_message(&cid, &cnt).await {
                    Ok(msg) => {
                        let _ = tx.send(AppEvent::ApiMessageSent(Ok(msg))).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::ApiMessageSent(Err(e.to_string()))).await;
                    }
                }
            });
        } else {
            // Local Demo/Offline Mode: Immediately append to channel messages and auto-scroll
            let current_user_id = self.state.current_user.as_ref().map(|u| u.id.clone()).unwrap_or_else(|| "user_me".to_string());
            let new_msg = Message {
                id: format!("demo_msg_{}", chrono::Utc::now().timestamp_millis()),
                channel: channel_id.clone(),
                author: current_user_id,
                content: Some(content.clone()),
                reactions: None,
                ..Default::default()
            };

            self.state.messages.entry(channel_id.clone()).or_default().push(new_msg);
            self.state.chat_scroll_offset = 0;

            // Simulate quick echo response from Alice if asking a question or demoing
            let tx = self.event_tx.clone();
            let cid = channel_id.clone();
            let cnt_lower = content.to_lowercase();

            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let reply_text = if cnt_lower.contains("help") || cnt_lower.contains("hello") || cnt_lower.contains("hi") {
                    "Hello! You're testing **Stoat TUI** in interactive demo mode. Try pressing `Ctrl+P` for the command palette or clicking channels with your mouse!".to_string()
                } else if cnt_lower.contains("ratatui") || cnt_lower.contains("rust") {
                    "🦀 Rust + Ratatui delivers blazingly fast 60fps terminal rendering with zero flicker!".to_string()
                } else {
                    format!("Received: \"{}\" — message dispatched smoothly across the Bonfire gateway simulation.", content)
                };

                let bot_msg = Message {
                    id: format!("bot_reply_{}", chrono::Utc::now().timestamp_millis()),
                    channel: cid.clone(),
                    author: "user_alice".to_string(),
                    content: Some(reply_text),
                    reactions: Some({
                        let mut r = HashMap::new();
                        r.insert("❤️".to_string(), vec!["user_alice".to_string()]);
                        r
                    }),
                    ..Default::default()
                };

                let _ = tx.send(AppEvent::ApiMessageSent(Ok(bot_msg))).await;
            });
        }
    }
}
