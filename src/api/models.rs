use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMetadata {
    pub revolt: Option<String>,
    pub stoat: Option<String>,
    pub ws: Option<String>,
    pub app: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginPayload {
    pub email: String,
    pub password: String,
    pub friendly_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaTotpPayload {
    pub mfa_ticket: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result")]
pub enum LoginResponse {
    Success {
        token: String,
        #[serde(default)]
        user_id: Option<String>,
        #[serde(default, rename = "_id")]
        _id: Option<String>,
    },
    MFA {
        ticket: String,
        #[serde(default)]
        allowed_methods: Vec<String>,
    },
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectLoginSuccess {
    pub token: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default, rename = "_id")]
    pub _id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevoltApiError {
    #[serde(rename = "type")]
    pub error_type: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct User {
    #[serde(rename = "_id")]
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<FileObject>,
    #[serde(default)]
    pub status: Option<UserStatus>,
    #[serde(default)]
    pub online: Option<bool>,
    #[serde(default)]
    pub bot: Option<BotInfo>,
}

impl User {
    pub fn name(&self) -> &str {
        if let Some(ref dn) = self.display_name {
            if !dn.trim().is_empty() {
                return dn.as_str();
            }
        }
        &self.username
    }

    pub fn presence_str(&self) -> &str {
        if let Some(ref st) = self.status {
            if let Some(ref pres) = st.presence {
                return pres.as_str();
            }
        }
        if self.online.unwrap_or(false) {
            "Online"
        } else {
            "Offline"
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserStatus {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub presence: Option<Presence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Presence {
    Online,
    Idle,
    Focus,
    Busy,
    Invisible,
    Offline,
}

impl Presence {
    pub fn as_str(&self) -> &str {
        match self {
            Presence::Online => "Online",
            Presence::Idle => "Idle",
            Presence::Focus => "Focus",
            Presence::Busy => "DND",
            Presence::Invisible => "Invisible",
            Presence::Offline => "Offline",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BotInfo {
    pub owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileObject {
    #[serde(rename = "_id")]
    pub id: String,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Server {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<FileObject>,
    pub banner: Option<FileObject>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub categories: Option<Vec<Category>>,
    #[serde(default)]
    pub roles: Option<HashMap<String, Role>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Category {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub channels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Role {
    pub name: String,
    #[serde(default)]
    pub colour: Option<String>,
    #[serde(default)]
    pub rank: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Channel {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(rename = "channel_type")]
    pub channel_type: String,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<FileObject>,
    #[serde(default)]
    pub recipients: Option<Vec<String>>,
    #[serde(default)]
    pub last_message_id: Option<String>,
}

impl Channel {
    pub fn display_name(&self, users: &HashMap<String, User>, current_user_id: Option<&str>) -> String {
        if let Some(ref name) = self.name {
            if !name.trim().is_empty() {
                return name.clone();
            }
        }
        if self.channel_type == "DirectMessage" {
            if let Some(ref recipients) = self.recipients {
                for r in recipients {
                    if Some(r.as_str()) != current_user_id {
                        if let Some(user) = users.get(r) {
                            return user.name().to_string();
                        }
                        return format!("User {}", &r[..r.len().min(6)]);
                    }
                }
            }
            return "Direct Message".to_string();
        }
        "channel".to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemberId {
    pub server: String,
    pub user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Member {
    #[serde(rename = "_id")]
    pub id: MemberId,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub avatar: Option<FileObject>,
    #[serde(default)]
    pub roles: Option<Vec<String>>,
    #[serde(default)]
    pub joined_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Message {
    #[serde(rename = "_id")]
    pub id: String,
    pub channel: String,
    pub author: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub attachments: Option<Vec<FileObject>>,
    #[serde(default)]
    pub edited: Option<String>,
    #[serde(default)]
    pub replies: Option<Vec<String>>,
    #[serde(default)]
    pub reactions: Option<HashMap<String, Vec<String>>>,
    #[serde(default)]
    pub masquerade: Option<Masquerade>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Masquerade {
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub colour: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessagePayload {
    pub content: String,
    #[serde(default)]
    pub replies: Option<Vec<String>>,
}
