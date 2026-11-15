use crate::api::models::{LoginResponse, Message};
use crate::api::DeltaClient;
use crate::config::Config;
use crate::event::AppEvent;
use crate::gateway::messages::ServerMessage;
use crate::gateway::{BonfireClient, GatewayEvent};
use crate::state::{ActiveRail, AppState, Focus, GatewayStatus, LoginStep, LoginTab, PaletteItemType};
use crate::{log_debug, log_error, log_info, log_warn};
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

        let (user_fetch_tx, mut user_fetch_rx) = mpsc::unbounded_channel::<String>();
        let mut state = AppState::new(config);
        state.user_fetch_tx = Some(user_fetch_tx);

        let api_client_clone = api_client.clone();
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some(user_id) = user_fetch_rx.recv().await {
                let client = api_client_clone.lock().await;
                match client.fetch_user(&user_id).await {
                    Ok(user) => {
                        let _ = event_tx_clone.send(AppEvent::ApiUserFetched(user)).await;
                    }
                    Err(e) => {
                        log_warn!("Background user fetch failed for {}: {}", user_id, e);
                    }
                }
            }
        });

        Self {
            state,
            api_client,
            event_tx,
            gateway_handle: None,
        }
    }

    pub fn startup(&mut self) {
        let token_opt = self.state.config.token.clone();
        let has_token = token_opt.as_deref().map(|t| !t.trim().is_empty()).unwrap_or(false);

        if !has_token {
            log_info!("Startup: No session token found. Displaying login modal.");
            self.state.show_login_modal = true;
            self.state.gateway_status = GatewayStatus::Disconnected("Not logged in".to_string());
            self.state.status_message = Some(("Please log in to connect".to_string(), chrono::Utc::now()));
            return;
        }

        // Wipe mock data immediately upon starting an authenticated session
        self.state.clear_mock_data();

        let token = token_opt.unwrap();
        log_info!("Startup: Found stored session token. Initiating startup authentication verification...");
        self.state.show_login_modal = false;
        self.state.gateway_status = GatewayStatus::Connecting;
        self.state.status_message = Some(("Connecting...".to_string(), chrono::Utc::now()));

        let api = self.api_client.clone();
        let tx = self.event_tx.clone();
        let gw_url_config = self.state.config.gateway_url.clone();

        tokio::spawn(async move {
            let client = api.lock().await;

            // 1. Call GET /users/@me with header x-session-token: <TOKEN>
            match client.get_current_user().await {
                Ok(user) => {
                    log_info!("Startup: Successfully authenticated user @{} ({})", user.username, user.id);
                    let _ = tx.send(AppEvent::ApiCurrentUser(user)).await;

                    // Also fetch initial DMs
                    if let Ok(dms) = client.fetch_dms().await {
                        let _ = tx.send(AppEvent::ApiDmsFetched(dms)).await;
                    }

                    // 2. Query GET / to get dynamic gateway WebSocket URL (meta.ws)
                    let effective_gw_url = match client.fetch_metadata().await {
                        Ok(meta) => {
                            if let Some(ws) = meta.ws {
                                log_info!("Startup: Dynamic gateway URL extracted: {}", ws);
                                Some(ws)
                            } else {
                                gw_url_config
                            }
                        }
                        Err(e) => {
                            log_warn!("Startup: Could not query dynamic gateway URL from GET /: {}", e);
                            gw_url_config
                        }
                    };

                    // 3. Connect to Bonfire WebSocket gateway
                    let _ = tx.send(AppEvent::StartGateway {
                        token,
                        gateway_url: effective_gw_url,
                    }).await;
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    log_warn!("Startup: GET /users/@me verification failed: {}", err_msg);
                    if err_msg.contains("401") || err_msg.contains("Unauthorized") || err_msg.contains("Session expired") {
                        log_warn!("Startup: Token expired or rejected (HTTP 401). Clearing token and requesting login.");
                        let _ = tx.send(AppEvent::ApiSessionExpired).await;
                    } else {
                        // Network error / connection timeout - still attempt gateway connection
                        log_warn!("Startup: Non-401 error during GET /users/@me: {}. Proceeding with gateway connection.", err_msg);
                        let _ = tx.send(AppEvent::StartGateway {
                            token,
                            gateway_url: gw_url_config,
                        }).await;
                    }
                }
            }
        });
    }

    pub fn start_gateway(&mut self, token: String, gateway_url: Option<String>) {
        if let Some(h) = self.gateway_handle.take() {
            h.abort();
        }

        let event_tx = self.event_tx.clone();
        let api_client = self.api_client.clone();

        let gw_url = gateway_url.unwrap_or_else(|| "wss://events.stoat.chat".to_string());

        // Spawn a gateway runner that queries GET / on the API endpoint to extract the active WebSocket URL
        let handle = tokio::spawn(async move {
            let (gw_tx, mut gw_rx) = mpsc::channel(128);

            // Dynamic Gateway URL: Query GET / on API endpoint to extract active WebSocket URL (ws field)
            let effective_gw_url = {
                let client = api_client.lock().await;
                match client.fetch_metadata().await {
                    Ok(meta) => {
                        if let Some(ws) = meta.ws {
                            ws
                        } else {
                            gw_url
                        }
                    }
                    Err(_) => gw_url,
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
            AppEvent::ApiLoginSuccess { token, is_bot } | AppEvent::LoginSuccess { token, is_bot } => {
                let trimmed_token = token.trim().to_string();
                log_info!("Authentication succeeded! Updating session token and persisting config...");
                self.state.login_loading = false;
                self.state.login_error = None;
                self.state.login_step = LoginStep::Credentials;
                self.state.show_login_modal = false;
                self.state.clear_mock_data();
                self.state.config.token = Some(trimmed_token.clone());
                self.state.config.is_bot = is_bot;
                if let Err(e) = self.state.config.save() {
                    log_error!("Failed to save config after login: {}", e);
                }

                {
                    let mut api = self.api_client.lock().await;
                    api.set_token(trimmed_token.clone(), is_bot);
                }

                self.start_gateway(trimmed_token, self.state.config.gateway_url.clone());
                self.fetch_initial_data().await;
            }
            AppEvent::ApiMfaRequired { ticket, allowed_methods: _ } => {
                self.state.login_loading = false;
                self.state.login_error = None;
                self.state.login_step = LoginStep::TotpChallenge { ticket };
                self.state.login_totp_input.clear();
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
            AppEvent::ApiMessagesFetched { channel_id, mut messages } => {
                // Stoat/Revolt Delta returns Latest first; sort by ULID id ascending for chronological feed
                messages.sort_by(|a, b| a.id.cmp(&b.id));
                for msg in &messages {
                    let _ = self.state.resolve_user(&msg.author);
                }
                self.state.messages.insert(channel_id, messages);
            }
            AppEvent::ApiCurrentUser(user) => {
                self.state.users.insert(user.id.clone(), user.clone());
                self.state.current_user = Some(user);
            }
            AppEvent::ApiUserFetched(user) => {
                log_info!("Hydrated user into cache: @{} ({})", user.username, user.id);
                self.state.users.insert(user.id.clone(), user);
            }
            AppEvent::ApiDmsFetched(dms) => {
                for dm in &dms {
                    if let Some(ref recipients) = dm.recipients {
                        for r in recipients {
                            let _ = self.state.resolve_user(r);
                        }
                    }
                }
                self.state.dms = dms;
            }
            AppEvent::ApiSessionExpired => {
                log_warn!("Handling ApiSessionExpired: Invalidating session token, showing login modal");
                self.state.config.token = None;
                let _ = self.state.config.save();
                self.state.current_user = None;
                self.state.login_loading = false;
                self.state.login_error = Some("Session expired, please log in again.".to_string());
                self.state.show_login_modal = true;
                self.state.gateway_status = GatewayStatus::Disconnected("Session expired".to_string());
                self.state.set_status("Session expired, please log in again.");
            }
            AppEvent::VideoFrameReady => {}
            AppEvent::CallConnected { channel_id } => {
                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        call.status = crate::voice::CallStatus::Connected;
                        call.connected_at = Some(std::time::Instant::now());
                    }
                }
            }
            AppEvent::StartGateway { token, gateway_url } => {
                self.start_gateway(token, gateway_url);
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
            if matches!(self.state.login_step, LoginStep::TotpChallenge { .. }) {
                for ch in text.chars() {
                    if ch.is_ascii_digit() && self.state.login_totp_input.len() < 8 {
                        self.state.login_totp_input.push(ch);
                    }
                }
                return;
            }

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
                                self.switch_to_server(Some(item.id.clone())).await;
                            }
                            PaletteItemType::Channel => {
                                self.jump_to_channel(item.id.clone(), item.server_id.clone()).await;
                            }
                            PaletteItemType::DirectMessage => {
                                self.jump_to_channel(item.id.clone(), None).await;
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
            if let LoginStep::TotpChallenge { ticket } = self.state.login_step.clone() {
                match key.code {
                    KeyCode::Esc => {
                        self.state.login_step = LoginStep::Credentials;
                        self.state.login_error = None;
                        self.state.login_totp_input.clear();
                    }
                    KeyCode::Backspace => {
                        self.state.login_totp_input.pop();
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() && self.state.login_totp_input.len() < 8 => {
                        self.state.login_totp_input.push(c);
                    }
                    KeyCode::Enter => {
                        let code = self.state.login_totp_input.trim().to_string();
                        if code.is_empty() {
                            self.state.login_error = Some("Please enter your 6-digit TOTP code".to_string());
                            return;
                        }
                        if code.len() < 6 {
                            self.state.login_error = Some("TOTP code must be at least 6 digits".to_string());
                            return;
                        }

                        self.state.login_loading = true;
                        self.state.login_error = None;

                        let api = self.api_client.clone();
                        let tx = self.event_tx.clone();
                        let ticket_clone = ticket.clone();

                        tokio::spawn(async move {
                            let client = api.lock().await;
                            match client.verify_mfa_totp(&ticket_clone, &code).await {
                                Ok(LoginResponse::Success { token, .. }) => {
                                    let trimmed = token.trim().to_string();
                                    log_info!("MFA TOTP verification succeeded! Emitting LoginSuccess event.");
                                    let _ = tx.send(AppEvent::LoginSuccess {
                                        token: trimmed,
                                        is_bot: false,
                                    }).await;
                                }
                                Ok(LoginResponse::MFA { ticket: new_ticket, allowed_methods }) => {
                                    let _ = tx.send(AppEvent::ApiMfaRequired {
                                        ticket: new_ticket,
                                        allowed_methods,
                                    }).await;
                                }
                                Ok(LoginResponse::Disabled) => {
                                    let _ = tx.send(AppEvent::ApiLoginFailure("Account is disabled".to_string())).await;
                                }
                                Err(e) => {
                                    let _ = tx.send(AppEvent::ApiLoginFailure(e.to_string())).await;
                                }
                            }
                        });
                    }
                    _ => {}
                }
                return;
            }

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
                                    Ok(LoginResponse::Success { token, .. }) => {
                                        let trimmed = token.trim().to_string();
                                        log_info!("Password login succeeded! Emitting ApiLoginSuccess event.");
                                        let _ = tx.send(AppEvent::ApiLoginSuccess {
                                            token: trimmed,
                                            is_bot: false,
                                        }).await;
                                    }
                                    Ok(LoginResponse::MFA { ticket, allowed_methods }) => {
                                        let _ = tx.send(AppEvent::ApiMfaRequired {
                                            ticket,
                                            allowed_methods,
                                        }).await;
                                    }
                                    Ok(LoginResponse::Disabled) => {
                                        let _ = tx.send(AppEvent::ApiLoginFailure("Account is disabled".to_string())).await;
                                    }
                                    Err(e) => {
                                        let _ = tx.send(AppEvent::ApiLoginFailure(e.to_string())).await;
                                    }
                                }
                            });
                        }
                        LoginTab::Token => {
                            let token = self.state.login_token_input.trim().to_string();
                            if token.is_empty() {
                                self.state.login_error = Some("Token cannot be empty".to_string());
                                return;
                            }
                            self.state.login_loading = true;
                            self.state.login_error = None;
                            log_info!("Manual token tab submitted! Emitting ApiLoginSuccess event.");
                            let _ = self.event_tx.send(AppEvent::ApiLoginSuccess {
                                token,
                                is_bot: false,
                            }).await;
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
        // Call & Stream Global Controls: Alt + c, s, m, d, f
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    self.toggle_call().await;
                    return;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let active = call.toggle_screen_share();
                        self.state.set_status(if active { "Screen sharing started" } else { "Screen sharing stopped" });
                    }
                    return;
                }
                KeyCode::Char('m') | KeyCode::Char('M') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let muted = call.toggle_mute();
                        self.state.set_status(if muted { "Microphone muted" } else { "Microphone unmuted" });
                    }
                    return;
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let deafened = call.toggle_deafen();
                        self.state.set_status(if deafened { "Audio deafened (mic muted)" } else { "Audio undeafened" });
                    }
                    return;
                }
                KeyCode::Char('f') | KeyCode::Char('F') => {
                    if self.state.active_call.is_some() {
                        self.state.call_fullscreen = !self.state.call_fullscreen;
                        self.state.set_status(if self.state.call_fullscreen { "Call fullscreen enabled" } else { "Call fullscreen disabled" });
                    }
                    return;
                }
                _ => {}
            }
        }

        // Fullscreen exit via Esc
        if self.state.call_fullscreen && key.code == KeyCode::Esc {
            self.state.call_fullscreen = false;
            return;
        }

        // Unfocused Call Shortcuts (c, s, m, d, f) when not typing in Input bar
        if self.state.focus != Focus::Input {
            match key.code {
                KeyCode::Char('c') => {
                    self.toggle_call().await;
                    return;
                }
                KeyCode::Char('s') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let active = call.toggle_screen_share();
                        self.state.set_status(if active { "Screen sharing started" } else { "Screen sharing stopped" });
                        return;
                    }
                }
                KeyCode::Char('m') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let muted = call.toggle_mute();
                        self.state.set_status(if muted { "Microphone muted" } else { "Microphone unmuted" });
                        return;
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let deafened = call.toggle_deafen();
                        self.state.set_status(if deafened { "Audio deafened (mic muted)" } else { "Audio undeafened" });
                        return;
                    }
                }
                KeyCode::Char('f') => {
                    if self.state.active_call.is_some() {
                        self.state.call_fullscreen = !self.state.call_fullscreen;
                        self.state.set_status(if self.state.call_fullscreen { "Call fullscreen enabled" } else { "Call fullscreen disabled" });
                        return;
                    }
                }
                _ => {}
            }
        }

        // Manual Login Modal Trigger: F2 or Ctrl+L
        if key.code == KeyCode::F(2)
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && (key.code == KeyCode::Char('l') || key.code == KeyCode::Char('L')))
        {
            self.state.show_login_modal = true;
            return;
        }

        // Command Palette
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            self.state.show_command_palette = true;
            self.state.command_palette_query.clear();
            self.state.command_palette_selected = 0;
            return;
        }

        // Fast Keyboard Navigation Shortcuts
        // 1. Server Switching: Ctrl + Alt + j/k OR Ctrl + Alt + Down/Up
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                    self.nav_next_server().await;
                    return;
                }
                KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                    self.nav_prev_server().await;
                    return;
                }
                _ => {}
            }
        }

        // 2. Unread Navigation: Alt + Shift + j/k OR Alt + Shift + Down/Up
        if key.modifiers.contains(KeyModifiers::ALT)
            && (key.modifiers.contains(KeyModifiers::SHIFT) || matches!(key.code, KeyCode::Char('J') | KeyCode::Char('K')))
        {
            match key.code {
                KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                    self.nav_next_unread().await;
                    return;
                }
                KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                    self.nav_prev_unread().await;
                    return;
                }
                _ => {}
            }
        }

        // 3. Channel Switching: Alt + j/k OR Alt + Down/Up
        if key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
        {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.nav_next_channel().await;
                    return;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.nav_prev_channel().await;
                    return;
                }
                _ => {}
            }
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
                    if self.state.selected_server_index < self.state.servers.len() {
                        self.state.selected_server_index += 1;
                    }
                }
                KeyCode::Enter => {
                    if self.state.selected_server_index == 0 {
                        self.switch_to_server(None).await;
                    } else if let Some(server) = self.state.servers.get(self.state.selected_server_index - 1) {
                        self.switch_to_server(Some(server.id.clone())).await;
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
                        self.switch_to_channel(cid.clone()).await;
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
                                PaletteItemType::Server => self.switch_to_server(Some(item.id.clone())).await,
                                PaletteItemType::Channel => {
                                    self.jump_to_channel(item.id.clone(), item.server_id.clone()).await;
                                }
                                PaletteItemType::DirectMessage => {
                                    self.jump_to_channel(item.id.clone(), None).await;
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

                // Call banner clicked under header
                if let Some(br) = self.state.rect_call_banner {
                    if mx >= br.left() && mx < br.right() && my >= br.top() && my < br.bottom() {
                        self.toggle_call().await;
                        return;
                    }
                }

                // 1. Click Left Rail (Servers & DMs)
                let rail = self.state.rect_rail;
                if mx >= rail.left() && mx < rail.right() && my >= rail.top() && my < rail.bottom() {
                    self.state.focus = Focus::Servers;

                    for (srv_opt, rect) in &self.state.rect_server_items {
                        if my >= rect.top() && my <= rect.bottom() {
                            self.switch_to_server(srv_opt.clone()).await;
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
                            self.switch_to_channel(cid.clone()).await;
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
            GatewayEvent::Connecting => {
                log_info!("Gateway connecting event handled");
                self.state.gateway_status = GatewayStatus::Connecting;
            }
            GatewayEvent::Connected => {
                log_info!("Gateway connected event handled");
                self.state.gateway_status = GatewayStatus::Connecting;
            }
            GatewayEvent::Authenticated => {
                log_info!("Gateway authenticated event handled: status set to ● Online");
                self.state.clear_mock_data();
                self.state.gateway_status = GatewayStatus::Connected { latency_ms: 20 };
                self.state.set_status("● Online");
            }
            GatewayEvent::Latency(ms) => {
                self.state.gateway_status = GatewayStatus::Connected { latency_ms: ms };
            }
            GatewayEvent::Disconnected(reason) => {
                log_warn!("Gateway disconnected event: {}", reason);
                self.state.gateway_status = GatewayStatus::Disconnected(reason);
            }
            GatewayEvent::MessageReceived(msg) => match msg {
                ServerMessage::Ready { users, servers, channels, members } => {
                    self.state.clear_mock_data();
                    self.state.gateway_status = match self.state.gateway_status {
                        GatewayStatus::Connected { latency_ms } => GatewayStatus::Connected { latency_ms },
                        _ => GatewayStatus::Connected { latency_ms: 20 },
                    };
                    self.state.set_status("● Online");

                    self.state.users.clear();
                    self.state.servers.clear();
                    self.state.channels.clear();
                    self.state.server_channels.clear();
                    self.state.dms.clear();
                    self.state.members.clear();

                    for u in users {
                        self.state.users.insert(u.id.clone(), u);
                    }
                    for s in servers {
                        self.state.servers.push(s);
                    }
                    for ch in channels {
                        let is_server_chan = ch.server.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
                        if is_server_chan {
                            let sid = ch.server.clone().unwrap();
                            self.state.server_channels.entry(sid).or_default().push(ch.id.clone());
                        } else if ch.channel_type == "DirectMessage" || ch.channel_type == "Group" {
                            self.state.dms.push(ch.clone());
                        }
                        self.state.channels.insert(ch.id.clone(), ch);
                    }

                    // Also populate server_channels from s.channels if any
                    for s in &self.state.servers {
                        let entry = self.state.server_channels.entry(s.id.clone()).or_default();
                        for cid in &s.channels {
                            if !entry.contains(cid) {
                                entry.push(cid.clone());
                            }
                        }
                    }

                    for m in members {
                        self.state.members.entry(m.id.server.clone()).or_default().push(m);
                    }

                    // Pre-resolve all DM recipients
                    for dm in &self.state.dms {
                        if let Some(ref recipients) = dm.recipients {
                            for r in recipients {
                                let _ = self.state.resolve_user(r);
                            }
                        }
                    }

                    log_info!(
                        "Ready loaded: {} servers, {} channels, {} users, {} dms",
                        self.state.servers.len(),
                        self.state.channels.len(),
                        self.state.users.len(),
                        self.state.dms.len()
                    );

                    // Restore saved navigation location across restarts:
                    // 1. If config.last_in_dm is true: keep active_server_id = None (DMs).
                    //    Restore config.last_channel_id if present in channels, else first DM.
                    // 2. Else if config.last_server_id exists in servers:
                    //    Switch to that server, restoring config.last_channel_id if it belongs to that server.
                    // 3. Fallback (first-time launch or missing saved target):
                    //    Select servers.first() and its first text channel, or DMs if no servers.
                    let last_in_dm = self.state.config.last_in_dm;
                    let last_srv_id = self.state.config.last_server_id.clone();
                    let last_chan_id = self.state.config.last_channel_id.clone();

                    let mut restored = false;

                    if last_in_dm {
                        log_info!("Restoring saved session in Direct Messages view");
                        self.state.active_rail = ActiveRail::DirectMessages;
                        self.state.active_server_id = None;
                        self.state.selected_server_index = 0;

                        if let Some(ref lcid) = last_chan_id {
                            if self.state.channels.contains_key(lcid) {
                                self.switch_to_channel(lcid.clone()).await;
                                restored = true;
                            }
                        }
                        if !restored {
                            if let Some(first_dm) = self.state.dms.first() {
                                self.switch_to_channel(first_dm.id.clone()).await;
                                restored = true;
                            }
                        }
                    } else if let Some(ref saved_sid) = last_srv_id {
                        if self.state.servers.iter().any(|s| &s.id == saved_sid) {
                            log_info!("Restoring saved session in server {}", saved_sid);
                            self.state.active_rail = ActiveRail::Server(saved_sid.clone());
                            self.state.active_server_id = Some(saved_sid.clone());
                            if let Some(pos) = self.state.servers.iter().position(|s| &s.id == saved_sid) {
                                self.state.selected_server_index = pos + 1;
                            }

                            // Check if saved_channel_id belongs to this server
                            if let Some(ref lcid) = last_chan_id {
                                let belongs_to_server = self.state.server_channels
                                    .get(saved_sid)
                                    .map(|cids| cids.contains(lcid))
                                    .unwrap_or(false)
                                    || self.state.channels.get(lcid).and_then(|c| c.server.as_ref()) == Some(saved_sid);

                                if belongs_to_server {
                                    self.switch_to_channel(lcid.clone()).await;
                                    restored = true;
                                }
                            }

                            if !restored {
                                // Fallback to this server's first text channel
                                self.switch_to_server(Some(saved_sid.clone())).await;
                                restored = true;
                            }
                        }
                    }

                    if !restored {
                        log_info!("First-time launch or no saved location: Selecting first server or DM fallback");
                        if let Some(first_srv) = self.state.servers.first() {
                            let sid = first_srv.id.clone();
                            self.switch_to_server(Some(sid)).await;
                        } else if !self.state.dms.is_empty() {
                            self.switch_to_server(None).await;
                        }
                    }
                }
                ServerMessage::Message(m) => {
                    let _ = self.state.resolve_user(&m.author);
                    let channel_id = m.channel.clone();
                    let is_active_channel = self.state.active_channel_id.as_deref() == Some(&channel_id);

                    // Track call_started system event as active call participant
                    if let Some(ref sys) = m.system {
                        let ev_type = sys.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if ev_type == "call_started" {
                            let entry = self.state.voice_channel_participants.entry(channel_id.clone()).or_default();
                            let user_id = sys.get("user").or_else(|| sys.get("id")).and_then(|u| u.as_str()).unwrap_or("caller").to_string();
                            if !entry.iter().any(|p| p.user_id == user_id) {
                                entry.push(crate::voice::VoiceParticipant {
                                    user_id,
                                    is_speaking: false,
                                    is_muted: false,
                                    is_deafened: false,
                                    is_screensharing: false,
                                });
                            }
                        }
                    }

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
                ServerMessage::VoiceChannelJoin { id, user } => {
                    let entry = self.state.voice_channel_participants.entry(id).or_default();
                    if !entry.iter().any(|p| p.user_id == user) {
                        entry.push(crate::voice::VoiceParticipant {
                            user_id: user,
                            is_speaking: false,
                            is_muted: false,
                            is_deafened: false,
                            is_screensharing: false,
                        });
                    }
                }
                ServerMessage::VoiceChannelLeave { id, user } => {
                    if let Some(entry) = self.state.voice_channel_participants.get_mut(&id) {
                        entry.retain(|p| p.user_id != user);
                    }
                }
                ServerMessage::UserVoiceStateUpdate {
                    id,
                    user,
                    speaking,
                    muted,
                    deafened,
                    screensharing,
                } => {
                    if let Some(entry) = self.state.voice_channel_participants.get_mut(&id) {
                        if let Some(p) = entry.iter_mut().find(|p| p.user_id == user) {
                            if let Some(s) = speaking {
                                p.is_speaking = s;
                            }
                            if let Some(m) = muted {
                                p.is_muted = m;
                            }
                            if let Some(d) = deafened {
                                p.is_deafened = d;
                            }
                            if let Some(sc) = screensharing {
                                p.is_screensharing = sc;
                            }
                        }
                    }
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

    pub async fn fetch_channel_messages(&self, channel_id: String) {
        log_debug!("Fetching messages for channel {}", channel_id);
        let api = self.api_client.clone();
        let tx = self.event_tx.clone();
        let cid = channel_id.clone();

        tokio::spawn(async move {
            let client = api.lock().await;
            match client.fetch_messages(&cid, 50).await {
                Ok(mut messages) => {
                    messages.sort_by(|a, b| a.id.cmp(&b.id));
                    let _ = tx.send(AppEvent::ApiMessagesFetched {
                        channel_id: cid,
                        messages,
                    }).await;
                }
                Err(e) => {
                    log_error!("Failed to fetch channel messages for {}: {}", cid, e);
                }
            }
        });
    }

    pub async fn switch_to_channel(&mut self, channel_id: String) {
        self.state.active_channel_id = Some(channel_id.clone());
        self.state.chat_scroll_offset = 0;
        self.state.unread_counts.remove(&channel_id);

        let current_cids = self.state.current_channel_ids();
        if let Some(pos) = current_cids.iter().position(|id| id == &channel_id) {
            self.state.selected_channel_index = pos;
        }

        // Persist session navigation state
        self.state.config.last_in_dm = self.state.active_server_id.is_none();
        self.state.config.last_server_id = self.state.active_server_id.clone();
        self.state.config.last_channel_id = Some(channel_id.clone());
        let _ = self.state.config.save();

        self.fetch_channel_messages(channel_id).await;
    }

    pub async fn switch_to_server(&mut self, server_id: Option<String>) {
        if let Some(ref sid) = server_id {
            self.state.active_rail = ActiveRail::Server(sid.clone());
            self.state.active_server_id = Some(sid.clone());

            if let Some(pos) = self.state.servers.iter().position(|s| &s.id == sid) {
                self.state.selected_server_index = pos + 1;
            }

            // Find first text channel in this server
            let mut text_cid = None;
            if let Some(cids) = self.state.server_channels.get(sid) {
                for cid in cids {
                    if let Some(chan) = self.state.channels.get(cid) {
                        if chan.channel_type == "TextChannel" || chan.channel_type == "Text" {
                            text_cid = Some(cid.clone());
                            break;
                        }
                    }
                }
                if text_cid.is_none() {
                    text_cid = cids.first().cloned();
                }
            }

            if text_cid.is_none() {
                for (cid, chan) in &self.state.channels {
                    if chan.server.as_deref() == Some(sid) {
                        if chan.channel_type == "TextChannel" || chan.channel_type == "Text" {
                            text_cid = Some(cid.clone());
                            break;
                        }
                    }
                }
            }

            if let Some(cid) = text_cid {
                self.switch_to_channel(cid).await;
            } else {
                self.state.active_channel_id = None;
                self.state.chat_scroll_offset = 0;
                self.state.config.last_in_dm = false;
                self.state.config.last_server_id = Some(sid.clone());
                self.state.config.last_channel_id = None;
                let _ = self.state.config.save();
            }
        } else {
            self.state.active_rail = ActiveRail::DirectMessages;
            self.state.active_server_id = None;
            self.state.selected_server_index = 0;

            if let Some(first_dm) = self.state.dms.first() {
                self.switch_to_channel(first_dm.id.clone()).await;
            } else {
                self.state.active_channel_id = None;
                self.state.chat_scroll_offset = 0;
                self.state.config.last_in_dm = true;
                self.state.config.last_server_id = None;
                self.state.config.last_channel_id = None;
                let _ = self.state.config.save();
            }
        }
    }

    pub async fn jump_to_channel(&mut self, channel_id: String, server_id: Option<String>) {
        if let Some(ref sid) = server_id {
            self.state.active_rail = ActiveRail::Server(sid.clone());
            self.state.active_server_id = Some(sid.clone());
            if let Some(pos) = self.state.servers.iter().position(|s| &s.id == sid) {
                self.state.selected_server_index = pos + 1;
            }
        } else {
            self.state.active_rail = ActiveRail::DirectMessages;
            self.state.active_server_id = None;
            self.state.selected_server_index = 0;
        }

        self.switch_to_channel(channel_id).await;
    }

    pub fn get_server_targets(&self) -> Vec<Option<String>> {
        let mut targets = vec![None]; // None = Direct Messages
        for s in &self.state.servers {
            targets.push(Some(s.id.clone()));
        }
        targets
    }

    pub async fn nav_next_server(&mut self) {
        let targets = self.get_server_targets();
        if targets.is_empty() {
            return;
        }

        let cur_idx = targets
            .iter()
            .position(|opt| opt == &self.state.active_server_id)
            .unwrap_or(0);

        let next_idx = (cur_idx + 1) % targets.len();
        let target = targets[next_idx].clone();
        self.switch_to_server(target).await;
    }

    pub async fn nav_prev_server(&mut self) {
        let targets = self.get_server_targets();
        if targets.is_empty() {
            return;
        }

        let cur_idx = targets
            .iter()
            .position(|opt| opt == &self.state.active_server_id)
            .unwrap_or(0);

        let prev_idx = if cur_idx == 0 {
            targets.len() - 1
        } else {
            cur_idx - 1
        };
        let target = targets[prev_idx].clone();
        self.switch_to_server(target).await;
    }

    pub async fn nav_next_channel(&mut self) {
        let channels = self.state.current_channel_ids();
        if channels.is_empty() {
            return;
        }

        let cur_idx = self
            .state
            .active_channel_id
            .as_ref()
            .and_then(|cid| channels.iter().position(|id| id == cid))
            .unwrap_or(0);

        let next_idx = (cur_idx + 1) % channels.len();
        let next_cid = channels[next_idx].clone();
        self.switch_to_channel(next_cid).await;
    }

    pub async fn nav_prev_channel(&mut self) {
        let channels = self.state.current_channel_ids();
        if channels.is_empty() {
            return;
        }

        let cur_idx = self
            .state
            .active_channel_id
            .as_ref()
            .and_then(|cid| channels.iter().position(|id| id == cid))
            .unwrap_or(0);

        let prev_idx = if cur_idx == 0 {
            channels.len() - 1
        } else {
            cur_idx - 1
        };
        let prev_cid = channels[prev_idx].clone();
        self.switch_to_channel(prev_cid).await;
    }

    pub fn get_all_channels_ordered(&self) -> Vec<(String, Option<String>)> {
        let mut list = Vec::new();
        for dm in &self.state.dms {
            list.push((dm.id.clone(), None));
        }
        for srv in &self.state.servers {
            if let Some(cids) = self.state.server_channels.get(&srv.id) {
                for cid in cids {
                    list.push((cid.clone(), Some(srv.id.clone())));
                }
            }
        }
        list
    }

    pub fn get_unread_channels_ordered(&self) -> Vec<(String, Option<String>)> {
        self.get_all_channels_ordered()
            .into_iter()
            .filter(|(cid, _)| self.state.unread_counts.get(cid).copied().unwrap_or(0) > 0)
            .collect()
    }

    pub async fn nav_next_unread(&mut self) {
        let unreads = self.get_unread_channels_ordered();
        if unreads.is_empty() {
            self.state.set_status("No unread channels");
            return;
        }

        let all = self.get_all_channels_ordered();
        let cur_pos = self
            .state
            .active_channel_id
            .as_ref()
            .and_then(|cid| all.iter().position(|(id, _)| id == cid))
            .unwrap_or(0);

        let next_target = unreads
            .iter()
            .find(|(cid, _)| {
                let pos = all.iter().position(|(id, _)| id == cid).unwrap_or(0);
                pos > cur_pos
            })
            .or_else(|| unreads.first());

        if let Some((cid, srv_opt)) = next_target.cloned() {
            self.jump_to_channel(cid, srv_opt).await;
        }
    }

    pub async fn nav_prev_unread(&mut self) {
        let unreads = self.get_unread_channels_ordered();
        if unreads.is_empty() {
            self.state.set_status("No unread channels");
            return;
        }

        let all = self.get_all_channels_ordered();
        let cur_pos = self
            .state
            .active_channel_id
            .as_ref()
            .and_then(|cid| all.iter().position(|(id, _)| id == cid))
            .unwrap_or(0);

        let prev_target = unreads
            .iter()
            .rfind(|(cid, _)| {
                let pos = all.iter().position(|(id, _)| id == cid).unwrap_or(0);
                pos < cur_pos
            })
            .or_else(|| unreads.last());

        if let Some((cid, srv_opt)) = prev_target.cloned() {
            self.jump_to_channel(cid, srv_opt).await;
        }
    }

    pub async fn toggle_call(&mut self) {
        if self.state.active_call.is_some() {
            self.state.active_call = None;
            self.state.call_fullscreen = false;
            self.state.set_status("Disconnected from call");
            return;
        }

        let channel_id = match &self.state.active_channel_id {
            Some(cid) => cid.clone(),
            None => {
                self.state.set_status("No channel selected to join call");
                return;
            }
        };

        // Immediate UI feedback on 'c' / 'Alt+c': initialize state.active_call instantly with status Connecting
        let mut call = crate::voice::ActiveCallState::new(channel_id.clone());
        call.status = crate::voice::CallStatus::Connecting;
        call.connected_at = None;

        let frame_tx = self.event_tx.clone();
        let (notify_tx, mut notify_rx) = mpsc::channel(16);

        tokio::spawn(async move {
            while notify_rx.recv().await.is_some() {
                let _ = frame_tx.send(AppEvent::VideoFrameReady).await;
            }
        });

        let _engine_handle = crate::voice::VoiceEngine::start(
            String::new(),
            None,
            call.muted.clone(),
            call.screensharing.clone(),
            call.current_frame.clone(),
            notify_tx,
        );

        self.state.active_call = Some(call);
        self.state.set_status(format!("📞 Connecting to call in channel {}... [c: Disconnect]", channel_id));

        // Asynchronously request call token from Delta API
        let api = self.api_client.clone();
        let cid_clone = channel_id.clone();
        let app_tx = self.event_tx.clone();
        tokio::spawn(async move {
            let client = api.lock().await;
            match client.join_call(&cid_clone).await {
                Ok(_join_res) => {
                    log_info!("join_call returned LiveKit token for channel {}", cid_clone);
                    let _ = app_tx.send(AppEvent::CallConnected { channel_id: cid_clone }).await;
                }
                Err(e) => {
                    log_warn!("join_call API request returned error (falling back to connected local engine): {}", e);
                    let _ = app_tx.send(AppEvent::CallConnected { channel_id: cid_clone }).await;
                }
            }
        });
    }
}
