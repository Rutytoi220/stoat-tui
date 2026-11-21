use stoat_tui::config::{parse_hex_color, Config};
use stoat_tui::gateway::messages::{ClientMessage, ServerMessage};
use stoat_tui::state::{AppState, Focus, PaletteItemType};
use stoat_tui::ui::markdown::MarkdownParser;
use ratatui::style::Color;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_CONFIG_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempConfigGuard {
    path: PathBuf,
}

impl Drop for TempConfigGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::remove_file(parent.join("nav_cache.json"));
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn isolated_config(token: Option<&str>) -> (Config, TempConfigGuard) {
    let idx = TEST_CONFIG_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("stoat-tui-test-{}-{}", std::process::id(), idx));
    let path = dir.join("config.toml");
    let cfg = Config {
        token: token.map(|t| t.to_string()),
        custom_path: Some(path.clone()),
        ..Default::default()
    };
    (cfg, TempConfigGuard { path })
}

#[test]
fn test_config_defaults_and_hex_color() {
    let cfg = Config::default();
    assert_eq!(cfg.api_url, "https://api.stoat.chat");
    assert!(!cfg.is_bot);
    assert!(cfg.token.is_none());

    let red = parse_hex_color("#FD6671");
    assert_eq!(red, Color::Rgb(253, 102, 113));

    let blue = parse_hex_color("#5970FD");
    assert_eq!(blue, Color::Rgb(89, 112, 253));

    let invalid = parse_hex_color("xyz");
    assert_eq!(invalid, Color::Reset);
}

#[test]
fn test_login_response_and_error_serde() {
    use stoat_tui::api::models::{DirectLoginSuccess, LoginResponse, RevoltApiError};

    // 1. Tagged Success
    let success_json = r#"{"result":"Success","token":"sess_abc123","user_id":"u_01"}"#;
    let resp: LoginResponse = serde_json::from_str(success_json).unwrap();
    match resp {
        LoginResponse::Success { token, user_id, .. } => {
            assert_eq!(token, "sess_abc123");
            assert_eq!(user_id.as_deref(), Some("u_01"));
        }
        _ => panic!("Expected LoginResponse::Success"),
    }

    // 2. Tagged MFA
    let mfa_json = r#"{"result":"MFA","ticket":"mfa_tick_789","allowed_methods":["totp"]}"#;
    let mfa_resp: LoginResponse = serde_json::from_str(mfa_json).unwrap();
    match mfa_resp {
        LoginResponse::MFA { ticket, allowed_methods } => {
            assert_eq!(ticket, "mfa_tick_789");
            assert_eq!(allowed_methods, vec!["totp".to_string()]);
        }
        _ => panic!("Expected LoginResponse::MFA"),
    }

    // 3. Tagged Disabled
    let disabled_json = r#"{"result":"Disabled"}"#;
    let dis_resp: LoginResponse = serde_json::from_str(disabled_json).unwrap();
    assert!(matches!(dis_resp, LoginResponse::Disabled));

    // 4. Direct session fallback
    let direct_json = r#"{"_id":"sess_1","token":"direct_tok_xyz","user_id":"u_99"}"#;
    let direct: DirectLoginSuccess = serde_json::from_str(direct_json).unwrap();
    assert_eq!(direct.token, "direct_tok_xyz");

    // 5. Stoat / Revolt API Error JSON
    let err_json = r#"{"type":"InvalidCredentials"}"#;
    let api_err: RevoltApiError = serde_json::from_str(err_json).unwrap();
    assert_eq!(api_err.error_type, "InvalidCredentials");

    let val_err_json = r#"{"type":"FailedValidation","error":"email: Invalid address format"}"#;
    let val_err: RevoltApiError = serde_json::from_str(val_err_json).unwrap();
    assert_eq!(val_err.error_type, "FailedValidation");
    assert_eq!(val_err.error.as_deref(), Some("email: Invalid address format"));

    // 6. MFA Specific errors
    let code_err: RevoltApiError = serde_json::from_str(r#"{"type":"InvalidCode"}"#).unwrap();
    assert_eq!(code_err.error_type, "InvalidCode");

    let ticket_err: RevoltApiError = serde_json::from_str(r#"{"type":"InvalidTicket"}"#).unwrap();
    assert_eq!(ticket_err.error_type, "InvalidTicket");

    // 7. MFA TOTP payload serialization
    use stoat_tui::api::models::MfaTotpPayload;
    let payload = MfaTotpPayload {
        mfa_ticket: "mfa_challenge_abc".to_string(),
        code: "123456".to_string(),
    };
    let payload_json = serde_json::to_string(&payload).unwrap();
    assert!(payload_json.contains("\"mfa_ticket\":\"mfa_challenge_abc\""));
    assert!(payload_json.contains("\"code\":\"123456\""));

    // 8. Stoat Delta MFA Ticket payload serialization
    use stoat_tui::api::models::{MfaTicketFallbackPayload, MfaTicketPayload, MfaTicketResult};
    let ticket_payload = MfaTicketPayload {
        ticket: "ticket_xyz".to_string(),
        result: MfaTicketResult::Totp {
            code: "654321".to_string(),
        },
    };
    let ticket_json = serde_json::to_string(&ticket_payload).unwrap();
    assert_eq!(
        ticket_json,
        r#"{"ticket":"ticket_xyz","result":{"type":"Totp","code":"654321"}}"#
    );

    let fallback_payload = MfaTicketFallbackPayload {
        ticket: "ticket_xyz".to_string(),
        code: "654321".to_string(),
    };
    let fallback_json = serde_json::to_string(&fallback_payload).unwrap();
    assert_eq!(
        fallback_json,
        r#"{"ticket":"ticket_xyz","code":"654321"}"#
    );

    // 9. Stoat Delta Native MFA Session Login payload serialization
    use stoat_tui::api::models::{MfaResponse, MfaSessionLoginPayload};
    let session_mfa_payload = MfaSessionLoginPayload {
        mfa_ticket: "ticket_abc123".to_string(),
        mfa_response: MfaResponse {
            totp_code: "123456".to_string(),
        },
        friendly_name: "stoat-tui".to_string(),
    };
    let session_mfa_json = serde_json::to_string(&session_mfa_payload).unwrap();
    assert_eq!(
        session_mfa_json,
        r#"{"mfa_ticket":"ticket_abc123","mfa_response":{"totp_code":"123456"},"friendly_name":"stoat-tui"}"#
    );
}

#[test]
fn test_mfa_login_step_transitions() {
    use stoat_tui::state::LoginStep;

    let (cfg, _guard) = isolated_config(None);
    let mut state = AppState::new(cfg);
    assert_eq!(state.login_step, LoginStep::Credentials);
    assert_eq!(state.login_totp_input, "");

    // Simulate receiving MFA challenge (ApiMfaRequired event clears any errors)
    state.login_loading = false;
    state.login_error = None;
    state.login_step = LoginStep::TotpChallenge {
        ticket: "challenge_ticket_123".to_string(),
    };
    state.login_totp_input = "654321".to_string();

    assert_eq!(state.login_error, None);

    if let LoginStep::TotpChallenge { ref ticket } = state.login_step {
        assert_eq!(ticket, "challenge_ticket_123");
        assert_eq!(state.login_totp_input, "654321");
    } else {
        panic!("Expected LoginStep::TotpChallenge");
    }

    // Simulate Esc key returning to Credentials
    state.login_step = LoginStep::Credentials;
    state.login_totp_input.clear();
    assert_eq!(state.login_step, LoginStep::Credentials);
    assert_eq!(state.login_totp_input, "");
}

#[test]
fn test_api_url_fallback() {
    let mut cfg = Config::default();
    assert_eq!(cfg.api_url, "https://api.stoat.chat");

    cfg.toggle_api_fallback();
    assert_eq!(cfg.api_url, "https://api.revolt.chat");

    cfg.toggle_api_fallback();
    assert_eq!(cfg.api_url, "https://api.stoat.chat");
}

#[test]
fn test_markdown_parser() {
    let parser = MarkdownParser::default();

    // 1. Inline bold, code, link
    let line = "Hello **world**, check `code` and https://stoat.chat at @alice!";
    let lines = parser.parse_message(line);
    assert_eq!(lines.len(), 1);
    let spans = &lines[0].spans;
    assert!(spans.iter().any(|s| s.content.as_ref() == "world"));
    assert!(spans.iter().any(|s| s.content.contains("code")));
    assert!(spans.iter().any(|s| s.content.as_ref() == "https://stoat.chat"));
    assert!(spans.iter().any(|s| s.content.as_ref() == "@alice"));

    // 2. Blockquote
    let quote = "> This is a quote message";
    let q_lines = parser.parse_message(quote);
    assert_eq!(q_lines.len(), 1);
    assert!(q_lines[0].spans[0].content.contains("▎"));

    // 3. Code block
    let code_block = "```rust\nfn main() {\n    println!(\"Hi\");\n}\n```";
    let cb_lines = parser.parse_message(code_block);
    assert!(cb_lines.len() >= 4);
    assert!(cb_lines[0].spans[0].content.contains("rust"));
    assert!(cb_lines.last().unwrap().spans[0].content.contains("└───"));
}

#[test]
fn test_focus_cycling() {
    let mut focus = Focus::Servers;

    // With member rail open: Servers -> Channels -> Chat -> Input -> Members -> Servers
    focus = focus.next(true);
    assert_eq!(focus, Focus::Channels);
    focus = focus.next(true);
    assert_eq!(focus, Focus::Chat);
    focus = focus.next(true);
    assert_eq!(focus, Focus::Input);
    focus = focus.next(true);
    assert_eq!(focus, Focus::Members);
    focus = focus.next(true);
    assert_eq!(focus, Focus::Servers);

    // Prev cycling
    focus = focus.prev(true);
    assert_eq!(focus, Focus::Members);
    focus = focus.prev(true);
    assert_eq!(focus, Focus::Input);

    // Without member rail
    let focus_no_mem = Focus::Input.next(false);
    assert_eq!(focus_no_mem, Focus::Servers);
}

#[test]
fn test_input_buffer_operations() {
    let (cfg, _guard) = isolated_config(None);
    let mut state = AppState::new(cfg);
    assert_eq!(state.input_buffer, "");
    assert_eq!(state.input_cursor, 0);

    state.input_insert_char('H');
    state.input_insert_char('i');
    assert_eq!(state.input_buffer, "Hi");
    assert_eq!(state.input_cursor, 2);

    state.input_cursor_left();
    assert_eq!(state.input_cursor, 1);

    state.input_insert_char('e');
    state.input_insert_char('y');
    assert_eq!(state.input_buffer, "Heyi");

    state.input_backspace();
    assert_eq!(state.input_buffer, "Hei");

    state.input_cursor_end();
    state.input_insert_char('!');
    assert_eq!(state.input_buffer, "Hei!");

    let cleared = state.input_clear();
    assert_eq!(cleared, "Hei!");
    assert_eq!(state.input_buffer, "");
    assert_eq!(state.input_cursor, 0);
}

#[test]
fn test_state_demo_data_and_palette() {
    let (cfg, _guard) = isolated_config(None);
    let mut state = AppState::new(cfg);
    state.load_demo_data();

    assert_eq!(state.servers.len(), 2);
    assert_eq!(state.channels.len(), 8);
    assert_eq!(state.dms.len(), 2);

    // Test command palette
    state.command_palette_query = "announcements".to_string();
    let results = state.get_palette_items();
    assert!(!results.is_empty());
    assert_eq!(results[0].item_type, PaletteItemType::Channel);
    assert!(results[0].title.contains("announcements"));

    // Search for DM
    state.command_palette_query = "Alice".to_string();
    let dm_results = state.get_palette_items();
    assert!(!dm_results.is_empty());
    assert!(dm_results.iter().any(|r| r.item_type == PaletteItemType::DirectMessage));
}

#[test]
fn test_gateway_messages_serde() {
    // Client message
    let ping = ClientMessage::Ping { data: 123456789 };
    let json = serde_json::to_string(&ping).unwrap();
    assert!(json.contains("\"type\":\"Ping\""));
    assert!(json.contains("123456789"));

    let auth = ClientMessage::Authenticate { token: "secret_token".to_string() };
    let auth_json = serde_json::to_string(&auth).unwrap();
    assert!(auth_json.contains("\"type\":\"Authenticate\""));
    assert!(auth_json.contains("secret_token"));

    // Server message
    let ready_json = r#"{
        "type": "Ready",
        "users": [
            { "_id": "u1", "username": "alice", "display_name": "Alice Wonder" }
        ],
        "servers": [
            { "_id": "s1", "name": "Stoat Server", "channels": ["c1"] }
        ],
        "channels": [
            { "_id": "c1", "channel_type": "TextChannel", "name": "general", "server": "s1" }
        ],
        "members": []
    }"#;

    let server_msg: ServerMessage = serde_json::from_str(ready_json).unwrap();
    match server_msg {
        ServerMessage::Ready { users, servers, channels, .. } => {
            assert_eq!(users.len(), 1);
            assert_eq!(users[0].username, "alice");
            assert_eq!(servers.len(), 1);
            assert_eq!(servers[0].name, "Stoat Server");
            assert_eq!(channels.len(), 1);
            assert_eq!(channels[0].name.as_deref(), Some("general"));
        }
        _ => panic!("Expected Ready message"),
    }

    // 2. Resilient Ready fallback with extra/missing fields and alternative type naming
    let malformed_ready = r#"{
        "type": "Ready",
        "users": [
            { "_id": "u2", "username": "bob", "extra_garbage_field": true },
            { "_id": "u3" }
        ],
        "servers": [
            { "_id": "s2", "name": "Resilient Server", "unexpected_field": 42 }
        ],
        "channels": [
            { "_id": "c2", "type": "TextChannel", "name": "dev", "server": "s2" },
            { "_id": "c3", "channel_type": "VoiceChannel" }
        ],
        "members": [
            { "_id": { "server": "s2", "user": "u2" }, "nickname": "Bobby" }
        ]
    }"#;

    let res_msg = ServerMessage::parse_resilient(malformed_ready).expect("Failed to parse resilient Ready");
    match res_msg {
        ServerMessage::Ready { users, servers, channels, members } => {
            assert_eq!(users.len(), 2);
            assert_eq!(users[0].id, "u2");
            assert_eq!(users[0].username, "bob");
            assert_eq!(users[1].id, "u3");

            assert_eq!(servers.len(), 1);
            assert_eq!(servers[0].id, "s2");
            assert_eq!(servers[0].name, "Resilient Server");

            assert_eq!(channels.len(), 2);
            assert_eq!(channels[0].id, "c2");
            assert_eq!(channels[0].channel_type, "TextChannel");
            assert_eq!(channels[0].name.as_deref(), Some("dev"));

            assert_eq!(members.len(), 1);
            assert_eq!(members[0].id.server, "s2");
            assert_eq!(members[0].id.user, "u2");
        }
        _ => panic!("Expected Ready message from resilient parser"),
    }
}

#[tokio::test]
async fn test_fast_keyboard_navigation_logic() {
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let (cfg, _guard) = isolated_config(None);
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.load_demo_data();

    // Verify initial demo state
    assert_eq!(app.state.servers.len(), 2);
    let s0_id = app.state.servers[0].id.clone();
    let s1_id = app.state.servers[1].id.clone();

    // Switch to first server
    app.switch_to_server(Some(s0_id.clone())).await;
    assert_eq!(app.state.active_server_id.as_deref(), Some(s0_id.as_str()));

    // Test channel cycling in current server
    let s0_channels = app.state.current_channel_ids();
    assert!(s0_channels.len() >= 2);
    let first_ch = s0_channels[0].clone();
    let second_ch = s0_channels[1].clone();

    assert_eq!(app.state.active_channel_id.as_deref(), Some(first_ch.as_str()));

    // Alt+j / Next channel
    app.nav_next_channel().await;
    assert_eq!(app.state.active_channel_id.as_deref(), Some(second_ch.as_str()));

    // Alt+k / Prev channel
    app.nav_prev_channel().await;
    assert_eq!(app.state.active_channel_id.as_deref(), Some(first_ch.as_str()));

    // Ctrl+Alt+j / Next server
    app.nav_next_server().await;
    assert_eq!(app.state.active_server_id.as_deref(), Some(s1_id.as_str()));

    // Ctrl+Alt+k / Prev server (back to first server)
    app.nav_prev_server().await;
    assert_eq!(app.state.active_server_id.as_deref(), Some(s0_id.as_str()));

    // Unread navigation test: clear and mark second channel in s0 with unread count
    app.state.unread_counts.clear();
    app.state.unread_counts.insert(second_ch.clone(), 3);
    assert_eq!(app.get_unread_channels_ordered().len(), 1);

    // Alt+Shift+j / Jump to next unread
    app.nav_next_unread().await;
    assert_eq!(app.state.active_channel_id.as_deref(), Some(second_ch.as_str()));
    // Unread count cleared on switch
    assert_eq!(app.state.unread_counts.get(&second_ch), None);
}

