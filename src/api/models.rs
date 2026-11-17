use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerMetadata {
    pub revolt: Option<String>,
    pub stoat: Option<String>,
    pub ws: Option<String>,
    pub app: Option<String>,
    #[serde(default)]
    pub features: Option<ServerFeatures>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerFeatures {
    #[serde(default)]
    pub autumn: Option<AutumnFeature>,
    #[serde(default)]
    pub livekit: Option<LivekitFeature>,
    #[serde(default)]
    pub voice: Option<LivekitFeature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutumnFeature {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LivekitNode {
    #[serde(default, alias = "id")]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LivekitFeature {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub nodes: Option<Vec<LivekitNode>>,
}

impl LivekitFeature {
    /// Resolves `(node_name, url)` from `nodes` (falling back to `"worldwide"` and top-level `url`).
    pub fn resolved_node(&self) -> (Option<String>, Option<String>) {
        if let Some(ref nodes) = self.nodes {
            if !nodes.is_empty() {
                let preferred = nodes
                    .iter()
                    .find(|n| n.name.trim().eq_ignore_ascii_case("worldwide") && !n.url.trim().is_empty())
                    .or_else(|| nodes.iter().find(|n| !n.url.trim().is_empty()))
                    .or_else(|| nodes.first());

                if let Some(node) = preferred {
                    let name = if node.name.trim().is_empty() {
                        Some("worldwide".to_string())
                    } else {
                        Some(node.name.trim().to_string())
                    };
                    let url = if node.url.trim().is_empty() {
                        self.url
                            .as_deref()
                            .map(str::trim)
                            .filter(|u| !u.is_empty())
                            .map(str::to_string)
                    } else {
                        Some(node.url.trim().to_string())
                    };
                    return (name, url);
                }
            }
        }

        let top_url = self
            .url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(str::to_string);
        let default_name = if self.enabled || top_url.is_some() || self.nodes.is_some() {
            Some("worldwide".to_string())
        } else {
            None
        };
        (default_name, top_url)
    }

    pub fn resolved_url(&self) -> Option<String> {
        self.resolved_node().1
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginPayload {
    pub email: String,
    pub password: String,
    pub friendly_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaSessionLoginPayload {
    pub mfa_ticket: String,
    pub mfa_response: MfaResponse,
    pub friendly_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaResponse {
    pub totp_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaTotpPayload {
    pub mfa_ticket: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaTicketPayload {
    pub ticket: String,
    pub result: MfaTicketResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MfaTicketResult {
    Totp { code: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MfaTicketFallbackPayload {
    pub ticket: String,
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
    #[serde(default)]
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
    #[serde(other)]
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
    #[serde(default)]
    pub owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileMetadata {
    #[serde(rename = "type", default)]
    pub file_type: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileObject {
    #[serde(rename = "_id", default)]
    pub id: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub metadata: Option<FileMetadata>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub deleted: Option<bool>,
    #[serde(default)]
    pub reported: Option<bool>,
}

pub type Attachment = FileObject;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Server {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<FileObject>,
    #[serde(default)]
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
    #[serde(alias = "type", rename = "channel_type", default)]
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
                let other_id = recipients
                    .iter()
                    .find(|&r| Some(r.as_str()) != current_user_id)
                    .or_else(|| recipients.first());

                if let Some(uid) = other_id {
                    if let Some(user) = users.get(uid) {
                        return user.name().to_string();
                    }
                    return format!("User {}", &uid[..uid.len().min(6)]);
                }
            }
            return "Direct Message".to_string();
        }
        "channel".to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerMembersResponse {
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub users: Vec<User>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserProfile {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub background: Option<FileObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemberId {
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Member {
    #[serde(rename = "_id", default)]
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
    pub embeds: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub system: Option<serde_json::Value>,
    #[serde(default)]
    pub edited: Option<String>,
    #[serde(default)]
    pub replies: Option<Vec<String>>,
    #[serde(default)]
    pub reactions: Option<HashMap<String, Vec<String>>>,
    #[serde(default)]
    pub masquerade: Option<Masquerade>,
    #[serde(default)]
    pub nonce: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Masquerade {
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub colour: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessagePayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutumnUploadResponse {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CallJoinResponse {
    pub token: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// Crockford Base32 character mapping for ULID timestamp decoding
const CROCKFORD_CHARS: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Extracts the 48-bit Unix timestamp in milliseconds from a 26-character Crockford Base32 ULID.
/// The first 10 characters encode the timestamp in milliseconds.
pub fn ulid_timestamp(id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let id_bytes = id.as_bytes();
    if id_bytes.len() < 10 {
        return None;
    }

    let mut timestamp_ms: u64 = 0;
    for &b in &id_bytes[..10] {
        let upper = b.to_ascii_uppercase();
        let val = CROCKFORD_CHARS.iter().position(|&c| c == upper)? as u64;
        timestamp_ms = (timestamp_ms << 5) | val;
    }

    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ulid_timestamp() {
        // ULID starting with 01ARZ3NDEK...
        // 01ARZ3NDEK -> timestamp ms: 1469922850259 (2016-07-30T23:54:10.259Z)
        let dt = ulid_timestamp("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert!(dt.is_some());
        let dt = dt.unwrap();
        assert_eq!(dt.timestamp_millis(), 1469922850259);
    }
}
