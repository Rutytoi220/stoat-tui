use crate::api::models::{LoginResponse, Message};
use crate::api::DeltaClient;
use crate::config::{Config, SavedAccount};
use crate::event::AppEvent;
use crate::gateway::messages::ServerMessage;
use crate::gateway::{BonfireClient, GatewayEvent};
use crate::state::{ActiveRail, AppState, Focus, GatewayStatus, LoginStep, LoginTab, PaletteItemType};
use crate::{log_debug, log_error, log_info, log_warn};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex};

pub struct AccountRuntime {
    pub account_id: String,
    pub api_url: String,
    pub api_client: Arc<Mutex<DeltaClient>>,
    pub gateway_handle: Option<tokio::task::JoinHandle<()>>,
}

pub struct App {
    pub state: AppState,
    pub api_client: Arc<Mutex<DeltaClient>>,
    pub event_tx: mpsc::Sender<AppEvent>,
    pub gateway_handle: Option<tokio::task::JoinHandle<()>>,
    pub accounts: HashMap<String, AccountRuntime>,
    pub account_clients: Arc<StdMutex<HashMap<String, DeltaClient>>>,
    pub pending_call_cleanups: Vec<tokio::task::JoinHandle<()>>,
    pub last_video_preview_encode: Option<std::time::Instant>,
}

