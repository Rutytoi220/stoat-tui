use crate::api::models::{Channel, Member, Message, User, UserProfile};
use crate::gateway::GatewayEvent;
use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, MouseEvent};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

#[allow(dead_code)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize(u16, u16),
    Tick,
    Gateway(GatewayEvent),
    AccountScoped { account_id: String, event: Box<AppEvent> },
    ApiLoginSuccess { token: String, is_bot: bool },
    LoginSuccess { token: String, is_bot: bool },
    AccountLoginSuccess { api_url: String, token: String, is_bot: bool, add_to_existing: bool },
    ApiMfaRequired { ticket: String, allowed_methods: Vec<String> },
    ApiLoginFailure(String),
    ApiMessageSent(Result<Message, String>),
    ApiMessagesFetched { channel_id: String, messages: Vec<Message> },
    ApiCurrentUser(User),
    ApiUserFetched(User),
    ApiDmsFetched(Vec<Channel>),
    ApiSessionExpired,
    StartGateway { token: String, gateway_url: Option<String> },
    VideoFrameReady,
    ScreenShareFailed { channel_id: String, error: String },
    ScreenShareCancelled { channel_id: String },
    ScreenShareSelectTarget {
        channel_id: String,
        targets: Vec<crate::voice::screen::ScreenShareTarget>,
    },
    CallConnected { channel_id: String },
    CallFailed { channel_id: String, error: String },
    CallDisconnected { channel_id: String, reason: String },
    CallParticipantJoined { channel_id: String, user_id: String, is_local: bool },
    CallParticipantLeft { channel_id: String, user_id: String },
    CallParticipantState {
        channel_id: String,
        user_id: String,
        fallback_name: Option<String>,
        is_muted: Option<bool>,
        is_deafened: Option<bool>,
    },
    CallActiveSpeakers { channel_id: String, speaker_ids: Vec<String> },
    ImageLoaded { key: String, dynamic_image: image::DynamicImage },
    AttachmentUploaded { id: String, filename: String },
    AttachmentUploadFailed(String),
    AutumnUrlResolved(String),
    LivekitUrlResolved(String),
    LivekitNodeResolved { node_name: String, url: Option<String> },
    PickerResolved(std::sync::Arc<ratatui_image::picker::Picker>),
    ServerMembersFetched { server_id: String, members: Vec<Member>, users: Vec<User> },
    UserProfileFetched { user_id: String, profile: UserProfile },
}

pub struct EventHandler {
    pub rx: mpsc::Receiver<AppEvent>,
    pub tx: mpsc::Sender<AppEvent>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let handler = Self::new_deferred(tick_rate);
        Self::spawn_stdin_reader(handler.tx.clone());
        handler
    }

    /// Starts only the UI tick timer without opening stdin `EventStream` yet,
    /// allowing an optional background graphics probe to read stdin safely after Frame 1.
    pub fn new_deferred(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let tick_tx = tx.clone();

        tokio::spawn(async move {
            let mut tick_interval = tokio::time::interval(tick_rate);
            loop {
                tick_interval.tick().await;
                if tick_tx.send(AppEvent::Tick).await.is_err() {
                    break;
                }
            }
        });

        Self { rx, tx }
    }

    pub fn spawn_stdin_reader(event_tx: mpsc::Sender<AppEvent>) {
        tokio::spawn(async move {
            let mut reader = event::EventStream::new();
            while let Some(crossterm_event) = reader.next().await {
                match crossterm_event {
                    Ok(CrosstermEvent::Key(key)) => {
                        if event_tx.send(AppEvent::Key(key)).await.is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Mouse(mouse)) => {
                        if event_tx.send(AppEvent::Mouse(mouse)).await.is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Paste(text)) => {
                        if event_tx.send(AppEvent::Paste(text)).await.is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Resize(w, h)) => {
                        if event_tx.send(AppEvent::Resize(w, h)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    pub fn sender(&self) -> mpsc::Sender<AppEvent> {
        self.tx.clone()
    }
}
