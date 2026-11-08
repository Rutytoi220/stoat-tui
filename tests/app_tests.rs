use stoat_tui::config::{parse_hex_color, Config};
use stoat_tui::gateway::messages::{ClientMessage, ServerMessage};
use stoat_tui::state::{AppState, Focus, PaletteItemType};
use stoat_tui::ui::markdown::MarkdownParser;
use ratatui::style::Color;

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

    let mut state = AppState::new(Config::default());
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
    let mut state = AppState::new(Config::default());
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
    let mut state = AppState::new(Config::default());
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
    let mut app = stoat_tui::app::App::new(Config::default(), tx);
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
    let config_none = Config { token: None, ..Default::default() };
    let (tx1, _rx1) = tokio::sync::mpsc::channel(64);
    let mut app_none = stoat_tui::app::App::new(config_none, tx1);
    app_none.startup();
    assert!(app_none.state.show_login_modal);
    assert!(matches!(app_none.state.gateway_status, GatewayStatus::Disconnected(_)));
    assert_eq!(
        app_none.state.status_message.as_ref().map(|(msg, _)| msg.as_str()),
        Some("Please log in to connect")
    );

    // Case 2: Token is whitespace / empty
    let config_empty = Config { token: Some("   ".to_string()), ..Default::default() };
    let (tx2, _rx2) = tokio::sync::mpsc::channel(64);
    let mut app_empty = stoat_tui::app::App::new(config_empty, tx2);
    app_empty.startup();
    assert!(app_empty.state.show_login_modal);
    assert!(matches!(app_empty.state.gateway_status, GatewayStatus::Disconnected(_)));

    // Case 3: Token is valid non-empty string
    let config_token = Config { token: Some("valid_secret_token".to_string()), ..Default::default() };
    let (tx3, mut rx3) = tokio::sync::mpsc::channel(64);
    let mut app_token = stoat_tui::app::App::new(config_token, tx3);
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
    let config = Config { token: Some("test_tok".to_string()), ..Default::default() };
    let mut app = stoat_tui::app::App::new(config, tx);

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

    // Press Ctrl+L -> opens login modal
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
    app.handle_event(stoat_tui::event::AppEvent::Key(ctrl_l)).await;
    assert!(app.state.show_login_modal);
}

#[tokio::test]
async fn test_api_session_expired_event() {
    use stoat_tui::api::models::User;
    use stoat_tui::event::AppEvent;
    use stoat_tui::state::GatewayStatus;

    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let config = Config { token: Some("expired_tok_123".to_string()), ..Default::default() };
    let mut app = stoat_tui::app::App::new(config, tx);

    app.state.current_user = Some(User {
        id: "u_abc".to_string(),
        username: "testuser".to_string(),
        display_name: Some("Test User".to_string()),
        avatar: None,
        status: None,
        bot: None,
        online: Some(true),
    });
    app.state.show_login_modal = false;

    // Dispatch ApiSessionExpired
    app.handle_event(AppEvent::ApiSessionExpired).await;

    // Token must be cleared from config
    assert!(app.state.config.token.is_none());
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
    let mut app = stoat_tui::app::App::new(Config::default(), tx);

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