impl App {
    pub fn new(config: Config, event_tx: mpsc::Sender<AppEvent>, picker: Arc<ratatui_image::picker::Picker>) -> Self {
        let api_client = Arc::new(Mutex::new(DeltaClient::new(
            config.api_url.clone(),
            config.token.clone(),
            config.is_bot,
        )));

        let (user_fetch_tx, mut user_fetch_rx) = mpsc::unbounded_channel::<(String, String)>();
        let (image_fetch_tx, mut image_fetch_rx) = mpsc::unbounded_channel::<(String, String)>();
        crate::voice::sounds::set_enabled(config.call_sounds_enabled);
        let mut state = AppState::new(config);
        state.user_fetch_tx = Some(user_fetch_tx);
        state.image_fetch_tx = Some(image_fetch_tx);
        state.image_cache = Some(crate::ui::images::ImageCache::new(picker));

        let mut accounts = HashMap::new();
        let mut initial_clients = HashMap::new();
        for acc in &state.config.accounts {
            let client = DeltaClient::new(acc.api_url.clone(), Some(acc.token.clone()), acc.is_bot);
            initial_clients.insert(acc.id.clone(), client.clone());
            accounts.insert(
                acc.id.clone(),
                AccountRuntime {
                    account_id: acc.id.clone(),
                    api_url: acc.api_url.clone(),
                    api_client: Arc::new(Mutex::new(client)),
                    gateway_handle: None,
                },
            );
        }
        let account_clients = Arc::new(StdMutex::new(initial_clients));

        let api_client_clone = api_client.clone();
        let account_clients_clone = account_clients.clone();
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some((account_id, user_id)) = user_fetch_rx.recv().await {
                let maybe_client = account_clients_clone
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&account_id).cloned());
                let client = match maybe_client {
                    Some(c) => c,
                    None => api_client_clone.lock().await.clone(),
                };
                match client.fetch_user(&user_id).await {
                    Ok(user) => {
                        let _ = event_tx_clone
                            .send(AppEvent::AccountScoped {
                                account_id,
                                event: Box::new(AppEvent::ApiUserFetched(user)),
                            })
                            .await;
                    }
                    Err(e) => {
                        log_warn!("Background user fetch failed for {} ({}): {}", user_id, account_id, e);
                    }
                }
            }
        });

        let api_client_img = api_client.clone();
        let account_clients_img = account_clients.clone();
        let event_tx_img = event_tx.clone();
        tokio::spawn(async move {
            let mut logged_fetches: usize = 0;
            while let Some((key, url)) = image_fetch_rx.recv().await {
                let url_origin = DeltaClient::origin_from_url(&url);
                let matched_client = account_clients_img.lock().ok().and_then(|map| {
                    if let Some(ref uo) = url_origin {
                        if let Some(c) = map.values().find(|c| {
                            DeltaClient::origin_from_url(c.api_url())
                                .map(|o| o.eq_ignore_ascii_case(uo))
                                .unwrap_or(false)
                        }) {
                            return Some(c.clone());
                        }
                    }
                    // Also check key format `<prefix>:<account_id>:<user_id>:<avatar_id>`
                    let mut parts = key.split(':');
                    let _prefix = parts.next();
                    if let Some(acc_id) = parts.next() {
                        if let Some(c) = map.get(acc_id) {
                            return Some(c.clone());
                        }
                    }
                    None
                });
                let client = match matched_client {
                    Some(c) => c,
                    None => api_client_img.lock().await.clone(),
                };
                let should_log = logged_fetches < 5;
                if should_log {
                    logged_fetches += 1;
                }
                match client.fetch_bytes_with_status(&url).await {
                    Ok((status, bytes)) => {
                        if should_log {
                            log_info!(
                                "Image fetch #{} OK: key={} url={} status={} bytes={}",
                                logged_fetches,
                                key,
                                url,
                                status,
                                bytes.len()
                            );
                        }
                        match image::load_from_memory(&bytes) {
                            Ok(dyn_img) => {
                                let _ = event_tx_img.send(AppEvent::ImageLoaded {
                                    key,
                                    dynamic_image: dyn_img,
                                }).await;
                            }
                            Err(e) => {
                                log_error!("Failed to decode image from {} ({} bytes): {}", url, bytes.len(), e);
                            }
                        }
                    }
                    Err(e) => {
                        log_error!("Failed to fetch asset #{} (key={} url={}): {}", logged_fetches, key, url, e);
                    }
                }
            }
        });

        Self {
            state,
            api_client,
            event_tx,
            gateway_handle: None,
            accounts,
            account_clients,
            pending_call_cleanups: Vec::new(),
            last_video_preview_encode: None,
        }
    }

    fn client_for_account_arc(&self, account_id: &str) -> Arc<Mutex<DeltaClient>> {
        self.accounts
            .get(account_id)
            .map(|r| r.api_client.clone())
            .unwrap_or_else(|| self.api_client.clone())
    }

    pub fn startup(&mut self) {
        self.state.config.migrate_legacy_account();
        let saved_accounts = self.state.config.accounts.clone();

        if saved_accounts.is_empty() {
            log_info!("Startup: No session token found. Displaying login modal.");
            self.state.show_login_modal = true;
            self.state.gateway_status = GatewayStatus::Disconnected("Not logged in".to_string());
            self.state.status_message = Some(("Please log in to connect".to_string(), chrono::Utc::now()));
            return;
        }

        // Wipe mock data immediately upon starting authenticated session(s)
        self.state.clear_mock_data();

        log_info!(
            "Startup: Connecting {} saved account(s) concurrently...",
            saved_accounts.len()
        );
        self.state.show_login_modal = false;
        self.state.gateway_status = GatewayStatus::Connecting;
        self.state.status_message = Some(("Connecting...".to_string(), chrono::Utc::now()));

        for acc in saved_accounts {
            self.connect_saved_account(acc);
        }
    }

    fn connect_saved_account(&mut self, acc: SavedAccount) {
        let account_id = acc.id.clone();
        let token = acc.token.clone();
        let api_url = acc.api_url.clone();
        let is_bot = acc.is_bot;

        let client = DeltaClient::new(api_url.clone(), Some(token.clone()), is_bot);
        if let Ok(mut map) = self.account_clients.lock() {
            map.insert(account_id.clone(), client.clone());
        }
        let client_arc = Arc::new(Mutex::new(client.clone()));
        if account_id == self.state.active_account_id {
            self.api_client = client_arc.clone();
        }
        self.accounts.insert(
            account_id.clone(),
            AccountRuntime {
                account_id: account_id.clone(),
                api_url: api_url.clone(),
                api_client: client_arc.clone(),
                gateway_handle: None,
            },
        );
        self.state.account_api_urls.insert(account_id.clone(), api_url.clone());
        self.state.account_autumn_urls.insert(
            account_id.clone(),
            DeltaClient::resolve_autumn_url(&api_url, None),
        );
        self.state
            .account_labels
            .insert(account_id.clone(), crate::config::short_instance_label(&api_url));
        self.state
            .account_gateway_statuses
            .insert(account_id.clone(), GatewayStatus::Connecting);

        let tx = self.event_tx.clone();
        let gw_url_config = if account_id == self.state.active_account_id {
            self.state.config.gateway_url.clone()
        } else {
            None
        };
        let cached_ws = if account_id == self.state.active_account_id {
            self.state.cached_ws_url.clone()
        } else {
            None
        };
        let initial_ws_hint = gw_url_config.clone().or(cached_ws);

        if let Some(ref ws_hint) = initial_ws_hint {
            log_info!(
                "Startup [{}]: Connecting Bonfire concurrently using WS hint: {}",
                account_id,
                ws_hint
            );
            self.start_gateway_for_account(account_id.clone(), token.clone(), Some(ws_hint.clone()));
        }

        // 1. Concurrent GET /users/@me verification
        let client_me = client.clone();
        let tx_me = tx.clone();
        let tok_me = token.clone();
        let acc_me = account_id.clone();
        let api_url_me = api_url.clone();
        let gw_cfg_me = gw_url_config.clone();
        let has_initial_ws = initial_ws_hint.is_some();
        tokio::spawn(async move {
            match client_me.fetch_me(&tok_me).await {
                Ok(user) => {
                    log_info!("Startup [{}]: Authenticated @{} ({})", acc_me, user.username, user.id);
                    let _ = tx_me
                        .send(AppEvent::AccountScoped {
                            account_id: acc_me,
                            event: Box::new(AppEvent::ApiCurrentUser(user)),
                        })
                        .await;
                }
                Err(e) => {
                    if !api_url_me.ends_with("/api") {
                        let probed = DeltaClient::normalize_and_probe_api_url(&api_url_me).await;
                        if probed != api_url_me {
                            let probed_client =
                                DeltaClient::new(probed.clone(), Some(tok_me.clone()), is_bot);
                            if let Ok(user) = probed_client.fetch_me(&tok_me).await {
                                log_info!(
                                    "Startup [{}]: Auto-upgraded api_url {} -> {} and authenticated @{}",
                                    acc_me,
                                    api_url_me,
                                    probed,
                                    user.username
                                );
                                let _ = tx_me
                                    .send(AppEvent::AccountLoginSuccess {
                                        api_url: probed,
                                        token: tok_me,
                                        is_bot,
                                        add_to_existing: true,
                                    })
                                    .await;
                                return;
                            }
                        }
                    }

                    let err_msg = e.to_string();
                    log_warn!("Startup [{}]: GET /users/@me failed: {}", acc_me, err_msg);
                    if err_msg.contains("401") || err_msg.contains("Unauthorized") || err_msg.contains("Session expired") {
                        let _ = tx_me
                            .send(AppEvent::AccountScoped {
                                account_id: acc_me,
                                event: Box::new(AppEvent::ApiSessionExpired),
                            })
                            .await;
                    } else if !has_initial_ws {
                        let _ = tx_me
                            .send(AppEvent::AccountScoped {
                                account_id: acc_me,
                                event: Box::new(AppEvent::StartGateway {
                                    token: tok_me,
                                    gateway_url: gw_cfg_me,
                                }),
                            })
                            .await;
                    }
                }
            }
        });

        // 2. Concurrent GET /users/dms fetch
        let client_dms = client.clone();
        let tx_dms = tx.clone();
        let acc_dms = account_id.clone();
        tokio::spawn(async move {
            if let Ok(dms) = client_dms.fetch_dms().await {
                let _ = tx_dms
                    .send(AppEvent::AccountScoped {
                        account_id: acc_dms,
                        event: Box::new(AppEvent::ApiDmsFetched(dms)),
                    })
                    .await;
            }
        });

        // 3. Concurrent GET / metadata (Autumn CDN URL, LiveKit node, dynamic ws URL)
        let client_meta = client;
        let tx_meta = tx;
        let tok_meta = token;
        let acc_meta = account_id;
        let gw_cfg_meta = gw_url_config;
        let initial_ws_cmp = initial_ws_hint;
        tokio::spawn(async move {
            let (effective_gw_url, autumn_url_opt, livekit_resolved) = match client_meta.fetch_metadata().await {
                Ok(meta) => {
                    let autumn_url = meta
                        .features
                        .as_ref()
                        .and_then(|f| f.autumn.as_ref())
                        .and_then(|a| a.url.clone());
                    let lk_resolved = meta
                        .features
                        .as_ref()
                        .and_then(|f| f.livekit.as_ref().or(f.voice.as_ref()))
                        .map(|lk| lk.resolved_node());
                    let ws_url = if let Some(ws) = meta.ws {
                        log_info!("Startup [{}]: Dynamic gateway URL: {}", acc_meta, ws);
                        Some(ws)
                    } else {
                        gw_cfg_meta
                    };
                    (ws_url, autumn_url, lk_resolved)
                }
                Err(e) => {
                    log_warn!("Startup [{}]: GET / metadata failed: {}", acc_meta, e);
                    (gw_cfg_meta, None, None)
                }
            };

            let resolved_autumn = DeltaClient::resolve_autumn_url(
                client_meta.api_url(),
                autumn_url_opt.as_deref(),
            );
            let _ = tx_meta
                .send(AppEvent::AccountScoped {
                    account_id: acc_meta.clone(),
                    event: Box::new(AppEvent::AutumnUrlResolved(resolved_autumn)),
                })
                .await;

            if let Some((node_opt, lk_url_opt)) = livekit_resolved {
                let node_name = node_opt.unwrap_or_else(|| "worldwide".to_string());
                let _ = tx_meta
                    .send(AppEvent::AccountScoped {
                        account_id: acc_meta.clone(),
                        event: Box::new(AppEvent::LivekitNodeResolved {
                            node_name,
                            url: lk_url_opt,
                        }),
                    })
                    .await;
            }

            if initial_ws_cmp.is_none() || (effective_gw_url.is_some() && effective_gw_url != initial_ws_cmp) {
                let _ = tx_meta
                    .send(AppEvent::AccountScoped {
                        account_id: acc_meta,
                        event: Box::new(AppEvent::StartGateway {
                            token: tok_meta,
                            gateway_url: effective_gw_url,
                        }),
                    })
                    .await;
            }
        });
    }

    pub fn start_gateway(&mut self, token: String, gateway_url: Option<String>) {
        let acc_id = self.state.active_account_id.clone();
        self.start_gateway_for_account(acc_id, token, gateway_url);
    }

    pub fn start_gateway_for_account(&mut self, account_id: String, token: String, gateway_url: Option<String>) {
        if let Some(rt) = self.accounts.get_mut(&account_id) {
            if let Some(h) = rt.gateway_handle.take() {
                h.abort();
            }
        }
        if account_id == self.state.active_account_id {
            if let Some(h) = self.gateway_handle.take() {
                h.abort();
            }
            if let Some(ref url) = gateway_url {
                if !url.trim().is_empty() {
                    self.state.cached_ws_url = Some(url.clone());
                }
            }
        }

        let event_tx = self.event_tx.clone();
        let api_client = self.client_for_account_arc(&account_id);
        let acc_id_spawn = account_id.clone();

        let handle = tokio::spawn(async move {
            let (gw_tx, mut gw_rx) = mpsc::channel(128);

            let effective_gw_url = if let Some(url) = gateway_url {
                url
            } else {
                let client = api_client.lock().await.clone();
                match client.fetch_metadata().await {
                    Ok(meta) => {
                        let raw_autumn = meta
                            .features
                            .as_ref()
                            .and_then(|f| f.autumn.as_ref())
                            .and_then(|a| a.url.as_deref());
                        let autumn_url = DeltaClient::resolve_autumn_url(client.api_url(), raw_autumn);
                        let _ = event_tx
                            .send(AppEvent::AccountScoped {
                                account_id: acc_id_spawn.clone(),
                                event: Box::new(AppEvent::AutumnUrlResolved(autumn_url)),
                            })
                            .await;

                        if let Some((node_opt, lk_url_opt)) = meta
                            .features
                            .as_ref()
                            .and_then(|f| f.livekit.as_ref().or(f.voice.as_ref()))
                            .map(|lk| lk.resolved_node())
                        {
                            let node_name = node_opt.unwrap_or_else(|| "worldwide".to_string());
                            let _ = event_tx
                                .send(AppEvent::AccountScoped {
                                    account_id: acc_id_spawn.clone(),
                                    event: Box::new(AppEvent::LivekitNodeResolved {
                                        node_name,
                                        url: lk_url_opt,
                                    }),
                                })
                                .await;
                        }

                        meta.ws.unwrap_or_else(|| "wss://events.stoat.chat".to_string())
                    }
                    Err(_) => "wss://events.stoat.chat".to_string(),
                }
            };

            let client = BonfireClient::new(effective_gw_url, token, gw_tx);
            let client_handle = client.start();

            while let Some(gw_event) = gw_rx.recv().await {
                if event_tx
                    .send(AppEvent::AccountScoped {
                        account_id: acc_id_spawn.clone(),
                        event: Box::new(AppEvent::Gateway(gw_event)),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }

            client_handle.abort();
        });

        if account_id == self.state.active_account_id {
            // Avoid keeping two Abort-less copies; store primary handle in `accounts` or `gateway_handle`
        }
        if let Some(rt) = self.accounts.get_mut(&account_id) {
            rt.gateway_handle = Some(handle);
        } else {
            self.gateway_handle = Some(handle);
        }
    }

    pub async fn handle_event(&mut self, event: AppEvent) {
        let default_acc = self.state.active_account_id.clone();
        match event {
            AppEvent::AccountScoped { account_id, event } => {
                self.handle_account_event(&account_id, *event).await;
            }
            other => {
                self.handle_account_event(&default_acc, other).await;
            }
        }
    }

    async fn handle_account_event(&mut self, account_id: &str, event: AppEvent) {
        match event {
            AppEvent::AccountScoped { account_id: inner_acc, event } => {
                Box::pin(self.handle_account_event(&inner_acc, *event)).await;
            }
            AppEvent::Key(key) => self.handle_key(key).await,
            AppEvent::Mouse(mouse) => self.handle_mouse(mouse).await,
            AppEvent::Paste(text) => self.handle_paste(text).await,
            AppEvent::Resize(_, _) => {}
            AppEvent::Tick => {}
            AppEvent::Gateway(gw) => self.handle_gateway_for_account(account_id, gw).await,
            AppEvent::ApiLoginSuccess { token, is_bot } | AppEvent::LoginSuccess { token, is_bot } => {
                let api_url = self.state.config.api_url.clone();
                self.handle_account_login_success(api_url, token, is_bot, false).await;
            }
            AppEvent::AccountLoginSuccess {
                api_url,
                token,
                is_bot,
                add_to_existing,
            } => {
                self.handle_account_login_success(api_url, token, is_bot, add_to_existing).await;
            }
            AppEvent::ApiMfaRequired { ticket, allowed_methods: _ } => {
                self.state.login_loading = false;
                self.state.login_error = None;
                self.state.login_step = LoginStep::TotpChallenge { ticket };
                self.state.login_totp_input.clear();
            }
            AppEvent::ApiLoginFailure(err) => {
                self.state.login_loading = false;
                self.state.set_status(format!("Login failed: {}", err));
                self.state.login_error = Some(err);
            }
            AppEvent::ApiMessageSent(res) => match res {
                Ok(msg) => {
                    let cid = msg.channel.clone();
                    self.state.upsert_message(&cid, msg);
                }
                Err(err) => {
                    self.state.set_status(format!("Error sending message: {}", err));
                }
            },
            AppEvent::ApiMessagesFetched { channel_id, mut messages } => {
                // Stoat/Revolt Delta returns Latest first; sort by ULID id ascending for chronological feed
                let ch_acc = self.state.account_for_channel(&channel_id).to_string();
                messages.sort_by(|a, b| a.id.cmp(&b.id));
                for msg in messages {
                    self.state.user_accounts.entry(msg.author.clone()).or_insert_with(|| ch_acc.clone());
                    let _ = self.state.resolve_user_in_account(&ch_acc, &msg.author);
                    self.state.upsert_message(&channel_id, msg);
                }
                if let Some(list) = self.state.messages.get_mut(&channel_id) {
                    list.sort_by(|a, b| {
                        let a_opt = a.id.starts_with("optimistic:");
                        let b_opt = b.id.starts_with("optimistic:");
                        a_opt.cmp(&b_opt).then_with(|| a.id.cmp(&b.id))
                    });
                }
            }
            AppEvent::ApiCurrentUser(user) => {
                self.state.user_accounts.insert(user.id.clone(), account_id.to_string());
                self.state.account_users.insert(account_id.to_string(), user.clone());
                self.state.users.insert(user.id.clone(), user.clone());
                if account_id == self.state.active_account_id || self.state.current_user.is_none() {
                    self.state.current_user = Some(user.clone());
                }
                if let Some(acc) = self.state.config.accounts.iter_mut().find(|a| a.id == account_id) {
                    acc.user_id = Some(user.id);
                    acc.username = Some(user.username);
                }
            }
            AppEvent::ApiUserFetched(user) => {
                log_info!("Hydrated user into cache [{}]: @{} ({})", account_id, user.username, user.id);
                self.state.user_accounts.insert(user.id.clone(), account_id.to_string());
                self.state.users.insert(user.id.clone(), user);
            }
            AppEvent::ApiDmsFetched(dms) => {
                let mut existing_other: Vec<_> = self
                    .state
                    .dms
                    .iter()
                    .filter(|d| self.state.account_for_channel(&d.id) != account_id)
                    .cloned()
                    .collect();
                for dm in &dms {
                    self.state.channel_accounts.insert(dm.id.clone(), account_id.to_string());
                    self.state.channels.insert(dm.id.clone(), dm.clone());
                    if let Some(ref recipients) = dm.recipients {
                        for r in recipients {
                            self.state.user_accounts.entry(r.clone()).or_insert_with(|| account_id.to_string());
                            let _ = self.state.resolve_user_in_account(account_id, r);
                        }
                    }
                }
                existing_other.extend(dms);
                self.state.dms = existing_other;
            }
            AppEvent::ApiSessionExpired => {
                log_warn!("Handling ApiSessionExpired [{}]: Session expired (preserving token on disk)", account_id);
                if let Some(rt) = self.accounts.get_mut(account_id) {
                    if let Some(h) = rt.gateway_handle.take() {
                        h.abort();
                    }
                }
                if account_id == self.state.active_account_id {
                    if let Some(h) = self.gateway_handle.take() {
                        h.abort();
                    }
                    self.state.current_user = None;
                }
                self.state
                    .account_gateway_statuses
                    .insert(account_id.to_string(), GatewayStatus::Disconnected("Session expired".to_string()));
                let any_other_active = self
                    .state
                    .account_gateway_statuses
                    .iter()
                    .any(|(id, st)| id != account_id && !matches!(st, GatewayStatus::Disconnected(_)));
                if !any_other_active {
                    let cache_path = self.state.config.nav_cache_path();
                    let _ = std::fs::remove_file(cache_path);
                    self.state.login_loading = false;
                    self.state.login_error = Some("Session expired, please log in again.".to_string());
                    self.state.show_login_modal = true;
                    self.state.gateway_status = GatewayStatus::Disconnected("Session expired".to_string());
                    self.state.set_status("Session expired, please log in again.");
                } else {
                    let tag = self.state.instance_tag_for_account(account_id).unwrap_or(account_id).to_string();
                    self.state.set_status(format!("Session expired on {} (press F2 to re-auth)", tag));
                }
            }
            AppEvent::VideoFrameReady => {
                let maybe_snap = self.state.active_call.as_ref().and_then(|c| {
                    c.latest_video_frame
                        .try_lock()
                        .ok()
                        .and_then(|g| g.clone())
                        .map(|snap| (snap, c.show_local_preview))
                });
                if let Some(ref mut cache) = self.state.image_cache {
                    if let Some((snap, show_local)) = maybe_snap {
                        if snap.is_remote || show_local {
                            let should_encode = self
                                .last_video_preview_encode
                                .map(|t| t.elapsed() >= std::time::Duration::from_millis(33))
                                .unwrap_or(true);
                            if should_encode {
                                self.last_video_preview_encode = Some(std::time::Instant::now());
                                cache.insert("active_call_video".to_string(), snap.image);
                            }
                        } else {
                            cache.remove("active_call_video");
                        }
                    } else {
                        cache.remove("active_call_video");
                    }
                }
            }
            AppEvent::ScreenShareFailed { channel_id, error } => {
                log_error!("Screen share failed in {}: {}", channel_id, error);
                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        call.screensharing.store(false, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                let clean = crate::ui::components::status_bar::sanitize_status_text(&format!(
                    "❌ Screen share failed: {}",
                    error
                ));
                self.state.set_status(clean);
            }
            AppEvent::ScreenShareCancelled { channel_id } => {
                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        call.screensharing.store(false, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                self.state.set_status("Screen share cancelled");
            }
            AppEvent::ScreenShareSelectTarget { channel_id, targets } => {
                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        call.screensharing.store(false, std::sync::atomic::Ordering::SeqCst);
                        self.state.screen_picker_targets = targets;
                        self.state.screen_picker_selected = 0;
                        self.state.show_screen_picker_modal = true;
                        self.state.set_status("Select a monitor or window to share");
                    }
                }
            }
            AppEvent::CallConnected { channel_id } => {
                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        call.status = crate::voice::CallStatus::Connected;
                        call.connected_at = Some(std::time::Instant::now());
                        self.state.set_status(format!("📞 Connected to call in channel {} [c: Leave]", channel_id));
                    }
                }
            }
            AppEvent::CallFailed { channel_id, error } => {
                log_error!("Call failed for channel {}: {}", channel_id, error);
                if self.state.active_call.as_ref().map(|c| c.channel_id == channel_id).unwrap_or(false) {
                    self.state.active_call = None;
                    self.state.call_fullscreen = false;
                }
                let clean = crate::ui::components::status_bar::sanitize_status_text(&format!("❌ Call failed: {}", error));
                self.state.set_status(clean);
            }
            AppEvent::CallDisconnected { channel_id, reason } => {
                if self.state.active_call.as_ref().map(|c| c.channel_id == channel_id).unwrap_or(false) {
                    crate::voice::sounds::play_leave_sound();
                    self.state.active_call = None;
                    self.state.call_fullscreen = false;
                }
                self.state.set_status(format!("📞 Call disconnected: {}", reason));
            }
            AppEvent::CallParticipantJoined {
                channel_id,
                user_id,
                is_local,
            } => {
                let norm_id = crate::voice::normalize_livekit_identity(&user_id);
                if norm_id.is_empty() || norm_id == "caller" {
                    return;
                }
                let acc_id = self.state.account_for_channel(&channel_id).to_string();
                let _ = self.state.resolve_user_in_account(&acc_id, &norm_id);
                let acc_self_uid = self
                    .state
                    .current_user_for_account(&acc_id)
                    .map(|u| u.id.clone());

                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        if is_local {
                            call.local_user_id = norm_id.clone();
                            let existing_idx = call
                                .participants
                                .iter()
                                .position(|p| p.is_local || p.user_id == norm_id);
                            if let Some(idx) = existing_idx {
                                call.participants[idx].user_id = norm_id.clone();
                                call.participants[idx].is_local = true;
                                call.participants[idx].account_id = acc_id.clone();
                                let mut seen_local = false;
                                call.participants.retain(|p| {
                                    if p.is_local || p.user_id == norm_id {
                                        if seen_local {
                                            return false;
                                        }
                                        seen_local = true;
                                    }
                                    true
                                });
                            } else {
                                call.participants.push(crate::voice::VoiceParticipant {
                                    user_id: norm_id.clone(),
                                    account_id: acc_id.clone(),
                                    is_local: true,
                                    is_speaking: call.local_speaking,
                                    is_muted: call.is_muted(),
                                    is_deafened: call.is_deafened(),
                                    is_screensharing: call.is_screensharing(),
                                });
                            }
                        } else {
                            let is_same_as_local = (!call.local_user_id.is_empty()
                                && call.local_user_id == norm_id)
                                || acc_self_uid.as_deref() == Some(norm_id.as_str());
                            if !is_same_as_local
                                && !call.participants.iter().any(|p| p.user_id == norm_id)
                            {
                                call.participants.push(crate::voice::VoiceParticipant {
                                    user_id: norm_id.clone(),
                                    account_id: acc_id.clone(),
                                    is_local: false,
                                    is_speaking: false,
                                    is_muted: false,
                                    is_deafened: false,
                                    is_screensharing: false,
                                });
                            }
                        }
                    }
                }

                let is_self_for_acc = acc_self_uid.as_deref() == Some(norm_id.as_str());
                let entry = self
                    .state
                    .voice_channel_participants
                    .entry(channel_id)
                    .or_default();
                if !entry.iter().any(|p| p.user_id == norm_id) {
                    entry.push(crate::voice::VoiceParticipant {
                        user_id: norm_id,
                        account_id: acc_id,
                        is_local: is_local || is_self_for_acc,
                        is_speaking: false,
                        is_muted: false,
                        is_deafened: false,
                        is_screensharing: false,
                    });
                }
            }
            AppEvent::CallParticipantLeft { channel_id, user_id } => {
                let norm_id = crate::voice::normalize_livekit_identity(&user_id);
                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        call.participants.retain(|p| p.user_id != norm_id);
                    }
                }
                if let Some(entry) = self.state.voice_channel_participants.get_mut(&channel_id) {
                    entry.retain(|p| p.user_id != norm_id);
                }
            }
            AppEvent::CallParticipantState {
                channel_id,
                user_id,
                fallback_name,
                is_muted,
                is_deafened,
            } => {
                let norm_id = crate::voice::normalize_livekit_identity(&user_id);
                if norm_id.is_empty() || norm_id == "caller" {
                    return;
                }
                if let Some(fb) = fallback_name {
                    let trimmed = fb.trim().to_string();
                    if !trimmed.is_empty() {
                        self.state.call_fallback_names.insert(norm_id.clone(), trimmed);
                    }
                }
                let acc_id = self.state.account_for_channel(&channel_id).to_string();
                let _ = self.state.resolve_user_in_account(&acc_id, &norm_id);
                let acc_self_uid = self
                    .state
                    .current_user_for_account(&acc_id)
                    .map(|u| u.id.clone());
                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        let is_self = (!call.local_user_id.is_empty()
                            && call.local_user_id == norm_id)
                            || acc_self_uid.as_deref() == Some(norm_id.as_str());
                        if !is_self {
                            if let Some(p) = call.participants.iter_mut().find(|p| p.user_id == norm_id) {
                                if let Some(m) = is_muted {
                                    p.is_muted = m;
                                }
                                if let Some(d) = is_deafened {
                                    p.is_deafened = d;
                                    if d {
                                        p.is_muted = true;
                                    }
                                }
                            } else {
                                let d = is_deafened.unwrap_or(false);
                                let m = is_muted.unwrap_or(false) || d;
                                call.participants.push(crate::voice::VoiceParticipant {
                                    user_id: norm_id.clone(),
                                    account_id: acc_id,
                                    is_local: false,
                                    is_speaking: false,
                                    is_muted: m,
                                    is_deafened: d,
                                    is_screensharing: false,
                                });
                            }
                        }
                    }
                }
                if let Some(entry) = self.state.voice_channel_participants.get_mut(&channel_id) {
                    if let Some(p) = entry.iter_mut().find(|p| p.user_id == norm_id) {
                        if let Some(m) = is_muted {
                            p.is_muted = m;
                        }
                        if let Some(d) = is_deafened {
                            p.is_deafened = d;
                            if d {
                                p.is_muted = true;
                            }
                        }
                    }
                }
            }
            AppEvent::CallActiveSpeakers { channel_id, speaker_ids } => {
                let norm_speakers: Vec<String> = speaker_ids
                    .iter()
                    .map(|s| crate::voice::normalize_livekit_identity(s))
                    .collect();
                let acc_id = self.state.account_for_channel(&channel_id).to_string();
                let acc_uid = self
                    .state
                    .current_user_for_account(&acc_id)
                    .map(|u| u.id.clone());
                if let Some(ref mut call) = self.state.active_call {
                    if call.channel_id == channel_id {
                        let local_uid = if !call.local_user_id.is_empty() {
                            Some(call.local_user_id.as_str())
                        } else {
                            acc_uid.as_deref()
                        };
                        call.local_speaking = local_uid
                            .map(|id| norm_speakers.iter().any(|s| s == id))
                            .unwrap_or(false);
                        for p in &mut call.participants {
                            p.is_speaking = norm_speakers.iter().any(|s| s == &p.user_id);
                        }
                    }
                }
                if let Some(entry) = self.state.voice_channel_participants.get_mut(&channel_id) {
                    for p in entry {
                        p.is_speaking = norm_speakers.iter().any(|s| s == &p.user_id);
                    }
                }
            }
            AppEvent::ImageLoaded { key, dynamic_image } => {
                if let Some(ref mut cache) = self.state.image_cache {
                    cache.insert(key, dynamic_image);
                }
            }
            AppEvent::AttachmentUploaded { id, filename } => {
                self.state.set_status(format!("📎 Attached {}", filename));
                self.state.pending_attachments.push(crate::state::PendingAttachment { id, filename });
            }
            AppEvent::AttachmentUploadFailed(err) => {
                self.state.set_status(format!("❌ Upload failed: {}", err));
            }
            AppEvent::AutumnUrlResolved(url) => {
                let trimmed = url.trim().trim_end_matches('/').to_string();
                if !trimmed.is_empty() {
                    self.state.account_autumn_urls.insert(account_id.to_string(), trimmed.clone());
                    if account_id == self.state.active_account_id {
                        self.state.autumn_url = trimmed;
                    }
                }
            }
            AppEvent::LivekitUrlResolved(url) => {
                let trimmed = url.trim().to_string();
                if !trimmed.is_empty() {
                    let node = self
                        .state
                        .account_livekit
                        .get(account_id)
                        .and_then(|(n, _)| n.clone());
                    self.state.account_livekit.insert(account_id.to_string(), (node, Some(trimmed.clone())));
                    if account_id == self.state.active_account_id {
                        self.state.livekit_url = Some(trimmed);
                    }
                }
            }
            AppEvent::LivekitNodeResolved { node_name, url } => {
                let trimmed_node = node_name.trim().to_string();
                let trimmed_url = url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
                let prev_url = self.state.account_livekit.get(account_id).and_then(|(_, u)| u.clone());
                let final_url = trimmed_url.clone().or(prev_url);
                let final_node = if !trimmed_node.is_empty() {
                    Some(trimmed_node.clone())
                } else {
                    self.state.account_livekit.get(account_id).and_then(|(n, _)| n.clone())
                };
                self.state.account_livekit.insert(account_id.to_string(), (final_node.clone(), final_url.clone()));
                if account_id == self.state.active_account_id {
                    if !trimmed_node.is_empty() {
                        self.state.livekit_node_name = Some(trimmed_node);
                    }
                    if let Some(u) = trimmed_url {
                        self.state.livekit_url = Some(u);
                    }
                }
            }
            AppEvent::PickerResolved(picker) => {
                log_info!(
                    "Background graphics probe resolved: protocol={:?}, font={}x{}",
                    picker.protocol_type(),
                    picker.font_size().width,
                    picker.font_size().height
                );
                if let Some(ref mut cache) = self.state.image_cache {
                    cache.set_picker(picker);
                }
            }
            AppEvent::ServerMembersFetched { server_id, members, users } => {
                let srv_acc = self.state.account_for_server(&server_id).to_string();
                for u in users {
                    self.state.user_accounts.insert(u.id.clone(), srv_acc.clone());
                    self.state.users.insert(u.id.clone(), u);
                }
                self.state.members.insert(server_id, members);
            }
            AppEvent::UserProfileFetched { user_id, profile } => {
                self.state.user_profiles.insert(user_id, profile);
            }
            AppEvent::StartGateway { token, gateway_url } => {
                self.start_gateway_for_account(account_id.to_string(), token, gateway_url);
            }
        }
    }

    async fn handle_account_login_success(
        &mut self,
        api_url: String,
        token: String,
        is_bot: bool,
        add_to_existing: bool,
    ) {
        let trimmed_token = token.trim().to_string();
        let norm_url = DeltaClient::normalize_url_str(&api_url);
        let replace_primary = !add_to_existing || self.state.config.accounts.is_empty();
        self.state.login_api_input = norm_url.clone();

        log_info!(
            "Authentication succeeded! api_url={}, replace_primary={}, total_accounts_before={}",
            norm_url,
            replace_primary,
            self.state.config.accounts.len()
        );

        self.state.login_loading = false;
        self.state.login_error = None;
        self.state.login_step = LoginStep::Credentials;
        self.state.login_email_input.clear();
        self.state.login_password_input.clear();
        self.state.login_totp_input.clear();
        self.state.login_token_input.clear();
        self.state.show_login_modal = false;
        self.state.is_demo_mode = false;
        self.state.clear_mock_data();

        if replace_primary {
            if let Some(h) = self.begin_disconnect_active_call() {
                self.pending_call_cleanups.push(h);
            }
            for (_, mut rt) in self.accounts.drain() {
                if let Some(h) = rt.gateway_handle.take() {
                    h.abort();
                }
            }
            if let Some(h) = self.gateway_handle.take() {
                h.abort();
            }
            if let Ok(mut map) = self.account_clients.lock() {
                map.clear();
            }

            self.state.servers.clear();
            self.state.channels.clear();
            self.state.server_channels.clear();
            self.state.dms.clear();
            self.state.members.clear();
            self.state.messages.clear();
            self.state.users.clear();
            self.state.voice_channel_participants.clear();
            self.state.server_accounts.clear();
            self.state.channel_accounts.clear();
            self.state.user_accounts.clear();
            self.state.account_users.clear();
            self.state.account_autumn_urls.clear();
            self.state.account_livekit.clear();
            self.state.account_gateway_statuses.clear();
            self.state.active_call = None;
            self.state.call_fullscreen = false;
            self.state.active_server_id = None;
            self.state.active_channel_id = None;
            self.state.selected_server_index = 0;
            self.state.selected_channel_index = 0;
            self.state.unread_counts.clear();
        }

        let acc_id = self
            .state
            .config
            .upsert_account(norm_url.clone(), trimmed_token.clone(), is_bot, replace_primary);
        if replace_primary {
            self.state.active_account_id = acc_id.clone();
        }
        match self.state.config.save() {
            Ok(_) => log_info!("Saved account '{}' to config.toml successfully", acc_id),
            Err(e) => log_error!("Failed to save config after login: {}", e),
        }

        if let Some(saved_acc) = self.state.config.accounts.iter().find(|a| a.id == acc_id).cloned() {
            self.connect_saved_account(saved_acc);
        }
        self.state.set_status(format!(
            "Connected {}",
            crate::config::short_instance_label(&norm_url).unwrap_or_else(|| "Stoat".to_string())
        ));
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
                LoginTab::EmailPassword => match self.state.login_field_index {
                    0 => self.state.login_email_input.push_str(&text),
                    1 => self.state.login_password_input.push_str(&text),
                    2 => self.state.login_api_input.push_str(&text),
                    _ => {}
                },
                LoginTab::Token => match self.state.login_field_index {
                    0 => self.state.login_token_input.push_str(&text),
                    1 => self.state.login_api_input.push_str(&text),
                    _ => {}
                },
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

        // 1b. If Screen Share Source Picker Modal is open
        if self.state.show_screen_picker_modal {
            let count = self.state.screen_picker_targets.len();
            let idx = if count > 0 {
                self.state.screen_picker_selected.min(count - 1)
            } else {
                0
            };
            match key.code {
                KeyCode::Esc => {
                    self.state.show_screen_picker_modal = false;
                    self.state.screen_picker_targets.clear();
                    self.state.set_status("Screen share cancelled");
                }
                KeyCode::Up | KeyCode::Char('k') if count > 0 => {
                    self.state.screen_picker_selected = if idx == 0 { count - 1 } else { idx - 1 };
                }
                KeyCode::Down | KeyCode::Char('j') if count > 0 => {
                    self.state.screen_picker_selected = (idx + 1) % count;
                }
                KeyCode::Enter if count > 0 => {
                    let chosen = self.state.screen_picker_targets[idx].clone();
                    self.state.show_screen_picker_modal = false;
                    self.state.screen_picker_targets.clear();
                    if let Some(ref mut call) = self.state.active_call {
                        if let Ok(mut slot) = call.pending_screen_target.lock() {
                            *slot = Some(chosen.kind);
                        }
                        call.screensharing.store(true, std::sync::atomic::Ordering::SeqCst);
                        self.state.set_status(format!("Screen sharing started: {}", chosen.label));
                    }
                }
                _ => {}
            }
            return;
        }

        // 1c. If Settings Modal is open
        if self.state.show_settings_modal {
            use crate::ui::modals::settings::SETTINGS_ITEM_COUNT;
            let count = SETTINGS_ITEM_COUNT + self.state.config.accounts.len();
            let idx = self.state.settings_selected_index.min(count.saturating_sub(1));
            match key.code {
                KeyCode::Esc | KeyCode::Char('S') => {
                    self.state.show_settings_modal = false;
                    let _ = self.state.config.save();
                }
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.state.show_settings_modal = false;
                    let _ = self.state.config.save();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.state.settings_selected_index = if idx == 0 { count - 1 } else { idx - 1 };
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.state.settings_selected_index = (idx + 1) % count;
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    self.adjust_setting(idx, -1);
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.adjust_setting(idx, 1);
                }
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Delete | KeyCode::Backspace => {
                    self.activate_setting(idx);
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

                        let target_api = self.state.login_api_input.trim();
                        let api_url = if target_api.is_empty() {
                            self.state.config.api_url.clone()
                        } else {
                            DeltaClient::normalize_url_str(target_api)
                        };
                        let add_to_existing = self.state.login_add_account;
                        let tx = self.event_tx.clone();
                        let ticket_clone = ticket.clone();

                        tokio::spawn(async move {
                            let resolved_api_url =
                                DeltaClient::normalize_and_probe_api_url(&api_url).await;
                            let client = DeltaClient::new(resolved_api_url.clone(), None, false);
                            match client.verify_mfa_totp(&ticket_clone, &code).await {
                                Ok(LoginResponse::Success { token, .. }) => {
                                    let trimmed = token.trim().to_string();
                                    log_info!("MFA TOTP verification succeeded on {}!", resolved_api_url);
                                    let _ = tx.send(AppEvent::AccountLoginSuccess {
                                        api_url: resolved_api_url,
                                        token: trimmed,
                                        is_bot: false,
                                        add_to_existing,
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
                KeyCode::Tab | KeyCode::Down => match self.state.login_tab {
                    LoginTab::EmailPassword => {
                        self.state.login_field_index = (self.state.login_field_index + 1) % 4;
                    }
                    LoginTab::Token => {
                        self.state.login_field_index = (self.state.login_field_index + 1) % 3;
                    }
                    LoginTab::Demo => {
                        self.state.login_tab = LoginTab::EmailPassword;
                        self.state.login_field_index = 0;
                    }
                },
                KeyCode::BackTab | KeyCode::Up => match self.state.login_tab {
                    LoginTab::EmailPassword => {
                        self.state.login_field_index = if self.state.login_field_index == 0 {
                            3
                        } else {
                            self.state.login_field_index - 1
                        };
                    }
                    LoginTab::Token => {
                        self.state.login_field_index = if self.state.login_field_index == 0 {
                            2
                        } else {
                            self.state.login_field_index - 1
                        };
                    }
                    LoginTab::Demo => {}
                },
                KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.state.login_tab = LoginTab::EmailPassword;
                    self.state.login_field_index = 0;
                    self.state.login_step = LoginStep::Credentials;
                    self.state.login_error = None;
                }
                KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.state.login_tab = LoginTab::Token;
                    self.state.login_field_index = 0;
                    self.state.login_step = LoginStep::Credentials;
                    self.state.login_error = None;
                }
                KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.state.login_tab = LoginTab::Demo;
                    self.state.login_field_index = 0;
                    self.state.login_step = LoginStep::Credentials;
                    self.state.login_error = None;
                }
                KeyCode::Left | KeyCode::Right if !key.modifiers.contains(KeyModifiers::ALT) => {
                    let on_mode_field = match self.state.login_tab {
                        LoginTab::EmailPassword => self.state.login_field_index == 3,
                        LoginTab::Token => self.state.login_field_index == 2,
                        LoginTab::Demo => false,
                    };
                    if on_mode_field {
                        self.state.login_add_account = !self.state.login_add_account;
                    }
                }
                KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.state.login_tab = match self.state.login_tab {
                        LoginTab::EmailPassword => LoginTab::Demo,
                        LoginTab::Token => LoginTab::EmailPassword,
                        LoginTab::Demo => LoginTab::Token,
                    };
                    self.state.login_field_index = 0;
                    self.state.login_step = LoginStep::Credentials;
                    self.state.login_error = None;
                }
                KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
                    self.state.login_tab = match self.state.login_tab {
                        LoginTab::EmailPassword => LoginTab::Token,
                        LoginTab::Token => LoginTab::Demo,
                        LoginTab::Demo => LoginTab::EmailPassword,
                    };
                    self.state.login_field_index = 0;
                    self.state.login_step = LoginStep::Credentials;
                    self.state.login_error = None;
                }
                KeyCode::Enter => {
                    let target_api = self.state.login_api_input.trim();
                    let api_url = if target_api.is_empty() {
                        self.state.config.api_url.clone()
                    } else {
                        DeltaClient::normalize_url_str(target_api)
                    };
                    let add_to_existing = self.state.login_add_account;

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

                            let tx = self.event_tx.clone();

                            tokio::spawn(async move {
                                let resolved_api_url =
                                    DeltaClient::normalize_and_probe_api_url(&api_url).await;
                                let client = DeltaClient::new(resolved_api_url.clone(), None, false);
                                match client.login_email(&email, &password).await {
                                    Ok(LoginResponse::Success { token, .. }) => {
                                        let trimmed = token.trim().to_string();
                                        log_info!("Password login succeeded on {}!", resolved_api_url);
                                        let _ = tx.send(AppEvent::AccountLoginSuccess {
                                            api_url: resolved_api_url,
                                            token: trimmed,
                                            is_bot: false,
                                            add_to_existing,
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
                            let tx = self.event_tx.clone();
                            log_info!("Manual token tab submitted for {} (add={})!", api_url, add_to_existing);
                            tokio::spawn(async move {
                                let resolved_api_url =
                                    DeltaClient::normalize_and_probe_api_url(&api_url).await;
                                let client =
                                    DeltaClient::new(resolved_api_url.clone(), Some(token.clone()), false);
                                match client.fetch_me(&token).await {
                                    Ok(_) => {
                                        let _ = tx
                                            .send(AppEvent::AccountLoginSuccess {
                                                api_url: resolved_api_url,
                                                token,
                                                is_bot: false,
                                                add_to_existing,
                                            })
                                            .await;
                                    }
                                    Err(e) => {
                                        let _ = tx.send(AppEvent::ApiLoginFailure(e.to_string())).await;
                                    }
                                }
                            });
                        }
                        LoginTab::Demo => {
                            self.state.load_demo_data();
                            self.state.show_login_modal = false;
                        }
                    }
                }
                KeyCode::Backspace => match self.state.login_tab {
                    LoginTab::EmailPassword => match self.state.login_field_index {
                        0 => { self.state.login_email_input.pop(); }
                        1 => { self.state.login_password_input.pop(); }
                        2 => { self.state.login_api_input.pop(); }
                        _ => {}
                    },
                    LoginTab::Token => match self.state.login_field_index {
                        0 => { self.state.login_token_input.pop(); }
                        1 => { self.state.login_api_input.pop(); }
                        _ => {}
                    },
                    LoginTab::Demo => {}
                },
                KeyCode::Char(' ') if matches!(
                    (self.state.login_tab, self.state.login_field_index),
                    (LoginTab::EmailPassword, 3) | (LoginTab::Token, 2)
                ) => {
                    self.state.login_add_account = !self.state.login_add_account;
                }
                KeyCode::Char(c) => match self.state.login_tab {
                    LoginTab::EmailPassword => match self.state.login_field_index {
                        0 => self.state.login_email_input.push(c),
                        1 => self.state.login_password_input.push(c),
                        2 => self.state.login_api_input.push(c),
                        _ => {}
                    },
                    LoginTab::Token => match self.state.login_field_index {
                        0 => self.state.login_token_input.push(c),
                        1 => self.state.login_api_input.push(c),
                        _ => {}
                    },
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
                        crate::voice::sounds::play_mute_sound(muted);
                        self.state.set_status(if muted { "Microphone muted" } else { "Microphone unmuted" });
                        return;
                    }
                    // When not in a call, Alt+m toggles member rail
                    self.state.show_member_rail = !self.state.show_member_rail;
                    self.state.member_rail_explicitly_toggled = true;
                    if !self.state.show_member_rail && self.state.focus == Focus::Members {
                        self.state.focus = Focus::Chat;
                    }
                    return;
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let deafened = call.toggle_deafen();
                        crate::voice::sounds::play_deafen_sound(deafened);
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
                KeyCode::Char('v') | KeyCode::Char('V') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let shown = call.toggle_local_preview();
                        self.state.set_status(if shown { "Local screen preview ON" } else { "Local screen preview OFF" });
                    }
                    return;
                }
                KeyCode::Char('u') | KeyCode::Char('U') => {
                    self.open_image_upload();
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

        // Open Settings Modal on Shift + S ('S') when not typing in Input
        if self.state.focus != Focus::Input
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
            && (key.code == KeyCode::Char('S')
                || (key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::SHIFT)))
        {
            self.state.show_settings_modal = true;
            self.state.settings_selected_index = 0;
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
                        crate::voice::sounds::play_mute_sound(muted);
                        self.state.set_status(if muted { "Microphone muted" } else { "Microphone unmuted" });
                        return;
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let deafened = call.toggle_deafen();
                        crate::voice::sounds::play_deafen_sound(deafened);
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
                KeyCode::Char('v') => {
                    if let Some(ref mut call) = self.state.active_call {
                        let shown = call.toggle_local_preview();
                        self.state.set_status(if shown { "Local screen preview ON" } else { "Local screen preview OFF" });
                        return;
                    }
                }
                KeyCode::Char('u') => {
                    self.open_image_upload();
                    return;
                }
                _ => {}
            }
        }

        // Member Rail toggle (F4 or Alt+m when not in call)
        if key.code == KeyCode::F(4)
            || (key.modifiers.contains(KeyModifiers::ALT) && (key.code == KeyCode::Char('m') || key.code == KeyCode::Char('M')) && self.state.active_call.is_none())
        {
            self.state.show_member_rail = !self.state.show_member_rail;
            self.state.member_rail_explicitly_toggled = true;
            if !self.state.show_member_rail && self.state.focus == Focus::Members {
                self.state.focus = Focus::Chat;
            }
            return;
        }

        // Left Sidebar Width Adjustment: Ctrl+H / Ctrl+Left (shrink), Ctrl+L / Ctrl+Right (expand)
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Left => {
                    self.state.left_sidebar_width = self.state.left_sidebar_width.saturating_sub(2).max(16);
                    self.state.config.left_sidebar_width = self.state.left_sidebar_width;
                    let _ = self.state.config.save();
                    return;
                }
                KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Right => {
                    self.state.left_sidebar_width = (self.state.left_sidebar_width + 2).min(52);
                    self.state.config.left_sidebar_width = self.state.left_sidebar_width;
                    let _ = self.state.config.save();
                    return;
                }
                _ => {}
            }
        }

        // Manual Login Modal Trigger: F2 or Alt+Shift+L
        if key.code == KeyCode::F(2)
            || (key.modifiers.contains(KeyModifiers::ALT)
                && (key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::Char('L'))
                && (key.code == KeyCode::Char('l') || key.code == KeyCode::Char('L')))
        {
            self.state.show_login_modal = true;
            self.state.login_field_index = 0;
            self.state.login_api_input = self.state.config.api_url.clone();
            self.state.login_add_account = !self.state.config.accounts.is_empty();
            self.state.login_step = LoginStep::Credentials;
            self.state.login_error = None;
            self.state.login_totp_input.clear();
            self.state.login_password_input.clear();
            self.state.login_token_input.clear();
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

        // Direct Pane Jump with bare 1/2/3/4 when not typing in Input (ideal for tiny/compact mode)
        if self.state.focus != Focus::Input && key.modifiers == KeyModifiers::NONE {
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
                _ => {}
            }
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
                    if !self.state.pending_attachments.is_empty() {
                        self.state.pending_attachments.clear();
                        self.state.set_status("Cleared pending attachments");
                    } else {
                        self.state.focus = Focus::Chat;
                    }
                }
                KeyCode::Enter => {
                    let content = self.state.input_clear();
                    if !content.trim().is_empty() || !self.state.pending_attachments.is_empty() {
                        self.send_message(content).await;
                    }
                }
                KeyCode::Backspace => {
                    if self.state.input_buffer.is_empty() && !self.state.pending_attachments.is_empty() {
                        let removed = self.state.pending_attachments.pop();
                        if let Some(att) = removed {
                            self.state.set_status(format!("Removed attachment {}", att.filename));
                        }
                    } else {
                        self.state.input_backspace();
                    }
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
                // If login modal is open, handle clicks on tabs, fields, submit button, or outside
                if self.state.show_login_modal {
                    // Check tabs
                    for (tab, rect) in &self.state.rect_login_tabs {
                        if mx >= rect.left() && mx < rect.right() && my == rect.top() {
                            self.state.login_tab = *tab;
                            self.state.login_step = LoginStep::Credentials;
                            self.state.login_error = None;
                            return;
                        }
                    }

                    // Check fields
                    for (idx, rect) in &self.state.rect_login_fields {
                        if mx >= rect.left() && mx < rect.right() && my == rect.top() {
                            self.state.login_field_index = *idx;
                            let is_mode = matches!(
                                (self.state.login_tab, *idx),
                                (LoginTab::EmailPassword, 3) | (LoginTab::Token, 2)
                            );
                            if is_mode {
                                self.state.login_add_account = !self.state.login_add_account;
                            }
                            return;
                        }
                    }

                    // Check submit button
                    if let Some(btn_rect) = self.state.rect_login_submit_btn {
                        if mx >= btn_rect.left() && mx < btn_rect.right() && my == btn_rect.top() {
                            let target_api = self.state.login_api_input.trim();
                            let api_url = if target_api.is_empty() {
                                self.state.config.api_url.clone()
                            } else {
                                DeltaClient::normalize_url_str(target_api)
                            };
                            let add_to_existing = self.state.login_add_account;
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
                                    let tx = self.event_tx.clone();
                                    tokio::spawn(async move {
                                        let resolved_api_url =
                                            DeltaClient::normalize_and_probe_api_url(&api_url).await;
                                        let client = DeltaClient::new(resolved_api_url.clone(), None, false);
                                        match client.login_email(&email, &password).await {
                                            Ok(LoginResponse::Success { token, .. }) => {
                                                let trimmed = token.trim().to_string();
                                                let _ = tx.send(AppEvent::AccountLoginSuccess { api_url: resolved_api_url, token: trimmed, is_bot: false, add_to_existing }).await;
                                            }
                                            Ok(LoginResponse::MFA { ticket, allowed_methods }) => {
                                                let _ = tx.send(AppEvent::ApiMfaRequired { ticket, allowed_methods }).await;
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
                                    let tx = self.event_tx.clone();
                                    tokio::spawn(async move {
                                        let resolved_api_url =
                                            DeltaClient::normalize_and_probe_api_url(&api_url).await;
                                        let client =
                                            DeltaClient::new(resolved_api_url.clone(), Some(token.clone()), false);
                                        match client.fetch_me(&token).await {
                                            Ok(_) => {
                                                let _ = tx
                                                    .send(AppEvent::AccountLoginSuccess {
                                                        api_url: resolved_api_url,
                                                        token,
                                                        is_bot: false,
                                                        add_to_existing,
                                                    })
                                                    .await;
                                            }
                                            Err(e) => {
                                                let _ = tx.send(AppEvent::ApiLoginFailure(e.to_string())).await;
                                            }
                                        }
                                    });
                                }
                                LoginTab::Demo => {
                                    self.state.load_demo_data();
                                    self.state.show_login_modal = false;
                                }
                            }
                            return;
                        }
                    }

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
                    self.state.member_rail_explicitly_toggled = true;
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

    async fn handle_gateway_for_account(&mut self, account_id: &str, event: GatewayEvent) {
        match event {
            GatewayEvent::Connecting => {
                log_info!("Gateway [{}] connecting", account_id);
                self.state.account_gateway_statuses.insert(account_id.to_string(), GatewayStatus::Connecting);
                if account_id == self.state.active_account_id {
                    self.state.gateway_status = GatewayStatus::Connecting;
                }
            }
            GatewayEvent::Connected => {
                log_info!("Gateway [{}] connected", account_id);
                self.state.account_gateway_statuses.insert(account_id.to_string(), GatewayStatus::Connecting);
                if account_id == self.state.active_account_id {
                    self.state.gateway_status = GatewayStatus::Connecting;
                }
            }
            GatewayEvent::Authenticated => {
                log_info!("Gateway [{}] authenticated: status set to ● Online", account_id);
                self.state.clear_mock_data();
                self.state.account_gateway_statuses.insert(account_id.to_string(), GatewayStatus::Connected { latency_ms: 20 });
                self.state.gateway_status = GatewayStatus::Connected { latency_ms: 20 };
                self.state.set_status("● Online");
            }
            GatewayEvent::Latency(ms) => {
                self.state.account_gateway_statuses.insert(account_id.to_string(), GatewayStatus::Connected { latency_ms: ms });
                if account_id == self.state.active_account_id || matches!(self.state.gateway_status, GatewayStatus::Connected { .. }) {
                    self.state.gateway_status = GatewayStatus::Connected { latency_ms: ms };
                }
            }
            GatewayEvent::Disconnected(reason) => {
                log_warn!("Gateway [{}] disconnected: {}", account_id, reason);
                self.state.account_gateway_statuses.insert(account_id.to_string(), GatewayStatus::Disconnected(reason.clone()));
                let any_connected = self.state.account_gateway_statuses.iter().any(|(id, st)| {
                    id != account_id && matches!(st, GatewayStatus::Connected { .. })
                });
                if !any_connected {
                    self.state.gateway_status = GatewayStatus::Disconnected(reason);
                }
            }
            GatewayEvent::MessageReceived(msg) => match msg {
                ServerMessage::Ready { users, servers, channels, members } => {
                    self.state.clear_mock_data();
                    self.state.account_gateway_statuses.insert(account_id.to_string(), GatewayStatus::Connected { latency_ms: 20 });
                    self.state.gateway_status = match self.state.gateway_status {
                        GatewayStatus::Connected { latency_ms } => GatewayStatus::Connected { latency_ms },
                        _ => GatewayStatus::Connected { latency_ms: 20 },
                    };
                    self.state.set_status("● Online");

                    // Retain items belonging to OTHER connected accounts; replace only this account's entries
                    self.state.purge_account_items(account_id);

                    for u in users {
                        self.state.user_accounts.insert(u.id.clone(), account_id.to_string());
                        self.state.users.insert(u.id.clone(), u);
                    }
                    for s in servers {
                        self.state.server_accounts.insert(s.id.clone(), account_id.to_string());
                        if !self.state.servers.iter().any(|existing| existing.id == s.id) {
                            self.state.servers.push(s);
                        }
                    }
                    for ch in channels {
                        self.state.channel_accounts.insert(ch.id.clone(), account_id.to_string());
                        let is_server_chan = ch.server.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
                        if is_server_chan {
                            let sid = ch.server.clone().unwrap();
                            let entry = self.state.server_channels.entry(sid).or_default();
                            if !entry.contains(&ch.id) {
                                entry.push(ch.id.clone());
                            }
                        } else if ch.channel_type == "DirectMessage" || ch.channel_type == "Group" {
                            if !self.state.dms.iter().any(|d| d.id == ch.id) {
                                self.state.dms.push(ch.clone());
                            }
                        }
                        self.state.channels.insert(ch.id.clone(), ch);
                    }

                    // Also populate server_channels from s.channels if any
                    for s in &self.state.servers {
                        if self.state.account_for_server(&s.id) == account_id {
                            let entry = self.state.server_channels.entry(s.id.clone()).or_default();
                            for cid in &s.channels {
                                self.state.channel_accounts.entry(cid.clone()).or_insert_with(|| account_id.to_string());
                                if !entry.contains(cid) {
                                    entry.push(cid.clone());
                                }
                            }
                        }
                    }

                    for m in members {
                        self.state.members.entry(m.id.server.clone()).or_default().push(m);
                    }

                    // Pre-resolve all DM recipients on this account
                    let dm_recipients: Vec<String> = self
                        .state
                        .dms
                        .iter()
                        .filter(|dm| self.state.account_for_channel(&dm.id) == account_id)
                        .flat_map(|dm| dm.recipients.clone().unwrap_or_default())
                        .collect();
                    for r in dm_recipients {
                        self.state.user_accounts.entry(r.clone()).or_insert_with(|| account_id.to_string());
                        let _ = self.state.resolve_user_in_account(account_id, &r);
                    }

                    log_info!(
                        "Ready [{}] merged: {} total servers, {} total channels, {} total users, {} total dms",
                        account_id,
                        self.state.servers.len(),
                        self.state.channels.len(),
                        self.state.users.len(),
                        self.state.dms.len()
                    );

                    // Restore saved navigation location across restarts (only if not yet navigated or primary account arrived)
                    let should_restore = self.state.active_channel_id.is_none()
                        || account_id == self.state.active_account_id;
                    if should_restore {
                        let last_in_dm = self.state.config.last_in_dm;
                        let last_srv_id = self.state.config.last_server_id.clone();
                        let last_chan_id = self.state.config.last_channel_id.clone();

                        let mut restored = false;

                        if last_in_dm {
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
                                self.state.active_rail = ActiveRail::Server(saved_sid.clone());
                                self.state.active_server_id = Some(saved_sid.clone());
                                if let Some(pos) = self.state.servers.iter().position(|s| &s.id == saved_sid) {
                                    self.state.selected_server_index = pos + 1;
                                }

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
                                    self.switch_to_server(Some(saved_sid.clone())).await;
                                    restored = true;
                                }
                            }
                        }

                        if !restored && self.state.active_channel_id.is_none() {
                            if let Some(first_srv) = self.state.servers.first() {
                                let sid = first_srv.id.clone();
                                self.switch_to_server(Some(sid)).await;
                            } else if !self.state.dms.is_empty() {
                                self.switch_to_server(None).await;
                            }
                        }
                    }

                    // Persist lightweight navigation cache for instant Frame 1 hydration on next launch
                    let nav_snapshot = self.state.build_nav_cache();
                    let nav_path = self.state.config.nav_cache_path();
                    tokio::task::spawn_blocking(move || {
                        let _ = nav_snapshot.save_to(&nav_path);
                    });
                }
                ServerMessage::Message(m) => {
                    self.state.user_accounts.entry(m.author.clone()).or_insert_with(|| account_id.to_string());
                    self.state.channel_accounts.entry(m.channel.clone()).or_insert_with(|| account_id.to_string());
                    let _ = self.state.resolve_user_in_account(account_id, &m.author);
                    let channel_id = m.channel.clone();
                    let is_active_channel = self.state.active_channel_id.as_deref() == Some(&channel_id);

                    // Track call_started system event as active call participant (only if a real user ID is present)
                    if let Some(ref sys) = m.system {
                        let ev_type = sys.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if ev_type == "call_started" {
                            let resolved_id = sys
                                .get("user")
                                .or_else(|| sys.get("by"))
                                .or_else(|| sys.get("id"))
                                .and_then(|u| u.as_str())
                                .map(|s| s.trim())
                                .filter(|s| !s.is_empty() && *s != "caller")
                                .or_else(|| {
                                    let a = m.author.trim();
                                    (!a.is_empty() && a != "00000000000000000000000000").then_some(a)
                                });
                            if let Some(uid) = resolved_id {
                                let norm_id = crate::voice::normalize_livekit_identity(uid);
                                let _ = self.state.resolve_user_in_account(account_id, &norm_id);
                                let entry = self.state.voice_channel_participants.entry(channel_id.clone()).or_default();
                                if !entry.iter().any(|p| p.user_id == norm_id) {
                                    entry.push(crate::voice::VoiceParticipant {
                                        user_id: norm_id,
                                        account_id: account_id.to_string(),
                                        is_local: false,
                                        is_speaking: false,
                                        is_muted: false,
                                        is_deafened: false,
                                        is_screensharing: false,
                                    });
                                }
                            }
                        }
                    }

                    self.state.upsert_message(&channel_id, m);

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
                    self.state.channel_accounts.insert(ch.id.clone(), account_id.to_string());
                    if let Some(ref sid) = ch.server {
                        self.state.server_channels.entry(sid.clone()).or_default().push(ch.id.clone());
                    } else if ch.channel_type == "DirectMessage" || ch.channel_type == "Group" {
                        if !self.state.dms.iter().any(|d| d.id == ch.id) {
                            self.state.dms.push(ch.clone());
                        }
                    }
                    self.state.channels.insert(ch.id.clone(), ch);
                }
                ServerMessage::ChannelDelete { id } => {
                    self.state.channels.remove(&id);
                }
                ServerMessage::VoiceChannelJoin { id, user } => {
                    let _ = self.state.resolve_user_in_account(account_id, &user);
                    let entry = self.state.voice_channel_participants.entry(id).or_default();
                    if !entry.iter().any(|p| p.user_id == user) {
                        entry.push(crate::voice::VoiceParticipant {
                            user_id: user,
                            account_id: account_id.to_string(),
                            is_local: false,
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

    pub fn open_image_upload(&mut self) {
        let acc_id = self.state.account_for_active_channel().to_string();
        let autumn_url = self.state.autumn_url_for_account(&acc_id).to_string();
        let api = self.client_for_account_arc(&acc_id);
        let tx = self.event_tx.clone();

        self.state.set_status("Opening file picker...");

        tokio::task::spawn_blocking(move || {
            let file_opt = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif"])
                .pick_file();

            if let Some(path) = file_opt {
                let filename = path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("image.png")
                    .to_string();

                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let tx_clone = tx.clone();
                        let filename_clone = filename.clone();
                        tokio::spawn(async move {
                            let client = api.lock().await;
                            match client.upload_autumn_attachment(&autumn_url, filename_clone.clone(), bytes).await {
                                Ok(id) => {
                                    let _ = tx_clone.send(AppEvent::AttachmentUploaded {
                                        id,
                                        filename: filename_clone,
                                    }).await;
                                }
                                Err(e) => {
                                    let _ = tx_clone.send(AppEvent::AttachmentUploadFailed(e.to_string())).await;
                                }
                            }
                        });
                    }
                    Err(e) => {
                        let _ = tx.blocking_send(AppEvent::AttachmentUploadFailed(format!("Failed to read file: {}", e)));
                    }
                }
            }
        });
    }

    async fn send_message(&mut self, content: String) {
        let channel_id = match &self.state.active_channel_id {
            Some(cid) => cid.clone(),
            None => return,
        };

        let pending = std::mem::take(&mut self.state.pending_attachments);
        let attachment_ids: Option<Vec<String>> = if !pending.is_empty() {
            Some(pending.iter().map(|a| a.id.clone()).collect())
        } else {
            None
        };

        let content_opt: Option<String> = if content.trim().is_empty() {
            None
        } else {
            Some(content.clone())
        };

        // Don't send completely empty messages if no content and no attachments
        if content_opt.is_none() && attachment_ids.is_none() {
            return;
        }

        // If authenticated with real token (either on owning account or legacy config.token), send via REST API
        let acc_id = self.state.account_for_channel(&channel_id).to_string();
        let has_auth = !self.state.is_demo_mode
            && (self.accounts.contains_key(&acc_id) || self.state.config.token.is_some());
        if has_auth {
            let nonce = self.state.next_nonce();
            let current_user_id = self
                .state
                .current_user_for_account(&acc_id)
                .map(|u| u.id.clone())
                .unwrap_or_default();
            let optimistic_msg = Message {
                id: format!("optimistic:{}", nonce),
                channel: channel_id.clone(),
                author: current_user_id,
                content: content_opt.clone(),
                nonce: Some(nonce.clone()),
                ..Default::default()
            };
            self.state.upsert_message(&channel_id, optimistic_msg);
            self.state.chat_scroll_offset = 0;

            let api = self.client_for_account_arc(&acc_id);
            let tx = self.event_tx.clone();
            let cid = channel_id.clone();

            tokio::spawn(async move {
                let client = api.lock().await.clone();
                match client
                    .send_message_with_nonce(&cid, content_opt, attachment_ids, Some(nonce))
                    .await
                {
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
            let demo_attachments = if !pending.is_empty() {
                Some(pending.into_iter().map(|p| crate::api::models::FileObject {
                    id: p.id,
                    filename: Some(p.filename),
                    ..Default::default()
                }).collect())
            } else {
                None
            };
            let new_msg = Message {
                id: format!("demo_msg_{}", chrono::Utc::now().timestamp_millis()),
                channel: channel_id.clone(),
                author: current_user_id,
                content: content_opt,
                attachments: demo_attachments,
                reactions: None,
                ..Default::default()
            };

            self.state.upsert_message(&channel_id, new_msg);
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
        let acc_id = self.state.account_for_channel(&channel_id).to_string();
        let api = self.client_for_account_arc(&acc_id);
        let tx = self.event_tx.clone();
        let cid = channel_id.clone();

        tokio::spawn(async move {
            let client = api.lock().await.clone();
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

    pub async fn fetch_server_members(&self, server_id: String) {
        log_debug!("Fetching members for server {}", server_id);
        let acc_id = self.state.account_for_server(&server_id).to_string();
        let api = self.client_for_account_arc(&acc_id);
        let tx = self.event_tx.clone();
        let sid = server_id.clone();

        tokio::spawn(async move {
            let client = api.lock().await.clone();
            match client.fetch_server_members(&sid).await {
                Ok(resp) => {
                    let _ = tx.send(AppEvent::ServerMembersFetched {
                        server_id: sid,
                        members: resp.members,
                        users: resp.users,
                    }).await;
                }
                Err(e) => {
                    log_warn!("Failed to fetch server members for {}: {}", sid, e);
                }
            }
        });
    }

    pub async fn fetch_user_profile(&self, user_id: String) {
        log_debug!("Fetching profile for user {}", user_id);
        let acc_id = self.state.account_for_user(&user_id).to_string();
        let api = self.client_for_account_arc(&acc_id);
        let tx = self.event_tx.clone();
        let uid = user_id.clone();

        tokio::spawn(async move {
            let client = api.lock().await.clone();
            match client.fetch_user_profile(&uid).await {
                Ok(profile) => {
                    let _ = tx.send(AppEvent::UserProfileFetched {
                        user_id: uid,
                        profile,
                    }).await;
                }
                Err(e) => {
                    log_debug!("Failed to fetch user profile for {}: {}", uid, e);
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

        // Persist session navigation state only if authenticated and not in demo/mock mode
        let is_mock_server = self.state.active_server_id.as_deref().map(|s| s.starts_with("srv_stoat") || s.starts_with("srv_rust")).unwrap_or(false);
        let is_mock_channel = channel_id.starts_with("ch_");
        if self.state.config.token.is_some() && !self.state.is_demo_mode && !is_mock_server && !is_mock_channel {
            self.state.config.last_in_dm = self.state.active_server_id.is_none();
            self.state.config.last_server_id = self.state.active_server_id.clone();
            self.state.config.last_channel_id = Some(channel_id.clone());
            let _ = self.state.config.save();
        }

        // If switching to a DM channel, fetch recipient's user profile if needed
        if self.state.active_server_id.is_none() {
            if let Some(ch) = self.state.channels.get(&channel_id) {
                if ch.channel_type == "DirectMessage" {
                    if let Some(ref recipients) = ch.recipients {
                        let cur_id = self.state.current_user.as_ref().map(|u| u.id.as_str());
                        if let Some(other_id) = recipients.iter().find(|&r| Some(r.as_str()) != cur_id).or_else(|| recipients.first()) {
                            if !self.state.user_profiles.contains_key(other_id) {
                                self.fetch_user_profile(other_id.clone()).await;
                            }
                        }
                    }
                }
            }
        }

        self.fetch_channel_messages(channel_id).await;
    }

    pub async fn switch_to_server(&mut self, server_id: Option<String>) {
        if let Some(ref sid) = server_id {
            self.state.active_rail = ActiveRail::Server(sid.clone());
            self.state.active_server_id = Some(sid.clone());

            // Fetch server members if not already fetched
            if !self.state.members.contains_key(sid) {
                self.fetch_server_members(sid.clone()).await;
            }

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
                let is_mock_server = sid.starts_with("srv_stoat") || sid.starts_with("srv_rust");
                if self.state.config.token.is_some() && !self.state.is_demo_mode && !is_mock_server {
                    self.state.config.last_in_dm = false;
                    self.state.config.last_server_id = Some(sid.clone());
                    self.state.config.last_channel_id = None;
                    let _ = self.state.config.save();
                }
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
                if self.state.config.token.is_some() && !self.state.is_demo_mode {
                    self.state.config.last_in_dm = true;
                    self.state.config.last_server_id = None;
                    self.state.config.last_channel_id = None;
                    let _ = self.state.config.save();
                }
            }
        }
    }

    pub async fn jump_to_channel(&mut self, channel_id: String, server_id: Option<String>) {
        if let Some(ref sid) = server_id {
            self.state.active_rail = ActiveRail::Server(sid.clone());
            self.state.active_server_id = Some(sid.clone());

            // Fetch server members if not already fetched
            if !self.state.members.contains_key(sid) {
                self.fetch_server_members(sid.clone()).await;
            }
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

    fn begin_disconnect_active_call(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        self.pending_call_cleanups.retain(|h| !h.is_finished());
        let mut call = self.state.active_call.take()?;
        let channel_id = call.channel_id.clone();
        let engine_handle = call.take_engine_handle();
        drop(call);

        self.state.call_fullscreen = false;
        if let Some(ref mut cache) = self.state.image_cache {
            cache.remove("active_call_video");
        }

        let acc_id = self.state.account_for_channel(&channel_id).to_string();
        let api = self.client_for_account_arc(&acc_id);

        Some(tokio::spawn(async move {
            let client = api.lock().await.clone();
            let leave_fut = async {
                if client.has_token() {
                    client.leave_call(&channel_id).await;
                }
            };
            let close_fut = async {
                if let Some(h) = engine_handle {
                    let abort_handle = h.abort_handle();
                    if tokio::time::timeout(std::time::Duration::from_millis(800), h)
                        .await
                        .is_err()
                    {
                        abort_handle.abort();
                    }
                }
            };
            tokio::join!(leave_fut, close_fut);
        }))
    }

    pub async fn disconnect_active_call(&mut self, wait: bool) {
        if let Some(handle) = self.begin_disconnect_active_call() {
            if wait {
                let _ = tokio::time::timeout(std::time::Duration::from_millis(2000), handle).await;
            } else {
                self.pending_call_cleanups.push(handle);
            }
        }
        if wait {
            let remaining: Vec<_> = self.pending_call_cleanups.drain(..).collect();
            for h in remaining {
                let _ = tokio::time::timeout(std::time::Duration::from_millis(1500), h).await;
            }
        }
    }

    pub async fn shutdown(&mut self) {
        self.disconnect_active_call(true).await;
    }

    pub async fn toggle_call(&mut self) {
        if self.state.active_call.is_some() {
            crate::voice::sounds::play_leave_sound();
            self.disconnect_active_call(false).await;
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

        let is_dm_or_group = self
            .state
            .channels
            .get(&channel_id)
            .map(|c| c.channel_type == "DirectMessage" || c.channel_type == "Group")
            .unwrap_or(false);

        let acc_id = self.state.account_for_channel(&channel_id).to_string();
        let acc_local_uid = self
            .state
            .current_user_for_account(&acc_id)
            .map(|u| u.id.clone())
            .unwrap_or_default();

        // Immediate UI feedback on 'c' / 'Alt+c': initialize state.active_call with status Connecting
        let mut call = crate::voice::ActiveCallState::new(channel_id.clone());
        call.account_id = acc_id.clone();
        call.local_user_id = acc_local_uid.clone();
        call.status = crate::voice::CallStatus::Connecting;
        call.connected_at = None;
        if !acc_local_uid.is_empty() {
            call.participants.push(crate::voice::VoiceParticipant {
                user_id: acc_local_uid,
                account_id: acc_id.clone(),
                is_local: true,
                is_speaking: false,
                is_muted: false,
                is_deafened: false,
                is_screensharing: false,
            });
        }

        let muted = call.muted.clone();
        let deafened = call.deafened.clone();
        let screensharing = call.screensharing.clone();
        let pending_screen_target = call.pending_screen_target.clone();
        let latest_video_frame = call.latest_video_frame.clone();
        let shutdown = call.shutdown.clone();

        let api = self.client_for_account_arc(&acc_id);
        let cid_clone = channel_id.clone();
        let app_tx = self.event_tx.clone();
        let (acc_lk_node, acc_lk_url) = self
            .state
            .account_livekit
            .get(&acc_id)
            .cloned()
            .unwrap_or((None, None));
        let livekit_node_name = acc_lk_node.or_else(|| self.state.livekit_node_name.clone());
        let fallback_lk_url = acc_lk_url.or_else(|| self.state.livekit_url.clone());

        let handle = tokio::spawn(async move {
            let client = api.lock().await.clone();

            let join_res = match client.join_call(&cid_clone, livekit_node_name.as_deref()).await {
                Ok(res) => res,
                Err(e) => {
                    let err_msg = e.to_string();
                    log_error!("join_call API failed for {}: {}", cid_clone, err_msg);
                    let _ = app_tx
                        .send(AppEvent::CallFailed {
                            channel_id: cid_clone,
                            error: err_msg,
                        })
                        .await;
                    return;
                }
            };

            if is_dm_or_group {
                client.ring_channel(&cid_clone).await;
            }

            let livekit_url = join_res
                .url
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_string)
                .or(fallback_lk_url);

            let Some(lk_url) = livekit_url else {
                let err_msg = "No LiveKit server URL available from metadata or join_call".to_string();
                log_error!("{}", err_msg);
                let _ = app_tx
                    .send(AppEvent::CallFailed {
                        channel_id: cid_clone,
                        error: err_msg,
                    })
                    .await;
                return;
            };

            let engine_handle = crate::voice::VoiceEngine::connect_and_run(
                cid_clone,
                lk_url,
                join_res.token,
                muted,
                deafened,
                screensharing,
                pending_screen_target,
                latest_video_frame,
                shutdown,
                app_tx,
            );
            let _ = engine_handle.await;
        });

        call.set_engine_handle(handle);
        self.state.active_call = Some(call);
        self.state.set_status(format!("📞 Connecting to call in channel {}... [c: Cancel]", channel_id));
    }

    fn adjust_setting(&mut self, idx: usize, delta: i32) {
        match idx {
            0 => {
                self.state.config.call_sounds_enabled = !self.state.config.call_sounds_enabled;
                crate::voice::sounds::set_enabled(self.state.config.call_sounds_enabled);
                let _ = self.state.config.save();
            }
            1 => {
                let cur = self.state.left_sidebar_width as i32;
                let next = (cur + delta * 2).clamp(16, 52) as u16;
                self.state.left_sidebar_width = next;
                self.state.config.left_sidebar_width = next;
                let _ = self.state.config.save();
            }
            2 => {
                self.state.config.compact_mode = !self.state.config.compact_mode;
                let _ = self.state.config.save();
            }
            3 => {
                let len = crate::config::ACCENT_PRESETS.len() as i32;
                if len > 0 {
                    let cur = self.state.current_accent_preset_index() as i32;
                    let next = (cur + delta).rem_euclid(len) as usize;
                    self.state.set_accent_preset(next);
                    let _ = self.state.config.save();
                }
            }
            _ => {}
        }
    }

    fn activate_setting(&mut self, idx: usize) {
        match idx {
            0 | 2 | 3 => self.adjust_setting(idx, 1),
            1 => {
                let next = if self.state.left_sidebar_width >= 52 {
                    16
                } else {
                    (self.state.left_sidebar_width + 2).min(52)
                };
                self.state.left_sidebar_width = next;
                self.state.config.left_sidebar_width = next;
                let _ = self.state.config.save();
            }
            4 => {
                self.state.show_settings_modal = false;
                self.state.show_login_modal = true;
                self.state.login_field_index = 0;
                self.state.login_api_input = self.state.config.api_url.clone();
                self.state.login_add_account = !self.state.config.accounts.is_empty();
                self.state.login_step = LoginStep::Credentials;
                self.state.login_error = None;
                self.state.login_totp_input.clear();
                self.state.login_password_input.clear();
                self.state.login_token_input.clear();
                let _ = self.state.config.save();
            }
            5 => {
                self.logout_current_session();
            }
            n if n >= 6 => {
                let acc_idx = n - 6;
                if let Some(acc) = self.state.config.accounts.get(acc_idx).cloned() {
                    self.remove_account_by_id(&acc.id);
                }
            }
            _ => {}
        }
    }

    fn remove_account_by_id(&mut self, account_id: &str) {
        if let Some(mut rt) = self.accounts.remove(account_id) {
            if let Some(h) = rt.gateway_handle.take() {
                h.abort();
            }
        }
        if let Ok(mut map) = self.account_clients.lock() {
            map.remove(account_id);
        }
        self.state.config.remove_account(account_id);
        let _ = self.state.config.save();

        self.state.purge_account_items(account_id);
        self.state.server_accounts.retain(|_, v| v != account_id);
        self.state.channel_accounts.retain(|_, v| v != account_id);
        self.state.user_accounts.retain(|_, v| v != account_id);
        self.state.account_users.remove(account_id);
        self.state.account_autumn_urls.remove(account_id);
        self.state.account_livekit.remove(account_id);
        self.state.account_gateway_statuses.remove(account_id);
        self.state.account_api_urls.remove(account_id);
        self.state.account_labels.remove(account_id);

        if let Some(first_acc) = self.state.config.accounts.first() {
            self.state.active_account_id = first_acc.id.clone();
            self.state.current_user = self.state.account_users.get(&first_acc.id).cloned();
        } else {
            self.logout_current_session();
        }
        let max_idx = 6 + self.state.config.accounts.len();
        if self.state.settings_selected_index >= max_idx {
            self.state.settings_selected_index = max_idx.saturating_sub(1);
        }
    }

    fn logout_current_session(&mut self) {
        if let Some(h) = self.begin_disconnect_active_call() {
            self.pending_call_cleanups.push(h);
        }
        for (_, mut rt) in self.accounts.drain() {
            if let Some(h) = rt.gateway_handle.take() {
                h.abort();
            }
        }
        if let Some(h) = self.gateway_handle.take() {
            h.abort();
        }
        if let Ok(mut map) = self.account_clients.lock() {
            map.clear();
        }
        self.state.active_call = None;
        self.state.call_fullscreen = false;
        self.state.config.clear_token();
        let _ = self.state.config.save();
        let _ = std::fs::remove_file(self.state.config.nav_cache_path());

        self.state.current_user = None;
        self.state.account_users.clear();
        self.state.account_autumn_urls.clear();
        self.state.account_livekit.clear();
        self.state.account_gateway_statuses.clear();
        self.state.server_accounts.clear();
        self.state.channel_accounts.clear();
        self.state.user_accounts.clear();
        self.state.servers.clear();
        self.state.channels.clear();
        self.state.server_channels.clear();
        self.state.dms.clear();
        self.state.messages.clear();
        self.state.members.clear();
        self.state.users.clear();
        self.state.active_server_id = None;
        self.state.active_channel_id = None;

        self.state.gateway_status = GatewayStatus::Disconnected("Logged out".to_string());
        self.state.show_settings_modal = false;
        self.state.show_login_modal = true;
        self.state.login_field_index = 0;
        self.state.login_add_account = false;
        self.state.login_step = LoginStep::Credentials;
        self.state.login_error = None;
        self.state.set_status("Logged out cleanly");
    }
}
