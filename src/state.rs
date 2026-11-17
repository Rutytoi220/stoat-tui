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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAttachment {
    pub id: String,
    pub filename: String,
}

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

    // Input buffer & pending uploads
    pub input_buffer: String,
    pub input_cursor: usize,
    pub pending_attachments: Vec<PendingAttachment>,

    // UI Toggles & Modals
    pub show_member_rail: bool,
    pub member_rail_explicitly_toggled: bool,
    pub left_sidebar_width: u16,
    pub show_command_palette: bool,
    pub command_palette_query: String,
    pub command_palette_selected: usize,

    pub show_settings_modal: bool,
    pub settings_selected_index: usize,

    pub show_screen_picker_modal: bool,
    pub screen_picker_targets: Vec<crate::voice::screen::ScreenShareTarget>,
    pub screen_picker_selected: usize,

    pub show_login_modal: bool,
    pub is_demo_mode: bool,
    pub login_tab: LoginTab,
    pub login_step: LoginStep,
    pub login_field_index: usize, // 0 = email/token, 1 = password
    pub login_email_input: String,
    pub login_password_input: String,
    pub login_totp_input: String,
    pub login_token_input: String,
    pub login_api_input: String,
    pub login_add_account: bool,
    pub login_error: Option<String>,
    pub login_loading: bool,

    // Multi-Account / Multi-Instance Tracking
    pub active_account_id: String,
    pub account_api_urls: HashMap<String, String>,
    pub account_labels: HashMap<String, Option<String>>,
    pub account_users: HashMap<String, User>,
    pub account_autumn_urls: HashMap<String, String>,
    pub account_livekit: HashMap<String, (Option<String>, Option<String>)>,
    pub account_gateway_statuses: HashMap<String, GatewayStatus>,
    pub server_accounts: HashMap<String, String>,
    pub channel_accounts: HashMap<String, String>,
    pub user_accounts: HashMap<String, String>,

    // Gateway / network
    pub gateway_status: GatewayStatus,
    pub status_message: Option<(String, chrono::DateTime<Utc>)>,
    pub cached_ws_url: Option<String>,

    // Data stores
    pub current_user: Option<User>,
    pub users: HashMap<String, User>,
    pub servers: Vec<Server>,
    pub channels: HashMap<String, Channel>,
    pub server_channels: HashMap<String, Vec<String>>,
    pub dms: Vec<Channel>,
    pub members: HashMap<String, Vec<Member>>,
    pub user_profiles: HashMap<String, crate::api::models::UserProfile>,
    pub messages: HashMap<String, Vec<Message>>,
    pub unread_counts: HashMap<String, usize>,

    // Voice & Video Call Subsystem
    pub active_call: Option<ActiveCallState>,
    pub call_fullscreen: bool,
    pub voice_channel_participants: HashMap<String, Vec<VoiceParticipant>>,
    pub livekit_node_name: Option<String>,
    pub livekit_url: Option<String>,
    pub call_fallback_names: HashMap<String, String>,

    // Dynamic user resolution cache
    pub pending_user_fetches: Arc<Mutex<HashSet<String>>>,
    pub user_fetch_tx: Option<mpsc::UnboundedSender<(String, String)>>,

    // Autumn CDN & Async Image Cache
    pub autumn_url: String,
    pub image_cache: Option<crate::ui::images::ImageCache>,
    pub image_fetch_tx: Option<mpsc::UnboundedSender<(String, String)>>, // (key, url)
    pub deferred_image_renders: Vec<(Rect, String)>,

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

    // Login Modal Hit-Testing
    pub rect_login_tabs: Vec<(LoginTab, Rect)>,
    pub rect_login_fields: Vec<(usize, Rect)>,
    pub rect_login_submit_btn: Option<Rect>,

    // Should quit
    pub should_quit: bool,
}