#[tokio::test]
async fn test_startup_auth_modal_and_session_check() {
    use stoat_tui::state::GatewayStatus;

    // Case 1: Token is None
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let (config_none, _g1) = isolated_config(None);
    let (tx1, _rx1) = tokio::sync::mpsc::channel(64);
    let mut app_none = stoat_tui::app::App::new(config_none, tx1, picker.clone());
    app_none.startup();
    assert!(app_none.state.show_login_modal);
    assert!(matches!(app_none.state.gateway_status, GatewayStatus::Disconnected(_)));
    assert_eq!(
        app_none.state.status_message.as_ref().map(|(msg, _)| msg.as_str()),
        Some("Please log in to connect")
    );

    // Case 2: Token is whitespace / empty
    let (config_empty, _g2) = isolated_config(Some("   "));
    let (tx2, _rx2) = tokio::sync::mpsc::channel(64);
    let mut app_empty = stoat_tui::app::App::new(config_empty, tx2, picker.clone());
    app_empty.startup();
    assert!(app_empty.state.show_login_modal);
    assert!(matches!(app_empty.state.gateway_status, GatewayStatus::Disconnected(_)));

    // Case 3: Token is valid non-empty string
    let (config_token, _g3) = isolated_config(Some("valid_secret_token"));
    let (tx3, mut rx3) = tokio::sync::mpsc::channel(64);
    let mut app_token = stoat_tui::app::App::new(config_token, tx3, picker.clone());
    app_token.startup();
    assert!(!app_token.state.show_login_modal);
    assert!(matches!(app_token.state.gateway_status, GatewayStatus::Connecting));
    assert_eq!(
        app_token.state.status_message.as_ref().map(|(msg, _)| msg.as_str()),
        Some("Connecting...")
    );

    // The spawned task will attempt get_current_user and eventually send an event
    let event = tokio::time::timeout(std::time::Duration::from_millis(500), rx3.recv()).await;
    assert!(event.is_ok());
}

#[tokio::test]
async fn test_manual_login_modal_keybindings_f2_ctrl_l() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (config, _guard) = isolated_config(Some("test_tok"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(config, tx, picker);

    // Initial state: token present, login modal closed
    app.state.show_login_modal = false;

    // Press F2 -> opens login modal
    let f2_key = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
    app.handle_event(stoat_tui::event::AppEvent::Key(f2_key)).await;
    assert!(app.state.show_login_modal);

    // Press Esc inside login modal -> closes login modal
    let esc_key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    app.handle_event(stoat_tui::event::AppEvent::Key(esc_key)).await;
    assert!(!app.state.show_login_modal);

    // Press Alt+Shift+L -> opens login modal (Ctrl+L is reserved for sidebar expand)
    let alt_shift_l = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::ALT | KeyModifiers::SHIFT);
    app.handle_event(stoat_tui::event::AppEvent::Key(alt_shift_l)).await;
    assert!(app.state.show_login_modal);
}

#[tokio::test]
async fn test_api_session_expired_event() {
    use stoat_tui::api::models::User;
    use stoat_tui::event::AppEvent;
    use stoat_tui::state::GatewayStatus;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (config, guard) = isolated_config(Some("expired_tok_123"));
    config.save().expect("Save initial temp config");
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(config, tx, picker);

    app.state.current_user = Some(User {
        id: "u_abc".to_string(),
        username: "testuser".to_string(),
        display_name: Some("Test User".to_string()),
        avatar: None,
        status: None,
        bot: None,
        online: Some(true),
        ..Default::default()
    });
    app.state.show_login_modal = false;

    // Dispatch ApiSessionExpired
    app.handle_event(AppEvent::ApiSessionExpired).await;

    // Token is NOT wiped from config or disk on session expired
    assert_eq!(app.state.config.token.as_deref(), Some("expired_tok_123"));
    let reloaded = Config::load_from(&guard.path);
    assert_eq!(reloaded.token.as_deref(), Some("expired_tok_123"));
    // Current user reset
    assert!(app.state.current_user.is_none());
    // Login modal auto-shown
    assert!(app.state.show_login_modal);
    // Error message displayed
    assert_eq!(
        app.state.login_error.as_deref(),
        Some("Session expired, please log in again.")
    );
    // Gateway status updated
    assert!(matches!(app.state.gateway_status, GatewayStatus::Disconnected(_)));
}

#[tokio::test]
async fn test_ready_and_authenticated_gateway_sync() {
    use stoat_tui::gateway::GatewayEvent;
    use stoat_tui::state::GatewayStatus;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let (cfg, _guard) = isolated_config(None);
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);

    // Authenticated event sets Connected with latency and ● Online
    app.handle_event(stoat_tui::event::AppEvent::Gateway(GatewayEvent::Authenticated)).await;
    match app.state.gateway_status {
        GatewayStatus::Connected { latency_ms } => assert!(latency_ms > 0),
        _ => panic!("Expected Connected gateway status"),
    }
    assert_eq!(
        app.state.status_message.as_ref().map(|(msg, _)| msg.as_str()),
        Some("● Online")
    );
}

#[test]
fn test_logger_file_output() {
    use std::fs;
    let path = stoat_tui::logger::init();
    assert!(path.exists() || path.parent().map(|p| p.exists()).unwrap_or(false));

    stoat_tui::log_info!("Test log entry from unit test");
    stoat_tui::log_warn!("Test warning entry");
    stoat_tui::log_error!("Test error entry");
    stoat_tui::log_debug!("Test debug entry");

    assert!(path.exists());
    let content = fs::read_to_string(&path).expect("Read debug log file");
    assert!(content.contains("[INFO] Test log entry from unit test"));
    assert!(content.contains("[WARN] Test warning entry"));
    assert!(content.contains("[ERROR] Test error entry"));
    assert!(content.contains("[DEBUG] Test debug entry"));
}

#[tokio::test]
async fn test_user_resolution_and_background_fetch() {
    use stoat_tui::api::models::User;
    use stoat_tui::event::AppEvent;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let (cfg, _guard) = isolated_config(None);
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);

    // Initial query for unknown user triggers background fetch
    let user_opt = app.state.resolve_user("01HMISSINGUSER");
    assert!(user_opt.is_none());

    // Second query does not re-queue fetch because it is pending
    let user_opt2 = app.state.resolve_user("01HMISSINGUSER");
    assert!(user_opt2.is_none());

    // When the user arrives via ApiUserFetched event
    let fetched_user = User {
        id: "01HMISSINGUSER".to_string(),
        username: "real_user".to_string(),
        display_name: Some("Real Display Name".to_string()),
        ..Default::default()
    };
    app.handle_event(AppEvent::ApiUserFetched(fetched_user.clone())).await;

    // Now resolve_user returns the user immediately
    let resolved = app.state.resolve_user("01HMISSINGUSER");
    assert!(resolved.is_some());
    assert_eq!(resolved.unwrap().name(), "Real Display Name");
}

#[test]
fn test_dm_channel_name_resolution() {
    use std::collections::HashMap;
    use stoat_tui::api::models::{Channel, User};

    let mut users = HashMap::new();
    users.insert(
        "user_other".to_string(),
        User {
            id: "user_other".to_string(),
            username: "other_friend".to_string(),
            display_name: Some("Best Friend".to_string()),
            ..Default::default()
        },
    );

    let dm_channel = Channel {
        id: "dm_123".to_string(),
        channel_type: "DirectMessage".to_string(),
        recipients: Some(vec!["user_me".to_string(), "user_other".to_string()]),
        ..Default::default()
    };

    // Resolves to other participant's display name
    let name = dm_channel.display_name(&users, Some("user_me"));
    assert_eq!(name, "Best Friend");

    // Fallback if recipient not in cache
    let uncached_channel = Channel {
        id: "dm_456".to_string(),
        channel_type: "DirectMessage".to_string(),
        recipients: Some(vec!["user_me".to_string(), "01J999XYZ".to_string()]),
        ..Default::default()
    };
    let uncached_name = uncached_channel.display_name(&users, Some("user_me"));
    assert_eq!(uncached_name, "User 01J999");
}

#[test]
fn test_server_badge_abbreviations() {
    fn badge(name: &str) -> String {
        let words: Vec<&str> = name.split_whitespace().collect();
        if words.len() >= 2 {
            let first = words[0].chars().next().unwrap_or('?');
            let second = words[1].chars().next().unwrap_or('?');
            format!("{}{}", first, second).to_uppercase()
        } else if let Some(first_word) = words.first() {
            let chars: Vec<char> = first_word.chars().collect();
            if chars.len() >= 2 {
                format!("{}{}", chars[0], chars[1]).to_uppercase()
            } else if let Some(&c) = chars.first() {
                format!("{} ", c).to_uppercase()
            } else {
                "??".to_string()
            }
        } else {
            "??".to_string()
        }
    }

    assert_eq!(badge("Zero"), "ZE");
    assert_eq!(badge("Origin"), "OR");
    assert_eq!(badge("ZE"), "ZE");
    assert_eq!(badge("OR"), "OR");
    assert_eq!(badge("Stoat Official"), "SO");
    assert_eq!(badge("Rust Language"), "RL");
    assert_eq!(badge("A"), "A ");
}

#[test]
fn test_purge_mock_servers() {
    use stoat_tui::api::models::Server;

    let (cfg, _guard) = isolated_config(None);
    let mut state = AppState::new(cfg);
    state.load_demo_data();

    // Verify mock servers are initially present in demo state
    assert!(state.servers.iter().any(|s| s.id == "srv_stoat"));

    // Add a real server
    state.servers.push(Server {
        id: "01J_REAL_SERVER".to_string(),
        name: "Zero Guild".to_string(),
        channels: vec!["c_real".to_string()],
        ..Default::default()
    });

    // Purge mock servers
    state.clear_mock_data();

    assert!(!state.servers.iter().any(|s| s.id == "srv_stoat"));
    assert!(!state.servers.iter().any(|s| s.id == "srv_rust"));
    assert!(state.servers.iter().any(|s| s.id == "01J_REAL_SERVER"));
}

