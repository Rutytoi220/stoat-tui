pub mod messages;

use futures_util::{SinkExt, StreamExt};
use messages::{ClientMessage, ServerMessage};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

#[derive(Debug, Clone)]
pub enum GatewayEvent {
    Connected,
    Authenticated,
    Latency(u64),
    MessageReceived(ServerMessage),
    Disconnected(String),
}

pub struct BonfireClient {
    gateway_url: String,
    token: String,
    event_tx: mpsc::Sender<GatewayEvent>,
}

impl BonfireClient {
    pub fn new(gateway_url: String, token: String, event_tx: mpsc::Sender<GatewayEvent>) -> Self {
        Self {
            gateway_url,
            token,
            event_tx,
        }
    }

    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(30);

            loop {
                let ws_url = if self.gateway_url.contains('?') {
                    format!("{}&format=json&version=1", self.gateway_url)
                } else {
                    format!("{}?format=json&version=1", self.gateway_url)
                };

                let _ = self.event_tx.send(GatewayEvent::Disconnected("Connecting...".to_string())).await;

                match connect_async(&ws_url).await {
                    Ok((ws_stream, _)) => {
                        let _ = self.event_tx.send(GatewayEvent::Connected).await;
                        backoff = Duration::from_secs(1); // Reset backoff on successful connect

                        let (mut write, mut read) = ws_stream.split();

                        // 1. Send Authenticate handshake
                        let auth_msg = ClientMessage::Authenticate {
                            token: self.token.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&auth_msg) {
                            if write.send(WsMessage::Text(json.into())).await.is_err() {
                                let _ = self.event_tx.send(GatewayEvent::Disconnected("Failed to send auth".to_string())).await;
                                sleep(backoff).await;
                                continue;
                            }
                        }

                        // Set up heartbeat timer channel
                        let (ping_tx, mut ping_rx) = mpsc::channel::<()>(4);
                        let heartbeat_task = tokio::spawn(async move {
                            loop {
                                sleep(Duration::from_secs(20)).await;
                                if ping_tx.send(()).await.is_err() {
                                    break;
                                }
                            }
                        });

                        loop {
                            tokio::select! {
                                Some(_) = ping_rx.recv() => {
                                    let now_ms = SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis() as u64;
                                    let ping = ClientMessage::Ping { data: now_ms };
                                    if let Ok(json) = serde_json::to_string(&ping) {
                                        if write.send(WsMessage::Text(json.into())).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                msg = read.next() => {
                                    match msg {
                                        Some(Ok(WsMessage::Text(text))) => {
                                            if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                                                match &server_msg {
                                                    ServerMessage::Authenticated => {
                                                        let _ = self.event_tx.send(GatewayEvent::Authenticated).await;
                                                    }
                                                    ServerMessage::Pong { data } => {
                                                        let now_ms = SystemTime::now()
                                                            .duration_since(UNIX_EPOCH)
                                                            .unwrap_or_default()
                                                            .as_millis() as u64;
                                                        let latency = now_ms.saturating_sub(*data);
                                                        let _ = self.event_tx.send(GatewayEvent::Latency(latency)).await;
                                                    }
                                                    _ => {}
                                                }
                                                let _ = self.event_tx.send(GatewayEvent::MessageReceived(server_msg)).await;
                                            }
                                        }
                                        Some(Ok(WsMessage::Close(frame))) => {
                                            let reason = frame.map(|f| f.reason.to_string()).unwrap_or_else(|| "Closed by server".to_string());
                                            let _ = self.event_tx.send(GatewayEvent::Disconnected(reason)).await;
                                            break;
                                        }
                                        Some(Ok(WsMessage::Ping(data))) => {
                                            let _ = write.send(WsMessage::Pong(data)).await;
                                        }
                                        Some(Err(e)) => {
                                            let _ = self.event_tx.send(GatewayEvent::Disconnected(format!("Error: {}", e))).await;
                                            break;
                                        }
                                        None => {
                                            let _ = self.event_tx.send(GatewayEvent::Disconnected("Stream ended".to_string())).await;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        heartbeat_task.abort();
                    }
                    Err(e) => {
                        let _ = self.event_tx.send(GatewayEvent::Disconnected(format!("Failed to connect: {}", e))).await;
                    }
                }

                sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        })
    }
}