impl AppState {
    pub fn new(mut config: Config) -> Self {
        config.migrate_legacy_account();
        let theme = Theme::from_config(&config.theme);
        let has_token = !config.accounts.is_empty()
            || config.token.as_deref().map(|t| !t.trim().is_empty()).unwrap_or(false);
        let show_login = !has_token;
        let left_sidebar_width = config.left_sidebar_width.clamp(16, 52);
        let initial_api = config.api_url.clone();

        let active_account_id = config
            .accounts
            .first()
            .map(|a| a.id.clone())
            .unwrap_or_else(|| "primary".to_string());
        let mut account_api_urls = HashMap::new();
        let mut account_labels = HashMap::new();
        for acc in &config.accounts {
            account_api_urls.insert(acc.id.clone(), acc.api_url.clone());
            account_labels.insert(acc.id.clone(), crate::config::short_instance_label(&acc.api_url));
        }
        account_api_urls
            .entry(active_account_id.clone())
            .or_insert_with(|| initial_api.clone());
        account_labels
            .entry(active_account_id.clone())
            .or_insert_with(|| crate::config::short_instance_label(&initial_api));

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
            pending_attachments: Vec::new(),

            show_member_rail: true,
            member_rail_explicitly_toggled: false,
            left_sidebar_width,
            show_command_palette: false,
            command_palette_query: String::new(),
            command_palette_selected: 0,

            show_settings_modal: false,
            settings_selected_index: 0,

            show_screen_picker_modal: false,
            screen_picker_targets: Vec::new(),
            screen_picker_selected: 0,

            show_login_modal: show_login,
            is_demo_mode: false,
            login_tab: LoginTab::EmailPassword,
            login_step: LoginStep::Credentials,
            login_field_index: 0,
            login_email_input: String::new(),
            login_password_input: String::new(),
            login_totp_input: String::new(),
            login_token_input: String::new(),
            login_api_input: initial_api,
            login_add_account: has_token,
            login_error: None,
            login_loading: false,

            active_account_id,
            account_api_urls,
            account_labels,
            account_users: HashMap::new(),
            account_autumn_urls: HashMap::new(),
            account_livekit: HashMap::new(),
            account_gateway_statuses: HashMap::new(),
            server_accounts: HashMap::new(),
            channel_accounts: HashMap::new(),
            user_accounts: HashMap::new(),

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
            cached_ws_url: None,

            current_user: None,
            users: HashMap::new(),
            servers: Vec::new(),
            channels: HashMap::new(),
            server_channels: HashMap::new(),
            dms: Vec::new(),
            members: HashMap::new(),
            user_profiles: HashMap::new(),
            messages: HashMap::new(),
            unread_counts: HashMap::new(),
            active_call: None,
            call_fullscreen: false,
            voice_channel_participants: HashMap::new(),
            livekit_node_name: None,
            livekit_url: None,
            call_fallback_names: HashMap::new(),
            pending_user_fetches: Arc::new(Mutex::new(HashSet::new())),
            user_fetch_tx: None,

            autumn_url: "https://cdn.stoatusercontent.com".to_string(),
            image_cache: None,
            image_fetch_tx: None,
            deferred_image_renders: Vec::new(),

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

            rect_login_tabs: Vec::new(),
            rect_login_fields: Vec::new(),
            rect_login_submit_btn: None,

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

    pub fn resolve_call_participant_name(&self, user_id: &str) -> String {
        if let Some(u) = self.users.get(user_id) {
            return u.name().to_string();
        }
        if let Some(fb) = self.call_fallback_names.get(user_id) {
            let trimmed = fb.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        user_id.chars().take(6).collect()
    }

    pub fn current_accent_preset_index(&self) -> usize {
        crate::config::ACCENT_PRESETS
            .iter()
            .position(|(_, hex)| hex.eq_ignore_ascii_case(&self.config.theme.primary))
            .unwrap_or(0)
    }

    pub fn set_accent_preset(&mut self, index: usize) {
        let len = crate::config::ACCENT_PRESETS.len();
        if len == 0 {
            return;
        }
        let (_, hex) = crate::config::ACCENT_PRESETS[index % len];
        self.config.theme.primary = hex.to_string();
        self.theme = Theme::from_config(&self.config.theme);
    }

    /// Hydrate server/DM navigation lists and last active channel from disk cache for Frame 1 paint.
    pub fn hydrate_nav_cache(&mut self, cache: crate::config::NavCache) {
        if cache.api_url != self.config.api_url {
            return;
        }
        if let Some(ws) = cache.ws_url {
            if !ws.trim().is_empty() {
                self.cached_ws_url = Some(ws);
            }
        }
        if let Some(autumn) = cache.autumn_url {
            if !autumn.trim().is_empty() {
                self.autumn_url = autumn;
            }
        }

        self.users.clear();
        for u in cache.users {
            self.users.insert(u.id.clone(), u);
        }
        self.user_accounts = cache.user_accounts;

        self.servers = cache.servers;
        self.server_accounts = cache.server_accounts;
        self.channels.clear();
        self.server_channels.clear();
        for ch in cache.channels {
            if let Some(ref sid) = ch.server {
                if !sid.trim().is_empty() {
                    self.server_channels.entry(sid.clone()).or_default().push(ch.id.clone());
                }
            }
            self.channels.insert(ch.id.clone(), ch);
        }
        self.channel_accounts = cache.channel_accounts;
        for s in &self.servers {
            let entry = self.server_channels.entry(s.id.clone()).or_default();
            for cid in &s.channels {
                if !entry.contains(cid) {
                    entry.push(cid.clone());
                }
            }
        }
        self.dms = cache.dms;
        for dm in &self.dms {
            self.channels.entry(dm.id.clone()).or_insert_with(|| dm.clone());
        }

        // Restore last active rail & channel immediately for frame 1 header/sidebar
        if self.config.last_in_dm {
            self.active_rail = ActiveRail::DirectMessages;
            self.active_server_id = None;
            self.selected_server_index = 0;
            if let Some(ref lcid) = self.config.last_channel_id {
                if self.channels.contains_key(lcid) {
                    self.active_channel_id = Some(lcid.clone());
                }
            }
            if self.active_channel_id.is_none() {
                self.active_channel_id = self.dms.first().map(|d| d.id.clone());
            }
        } else if let Some(ref saved_sid) = self.config.last_server_id {
            if let Some(pos) = self.servers.iter().position(|s| &s.id == saved_sid) {
                self.active_rail = ActiveRail::Server(saved_sid.clone());
                self.active_server_id = Some(saved_sid.clone());
                self.selected_server_index = pos + 1;
                if let Some(ref lcid) = self.config.last_channel_id {
                    if self.channels.contains_key(lcid) {
                        self.active_channel_id = Some(lcid.clone());
                    }
                }
                if self.active_channel_id.is_none() {
                    if let Some(cids) = self.server_channels.get(saved_sid) {
                        self.active_channel_id = cids
                            .iter()
                            .find(|cid| {
                                self.channels
                                    .get(*cid)
                                    .map(|c| c.channel_type == "TextChannel")
                                    .unwrap_or(false)
                            })
                            .or_else(|| cids.first())
                            .cloned();
                    }
                }
            }
        }

        if self.active_channel_id.is_none() {
            if let Some(first_srv) = self.servers.first() {
                let sid = first_srv.id.clone();
                self.active_rail = ActiveRail::Server(sid.clone());
                self.active_server_id = Some(sid.clone());
                self.selected_server_index = 1;
                if let Some(cids) = self.server_channels.get(&sid) {
                    self.active_channel_id = cids.first().cloned();
                }
            } else if let Some(first_dm) = self.dms.first() {
                self.active_rail = ActiveRail::DirectMessages;
                self.active_server_id = None;
                self.selected_server_index = 0;
                self.active_channel_id = Some(first_dm.id.clone());
            }
        }
    }

    /// Build a compact `NavCache` snapshot (omitting bulky role/message maps).
    pub fn build_nav_cache(&self) -> crate::config::NavCache {
        let mut servers = Vec::with_capacity(self.servers.len());
        for s in &self.servers {
            let mut compact = s.clone();
            compact.roles = None;
            servers.push(compact);
        }
        let channels: Vec<Channel> = self
            .channels
            .values()
            .filter(|c| c.server.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false))
            .cloned()
            .collect();

        let mut dm_user_ids = HashSet::new();
        for dm in &self.dms {
            if let Some(ref recs) = dm.recipients {
                for r in recs {
                    dm_user_ids.insert(r.clone());
                }
            }
        }
        if let Some(ref me) = self.current_user {
            dm_user_ids.insert(me.id.clone());
        }
        let users: Vec<User> = dm_user_ids
            .iter()
            .filter_map(|uid| self.users.get(uid).cloned())
            .collect();

        crate::config::NavCache {
            api_url: self.config.api_url.clone(),
            ws_url: self.cached_ws_url.clone(),
            autumn_url: Some(self.autumn_url.clone()),
            servers,
            channels,
            dms: self.dms.clone(),
            users,
            server_accounts: self.server_accounts.clone(),
            channel_accounts: self.channel_accounts.clone(),
            user_accounts: self.user_accounts.clone(),
        }
    }

    pub fn account_for_channel(&self, channel_id: &str) -> &str {
        self.channel_accounts
            .get(channel_id)
            .map(String::as_str)
            .unwrap_or(self.active_account_id.as_str())
    }

    pub fn account_for_server(&self, server_id: &str) -> &str {
        self.server_accounts
            .get(server_id)
            .map(String::as_str)
            .unwrap_or(self.active_account_id.as_str())
    }

    pub fn account_for_user(&self, user_id: &str) -> &str {
        self.user_accounts
            .get(user_id)
            .map(String::as_str)
            .unwrap_or(self.active_account_id.as_str())
    }

    pub fn account_for_active_channel(&self) -> &str {
        if let Some(ref cid) = self.active_channel_id {
            self.account_for_channel(cid)
        } else if let Some(ref sid) = self.active_server_id {
            self.account_for_server(sid)
        } else {
            self.active_account_id.as_str()
        }
    }

    pub fn instance_tag_for_account(&self, account_id: &str) -> Option<&str> {
        self.account_labels
            .get(account_id)
            .and_then(|opt| opt.as_deref())
    }

    pub fn autumn_url_for_account(&self, account_id: &str) -> String {
        if let Some(u) = self.account_autumn_urls.get(account_id) {
            if !u.trim().is_empty() {
                return u.clone();
            }
        }
        if let Some(api_url) = self.account_api_urls.get(account_id) {
            let host = crate::config::instance_host(api_url);
            if !host.is_empty() && host != "api.stoat.chat" && host != "api.revolt.chat" {
                return crate::api::DeltaClient::resolve_autumn_url(api_url, None);
            }
        }
        self.autumn_url.clone()
    }

    pub fn api_url_for_account(&self, account_id: &str) -> &str {
        self.account_api_urls
            .get(account_id)
            .map(String::as_str)
            .unwrap_or(self.config.api_url.as_str())
    }

    pub fn current_user_for_account(&self, account_id: &str) -> Option<&User> {
        self.account_users
            .get(account_id)
            .or(self.current_user.as_ref())
    }

    pub fn purge_account_items(&mut self, account_id: &str) {
        let active_acc = &self.active_account_id;
        let server_accounts = &self.server_accounts;
        let channel_accounts = &self.channel_accounts;

        let old_server_ids: Vec<String> = self
            .servers
            .iter()
            .filter(|s| {
                server_accounts
                    .get(&s.id)
                    .map(String::as_str)
                    .unwrap_or(active_acc.as_str())
                    == account_id
            })
            .map(|s| s.id.clone())
            .collect();

        self.servers.retain(|s| {
            server_accounts
                .get(&s.id)
                .map(String::as_str)
                .unwrap_or(active_acc.as_str())
                != account_id
        });
        for sid in old_server_ids {
            self.server_channels.remove(&sid);
            self.members.remove(&sid);
        }
        self.dms.retain(|d| {
            channel_accounts
                .get(&d.id)
                .map(String::as_str)
                .unwrap_or(active_acc.as_str())
                != account_id
        });
        self.channels.retain(|cid, _| {
            channel_accounts
                .get(cid)
                .map(String::as_str)
                .unwrap_or(active_acc.as_str())
                != account_id
        });
    }

    /// Resolve a user from the cache, or dispatch a non-blocking background fetch on a specific account.
    pub fn resolve_user_in_account(&self, account_id: &str, user_id: &str) -> Option<&User> {
        if let Some(user) = self.users.get(user_id) {
            return Some(user);
        }

        if let Ok(mut pending) = self.pending_user_fetches.lock() {
            if pending.insert(user_id.to_string()) {
                if let Some(ref tx) = self.user_fetch_tx {
                    let _ = tx.send((account_id.to_string(), user_id.to_string()));
                }
            }
        }

        None
    }

    /// Resolve a user from the cache, or dispatch a non-blocking background fetch if missing.
    pub fn resolve_user(&self, user_id: &str) -> Option<&User> {
        let acc = self.account_for_user(user_id);
        self.resolve_user_in_account(acc, user_id)
    }

    /// Clear all mock demo data (servers, channels, mock messages, demo users) on real authentication
    pub fn clear_mock_data(&mut self) {
        self.is_demo_mode = false;
        self.servers.retain(|s| s.id != "srv_stoat" && s.id != "srv_rust");
        self.channels.retain(|id, _| !id.starts_with("ch_stoat_") && !id.starts_with("ch_rust_") && id != "dm_alice" && id != "dm_bob");
        self.server_channels.retain(|sid, _| sid != "srv_stoat" && sid != "srv_rust");
        self.messages.retain(|cid, _| !cid.starts_with("ch_stoat_") && !cid.starts_with("ch_rust_") && cid != "dm_alice" && cid != "dm_bob");
        self.members.retain(|sid, _| sid != "srv_stoat" && sid != "srv_rust");
        self.dms.retain(|ch| ch.id != "dm_alice" && ch.id != "dm_bob");
        self.unread_counts.retain(|cid, _| !cid.starts_with("ch_stoat_") && !cid.starts_with("ch_rust_"));
        self.users.retain(|uid, _| uid != "user_me" && uid != "user_alice" && uid != "user_bob" && uid != "user_charlie" && uid != "user_dana" && uid != "user_eve");
        if let Some(ref sid) = self.active_server_id {
            if sid == "srv_stoat" || sid == "srv_rust" {
                self.active_server_id = None;
                self.active_channel_id = None;
            }
        }
    }

    pub fn next_nonce(&self) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NONCE_SEQ: AtomicU64 = AtomicU64::new(1);
        let seq = NONCE_SEQ.fetch_add(1, Ordering::Relaxed);
        format!("stoat_tui_{}_{}", Utc::now().timestamp_millis(), seq)
    }

