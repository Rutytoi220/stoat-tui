use anyhow::{Context, Result};
#[cfg(not(test))]
use directories::ProjectDirs;
use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const DEFAULT_STOAT_API: &str = "https://api.stoat.chat";
pub const DEFAULT_REVOLT_API: &str = "https://api.revolt.chat";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedAccount {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub api_url: String,
    pub token: String,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// Extracts the full hostname (without scheme, path, or port) from an API URL.
pub fn instance_host(api_url: &str) -> String {
    let trimmed = api_url.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let authority = without_scheme.split('/').next().unwrap_or("");
    authority.split(':').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// Derives a concise display host label from an instance API URL.
/// Primary Stoat/Revolt hosts return `None`; self-hosted instances return `Some("short-name")`
/// (e.g. `https://alpha-node.tail1234.ts.net/api` -> `Some("alpha-node")`).
pub fn short_instance_label(api_url: &str) -> Option<String> {
    let host = instance_host(api_url);
    if host.is_empty()
        || host == "api.stoat.chat"
        || host == "stoat.chat"
        || host == "api.revolt.chat"
        || host == "revolt.chat"
    {
        return None;
    }

    // Keep raw IPv4 addresses and localhost intact
    if host == "localhost" || host.split('.').all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit())) {
        return Some(host);
    }

    let mut clean = host.as_str();
    if let Some(rest) = clean.strip_prefix("api.") {
        clean = rest;
    }

    for suffix in [
        ".ts.net",
        ".local",
        ".lan",
        ".chat",
        ".com",
        ".net",
        ".org",
        ".io",
        ".dev",
        ".app",
    ] {
        if let Some(stripped) = clean.strip_suffix(suffix) {
            clean = stripped;
            break;
        }
    }

    let first_seg = clean.split('.').next().unwrap_or(clean).trim();
    if first_seg.is_empty() {
        Some(host)
    } else {
        Some(first_seg.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Base API URL (e.g. "https://api.stoat.chat" or "https://api.revolt.chat")
    #[serde(default = "default_api_url")]
    pub api_url: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,

    #[serde(default)]
    pub is_bot: bool,

    #[serde(default)]
    pub last_in_dm: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_server_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_channel_id: Option<String>,

    #[serde(default = "default_sidebar_width")]
    pub left_sidebar_width: u16,

    #[serde(default = "default_true")]
    pub call_sounds_enabled: bool,

    #[serde(default)]
    pub compact_mode: bool,

    #[serde(default)]
    pub theme: ThemeConfig,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<SavedAccount>,

    #[serde(skip)]
    pub explicit_token_cleared: bool,

    #[serde(skip)]
    pub custom_path: Option<PathBuf>,
}

pub const ACCENT_PRESETS: [(&str, &str); 5] = [
    ("Stoat Pink", "#FD6671"),
    ("Blurple", "#5970FD"),
    ("Emerald", "#23A55A"),
    ("Amber", "#F0B232"),
    ("Cyan", "#00A8FC"),
];

fn default_true() -> bool {
    true
}

fn default_sidebar_width() -> u16 {
    26
}

fn default_api_url() -> String {
    DEFAULT_STOAT_API.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_primary")]
    pub primary: String,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default = "default_sidebar_bg")]
    pub sidebar_bg: String,
    #[serde(default = "default_rail_bg")]
    pub rail_bg: String,
    #[serde(default = "default_text")]
    pub text: String,
    #[serde(default = "default_muted")]
    pub muted: String,
}

fn default_primary() -> String {
    "#FD6671".to_string() // Stoat/Revolt signature vibrant coral/red
}
fn default_accent() -> String {
    "#5970FD".to_string() // Stoat accent blue/indigo
}
fn default_background() -> String {
    "#191919".to_string()
}
fn default_sidebar_bg() -> String {
    "#202020".to_string()
}
fn default_rail_bg() -> String {
    "#151515".to_string()
}
fn default_text() -> String {
    "#F6F6F6".to_string()
}
fn default_muted() -> String {
    "#848484".to_string()
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            primary: default_primary(),
            accent: default_accent(),
            background: default_background(),
            sidebar_bg: default_sidebar_bg(),
            rail_bg: default_rail_bg(),
            text: default_text(),
            muted: default_muted(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
            gateway_url: None,
            token: None,
            is_bot: false,
            last_in_dm: false,
            last_server_id: None,
            last_channel_id: None,
            left_sidebar_width: default_sidebar_width(),
            call_sounds_enabled: true,
            compact_mode: false,
            theme: ThemeConfig::default(),
            accounts: Vec::new(),
            explicit_token_cleared: false,
            custom_path: None,
        }
    }
}

pub fn parse_hex_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&hex[0..2], 16),
            u8::from_str_radix(&hex[2..4], 16),
            u8::from_str_radix(&hex[4..6], 16),
        ) {
            return Color::Rgb(r, g, b);
        }
    }
    Color::Reset
}