#[test]
fn test_gateway_resilience_and_heartbeat_ping() {
    use stoat_tui::gateway::messages::{ClientMessage, ServerMessage};

    // Heartbeat ping serialization
    let ping = ClientMessage::Ping { data: 0 };
    let json = serde_json::to_string(&ping).unwrap();
    assert_eq!(json, r#"{"type":"Ping","data":0}"#);

    // Resilient parsing of unknown/unhandled events (typing indicators, reactions, etc.)
    let typing_json = r#"{"type":"ChannelStartTyping","id":"c_123","user":"u_456"}"#;
    let msg = ServerMessage::parse_resilient(typing_json);
    assert!(msg.is_some());
    assert!(matches!(msg.unwrap(), ServerMessage::Unknown));

    let reaction_json = r#"{"type":"MessageReact","id":"m_1","channel_id":"c_1","user_id":"u_1","emoji_id":"👍"}"#;
    let msg_react = ServerMessage::parse_resilient(reaction_json);
    assert!(msg_react.is_some());
    assert!(matches!(msg_react.unwrap(), ServerMessage::Unknown));
}

#[test]
fn test_bonfire_url_construction() {
    use stoat_tui::gateway::build_gateway_url;

    // 1. Bare domain without path or query -> ensures trailing slash and ?format=json
    let u1 = build_gateway_url("wss://events.stoat.chat").unwrap();
    assert_eq!(u1.as_str(), "wss://events.stoat.chat/?format=json");

    // 2. Erroneous version=1 and missing path -> strips version=1, adds path slash, sets ?format=json
    let u2 = build_gateway_url("wss://events.stoat.chat?format=json&version=1").unwrap();
    assert_eq!(u2.as_str(), "wss://events.stoat.chat/?format=json");

    // 3. Trailing slash already present
    let u3 = build_gateway_url("wss://events.stoat.chat/").unwrap();
    assert_eq!(u3.as_str(), "wss://events.stoat.chat/?format=json");

    // 4. Subpath preserved with ?format=json
    let u4 = build_gateway_url("wss://api.revolt.chat/ws").unwrap();
    assert_eq!(u4.as_str(), "wss://api.revolt.chat/ws?format=json");
}

#[test]
fn test_config_token_serialization_and_persistence() {
    let mut config = Config::default();
    assert!(config.token.is_none());

    // When token is None, skip_serializing_if omits it
    let toml_none = toml::to_string_pretty(&config).unwrap();
    assert!(!toml_none.contains("token ="));

    // When token is Some, it must be serialized cleanly
    config.token = Some("test_session_token_12345".to_string());
    let toml_some = toml::to_string_pretty(&config).unwrap();
    assert!(toml_some.contains(r#"token = "test_session_token_12345""#));

    // Deserialization check
    let parsed: Config = toml::from_str(&toml_some).unwrap();
    assert_eq!(parsed.token.as_deref(), Some("test_session_token_12345"));
}

#[test]
fn test_config_last_navigation_persistence() {
    let mut config = Config::default();
    assert!(!config.last_in_dm);
    assert!(config.last_server_id.is_none());
    assert!(config.last_channel_id.is_none());

    // Update navigation fields
    config.last_in_dm = false;
    config.last_server_id = Some("srv_01J_RUST".to_string());
    config.last_channel_id = Some("chan_01J_DEV".to_string());

    let toml_str = toml::to_string_pretty(&config).unwrap();
    assert!(toml_str.contains(r#"last_server_id = "srv_01J_RUST""#));
    assert!(toml_str.contains(r#"last_channel_id = "chan_01J_DEV""#));

    let parsed: Config = toml::from_str(&toml_str).unwrap();
    assert!(!parsed.last_in_dm);
    assert_eq!(parsed.last_server_id.as_deref(), Some("srv_01J_RUST"));
    assert_eq!(parsed.last_channel_id.as_deref(), Some("chan_01J_DEV"));

    // DM navigation
    config.last_in_dm = true;
    config.last_server_id = None;
    config.last_channel_id = Some("dm_01J_ALICE".to_string());

    let toml_dm = toml::to_string_pretty(&config).unwrap();
    assert!(toml_dm.contains("last_in_dm = true"));
    assert!(!toml_dm.contains("last_server_id ="));
    assert!(toml_dm.contains(r#"last_channel_id = "dm_01J_ALICE""#));

    let parsed_dm: Config = toml::from_str(&toml_dm).unwrap();
    assert!(parsed_dm.last_in_dm);
    assert!(parsed_dm.last_server_id.is_none());
    assert_eq!(parsed_dm.last_channel_id.as_deref(), Some("dm_01J_ALICE"));
}

#[test]
fn test_image_cache_lru_and_pending() {
    use stoat_tui::ui::images::ImageCache;
    use std::sync::Arc;
    use image::RgbaImage;

    let picker = Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut cache = ImageCache::new(picker);

    // Initial state
    assert!(!cache.contains("img_1"));
    assert!(!cache.is_pending("img_1"));

    // Mark pending
    assert!(cache.mark_pending("img_1".to_string()));
    assert!(cache.is_pending("img_1"));
    // Re-marking pending returns false
    assert!(!cache.mark_pending("img_1".to_string()));

    // Insert decoded image
    let dyn_img = image::DynamicImage::ImageRgba8(RgbaImage::new(16, 16));
    cache.insert("img_1".to_string(), dyn_img);

    assert!(cache.contains("img_1"));
    assert!(!cache.is_pending("img_1"));
    assert!(cache.get_mut("img_1").is_some());
}

#[tokio::test]
async fn test_login_modal_reset_on_f2_and_account_switch_cleanup() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use stoat_tui::api::models::{Channel, Message, Server};
    use stoat_tui::event::AppEvent;
    use stoat_tui::state::LoginStep;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (config, _guard) = isolated_config(Some("old_token"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(config, tx, picker);

    // Populate state with existing session data
    app.state.servers.push(Server { id: "srv_old".to_string(), name: "Old Server".to_string(), ..Default::default() });
    app.state.channels.insert("ch_old".to_string(), Channel { id: "ch_old".to_string(), name: Some("old-channel".to_string()), ..Default::default() });
    app.state.messages.insert("ch_old".to_string(), vec![Message { id: "m1".to_string(), ..Default::default() }]);
    app.state.login_step = LoginStep::TotpChallenge { ticket: "ticket_123".to_string() };
    app.state.login_error = Some("Previous error".to_string());
    app.state.login_totp_input = "123456".to_string();
    app.state.login_password_input = "secret_password".to_string();

    // Trigger F2 to open login modal: state must be cleanly reset
    let f2_key = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
    app.handle_event(AppEvent::Key(f2_key)).await;

    assert!(app.state.show_login_modal);
    assert_eq!(app.state.login_step, LoginStep::Credentials);
    assert!(app.state.login_error.is_none());
    assert!(app.state.login_totp_input.is_empty());
    assert!(app.state.login_password_input.is_empty());

    // Switch account via ApiLoginSuccess
    app.handle_event(AppEvent::ApiLoginSuccess {
        token: "brand_new_token_456".to_string(),
        is_bot: false,
    }).await;

    // Previous data must be completely wiped
    assert!(app.state.servers.is_empty());
    assert!(app.state.channels.is_empty());
    assert!(app.state.messages.is_empty());
    assert!(!app.state.show_login_modal);
    assert_eq!(app.state.config.token.as_deref(), Some("brand_new_token_456"));
}

#[tokio::test]
async fn test_pending_attachment_and_clear_shortcuts() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use stoat_tui::event::AppEvent;
    use stoat_tui::state::{Focus, PendingAttachment};

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (config, _guard) = isolated_config(Some("tok_123"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(config, tx, picker);
    app.state.show_login_modal = false;
    app.state.focus = Focus::Input;

    // Simulate uploaded attachment event
    app.handle_event(AppEvent::AttachmentUploaded {
        id: "att_01J_FILE".to_string(),
        filename: "screenshot.png".to_string(),
    }).await;

    assert_eq!(app.state.pending_attachments.len(), 1);
    assert_eq!(app.state.pending_attachments[0].filename, "screenshot.png");

    // Backspace with empty input buffer removes the attachment
    let backspace_key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    app.handle_event(AppEvent::Key(backspace_key)).await;
    assert!(app.state.pending_attachments.is_empty());

    // Add back an attachment and test Esc clears it
    app.state.pending_attachments.push(PendingAttachment {
        id: "att_02J_TEST".to_string(),
        filename: "photo.jpg".to_string(),
    });
    let esc_key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    app.handle_event(AppEvent::Key(esc_key)).await;
    assert!(app.state.pending_attachments.is_empty());
}

#[tokio::test]
async fn test_no_mock_data_persisted_to_config() {
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (mut config, _guard) = isolated_config(Some("valid_token"));
    config.last_server_id = Some("srv_REAL".to_string());
    config.last_channel_id = Some("chan_REAL".to_string());

    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(config, tx, picker);
    app.state.load_demo_data();

    // In demo mode or with mock IDs, switch_to_channel should NOT update config navigation
    app.switch_to_channel("ch_stoat_general".to_string()).await;
    assert_eq!(app.state.config.last_server_id.as_deref(), Some("srv_REAL"));
    assert_eq!(app.state.config.last_channel_id.as_deref(), Some("chan_REAL"));

    // switch_to_server with mock server ID should NOT update config navigation
    app.switch_to_server(Some("srv_stoat".to_string())).await;
    assert_eq!(app.state.config.last_server_id.as_deref(), Some("srv_REAL"));
    assert_eq!(app.state.config.last_channel_id.as_deref(), Some("chan_REAL"));
}

#[tokio::test]
async fn test_left_sidebar_width_resizing_and_member_rail_toggle() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use stoat_tui::event::AppEvent;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (mut config, _guard) = isolated_config(Some("valid_token"));
    config.left_sidebar_width = 26;

    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(config, tx, picker);
    app.state.show_login_modal = false;

    assert_eq!(app.state.left_sidebar_width, 26);

    // Ctrl+H shrinks sidebar by 2 (min 16)
    let ctrl_h = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);
    app.handle_event(AppEvent::Key(ctrl_h)).await;
    assert_eq!(app.state.left_sidebar_width, 24);
    assert_eq!(app.state.config.left_sidebar_width, 24);

    // Ctrl+L expands sidebar by 2 (max 52)
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
    app.handle_event(AppEvent::Key(ctrl_l)).await;
    assert_eq!(app.state.left_sidebar_width, 26);
    assert_eq!(app.state.config.left_sidebar_width, 26);

    // Test Alt+M toggles member rail when not in a call
    assert!(app.state.show_member_rail);
    let alt_m = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT);
    app.handle_event(AppEvent::Key(alt_m)).await;
    assert!(!app.state.show_member_rail);
    app.handle_event(AppEvent::Key(alt_m)).await;
    assert!(app.state.show_member_rail);
}

#[test]
fn test_config_sidebar_width_serde() {
    let toml_str = r#"
api_url = "https://api.stoat.chat"
token = "tok_abc"
left_sidebar_width = 34
"#;
    let config: Config = toml::from_str(toml_str).expect("Failed to deserialize config with left_sidebar_width");
    assert_eq!(config.left_sidebar_width, 34);

    let serialized = toml::to_string(&config).expect("Failed to serialize config");
    assert!(serialized.contains("left_sidebar_width = 34"));
}

#[tokio::test]
async fn test_fetch_me_sends_session_token_header() {
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    let server_handle = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);

            // Assert that the request contains the expected x-session-token header
            assert!(
                req.contains("x-session-token: valid_session_token_xyz")
                    || req.contains("x-session-token:valid_session_token_xyz"),
                "Request headers missing x-session-token: {}",
                req
            );

            let body = r#"{"_id":"01JUSER","username":"stoatuser","online":true}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(server_url, Some("valid_session_token_xyz".to_string()), false);
    let user = client.fetch_me("valid_session_token_xyz").await.expect("fetch_me should succeed");

    assert_eq!(user.id, "01JUSER");
    assert_eq!(user.username, "stoatuser");

    server_handle.await.unwrap();
}

#[test]
fn test_attachment_resilient_deserialization_and_autumn_urls() {
    use stoat_tui::api::models::{Attachment, Server, User};
    use stoat_tui::state::AppState;

    // Minimal Attachment JSON missing optional fields (filename, metadata, deleted, reported)
    let user_json = r#"{
        "_id": "01JUSER123",
        "username": "alice",
        "avatar": {
            "_id": "01JAVATAR999",
            "tag": "avatars"
        }
    }"#;
    let user: User = serde_json::from_str(user_json).expect("User with minimal avatar should deserialize");
    let av: &Attachment = user.avatar.as_ref().expect("User.avatar must not be dropped");
    assert_eq!(av.id, "01JAVATAR999");

    // Minimal Server icon JSON
    let server_json = r#"{
        "_id": "01JSRV123",
        "name": "Zero",
        "icon": {
            "_id": "01JICON777",
            "deleted": false,
            "reported": false
        }
    }"#;
    let srv: Server = serde_json::from_str(server_json).expect("Server with minimal icon should deserialize");
    assert_eq!(srv.icon.as_ref().unwrap().id, "01JICON777");

    let (mut state_cfg, _guard) = isolated_config(None);
    state_cfg.api_url = "https://api.stoat.chat/".to_string();
    let mut state = AppState::new(state_cfg);
    state.autumn_url = "https://cdn.example.com".to_string();

    assert_eq!(
        state.avatar_url_for_user(&user),
        "https://cdn.example.com/avatars/01JAVATAR999?max_side=128"
    );

    let user_no_av = User {
        id: "01JNOAV".to_string(),
        username: "bob".to_string(),
        ..Default::default()
    };
    assert_eq!(
        state.avatar_url_for_user(&user_no_av),
        "https://api.stoat.chat/users/01JNOAV/default_avatar"
    );

    assert_eq!(
        state.server_icon_url(&srv).as_deref(),
        Some("https://cdn.example.com/icons/01JICON777?max_side=128")
    );
}

#[tokio::test]
async fn test_autumn_url_resolved_event_and_sidebar_pfp_enqueue() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::api::models::{Channel, FileObject, Server, User};
    use stoat_tui::event::AppEvent;
    use stoat_tui::state::ActiveRail;
    use stoat_tui::ui;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (config, _guard) = isolated_config(Some("tok_123"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(config, tx, picker);
    app.state.show_login_modal = false;

    app.handle_event(AppEvent::AutumnUrlResolved("https://autumn.stoat.chat/".to_string())).await;
    assert_eq!(app.state.autumn_url, "https://autumn.stoat.chat");

    // Populate a DM and a server with an icon, and insert a cached image for the DM user
    app.state.current_user = Some(User {
        id: "me_1".to_string(),
        username: "me".to_string(),
        ..Default::default()
    });
    app.state.users.insert(
        "friend_1".to_string(),
        User {
            id: "friend_1".to_string(),
            username: "friend_alice".to_string(),
            avatar: Some(FileObject {
                id: "av_friend_1".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    app.state.dms.push(Channel {
        id: "dm_1".to_string(),
        channel_type: "DirectMessage".to_string(),
        recipients: Some(vec!["me_1".to_string(), "friend_1".to_string()]),
        ..Default::default()
    });
    app.state.servers.push(Server {
        id: "srv_1".to_string(),
        name: "Zero".to_string(),
        icon: Some(FileObject {
            id: "ic_srv_1".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    app.state.active_rail = ActiveRail::DirectMessages;
    app.state.active_server_id = None;

    // Insert a tiny 2x2 image for friend_1 into cache so deferred pass renders it via StatefulImage
    let img = image::DynamicImage::new_rgba8(4, 4);
    if let Some(ref mut cache) = app.state.image_cache {
        cache.insert("avatar_friend_1".to_string(), img);
    }

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| ui::render(f, &mut app.state))
        .expect("Render with DM PFPs and server icons should succeed");

    // After render, image_fetch_tx should have marked server_icon_srv_1 as pending
    let cache = app.state.image_cache.as_ref().unwrap();
    assert!(cache.is_pending("server_icon_srv_1"));
}

#[test]
fn test_config_custom_path_and_token_preservation() {
    let (cfg, guard) = isolated_config(Some("disk_secret_token"));
    cfg.save_to(&guard.path).expect("save_to should succeed");

    let loaded = Config::load_from(&guard.path);
    assert_eq!(loaded.token.as_deref(), Some("disk_secret_token"));
    assert_eq!(loaded.custom_path.as_deref(), Some(guard.path.as_path()));

    // Saving a config with token: None and explicit_token_cleared == false must preserve disk token
    let mut cfg_without_token = Config {
        token: None,
        left_sidebar_width: 30,
        custom_path: Some(guard.path.clone()),
        ..Default::default()
    };
    cfg_without_token.save().expect("save via custom_path should succeed");

    let after_preserve = Config::load_from(&guard.path);
    assert_eq!(after_preserve.token.as_deref(), Some("disk_secret_token"));
    assert_eq!(after_preserve.left_sidebar_width, 30);

    // Explicit clear_token() removes token from disk on save
    cfg_without_token.clear_token();
    cfg_without_token.save().expect("save after clear_token should succeed");

    let after_clear = Config::load_from(&guard.path);
    assert!(after_clear.token.is_none());
}

#[test]
fn test_config_env_dir_override() {
    let prev = std::env::var("STOAT_TUI_CONFIG_DIR").ok();
    let custom_dir = std::env::temp_dir().join(format!("stoat-tui-env-test-{}", std::process::id()));
    std::env::set_var("STOAT_TUI_CONFIG_DIR", &custom_dir);

    let resolved = Config::config_path();
    assert_eq!(resolved, custom_dir.join("config.toml"));

    if let Some(val) = prev {
        std::env::set_var("STOAT_TUI_CONFIG_DIR", val);
    } else {
        std::env::remove_var("STOAT_TUI_CONFIG_DIR");
    }
}

#[test]
fn test_metadata_livekit_and_voice_feature_parsing() {
    use stoat_tui::api::models::ServerMetadata;

    let json_livekit = r#"{
        "ws": "wss://events.stoat.chat",
        "features": {
            "autumn": { "enabled": true, "url": "https://cdn.stoatusercontent.com" },
            "livekit": { "enabled": true, "url": "wss://livekit.stoat.chat" }
        }
    }"#;
    let meta1: ServerMetadata = serde_json::from_str(json_livekit).unwrap();
    let lk1 = meta1
        .features
        .as_ref()
        .and_then(|f| f.livekit.as_ref().or(f.voice.as_ref()))
        .and_then(|lk| lk.resolved_url());
    assert_eq!(lk1.as_deref(), Some("wss://livekit.stoat.chat"));

    let json_voice_nodes = r#"{
        "ws": "wss://events.stoat.chat",
        "features": {
            "voice": {
                "enabled": true,
                "nodes": [ { "name": "eu-west", "url": "wss://voice-eu.stoat.chat" } ]
            }
        }
    }"#;
    let meta2: ServerMetadata = serde_json::from_str(json_voice_nodes).unwrap();
    let lk2 = meta2
        .features
        .as_ref()
        .and_then(|f| f.livekit.as_ref().or(f.voice.as_ref()))
        .and_then(|lk| lk.resolved_url());
    assert_eq!(lk2.as_deref(), Some("wss://voice-eu.stoat.chat"));
}

#[tokio::test]
async fn test_join_call_and_ring_channel_http_signaling() {
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    let server_handle = tokio::spawn(async move {
        // Expect 2 requests: POST /channels/ch_dm_1/join_call, then POST /channels/ch_dm_1/ring
        for expected_path in ["/channels/ch_dm_1/join_call", "/channels/ch_dm_1/ring"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains(&format!("POST {} ", expected_path)),
                "Unexpected request path: {}",
                req
            );
            assert!(
                req.contains("x-session-token: call_tok_123")
                    || req.contains("x-session-token:call_tok_123"),
                "Missing x-session-token header: {}",
                req
            );
            let expected_body = if expected_path.ends_with("/join_call") {
                r#"{"node":"worldwide"}"#
            } else {
                "{}"
            };
            assert!(
                req.to_ascii_lowercase().contains("content-type: application/json")
                    && req.ends_with(expected_body),
                "Expected Content-Type: application/json and {} body, got: {}",
                expected_body,
                req
            );

            let (status_line, body) = if expected_path.ends_with("/join_call") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"token":"eyJ_mock_jwt_secret","url":"wss://lk.example.com"}"#,
                )
            } else {
                ("HTTP/1.1 204 No Content", "")
            };
            let resp = format!(
                "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_line,
                body.len(),
                body
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(server_url, Some("call_tok_123".to_string()), false);
    let join_res = client.join_call("ch_dm_1", None).await.expect("join_call should succeed");
    assert_eq!(join_res.token, "eyJ_mock_jwt_secret");
    assert_eq!(join_res.url.as_deref(), Some("wss://lk.example.com"));

    client.ring_channel("ch_dm_1").await;
    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_call_events_lifecycle_and_active_speakers() {
    use stoat_tui::api::models::User;
    use stoat_tui::event::AppEvent;
    use stoat_tui::voice::{ActiveCallState, CallStatus};

    std::env::set_var("STOAT_TUI_DISABLE_AUDIO", "1");

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_call"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);

    app.state.current_user = Some(User {
        id: "u_me".to_string(),
        username: "me".to_string(),
        ..Default::default()
    });

    // Resolve LiveKit node & URL
    app.handle_event(AppEvent::LivekitNodeResolved {
        node_name: "worldwide".to_string(),
        url: Some("wss://livekit.stoat.chat".to_string()),
    }).await;
    assert_eq!(app.state.livekit_node_name.as_deref(), Some("worldwide"));
    assert_eq!(app.state.livekit_url.as_deref(), Some("wss://livekit.stoat.chat"));

    // Start call in Connecting state
    app.state.active_call = Some(ActiveCallState::new("ch_voice_1".to_string()));
    assert_eq!(app.state.active_call.as_ref().unwrap().status, CallStatus::Connecting);

    // Connect succeeds
    app.handle_event(AppEvent::CallConnected { channel_id: "ch_voice_1".to_string() }).await;
    assert_eq!(app.state.active_call.as_ref().unwrap().status, CallStatus::Connected);

    // Remote participant joins & speaks
    app.handle_event(AppEvent::CallParticipantJoined {
        channel_id: "ch_voice_1".to_string(),
        user_id: "u_friend".to_string(),
        is_local: false,
    }).await;
    assert_eq!(app.state.active_call.as_ref().unwrap().participants.len(), 1);

    app.handle_event(AppEvent::CallActiveSpeakers {
        channel_id: "ch_voice_1".to_string(),
        speaker_ids: vec!["u_me".to_string(), "u_friend".to_string()],
    }).await;
    assert!(app.state.active_call.as_ref().unwrap().local_speaking);
    assert!(app.state.active_call.as_ref().unwrap().participants[0].is_speaking);

    // Remote participant leaves
    app.handle_event(AppEvent::CallParticipantLeft {
        channel_id: "ch_voice_1".to_string(),
        user_id: "u_friend".to_string(),
    }).await;
    assert!(app.state.active_call.as_ref().unwrap().participants.is_empty());

    // Call failure closes active_call and displays error status
    app.handle_event(AppEvent::CallFailed {
        channel_id: "ch_voice_1".to_string(),
        error: "SFU unreachable".to_string(),
    }).await;
    assert!(app.state.active_call.is_none());
    let (msg, _) = app.state.status_message.as_ref().unwrap();
    assert!(msg.contains("Call failed: SFU unreachable"));
}

#[test]
fn test_voice_resampler_and_synthesized_call_sounds() {
    use stoat_tui::voice::sounds::{
        synthesize_deafen_sound, synthesize_join_sound, synthesize_leave_sound,
        synthesize_mute_sound, SOUND_SAMPLE_RATE,
    };
    use stoat_tui::voice::Mono48kResampler;

    // 1. Resampler from 44.1kHz -> 48kHz (100ms = 4410 -> ~4800 samples)
    let mut resampler = Mono48kResampler::new(44_100);
    let input_44k: Vec<f32> = (0..4410)
        .map(|i| (2.0 * std::f32::consts::PI * 440.0 * (i as f32 / 44_100.0)).sin() * 0.5)
        .collect();
    let mut out_48k = Vec::new();
    resampler.push(&input_44k, &mut out_48k);
    assert!((4795..=4805).contains(&out_48k.len()), "Expected ~4800 samples, got {}", out_48k.len());

    // 2. Synthesized sound effects
    let join = synthesize_join_sound();
    let leave = synthesize_leave_sound();
    let mute_on = synthesize_mute_sound(true);
    let mute_off = synthesize_mute_sound(false);
    let deaf_on = synthesize_deafen_sound(true);
    let deaf_off = synthesize_deafen_sound(false);

    // Join & leave ~180ms at 48kHz (~8640 samples)
    assert!((8000..=9200).contains(&join.len()));
    assert!((8000..=9200).contains(&leave.len()));
    // Mute ~80ms at 48kHz (~3840 samples)
    assert!((3500..=4200).contains(&mute_on.len()));
    assert!((3500..=4200).contains(&mute_off.len()));
    // Deafen ~110ms at 48kHz (~5280 samples)
    assert!((4800..=5800).contains(&deaf_on.len()));
    assert!((4800..=5800).contains(&deaf_off.len()));
    assert_eq!(SOUND_SAMPLE_RATE, 48_000);

    for buf in [&join, &leave, &mute_on, &mute_off, &deaf_on, &deaf_off] {
        assert!(buf.iter().all(|&s| s.is_finite() && (-1.0..=1.0).contains(&s)));
        assert!(buf.iter().any(|&s| s.abs() > 0.05));
    }
}

#[tokio::test]
async fn test_sanitize_status_text_and_join_call_fallback() {
    use stoat_tui::api::DeltaClient;
    use stoat_tui::ui::components::status_bar::sanitize_status_text;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 1. Multi-line HTML error sanitization
    let raw_html_err = "❌ Call failed: Join call error (HTTP 400 Bad Request): <!DOCTYPE html>\r\n<html>\n  <head><title>400 Bad Request</title></head>\n  <body><h1>400 Bad Request</h1></body>\n</html>";
    let clean = sanitize_status_text(raw_html_err);
    assert!(!clean.contains('\n'));
    assert!(!clean.contains('\r'));
    assert!(!clean.to_ascii_lowercase().contains("<!doctype"));
    assert!(!clean.to_ascii_lowercase().contains("<html>"));
    assert!(clean.contains("Unexpected HTML response from server (check debug.log)"));
    assert!(clean.starts_with("❌ Call failed: Join call error (HTTP 400 Bad Request):"));

    let plain_multiline = "Line 1\r\n   Line 2\n\n  Line 3  ";
    assert_eq!(sanitize_status_text(plain_multiline), "Line 1 Line 2 Line 3");

    // 2. Fallback from 400 HTML on /join_call -> GET / metadata -> POST /call with {} JSON
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    let server_handle = tokio::spawn(async move {
        // Request 1: POST /channels/c1/join_call -> 400 HTML
        {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("POST /channels/c1/join_call "));
            assert!(req.ends_with(r#"{"node":"worldwide"}"#));
            let html = "<!DOCTYPE html><html><body>400 Bad Request</body></html>";
            let resp = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
        // Request 2: GET / -> metadata with livekit url
        {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("GET / "));
            let meta = r#"{"features":{"livekit":{"enabled":true,"url":"wss://meta-lk.example.com"}}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                meta.len(),
                meta
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
        // Request 3: POST /channels/c1/call -> 200 OK with token
        {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("POST /channels/c1/call "));
            assert!(req.ends_with(r#"{"node":"worldwide"}"#));
            let body = r#"{"token":"fallback_jwt_tok"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(server_url, Some("tok_fb".to_string()), false);
    let res = client.join_call("c1", None).await.expect("Fallback /call should succeed");
    assert_eq!(res.token, "fallback_jwt_tok");
    assert_eq!(res.url.as_deref(), Some("wss://meta-lk.example.com"));

    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_livekit_nodes_parsing_and_join_call_node_payload() {
    use stoat_tui::api::models::ServerMetadata;
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 1. Parse features.livekit.nodes with name, url, lat, lon and "id" alias
    let root_json = r#"{
        "features": {
            "livekit": {
                "enabled": true,
                "nodes": [
                    { "name": "europe", "url": "wss://europe.voice.stoat.chat", "lat": 50.1, "lon": 8.6 },
                    { "id": "us-east", "url": "wss://us.voice.stoat.chat", "lat": 39.0, "lon": -77.5 }
                ]
            }
        }
    }"#;
    let meta: ServerMetadata = serde_json::from_str(root_json).unwrap();
    let lk = meta.features.as_ref().unwrap().livekit.as_ref().unwrap();
    assert_eq!(lk.nodes.as_ref().unwrap().len(), 2);
    assert_eq!(lk.nodes.as_ref().unwrap()[1].name, "us-east");
    assert_eq!(lk.nodes.as_ref().unwrap()[0].lat, Some(50.1));
    assert_eq!(lk.nodes.as_ref().unwrap()[0].lon, Some(8.6));

    let (node_name, node_url) = lk.resolved_node();
    assert_eq!(node_name.as_deref(), Some("europe"));
    assert_eq!(node_url.as_deref(), Some("wss://europe.voice.stoat.chat"));

    // Empty nodes falls back to "worldwide"
    let empty_nodes_json = r#"{"features":{"livekit":{"enabled":true,"nodes":[]}}}"#;
    let meta_empty: ServerMetadata = serde_json::from_str(empty_nodes_json).unwrap();
    let (def_node, _) = meta_empty.features.unwrap().livekit.unwrap().resolved_node();
    assert_eq!(def_node.as_deref(), Some("worldwide"));

    // 2. Verify join_call sends {"node": "europe"} in POST body
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    let server_handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let n = sock.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        assert!(req.contains("POST /channels/ch_eu_1/join_call "));
        assert!(
            req.ends_with(r#"{"node":"europe"}"#),
            "Expected {{\"node\":\"europe\"}} payload, got: {}",
            req
        );

        let body = r#"{"token":"eu_jwt_token_abc"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
    });

    let client = DeltaClient::new(server_url, Some("sess_eu".to_string()), false);
    let join_res = client
        .join_call("ch_eu_1", node_name.as_deref())
        .await
        .expect("join_call with node_name should succeed");
    assert_eq!(join_res.token, "eu_jwt_token_abc");

    server_handle.await.unwrap();
}

#[test]
fn test_screen_capture_ppm_downscale_and_i420_roundtrip() {
    use livekit::webrtc::video_frame::{BoxVideoBuffer, VideoFrame, VideoRotation};
    use stoat_tui::voice::screen::{downscale_rgba_even, parse_ppm_p6};
    use stoat_tui::voice::{rgba_to_i420_buffer, video_frame_to_rgba};

    // 1. Parse PPM P6 with comment
    let mut ppm = b"P6\n# grim screenshot\n4 2\n255\n".to_vec();
    for _ in 0..8 {
        ppm.extend_from_slice(&[200, 40, 20]);
    }
    let (w, h, rgba) = parse_ppm_p6(&ppm).expect("Valid P6 PPM should parse");
    assert_eq!((w, h), (4, 2));
    assert_eq!(&rgba[0..4], &[200, 40, 20, 255]);

    // 2. Downscale 2560x1440 -> 1280x720 (even dims)
    let large = vec![128u8; 2560 * 1440 * 4];
    let (dw, dh, scaled) = downscale_rgba_even(&large, 2560, 1440, 1280, 720);
    assert_eq!((dw, dh), (1280, 720));
    assert_eq!(dw % 2, 0);
    assert_eq!(dh % 2, 0);
    assert_eq!(scaled.len(), 1280 * 720 * 4);

    // 4. RGBA -> I420 -> BoxVideoFrame -> RGBA roundtrip preserves dominant red channel
    let mut red_rgba = Vec::with_capacity(4 * 4 * 4);
    for _ in 0..16 {
        red_rgba.extend_from_slice(&[240, 10, 10, 255]);
    }
    let i420 = rgba_to_i420_buffer(&red_rgba, 4, 4).expect("Valid even 4x4 RGBA buffer should convert");
    let box_buf: BoxVideoBuffer = Box::new(i420);
    let vf = VideoFrame::new(VideoRotation::VideoRotation0, box_buf);
    let (rw, rh, out_rgba) = video_frame_to_rgba(&vf).expect("Roundtrip should succeed");
    assert_eq!((rw, rh), (4, 4));
    assert!(out_rgba[0] > 200 && out_rgba[1] < 40 && out_rgba[2] < 40, "Unexpected RGB: {:?}", &out_rgba[0..4]);
}

#[tokio::test]
async fn test_call_stage_video_streaming_and_screen_share_failed() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::api::models::User;
    use stoat_tui::event::AppEvent;
    use stoat_tui::ui;
    use stoat_tui::voice::{ActiveCallState, CallStatus, VideoFrameSnapshot};

    std::env::set_var("STOAT_TUI_DISABLE_AUDIO", "1");

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_vid"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.show_login_modal = false;

    app.state.users.insert(
        "u_sharer".to_string(),
        User {
            id: "u_sharer".to_string(),
            username: "sharer".to_string(),
            display_name: Some("Alice Stream".to_string()),
            ..Default::default()
        },
    );

    let mut call = ActiveCallState::new("ch_v1".to_string());
    call.status = CallStatus::Connected;
    call.connected_at = Some(std::time::Instant::now());
    call.screensharing.store(true, std::sync::atomic::Ordering::SeqCst);
    *call.latest_video_frame.lock().await = Some(VideoFrameSnapshot {
        image: image::DynamicImage::new_rgba8(16, 16),
        owner_id: "u_sharer".to_string(),
        fps: 20,
        is_remote: true,
    });
    app.state.active_call = Some(call);

    app.handle_event(AppEvent::VideoFrameReady).await;
    assert!(app.state.image_cache.as_ref().unwrap().contains("active_call_video"));

    // Render normal and fullscreen modes via TestBackend
    let backend = TestBackend::new(110, 34);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::render(f, &mut app.state)).expect("Normal render with video should succeed");

    // Check streaming badge text appears in buffer
    let buf_text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        buf_text.contains("STREAMING • 20fps • Alice Stream"),
        "Missing streaming badge in buffer"
    );

    // Test fullscreen render
    app.state.call_fullscreen = true;
    terminal.draw(|f| ui::render(f, &mut app.state)).expect("Fullscreen render with video should succeed");
    app.state.call_fullscreen = false;

    // Test `v` toggles local preview
    let v_key = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE);
    app.handle_event(AppEvent::Key(v_key)).await;
    assert!(!app.state.active_call.as_ref().unwrap().show_local_preview);

    // ScreenShareFailed resets screensharing flag and sets sanitized red status
    app.handle_event(AppEvent::ScreenShareFailed {
        channel_id: "ch_v1".to_string(),
        error: "Portal denied\n<html>err</html>".to_string(),
    }).await;
    assert!(!app.state.active_call.as_ref().unwrap().is_screensharing());
    let (status_msg, _) = app.state.status_message.as_ref().unwrap();
    assert!(status_msg.contains("Screen share failed:"));
    assert!(!status_msg.contains('\n'));
}

#[test]
fn test_tiny_10x10_render_all_views_and_modals() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::api::models::{Channel, Message, Server, User};
    use stoat_tui::state::{LoginStep, LoginTab};
    use stoat_tui::ui;
    use stoat_tui::voice::{ActiveCallState, CallStatus};

    let (cfg, _guard) = isolated_config(Some("tok_tiny"));
    let mut state = AppState::new(cfg);
    state.show_login_modal = false;

    state.users.insert(
        "u1".to_string(),
        User {
            id: "u1".to_string(),
            username: "alice".to_string(),
            display_name: Some("Alice".to_string()),
            ..Default::default()
        },
    );
    state.servers.push(Server {
        id: "srv1".to_string(),
        name: "Stoat".to_string(),
        channels: vec!["ch1".to_string()],
        ..Default::default()
    });
    state.channels.insert(
        "ch1".to_string(),
        Channel {
            id: "ch1".to_string(),
            channel_type: "TextChannel".to_string(),
            server: Some("srv1".to_string()),
            name: Some("general".to_string()),
            ..Default::default()
        },
    );
    state.server_channels.insert("srv1".to_string(), vec!["ch1".to_string()]);
    state.active_server_id = Some("srv1".to_string());
    state.active_rail = stoat_tui::state::ActiveRail::Server("srv1".to_string());
    state.active_channel_id = Some("ch1".to_string());
    state.messages.insert(
        "ch1".to_string(),
        vec![Message {
            id: "01J00000000000000000000001".to_string(),
            channel: "ch1".to_string(),
            author: "u1".to_string(),
            content: Some("Hello tiny!".to_string()),
            ..Default::default()
        }],
    );

    let read_buf = |term: &Terminal<TestBackend>| -> String {
        term.backend().buffer().content().iter().map(|c| c.symbol()).collect()
    };

    // 1. 10x10 Chat
    state.focus = Focus::Chat;
    let mut term = Terminal::new(TestBackend::new(10, 10)).unwrap();
    term.draw(|f| ui::render(f, &mut state)).unwrap();
    let txt = read_buf(&term);
    assert!(txt.contains("#general") || txt.contains("#genera"), "Header missing at 10x10: {}", txt);
    assert!(txt.contains("Hello"), "Message missing at 10x10 Chat: {}", txt);

    // 2. 10x10 Channels
    state.focus = Focus::Channels;
    term.draw(|f| ui::render(f, &mut state)).unwrap();
    let txt = read_buf(&term);
    assert!(txt.contains("general") || txt.contains("Stoat"), "Channel missing at 10x10: {}", txt);

    // 3. 10x10 Servers
    state.focus = Focus::Servers;
    term.draw(|f| ui::render(f, &mut state)).unwrap();
    let txt = read_buf(&term);
    assert!(txt.contains("@") && txt.contains("ST"), "Server badge missing at 10x10: {}", txt);

    // 4. 10x10 Input
    state.focus = Focus::Input;
    state.input_buffer = "hi!".to_string();
    state.input_cursor = 3;
    term.draw(|f| ui::render(f, &mut state)).unwrap();
    let txt = read_buf(&term);
    assert!(txt.contains("hi!"), "Input text missing at 10x10: {}", txt);

    // 5. 10x10 Active Call (split + fullscreen)
    state.focus = Focus::Chat;
    let mut call = ActiveCallState::new("ch1".to_string());
    call.status = CallStatus::Connected;
    call.connected_at = Some(std::time::Instant::now());
    state.active_call = Some(call);
    term.draw(|f| ui::render(f, &mut state)).unwrap();
    let txt = read_buf(&term);
    assert!(txt.contains("📞"), "Call icon missing at 10x10: {}", txt);
    state.call_fullscreen = true;
    term.draw(|f| ui::render(f, &mut state)).unwrap();
    state.call_fullscreen = false;
    state.active_call = None;

    // 6. 10x10 Login Modal (all tabs + TOTP)
    state.show_login_modal = true;
    for tab in [LoginTab::EmailPassword, LoginTab::Token, LoginTab::Demo] {
        state.login_tab = tab;
        term.draw(|f| ui::render(f, &mut state)).unwrap();
        let txt = read_buf(&term);
        assert!(txt.contains("Login"), "Login modal missing at 10x10: {}", txt);
    }
    state.login_step = LoginStep::TotpChallenge { ticket: "t".to_string() };
    term.draw(|f| ui::render(f, &mut state)).unwrap();
    assert!(read_buf(&term).contains("2FA"));
    state.show_login_modal = false;

    // 7. 10x10 Command Palette Modal
    state.show_command_palette = true;
    state.command_palette_query = "gen".to_string();
    term.draw(|f| ui::render(f, &mut state)).unwrap();
    assert!(read_buf(&term).contains("/gen"));
    state.show_command_palette = false;

    // 8. Extreme tiny edge sizes: 0x0, 1x1, 10x4 (height < 6 pure pane mode)
    for (w, h) in [(0, 0), (1, 1), (10, 4)] {
        let mut tiny_term = Terminal::new(TestBackend::new(w, h)).unwrap();
        state.focus = Focus::Chat;
        tiny_term.draw(|f| ui::render(f, &mut state)).unwrap();
    }

    // 9. Medium Mode (width = 60, height = 24): right member rail auto-hidden and center >= 60%
    let mut med_term = Terminal::new(TestBackend::new(60, 24)).unwrap();
    med_term.draw(|f| ui::render(f, &mut state)).unwrap();
    assert!(state.rect_chat.width >= 36, "Center width {} must be >= 60% of 60", state.rect_chat.width);
    assert_eq!(state.rect_members.width, 0, "Member rail should auto-hide in medium mode unless toggled");
}

#[tokio::test]
async fn test_nav_cache_hydration_and_tiny_pane_switching() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use stoat_tui::api::models::{Channel, Server, User};
    use stoat_tui::config::NavCache;
    use stoat_tui::event::AppEvent;

    let (mut cfg, _guard) = isolated_config(Some("tok_cache"));
    cfg.last_server_id = Some("s_cache".to_string());
    cfg.last_channel_id = Some("c_cache".to_string());
    let cache_path = cfg.nav_cache_path();

    let nav = NavCache {
        api_url: cfg.api_url.clone(),
        ws_url: Some("wss://events.cached.test".to_string()),
        autumn_url: Some("https://cdn.cached.test".to_string()),
        servers: vec![Server {
            id: "s_cache".to_string(),
            name: "CachedGuild".to_string(),
            channels: vec!["c_cache".to_string()],
            ..Default::default()
        }],
        channels: vec![Channel {
            id: "c_cache".to_string(),
            channel_type: "TextChannel".to_string(),
            server: Some("s_cache".to_string()),
            name: Some("cached-room".to_string()),
            ..Default::default()
        }],
        dms: Vec::new(),
        users: vec![User {
            id: "u_c".to_string(),
            username: "cached_user".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    nav.save_to(&cache_path).expect("NavCache save should succeed");
    let loaded = NavCache::load_from(&cache_path).expect("NavCache load should succeed");

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.hydrate_nav_cache(loaded);

    assert_eq!(app.state.servers.len(), 1);
    assert_eq!(app.state.active_server_id.as_deref(), Some("s_cache"));
    assert_eq!(app.state.active_channel_id.as_deref(), Some("c_cache"));
    assert_eq!(app.state.cached_ws_url.as_deref(), Some("wss://events.cached.test"));
    assert_eq!(app.state.autumn_url, "https://cdn.cached.test");

    // Test bare 1/2/3/4 direct pane jump outside Input + Esc back from Input
    app.state.focus = Focus::Chat;
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))).await;
    assert_eq!(app.state.focus, Focus::Servers);
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))).await;
    assert_eq!(app.state.focus, Focus::Channels);
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE))).await;
    assert_eq!(app.state.focus, Focus::Chat);
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE))).await;
    assert_eq!(app.state.focus, Focus::Input);
    // In Input, '1' types into input buffer rather than switching pane
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))).await;
    assert_eq!(app.state.focus, Focus::Input);
    assert_eq!(app.state.input_buffer, "1");
}

