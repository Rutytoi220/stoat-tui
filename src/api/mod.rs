pub mod models;

use anyhow::{anyhow, Context, Result};
use models::*;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct DeltaClient {
    client: Client,
    api_url: String,
    token: Option<String>,
    is_bot: bool,
}

impl DeltaClient {
    pub fn new(api_url: String, token: Option<String>, is_bot: bool) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| Client::new());

        let api_url = api_url.trim_end_matches('/').to_string();

        Self {
            client,
            api_url,
            token,
            is_bot,
        }
    }

    pub fn set_token(&mut self, token: String, is_bot: bool) {
        self.token = Some(token);
        self.is_bot = is_bot;
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(ref token) = self.token {
            let header_key = if self.is_bot {
                "x-bot-token"
            } else {
                "x-session-token"
            };
            if let Ok(val) = HeaderValue::from_str(token) {
                headers.insert(header_key, val);
            }
        }
        headers
    }

    pub async fn fetch_metadata(&self) -> Result<ServerMetadata> {
        let url = format!("{}/", self.api_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to reach API at {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!("API returned status {}", status));
        }

        let metadata: ServerMetadata = resp
            .json()
            .await
            .context("Failed to parse server metadata JSON")?;
        Ok(metadata)
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<LoginResponse> {
        let url = format!("{}/auth/session/login", self.api_url);
        let payload = LoginPayload {
            email: email.to_string(),
            password: password.to_string(),
            friendly_name: "stoat-tui".to_string(),
        };

        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to connect to API server at {}", url))?;

        let status = resp.status();
        let raw_body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            // 1. Attempt to deserialize standard Stoat/Revolt JSON error
            if let Ok(api_err) = serde_json::from_str::<RevoltApiError>(&raw_body) {
                let description = match api_err.error_type.as_str() {
                    "InvalidCredentials" => "Invalid credentials",
                    "UserNotFound" => "Account not found",
                    "FailedValidation" => "Failed validation (invalid email or password format)",
                    "InvalidSession" => "Invalid session",
                    "CompromisedPassword" => "Compromised password, please reset password",
                    "Blocked" => "Access blocked",
                    "TooManyRequests" => "Too many login attempts, please wait and try again",
                    other => other,
                };
                let extra = api_err.error.as_deref().unwrap_or("");
                if extra.is_empty() {
                    return Err(anyhow!("{} ({})", description, status.as_u16()));
                } else {
                    return Err(anyhow!("{} ({}): {}", description, status.as_u16(), extra));
                }
            }

            // 2. Handle known HTTP status codes gracefully
            let clean_body = raw_body.trim();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(anyhow!("HTTP 404 - Check API URL (endpoint not found at {})", self.api_url));
            }
            if clean_body.is_empty() {
                return Err(anyhow!("Server returned HTTP {}", status));
            } else if clean_body.starts_with('<') {
                return Err(anyhow!("Server returned HTTP {} (HTML response - verify API URL)", status));
            } else {
                let preview = if clean_body.len() > 100 {
                    format!("{}...", &clean_body[..100])
                } else {
                    clean_body.to_string()
                };
                return Err(anyhow!("Server returned HTTP {}: {}", status, preview));
            }
        }

        // 3. Success deserialization: try tagged LoginResponse first, then direct
        if let Ok(login_resp) = serde_json::from_str::<LoginResponse>(&raw_body) {
            return Ok(login_resp);
        }

        if let Ok(direct) = serde_json::from_str::<DirectLoginSuccess>(&raw_body) {
            return Ok(LoginResponse::Success {
                token: direct.token,
                user_id: direct.user_id,
                _id: direct._id,
            });
        }

        Err(anyhow!(
            "Unexpected server response format (HTTP {}): {}",
            status,
            if raw_body.len() > 80 { format!("{}...", &raw_body[..80]) } else { raw_body }
        ))
    }

    pub async fn get_current_user(&self) -> Result<User> {
        let url = format!("{}/users/@me", self.api_url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to query current user at {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to get user ({}): {}", status, err_text));
        }

        let user: User = resp
            .json()
            .await
            .context("Failed to parse user JSON")?;
        Ok(user)
    }

    #[allow(dead_code)]
    pub async fn fetch_messages(&self, channel_id: &str, limit: u32) -> Result<Vec<Message>> {
        let url = format!(
            "{}/channels/{}/messages?limit={}&sort=Latest",
            self.api_url, channel_id, limit
        );
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to fetch messages for channel {}", channel_id))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetch messages error ({}): {}", status, err_text));
        }

        let text = resp.text().await?;
        if let Ok(list) = serde_json::from_str::<Vec<Message>>(&text) {
            return Ok(list);
        }

        #[derive(Deserialize)]
        struct MessagesWrapper {
            messages: Vec<Message>,
        }
        if let Ok(wrapper) = serde_json::from_str::<MessagesWrapper>(&text) {
            return Ok(wrapper.messages);
        }

        Err(anyhow!("Unrecognized messages format from API: {}", text))
    }

    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<Message> {
        let url = format!("{}/channels/{}/messages", self.api_url, channel_id);
        let payload = SendMessagePayload {
            content: content.to_string(),
            replies: None,
        };

        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to post message to channel {}", channel_id))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Send message error ({}): {}", status, err_text));
        }

        let msg: Message = resp
            .json()
            .await
            .context("Failed to parse returned message JSON")?;
        Ok(msg)
    }

    pub async fn fetch_dms(&self) -> Result<Vec<Channel>> {
        let url = format!("{}/users/dms", self.api_url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to fetch DMs at {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetch DMs error ({}): {}", status, err_text));
        }

        let channels: Vec<Channel> = resp
            .json()
            .await
            .context("Failed to parse DMs JSON")?;
        Ok(channels)
    }
}
