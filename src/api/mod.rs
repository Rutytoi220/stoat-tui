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
            .user_agent("stoat-tui/0.1.0")
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

    pub fn has_token(&self) -> bool {
        self.token
            .as_deref()
            .map(|t| !t.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    /// Extracts `scheme://authority` origin from an HTTP(S) URL.
    pub fn origin_from_url(raw_url: &str) -> Option<String> {
        let trimmed = raw_url.trim();
        let scheme_end = trimmed.find("://")?;
        let scheme = &trimmed[..scheme_end];
        let rest = &trimmed[scheme_end + 3..];
        let authority = rest.split('/').next().unwrap_or(rest).trim();
        if authority.is_empty() {
            None
        } else {
            Some(format!("{}://{}", scheme, authority))
        }
    }

    /// Resolves `features.autumn.url` (including relative paths like `"/autumn"`)
    /// against an instance's `api_url` origin, falling back to `{origin}/autumn`
    /// for self-hosted servers or `https://cdn.stoatusercontent.com` for primary Stoat.
    pub fn resolve_autumn_url(api_url: &str, raw_autumn: Option<&str>) -> String {
        let origin = Self::origin_from_url(api_url)
            .unwrap_or_else(|| api_url.trim().trim_end_matches('/').to_string());
        if let Some(raw) = raw_autumn.map(str::trim).filter(|s| !s.is_empty()) {
            if raw.starts_with("http://") || raw.starts_with("https://") {
                return raw.trim_end_matches('/').to_string();
            }
            let rel = raw.trim_start_matches('/').trim_end_matches('/');
            if rel.is_empty() {
                return origin;
            }
            return format!("{}/{}", origin, rel);
        }
        let host = crate::config::instance_host(api_url);
        if host.is_empty() || host == "api.stoat.chat" || host == "api.revolt.chat" {
            "https://cdn.stoatusercontent.com".to_string()
        } else {
            format!("{}/autumn", origin)
        }
    }

    /// Normalizes a user-entered server URL by trimming trailing `/` and prefixing `https://`
    /// (or `http://` for localhost / private LAN IPs) if no scheme was specified.
    pub fn normalize_url_str(raw_url: &str) -> String {
        let trimmed = raw_url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return "https://api.stoat.chat".to_string();
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return trimmed.to_string();
        }
        let host_port = trimmed.split('/').next().unwrap_or(trimmed);
        let host = host_port
            .split(':')
            .next()
            .unwrap_or(host_port)
            .to_ascii_lowercase();
        let is_local = host == "localhost"
            || host == "[::1]"
            || host.starts_with("127.")
            || host.starts_with("10.")
            || host.starts_with("192.168.")
            || (host.starts_with("172.")
                && host
                    .split('.')
                    .nth(1)
                    .and_then(|o| o.parse::<u8>().ok())
                    .map(|o| (16..=31).contains(&o))
                    .unwrap_or(false));
        let scheme = if is_local { "http" } else { "https" };
        format!("{}://{}", scheme, trimmed)
    }

    fn is_delta_metadata_json(body: &str) -> bool {
        let trimmed = body.trim();
        if trimmed.is_empty() || trimmed.starts_with('<') {
            return false;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(obj) = val.as_object() {
                return obj.contains_key("revolt")
                    || obj.contains_key("features")
                    || obj.contains_key("stoat");
            }
        }
        false
    }

    /// Normalizes `raw_url` and probes `GET {url}/` for Delta's root metadata (`"revolt"` or `"features"`).
    /// If `/` returns HTML, 404, or lacks Delta's metadata signature, automatically probes `GET {url}/api/`
    /// and returns `{url}/api` when that endpoint serves Delta's root JSON.
    pub async fn normalize_and_probe_api_url(raw_url: &str) -> String {
        let base = Self::normalize_url_str(raw_url);
        let probe_client = Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent("stoat-tui/0.1.0")
            .build()
            .unwrap_or_else(|_| Client::new());

        let root_probe = format!("{}/", base);
        log_debug!("Probing Stoat API root metadata at GET {}", root_probe);
        if let Ok(resp) = probe_client.get(&root_probe).send().await {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() && Self::is_delta_metadata_json(&body) {
                log_info!("GET {} confirmed valid Delta API root (HTTP {})", root_probe, status);
                return base;
            }
            let preview: String = body.chars().take(120).collect();
            log_debug!(
                "GET {} did not match Delta metadata (HTTP {}, preview={:?}); checking /api fallback...",
                root_probe,
                status,
                preview
            );
        } else {
            log_debug!("GET {} probe failed to connect", root_probe);
            return base;
        }

        if !base.ends_with("/api") {
            let api_base = format!("{}/api", base);
            for candidate_probe in [format!("{}/", api_base), api_base.clone()] {
                log_debug!("Probing fallback Stoat API path at GET {}", candidate_probe);
                if let Ok(resp) = probe_client.get(&candidate_probe).send().await {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if status.is_success() && Self::is_delta_metadata_json(&body) {
                        log_info!(
                            "Auto-detected self-hosted Delta API base URL: {} (via GET {} -> HTTP {})",
                            api_base,
                            candidate_probe,
                            status
                        );
                        return api_base;
                    }
                }
            }
        }

        base
    }

    fn redact_auth_response(raw_body: &str) -> String {
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(raw_body) {
            if let Some(obj) = val.as_object_mut() {
                for secret_key in ["token", "password", "ticket", "mfa_ticket"] {
                    if let Some(s) = obj.get(secret_key).and_then(|v| v.as_str()) {
                        let len = s.len();
                        obj.insert(
                            secret_key.to_string(),
                            serde_json::Value::String(format!("<redacted len={}>", len)),
                        );
                    }
                }
            }
            val.to_string()
        } else {
            raw_body.chars().take(300).collect()
        }
    }

    fn format_structured_auth_error(status: reqwest::StatusCode, raw_body: &str) -> Option<String> {
        let val = serde_json::from_str::<serde_json::Value>(raw_body).ok()?;
        let obj = val.as_object()?;
        let err_type = obj
            .get("type")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("error").and_then(|v| v.as_str()))?;

        let description = match err_type {
            "InvalidCredentials" => "InvalidCredentials: Invalid email or password",
            "UnverifiedAccount" => "UnverifiedAccount: Account email is not verified",
            "UserNotFound" => "UserNotFound: Account not found",
            "FailedValidation" => "FailedValidation: Invalid email or password format",
            "InvalidSession" => "InvalidSession: Invalid or expired session",
            "CompromisedPassword" => "CompromisedPassword: Compromised password, please reset password",
            "Blocked" => "Blocked: Account access blocked",
            "LockedOut" => "LockedOut: Account temporarily locked out",
            "TooManyRequests" => "TooManyRequests: Too many login attempts, please wait and try again",
            other => {
                let extra = obj
                    .get("message")
                    .or_else(|| obj.get("error"))
                    .and_then(|v| v.as_str())
                    .filter(|s| *s != other)
                    .unwrap_or("");
                return if extra.is_empty() {
                    Some(format!("{} (HTTP {})", other, status.as_u16()))
                } else {
                    Some(format!("{} (HTTP {}): {}", other, status.as_u16(), extra))
                };
            }
        };

        let extra = obj
            .get("message")
            .or_else(|| obj.get("error"))
            .and_then(|v| v.as_str())
            .filter(|s| *s != err_type)
            .unwrap_or("");
        if extra.is_empty() {
            Some(format!("{} (HTTP {})", description, status.as_u16()))
        } else {
            Some(format!("{} (HTTP {}): {}", description, status.as_u16(), extra))
        }
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

        let raw_text = resp
            .text()
            .await
            .context("Failed to read server metadata response body")?;

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw_text) {
            if let Some(features) = val.get("features") {
                if let Some(lk) = features.get("livekit") {
                    log_info!("GET / features.livekit JSON: {}", lk);
                }
                if let Some(voice) = features.get("voice") {
                    log_info!("GET / features.voice JSON: {}", voice);
                }
            }
        }

        let metadata: ServerMetadata = serde_json::from_str(&raw_text)
            .context("Failed to parse server metadata JSON")?;
        Ok(metadata)
    }

    pub async fn login_email(&self, email: &str, password: &str) -> Result<LoginResponse> {
        let url = format!("{}/auth/session/login", self.api_url.trim_end_matches('/'));
        let email_trimmed = email.trim();
        log_info!(
            "POST {} | payload={{\"email\":{:?},\"password\":\"<redacted>\",\"friendly_name\":\"stoat-tui\"}}",
            url,
            email_trimmed
        );
        let payload = serde_json::json!({
            "email": email_trimmed,
            "password": password,
            "friendly_name": "stoat-tui"
        });

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
        let redacted = Self::redact_auth_response(&raw_body);
        log_info!("POST {} -> HTTP {} | body: {}", url, status, redacted);

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw_body) {
            if val.get("result").and_then(|v| v.as_str()) == Some("MFA") {
                let msg = "Account has 2FA/MFA enabled — log in via Token tab instead";
                log_warn!("POST {} returned result=MFA: {}", url, msg);
                return Err(anyhow!(msg));
            }
            if val.get("result").and_then(|v| v.as_str()) == Some("Disabled") {
                return Err(anyhow!("Account is disabled"));
            }
        }

        if !status.is_success() {
            log_warn!("Login failed at {} with HTTP {}: {}", url, status, redacted);
            if let Some(structured) = Self::format_structured_auth_error(status, &raw_body) {
                return Err(anyhow!(structured));
            }

            let clean_body = raw_body.trim();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(anyhow!(
                    "HTTP 404 - Check API URL (endpoint not found at {})",
                    self.api_url
                ));
            }
            if clean_body.is_empty() {
                return Err(anyhow!("Server returned HTTP {}", status));
            } else if clean_body.starts_with('<') {
                return Err(anyhow!(
                    "Server returned HTTP {} (HTML response - verify API URL)",
                    status
                ));
            } else {
                let preview = if clean_body.len() > 100 {
                    format!("{}...", &clean_body[..100])
                } else {
                    clean_body.to_string()
                };
                return Err(anyhow!("Server returned HTTP {}: {}", status, preview));
            }
        }

        if let Ok(login_resp) = serde_json::from_str::<LoginResponse>(&raw_body) {
            return match login_resp {
                LoginResponse::MFA { .. } => Err(anyhow!(
                    "Account has 2FA/MFA enabled — log in via Token tab instead"
                )),
                LoginResponse::Disabled => Err(anyhow!("Account is disabled")),
                other => Ok(other),
            };
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
            if raw_body.len() > 80 {
                format!("{}...", &raw_body[..80])
            } else {
                raw_body
            }
        ))
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<LoginResponse> {
        self.login_email(email, password).await
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

    pub async fn fetch_me(&self, token: &str) -> Result<User> {
        let url = format!("{}/users/@me", self.api_url.trim_end_matches('/'));
        let header_name = if self.is_bot {
            "x-bot-token"
        } else {
            "x-session-token"
        };
        let token_trimmed = token.trim();
        log_debug!("GET {} (fetching current user profile with header {})", url, header_name);

        let resp = self
            .client
            .get(&url)
            .header(header_name, token_trimmed)
            .send()
            .await
            .with_context(|| format!("Failed to query current user at {}", url))?;

        let status = resp.status();
        log_debug!("GET {} -> Status: {}", url, status);

        if status == reqwest::StatusCode::UNAUTHORIZED {
            let err_text = resp.text().await.unwrap_or_default();
            log_warn!(
                "GET {} failed with 401 Unauthorized (session expired or invalid) [header={}]: {}",
                url,
                header_name,
                err_text
            );
            if let Some(structured) = Self::format_structured_auth_error(status, &err_text) {
                return Err(anyhow!("401 Unauthorized: {}", structured));
            }
            return Err(anyhow!("401 Unauthorized: Session expired"));
        }

        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            log_warn!(
                "GET {} returned non-200 status {} [header={}]: {}",
                url,
                status,
                header_name,
                err_text
            );
            if let Some(structured) = Self::format_structured_auth_error(status, &err_text) {
                return Err(anyhow!("{}", structured));
            }
            return Err(anyhow!("Failed to get user ({}): {}", status, err_text));
        }

        let user: User = resp
            .json()
            .await
            .context("Failed to parse user JSON")?;
        log_info!("GET {} successful for user @{} ({})", url, user.username, user.id);
        Ok(user)
    }

    pub async fn get_current_user(&self) -> Result<User> {
        if let Some(ref token) = self.token {
            self.fetch_me(token).await
        } else {
            Err(anyhow!("No session token set"))
        }
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
        self.send_message_with_nonce(
            channel_id,
            if content.trim().is_empty() { None } else { Some(content.to_string()) },
            None,
            None,
        )
        .await
    }

    pub async fn send_message_with_attachments(
        &self,
        channel_id: &str,
        content: Option<String>,
        attachments: Option<Vec<String>>,
    ) -> Result<Message> {
        self.send_message_with_nonce(channel_id, content, attachments, None)
            .await
    }

    pub async fn send_message_with_nonce(
        &self,
        channel_id: &str,
        content: Option<String>,
        attachments: Option<Vec<String>>,
        nonce: Option<String>,
    ) -> Result<Message> {
        let url = format!("{}/channels/{}/messages", self.api_url, channel_id);
        log_debug!(
            "POST {} (sending message, content_len: {:?}, attachments: {:?}, nonce: {:?})",
            url,
            content.as_ref().map(|c| c.len()),
            attachments.as_ref().map(|a| a.len()),
            nonce
        );
        let payload = SendMessagePayload {
            content,
            attachments,
            replies: None,
            nonce,
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

    pub async fn upload_autumn_bytes(
        &self,
        autumn_url: &str,
        tag: &str,
        filename: String,
        bytes: Vec<u8>,
    ) -> Result<String> {
        let url = format!(
            "{}/{}",
            autumn_url.trim_end_matches('/'),
            tag.trim_matches('/')
        );
        log_info!(
            "Uploading file '{}' ({} bytes) to Autumn at {}...",
            filename,
            bytes.len(),
            url
        );

        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename.clone());
        let form = reqwest::multipart::Form::new().part("file", part);

        let mut req = self.client.post(&url).multipart(form);
        if let Some(ref token) = self.token {
            let header_key = if self.is_bot {
                "x-bot-token"
            } else {
                "x-session-token"
            };
            if let Ok(val) = HeaderValue::from_str(token.trim()) {
                req = req.header(header_key, val);
            }
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to upload {} to {}", tag, url))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            log_error!("Autumn upload ({}) failed (HTTP {}): {}", tag, status, err_text);
            return Err(anyhow!("Autumn upload returned HTTP {}: {}", status, err_text));
        }

        let upload_resp: AutumnUploadResponse = resp
            .json()
            .await
            .context("Failed to parse Autumn upload response")?;
        log_info!(
            "Autumn file '{}' uploaded to {} successfully with id {}",
            filename,
            tag,
            upload_resp.id
        );
        Ok(upload_resp.id)
    }

    pub async fn upload_autumn_attachment(
        &self,
        autumn_url: &str,
        filename: String,
        bytes: Vec<u8>,
    ) -> Result<String> {
        self.upload_autumn_bytes(autumn_url, "attachments", filename, bytes)
            .await
    }

    pub async fn upload_autumn_tag(
        &self,
        autumn_url: &str,
        tag: &str,
        file_path: &std::path::Path,
    ) -> Result<String> {
        let filename = file_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("upload.png")
            .to_string();
        let bytes = tokio::fs::read(file_path)
            .await
            .with_context(|| format!("Failed to read file {}", file_path.display()))?;
        self.upload_autumn_bytes(autumn_url, tag, filename, bytes)
            .await
    }

    fn parse_retry_after_ms(headers: &HeaderMap, raw_body: &str) -> u64 {
        for key in ["retry-after", "x-ratelimit-reset-after"] {
            if let Some(val) = headers.get(key).and_then(|v| v.to_str().ok()) {
                if let Ok(secs) = val.trim().parse::<f64>() {
                    if secs.is_finite() && secs >= 0.0 {
                        return ((secs * 1000.0).round() as u64).clamp(50, 5000);
                    }
                }
            }
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw_body) {
            if let Some(ms) = val.get("retry_after").and_then(|v| v.as_u64()) {
                return ms.clamp(50, 5000);
            }
            if let Some(ms_f) = val.get("retry_after").and_then(|v| v.as_f64()) {
                if ms_f.is_finite() && ms_f >= 0.0 {
                    return (ms_f.round() as u64).clamp(50, 5000);
                }
            }
        }
        1500
    }

    async fn patch_json_with_429_retry<T: serde::Serialize + ?Sized>(
        &self,
        url: &str,
        payload: &T,
    ) -> Result<(reqwest::StatusCode, String)> {
        let mut attempt = 0u8;
        loop {
            let resp = self
                .client
                .patch(url)
                .headers(self.headers())
                .json(payload)
                .send()
                .await
                .with_context(|| format!("Failed to PATCH {}", url))?;

            let status = resp.status();
            let resp_headers = resp.headers().clone();
            let body = resp.text().await.unwrap_or_default();
            log_debug!("PATCH {} -> HTTP {}", url, status);

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait_ms = Self::parse_retry_after_ms(&resp_headers, &body);
                if attempt == 0 {
                    attempt += 1;
                    log_warn!(
                        "PATCH {} hit HTTP 429; waiting {}ms before automatic retry...",
                        url,
                        wait_ms
                    );
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                let secs = ((wait_ms as f64) / 1000.0).ceil().max(1.0) as u64;
                return Err(anyhow!("Rate limited — try again in {}s", secs));
            }

            return Ok((status, body));
        }
    }

    pub async fn edit_user(&self, user_id: &str, payload: EditUserRequest) -> Result<User> {
        let url = format!("{}/users/{}", self.api_url.trim_end_matches('/'), user_id);
        log_info!("PATCH {} (editing user profile)", url);
        let (status, body) = self.patch_json_with_429_retry(&url, &payload).await?;

        if !status.is_success() {
            log_warn!("PATCH {} failed (HTTP {}): {}", url, status, body);
            if let Some(structured) = Self::format_structured_auth_error(status, &body) {
                return Err(anyhow!(structured));
            }
            return Err(anyhow!("Profile update failed (HTTP {}): {}", status, body));
        }

        let user: User = serde_json::from_str(&body).context("Failed to parse updated User JSON")?;
        Ok(user)
    }

    pub async fn change_username(&self, new_username: &str, password: &str) -> Result<User> {
        let url = format!("{}/users/@me/username", self.api_url.trim_end_matches('/'));
        let trimmed = new_username.trim();
        log_info!("PATCH {} (changing username to '{}')", url, trimmed);
        let payload = serde_json::json!({
            "username": trimmed,
            "password": password,
        });

        let (status, body) = self.patch_json_with_429_retry(&url, &payload).await?;

        if !status.is_success() {
            log_warn!("PATCH {} failed (HTTP {}): {}", url, status, body);
            if let Some(structured) = Self::format_structured_auth_error(status, &body) {
                return Err(anyhow!(structured));
            }
            return Err(anyhow!("Username change failed (HTTP {}): {}", status, body));
        }

        let user: User = serde_json::from_str(&body).context("Failed to parse updated User JSON")?;
        Ok(user)
    }

    pub async fn fetch_server_members(&self, server_id: &str) -> Result<ServerMembersResponse> {
        let url = format!("{}/servers/{}/members", self.api_url, server_id);
        log_debug!("GET {} (fetching server members)", url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to fetch server members at {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetch server members error ({}): {}", status, err_text));
        }

        let res: ServerMembersResponse = resp
            .json()
            .await
            .context("Failed to parse server members JSON")?;
        Ok(res)
    }

    pub async fn fetch_user_profile(&self, user_id: &str) -> Result<UserProfile> {
        let url = format!("{}/users/{}/profile", self.api_url, user_id);
        log_debug!("GET {} (fetching user profile)", url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .with_context(|| format!("Failed to fetch user profile at {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetch user profile error ({}): {}", status, err_text));
        }

        let res: UserProfile = resp
            .json()
            .await
            .context("Failed to parse user profile JSON")?;
        Ok(res)
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

    async fn try_call_request(
        &self,
        method: reqwest::Method,
        url: &str,
        payload: &serde_json::Value,
    ) -> std::result::Result<CallJoinResponse, (Option<reqwest::StatusCode>, String)> {
        log_info!("Calling {} {} (payload={})...", method, url, payload);

        let resp = self
            .client
            .request(method.clone(), url)
            .headers(self.headers())
            .json(payload)
            .send()
            .await
            .map_err(|e| {
                log_error!("{} {} transport error: {}", method, url, e);
                (None, format!("Failed to call {} at {}: {}", method, url, e))
            })?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        // Sanitize token when JSON, otherwise log first 300 chars preview
        let preview = if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(obj) = val.as_object_mut() {
                if let Some(tok) = obj.get("token").and_then(|v| v.as_str()) {
                    let len = tok.len();
                    obj.insert(
                        "token".to_string(),
                        serde_json::Value::String(format!("<redacted len={}>", len)),
                    );
                }
            }
            let s = val.to_string();
            s.chars().take(300).collect::<String>()
        } else {
            body.chars().take(300).collect::<String>()
        };
        log_debug!("{} {} -> HTTP {} | preview: {}", method, url, status, preview);

        if !status.is_success() {
            // Log full untruncated error body to debug.log for diagnostics
            log_error!("{} {} failed with HTTP {} | full body: {}", method, url, status, body);
            return Err((Some(status), format!("Join call error (HTTP {}): {}", status, body)));
        }

        let join_resp: CallJoinResponse = serde_json::from_str(&body).map_err(|e| {
            log_error!("Failed to parse {} {} response JSON: {} | body: {}", method, url, e, body);
            (Some(status), format!("Failed to parse call response JSON: {}", e))
        })?;
        Ok(join_resp)
    }

    pub async fn join_call(&self, channel_id: &str, node_name: Option<&str>) -> Result<CallJoinResponse> {
        let base_api = self.api_url.trim_end_matches('/');
        let primary_url = format!("{}/channels/{}/join_call", base_api, channel_id);
        let initial_node = node_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("worldwide")
            .to_string();
        let payload = serde_json::json!({ "node": initial_node });

        match self.try_call_request(reqwest::Method::POST, &primary_url, &payload).await {
            Ok(res) => Ok(res),
            Err((status_opt, primary_err)) => {
                if status_opt == Some(reqwest::StatusCode::BAD_REQUEST)
                    && primary_err.contains("AlreadyConnected")
                {
                    let mut last_err = primary_err;
                    let backoff_ms = [300u64, 600, 1000];
                    for (idx, delay_ms) in backoff_ms.iter().enumerate() {
                        log_warn!(
                            "Detected AlreadyConnected ghost session (attempt {}/{}); sending leave_call and retrying in {}ms...",
                            idx + 1,
                            backoff_ms.len(),
                            delay_ms
                        );
                        self.leave_call(channel_id).await;
                        tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
                        match self
                            .try_call_request(reqwest::Method::POST, &primary_url, &payload)
                            .await
                        {
                            Ok(res) => return Ok(res),
                            Err((Some(reqwest::StatusCode::BAD_REQUEST), retry_err))
                                if retry_err.contains("AlreadyConnected") =>
                            {
                                last_err = retry_err;
                            }
                            Err((_, retry_err)) => return Err(anyhow!(retry_err)),
                        }
                    }
                    return Err(anyhow!(last_err));
                }

                let should_fallback = matches!(
                    status_opt,
                    Some(reqwest::StatusCode::NOT_FOUND) | Some(reqwest::StatusCode::BAD_REQUEST)
                );
                if !should_fallback {
                    return Err(anyhow!(primary_err));
                }

                log_warn!(
                    "Primary join_call returned {:?}; inspecting root metadata and trying discovered node / fallback voice endpoints...",
                    status_opt
                );

                // Check GET / root metadata for features.livekit / features.voice nodes
                let (meta_node, meta_voice_url) = match self.fetch_metadata().await {
                    Ok(meta) => {
                        let (n, u) = meta
                            .features
                            .as_ref()
                            .and_then(|f| f.livekit.as_ref().or(f.voice.as_ref()))
                            .map(|lk| lk.resolved_node())
                            .unwrap_or((None, None));
                        log_debug!("Fallback metadata voice/livekit node={:?} url={:?}", n, u);
                        (n, u)
                    }
                    Err(e) => {
                        log_debug!("Fallback metadata fetch failed: {}", e);
                        (None, None)
                    }
                };

                let fallback_node = meta_node
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(initial_node.as_str())
                    .to_string();
                let fallback_payload = serde_json::json!({ "node": fallback_node });

                // If GET / revealed a different node name than initial_node, retry /join_call with it first
                if fallback_node != initial_node {
                    if let Ok(mut res) = self
                        .try_call_request(reqwest::Method::POST, &primary_url, &fallback_payload)
                        .await
                    {
                        if res.url.as_deref().map(str::trim).unwrap_or("").is_empty() {
                            res.url = meta_voice_url;
                        }
                        return Ok(res);
                    }
                }

                let fallback_url = format!("{}/channels/{}/call", base_api, channel_id);
                for method in [reqwest::Method::POST, reqwest::Method::PUT] {
                    if let Ok(mut res) = self.try_call_request(method, &fallback_url, &fallback_payload).await {
                        if res.url.as_deref().map(str::trim).unwrap_or("").is_empty() {
                            res.url = meta_voice_url;
                        }
                        return Ok(res);
                    }
                }

                Err(anyhow!(primary_err))
            }
        }
    }

    pub async fn leave_call(&self, channel_id: &str) {
        let base_api = self.api_url.trim_end_matches('/');
        let attempts = [
            (
                reqwest::Method::POST,
                format!("{}/channels/{}/leave_call", base_api, channel_id),
                true,
            ),
            (
                reqwest::Method::DELETE,
                format!("{}/channels/{}/call", base_api, channel_id),
                false,
            ),
            (
                reqwest::Method::DELETE,
                format!("{}/channels/{}/ring", base_api, channel_id),
                false,
            ),
            (
                reqwest::Method::POST,
                format!("{}/channels/{}/end_ring", base_api, channel_id),
                true,
            ),
        ];

        log_info!("Disconnecting call session for channel {}...", channel_id);
        for (method, url, send_json) in attempts {
            let mut req = self.client.request(method.clone(), &url).headers(self.headers());
            if send_json {
                req = req.json(&serde_json::json!({}));
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        log_info!("{} {} -> HTTP {} (call session left)", method, url, status);
                        return;
                    }
                    log_debug!("{} {} -> HTTP {} | body: {}", method, url, status, body);
                }
                Err(e) => {
                    log_debug!("{} {} request error (ignored non-fatal): {}", method, url, e);
                }
            }
        }
    }

    pub async fn ring_channel(&self, channel_id: &str) {
        let base_api = self.api_url.trim_end_matches('/');
        let url = format!("{}/channels/{}/ring", base_api, channel_id);
        log_info!("Calling POST {} to ring DM/group participants...", url);
        match self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    log_info!("POST {} -> HTTP {} (ring sent)", url, status);
                } else {
                    log_debug!("POST {} -> HTTP {} (ignored non-fatal ring response): {}", url, status, body);
                }
            }
            Err(e) => {
                log_debug!("POST {} request error (ignored non-fatal): {}", url, e);
            }
        }
    }

    pub async fn fetch_bytes_with_status(&self, url: &str) -> Result<(reqwest::StatusCode, Vec<u8>)> {
        let mut req = self.client.get(url);
        if let (Some(api_origin), Some(asset_origin)) = (
            Self::origin_from_url(&self.api_url),
            Self::origin_from_url(url),
        ) {
            if api_origin.eq_ignore_ascii_case(&asset_origin) {
                req = req.headers(self.headers());
            }
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to fetch asset from {}", url))?;
        let status = resp.status();
        if !status.is_success() {
            let body_preview = resp.text().await.unwrap_or_default();
            log_error!("Asset fetch from {} returned HTTP {}: {}", url, status, body_preview);
            return Err(anyhow!("Asset fetch returned HTTP {}: {}", status, body_preview));
        }
        let bytes = resp.bytes().await.context("Failed to read asset bytes")?;
        Ok((status, bytes.to_vec()))
    }

    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let (_, bytes) = self.fetch_bytes_with_status(url).await?;
        Ok(bytes)
    }
}
