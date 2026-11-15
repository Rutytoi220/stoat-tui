use anyhow::{Context, Result};
use directories::ProjectDirs;
use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const DEFAULT_STOAT_API: &str = "https://api.stoat.chat";
pub const DEFAULT_REVOLT_API: &str = "https://api.revolt.chat";

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
    pub theme: ThemeConfig,
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
            theme: ThemeConfig::default(),
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

    pub fn config_dir() -> PathBuf {
        if let Some(proj_dirs) = ProjectDirs::from("chat", "stoat", "stoat-tui") {
            proj_dirs.config_dir().to_path_buf()
        } else {
            PathBuf::from(".stoat-tui")
        }
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config file at {:?}", path))?;
            let config: Config = toml::from_str(&contents)
                .with_context(|| format!("Failed to parse config file at {:?}", path))?;
            Ok(config)
        } else {
            let default_config = Config::default();
            // Try creating the directory and file
            let _ = Self::save_config(&default_config);
            Ok(default_config)
        }
    }

    pub fn save(&self) -> Result<()> {
        Self::save_config(self)
    }

    fn save_config(config: &Config) -> Result<()> {
        let dir = Self::config_dir();
        fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create config directory {:?}", dir))?;
        let path = Self::config_path();
        let toml_str = toml::to_string_pretty(config)
            .context("Failed to serialize config to TOML")?;
        fs::write(&path, toml_str)
            .with_context(|| format!("Failed to write config file to {:?}", path))?;

        #[cfg(unix)]
        {
            if let Ok(metadata) = fs::metadata(&path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o600);
                if let Err(e) = fs::set_permissions(&path, perms) {
                    crate::log_warn!("Failed to set 0600 permissions on config file {:?}: {}", path, e);
                }
            }
        }

        let has_token = config.token.as_ref().map(|t| !t.trim().is_empty()).unwrap_or(false);
        crate::log_info!("Saved config to {:?} (token.is_some()={})", path, has_token);

        Ok(())
    }
}
