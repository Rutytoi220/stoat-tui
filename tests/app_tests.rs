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
}
