use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

use crate::stt::SttProvider;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyCombo {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub win: bool,
    #[serde(default)]
    pub key: u16,
}

impl Default for KeyCombo {
    fn default() -> Self {
        Self::single(0x77) // F8
    }
}

impl KeyCombo {
    pub const fn single(key: u16) -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key,
        }
    }

    pub const fn combo(ctrl: bool, alt: bool, shift: bool, win: bool, key: u16) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            win,
            key,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.key == 0 && !self.ctrl && !self.alt && !self.shift && !self.win
    }

    pub fn has_modifiers(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.win
    }

    pub fn to_u32(&self) -> u32 {
        let mut val = self.key as u32;
        if self.ctrl {
            val |= 1 << 16;
        }
        if self.alt {
            val |= 1 << 17;
        }
        if self.shift {
            val |= 1 << 18;
        }
        if self.win {
            val |= 1 << 19;
        }
        val
    }

    pub fn from_u32(val: u32) -> Self {
        Self {
            key: (val & 0xFFFF) as u16,
            ctrl: (val & (1 << 16)) != 0,
            alt: (val & (1 << 17)) != 0,
            shift: (val & (1 << 18)) != 0,
            win: (val & (1 << 19)) != 0,
        }
    }

    pub fn format_display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.win {
            parts.push("Win".to_string());
        }
        if self.key != 0 {
            // Check if key is modifier itself and already printed
            let is_mod_dup = (self.ctrl && matches!(self.key, 0x11 | 0xA2 | 0xA3))
                || (self.alt && matches!(self.key, 0x12 | 0xA4 | 0xA5))
                || (self.shift && matches!(self.key, 0x10 | 0xA0 | 0xA1))
                || (self.win && matches!(self.key, 0x5B | 0x5C));
            if !is_mod_dup {
                let name = crate::platform::vk_to_string(self.key);
                parts.push(name);
            }
        }
        if parts.is_empty() {
            "None".to_string()
        } else {
            parts.join(" + ")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub endpoint_url: String,
    pub user_id: String,
    #[serde(default = "default_hotkey_combo")]
    pub hotkey: KeyCombo,
    #[serde(default = "default_toggle_combo")]
    pub toggle_window: KeyCombo,
    #[serde(default = "default_hotkey_vk")]
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
    #[serde(default = "default_ai_formatting")]
    pub ai_formatting: bool,
    #[serde(default = "default_writing_style")]
    pub writing_style: String,
    #[serde(default = "default_sound_effects")]
    pub sound_effects: bool,
    #[serde(default)]
    pub input_device_name: Option<String>,
    #[serde(default = "default_recording_trigger_mode")]
    pub recording_trigger_mode: String,
    pub language_code: String,
    pub auto_paste: bool,
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default)]
    pub auth_token: Option<String>,
}

fn default_hotkey_combo() -> KeyCombo {
    KeyCombo::single(0x77) // F8
}

fn default_toggle_combo() -> KeyCombo {
    KeyCombo::single(0x78) // F9
}

fn default_hotkey_vk() -> u16 {
    0x77 // F8
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

fn default_ai_formatting() -> bool {
    true
}

fn default_writing_style() -> String {
    "smart_cleanup".to_string()
}

fn default_sound_effects() -> bool {
    true
}

fn default_recording_trigger_mode() -> String {
    "push_to_talk".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        let (token, uid) = Self::try_load_wispr_session().unwrap_or((None, uuid::Uuid::new_v4().to_string()));
        Self {
            endpoint_url: "https://inference.wisprflow.com:443".to_string(),
            user_id: uid,
            hotkey: default_hotkey_combo(),
            toggle_window: default_toggle_combo(),
            hotkey_vk: 0x77, // F8
            toggle_window_vk: 0x78, // F9
            stt_provider: SttProvider::WisprFlow,
            groq_api_key: None,
            groq_model: default_groq_model(),
            elevenlabs_api_key: None,
            elevenlabs_model: default_elevenlabs_model(),
            show_floating_hud: true,
            ai_formatting: true,
            writing_style: default_writing_style(),
            sound_effects: true,
            input_device_name: None,
            recording_trigger_mode: default_recording_trigger_mode(),
            language_code: "auto".to_string(),
            auto_paste: true,
            auto_start: false,
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

                    // Migrate legacy single vk to KeyCombo if needed
                    if config.hotkey.is_empty() && config.hotkey_vk != 0 {
                        config.hotkey = KeyCombo::single(config.hotkey_vk);
                        modified = true;
                    }
                    if config.toggle_window.is_empty() && config.toggle_window_vk != 0 {
                        config.toggle_window = KeyCombo::single(config.toggle_window_vk);
                        modified = true;
                    }
                    // Sync legacy fields
                    config.hotkey_vk = config.hotkey.key;
                    config.toggle_window_vk = config.toggle_window.key;

                    if modified {
                        let _ = config.save(Some(&path));
                    }

                    config.auto_start = crate::platform::PlatformManager::is_autostart_enabled();
                    info!("Loaded configuration from {:?} (Endpoint: {}, Autostart: {})", path, config.endpoint_url, config.auto_start);
                    return config;
                }
            }
        }

        let mut default_config = Self::default();
        default_config.auto_start = crate::platform::PlatformManager::is_autostart_enabled();
        if let Ok(json) = serde_json::to_string_pretty(&default_config) {
            let _ = std::fs::write(&path, json);
        }

        default_config
    }

    pub fn save(&self, custom_path: Option<&Path>) -> anyhow::Result<()> {
        let mut clone = self.clone();
        clone.hotkey_vk = clone.hotkey.key;
        clone.toggle_window_vk = clone.toggle_window.key;
        let path = match custom_path {
            Some(p) => p.to_path_buf(),
            None => Self::default_path(),
        };
        let json = serde_json::to_string_pretty(&clone)?;
        std::fs::write(&path, json)?;
        info!("Saved configuration to {:?}", path);
        Ok(())
    }
}