#[tokio::test]
async fn test_call_participant_deduplication_and_real_resolution() {
    use stoat_tui::api::models::{Message, User};
    use stoat_tui::event::AppEvent;
    use stoat_tui::gateway::messages::ServerMessage;
    use stoat_tui::gateway::GatewayEvent;
    use stoat_tui::voice::{normalize_livekit_identity, ActiveCallState, CallStatus};

    assert_eq!(normalize_livekit_identity("01ABCDEF123:desktop"), "01ABCDEF123");
    assert_eq!(normalize_livekit_identity("01ABCDEF123"), "01ABCDEF123");

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_call"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);

    app.state.current_user = Some(User {
        id: "01ME000001".to_string(),
        username: "me_user".to_string(),
        display_name: Some("Me".to_string()),
        ..Default::default()
    });
    app.state.users.insert(
        "01REMOTE01".to_string(),
        User {
            id: "01REMOTE01".to_string(),
            username: "rom_user".to_string(),
            display_name: Some("Rom".to_string()),
            ..Default::default()
        },
    );

    let mut call = ActiveCallState::new("ch_call_dedup".to_string());
    call.status = CallStatus::Connected;
    app.state.active_call = Some(call);

    // Local identity with device suffix must NOT be duplicated in call.participants
    app.handle_event(AppEvent::CallParticipantJoined {
        channel_id: "ch_call_dedup".to_string(),
        user_id: "01ME000001:cli".to_string(),
        is_local: false,
    })
    .await;
    assert!(app.state.active_call.as_ref().unwrap().participants.is_empty());

    // Remote identity joined from two device suffixes -> deduplicated to 1 entry
    app.handle_event(AppEvent::CallParticipantJoined {
        channel_id: "ch_call_dedup".to_string(),
        user_id: "01REMOTE01:web".to_string(),
        is_local: false,
    })
    .await;
    app.handle_event(AppEvent::CallParticipantJoined {
        channel_id: "ch_call_dedup".to_string(),
        user_id: "01REMOTE01:mobile".to_string(),
        is_local: false,
    })
    .await;
    assert_eq!(app.state.active_call.as_ref().unwrap().participants.len(), 1);
    assert_eq!(
        app.state.active_call.as_ref().unwrap().participants[0].user_id,
        "01REMOTE01"
    );

    // Synthetic "caller" or call_started without real user never inserts "caller"
    app.handle_event(AppEvent::CallParticipantJoined {
        channel_id: "ch_call_dedup".to_string(),
        user_id: "caller".to_string(),
        is_local: false,
    })
    .await;
    app.handle_event(AppEvent::Gateway(GatewayEvent::MessageReceived(
        ServerMessage::Message(Message {
            id: "01M1".to_string(),
            channel: "ch_call_dedup".to_string(),
            author: "00000000000000000000000000".to_string(),
            system: Some(serde_json::json!({"type": "call_started"})),
            ..Default::default()
        }),
    )))
    .await;
    assert_eq!(app.state.active_call.as_ref().unwrap().participants.len(), 1);
    assert!(
        !app.state
            .voice_channel_participants
            .get("ch_call_dedup")
            .unwrap()
            .iter()
            .any(|p| p.user_id == "caller")
    );

    // Resolve names cleanly without "User" prefix
    assert_eq!(app.state.resolve_call_participant_name("01REMOTE01"), "Rom");
    assert_eq!(app.state.resolve_call_participant_name("01UNKNWN99"), "01UNKN");

    // LiveKit state event updates fallback_name, muted, and deafened
    app.handle_event(AppEvent::CallParticipantState {
        channel_id: "ch_call_dedup".to_string(),
        user_id: "01UNKNWN99:web".to_string(),
        fallback_name: Some("Nick".to_string()),
        is_muted: Some(true),
        is_deafened: Some(true),
    })
    .await;
    assert_eq!(app.state.resolve_call_participant_name("01UNKNWN99"), "Nick");
    let p_nick = app
        .state
        .active_call
        .as_ref()
        .unwrap()
        .participants
        .iter()
        .find(|p| p.user_id == "01UNKNWN99")
        .unwrap();
    assert!(p_nick.is_muted);
    assert!(p_nick.is_deafened);
}

