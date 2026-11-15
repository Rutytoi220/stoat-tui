use crate::api::models::{Category, Channel, Member, Message, Server, User};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    Authenticate { token: String },
    Ping { data: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    Authenticated,
    Ready {
        #[serde(default)]
        users: Vec<User>,
        #[serde(default)]
        servers: Vec<Server>,
        #[serde(default)]
        channels: Vec<Channel>,
        #[serde(default)]
        members: Vec<Member>,
    },
    Pong {
        data: u64,
    },
    Message(Message),
    MessageUpdate {
        id: String,
        data: serde_json::Value,
    },
    MessageDelete {
        id: String,
        channel: String,
    },
    ChannelCreate(Channel),
    ChannelUpdate {
        id: String,
        data: serde_json::Value,
    },
    ChannelDelete {
        id: String,
    },
    ServerMemberJoin {
        id: String, // Server ID
        user: String, // User ID
    },
    ServerMemberLeave {
        id: String, // Server ID
        user: String, // User ID
    },
    UserUpdate {
        id: String,
        data: serde_json::Value,
    },
    VoiceChannelJoin {
        id: String,
        user: String,
    },
    VoiceChannelLeave {
        id: String,
        user: String,
    },
    UserVoiceStateUpdate {
        id: String,
        user: String,
        #[serde(default)]
        speaking: Option<bool>,
        #[serde(default)]
        muted: Option<bool>,
        #[serde(default)]
        deafened: Option<bool>,
        #[serde(default)]
        screensharing: Option<bool>,
    },
    Error {
        error: String,
    },
    #[serde(other)]
    Unknown,
}

impl ServerMessage {
    pub fn parse_resilient(text: &str) -> Option<Self> {
        if let Ok(msg) = serde_json::from_str::<ServerMessage>(text) {
            return Some(msg);
        }

        // Resilient fallback: parse into Value and extract elements
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
            let msg_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match msg_type {
                "Ready" => Some(parse_resilient_ready(&val)),
                "Authenticated" => Some(ServerMessage::Authenticated),
                "Pong" => {
                    let data = val.get("data").and_then(|d| d.as_u64()).unwrap_or(0);
                    Some(ServerMessage::Pong { data })
                }
                "Message" => {
                    if let Ok(m) = serde_json::from_value::<Message>(val.clone()) {
                        Some(ServerMessage::Message(m))
                    } else if let Some(data) = val.get("data") {
                        serde_json::from_value::<Message>(data.clone()).ok().map(ServerMessage::Message)
                    } else {
                        None
                    }
                }
                "ChannelCreate" => {
                    if let Ok(c) = serde_json::from_value::<Channel>(val.clone()) {
                        Some(ServerMessage::ChannelCreate(c))
                    } else if let Some(data) = val.get("data") {
                        serde_json::from_value::<Channel>(data.clone()).ok().map(ServerMessage::ChannelCreate)
                    } else {
                        None
                    }
                }
                "VoiceChannelJoin" => {
                    let id = val.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let user = val.get("user").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    Some(ServerMessage::VoiceChannelJoin { id, user })
                }
                "VoiceChannelLeave" => {
                    let id = val.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let user = val.get("user").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    Some(ServerMessage::VoiceChannelLeave { id, user })
                }
                "UserVoiceStateUpdate" => {
                    let id = val.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let user = val.get("user").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let speaking = val.get("speaking").and_then(|v| v.as_bool());
                    let muted = val.get("muted").and_then(|v| v.as_bool());
                    let deafened = val.get("deafened").and_then(|v| v.as_bool());
                    let screensharing = val.get("screensharing").and_then(|v| v.as_bool());
                    Some(ServerMessage::UserVoiceStateUpdate {
                        id,
                        user,
                        speaking,
                        muted,
                        deafened,
                        screensharing,
                    })
                }
                _ => Some(ServerMessage::Unknown),
            }
        } else {
            None
        }
    }
}

pub fn parse_resilient_ready(val: &serde_json::Value) -> ServerMessage {
    let mut users = Vec::new();
    if let Some(user_arr) = val.get("users").and_then(|v| v.as_array()) {
        for u_val in user_arr {
            if let Ok(u) = serde_json::from_value::<User>(u_val.clone()) {
                users.push(u);
            } else if let Some(id) = u_val.get("_id").and_then(|i| i.as_str()) {
                let username = u_val.get("username").and_then(|u| u.as_str()).unwrap_or("user").to_string();
                let display_name = u_val.get("display_name").and_then(|d| d.as_str()).map(|s| s.to_string());
                users.push(User {
                    id: id.to_string(),
                    username,
                    display_name,
                    ..Default::default()
                });
            }
        }
    }

    let mut servers = Vec::new();
    if let Some(srv_arr) = val.get("servers").and_then(|v| v.as_array()) {
        for s_val in srv_arr {
            if let Ok(s) = serde_json::from_value::<Server>(s_val.clone()) {
                servers.push(s);
            } else if let Some(id) = s_val.get("_id").and_then(|i| i.as_str()) {
                let name = s_val.get("name").and_then(|n| n.as_str()).unwrap_or("Server").to_string();
                let description = s_val.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                let channels = s_val.get("channels")
                    .and_then(|c| c.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                let categories = s_val.get("categories")
                    .and_then(|c| serde_json::from_value::<Vec<Category>>(c.clone()).ok());
                servers.push(Server {
                    id: id.to_string(),
                    name,
                    description,
                    channels,
                    categories,
                    ..Default::default()
                });
            }
        }
    }

    let mut channels = Vec::new();
    if let Some(chan_arr) = val.get("channels").and_then(|v| v.as_array()) {
        for c_val in chan_arr {
            if let Ok(c) = serde_json::from_value::<Channel>(c_val.clone()) {
                channels.push(c);
            } else if let Some(id) = c_val.get("_id").and_then(|i| i.as_str()) {
                let channel_type = c_val.get("channel_type")
                    .or_else(|| c_val.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("TextChannel")
                    .to_string();
                let name = c_val.get("name").and_then(|n| n.as_str()).map(|s| s.to_string());
                let server = c_val.get("server").and_then(|s| s.as_str()).map(|s| s.to_string());
                let description = c_val.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                let recipients = c_val.get("recipients")
                    .and_then(|r| r.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());
                channels.push(Channel {
                    id: id.to_string(),
                    channel_type,
                    server,
                    name,
                    description,
                    recipients,
                    ..Default::default()
                });
            }
        }
    }

    let mut members = Vec::new();
    if let Some(mem_arr) = val.get("members").and_then(|v| v.as_array()) {
        for m_val in mem_arr {
            if let Ok(m) = serde_json::from_value::<Member>(m_val.clone()) {
                members.push(m);
            } else if let Some(id_obj) = m_val.get("_id") {
                let server = id_obj.get("server").and_then(|s| s.as_str()).unwrap_or("").to_string();
                let user = id_obj.get("user").and_then(|u| u.as_str()).unwrap_or("").to_string();
                let nickname = m_val.get("nickname").and_then(|n| n.as_str()).map(|s| s.to_string());
                if !server.is_empty() && !user.is_empty() {
                    use crate::api::models::MemberId;
                    members.push(Member {
                        id: MemberId { server, user },
                        nickname,
                        ..Default::default()
                    });
                }
            }
        }
    }

    ServerMessage::Ready {
        users,
        servers,
        channels,
        members,
    }
}