impl Config {
    pub fn toggle_api_fallback(&mut self) {
        if self.api_url.contains("revolt.chat") {
            self.api_url = DEFAULT_STOAT_API.to_string();
        } else {
            self.api_url = DEFAULT_REVOLT_API.to_string();
        }
    }

    pub fn clear_token(&mut self) {
        self.token = None;
        self.accounts.clear();
        self.explicit_token_cleared = true;
    }

    /// Migrates legacy single-account `api_url` + `token` into `accounts[0]` if `accounts` is empty.
    pub fn migrate_legacy_account(&mut self) {
        if self.accounts.is_empty() {
            if let Some(ref tok) = self.token {
                let trimmed = tok.trim();
                if !trimmed.is_empty() {
                    let label = short_instance_label(&self.api_url).unwrap_or_else(|| "primary".to_string());
                    self.accounts.push(SavedAccount {
                        id: "primary".to_string(),
                        label,
                        api_url: self.api_url.trim_end_matches('/').to_string(),
                        token: trimmed.to_string(),
                        is_bot: self.is_bot,
                        user_id: None,
                        username: None,
                    });
                }
            }
        } else if self.token.is_none() && !self.explicit_token_cleared {
            if let Some(first) = self.accounts.first() {
                if !first.token.trim().is_empty() {
                    self.token = Some(first.token.clone());
                    self.api_url = first.api_url.clone();
                    self.is_bot = first.is_bot;
                }
            }
        }
    }

    /// Inserts or updates a saved account, returning its stable `account_id`.
    pub fn upsert_account(&mut self, api_url: String, token: String, is_bot: bool, replace_primary: bool) -> String {
        let normalized_url = api_url.trim().trim_end_matches('/').to_string();
        let trimmed_tok = token.trim().to_string();
        let label = short_instance_label(&normalized_url).unwrap_or_else(|| "primary".to_string());
        self.explicit_token_cleared = false;

        if replace_primary || self.accounts.is_empty() {
            let id = self
                .accounts
                .first()
                .map(|a| a.id.clone())
                .unwrap_or_else(|| "primary".to_string());
            let entry = SavedAccount {
                id: id.clone(),
                label,
                api_url: normalized_url.clone(),
                token: trimmed_tok.clone(),
                is_bot,
                user_id: None,
                username: None,
            };
            if self.accounts.is_empty() {
                self.accounts.push(entry);
            } else {
                self.accounts[0] = entry;
            }
            self.api_url = normalized_url;
            self.token = Some(trimmed_tok);
            self.is_bot = is_bot;
            return id;
        }

        // Update existing entry matching same normalized api_url if any, else append
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.api_url == normalized_url) {
            existing.token = trimmed_tok;
            existing.is_bot = is_bot;
            existing.label = label;
            return existing.id.clone();
        }