#[test]
fn test_call_stage_cards_speaking_border_and_mute_deafen_icons() {
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;
    use stoat_tui::api::models::User;
    use stoat_tui::ui;
    use stoat_tui::voice::{ActiveCallState, CallStatus, VoiceParticipant};

    let (cfg, _guard) = isolated_config(Some("tok_cards"));
    let mut state = AppState::new(cfg);
    state.show_login_modal = false;
    state.current_user = Some(User {
        id: "u_me".to_string(),
        username: "me".to_string(),
        display_name: Some("Me".to_string()),
        ..Default::default()
    });
    state.users.insert(
        "u_a".to_string(),
        User {
            id: "u_a".to_string(),
            username: "alice".to_string(),
            display_name: Some("Alice".to_string()),
            ..Default::default()
        },
    );
    state.users.insert(
        "u_b".to_string(),
        User {
            id: "u_b".to_string(),
            username: "bob".to_string(),
            display_name: Some("Bob".to_string()),
            ..Default::default()
        },
    );
    state.users.insert(
        "u_c".to_string(),
        User {
            id: "u_c".to_string(),
            username: "carol".to_string(),
            display_name: Some("Carol".to_string()),
            ..Default::default()
        },
    );

    let mut call = ActiveCallState::new("ch_cards".to_string());
    call.status = CallStatus::Connected;
    call.connected_at = Some(std::time::Instant::now());
    call.local_speaking = false;
    call.participants = vec![
        VoiceParticipant {
            user_id: "u_a".to_string(),
            is_speaking: true,
            ..Default::default()
        },
        VoiceParticipant {
            user_id: "u_b".to_string(),
            is_muted: true,
            ..Default::default()
        },
        VoiceParticipant {
            user_id: "u_c".to_string(),
            is_muted: true,
            is_deafened: true,
            ..Default::default()
        },
    ];
    state.active_call = Some(call);

    let mut term = Terminal::new(TestBackend::new(110, 34)).unwrap();
    term.draw(|f| ui::render(f, &mut state)).unwrap();

    let buf = term.backend().buffer();
    let txt: String = buf.content().iter().map(|c| c.symbol()).collect();

    assert!(txt.contains("Me"), "Missing local user card: {}", txt);
    assert!(!txt.contains("Me (You)"), "Must not append awkward (You) suffix: {}", txt);
    assert!(txt.contains("Alice"), "Missing speaking user card: {}", txt);
    assert!(
        txt.contains("Bob 🔇"),
        "Missing muted badge on Bob card: {}",
        txt
    );
    assert!(
        txt.contains("Carol 🎧"),
        "Missing deafened badge on Carol card: {}",
        txt
    );
    assert!(!txt.contains("IDLE"), "Must not render IDLE word");
    assert!(!txt.contains("SPEAKING"), "Must not render SPEAKING word");
    assert!(!txt.contains("User caller"), "Must not render User caller");

    // Verify bright green rounded border exists on buffer (for speaking Alice)
    let green_border_cells = buf
        .content()
        .iter()
        .filter(|c| c.fg == Color::Rgb(35, 165, 90) && matches!(c.symbol(), "╭" | "╮" | "╰" | "╯" | "─" | "│"))
        .count();
    assert!(green_border_cells > 0, "Expected green border cells for speaking participant");
}

#[tokio::test]
async fn test_settings_modal_shift_s_persistence_and_10x10_render() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::config::ACCENT_PRESETS;
    use stoat_tui::event::AppEvent;
    use stoat_tui::ui;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_set"));
    let cfg_path = cfg.custom_path.clone().unwrap();
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.focus = Focus::Chat;

    // Press Shift+S ('S') -> opens Settings modal
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))).await;
    assert!(app.state.show_settings_modal);

    // Move past the 8 My Profile items to Call Sound Effects
    for _ in 0..stoat_tui::ui::modals::settings::PROFILE_ITEM_COUNT {
        app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))).await;
    }

    // Toggle Call Sound Effects (false)
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))).await;
    assert!(!app.state.config.call_sounds_enabled);

    // Move to Left Sidebar Width and expand (+2 -> 28)
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))).await;
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))).await;
    assert_eq!(app.state.left_sidebar_width, 28);
    assert_eq!(app.state.config.left_sidebar_width, 28);

    // Move to Compact Mode and toggle ON
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))).await;
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))).await;
    assert!(app.state.config.compact_mode);

    // Move to Accent Color Preset and cycle to Blurple
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))).await;
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))).await;
    assert_eq!(app.state.config.theme.primary, ACCENT_PRESETS[1].1);

    // Render Settings modal at 80x28 and 10x10
    let mut term80 = Terminal::new(TestBackend::new(80, 28)).unwrap();
    term80.draw(|f| ui::render(f, &mut app.state)).unwrap();
    let txt80: String = term80.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(txt80.contains("Call Sound Effects"));
    assert!(txt80.contains("Compact / Dense Chat"));

    let mut term10 = Terminal::new(TestBackend::new(10, 10)).unwrap();
    term10.draw(|f| ui::render(f, &mut app.state)).unwrap();
    let txt10: String = term10.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(txt10.contains("Settings"));

    // Close with Esc and verify config persisted cleanly to isolated path
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))).await;
    assert!(!app.state.show_settings_modal);

    let reloaded = Config::load_from(&cfg_path);
    assert!(!reloaded.call_sounds_enabled);
    assert_eq!(reloaded.left_sidebar_width, 28);
    assert!(reloaded.compact_mode);
    assert_eq!(reloaded.theme.primary, ACCENT_PRESETS[1].1);
}

#[test]
fn test_call_stage_dynamic_desired_height() {
    use stoat_tui::ui::components::call::CallStage;
    use stoat_tui::voice::{ActiveCallState, CallStatus, VoiceParticipant};

    let mut call = ActiveCallState::new("ch_dyn".to_string());
    call.status = CallStatus::Connected;
    call.participants = vec![
        VoiceParticipant {
            user_id: "u_r1".to_string(),
            ..Default::default()
        },
        VoiceParticipant {
            user_id: "u_r2".to_string(),
            is_speaking: true,
            ..Default::default()
        },
    ];

    // 3 participants (1 local + 2 remote) on width 60 -> 4 cols/row -> 1 row -> 2 + 1 + 4 = 7 rows
    assert_eq!(CallStage::desired_height(60, &call), 7);

    // Narrower width 30 -> 2 cols/row -> 2 rows -> 2 + 1 + 8 = 11 rows
    assert_eq!(CallStage::desired_height(30, &call), 11);

    // When screen sharing is active -> expands to show video canvas
    call.screensharing.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(CallStage::desired_height(60, &call) >= 14);
}

#[tokio::test]
async fn test_multi_account_migration_and_unified_dm_list() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::api::models::{Channel, Server, User};
    use stoat_tui::config::short_instance_label;
    use stoat_tui::event::AppEvent;
    use stoat_tui::gateway::messages::ServerMessage;
    use stoat_tui::gateway::GatewayEvent;
    use stoat_tui::state::ActiveRail;
    use stoat_tui::ui;

    assert_eq!(short_instance_label("https://api.stoat.chat"), None);
    assert_eq!(
        short_instance_label("https://alpha-node.tail1234.ts.net/api"),
        Some("alpha-node".to_string())
    );

    // Legacy single-account config migrates into accounts[0]
    let (cfg, _guard) = isolated_config(Some("tok_primary"));
    let cfg_path = cfg.custom_path.clone().unwrap();
    cfg.save().expect("Save initial config");
    let migrated = Config::load_from(&cfg_path);
    assert_eq!(migrated.accounts.len(), 1);
    assert_eq!(migrated.accounts[0].id, "primary");
    assert_eq!(migrated.accounts[0].api_url, "https://api.stoat.chat");

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(migrated, tx, picker);
    app.state.show_login_modal = false;

    // Add second self-hosted account (keep existing)
    app.handle_event(AppEvent::AccountLoginSuccess {
        api_url: "https://alpha-node.tail1234.ts.net".to_string(),
        token: "tok_alpha".to_string(),
        is_bot: false,
        add_to_existing: true,
    })
    .await;
    assert_eq!(app.state.config.accounts.len(), 2);
    assert_eq!(app.state.config.accounts[0].api_url, "https://api.stoat.chat");
    assert_eq!(app.state.config.accounts[1].id, "alpha-node");
    assert_eq!(app.state.config.accounts[1].label, "alpha-node");

    // Ready from primary account
    app.handle_event(AppEvent::AccountScoped {
        account_id: "primary".to_string(),
        event: Box::new(AppEvent::Gateway(GatewayEvent::MessageReceived(
            ServerMessage::Ready {
                users: vec![
                    User {
                        id: "u_me_p".to_string(),
                        username: "me".to_string(),
                        ..Default::default()
                    },
                    User {
                        id: "u_roman".to_string(),
                        username: "roman".to_string(),
                        display_name: Some("Roman".to_string()),
                        ..Default::default()
                    },
                ],
                servers: vec![Server {
                    id: "srv_p".to_string(),
                    name: "Official".to_string(),
                    channels: vec![],
                    ..Default::default()
                }],
                channels: vec![Channel {
                    id: "dm_roman".to_string(),
                    channel_type: "DirectMessage".to_string(),
                    recipients: Some(vec!["u_me_p".to_string(), "u_roman".to_string()]),
                    ..Default::default()
                }],
                members: vec![],
            },
        ))),
    })
    .await;

    // Ready from self-hosted alpha-node merges cleanly without wiping primary
    app.handle_event(AppEvent::AccountScoped {
        account_id: "alpha-node".to_string(),
        event: Box::new(AppEvent::Gateway(GatewayEvent::MessageReceived(
            ServerMessage::Ready {
                users: vec![
                    User {
                        id: "u_me_a".to_string(),
                        username: "me_alpha".to_string(),
                        ..Default::default()
                    },
                    User {
                        id: "u_coucou".to_string(),
                        username: "coucou".to_string(),
                        display_name: Some("coucou".to_string()),
                        ..Default::default()
                    },
                ],
                servers: vec![Server {
                    id: "srv_a".to_string(),
                    name: "School".to_string(),
                    channels: vec![],
                    ..Default::default()
                }],
                channels: vec![Channel {
                    id: "dm_coucou".to_string(),
                    channel_type: "DirectMessage".to_string(),
                    recipients: Some(vec!["u_me_a".to_string(), "u_coucou".to_string()]),
                    ..Default::default()
                }],
                members: vec![],
            },
        ))),
    })
    .await;

    // Set ApiCurrentUser on both accounts so DM recipient filtering identifies self on each instance
    app.handle_event(AppEvent::AccountScoped {
        account_id: "primary".to_string(),
        event: Box::new(AppEvent::ApiCurrentUser(User {
            id: "u_me_p".to_string(),
            username: "me".to_string(),
            ..Default::default()
        })),
    })
    .await;
    app.handle_event(AppEvent::AccountScoped {
        account_id: "alpha-node".to_string(),
        event: Box::new(AppEvent::ApiCurrentUser(User {
            id: "u_me_a".to_string(),
            username: "me_alpha".to_string(),
            ..Default::default()
        })),
    })
    .await;

    assert_eq!(app.state.servers.len(), 2);
    assert_eq!(app.state.dms.len(), 2);
    assert_eq!(app.state.account_for_channel("dm_roman"), "primary");
    assert_eq!(app.state.account_for_channel("dm_coucou"), "alpha-node");

    app.state.active_rail = ActiveRail::DirectMessages;
    app.state.active_server_id = None;
    app.state.left_sidebar_width = 34;

    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    term.draw(|f| ui::render(f, &mut app.state)).unwrap();
    let txt: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(txt.contains("@ Roman"), "Missing primary DM in unified list: {}", txt);
    assert!(
        txt.contains("@ coucou (alpha-node)"),
        "Missing tagged secondary DM in unified list: {}",
        txt
    );
}

