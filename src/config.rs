use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

use crate::secrets::{env_non_empty, Secrets};
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

    #[allow(clippy::wrong_self_convention)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Secret: persisted in `secrets.json`, never in `config.json`.
    #[serde(default, skip_serializing)]
    pub groq_api_key: Option<String>,
    #[serde(default = "default_groq_model")]
    pub groq_model: String,
    /// Secret: persisted in `secrets.json`, never in `config.json`.
    #[serde(default, skip_serializing)]
    pub elevenlabs_api_key: Option<String>,
    #[serde(default = "default_elevenlabs_model")]
    pub elevenlabs_model: String,
    /// Secret: persisted in `secrets.json`, never in `config.json`.
    #[serde(default, skip_serializing)]
    pub baseten_api_key: Option<String>,
    #[serde(default = "default_baseten_model")]
    pub baseten_model_id: String,
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
    /// Secret: persisted in `secrets.json`, never in `config.json`.
    #[serde(default, skip_serializing)]
    pub auth_token: Option<String>,
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    /// Send surrounding text / active app context to the STT cloud provider.
    #[serde(default = "default_true")]
    pub share_context: bool,
    /// Keep the contents of the focused text box in local history.
    #[serde(default)]
    pub store_textbox_contents: bool,
    /// Automatically look for new releases.
    #[serde(default = "default_true")]
    pub check_updates: bool,
    #[serde(default)]
    pub include_prerelease: bool,
    #[serde(default = "default_update_interval")]
    pub update_check_interval_hours: u64,
    #[serde(default)]
    pub last_update_check: Option<String>,
    #[serde(default)]
    pub skipped_update_version: Option<String>,
    /// Hard safety limit so a forgotten hotkey cannot fill the disk/RAM.
    #[serde(default = "default_max_recording_secs")]
    pub max_recording_secs: u64,
    /// Network timeout for speech-to-text requests, in seconds.
    #[serde(default = "default_stt_timeout_secs")]
    pub stt_timeout_secs: u64,
    /// Filter out silence and background noise while dictating.
    #[serde(default = "default_true")]
    pub silence_filter: bool,
    /// Extra head-room above the measured noise floor (0.0 - 1.0). Lower
    /// values are more sensitive.
    #[serde(default = "default_silence_sensitivity")]
    pub silence_sensitivity: f32,
    /// Stop recording after this much continuous silence (0 = never).
    #[serde(default = "default_silence_stop_secs")]
    pub silence_stop_secs: u64,
    /// Reject transcripts that look like model hallucinations.
    #[serde(default = "default_true")]
    pub hallucination_guard: bool,
    /// Correct spelling with the offline dictionary.
    #[serde(default = "default_true")]
    pub autocorrect_enabled: bool,
    /// Also correct spelling with the language model.
    #[serde(default = "default_true")]
    pub autocorrect_use_llm: bool,
    /// Comma separated language codes to correct ("th,en"). Empty = auto.
    #[serde(default)]
    pub autocorrect_languages: String,
    /// Insert the missing space between Thai and Latin words.
    #[serde(default = "default_true")]
    pub mixed_spacing: bool,
    /// Client name announced to the gRPC backend.
    #[serde(default = "default_client_name")]
    pub client_name: String,
    /// Protocol compatibility version announced to the gRPC backend.
    #[serde(default = "default_client_version")]
    pub client_version: String,
}

fn default_true() -> bool {
    true
}
fn default_update_interval() -> u64 {
    24
}
fn default_max_recording_secs() -> u64 {
    300
}
fn default_silence_sensitivity() -> f32 {
    0.5
}
fn default_silence_stop_secs() -> u64 {
    0
}
fn default_stt_timeout_secs() -> u64 {
    30
}
fn default_client_name() -> String {
    "Wflow".to_string()
}
fn default_client_version() -> String {
    "1.6.1147".to_string()
}

fn default_close_to_tray() -> bool {
    cfg!(target_os = "windows")
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

fn default_baseten_model() -> String {
    "model-v31pl413".to_string()
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
        let (token, uid) =
            Self::try_load_wispr_session().unwrap_or((None, uuid::Uuid::new_v4().to_string()));
        Self {
            endpoint_url: "https://inference.wisprflow.com:443".to_string(),
            user_id: uid,
            hotkey: default_hotkey_combo(),
            toggle_window: default_toggle_combo(),
            hotkey_vk: 0x77,        // F8
            toggle_window_vk: 0x78, // F9
            stt_provider: SttProvider::WisprFlow,
            groq_api_key: None,
            groq_model: default_groq_model(),
            elevenlabs_api_key: None,
            elevenlabs_model: default_elevenlabs_model(),
            baseten_api_key: None,
            baseten_model_id: default_baseten_model(),
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
            close_to_tray: default_close_to_tray(),
            share_context: true,
            store_textbox_contents: false,
            check_updates: true,
            include_prerelease: false,
            update_check_interval_hours: default_update_interval(),
            last_update_check: None,
            skipped_update_version: None,
            max_recording_secs: default_max_recording_secs(),
            stt_timeout_secs: default_stt_timeout_secs(),
            silence_filter: true,
            silence_sensitivity: default_silence_sensitivity(),
            silence_stop_secs: default_silence_stop_secs(),
            hallucination_guard: true,
            autocorrect_enabled: true,
            autocorrect_use_llm: true,
            autocorrect_languages: String::new(),
            mixed_spacing: true,
            client_name: default_client_name(),
            client_version: default_client_version(),
        }
    }
}

