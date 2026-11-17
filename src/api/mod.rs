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

    pub async fn upload_autumn_attachment(
        &self,
        autumn_url: &str,
        filename: String,
        bytes: Vec<u8>,
    ) -> Result<String> {
        let url = format!("{}/attachments", autumn_url.trim_end_matches('/'));
        log_info!("Uploading file '{}' ({} bytes) to Autumn at {}...", filename, bytes.len(), url);

        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.clone());
        let form = reqwest::multipart::Form::new().part("file", part);

        let resp = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .with_context(|| format!("Failed to upload attachment to {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            log_error!("Autumn upload failed (HTTP {}): {}", status, err_text);
            return Err(anyhow!("Autumn upload returned HTTP {}: {}", status, err_text));
        }

        let upload_resp: AutumnUploadResponse = resp
            .json()
            .await
            .context("Failed to parse Autumn upload response")?;
        log_info!("Autumn file '{}' uploaded successfully with id {}", filename, upload_resp.id);
        Ok(upload_resp.id)
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
                    log_warn!(
                        "Detected AlreadyConnected ghost session; sending leave_call and retrying..."
                    );
                    self.leave_call(channel_id).await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    return match self
                        .try_call_request(reqwest::Method::POST, &primary_url, &payload)
                        .await
                    {
                        Ok(res) => Ok(res),
                        Err((_, retry_err)) => Err(anyhow!(retry_err)),
                    };
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
        let leave_url = format!("{}/channels/{}/leave_call", base_api, channel_id);
        log_info!("Calling POST {} to leave call cleanly...", leave_url);

        let fallback_needed = match self
            .client
            .post(&leave_url)
            .headers(self.headers())
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    log_info!("POST {} -> HTTP {} (left call)", leave_url, status);
                    false
                } else {
                    log_debug!("POST {} -> HTTP {} | body: {}", leave_url, status, body);
                    matches!(
                        status,
                        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
                    )
                }
            }
            Err(e) => {
                log_debug!("POST {} request error (ignored non-fatal): {}", leave_url, e);
                false
            }
        };

        if !fallback_needed {
            return;
        }

        let ring_url = format!("{}/channels/{}/ring", base_api, channel_id);
        match self
            .client
            .delete(&ring_url)
            .headers(self.headers())
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if status.is_success() {
                    log_info!("DELETE {} -> HTTP {} (ended call ring)", ring_url, status);
                    return;
                }
                log_debug!("DELETE {} -> HTTP {} | body: {}", ring_url, status, body);
            }
            Err(e) => {
                log_debug!("DELETE {} request error (ignored non-fatal): {}", ring_url, e);
            }
        }

        let end_ring_url = format!("{}/channels/{}/end_ring", base_api, channel_id);
        match self
            .client
            .post(&end_ring_url)
            .headers(self.headers())
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                log_debug!("POST {} -> HTTP {} | body: {}", end_ring_url, status, body);
            }
            Err(e) => {
                log_debug!("POST {} request error (ignored non-fatal): {}", end_ring_url, e);
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
        let resp = self
            .client
            .get(url)
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