#[tokio::test]
async fn test_message_deduplication_by_id_and_nonce() {
    use stoat_tui::api::models::Message;
    use stoat_tui::event::AppEvent;
    use stoat_tui::gateway::messages::ServerMessage;
    use stoat_tui::gateway::GatewayEvent;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_dedup"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    let channel_id = "01CH_DEDUP".to_string();

    // 1. Insert optimistic message with nonce = "n1"
    app.state.upsert_message(
        &channel_id,
        Message {
            id: "optimistic:n1".to_string(),
            channel: channel_id.clone(),
            author: "01ME".to_string(),
            content: Some("Hello dedup".to_string()),
            nonce: Some("n1".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(app.state.messages[&channel_id].len(), 1);

    // 2. Receive HTTP POST response (_id = "01REALID", nonce = Some("n1"))
    app.handle_event(AppEvent::ApiMessageSent(Ok(Message {
        id: "01REALID".to_string(),
        channel: channel_id.clone(),
        author: "01ME".to_string(),
        content: Some("Hello dedup".to_string()),
        nonce: Some("n1".to_string()),
        ..Default::default()
    })))
    .await;
    assert_eq!(app.state.messages[&channel_id].len(), 1);
    assert_eq!(app.state.messages[&channel_id][0].id, "01REALID");

    // 3. Receive Bonfire WebSocket Message broadcast (_id = "01REALID", nonce = Some("n1"))
    app.handle_event(AppEvent::Gateway(GatewayEvent::MessageReceived(
        ServerMessage::Message(Message {
            id: "01REALID".to_string(),
            channel: channel_id.clone(),
            author: "01ME".to_string(),
            content: Some("Hello dedup".to_string()),
            nonce: Some("n1".to_string()),
            ..Default::default()
        }),
    )))
    .await;

    assert_eq!(app.state.messages[&channel_id].len(), 1);
    assert_eq!(app.state.messages[&channel_id][0].id, "01REALID");
}

#[tokio::test]
async fn test_join_call_already_connected_auto_leave_and_retry() {
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    let server_handle = tokio::spawn(async move {
        let steps = [
            (
                "POST /channels/ch_ghost/join_call ",
                r#"{"node":"europe"}"#,
                "HTTP/1.1 400 Bad Request",
                r#"{"type":"AlreadyConnected","location":"voice_join.rs:80:10"}"#,
            ),
            (
                "POST /channels/ch_ghost/leave_call ",
                "{}",
                "HTTP/1.1 204 No Content",
                "",
            ),
            (
                "POST /channels/ch_ghost/join_call ",
                r#"{"node":"europe"}"#,
                "HTTP/1.1 400 Bad Request",
                r#"{"type":"AlreadyConnected"}"#,
            ),
            (
                "POST /channels/ch_ghost/leave_call ",
                "{}",
                "HTTP/1.1 204 No Content",
                "",
            ),
            (
                "POST /channels/ch_ghost/join_call ",
                r#"{"node":"europe"}"#,
                "HTTP/1.1 200 OK",
                r#"{"token":"jwt_recovered","url":"wss://lk.example.com"}"#,
            ),
        ];

        for (expected_line, expected_body, status_line, resp_body) in steps {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains(expected_line),
                "Expected '{}', got:\n{}",
                expected_line,
                req
            );
            assert!(
                req.to_ascii_lowercase().contains("x-session-token: ghost_tok")
                    || req.to_ascii_lowercase().contains("x-session-token:ghost_tok"),
                "Missing x-session-token header in:\n{}",
                req
            );
            assert!(
                req.ends_with(expected_body),
                "Expected body '{}', got:\n{}",
                expected_body,
                req
            );

            let resp = format!(
                "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_line,
                resp_body.len(),
                resp_body
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(server_url, Some("ghost_tok".to_string()), false);
    let join_res = client
        .join_call("ch_ghost", Some("europe"))
        .await
        .expect("join_call should recover from AlreadyConnected via leave_call and retry");
    assert_eq!(join_res.token, "jwt_recovered");
    assert_eq!(join_res.url.as_deref(), Some("wss://lk.example.com"));

    // Also verify StatusBar cleanly extracts Stoat JSON error "type"
    let sanitized = stoat_tui::ui::components::status_bar::sanitize_status_text(
        r#"Join call error (HTTP 400): {"type":"AlreadyConnected","location":"crates/delta/src/routes/channels/voice_join.rs:80:10"}"#,
    );
    assert_eq!(sanitized, "Join call error (HTTP 400): AlreadyConnected");

    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_leave_call_fallback_and_app_shutdown_leaves_call() {
    use stoat_tui::api::DeltaClient;
    use stoat_tui::voice::ActiveCallState;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 1. Test DeltaClient::leave_call fallback chain: POST /leave_call -> DELETE /call -> DELETE /ring -> POST /end_ring
    let listener1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url1 = format!("http://127.0.0.1:{}", listener1.local_addr().unwrap().port());
    let h1 = tokio::spawn(async move {
        let steps = [
            ("POST /channels/c_fb/leave_call ", "HTTP/1.1 404 Not Found"),
            ("DELETE /channels/c_fb/call ", "HTTP/1.1 404 Not Found"),
            ("DELETE /channels/c_fb/ring ", "HTTP/1.1 404 Not Found"),
            ("POST /channels/c_fb/end_ring ", "HTTP/1.1 204 No Content"),
        ];
        for (expected_line, status_line) in steps {
            let (mut socket, _) = listener1.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains(expected_line),
                "Expected '{}', got:\n{}",
                expected_line,
                req
            );
            let resp = format!("{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", status_line);
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(url1, Some("tok_fb".to_string()), false);
    client.leave_call("c_fb").await;
    h1.await.unwrap();

    // 2. Test App::shutdown cleanly invokes POST /channels/ch_exit/leave_call
    let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url2 = format!("http://127.0.0.1:{}", listener2.local_addr().unwrap().port());
    let h2 = tokio::spawn(async move {
        let (mut socket, _) = listener2.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let n = socket.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        assert!(
            req.contains("POST /channels/ch_exit/leave_call "),
            "Expected POST /leave_call on App::shutdown, got:\n{}",
            req
        );
        let resp = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        socket.write_all(resp.as_bytes()).await.unwrap();
    });

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (mut cfg, _guard) = isolated_config(Some("tok_exit"));
    cfg.api_url = url2.clone();
    cfg.accounts = vec![stoat_tui::config::SavedAccount {
        id: "primary".to_string(),
        label: "".to_string(),
        api_url: url2,
        token: "tok_exit".to_string(),
        is_bot: false,
        user_id: None,
        username: None,
    }];
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.channel_accounts.insert("ch_exit".to_string(), "primary".to_string());
    app.state.active_call = Some(ActiveCallState::new("ch_exit".to_string()));

    app.shutdown().await;
    assert!(app.state.active_call.is_none());
    h2.await.unwrap();
}

#[tokio::test]
async fn test_normalize_and_probe_api_url_auto_detects_api_subpath() {
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 1. Verify scheme normalization for public domains vs local LAN IPs
    assert_eq!(
        DeltaClient::normalize_url_str("alpha-node.example.ts.net/"),
        "https://alpha-node.example.ts.net"
    );
    assert_eq!(
        DeltaClient::normalize_url_str("192.168.1.42:8000/"),
        "http://192.168.1.42:8000"
    );

    // 2. When GET / returns HTML and GET /api/ returns Delta root JSON, resolve to {base}/api
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{}", port);

    let server_handle = tokio::spawn(async move {
        // Request 1: GET / -> 200 OK HTML (web client frontend)
        {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("GET / HTTP/1.1"), "Expected GET /, got:\n{}", req);
            let html = "<!DOCTYPE html><html><body>Stoat Web</body></html>";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
        // Request 2: GET /api/ -> 200 OK Delta JSON metadata
        {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains("GET /api/ HTTP/1.1"),
                "Expected GET /api/, got:\n{}",
                req
            );
            let body = r#"{"revolt":"0.6.5","features":{"autumn":{"enabled":true,"url":"https://alpha-node.example.ts.net/autumn"}},"ws":"wss://alpha-node.example.ts.net/ws"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let resolved = DeltaClient::normalize_and_probe_api_url(&format!("{}/", base_url)).await;
    assert_eq!(resolved, format!("{}/api", base_url));
    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_login_email_friendly_name_and_structured_errors() {
    use stoat_tui::api::models::LoginResponse;
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    let server_handle = tokio::spawn(async move {
        let responses = [
            (
                "HTTP/1.1 401 Unauthorized",
                r#"{"type":"InvalidCredentials"}"#,
            ),
            (
                "HTTP/1.1 401 Unauthorized",
                r#"{"type":"UnverifiedAccount"}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                r#"{"result":"MFA","ticket":"mfa_t_1","allowed_methods":["Totp"]}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                r#"{"result":"Success","token":"sess_tok_ok","user_id":"01USER"}"#,
            ),
        ];

        for (status_line, resp_body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains("POST /auth/session/login HTTP/1.1"),
                "Unexpected request line:\n{}",
                req
            );
            let body_part = req.split("\r\n\r\n").nth(1).unwrap_or("");
            let parsed: serde_json::Value =
                serde_json::from_str(body_part).expect("Login request body should be valid JSON");
            assert_eq!(parsed.get("email").and_then(|v| v.as_str()), Some("alice@example.com"));
            assert_eq!(parsed.get("password").and_then(|v| v.as_str()), Some("pw123"));
            assert_eq!(
                parsed.get("friendly_name").and_then(|v| v.as_str()),
                Some("stoat-tui")
            );

            let resp = format!(
                "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_line,
                resp_body.len(),
                resp_body
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(server_url, None, false);

    // 1. InvalidCredentials error
    let err1 = client
        .login_email("  alice@example.com  ", "pw123")
        .await
        .expect_err("Should fail on InvalidCredentials")
        .to_string();
    assert!(
        err1.contains("InvalidCredentials"),
        "Expected InvalidCredentials in error, got: {}",
        err1
    );

    // 2. UnverifiedAccount error
    let err2 = client
        .login_email("alice@example.com", "pw123")
        .await
        .expect_err("Should fail on UnverifiedAccount")
        .to_string();
    assert!(
        err2.contains("UnverifiedAccount"),
        "Expected UnverifiedAccount in error, got: {}",
        err2
    );

    // 3. MFA challenge surfaces clear Token tab guidance
    let err3 = client
        .login_email("alice@example.com", "pw123")
        .await
        .expect_err("Should return MFA guidance error")
        .to_string();
    assert!(
        err3.contains("Account has 2FA/MFA enabled — log in via Token tab instead"),
        "Expected MFA guidance message, got: {}",
        err3
    );

    // 4. Success returns session token
    let ok_res = client
        .login_email("alice@example.com", "pw123")
        .await
        .expect("Should succeed on 4th attempt");
    match ok_res {
        LoginResponse::Success { token, .. } => assert_eq!(token, "sess_tok_ok"),
        other => panic!("Expected LoginResponse::Success, got {:?}", other),
    }

    server_handle.await.unwrap();
}

#[test]
fn test_rgba_to_i420_bounds_validation_and_ppm_roundtrip() {
    use stoat_tui::voice::rgba_to_i420_buffer;
    use stoat_tui::voice::screen::{downscale_rgba_even, parse_ppm_p6};

    // Zero dimensions must return None safely
    assert!(rgba_to_i420_buffer(&[], 0, 0).is_none());
    assert!(rgba_to_i420_buffer(&[0u8; 16], 0, 4).is_none());
    assert!(rgba_to_i420_buffer(&[0u8; 16], 4, 0).is_none());

    // Odd dimensions must return None safely (required for I420 4:2:0 subsampling)
    assert!(rgba_to_i420_buffer(&vec![0u8; 3 * 4 * 4], 3, 4).is_none());
    assert!(rgba_to_i420_buffer(&vec![0u8; 4 * 5 * 4], 4, 5).is_none());

    // Truncated or oversized RGBA buffer must return None without out-of-bounds read
    assert!(rgba_to_i420_buffer(&vec![0u8; 4 * 4 * 4 - 1], 4, 4).is_none());
    assert!(rgba_to_i420_buffer(&vec![0u8; 4 * 4 * 4 + 4], 4, 4).is_none());

    // Valid even PPM -> downscale_rgba_even -> rgba_to_i420_buffer roundtrip
    let mut ppm = b"P6\n5 3\n255\n".to_vec();
    for _ in 0..15 {
        ppm.extend_from_slice(&[20, 180, 90]);
    }
    let (w, h, rgba) = parse_ppm_p6(&ppm).expect("Valid 5x3 P6 PPM should parse");
    assert_eq!((w, h), (5, 3));
    let (dw, dh, scaled) = downscale_rgba_even(&rgba, w, h, 1280, 720);
    assert_eq!((dw % 2, dh % 2), (0, 0));
    assert!(rgba_to_i420_buffer(&scaled, dw, dh).is_some());
}

#[tokio::test]
async fn test_hyprland_target_parsing_grim_args_and_screen_picker_modal() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::event::AppEvent;
    use stoat_tui::ui;
    use stoat_tui::voice::screen::{
        build_hyprland_targets, grim_args_for_target, wf_recorder_args_for_target,
        ScreenShareTargetKind,
    };
    use stoat_tui::voice::ActiveCallState;

    let monitors_json = r#"[
        {"name": "DP-1", "width": 2560, "height": 1440, "x": 0, "y": 0},
        {"name": "HDMI-A-1", "width": 1920, "height": 1080, "x": 2560, "y": 0}
    ]"#;
    let clients_json = r#"[
        {"mapped": true, "hidden": false, "title": "Neovim", "class": "kitty", "at": [100, 50], "size": [1200, 800]},
        {"mapped": false, "hidden": false, "title": "Ghost", "class": "x", "at": [0, 0], "size": [100, 100]}
    ]"#;

    let targets = build_hyprland_targets(Some(monitors_json), Some(clients_json));
    assert_eq!(targets.len(), 4);
    assert_eq!(
        targets[0].kind,
        ScreenShareTargetKind::Monitor {
            name: "DP-1".to_string(),
            width: 2560,
            height: 1440,
        }
    );
    assert_eq!(
        targets[2].kind,
        ScreenShareTargetKind::Window {
            x: 100,
            y: 50,
            width: 1200,
            height: 800,
            class: "kitty".to_string(),
        }
    );
    assert_eq!(targets[3].kind, ScreenShareTargetKind::EntireDesktop);

    assert_eq!(
        grim_args_for_target(&targets[0].kind),
        vec!["-o", "DP-1", "-t", "ppm", "-"]
    );
    assert_eq!(
        grim_args_for_target(&targets[2].kind),
        vec!["-g", "100,50 1200x800", "-t", "ppm", "-"]
    );
    assert_eq!(
        grim_args_for_target(&targets[3].kind),
        vec!["-t", "ppm", "-"]
    );

    // Verify wf-recorder raw RGBA stdout arguments (--output / --geometry / None)
    let (wf_mon_args, mw, mh) = wf_recorder_args_for_target(&targets[0].kind)
        .expect("Monitor target should produce wf-recorder args");
    assert_eq!((mw, mh), (2560, 1440));
    assert_eq!(
        wf_mon_args,
        vec![
            "--output",
            "DP-1",
            "-c",
            "rawvideo",
            "-m",
            "rawvideo",
            "-x",
            "rgba",
            "-f",
            "pipe:1",
        ]
    );
    let (wf_win_args, ww, wh) = wf_recorder_args_for_target(&targets[2].kind)
        .expect("Window target should produce wf-recorder args");
    assert_eq!((ww, wh), (1200, 800));
    assert_eq!(
        wf_win_args,
        vec![
            "--geometry",
            "100,50 1200x800",
            "-c",
            "rawvideo",
            "-m",
            "rawvideo",
            "-x",
            "rgba",
            "-f",
            "pipe:1",
        ]
    );
    assert!(wf_recorder_args_for_target(&targets[3].kind).is_none());

    // Exercise ScreenShareSelectTarget -> modal render -> j/Enter selection -> ScreenShareCancelled
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_scr"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.show_login_modal = false;
    app.state.active_call = Some(ActiveCallState::new("ch_scr".to_string()));

    app.handle_event(AppEvent::ScreenShareSelectTarget {
        channel_id: "ch_scr".to_string(),
        targets: targets.clone(),
    })
    .await;
    assert!(app.state.show_screen_picker_modal);

    let mut term = Terminal::new(TestBackend::new(90, 24)).unwrap();
    term.draw(|f| ui::render(f, &mut app.state)).unwrap();
    let txt: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(txt.contains("Select Screen Share Source"), "Missing picker title: {}", txt);
    assert!(txt.contains("DP-1"), "Missing monitor entry: {}", txt);

    // Move down twice (to Window) and confirm with Enter
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)))
        .await;
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)))
        .await;
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
        .await;

    assert!(!app.state.show_screen_picker_modal);
    let call = app.state.active_call.as_ref().unwrap();
    assert!(call.is_screensharing());
    let chosen = call.pending_screen_target.lock().unwrap().clone();
    assert_eq!(chosen, Some(targets[2].kind.clone()));

    app.handle_event(AppEvent::ScreenShareCancelled {
        channel_id: "ch_scr".to_string(),
    })
    .await;
    assert!(!app.state.active_call.as_ref().unwrap().is_screensharing());
}

#[tokio::test]
async fn test_start_screen_capture_wayland_routes_through_target_picker() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use stoat_tui::voice::screen::{start_screen_capture, ScreenCaptureError};

    let tmp_dir = std::env::temp_dir().join(format!("stoat_tui_bin_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let hyprctl_path = tmp_dir.join("hyprctl");
    std::fs::write(
        &hyprctl_path,
        r#"#!/bin/sh
if [ "$1" = "monitors" ]; then
  echo '[{"name":"ED240Q S3","width":1920,"height":1080,"x":0,"y":0}]'
elif [ "$1" = "clients" ]; then
  echo '[{"mapped":true,"hidden":false,"title":"Kitty","class":"kitty","at":[0,0],"size":[800,600]}]'
fi
"#,
    )
    .unwrap();
    let mut perms = std::fs::metadata(&hyprctl_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&hyprctl_path, perms).unwrap();

    let prev_path = std::env::var("PATH").unwrap_or_default();
    let prev_wayland = std::env::var("WAYLAND_DISPLAY").ok();
    std::env::set_var("PATH", format!("{}:{}", tmp_dir.display(), prev_path));
    std::env::set_var("WAYLAND_DISPLAY", "wayland-test-1");

    let res = start_screen_capture(Arc::new(AtomicBool::new(false)), None).await;

    std::env::set_var("PATH", prev_path);
    match prev_wayland {
        Some(v) => std::env::set_var("WAYLAND_DISPLAY", v),
        None => std::env::remove_var("WAYLAND_DISPLAY"),
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);

    match res {
        Err(ScreenCaptureError::NeedTargetSelection(targets)) => {
            assert!(targets.iter().any(|t| t.label.contains("ED240Q S3")));
            assert!(targets.iter().any(|t| t.label.contains("Kitty")));
            assert!(targets.iter().any(|t| t.label.contains("Entire Desktop")));
        }
        Ok(_) => panic!("Expected NeedTargetSelection when multiple targets exist"),
        Err(other) => panic!("Unexpected ScreenCaptureError: {:?}", other),
    }
}

#[tokio::test]
async fn test_secondary_account_call_no_duplicate_local_participant() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::api::models::{Channel, User};
    use stoat_tui::event::AppEvent;
    use stoat_tui::ui;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_primary"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.show_login_modal = false;

    // Primary account: Rutytoi (id_primary)
    app.handle_event(AppEvent::AccountScoped {
        account_id: "primary".to_string(),
        event: Box::new(AppEvent::ApiCurrentUser(User {
            id: "id_primary".to_string(),
            username: "Rutytoi".to_string(),
            ..Default::default()
        })),
    })
    .await;

    // Secondary alpha-node account: rutytoi (id_selfhosted)
    app.handle_event(AppEvent::AccountScoped {
        account_id: "alpha-node".to_string(),
        event: Box::new(AppEvent::ApiCurrentUser(User {
            id: "id_selfhosted".to_string(),
            username: "rutytoi".to_string(),
            ..Default::default()
        })),
    })
    .await;

    // Channel owned by alpha-node
    app.state.channel_accounts.insert("ch_alpha".to_string(), "alpha-node".to_string());
    app.state.channels.insert(
        "ch_alpha".to_string(),
        Channel {
            id: "ch_alpha".to_string(),
            channel_type: "DirectMessage".to_string(),
            ..Default::default()
        },
    );
    app.state.active_channel_id = Some("ch_alpha".to_string());

    // Join call on alpha-node channel
    app.toggle_call().await;
    {
        let call = app.state.active_call.as_ref().expect("Call should be initialized");
        assert_eq!(call.account_id, "alpha-node");
        assert_eq!(call.local_user_id, "id_selfhosted");
        assert_eq!(call.participants.len(), 1);
        assert_eq!(call.participants[0].user_id, "id_selfhosted");
        assert!(call.participants[0].is_local);
    }

    // Simulate LiveKit reporting local_participant identity with device suffix + duplicate Bonfire join
    app.handle_event(AppEvent::CallParticipantJoined {
        channel_id: "ch_alpha".to_string(),
        user_id: "id_selfhosted:desktop".to_string(),
        is_local: true,
    })
    .await;
    app.handle_event(AppEvent::CallParticipantJoined {
        channel_id: "ch_alpha".to_string(),
        user_id: "id_selfhosted".to_string(),
        is_local: false,
    })
    .await;

    let call = app.state.active_call.as_ref().unwrap();
    assert_eq!(call.participants.len(), 1, "Must not duplicate local participant");
    assert_eq!(call.participants[0].user_id, "id_selfhosted");

    // Render CallStage and verify only 'rutytoi' appears in the stage, never primary 'Rutytoi'
    let mut term = Terminal::new(TestBackend::new(90, 26)).unwrap();
    term.draw(|f| ui::render(f, &mut app.state)).unwrap();
    let txt: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    let stage_slice = txt
        .split("Live Voice & Video Stage")
        .nth(1)
        .and_then(|s| s.split("Welcome to").next())
        .unwrap_or(&txt);
    assert!(
        stage_slice.contains("rutytoi"),
        "Missing alpha-node local user 'rutytoi' in CallStage: {}",
        stage_slice
    );
    assert!(
        !stage_slice.contains("Rutytoi"),
        "Primary user 'Rutytoi' must not appear in alpha-node CallStage: {}",
        stage_slice
    );
}

#[tokio::test]
async fn test_secondary_account_custom_avatar_uses_instance_autumn_url() {
    use stoat_tui::api::models::{FileObject, User};
    use stoat_tui::api::DeltaClient;
    use stoat_tui::event::AppEvent;
    use stoat_tui::state::AppState;
    use stoat_tui::ui::images::ImageCache;

    // 1. Relative "/autumn" resolves against self-hosted api_url origin
    assert_eq!(
        DeltaClient::resolve_autumn_url("https://alpha-node.example.ts.net/api", Some("/autumn")),
        "https://alpha-node.example.ts.net/autumn"
    );
    assert_eq!(
        DeltaClient::resolve_autumn_url("https://alpha-node.example.ts.net/api", None),
        "https://alpha-node.example.ts.net/autumn"
    );
    assert_eq!(
        DeltaClient::resolve_autumn_url("https://api.stoat.chat", None),
        "https://cdn.stoatusercontent.com"
    );

    // 2. request_user_avatar_with_key_in_account uses alpha-node's autumn URL + avatar._id
    let (cfg, _guard) = isolated_config(Some("tok_primary"));
    let mut state = AppState::new(cfg);
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    state.image_cache = Some(ImageCache::new(picker));
    let (img_tx, mut img_rx) = tokio::sync::mpsc::unbounded_channel();
    state.image_fetch_tx = Some(img_tx);

    state.autumn_url = "https://cdn.stoatusercontent.com".to_string();
    state.account_api_urls.insert(
        "alpha-node".to_string(),
        "https://alpha-node.example.ts.net/api".to_string(),
    );
    state.account_autumn_urls.insert(
        "alpha-node".to_string(),
        "https://alpha-node.example.ts.net/autumn".to_string(),
    );

    let alpha_user = User {
        id: "01ALPHA_USER".to_string(),
        username: "rutytoi".to_string(),
        avatar: Some(FileObject {
            id: "01AVATAR_CUSTOM".to_string(),
            tag: Some("avatars".to_string()),
            filename: Some("pfp.png".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    state.users.insert(alpha_user.id.clone(), alpha_user.clone());

    let key = state.avatar_cache_key("alpha-node", &alpha_user.id, "avatar");
    assert_eq!(key, "avatar:alpha-node:01ALPHA_USER:01AVATAR_CUSTOM");

    state.request_user_avatar_with_key_in_account("alpha-node", &alpha_user.id, &key);
    let (fetched_key, fetched_url) = img_rx
        .try_recv()
        .expect("Should enqueue custom avatar fetch for alpha-node user");
    assert_eq!(fetched_key, key);
    assert_eq!(
        fetched_url,
        "https://alpha-node.example.ts.net/autumn/avatars/01AVATAR_CUSTOM?max_side=128"
    );
    assert!(
        !fetched_url.contains("stoatusercontent.com") && !fetched_url.contains("default_avatar"),
        "Must not fall back to primary CDN or default_avatar: {}",
        fetched_url
    );

    // 3. Also verify AppEvent::AccountScoped(AutumnUrlResolved("/autumn")) resolves cleanly
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let (cfg2, _guard2) = isolated_config(Some("tok_p"));
    let picker2 = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg2, tx, picker2);
    app.state.account_api_urls.insert(
        "alpha-node".to_string(),
        "https://alpha-node.example.ts.net/api".to_string(),
    );
    app.handle_event(AppEvent::AccountScoped {
        account_id: "alpha-node".to_string(),
        event: Box::new(AppEvent::AutumnUrlResolved(DeltaClient::resolve_autumn_url(
            "https://alpha-node.example.ts.net/api",
            Some("/autumn"),
        ))),
    })
    .await;
    assert_eq!(
        app.state.autumn_url_for_account("alpha-node"),
        "https://alpha-node.example.ts.net/autumn"
    );
}

#[tokio::test]
async fn test_profile_api_upload_edit_user_and_change_username() {
    use stoat_tui::api::models::{EditUserRequest, Presence, UserProfileUpdate, UserStatus};
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // 1. Test DeltaClient::upload_autumn_tag (multipart POST /avatars)
    let tmp_img = std::env::temp_dir().join(format!("stoat_pfp_{}.png", std::process::id()));
    std::fs::write(&tmp_img, b"PNG_FAKE_BYTES").unwrap();

    let autumn_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let autumn_url = format!("http://127.0.0.1:{}", autumn_listener.local_addr().unwrap().port());
    let h_autumn = tokio::spawn(async move {
        let (mut socket, _) = autumn_listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        assert!(req.contains("POST /avatars HTTP/1.1"), "Expected POST /avatars, got:\n{}", req);
        assert!(
            req.to_ascii_lowercase().contains("multipart/form-data"),
            "Expected multipart/form-data header"
        );
        assert!(req.contains("PNG_FAKE_BYTES"), "Missing file bytes in multipart body");
        let body = r#"{"id":"01AUTUMN_PFP_ID"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(resp.as_bytes()).await.unwrap();
    });

    let api_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://127.0.0.1:{}", api_listener.local_addr().unwrap().port());
    let h_api = tokio::spawn(async move {
        // Request 1: PATCH /users/01ME
        {
            let (mut socket, _) = api_listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("PATCH /users/01ME HTTP/1.1"), "Expected PATCH /users/01ME, got:\n{}", req);
            let body_part = req.split("\r\n\r\n").nth(1).unwrap_or("");
            let parsed: serde_json::Value = serde_json::from_str(body_part).unwrap();
            assert_eq!(parsed["display_name"], "Ruty Updated");
            assert_eq!(parsed["avatar"], "01AUTUMN_PFP_ID");
            assert_eq!(parsed["status"]["presence"], "Focus");
            assert_eq!(parsed["status"]["text"], "Coding in Rust");
            assert_eq!(parsed["profile"]["content"], "TUI enjoyer");

            let resp_body = r#"{"_id":"01ME","username":"rutytoi","display_name":"Ruty Updated","avatar":{"_id":"01AUTUMN_PFP_ID","tag":"avatars","filename":"pfp.png"},"status":{"presence":"Focus","text":"Coding in Rust"}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
        // Request 2: PATCH /users/@me/username
        {
            let (mut socket, _) = api_listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(
                req.contains("PATCH /users/@me/username HTTP/1.1"),
                "Expected PATCH /users/@me/username, got:\n{}",
                req
            );
            let body_part = req.split("\r\n\r\n").nth(1).unwrap_or("");
            let parsed: serde_json::Value = serde_json::from_str(body_part).unwrap();
            assert_eq!(parsed["username"], "ruty_new");
            assert_eq!(parsed["password"], "pw_secret");

            let resp_body = r#"{"_id":"01ME","username":"ruty_new"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(api_url, Some("tok_prof".to_string()), false);
    let uploaded_id = client
        .upload_autumn_tag(&autumn_url, "avatars", &tmp_img)
        .await
        .expect("upload_autumn_tag should succeed");
    assert_eq!(uploaded_id, "01AUTUMN_PFP_ID");
    let _ = std::fs::remove_file(&tmp_img);
    h_autumn.await.unwrap();

    let updated_user = client
        .edit_user(
            "01ME",
            EditUserRequest {
                display_name: Some("Ruty Updated".to_string()),
                avatar: Some(uploaded_id),
                status: Some(UserStatus {
                    presence: Some(Presence::Focus),
                    text: Some("Coding in Rust".to_string()),
                }),
                profile: Some(UserProfileUpdate {
                    content: Some("TUI enjoyer".to_string()),
                    background: None,
                }),
                remove: None,
            },
        )
        .await
        .expect("edit_user should succeed");
    assert_eq!(updated_user.display_name.as_deref(), Some("Ruty Updated"));
    assert_eq!(updated_user.avatar.as_ref().map(|a| a.id.as_str()), Some("01AUTUMN_PFP_ID"));

    let renamed_user = client
        .change_username("  ruty_new  ", "pw_secret")
        .await
        .expect("change_username should succeed");
    assert_eq!(renamed_user.username, "ruty_new");
    h_api.await.unwrap();
}

#[tokio::test]
async fn test_settings_modal_profile_editor_render_and_user_profile_updated() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::api::models::{FileObject, User};
    use stoat_tui::event::AppEvent;
    use stoat_tui::ui;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (mut cfg, _guard) = isolated_config(Some("tok_prof_ui"));
    let alpha_id = cfg.upsert_account(
        "https://alpha-node.example.ts.net/api".to_string(),
        "tok_alpha".to_string(),
        false,
        false,
    );
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.focus = Focus::Chat;

    // Seed alpha-node user and stale cached default avatar
    app.handle_event(AppEvent::AccountScoped {
        account_id: alpha_id.clone(),
        event: Box::new(AppEvent::ApiCurrentUser(User {
            id: "01ALPHA_ME".to_string(),
            username: "ruty_old".to_string(),
            ..Default::default()
        })),
    })
    .await;
    if let Some(ref mut cache) = app.state.image_cache {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2));
        cache.insert(format!("avatar:{}:01ALPHA_ME:default", alpha_id), img);
    }

    // Open Settings modal and render at 80x30
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))).await;
    assert!(app.state.show_settings_modal);

    let mut term = Terminal::new(TestBackend::new(80, 30)).unwrap();
    term.draw(|f| ui::render(f, &mut app.state)).unwrap();
    let txt: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(txt.contains("My Profile: Account"), "Missing Account row: {}", txt);
    assert!(txt.contains("My Profile: Presence"), "Missing Presence row: {}", txt);
    assert!(txt.contains("My Profile: Display Name"), "Missing Display Name row: {}", txt);
    assert!(txt.contains("My Profile: Upload PFP"), "Missing Upload PFP row: {}", txt);
    assert!(txt.contains("My Profile: Upload Banner"), "Missing Upload Banner row: {}", txt);

    // Dispatch UserProfileUpdated and verify state + ImageCache busting + SavedAccount sync
    app.handle_event(AppEvent::UserProfileUpdated {
        account_id: alpha_id.clone(),
        user: User {
            id: "01ALPHA_ME".to_string(),
            username: "ruty_renamed".to_string(),
            display_name: Some("Ruty Alpha".to_string()),
            avatar: Some(FileObject {
                id: "01NEW_AVATAR".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        },
    })
    .await;

    assert_eq!(
        app.state.account_users[&alpha_id].username,
        "ruty_renamed"
    );
    assert_eq!(
        app.state.users["01ALPHA_ME"].display_name.as_deref(),
        Some("Ruty Alpha")
    );
    assert!(
        !app.state
            .image_cache
            .as_ref()
            .unwrap()
            .contains(&format!("avatar:{}:01ALPHA_ME:default", alpha_id)),
        "Stale default avatar cache entry must be busted on UserProfileUpdated"
    );
    let saved_alpha = app
        .state
        .config
        .accounts
        .iter()
        .find(|a| a.id == alpha_id)
        .unwrap();
    assert_eq!(saved_alpha.username.as_deref(), Some("ruty_renamed"));
}

#[tokio::test]
async fn test_presence_cycling_debounce_coalesces_single_patch() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use stoat_tui::api::models::{Presence, User};
    use stoat_tui::event::AppEvent;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let req_count = Arc::new(AtomicUsize::new(0));
    let req_count_srv = req_count.clone();

    let server_handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        req_count_srv.fetch_add(1, Ordering::SeqCst);
        let mut buf = [0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        assert!(req.contains("PATCH /users/01ME HTTP/1.1"), "Unexpected req:\n{}", req);
        let body_part = req.split("\r\n\r\n").nth(1).unwrap_or("");
        let parsed: serde_json::Value = serde_json::from_str(body_part).unwrap();
        assert_eq!(parsed["status"]["presence"], "Busy");

        let resp_body = r#"{"_id":"01ME","username":"me","status":{"presence":"Busy"}}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            resp_body.len(),
            resp_body
        );
        socket.write_all(resp.as_bytes()).await.unwrap();
    });

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let (mut cfg, _guard) = isolated_config(Some("tok_deb"));
    cfg.api_url = server_url.clone();
    cfg.upsert_account(server_url, "tok_deb".to_string(), false, true);
    let picker = Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.focus = Focus::Chat;

    app.handle_event(AppEvent::AccountScoped {
        account_id: "primary".to_string(),
        event: Box::new(AppEvent::ApiCurrentUser(User {
            id: "01ME".to_string(),
            username: "me".to_string(),
            ..Default::default()
        })),
    })
    .await;

    // Open Settings modal and move to Presence (idx 1)
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))).await;
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))).await;

    // Rapidly cycle 3 times: Online -> Idle -> Focus -> Busy
    for _ in 0..3 {
        app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))).await;
    }

    assert!(app.state.pending_presence_update.is_some());
    assert_eq!(
        app.state.account_users["primary"]
            .status
            .as_ref()
            .and_then(|s| s.presence.clone()),
        Some(Presence::Busy)
    );

    // Tick before 800ms must NOT fire any HTTP request
    app.handle_event(AppEvent::Tick).await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(req_count.load(Ordering::SeqCst), 0, "Must debounce intermediate presence changes");

    // Closing modal flushes the single coalesced PATCH immediately
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))).await;
    server_handle.await.unwrap();
    assert_eq!(req_count.load(Ordering::SeqCst), 1);

    let ev = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
        .await
        .expect("Should receive UserProfileUpdated")
        .unwrap();
    assert!(matches!(ev, AppEvent::UserProfileUpdated { .. }));
}

