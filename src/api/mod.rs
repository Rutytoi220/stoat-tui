pub mod models;

use crate::{log_debug, log_error, log_info, log_warn};
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
        log_debug!("GET {} (fetching metadata)", url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to reach API at {}", url))?;

        let status = resp.status();
        log_debug!("GET {} -> Status: {}", url, status);
        if !status.is_success() {
            log_warn!("API {} returned status {}", url, status);
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
        log_info!("POST {} (logging in user '{}')", url, email);
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
        log_info!("POST {} -> Status: {}", url, status);

        if !status.is_success() {
            log_warn!("Login failed with status {}: {}", status, raw_body);
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

    pub async fn verify_mfa_totp(&self, ticket: &str, code: &str) -> Result<LoginResponse> {
        let url = format!("{}/auth/session/login", self.api_url);
        log_info!("POST {} (verifying MFA TOTP code)", url);
        let payload = MfaSessionLoginPayload {
            mfa_ticket: ticket.to_string(),
            mfa_response: MfaResponse {
                totp_code: code.to_string(),
            },
            friendly_name: "stoat-tui".to_string(),
        };

        // Ensure the request does NOT send an x-session-token header
        let mut anon_headers = HeaderMap::new();
        anon_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let resp = self
            .client
            .post(&url)
            .headers(anon_headers)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to connect to API server at {}", url))?;

        let status = resp.status();
        let raw_body = resp.text().await.unwrap_or_default();
        log_info!("POST {} (MFA verify) -> Status: {}", url, status);

        if !status.is_success() {
            log_warn!("MFA TOTP verification failed with status {}: {}", status, raw_body);
            // Attempt to deserialize standard Stoat/Revolt JSON error
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&raw_body) {
                let error_type = json_val
                    .get("type")
                    .and_then(|v| v.as_str())
                    .or_else(|| json_val.get("error").and_then(|v| v.as_str()))
                    .or_else(|| json_val.get("message").and_then(|v| v.as_str()));

                if let Some(err_type) = error_type {
                    match err_type {
                        "InvalidCode" => return Err(anyhow!("Invalid 2FA code. Please try again.")),
                        "InvalidTicket" => return Err(anyhow!("MFA session timed out or invalid ticket. Please log in again.")),
                        "TooManyRequests" => return Err(anyhow!("Too many verification attempts, please wait and try again")),
                        other => return Err(anyhow!("Error: {}", other)),
                    }
                }
            }

            if let Ok(api_err) = serde_json::from_str::<RevoltApiError>(&raw_body) {
                let description = match api_err.error_type.as_str() {
                    "InvalidCode" => "Invalid 2FA code. Please try again.",
                    "InvalidTicket" => "MFA session timed out or invalid ticket. Please log in again.",
                    "TooManyRequests" => "Too many verification attempts, please wait and try again",
                    other => other,
                };
                return Err(anyhow!("{}", description));
            }

            // Fallback to HTTP status info
            let clean_body = raw_body.trim();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(anyhow!("HTTP 404 - Login endpoint not found at {}", self.api_url));
            }
            if clean_body.is_empty() {
                return Err(anyhow!("Server returned HTTP {}", status));
            } else if clean_body.starts_with('<') {
                return Err(anyhow!("Server returned HTTP {} (HTML response - check API URL)", status));
            } else {
                let preview = if clean_body.len() > 100 {
                    format!("{}...", &clean_body[..100])
                } else {
                    clean_body.to_string()
                };
                return Err(anyhow!("Server returned HTTP {}: {}", status, preview));
            }
        }

        // Success deserialization: try tagged LoginResponse first, then direct session object
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

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw_body) {
            if let Some(tok) = val.get("token").and_then(|t| t.as_str()) {
                let user_id = val.get("user_id").and_then(|u| u.as_str()).map(|s| s.to_string());
                let id = val.get("_id").and_then(|i| i.as_str()).map(|s| s.to_string());
                return Ok(LoginResponse::Success {
                    token: tok.to_string(),
                    user_id,
                    _id: id,
                });
            }
        }

        Err(anyhow!(
            "Unexpected server response format (HTTP {}): {}",
            status,
            if raw_body.len() > 80 { format!("{}...", &raw_body[..80]) } else { raw_body }
        ))
    }

    pub async fn get_current_user(&self) -> Result<User> {
        let url = format!("{}/users/@me", self.api_url);
        log_debug!("GET {} (fetching current user profile)", url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to query current user at {}", url))?;

        let status = resp.status();
        log_debug!("GET {} -> Status: {}", url, status);

        if status == reqwest::StatusCode::UNAUTHORIZED {
            let err_text = resp.text().await.unwrap_or_default();
            log_warn!("GET {} failed with 401 Unauthorized (session expired or invalid): {}", url, err_text);
            return Err(anyhow!("401 Unauthorized: Session expired"));
        }

        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            log_warn!("GET {} returned error status {}: {}", url, status, err_text);
            return Err(anyhow!("Failed to get user ({}): {}", status, err_text));
        }

        let user: User = resp
            .json()
            .await
            .context("Failed to parse user JSON")?;
        log_info!("GET {} successful for user @{} ({})", url, user.username, user.id);
        Ok(user)
    }

    pub async fn fetch_user(&self, user_id: &str) -> Result<User> {
        let url = format!("{}/users/{}", self.api_url, user_id);
        log_debug!("GET {} (fetching user profile)", url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to fetch user at {}", url))?;

        let status = resp.status();
        log_debug!("GET {} -> Status: {}", url, status);
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            log_warn!("GET {} failed with status {}: {}", url, status, err_text);
            return Err(anyhow!("Failed to fetch user {}: HTTP {}", user_id, status));
        }

        let user: User = resp
            .json()
            .await
            .context("Failed to parse user JSON")?;
        log_info!("Fetched user: @{} ({})", user.username, user.id);
        Ok(user)
    }

    #[allow(dead_code)]
    pub async fn fetch_messages(&self, channel_id: &str, limit: u32) -> Result<Vec<Message>> {
        let url = format!(
            "{}/channels/{}/messages?limit={}&sort=Latest",
            self.api_url, channel_id, limit
        );
        log_debug!("GET {} (fetching messages)", url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to fetch messages for channel {}", channel_id))?;

        let status = resp.status();
        log_debug!("GET {} -> Status: {}", url, status);
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            log_warn!("GET {} returned error status {}: {}", url, status, err_text);
            return Err(anyhow!("Fetch messages error ({}): {}", status, err_text));
        }

        let text = resp.text().await?;
        if let Ok(list) = serde_json::from_str::<Vec<Message>>(&text) {
            log_debug!("Fetched {} messages for channel {}", list.len(), channel_id);
            return Ok(list);
        }

        #[derive(Deserialize)]
        struct MessagesWrapper {
            messages: Vec<Message>,
        }
        if let Ok(wrapper) = serde_json::from_str::<MessagesWrapper>(&text) {
            log_debug!("Fetched {} messages (wrapped) for channel {}", wrapper.messages.len(), channel_id);
            return Ok(wrapper.messages);
        }

        log_error!("Unrecognized messages format from API for channel {}: {}", channel_id, text);
        Err(anyhow!("Unrecognized messages format from API: {}", text))
    }

    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<Message> {
        let url = format!("{}/channels/{}/messages", self.api_url, channel_id);
        log_debug!("POST {} (sending message, len: {})", url, content.len());
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
        log_debug!("POST {} -> Status: {}", url, status);
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            log_warn!("POST {} returned error status {}: {}", url, status, err_text);
            return Err(anyhow!("Send message error ({}): {}", status, err_text));
        }

        let msg: Message = resp
            .json()
            .await
            .context("Failed to parse returned message JSON")?;
        log_debug!("Message {} posted successfully to channel {}", msg.id, channel_id);
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
