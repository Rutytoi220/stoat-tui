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