#[tokio::test]
async fn test_edit_user_retries_once_on_http_429_retry_after() {
    use stoat_tui::api::models::{EditUserRequest, Presence, UserStatus};
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());

    let server_handle = tokio::spawn(async move {
        // Request 1 -> HTTP 429 with retry_after: 60ms
        {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = socket.read(&mut buf).await.unwrap();
            let err_body = r#"{"type":"TooManyRequests","retry_after":60}"#;
            let resp = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                err_body.len(),
                err_body
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
        // Request 2 -> HTTP 200 OK
        {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = socket.read(&mut buf).await.unwrap();
            let ok_body = r#"{"_id":"01ME","username":"me","status":{"presence":"Idle"}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                ok_body.len(),
                ok_body
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(api_url, Some("tok_429".to_string()), false);
    let updated = client
        .edit_user(
            "01ME",
            EditUserRequest {
                status: Some(UserStatus {
                    presence: Some(Presence::Idle),
                    text: None,
                }),
                ..Default::default()
            },
        )
        .await
        .expect("edit_user should retry after 429 and succeed");

    assert_eq!(updated.presence_str(), "Idle");
    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_delta_client_friend_requests_and_dm() {
    use stoat_tui::api::models::RelationshipStatus;
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());

    let server_handle = tokio::spawn(async move {
        // 1. POST /users/friend
        {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("POST /users/friend "), "Unexpected req: {}", req);
            assert!(req.contains("\"username\":\"alice#1234\""), "Missing username in body: {}", req);
            let body = r#"{"_id":"u_alice","username":"alice","discriminator":"1234","relationship":"Outgoing"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
        // 2. PUT /users/u_bob/friend
        {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("PUT /users/u_bob/friend "), "Unexpected req: {}", req);
            let body = r#"{"_id":"u_bob","username":"bob","discriminator":"5678","relationship":"Friend"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
        // 3. DELETE /users/u_bob/friend
        {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("DELETE /users/u_bob/friend "), "Unexpected req: {}", req);
            let body = r#"{"_id":"u_bob","username":"bob","discriminator":"5678","relationship":"None"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
        // 4. GET /users/u_alice/dm
        {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("GET /users/u_alice/dm "), "Unexpected req: {}", req);
            let body = r#"{"_id":"dm_alice","channel_type":"DirectMessage","recipients":["u_me","u_alice"]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let client = DeltaClient::new(api_url, Some("tok_friends".to_string()), false);
    let sent = client.send_friend_request("alice#1234").await.unwrap();
    assert_eq!(sent.id, "u_alice");
    assert_eq!(sent.relationship, Some(RelationshipStatus::Outgoing));

    let accepted = client.accept_friend_request("u_bob").await.unwrap();
    assert_eq!(accepted.relationship, Some(RelationshipStatus::Friend));

    let removed = client.remove_friend("u_bob").await.unwrap();
    assert_eq!(removed.relationship, Some(RelationshipStatus::None));

    let dm = client.open_dm("u_alice").await.unwrap();
    assert_eq!(dm.id, "dm_alice");
    assert_eq!(dm.channel_type, "DirectMessage");

    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_friends_modal_bonfire_sync_render_and_actions() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::api::models::{Channel, RelationshipStatus, User};
    use stoat_tui::event::AppEvent;
    use stoat_tui::gateway::messages::ServerMessage;
    use stoat_tui::gateway::GatewayEvent;
    use stoat_tui::state::Focus;
    use stoat_tui::ui;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_f_modal"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.show_login_modal = false;
    app.state.focus = Focus::Chat;

    // 1. Parse live Bonfire UserRelationship event for an incoming friend request
    let raw_ev = r#"{"type":"UserRelationship","id":"u_me","user":{"_id":"u_in","username":"roman","discriminator":"9001","relationship":"Incoming"}}"#;
    let parsed = ServerMessage::parse_resilient(raw_ev).expect("UserRelationship should parse");
    assert!(matches!(parsed, ServerMessage::UserRelationship { .. }));

    app.handle_event(AppEvent::Gateway(GatewayEvent::MessageReceived(parsed))).await;
    assert_eq!(app.state.incoming_friend_request_count(), 1);
    assert!(app
        .state
        .status_message
        .as_ref()
        .map(|(s, _)| s.contains("roman"))
        .unwrap_or(false));

    // 2. Open Friends modal with Shift+F and render
    app.handle_event(AppEvent::Key(KeyEvent::new(
        KeyCode::Char('F'),
        KeyModifiers::SHIFT,
    )))
    .await;
    assert!(app.state.show_friends_modal);

    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| ui::render(f, &mut app.state))
        .expect("FriendsModal should render cleanly");
    let buf_text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(buf_text.contains("Friends & Requests (1 pending)"), "Missing modal title");
    assert!(buf_text.contains("roman#9001"), "Missing incoming user in FriendsModal");

    // 3. Focus Add Friend input with 'a', type username, Esc returns to list
    app.handle_event(AppEvent::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
    )))
    .await;
    assert!(app.state.friends_input_focused);
    for ch in "bob#1234".chars() {
        app.handle_event(AppEvent::Key(KeyEvent::new(
            KeyCode::Char(ch),
            KeyModifiers::NONE,
        )))
        .await;
    }
    assert_eq!(app.state.friends_input_buffer, "bob#1234");
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))).await;
    assert!(!app.state.friends_input_focused);

    // 4. FriendActionSucceeded promotes incoming -> Friend
    let acc_id = app.state.active_account_id.clone();
    app.handle_event(AppEvent::FriendActionSucceeded {
        account_id: acc_id.clone(),
        user: User {
            id: "u_in".to_string(),
            username: "roman".to_string(),
            discriminator: Some("9001".to_string()),
            relationship: Some(RelationshipStatus::Friend),
            ..Default::default()
        },
        message: "✓ Accepted friend request from @roman".to_string(),
    })
    .await;
    assert_eq!(app.state.incoming_friend_request_count(), 0);
    assert_eq!(
        app.state.account_relationships.get(&(acc_id.clone(), "u_in".to_string())),
        Some(&RelationshipStatus::Friend)
    );

    // 5. DmChannelOpened closes FriendsModal and jumps to the DM
    app.handle_event(AppEvent::DmChannelOpened {
        account_id: acc_id,
        channel: Channel {
            id: "dm_roman".to_string(),
            channel_type: "DirectMessage".to_string(),
            recipients: Some(vec!["u_me".to_string(), "u_in".to_string()]),
            ..Default::default()
        },
    })
    .await;
    assert!(!app.state.show_friends_modal);
    assert_eq!(app.state.active_channel_id.as_deref(), Some("dm_roman"));
}