impl AppConfig {
    /// New canonical location, e.g. `%APPDATA%\Wflow\config.json` or
    /// `~/.config/wflow/config.json`.
    pub fn default_path() -> PathBuf {
        crate::paths::ensure_dir(crate::paths::config_dir()).join("config.json")
    }

    /// Previous locations that are migrated on first launch.
    fn legacy_paths() -> Vec<PathBuf> {
        #[allow(unused_mut)]
        let mut paths = vec![
            PathBuf::from("config.json"),
            PathBuf::from("Wispr Flow").join("flow-rust.json"),
        ];
        #[cfg(target_os = "windows")]
        {
            if let Ok(app_data) = std::env::var("APPDATA") {
                paths.insert(
                    0,
                    PathBuf::from(app_data)
                        .join("Wispr Flow")
                        .join("flow-rust.json"),
                );
            }
        }
        paths
    }

    /// Automatically scans and extracts a logged-in Wispr Flow JWT token from
    /// the official client's `session.json`.
    pub fn try_load_wispr_session() -> Option<(Option<String>, String)> {
        let app_data = std::env::var("APPDATA").ok()?;
        let session_path = PathBuf::from(app_data)
            .join("Wispr Flow")
            .join("session.json");
        if !session_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&session_path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                if k.ends_with("-auth-token") {
                    if let Some(token_str) = val.as_str() {
                        if let Ok(token_obj) = serde_json::from_str::<serde_json::Value>(token_str)
                        {
                            let access_token = token_obj.get("access_token")?.as_str()?.to_string();
                            let user_id = token_obj.get("user")?.get("id")?.as_str()?.to_string();
                            info!(
                                "Auto-detected authenticated Wispr Flow session for User ID: {}",
                                user_id
                            );
                            return Some((Some(access_token), user_id));
                        }
                    }
                }
            }
        }
        None
    }

    /// Copies secrets from the in-memory config into a `Secrets` value.
    pub fn secrets(&self) -> Secrets {
        Secrets {
            auth_token: self.auth_token.clone(),
            groq_api_key: self.groq_api_key.clone(),
            elevenlabs_api_key: self.elevenlabs_api_key.clone(),
            baseten_api_key: self.baseten_api_key.clone(),
        }
    }

    /// Fills any unset secret from the secrets file and then from environment
    /// variables (environment wins).
    fn merge_secrets(&mut self, config_path: &Path) -> bool {
        let stored = Secrets::load(&Secrets::path_for(config_path));
        let mut changed = false;

        let mut apply = |field: &mut Option<String>, value: Option<String>| {
            if field.as_deref().map(str::trim).unwrap_or("").is_empty() {
                if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
                    *field = Some(value);
                    changed = true;
                }
            }
        };

        apply(&mut self.auth_token, stored.auth_token);
        apply(&mut self.groq_api_key, stored.groq_api_key);
        apply(&mut self.elevenlabs_api_key, stored.elevenlabs_api_key);
        apply(&mut self.baseten_api_key, stored.baseten_api_key);

        // Environment overrides are useful for CI, containers and power users.
        apply(&mut self.auth_token, env_non_empty("WFLOW_AUTH_TOKEN"));
        apply(&mut self.groq_api_key, env_non_empty("WFLOW_GROQ_API_KEY"));
        apply(
            &mut self.elevenlabs_api_key,
            env_non_empty("WFLOW_ELEVENLABS_API_KEY"),
        );
        apply(
            &mut self.baseten_api_key,
            env_non_empty("WFLOW_BASETEN_API_KEY"),
        );

        changed
    }

    pub fn load_or_default(custom_path: Option<&Path>) -> Self {
        let default_path = Self::default_path();

        let path = match custom_path {
            Some(p) => p.to_path_buf(),
            None => {
                if default_path.exists() {
                    default_path.clone()
                } else {
                    Self::legacy_paths()
                        .into_iter()
                        .find(|candidate| candidate.exists())
                        .unwrap_or(default_path.clone())
                }
            }
        };

        let mut needs_save = false;

        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let raw: Option<serde_json::Value> = serde_json::from_str(&content).ok();
                let had_inline_secrets = raw
                    .as_ref()
                    .and_then(|v| v.as_object())
                    .map(|obj| {
                        [
                            "auth_token",
                            "groq_api_key",
                            "elevenlabs_api_key",
                            "baseten_api_key",
                        ]
                        .iter()
                        .any(|key| obj.get(*key).map(|v| !v.is_null()).unwrap_or(false))
                    })
                    .unwrap_or(false);

                if let Ok(mut config) = serde_json::from_str::<AppConfig>(&content) {
                    let mut modified = had_inline_secrets;

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

                    modified |= config.merge_secrets(&path);

                    if modified {
                        needs_save = true;
                    }

                    config.auto_start = crate::platform::PlatformManager::is_autostart_enabled();
                    info!(
                        "Loaded configuration from {:?} (Endpoint: {}, Autostart: {}, Secrets: {})",
                        path,
                        config.endpoint_url,
                        config.auto_start,
                        !config.secrets().is_empty()
                    );

                    // Relocate legacy files into the canonical directory.
                    if custom_path.is_none() && path != default_path {
                        needs_save = true;
                    }
                    if needs_save {
                        let target = custom_path.or(Some(default_path.as_path()));
                        if let Err(err) = config.save(target) {
                            tracing::warn!("Could not persist migrated configuration: {}", err);
                        } else if custom_path.is_none() && path != default_path {
                            info!("Migrated configuration to {:?}", default_path);
                        }
                    }
                    return config;
                }
            }
        }

        let mut default_config = Self {
            auto_start: crate::platform::PlatformManager::is_autostart_enabled(),
            ..Self::default()
        };
        default_config.merge_secrets(&path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(err) = default_config.save(custom_path) {
            tracing::warn!("Could not write default configuration: {}", err);
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
        if let Some(parent) = path.parent() {
            crate::paths::ensure_dir(parent.to_path_buf());
        }

        // Secrets go to a separate, permission-restricted file.
        let secrets = clone.secrets();
        if !secrets.is_empty() {
            secrets.save(&Secrets::path_for(&path))?;
        }

        let json = serde_json::to_string_pretty(&clone)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        info!("Saved configuration to {:?}", path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_combo_display_and_packing() {
        let f8 = KeyCombo::single(0x77);
        assert_eq!(f8.format_display(), "F8");
        assert!(!f8.has_modifiers());
        assert_eq!(KeyCombo::from_u32(f8.to_u32()), f8);

        let ctrl_space = KeyCombo::combo(true, false, false, false, 0x20);
        assert_eq!(ctrl_space.format_display(), "Ctrl + Space");
        assert!(ctrl_space.has_modifiers());
        assert_eq!(KeyCombo::from_u32(ctrl_space.to_u32()), ctrl_space);

        let alt_space = KeyCombo::combo(false, true, false, false, 0x20);
        assert_eq!(alt_space.format_display(), "Alt + Space");
        assert_eq!(KeyCombo::from_u32(alt_space.to_u32()), alt_space);

        let ctrl_shift_d = KeyCombo::combo(true, false, true, false, 0x44);
        assert_eq!(ctrl_shift_d.format_display(), "Ctrl + Shift + D");
        assert_eq!(KeyCombo::from_u32(ctrl_shift_d.to_u32()), ctrl_shift_d);

        let win_h = KeyCombo::combo(false, false, false, true, 0x48);
        assert_eq!(win_h.format_display(), "Win + H");
        assert_eq!(KeyCombo::from_u32(win_h.to_u32()), win_h);
    }

    #[test]
    fn secrets_never_reach_config_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let config = AppConfig {
            groq_api_key: Some("gsk_secret".into()),
            auth_token: Some("jwt_secret".into()),
            ..AppConfig::default()
        };
        config.save(Some(&path)).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("gsk_secret"));
        assert!(!raw.contains("jwt_secret"));

        let secrets = Secrets::load(&Secrets::path_for(&path));
        assert_eq!(secrets.groq_api_key.as_deref(), Some("gsk_secret"));
        assert_eq!(secrets.auth_token.as_deref(), Some("jwt_secret"));
    }

    #[test]
    fn inline_secrets_are_migrated_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let config = AppConfig::default();
        let mut value = serde_json::to_value(config).unwrap();
        value["groq_api_key"] = serde_json::json!("gsk_legacy");
        value["auth_token"] = serde_json::json!("jwt_legacy");
        std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = AppConfig::load_or_default(Some(&path));
        assert_eq!(loaded.groq_api_key.as_deref(), Some("gsk_legacy"));

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("gsk_legacy"), "legacy secret must be removed");
        let secrets = Secrets::load(&Secrets::path_for(&path));
        assert_eq!(secrets.groq_api_key.as_deref(), Some("gsk_legacy"));
    }
}
