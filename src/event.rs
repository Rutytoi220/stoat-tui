use crate::api::models::{Channel, Message, User};
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
    ApiLoginSuccess { token: String, is_bot: bool },
    LoginSuccess { token: String, is_bot: bool },
    ApiMfaRequired { ticket: String, allowed_methods: Vec<String> },
    ApiLoginFailure(String),
    ApiMessageSent(Result<Message, String>),
    ApiMessagesFetched { channel_id: String, messages: Vec<Message> },
    ApiCurrentUser(User),
    ApiUserFetched(User),
    ApiDmsFetched(Vec<Channel>),
    ApiSessionExpired,
    StartGateway { token: String, gateway_url: Option<String> },
}

pub struct EventHandler {
    pub rx: mpsc::Receiver<AppEvent>,
    pub tx: mpsc::Sender<AppEvent>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let event_tx = tx.clone();

        // Spawn crossterm event reader
        tokio::spawn(async move {
            let mut reader = event::EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_rate);

            loop {
                tokio::select! {
                    _ = tick_interval.tick() => {
                        if event_tx.send(AppEvent::Tick).await.is_err() {
                            break;
                        }
                    }
                    Some(crossterm_event) = reader.next() => {
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
                }
            }
        });

        Self { rx, tx }
    }

    pub fn sender(&self) -> mpsc::Sender<AppEvent> {
        self.tx.clone()
    }
}
