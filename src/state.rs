use crate::api::models::{Category, Channel, Member, MemberId, Message, Role, Server, User, UserStatus, Presence};
use crate::config::Config;
use crate::ui::theme::Theme;
use crate::voice::{ActiveCallState, VoiceParticipant};
use chrono::Utc;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Servers,
    Channels,
    Chat,
    Input,
    Members,
}

impl Focus {
    pub fn next(&self, member_rail_open: bool) -> Self {
        match self {
            Focus::Servers => Focus::Channels,
            Focus::Channels => Focus::Chat,
            Focus::Chat => Focus::Input,
            Focus::Input => {
                if member_rail_open {
                    Focus::Members
                } else {
                    Focus::Servers
                }
            }
            Focus::Members => Focus::Servers,
        }
    }

    pub fn prev(&self, member_rail_open: bool) -> Self {
        match self {
            Focus::Servers => {
                if member_rail_open {
                    Focus::Members
                } else {
                    Focus::Input
                }
            }
            Focus::Channels => Focus::Servers,
            Focus::Chat => Focus::Channels,
            Focus::Input => Focus::Chat,
            Focus::Members => Focus::Input,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveRail {
    DirectMessages,
    Server(String),
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum GatewayStatus {
    Disconnected(String),
    Connecting,
    Connected { latency_ms: u64 },
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub item_type: PaletteItemType,
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteItemType {
    Server,
    Channel,
    DirectMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginTab {
    EmailPassword,
    Token,
    Demo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStep {
    Credentials,
    TotpChallenge { ticket: String },
}

pub struct AppState {
    pub config: Config,
    pub theme: Theme,
    pub focus: Focus,
    pub active_rail: ActiveRail,
    pub active_server_id: Option<String>,
    pub active_channel_id: Option<String>,

    // Lists selection
    pub selected_server_index: usize,
    pub selected_channel_index: usize,
    pub selected_member_index: usize,

    // Chat scroll (0 = bottom, >0 = scrolled up by N lines)
    pub chat_scroll_offset: usize,

    // Input buffer
    pub input_buffer: String,
    pub input_cursor: usize,

    // UI Toggles & Modals
    pub show_member_rail: bool,
    pub show_command_palette: bool,
    pub command_palette_query: String,
    pub command_palette_selected: usize,

    pub show_login_modal: bool,
    pub login_tab: LoginTab,
    pub login_step: LoginStep,
    pub login_field_index: usize, // 0 = email/token, 1 = password
    pub login_email_input: String,
    pub login_password_input: String,
    pub login_totp_input: String,
    pub login_token_input: String,
    pub login_error: Option<String>,
    pub login_loading: bool,

    // Gateway / network
    pub gateway_status: GatewayStatus,
    pub status_message: Option<(String, chrono::DateTime<Utc>)>,

    // Data stores
    pub current_user: Option<User>,
    pub users: HashMap<String, User>,
    pub servers: Vec<Server>,
    pub channels: HashMap<String, Channel>,
    pub server_channels: HashMap<String, Vec<String>>,
    pub dms: Vec<Channel>,
    pub members: HashMap<String, Vec<Member>>,
    pub messages: HashMap<String, Vec<Message>>,
    pub unread_counts: HashMap<String, usize>,

    // Voice & Video Call Subsystem
    pub active_call: Option<ActiveCallState>,
    pub call_fullscreen: bool,
    pub voice_channel_participants: HashMap<String, Vec<VoiceParticipant>>,

    // Dynamic user resolution cache
    pub pending_user_fetches: Arc<Mutex<HashSet<String>>>,
    pub user_fetch_tx: Option<mpsc::UnboundedSender<String>>,

    // Precision hit-test rects from last render frame
    pub rect_rail: Rect,
    pub rect_server_items: Vec<(Option<String>, Rect)>, // None = DMs button, Some(id) = server
    pub rect_channels: Rect,
    pub rect_channel_items: Vec<(String, Rect)>, // channel ID -> Rect
    pub rect_chat: Rect,
    pub rect_input: Rect,
    pub rect_members: Rect,
    pub rect_header: Rect,
    pub rect_call_banner: Option<Rect>,
    pub rect_member_toggle: Rect,
    pub rect_command_palette: Rect,

    // Should quit
    pub should_quit: bool,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let theme = Theme::from_config(&config.theme);
        let has_token = config.token.as_deref().map(|t| !t.trim().is_empty()).unwrap_or(false);
        let show_login = !has_token;

        let mut state = Self {
            config,
            theme,
            focus: Focus::Chat,
            active_rail: ActiveRail::DirectMessages,
            active_server_id: None,
            active_channel_id: None,

            selected_server_index: 0,
            selected_channel_index: 0,
            selected_member_index: 0,

            chat_scroll_offset: 0,

            input_buffer: String::new(),
            input_cursor: 0,

            show_member_rail: true,
            show_command_palette: false,
            command_palette_query: String::new(),
            command_palette_selected: 0,

            show_login_modal: show_login,
            login_tab: LoginTab::EmailPassword,
            login_step: LoginStep::Credentials,
            login_field_index: 0,
            login_email_input: String::new(),
            login_password_input: String::new(),
            login_totp_input: String::new(),
            login_token_input: String::new(),
            login_error: None,
            login_loading: false,

            gateway_status: if show_login {
                GatewayStatus::Disconnected("Not logged in".to_string())
            } else {
                GatewayStatus::Connecting
            },
            status_message: if show_login {
                Some(("Please log in to connect".to_string(), Utc::now()))
            } else {
                Some(("Connecting...".to_string(), Utc::now()))
            },

            current_user: None,
            users: HashMap::new(),
            servers: Vec::new(),
            channels: HashMap::new(),
            server_channels: HashMap::new(),
            dms: Vec::new(),
            members: HashMap::new(),
            messages: HashMap::new(),
            unread_counts: HashMap::new(),
            active_call: None,
            call_fullscreen: false,
            voice_channel_participants: HashMap::new(),
            pending_user_fetches: Arc::new(Mutex::new(HashSet::new())),
            user_fetch_tx: None,

            rect_rail: Rect::default(),
            rect_server_items: Vec::new(),
            rect_channels: Rect::default(),
            rect_channel_items: Vec::new(),
            rect_chat: Rect::default(),
            rect_input: Rect::default(),
            rect_members: Rect::default(),
            rect_header: Rect::default(),
            rect_call_banner: None,
            rect_member_toggle: Rect::default(),
            rect_command_palette: Rect::default(),

            should_quit: false,
        };

        if show_login {
            // Load demo data by default so the user sees a beautiful fully functional UI immediately
            state.load_demo_data();
        }

        state
    }

    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status_message = Some((text.into(), Utc::now()));
    }

    /// Resolve a user from the cache, or dispatch a non-blocking background fetch if missing.
    pub fn resolve_user(&self, user_id: &str) -> Option<&User> {
        if let Some(user) = self.users.get(user_id) {
            return Some(user);
        }

        // Trigger background fetch if not already in-flight
        if let Ok(mut pending) = self.pending_user_fetches.lock() {
            if pending.insert(user_id.to_string()) {
                if let Some(ref tx) = self.user_fetch_tx {
                    let _ = tx.send(user_id.to_string());
                }
            }
        }

        None
    }

    /// Clear all mock demo data (servers, channels, mock messages, demo users) on real authentication
    pub fn clear_mock_data(&mut self) {
        self.servers.retain(|s| s.id != "srv_stoat" && s.id != "srv_rust");
        self.channels.retain(|id, _| !id.starts_with("ch_stoat_") && !id.starts_with("ch_rust_") && !id.starts_with("dm_"));
        self.server_channels.retain(|sid, _| sid != "srv_stoat" && sid != "srv_rust");
        self.messages.retain(|cid, _| !cid.starts_with("ch_stoat_") && !cid.starts_with("ch_rust_") && !cid.starts_with("dm_"));
        self.members.retain(|sid, _| sid != "srv_stoat" && sid != "srv_rust");
        self.dms.retain(|ch| !ch.id.starts_with("dm_"));
        self.unread_counts.retain(|cid, _| !cid.starts_with("ch_stoat_") && !cid.starts_with("ch_rust_"));
        self.users.retain(|uid, _| uid != "user_me" && uid != "user_alice" && uid != "user_bob" && uid != "user_charlie" && uid != "user_dana" && uid != "user_eve");
        if let Some(ref sid) = self.active_server_id {
            if sid == "srv_stoat" || sid == "srv_rust" {
                self.active_server_id = None;
                self.active_channel_id = None;
            }
        }
    }

    pub fn active_channel(&self) -> Option<&Channel> {
        self.active_channel_id.as_ref().and_then(|id| self.channels.get(id))
    }

    #[allow(dead_code)]
    pub fn active_server(&self) -> Option<&Server> {
        self.active_server_id.as_ref().and_then(|id| self.servers.iter().find(|s| &s.id == id))
    }

    pub fn current_channel_ids(&self) -> Vec<String> {
        if let Some(ref sid) = self.active_server_id {
            self.server_channels.get(sid).cloned().unwrap_or_default()
        } else {
            self.dms.iter().map(|d| d.id.clone()).collect()
        }
    }

    pub fn select_server(&mut self, server_id: Option<String>) {
        if let Some(id) = server_id {
            self.active_rail = ActiveRail::Server(id.clone());
            self.active_server_id = Some(id.clone());

            // Switch to first channel of this server if available (preferring text channel)
            if let Some(chan_ids) = self.server_channels.get(&id) {
                let first_text = chan_ids.iter().find(|cid| {
                    self.channels.get(*cid).map(|c| c.channel_type == "TextChannel" || c.channel_type == "Text").unwrap_or(false)
                }).or_else(|| chan_ids.first());

                if let Some(first) = first_text {
                    self.active_channel_id = Some(first.clone());
                    self.chat_scroll_offset = 0;
                    self.selected_channel_index = 0;
                }
            }
        } else {
            self.active_rail = ActiveRail::DirectMessages;
            self.active_server_id = None;
            if let Some(first_dm) = self.dms.first() {
                self.active_channel_id = Some(first_dm.id.clone());
                self.chat_scroll_offset = 0;
                self.selected_channel_index = 0;
            }
        }
    }

    pub fn select_channel(&mut self, channel_id: String) {
        if self.channels.contains_key(&channel_id) {
            self.active_channel_id = Some(channel_id.clone());
            self.chat_scroll_offset = 0;
            // Clear unread badge for this channel
            self.unread_counts.remove(&channel_id);
        }
    }

    // Input handling
    pub fn input_insert_char(&mut self, c: char) {
        let mut byte_idx = 0;
        for (i, (b_idx, _)) in self.input_buffer.char_indices().enumerate() {
            if i == self.input_cursor {
                byte_idx = b_idx;
                break;
            }
            byte_idx = self.input_buffer.len();
        }
        self.input_buffer.insert(byte_idx, c);
        self.input_cursor += 1;
    }

    pub fn input_insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.input_insert_char(c);
        }
    }

    pub fn input_backspace(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
            let mut byte_idx = 0;
            for (i, (b_idx, _)) in self.input_buffer.char_indices().enumerate() {
                if i == self.input_cursor {
                    byte_idx = b_idx;
                    break;
                }
            }
            if byte_idx < self.input_buffer.len() {
                self.input_buffer.remove(byte_idx);
            }
        }
    }

    pub fn input_delete(&mut self) {
        let mut byte_idx = 0;
        let mut found = false;
        for (i, (b_idx, _)) in self.input_buffer.char_indices().enumerate() {
            if i == self.input_cursor {
                byte_idx = b_idx;
                found = true;
                break;
            }
        }
        if found && byte_idx < self.input_buffer.len() {
            self.input_buffer.remove(byte_idx);
        }
    }

    pub fn input_cursor_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
        }
    }

