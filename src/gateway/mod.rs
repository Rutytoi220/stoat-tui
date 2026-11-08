pub mod messages;

use crate::{log_debug, log_error, log_info, log_warn};
use futures_util::{SinkExt, StreamExt};
use messages::{ClientMessage, ServerMessage};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

#[allow(dead_code, clippy::large_enum_variant)]
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

                log_info!("Connecting to Bonfire gateway at {}", ws_url);
                let _ = self.event_tx.send(GatewayEvent::Disconnected("Connecting...".to_string())).await;

                match connect_async(&ws_url).await {
                    Ok((ws_stream, _)) => {
                        log_info!("Connected to Bonfire gateway WebSocket at {}", ws_url);
                        let _ = self.event_tx.send(GatewayEvent::Connected).await;
                        backoff = Duration::from_secs(1); // Reset backoff on successful connect

                        let (mut write, mut read) = ws_stream.split();

                        // 1. Send Authenticate handshake
                        let auth_msg = ClientMessage::Authenticate {
                            token: self.token.clone(),
                        };
                        if let Ok(json) = serde_json::to_string(&auth_msg) {
                            log_debug!("Sending Authenticate handshake to Bonfire gateway");
                            if write.send(WsMessage::Text(json)).await.is_err() {
                                log_error!("Failed to send Authenticate message to Bonfire gateway");
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
                                        if write.send(WsMessage::Text(json)).await.is_err() {
                                            log_warn!("Failed to send gateway heartbeat ping");
                                            break;
                                        }
                                    }
                                }
                                msg = read.next() => {
                                    match msg {
                                        Some(Ok(WsMessage::Text(text))) => {
                                            if let Some(server_msg) = ServerMessage::parse_resilient(&text) {
                                                match &server_msg {
                                                    ServerMessage::Ready { servers, channels, users, .. } => {
                                                        log_info!(
                                                            "Gateway Ready event: {} servers, {} channels, {} users",
                                                            servers.len(),
                                                            channels.len(),
                                                            users.len()
                                                        );
                                                    }
                                                    ServerMessage::Authenticated => {
                                                        log_info!("Gateway Authenticated event received");
                                                        let _ = self.event_tx.send(GatewayEvent::Authenticated).await;
                                                    }
                                                    ServerMessage::Pong { data } => {
                                                        let now_ms = SystemTime::now()
                                                            .duration_since(UNIX_EPOCH)
                                                            .unwrap_or_default()
                                                            .as_millis() as u64;
                                                        let latency = now_ms.saturating_sub(*data);
                                                        log_debug!("Gateway Pong: {}ms latency", latency);
                                                        let _ = self.event_tx.send(GatewayEvent::Latency(latency)).await;
                                                    }
                                                    _ => {}
                                                }
                                                let _ = self.event_tx.send(GatewayEvent::MessageReceived(server_msg)).await;
                                            }
                                        }
                                        Some(Ok(WsMessage::Close(frame))) => {
                                            let reason = frame.map(|f| format!("code: {}, reason: {}", f.code, f.reason)).unwrap_or_else(|| "Closed by server".to_string());
                                            log_warn!("Bonfire gateway WebSocket closed: {}", reason);
                                            let _ = self.event_tx.send(GatewayEvent::Disconnected(reason)).await;
                                            break;
                                        }
                                        Some(Ok(WsMessage::Ping(data))) => {
                                            let _ = write.send(WsMessage::Pong(data)).await;
                                        }
                                        Some(Err(e)) => {
                                            log_error!("Bonfire gateway WebSocket error: {}", e);
                                            let _ = self.event_tx.send(GatewayEvent::Disconnected(format!("Error: {}", e))).await;
                                            break;
                                        }
                                        None => {
                                            log_warn!("Bonfire gateway WebSocket stream ended");
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
                        log_error!("Failed to connect to Bonfire gateway at {}: {}", ws_url, e);
                        let _ = self.event_tx.send(GatewayEvent::Disconnected(format!("Failed to connect: {}", e))).await;
                    }
                }

                sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        })
    }
}
