use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

use crate::stt::SttProvider;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub endpoint_url: String,
    pub user_id: String,
    pub hotkey_vk: u16,
    #[serde(default = "default_toggle_window_vk")]
    pub toggle_window_vk: u16,
    #[serde(default)]
    pub stt_provider: SttProvider,
    #[serde(default)]
    pub groq_api_key: Option<String>,
    #[serde(default = "default_groq_model")]
    pub groq_model: String,
    #[serde(default)]
    pub elevenlabs_api_key: Option<String>,
    #[serde(default = "default_elevenlabs_model")]
    pub elevenlabs_model: String,
    #[serde(default = "default_show_floating_hud")]
    pub show_floating_hud: bool,
    pub writing_style: String,
    pub language_code: String,
    pub auto_paste: bool,
    #[serde(default)]
    pub auth_token: Option<String>,
}

fn default_toggle_window_vk() -> u16 {
    0x78 // F9
}

fn default_groq_model() -> String {
    "whisper-large-v3-turbo".to_string()
}

fn default_elevenlabs_model() -> String {
    "scribe_v1".to_string()
}

fn default_show_floating_hud() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        let (token, uid) = Self::try_load_wispr_session().unwrap_or((None, uuid::Uuid::new_v4().to_string()));
        Self {
            endpoint_url: "https://inference.wisprflow.com:443".to_string(),
            user_id: uid,
            hotkey_vk: 0x77, // F8
            toggle_window_vk: 0x78, // F9
            stt_provider: SttProvider::WisprFlow,
            groq_api_key: None,
            groq_model: default_groq_model(),
            elevenlabs_api_key: None,
            elevenlabs_model: default_elevenlabs_model(),
            show_floating_hud: true,
            writing_style: "default".to_string(),
            language_code: "auto".to_string(),
            auto_paste: true,
            auth_token: token,
        }
    }
}

impl AppConfig {
    pub fn default_path() -> PathBuf {
        let app_data = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(app_data).join("Wispr Flow").join("flow-rust.json")
    }

    /// Automatically scans and extracts logged-in Wispr Flow JWT token from session.json
    pub fn try_load_wispr_session() -> Option<(Option<String>, String)> {
        let app_data = std::env::var("APPDATA").ok()?;
        let session_path = PathBuf::from(app_data).join("Wispr Flow").join("session.json");
        if !session_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&session_path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                if k.ends_with("-auth-token") {
                    if let Some(token_str) = val.as_str() {
                        if let Ok(token_obj) = serde_json::from_str::<serde_json::Value>(token_str) {
                            let access_token = token_obj.get("access_token")?.as_str()?.to_string();
                            let user_id = token_obj.get("user")?.get("id")?.as_str()?.to_string();
                            info!("Auto-detected authenticated Wispr Flow session for User ID: {}", user_id);
                            return Some((Some(access_token), user_id));
                        }
                    }
                }
            }
        }
        None
    }

    pub fn load_or_default(custom_path: Option<&Path>) -> Self {
        let path = match custom_path {
            Some(p) => p.to_path_buf(),
            None => Self::default_path(),
        };

        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(mut config) = serde_json::from_str::<AppConfig>(&content) {
                    let mut modified = false;

                    // Migrate old dogfood URL to production endpoint
                    if config.endpoint_url.contains("modal.direct") {
                        config.endpoint_url = "https://inference.wisprflow.com:443".to_string();
                        modified = true;
                    }

                    // Auto-fill token from session.json if missing
                    if config.auth_token.is_none() {
                        if let Some((tok, uid)) = Self::try_load_wispr_session() {
                            config.auth_token = tok;
                            config.user_id = uid;
                            modified = true;
                        }
                    }

                    if modified {
                        let _ = config.save(Some(&path));
                    }

                    info!("Loaded configuration from {:?} (Endpoint: {})", path, config.endpoint_url);
                    return config;
                }
            }
        }

        let default_config = Self::default();
        if let Ok(json) = serde_json::to_string_pretty(&default_config) {
            let _ = std::fs::write(&path, json);
        }

        default_config
    }

    pub fn save(&self, custom_path: Option<&Path>) -> anyhow::Result<()> {
        let path = match custom_path {
            Some(p) => p.to_path_buf(),
            None => Self::default_path(),
        };
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        info!("Saved configuration to {:?}", path);
        Ok(())
    }
}