    pub fn input_cursor_right(&mut self) {
        let char_count = self.input_buffer.chars().count();
        if self.input_cursor < char_count {
            self.input_cursor += 1;
        }
    }

    pub fn input_cursor_home(&mut self) {
        self.input_cursor = 0;
    }

    pub fn input_cursor_end(&mut self) {
        self.input_cursor = self.input_buffer.chars().count();
    }

    pub fn input_clear(&mut self) -> String {
        let msg = self.input_buffer.clone();
        self.input_buffer.clear();
        self.input_cursor = 0;
        msg
    }

    // Palette searching
    pub fn get_palette_items(&self) -> Vec<PaletteItem> {
        let matcher = SkimMatcherV2::default();
        let query = self.command_palette_query.trim().to_lowercase();

        let mut items = Vec::new();

        // 1. Direct Messages
        for dm in &self.dms {
            let title = dm.display_name(&self.users, self.current_user.as_ref().map(|u| u.id.as_str()));
            items.push(PaletteItem {
                id: dm.id.clone(),
                title: format!("@ {}", title),
                subtitle: "Direct Message".to_string(),
                item_type: PaletteItemType::DirectMessage,
                server_id: None,
            });
        }

        // 2. Servers
        for server in &self.servers {
            items.push(PaletteItem {
                id: server.id.clone(),
                title: server.name.clone(),
                subtitle: "Server".to_string(),
                item_type: PaletteItemType::Server,
                server_id: Some(server.id.clone()),
            });

            // 3. Channels in servers
            if let Some(ch_ids) = self.server_channels.get(&server.id) {
                for cid in ch_ids {
                    if let Some(ch) = self.channels.get(cid) {
                        let name = ch.name.as_deref().unwrap_or("channel");
                        let prefix = if ch.channel_type == "VoiceChannel" { "🔊" } else { "#" };
                        items.push(PaletteItem {
                            id: ch.id.clone(),
                            title: format!("{} {}", prefix, name),
                            subtitle: format!("in {}", server.name),
                            item_type: PaletteItemType::Channel,
                            server_id: Some(server.id.clone()),
                        });
                    }
                }
            }
        }

        if query.is_empty() {
            return items;
        }

        let mut scored_items: Vec<(i64, PaletteItem)> = items
            .into_iter()
            .filter_map(|item| {
                let score1 = matcher.fuzzy_match(&item.title.to_lowercase(), &query);
                let score2 = matcher.fuzzy_match(&item.subtitle.to_lowercase(), &query);
                let best = score1.max(score2);
                best.map(|score| (score, item))
            })
            .collect();

        scored_items.sort_by(|a, b| b.0.cmp(&a.0));
        scored_items.into_iter().map(|(_, item)| item).collect()
    }