        let base = if label == "primary" { "stoat" } else { label.as_str() };
        let mut candidate = base.to_string();
        let mut n = 2usize;
        while self.accounts.iter().any(|a| a.id == candidate) {
            candidate = format!("{}-{}", base, n);
            n += 1;
        }
        self.accounts.push(SavedAccount {
            id: candidate.clone(),
            label,
            api_url: normalized_url,
            token: trimmed_tok,
            is_bot,
            user_id: None,
            username: None,
        });
        candidate
    }

    /// Removes a connected account by `account_id` and updates primary legacy fields cleanly.
    pub fn remove_account(&mut self, account_id: &str) -> bool {
        let before = self.accounts.len();
        self.accounts.retain(|a| a.id != account_id);
        if self.accounts.len() == before {
            return false;
        }
        if let Some(first) = self.accounts.first() {
            self.api_url = first.api_url.clone();
            self.token = Some(first.token.clone());
            self.is_bot = first.is_bot;
        } else {
            self.token = None;
            self.explicit_token_cleared = true;
        }
        true
    }

    pub fn config_dir() -> PathBuf {
        if let Ok(env_dir) = std::env::var("STOAT_TUI_CONFIG_DIR") {
            let trimmed = env_dir.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }

        #[cfg(test)]
        {
            return std::env::temp_dir().join(format!("stoat-tui-unit-test-{}", std::process::id()));
        }

        #[cfg(not(test))]
        {
            if let Some(proj_dirs) = ProjectDirs::from("chat", "stoat", "stoat-tui") {
                proj_dirs.config_dir().to_path_buf()
            } else {
                PathBuf::from(".stoat-tui")
            }
        }
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn nav_cache_path(&self) -> PathBuf {
        if let Some(ref custom) = self.custom_path {
            if let Some(dir) = custom.parent() {
                return dir.join("nav_cache.json");
            }
        }
        NavCache::cache_path()
    }

    pub fn load_from(path: &Path) -> Self {
        if path.exists() {
            if let Ok(contents) = fs::read_to_string(path) {
                if let Ok(mut config) = toml::from_str::<Config>(&contents) {
                    config.custom_path = Some(path.to_path_buf());
                    config.migrate_legacy_account();
                    return config;
                }
            }
        }
        Config {
            custom_path: Some(path.to_path_buf()),
            ..Default::default()
        }
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config file at {:?}", path))?;
            let mut config: Config = toml::from_str(&contents)
                .with_context(|| format!("Failed to parse config file at {:?}", path))?;
            config.custom_path = None;
            config.migrate_legacy_account();
            Ok(config)
        } else {
            let default_config = Config::default();
            // Try creating the directory and file
            let _ = default_config.save_to(&path);
            Ok(default_config)
        }
    }

    pub fn save(&self) -> Result<()> {
        let default_path = Self::config_path();
        let target_path = self.custom_path.as_deref().unwrap_or(&default_path);
        self.save_to(target_path).map_err(|e| anyhow::anyhow!(e))
    }

    pub fn save_to(&self, path: &Path) -> std::result::Result<(), String> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)
                    .map_err(|e| format!("Failed to create config directory {:?}: {}", dir, e))?;
            }
        }

        let mut config_to_write = self.clone();

        // Enforce token disk preservation:
        // If current in-memory token is None, but target config file on disk currently contains a non-empty token,
        // and clear_token() was NOT explicitly called, preserve the existing token from disk.
        if config_to_write.token.is_none() && !config_to_write.explicit_token_cleared && path.exists() {
            if let Ok(existing_str) = fs::read_to_string(path) {
                if let Ok(existing_cfg) = toml::from_str::<Config>(&existing_str) {
                    if let Some(existing_tok) = existing_cfg.token {
                        if !existing_tok.trim().is_empty() {
                            crate::log_info!("Preserving existing token from disk in {:?}", path);
                            config_to_write.token = Some(existing_tok);
                        }
                    }
                    if config_to_write.accounts.is_empty() && !existing_cfg.accounts.is_empty() {
                        config_to_write.accounts = existing_cfg.accounts;
                    }
                }
            }
        }

        let toml_str = toml::to_string_pretty(&config_to_write)
            .map_err(|e| format!("Failed to serialize config to TOML: {}", e))?;
        fs::write(path, toml_str)
            .map_err(|e| format!("Failed to write config file to {:?}: {}", path, e))?;

        #[cfg(unix)]
        {
            if let Ok(metadata) = fs::metadata(path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o600);
                if let Err(e) = fs::set_permissions(path, perms) {
                    crate::log_warn!("Failed to set 0600 permissions on config file {:?}: {}", path, e);
                }
            }
        }

        let has_token = config_to_write.token.as_ref().map(|t| !t.trim().is_empty()).unwrap_or(false);
        crate::log_info!(
            "Saving config to {:?}: has_token={}, last_in_dm={}, last_server_id={:?}, last_channel_id={:?}",
            path,
            has_token,
            config_to_write.last_in_dm,
            config_to_write.last_server_id,
            config_to_write.last_channel_id
        );

        Ok(())
    }
}

/// Lightweight persistent navigation cache (`~/.cache/stoat-tui/nav_cache.json`)
/// for instant sidebar/header hydration on frame 1 before network I/O completes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NavCache {
    #[serde(default)]
    pub api_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ws_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autumn_url: Option<String>,
    #[serde(default)]
    pub servers: Vec<crate::api::models::Server>,
    #[serde(default)]
    pub channels: Vec<crate::api::models::Channel>,
    #[serde(default)]
    pub dms: Vec<crate::api::models::Channel>,
    #[serde(default)]
    pub users: Vec<crate::api::models::User>,
    #[serde(default)]
    pub server_accounts: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub channel_accounts: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub user_accounts: std::collections::HashMap<String, String>,
}

impl NavCache {
    pub fn cache_path() -> PathBuf {
        if let Ok(env_dir) = std::env::var("STOAT_TUI_CONFIG_DIR") {
            let trimmed = env_dir.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join("nav_cache.json");
            }
        }

        #[cfg(test)]
        {
            return std::env::temp_dir()
                .join(format!("stoat-tui-unit-test-{}", std::process::id()))
                .join("nav_cache.json");
        }

        #[cfg(not(test))]
        {
            if let Some(proj_dirs) = ProjectDirs::from("chat", "stoat", "stoat-tui") {
                proj_dirs.cache_dir().join("nav_cache.json")
            } else {
                PathBuf::from(".stoat-tui").join("nav_cache.json")
            }
        }
    }

    pub fn load() -> Option<Self> {
        let path = Self::cache_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        let contents = fs::read_to_string(path).ok()?;
        serde_json::from_str::<NavCache>(&contents).ok()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::cache_path();
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        let json = serde_json::to_string(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn clear() {
        let path = Self::cache_path();
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfg_test_uses_temp_dir_config_path() {
        let path = Config::config_path();
        let temp_root = std::env::temp_dir();
        assert!(
            path.starts_with(&temp_root),
            "Unit test config path {:?} must be under temp dir {:?}",
            path,
            temp_root
        );
    }
}