#[tokio::test]
async fn test_delta_client_create_group_endpoint() {
    use stoat_tui::api::DeltaClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());

    let server_handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        assert!(req.starts_with("POST /channels/create "), "Unexpected req: {}", req);
        assert!(req.contains("\"name\":\"Dev Squad\""), "Missing name: {}", req);
        assert!(req.contains("\"description\":\"Late night\""), "Missing desc: {}", req);
        assert!(req.contains("\"u_a\"") && req.contains("\"u_b\""), "Missing users: {}", req);

        let body = r#"{"_id":"grp_1","channel_type":"Group","name":"Dev Squad","description":"Late night","recipients":["u_me","u_a","u_b"]}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
    });

    let client = DeltaClient::new(api_url, Some("tok_grp".to_string()), false);
    let ch = client
        .create_group(
            "Dev Squad",
            Some("Late night"),
            &["u_a".to_string(), "u_b".to_string()],
        )
        .await
        .expect("create_group should succeed");

    assert_eq!(ch.id, "grp_1");
    assert_eq!(ch.channel_type, "Group");
    assert_eq!(ch.name.as_deref(), Some("Dev Squad"));
    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_create_group_modal_flow_render_and_event() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use stoat_tui::api::models::{Channel, RelationshipStatus, User};
    use stoat_tui::event::AppEvent;
    use stoat_tui::state::Focus;
    use stoat_tui::ui;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_grp_modal"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.show_login_modal = false;
    app.state.focus = Focus::Chat;

    let acc_id = app.state.active_account_id.clone();
    let friend = User {
        id: "u_friend1".to_string(),
        username: "roman".to_string(),
        discriminator: Some("9001".to_string()),
        relationship: Some(RelationshipStatus::Friend),
        online: Some(true),
        ..Default::default()
    };
    app.state.sync_user_relationships(&acc_id, &friend);
    app.state.users.insert(friend.id.clone(), friend);

    // 1. Open Create Group modal with Shift+G
    app.handle_event(AppEvent::Key(KeyEvent::new(
        KeyCode::Char('G'),
        KeyModifiers::SHIFT,
    )))
    .await;
    assert!(app.state.show_create_group_modal);
    assert_eq!(app.state.group_focus_section, 0);

    // 2. Type group name, Tab to Desc, Tab to Friends list, Space to select friend
    for ch in "Squad".chars() {
        app.handle_event(AppEvent::Key(KeyEvent::new(
            KeyCode::Char(ch),
            KeyModifiers::NONE,
        )))
        .await;
    }
    assert_eq!(app.state.group_name_input, "Squad");

    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)))
        .await;
    assert_eq!(app.state.group_focus_section, 1);

    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)))
        .await;
    assert_eq!(app.state.group_focus_section, 2);

    app.handle_event(AppEvent::Key(KeyEvent::new(
        KeyCode::Char(' '),
        KeyModifiers::NONE,
    )))
    .await;
    assert!(app.state.group_selected_users.contains("u_friend1"));

    // 3. Render CreateGroupModal via TestBackend
    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| ui::render(f, &mut app.state))
        .expect("CreateGroupModal should render cleanly");
    let buf_text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(buf_text.contains("Create Group (1 selected)"), "Missing modal title");
    assert!(buf_text.contains("[✓]"), "Missing checked friend box");
    assert!(buf_text.contains("@roman#9001"), "Missing friend handle");

    // 4. GroupCreated event closes modal, adds group to DMs, and jumps into it
    app.handle_event(AppEvent::GroupCreated {
        account_id: acc_id,
        channel: Channel {
            id: "grp_squad".to_string(),
            channel_type: "Group".to_string(),
            name: Some("Squad".to_string()),
            recipients: Some(vec!["u_me".to_string(), "u_friend1".to_string()]),
            ..Default::default()
        },
    })
    .await;
    assert!(!app.state.show_create_group_modal);
    assert_eq!(app.state.active_channel_id.as_deref(), Some("grp_squad"));
    assert!(app.state.dms.iter().any(|d| d.id == "grp_squad"));
}

#[tokio::test]
async fn test_delete_or_leave_group_endpoint_and_two_step_confirm() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use stoat_tui::api::models::Channel;
    use stoat_tui::api::DeltaClient;
    use stoat_tui::event::AppEvent;
    use stoat_tui::state::Focus;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());

    let server_handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        assert!(
            req.starts_with("DELETE /channels/grp_del_1 "),
            "Unexpected req: {}",
            req
        );
        let resp = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        sock.write_all(resp.as_bytes()).await.unwrap();
    });

    // 1. Direct DeltaClient DELETE /channels/{id}
    let client = DeltaClient::new(api_url, Some("tok_del".to_string()), false);
    client
        .delete_or_leave_channel("grp_del_1")
        .await
        .expect("DELETE /channels/{id} should succeed on 204");
    server_handle.await.unwrap();

    // 2. Two-step confirmation via Shift+X and ChannelDeleted cleanup
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (cfg, _guard) = isolated_config(Some("tok_del_app"));
    let picker = std::sync::Arc::new(ratatui_image::picker::Picker::halfblocks());
    let mut app = stoat_tui::app::App::new(cfg, tx, picker);
    app.state.show_login_modal = false;
    app.state.focus = Focus::Chat;

    let grp = Channel {
        id: "grp_del_2".to_string(),
        channel_type: "Group".to_string(),
        name: Some("Temp Group".to_string()),
        ..Default::default()
    };
    app.state.dms.push(grp.clone());
    app.state.channels.insert(grp.id.clone(), grp);
    app.state.active_channel_id = Some("grp_del_2".to_string());

    // First Ctrl+X opens the ConfirmDeleteModal without deleting yet
    app.handle_event(AppEvent::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::CONTROL,
    )))
    .await;
    assert!(app.state.pending_delete_channel.is_some());
    assert!(app.state.channels.contains_key("grp_del_2"));

    // Verify ConfirmDeleteModal renders clearly
    let backend = ratatui::backend::TestBackend::new(100, 28);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|f| stoat_tui::ui::render(f, &mut app.state))
        .expect("ConfirmDeleteModal should render cleanly");
    let buf_text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(buf_text.contains("Confirm Delete / Leave"), "Missing confirm modal title");
    assert!(buf_text.contains("Temp Group"), "Missing group name in confirm modal");

    // Esc cancels cleanly
    app.handle_event(AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
        .await;
    assert!(app.state.pending_delete_channel.is_none());

    // ChannelDeleted removes channel from channels, dms, and clears active_channel_id
    app.handle_event(AppEvent::ChannelDeleted {
        channel_id: "grp_del_2".to_string(),
        name: "Temp Group".to_string(),
    })
    .await;
    assert!(!app.state.channels.contains_key("grp_del_2"));
    assert!(!app.state.dms.iter().any(|d| d.id == "grp_del_2"));
    assert_ne!(app.state.active_channel_id.as_deref(), Some("grp_del_2"));
}