    pub fn load_demo_data(&mut self) {
        self.users.clear();
        self.servers.clear();
        self.channels.clear();
        self.server_channels.clear();
        self.dms.clear();
        self.members.clear();
        self.messages.clear();
        self.unread_counts.clear();

        // Current user
        self.current_user = Some(User {
            id: "user_me".to_string(),
            username: "rustacean".to_string(),
            display_name: Some("Ferris".to_string()),
            status: Some(UserStatus {
                text: Some("Hacking in Rust & Ratatui 🦀".to_string()),
                presence: Some(Presence::Online),
            }),
            online: Some(true),
            ..Default::default()
        });

        // Other users
        let users_data = vec![
            ("user_alice", "alice", "Alice Wonder", Presence::Online, "Building Stoat TUI client"),
            ("user_bob", "bob_dev", "Bob Builder", Presence::Idle, "Afk getting coffee ☕"),
            ("user_charlie", "charlie_z", "Charlie", Presence::Busy, "Do Not Disturb - Deep coding"),
            ("user_dana", "dana", "Dana Scully", Presence::Offline, ""),
            ("user_eve", "stoat_bot", "Stoat Gateway Bot", Presence::Online, "Automated CI/CD Assistant"),
        ];

        for (uid, uname, dname, presence, stext) in users_data {
            self.users.insert(
                uid.to_string(),
                User {
                    id: uid.to_string(),
                    username: uname.to_string(),
                    display_name: Some(dname.to_string()),
                    status: Some(UserStatus {
                        text: if stext.is_empty() { None } else { Some(stext.to_string()) },
                        presence: Some(presence.clone()),
                    }),
                    online: Some(presence != Presence::Offline),
                    ..Default::default()
                },
            );
        }

        // Servers
        let s1 = Server {
            id: "srv_stoat".to_string(),
            name: "Stoat Official".to_string(),
            description: Some("Official community server for the Stoat chat platform".to_string()),
            channels: vec!["ch_stoat_welcome".to_string(), "ch_stoat_general".to_string(), "ch_stoat_dev".to_string(), "ch_stoat_voice".to_string()],
            categories: Some(vec![
                Category {
                    id: "cat_welcome".to_string(),
                    title: "WELCOME".to_string(),
                    channels: vec!["ch_stoat_welcome".to_string()],
                },
                Category {
                    id: "cat_text".to_string(),
                    title: "TEXT CHANNELS".to_string(),
                    channels: vec!["ch_stoat_general".to_string(), "ch_stoat_dev".to_string()],
                },
                Category {
                    id: "cat_voice".to_string(),
                    title: "VOICE LOUNGE".to_string(),
                    channels: vec!["ch_stoat_voice".to_string()],
                },
            ]),
            roles: Some({
                let mut r = HashMap::new();
                r.insert("role_admin".to_string(), Role { name: "Core Team".to_string(), colour: Some("#FD6671".to_string()), rank: Some(10) });
                r.insert("role_mod".to_string(), Role { name: "Moderator".to_string(), colour: Some("#5970FD".to_string()), rank: Some(5) });
                r.insert("role_member".to_string(), Role { name: "Member".to_string(), colour: None, rank: Some(1) });
                r
            }),
            ..Default::default()
        };

        let s2 = Server {
            id: "srv_rust".to_string(),
            name: "Rustaceans Lounge".to_string(),
            description: Some("Async systems programming & terminal UI craftsmanship".to_string()),
            channels: vec!["ch_rust_general".to_string(), "ch_rust_ratatui".to_string()],
            categories: Some(vec![
                Category {
                    id: "cat_rust".to_string(),
                    title: "RUST DISCUSSIONS".to_string(),
                    channels: vec!["ch_rust_general".to_string(), "ch_rust_ratatui".to_string()],
                },
            ]),
            roles: None,
            ..Default::default()
        };

        // Channels
        let ch_list = vec![
            Channel {
                id: "ch_stoat_welcome".to_string(),
                channel_type: "TextChannel".to_string(),
                server: Some("srv_stoat".to_string()),
                name: Some("announcements".to_string()),
                description: Some("Official announcements, releases, and platform updates".to_string()),
                ..Default::default()
            },
            Channel {
                id: "ch_stoat_general".to_string(),
                channel_type: "TextChannel".to_string(),
                server: Some("srv_stoat".to_string()),
                name: Some("general".to_string()),
                description: Some("General discussion for all Stoat community members".to_string()),
                ..Default::default()
            },
            Channel {
                id: "ch_stoat_dev".to_string(),
                channel_type: "TextChannel".to_string(),
                server: Some("srv_stoat".to_string()),
                name: Some("development".to_string()),
                description: Some("Architecture, API discussion, and Bonfire gateway debugging".to_string()),
                ..Default::default()
            },
            Channel {
                id: "ch_stoat_voice".to_string(),
                channel_type: "VoiceChannel".to_string(),
                server: Some("srv_stoat".to_string()),
                name: Some("Lounge 🔊".to_string()),
                description: Some("Voice hangout for developers".to_string()),
                ..Default::default()
            },
            Channel {
                id: "ch_rust_general".to_string(),
                channel_type: "TextChannel".to_string(),
                server: Some("srv_rust".to_string()),
                name: Some("general-rust".to_string()),
                description: Some("Chat about Rust 2024 edition, tokio, and ownership".to_string()),
                ..Default::default()
            },
            Channel {
                id: "ch_rust_ratatui".to_string(),
                channel_type: "TextChannel".to_string(),
                server: Some("srv_rust".to_string()),
                name: Some("ratatui-dev".to_string()),
                description: Some("Building blazing fast, gorgeous terminal interfaces".to_string()),
                ..Default::default()
            },
            // DMs
            Channel {
                id: "dm_alice".to_string(),
                channel_type: "DirectMessage".to_string(),
                recipients: Some(vec!["user_me".to_string(), "user_alice".to_string()]),
                name: None,
                ..Default::default()
            },
            Channel {
                id: "dm_bob".to_string(),
                channel_type: "DirectMessage".to_string(),
                recipients: Some(vec!["user_me".to_string(), "user_bob".to_string()]),
                name: None,
                ..Default::default()
            },
        ];

        for ch in ch_list {
            if let Some(ref sid) = ch.server {
                self.server_channels.entry(sid.clone()).or_default().push(ch.id.clone());
            } else if ch.channel_type == "DirectMessage" {
                self.dms.push(ch.clone());
            }
            self.channels.insert(ch.id.clone(), ch);
        }

        self.servers = vec![s1, s2];

        // Members
        let s1_members = vec![
            Member {
                id: MemberId { server: "srv_stoat".to_string(), user: "user_alice".to_string() },
                nickname: Some("Alice [Core]".to_string()),
                roles: Some(vec!["role_admin".to_string()]),
                joined_at: Some("2026-01-01T00:00:00Z".to_string()),
                ..Default::default()
            },
            Member {
                id: MemberId { server: "srv_stoat".to_string(), user: "user_bob".to_string() },
                nickname: Some("Bob".to_string()),
                roles: Some(vec!["role_mod".to_string()]),
                ..Default::default()
            },
            Member {
                id: MemberId { server: "srv_stoat".to_string(), user: "user_charlie".to_string() },
                nickname: Some("Charlie".to_string()),
                roles: Some(vec!["role_member".to_string()]),
                ..Default::default()
            },
            Member {
                id: MemberId { server: "srv_stoat".to_string(), user: "user_dana".to_string() },
                nickname: Some("Dana".to_string()),
                roles: Some(vec!["role_member".to_string()]),
                ..Default::default()
            },
            Member {
                id: MemberId { server: "srv_stoat".to_string(), user: "user_eve".to_string() },
                nickname: Some("Stoat Bot [BOT]".to_string()),
                roles: Some(vec!["role_mod".to_string()]),
                ..Default::default()
            },
            Member {
                id: MemberId { server: "srv_stoat".to_string(), user: "user_me".to_string() },
                nickname: Some("Ferris 🦀".to_string()),
                roles: Some(vec!["role_admin".to_string()]),
                ..Default::default()
            },
        ];
        self.members.insert("srv_stoat".to_string(), s1_members);

        // Messages for ch_stoat_general
        let mut msg_reactions1 = HashMap::new();
        msg_reactions1.insert("🔥".to_string(), vec!["user_bob".to_string(), "user_charlie".to_string(), "user_me".to_string()]);
        msg_reactions1.insert("🦀".to_string(), vec!["user_me".to_string(), "user_alice".to_string()]);

        let msgs = vec![
            Message {
                id: "msg_1".to_string(),
                channel: "ch_stoat_general".to_string(),
                author: "user_alice".to_string(),
                content: Some("Welcome to **Stoat TUI**! Built with **Ratatui** + **Tokio** in 100% Rust.\n> Fast, responsive, keyboard-first, and fully mouse-accessible!".to_string()),
                reactions: Some(msg_reactions1),
                ..Default::default()
            },
            Message {
                id: "msg_2".to_string(),
                channel: "ch_stoat_general".to_string(),
                author: "user_bob".to_string(),
                content: Some("The layout is ultra slick! Love the 3-column Stoat desktop aesthetic.\nCheck out this snippet:\n```rust\nfn connect_gateway() -> Result<()> {\n    let ws = BonfireClient::connect(\"wss://ws.stoat.chat\");\n    println!(\"Connected!\");\n    Ok(())\n}\n```".to_string()),
                ..Default::default()
            },
            Message {
                id: "msg_3".to_string(),
                channel: "ch_stoat_general".to_string(),
                author: "user_charlie".to_string(),
                content: Some("You can cycle panes with `Tab` or jump directly with `Alt+1`, `Alt+2`, `Alt+3`.\nPress `Ctrl+P` anytime to open the fuzzy command palette!".to_string()),
                ..Default::default()
            },
            Message {
                id: "msg_4".to_string(),
                channel: "ch_stoat_general".to_string(),
                author: "user_eve".to_string(),
                content: Some("[BOT] Bonfire gateway heartbeat latency: **18ms**. All systems operational.\nVisit https://stoat.chat for documentation and API specifications.".to_string()),
                ..Default::default()
            },
        ];

        self.messages.insert("ch_stoat_general".to_string(), msgs);

        // Messages for announcement
        self.messages.insert("ch_stoat_welcome".to_string(), vec![
            Message {
                id: "msg_ann_1".to_string(),
                channel: "ch_stoat_welcome".to_string(),
                author: "user_alice".to_string(),
                content: Some("📢 **Stoat TUI v0.1.0 Released**\n\nFull mouse support, instant channel switching, markdown rendering, and live Bonfire sync enabled!".to_string()),
                ..Default::default()
            }
        ]);

        // Messages for DM
        self.messages.insert("dm_alice".to_string(), vec![
            Message {
                id: "msg_dm_1".to_string(),
                channel: "dm_alice".to_string(),
                author: "user_alice".to_string(),
                content: Some("Hey Ferris! Testing direct messages in the Stoat terminal client.".to_string()),
                ..Default::default()
            }
        ]);

        // Unread badge on dev channel
        self.unread_counts.insert("ch_stoat_dev".to_string(), 3);
        self.unread_counts.insert("dm_bob".to_string(), 1);

        // Default selection
        self.active_rail = ActiveRail::Server("srv_stoat".to_string());
        self.active_server_id = Some("srv_stoat".to_string());
        self.active_channel_id = Some("ch_stoat_general".to_string());
        self.gateway_status = GatewayStatus::Connected { latency_ms: 18 };
    }
}
