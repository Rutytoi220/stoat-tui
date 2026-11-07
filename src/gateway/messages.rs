use crate::api::models::{Channel, Member, Message, Server, User};
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
    Error {
        error: String,
    },
    #[serde(other)]
    Unknown,
}