    /// Upserts a message into `self.messages[channel_id]`, deduplicating by canonical `_id` and client `nonce`.
    pub fn upsert_message(&mut self, channel_id: &str, msg: Message) {
        let list = self.messages.entry(channel_id.to_string()).or_default();

        if !msg.id.is_empty() {
            if let Some(pos) = list.iter().position(|m| m.id == msg.id) {
                let prev_nonce = list[pos].nonce.clone();
                list[pos] = msg;
                if list[pos].nonce.is_none() {
                    list[pos].nonce = prev_nonce;
                }
                return;
            }
        }

        if let Some(ref n) = msg.nonce {
            if !n.is_empty() {
                let opt_id = format!("optimistic:{}", n);
                if let Some(pos) = list
                    .iter()
                    .position(|m| m.nonce.as_deref() == Some(n.as_str()) || m.id == opt_id)
                {
                    list[pos] = msg;
                    return;
                }
            }
        }

        list.push(msg);
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
        self.is_demo_mode = true;
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

    pub fn avatar_url_for_user_in_account(&self, account_id: &str, user: &User) -> String {
        let autumn_binding = self.autumn_url_for_account(account_id);
        let autumn = autumn_binding.trim_end_matches('/');
        if let Some(ref av) = user.avatar {
            if !av.id.is_empty() {
                return format!("{}/avatars/{}?max_side=128", autumn, av.id);
            }
        }
        let api = self.api_url_for_account(account_id).trim_end_matches('/');
        format!("{}/users/{}/default_avatar", api, user.id)
    }

    pub fn avatar_url_for_user(&self, user: &User) -> String {
        let acc = self.account_for_user(&user.id).to_string();
        self.avatar_url_for_user_in_account(&acc, user)
    }

    /// Returns an `ImageCache` key containing `account_id`, `user_id`, and `avatar._id`
    /// (or `"default"`) so a cached default avatar never shadows a real uploaded PFP.
    pub fn avatar_cache_key(&self, account_id: &str, user_id: &str, prefix: &str) -> String {
        let acc = if account_id.is_empty() {
            self.account_for_user(user_id)
        } else {
            account_id
        };
        let av_part = self
            .users
            .get(user_id)
            .and_then(|u| u.avatar.as_ref())
            .map(|av| av.id.trim())
            .filter(|id| !id.is_empty())
            .unwrap_or("default");
        format!("{}:{}:{}:{}", prefix, acc, user_id, av_part)
    }

    pub fn server_icon_url(&self, server: &Server) -> Option<String> {
        let acc = self.account_for_server(&server.id);
        let autumn_binding = self.autumn_url_for_account(acc);
        let autumn = autumn_binding.trim_end_matches('/');
        server.icon.as_ref().and_then(|ic| {
            if ic.id.is_empty() {
                None
            } else {
                Some(format!("{}/icons/{}?max_side=128", autumn, ic.id))
            }
        })
    }

    pub fn attachment_image_url(&self, file_id: &str) -> String {
        let acc = self.account_for_active_channel();
        let autumn_binding = self.autumn_url_for_account(acc);
        let autumn = autumn_binding.trim_end_matches('/');
        format!("{}/attachments/{}?max_side=800", autumn, file_id)
    }

    pub fn request_image(&mut self, key: String, url: String) {
        if let Some(ref mut cache) = self.image_cache {
            if !cache.contains(&key) && !cache.is_pending(&key) {
                cache.mark_pending(key.clone());
                if let Some(ref tx) = self.image_fetch_tx {
                    let _ = tx.send((key, url));
                }
            }
        }
    }

    pub fn request_user_avatar(&mut self, user_id: &str) {
        let acc = self.account_for_user(user_id).to_string();
        let key = self.avatar_cache_key(&acc, user_id, "avatar");
        self.request_user_avatar_with_key_in_account(&acc, user_id, &key);
    }

    pub fn request_user_avatar_with_key(&mut self, user_id: &str, key: &str) {
        let acc = self.account_for_user(user_id).to_string();
        self.request_user_avatar_with_key_in_account(&acc, user_id, key);
    }

    pub fn request_user_avatar_with_key_in_account(
        &mut self,
        account_id: &str,
        user_id: &str,
        key: &str,
    ) {
        let eff_acc = if account_id.is_empty() {
            self.account_for_user(user_id).to_string()
        } else {
            self.user_accounts
                .entry(user_id.to_string())
                .or_insert_with(|| account_id.to_string());
            account_id.to_string()
        };
        if let Some(user) = self.users.get(user_id).cloned() {
            let url = self.avatar_url_for_user_in_account(&eff_acc, &user);
            self.request_image(key.to_string(), url);
        } else {
            let _ = self.resolve_user_in_account(&eff_acc, user_id);
        }
    }
}
