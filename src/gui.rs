use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use tokio::sync::mpsc;

use crate::config::{AppConfig, KeyCombo};
use crate::db::{DictionaryEntry, HistoryRecord};
use crate::platform::{ActiveAppContext, PlatformManager};
use crate::stt::SttProvider;
use crate::thai::shape_thai;
use crate::ui::DictationStatus;
use crate::update::UpdateInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Studio,
    History,
    Dictionary,
    Updates,
    Settings,
}

#[derive(Debug, Clone)]
pub enum GuiEvent {
    StartDictation,
    StopDictation,
    RefreshHistory,
    ClearHistory,
    SaveConfig(Box<AppConfig>),
    DeleteHistory(String),
    AddDictionaryEntry {
        word: String,
        replacement: Option<String>,
    },
    DeleteDictionaryEntry(String),
    RefreshDictionary,
    CheckUpdate,
    SkipUpdateVersion(String),
    OpenUrl(String),
    OpenLogFolder,
}

/// Locks a mutex, recovering from poisoning instead of panicking the GUI.
///
/// A panic in any worker thread would otherwise poison the shared state and
/// take the whole interface down.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

/// Shared state between the async dictation engine and the GUI
#[derive(Clone)]
pub struct AppState {
    pub is_recording: Arc<AtomicBool>,
    pub current_amplitude: Arc<Mutex<f32>>,
    pub status: Arc<Mutex<DictationStatus>>,
    pub last_transcript: Arc<Mutex<String>>,
    pub last_latency: Arc<Mutex<f64>>,
    pub active_app: Arc<Mutex<ActiveAppContext>>,
    pub config: Arc<Mutex<AppConfig>>,
    pub history: Arc<Mutex<Vec<HistoryRecord>>>,
    pub dictionary_entries: Arc<Mutex<Vec<DictionaryEntry>>>,
    pub toast: Arc<Mutex<Option<(String, Instant)>>>,
    pub window_visible: Arc<AtomicBool>,
    #[allow(dead_code)]
    pub pending_toggle_window: Arc<AtomicBool>,
    pub pending_quit: Arc<AtomicBool>,
    pub egui_ctx: Arc<Mutex<Option<egui::Context>>>,
    pub recorded_key: Arc<Mutex<Option<(u8, KeyCombo)>>>,
    pub update_info: Arc<Mutex<Option<UpdateInfo>>>,
    pub update_error: Arc<Mutex<Option<String>>>,
    pub update_check_running: Arc<AtomicBool>,
    /// The active noise gate, so the studio can show the voice activity.
    pub voice_gate: Arc<Mutex<Option<Arc<Mutex<crate::vad::VoiceGate>>>>>,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        initial_history: Vec<HistoryRecord>,
        initial_dict: Vec<DictionaryEntry>,
    ) -> Self {
        Self {
            is_recording: Arc::new(AtomicBool::new(false)),
            current_amplitude: Arc::new(Mutex::new(0.0)),
            status: Arc::new(Mutex::new(DictationStatus::Idle)),
            last_transcript: Arc::new(Mutex::new(String::new())),
            last_latency: Arc::new(Mutex::new(0.0)),
            active_app: Arc::new(Mutex::new(ActiveAppContext::default())),
            config: Arc::new(Mutex::new(config)),
            history: Arc::new(Mutex::new(initial_history)),
            dictionary_entries: Arc::new(Mutex::new(initial_dict)),
            toast: Arc::new(Mutex::new(None)),
            window_visible: Arc::new(AtomicBool::new(true)),
            pending_toggle_window: Arc::new(AtomicBool::new(false)),
            pending_quit: Arc::new(AtomicBool::new(false)),
            egui_ctx: Arc::new(Mutex::new(None)),
            recorded_key: Arc::new(Mutex::new(None)),
            update_info: Arc::new(Mutex::new(None)),
            update_error: Arc::new(Mutex::new(None)),
            update_check_running: Arc::new(AtomicBool::new(false)),
            voice_gate: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_toast(&self, message: &str) {
        if let Ok(mut lock) = self.toast.lock() {
            *lock = Some((message.to_string(), Instant::now()));
        }
    }
}

pub struct WisprFlowGui {
    state: AppState,
    event_tx: mpsc::UnboundedSender<GuiEvent>,
    active_tab: ActiveTab,
    search_query: String,
    // Settings editable fields
    settings_endpoint: String,
    settings_token: String,
    settings_language: String,
    settings_hotkey: KeyCombo,
    settings_toggle: KeyCombo,
    recording_mode: Option<u8>,
    settings_auto_paste: bool,
    settings_auto_start: bool,
    settings_close_to_tray: bool,
    settings_stt_provider: SttProvider,
    settings_groq_api_key: String,
    settings_groq_model: String,
    settings_elevenlabs_api_key: String,
    settings_elevenlabs_model: String,
    settings_show_floating_hud: bool,
    settings_baseten_api_key: String,
    settings_baseten_model: String,
    // Privacy & maintenance
    settings_share_context: bool,
    settings_store_textbox_contents: bool,
    settings_max_recording_secs: u64,
    settings_stt_timeout_secs: u64,
    // Voice filter & text quality
    settings_silence_filter: bool,
    settings_silence_sensitivity: f32,
    settings_silence_stop_secs: u64,
    settings_hallucination_guard: bool,
    settings_autocorrect_enabled: bool,
    settings_autocorrect_use_llm: bool,
    settings_autocorrect_languages: String,
    settings_mixed_spacing: bool,
    clear_history_armed: bool,
    // Update checker
    settings_check_updates: bool,
    settings_include_prerelease: bool,
    settings_update_interval_hours: u64,
    // Features 1, 2, 3, 5: AI Formatting, Writing Style, Sound Effects, Mic, Trigger Mode
    settings_ai_formatting: bool,
    settings_writing_style: String,
    settings_sound_effects: bool,
    settings_input_device_name: Option<String>,
    settings_recording_trigger_mode: String,
    available_devices: Vec<String>,
    // Feature 6: Dictionary & Snippets UI inputs
    dict_word_input: String,
    dict_replacement_input: String,
    dict_search_query: String,
    // Live animations & metrics
    animation_time: f32,
    recording_start: Option<Instant>,
    fonts_initialized: bool,
    logo_texture: Option<egui::TextureHandle>,
    #[allow(dead_code)]
    window_icon_applied: bool,
    show_exit_confirmation: bool,
    stt_test_status: Arc<Mutex<Option<Result<String, String>>>>,
    /// Last update payload the GUI has rendered, used to detect changes.
    last_seen_update: Option<UpdateInfo>,
    stt_test_in_progress: Arc<AtomicBool>,
}

pub const POPULAR_HOTKEYS: &[(&str, KeyCombo)] = &[
    ("F8 (Wflow Default)", KeyCombo::single(0x77)),
    ("F9 (Toggle Default)", KeyCombo::single(0x78)),
    (
        "Ctrl + Space",
        KeyCombo::combo(true, false, false, false, 0x20),
    ),
    (
        "Alt + Space",
        KeyCombo::combo(false, true, false, false, 0x20),
    ),
    (
        "Ctrl + Shift + D",
        KeyCombo::combo(true, false, true, false, 0x44),
    ),
    ("Win + H", KeyCombo::combo(false, false, false, true, 0x48)),
    (
        "Ctrl + Alt + V",
        KeyCombo::combo(true, true, false, false, 0x56),
    ),
    ("Right Control", KeyCombo::single(0xA3)),
    ("Left Control", KeyCombo::single(0xA2)),
    ("Right Alt", KeyCombo::single(0xA5)),
    ("Left Alt", KeyCombo::single(0xA4)),
    ("Caps Lock", KeyCombo::single(0x14)),
    ("F1", KeyCombo::single(0x70)),
    ("F2", KeyCombo::single(0x71)),
    ("F3", KeyCombo::single(0x72)),
    ("F4", KeyCombo::single(0x73)),
    ("F5", KeyCombo::single(0x74)),
    ("F6", KeyCombo::single(0x75)),
    ("F7", KeyCombo::single(0x76)),
    ("F8", KeyCombo::single(0x77)),
    ("F9", KeyCombo::single(0x78)),
    ("F10", KeyCombo::single(0x79)),
    ("F11", KeyCombo::single(0x7A)),
    ("F12", KeyCombo::single(0x7B)),
    ("Scroll Lock", KeyCombo::single(0x91)),
    ("Pause / Break", KeyCombo::single(0x13)),
    ("Insert", KeyCombo::single(0x2D)),
    ("Tilde (~)", KeyCombo::single(0xC0)),
];

const LANGUAGE_OPTIONS: &[(&str, &str)] = &[
    ("Auto-detect", "auto"),
    ("English (US)", "en"),
    ("Thai (ภาษาไทย)", "th"),
    ("Japanese (日本語)", "ja"),
    ("German (Deutsch)", "de"),
    ("Spanish (Español)", "es"),
    ("French (Français)", "fr"),
];

impl WisprFlowGui {
    pub fn new(state: AppState, event_tx: mpsc::UnboundedSender<GuiEvent>) -> Self {
        let (
            endpoint,
            token,
            language,
            hotkey,
            toggle,
            auto_paste,
            auto_start,
            stt_provider,
            groq_key,
            groq_model,
            elevenlabs_key,
            elevenlabs_model,
            show_hud,
            ai_formatting,
            writing_style,
            sound_effects,
            input_device,
            trigger_mode,
            close_to_tray,
            baseten_key,
            baseten_model,
            share_context,
            store_textbox_contents,
            max_recording_secs,
            stt_timeout_secs,
            silence_filter,
            silence_sensitivity,
            silence_stop_secs,
            hallucination_guard,
            autocorrect_enabled,
            autocorrect_use_llm,
            autocorrect_languages,
            mixed_spacing,
            check_updates,
            include_prerelease,
            update_interval_hours,
        ) = {
            let cfg = lock(&state.config);
            (
                cfg.endpoint_url.clone(),
                cfg.auth_token.clone().unwrap_or_default(),
                cfg.language_code.clone(),
                cfg.hotkey,
                cfg.toggle_window,
                cfg.auto_paste,
                cfg.auto_start,
                cfg.stt_provider,
                cfg.groq_api_key.clone().unwrap_or_default(),
                cfg.groq_model.clone(),
                cfg.elevenlabs_api_key.clone().unwrap_or_default(),
                cfg.elevenlabs_model.clone(),
                cfg.show_floating_hud,
                cfg.ai_formatting,
                cfg.writing_style.clone(),
                cfg.sound_effects,
                cfg.input_device_name.clone(),
                cfg.recording_trigger_mode.clone(),
                cfg.close_to_tray,
                cfg.baseten_api_key.clone().unwrap_or_default(),
                cfg.baseten_model_id.clone(),
                cfg.share_context,
                cfg.store_textbox_contents,
                cfg.max_recording_secs,
                cfg.stt_timeout_secs,
                cfg.silence_filter,
                cfg.silence_sensitivity,
                cfg.silence_stop_secs,
                cfg.hallucination_guard,
                cfg.autocorrect_enabled,
                cfg.autocorrect_use_llm,
                cfg.autocorrect_languages.clone(),
                cfg.mixed_spacing,
                cfg.check_updates,
                cfg.include_prerelease,
                cfg.update_check_interval_hours,
            )
        };

        Self {
            state,
            event_tx,
            active_tab: ActiveTab::Studio,
            search_query: String::new(),
            settings_endpoint: endpoint,
            settings_token: token,
            settings_language: language,
            settings_hotkey: hotkey,
            settings_toggle: toggle,
            recording_mode: None,
            settings_auto_paste: auto_paste,
            settings_auto_start: auto_start,
            settings_close_to_tray: close_to_tray,
            settings_stt_provider: stt_provider,
            settings_groq_api_key: groq_key,
            settings_groq_model: groq_model,
            settings_elevenlabs_api_key: elevenlabs_key,
            settings_elevenlabs_model: elevenlabs_model,
            settings_show_floating_hud: show_hud,
            settings_baseten_api_key: baseten_key,
            settings_baseten_model: baseten_model,
            settings_share_context: share_context,
            settings_store_textbox_contents: store_textbox_contents,
            settings_max_recording_secs: max_recording_secs,
            settings_stt_timeout_secs: stt_timeout_secs,
            settings_silence_filter: silence_filter,
            settings_silence_sensitivity: silence_sensitivity,
            settings_silence_stop_secs: silence_stop_secs,
            settings_hallucination_guard: hallucination_guard,
            settings_autocorrect_enabled: autocorrect_enabled,
            settings_autocorrect_use_llm: autocorrect_use_llm,
            settings_autocorrect_languages: autocorrect_languages,
            settings_mixed_spacing: mixed_spacing,
            clear_history_armed: false,
            settings_check_updates: check_updates,
            settings_include_prerelease: include_prerelease,
            settings_update_interval_hours: update_interval_hours,
            settings_ai_formatting: ai_formatting,
            settings_writing_style: writing_style,
            settings_sound_effects: sound_effects,
            settings_input_device_name: input_device,
            settings_recording_trigger_mode: trigger_mode,
            available_devices: crate::audio::list_input_devices(),
            dict_word_input: String::new(),
            dict_replacement_input: String::new(),
            dict_search_query: String::new(),
            animation_time: 0.0,
            recording_start: None,
            fonts_initialized: false,
            logo_texture: None,
            window_icon_applied: false,
            show_exit_confirmation: false,
            stt_test_status: Arc::new(Mutex::new(None)),
            stt_test_in_progress: Arc::new(AtomicBool::new(false)),
            last_seen_update: None,
        }
    }

    fn configure_thai_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();

        let mut loaded_primary = false;
        let mut loaded_thai = false;
        let mut loaded_cjk = false;
        let mut loaded_emoji = false;

        // 1. Primary UI Latin font (Segoe UI on Windows, Ubuntu/Liberation/DejaVu on Linux)
        #[cfg(target_os = "windows")]
        {
            let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
            let font_dir = std::path::Path::new(&win_dir).join("Fonts");
            let segoe_candidates = [font_dir.join("SegUIVar.ttf"), font_dir.join("segoeui.ttf")];
            for path in &segoe_candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(path) {
                        fonts
                            .font_data
                            .insert("primary_ui".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_primary = true;
                        break;
                    }
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut candidates: Vec<std::path::PathBuf> = Vec::new();
            if let Ok(out) = std::process::Command::new("fc-match")
                .args(["-f", "%{file}", "sans-serif"])
                .output()
            {
                if out.status.success() {
                    let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        candidates.push(std::path::PathBuf::from(path_str));
                    }
                }
            }
            candidates.extend(vec![
                std::path::PathBuf::from("/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf"),
                std::path::PathBuf::from(
                    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                ),
                std::path::PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf"),
            ]);
            for path in candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(&path) {
                        fonts
                            .font_data
                            .insert("primary_ui".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_primary = true;
                        break;
                    }
                }
            }
        }

        // 2. Thai font candidates (Leelawadee UI / Tahoma on Windows; Noto Sans Thai / TLWG / Sarabun on Linux)
        #[cfg(target_os = "windows")]
        {
            let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
            let font_dir = std::path::Path::new(&win_dir).join("Fonts");
            let thai_candidates = [
                font_dir.join("LeelawUI.ttf"),
                font_dir.join("leelawad.ttf"),
                font_dir.join("tahoma.ttf"),
            ];
            for path in &thai_candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(path) {
                        fonts
                            .font_data
                            .insert("thai_font".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_thai = true;
                        break;
                    }
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut candidates: Vec<std::path::PathBuf> = Vec::new();
            // Prioritize standard WTT 2.0 / Windows PUA fonts (Garuda, Waree, etc.)
            // These fonts contain glyphs at 0xF700..0xF71A that eliminate floating tone marks ("สระลอย")
            // and ascender collisions in toolkits without HarfBuzz.
            candidates.extend(vec![
                std::path::PathBuf::from("/usr/share/fonts/truetype/tlwg/Garuda.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/tlwg/Waree.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/tlwg/Laksaman.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/tlwg/Umpush.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/tlwg/Norasi.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/tlwg/TlwgMono.ttf"),
            ]);
            if let Ok(home) = std::env::var("HOME") {
                candidates.push(
                    std::path::PathBuf::from(home)
                        .join(".local/share/fonts/truetype/sarabun/Sarabun-Medium.ttf"),
                );
            }
            if let Ok(out) = std::process::Command::new("fc-match")
                .args(["-f", "%{file}", ":lang=th"])
                .output()
            {
                if out.status.success() {
                    let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        candidates.push(std::path::PathBuf::from(path_str));
                    }
                }
            }
            candidates.extend(vec![
                std::path::PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf"),
                std::path::PathBuf::from(
                    "/usr/share/fonts/truetype/noto/NotoSansThaiUI-Regular.ttf",
                ),
                std::path::PathBuf::from(
                    "/usr/share/fonts/truetype/noto/NotoLoopedThai-Regular.ttf",
                ),
                std::path::PathBuf::from("/usr/share/fonts/truetype/freefont/FreeSerif.ttf"),
            ]);
            for path in candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(&path) {
                        fonts
                            .font_data
                            .insert("thai_font".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_thai = true;
                        tracing::info!("Loaded Linux Thai font from {:?}", path);
                        break;
                    }
                }
            }
        }

        // 3. Japanese & CJK font candidates (Yu Gothic, MS Gothic on Windows; IPA Gothic, Noto CJK on Linux)
        #[cfg(target_os = "windows")]
        {
            let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
            let font_dir = std::path::Path::new(&win_dir).join("Fonts");
            let cjk_candidates = [
                font_dir.join("YuGothM.ttc"),
                font_dir.join("YuGothR.ttc"),
                font_dir.join("msgothic.ttc"),
                font_dir.join("meiryo.ttc"),
                font_dir.join("msyh.ttc"),
            ];
            for path in &cjk_candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(path) {
                        fonts
                            .font_data
                            .insert("cjk_font".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_cjk = true;
                        break;
                    }
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut candidates: Vec<std::path::PathBuf> = Vec::new();
            if let Ok(out) = std::process::Command::new("fc-match")
                .args(["-f", "%{file}", ":lang=ja"])
                .output()
            {
                if out.status.success() {
                    let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        candidates.push(std::path::PathBuf::from(path_str));
                    }
                }
            }
            candidates.extend(vec![
                std::path::PathBuf::from("/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/opentype/ipafont-gothic/ipagp.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/fonts-japanese-gothic.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
                std::path::PathBuf::from(
                    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
                ),
                std::path::PathBuf::from("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"),
            ]);
            for path in candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(&path) {
                        fonts
                            .font_data
                            .insert("cjk_font".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_cjk = true;
                        tracing::info!("Loaded Linux Japanese/CJK font from {:?}", path);
                        break;
                    }
                }
            }
        }

        // 4. Emoji font
        #[cfg(target_os = "windows")]
        {
            let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
            let emoji_path = std::path::Path::new(&win_dir)
                .join("Fonts")
                .join("seguiemj.ttf");
            if emoji_path.exists() {
                if let Ok(bytes) = std::fs::read(&emoji_path) {
                    fonts
                        .font_data
                        .insert("emoji_font".to_owned(), egui::FontData::from_owned(bytes));
                    loaded_emoji = true;
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let emoji_candidates = [
                std::path::PathBuf::from("/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/ancient-scripts/Symbola.ttf"),
            ];
            for path in emoji_candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(&path) {
                        fonts
                            .font_data
                            .insert("emoji_font".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_emoji = true;
                        break;
                    }
                }
            }
        }

        // Configure font fallback priority: English/Latin -> Thai -> Japanese/CJK -> Emoji
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            if loaded_primary {
                family.insert(0, "primary_ui".to_owned());
            }
            if loaded_thai {
                family.push("thai_font".to_owned());
            }
            if loaded_cjk {
                family.push("cjk_font".to_owned());
            }
            if loaded_emoji {
                family.push("emoji_font".to_owned());
            }
        }

        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            if loaded_thai {
                family.push("thai_font".to_owned());
            }
            if loaded_cjk {
                family.push("cjk_font".to_owned());
            }
            if loaded_emoji {
                family.push("emoji_font".to_owned());
            }
        }

        if loaded_primary || loaded_thai || loaded_cjk {
            ctx.set_fonts(fonts);
            tracing::info!(
                "Successfully applied typography: primary={}, thai={}, cjk={}, emoji={}",
                loaded_primary,
                loaded_thai,
                loaded_cjk,
                loaded_emoji
            );
        }
    }

    fn apply_custom_theme(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(248, 250, 252));
        visuals.panel_fill = Color32::from_rgb(11, 15, 23); // Obsidian slate background
        visuals.window_fill = Color32::from_rgb(18, 24, 38);
        visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(34, 44, 64));
        visuals.window_rounding = Rounding::same(14.0);

        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(17, 23, 36);
        visuals.widgets.noninteractive.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66));
        visuals.widgets.noninteractive.rounding = Rounding::same(12.0);

        visuals.widgets.inactive.bg_fill = Color32::from_rgb(25, 33, 50);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(42, 54, 80));
        visuals.widgets.inactive.rounding = Rounding::same(8.0);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(203, 213, 225));

        visuals.widgets.hovered.bg_fill = Color32::from_rgb(37, 49, 74);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(99, 102, 241));
        visuals.widgets.hovered.rounding = Rounding::same(8.0);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);

        visuals.widgets.active.bg_fill = Color32::from_rgb(79, 70, 229);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(129, 140, 248));
        visuals.widgets.active.rounding = Rounding::same(8.0);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);

        visuals.extreme_bg_color = Color32::from_rgb(14, 19, 30);

        ctx.set_visuals(visuals);
    }
}

impl eframe::App for WisprFlowGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok(mut lock) = self.state.egui_ctx.lock() {
            if lock.is_none() {
                *lock = Some(ctx.clone());
            }
        }

        if !self.fonts_initialized {
            Self::configure_thai_fonts(ctx);
            self.fonts_initialized = true;
        }

        if self.logo_texture.is_none() {
            if let Ok(img) = image::load_from_memory(include_bytes!("../assets/icon_256.png")) {
                let rgba = img.into_rgba8();
                let (w, h) = rgba.dimensions();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    &rgba.into_raw(),
                );
                self.logo_texture = Some(ctx.load_texture(
                    "wispr_flow_app_logo",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }

        #[cfg(target_os = "windows")]
        if !self.window_icon_applied {
            crate::platform::windows::PlatformManager::apply_window_icon(
                crate::platform::windows::PlatformManager::get_app_icon(1),
            );
            if !self.state.window_visible.load(Ordering::SeqCst) {
                crate::platform::windows::PlatformManager::hide_main_window();
            }
            self.window_icon_applied = true;
        }

        if let Ok(cfg) = self.state.config.try_lock() {
            if self.settings_auto_start != cfg.auto_start {
                self.settings_auto_start = cfg.auto_start;
            }
        }

        // Handle recorded key from low-level hook
        if let Ok(mut lock) = self.state.recorded_key.lock() {
            if let Some((mode, combo)) = lock.take() {
                self.recording_mode = None;
                if combo.is_empty() {
                    self.state.set_toast("Key recording cancelled");
                } else {
                    let key_name = combo.format_display();
                    if mode == 1 {
                        self.settings_hotkey = combo;
                        self.state
                            .set_toast(&format!("Dictation hotkey changed to: {}", key_name));
                    } else if mode == 2 {
                        self.settings_toggle = combo;
                        self.state
                            .set_toast(&format!("Window Toggle hotkey changed to: {}", key_name));
                    }
                }
            }
        }

        // When in key recording mode, request continuous repaint and listen for Escape
        if self.recording_mode.is_some() {
            ctx.request_repaint();
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.recording_mode = None;
                PlatformManager::stop_recording_hotkey();
                self.state.set_toast("Key recording cancelled");
            }
        }

        let (dictate_key_name, toggle_key_name) = {
            let cfg = lock(&self.state.config);
            (
                cfg.hotkey.format_display(),
                cfg.toggle_window.format_display(),
            )
        };

        if self.state.pending_quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Window close [X] -> minimize to tray or show exit confirmation modal
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.settings_close_to_tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                #[cfg(target_os = "windows")]
                crate::platform::windows::PlatformManager::hide_main_window();
                #[cfg(not(target_os = "windows"))]
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    crate::platform::PlatformManager::hide_main_window();
                }
                self.state.window_visible.store(false, Ordering::SeqCst);
                self.state.set_toast(&format!(
                    "Minimized to Tray. Press {} to reopen.",
                    toggle_key_name
                ));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.show_exit_confirmation = true;
            }
        }

        Self::apply_custom_theme(ctx);
        self.animation_time += 0.04;

        let is_rec = self.state.is_recording.load(Ordering::SeqCst);
        let amplitude = *lock(&self.state.current_amplitude);
        let status = *lock(&self.state.status);

        // Track live recording timer
        if is_rec {
            if self.recording_start.is_none() {
                self.recording_start = Some(Instant::now());
            }
        } else {
            self.recording_start = None;
        }

        // 60 FPS repainting when recording or animating
        if is_rec || status != DictationStatus::Idle || self.recording_mode.is_some() {
            ctx.request_repaint();
        }

        // Keep the update button in sync with the background checker: repaint
        // while a check is running and whenever the result changes.
        if self.state.update_check_running.load(Ordering::SeqCst) {
            ctx.request_repaint();
        }
        let current_update = lock(&self.state.update_info).clone();
        if current_update != self.last_seen_update {
            self.last_seen_update = current_update;
            ctx.request_repaint();
        }
        // egui only redraws on demand, so a background result can otherwise land
        // on a stale frame. A cheap heartbeat keeps the status bar, the update
        // button and the recording timer in sync.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        // In-window hotkey trigger (mirrors the configured global hotkey)
        let hotkey_vk = self
            .state
            .config
            .lock()
            .map(|c| c.hotkey.key)
            .unwrap_or(0x77);
        if self.recording_mode.is_none()
            && egui_key_from_vk(hotkey_vk)
                .map(|key| ctx.input(|i| i.key_pressed(key)))
                .unwrap_or(false)
        {
            if is_rec {
                let _ = self.event_tx.send(GuiEvent::StopDictation);
            } else {
                let _ = self.event_tx.send(GuiEvent::StartDictation);
            }
        }

        // 1. Modern Left Sidebar Navigation
        egui::SidePanel::left("wflow_left_sidebar")
            .resizable(false)
            .exact_width(220.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(12, 16, 26))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(26, 36, 54)))
                    .inner_margin(egui::Margin::symmetric(14.0, 16.0)),
            )
            .show(ctx, |ui| {
                self.render_sidebar(ui, &dictate_key_name, &toggle_key_name);
            });

        // 2. Main Central Area
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(15, 20, 32))
                    .inner_margin(egui::Margin::symmetric(20.0, 16.0)),
            )
            .show(ctx, |ui| {
                // Top Action & Status Bar
                self.render_top_bar(ui, status, is_rec, &dictate_key_name, &toggle_key_name);

                self.render_update_banner(ui);

                ui.add_space(14.0);

                // Active Tab View
                match self.active_tab {
                    ActiveTab::Studio => {
                        self.render_studio_tab(ui, is_rec, amplitude, status, &dictate_key_name)
                    }
                    ActiveTab::History => self.render_history_tab(ui),
                    ActiveTab::Dictionary => self.render_dictionary_tab(ui),
                    ActiveTab::Updates => self.render_updates_tab(ui),
                    ActiveTab::Settings => self.render_settings_tab(ui),
                }

                // Toast notification overlay
                self.render_toast(ui);
            });

        // Exit confirmation modal
        if self.show_exit_confirmation {
            self.render_exit_confirmation_modal(ctx);
        }

        // Linux/Wayland: the native HUD is Windows-only, so draw an egui viewport.
        #[cfg(not(target_os = "windows"))]
        self.render_floating_hud(ctx, status, amplitude);
    }
}

/// Visual state of the "update" button in the top bar.
///
/// Returns the label plus the fill / stroke / text colours.
fn update_button_style(
    latest: Option<&str>,
    running: bool,
    current_version: &str,
    animation_time: f32,
) -> (String, Color32, Color32, Color32) {
    if let Some(latest) = latest {
        // Gentle pulse so the action is impossible to miss.
        let pulse = 0.5 + 0.5 * (animation_time * 3.0).sin();
        return (
            format!("⬆️  อัปเดตใหม่ v{}", latest),
            Color32::from_rgb(22, 163, 74),
            Color32::from_rgb(74, 222, 128).gamma_multiply(0.65 + 0.35 * pulse),
            Color32::WHITE,
        );
    }
    if running {
        return (
            "⏳  กำลังตรวจสอบ...".to_string(),
            Color32::from_rgb(30, 41, 59),
            Color32::from_rgb(100, 116, 139),
            Color32::from_rgb(203, 213, 225),
        );
    }
    (
        format!("🔄  อัปเดต · v{}", current_version),
        Color32::from_rgb(30, 41, 59),
        Color32::from_rgb(71, 85, 105),
        Color32::from_rgb(203, 213, 225),
    )
}

/// Maps a Windows virtual-key code to an egui key for the in-window shortcut.
fn egui_key_from_vk(vk: u16) -> Option<egui::Key> {
    match vk {
        0x08 => Some(egui::Key::Backspace),
        0x09 => Some(egui::Key::Tab),
        0x0D => Some(egui::Key::Enter),
        0x1B => Some(egui::Key::Escape),
        0x20 => Some(egui::Key::Space),
        0x30..=0x39 => match vk {
            0x30 => Some(egui::Key::Num0),
            0x31 => Some(egui::Key::Num1),
            0x32 => Some(egui::Key::Num2),
            0x33 => Some(egui::Key::Num3),
            0x34 => Some(egui::Key::Num4),
            0x35 => Some(egui::Key::Num5),
            0x36 => Some(egui::Key::Num6),
            0x37 => Some(egui::Key::Num7),
            0x38 => Some(egui::Key::Num8),
            _ => Some(egui::Key::Num9),
        },
        0x41..=0x5A => match vk {
            0x41 => Some(egui::Key::A),
            0x42 => Some(egui::Key::B),
            0x43 => Some(egui::Key::C),
            0x44 => Some(egui::Key::D),
            0x45 => Some(egui::Key::E),
            0x46 => Some(egui::Key::F),
            0x47 => Some(egui::Key::G),
            0x48 => Some(egui::Key::H),
            0x49 => Some(egui::Key::I),
            0x4A => Some(egui::Key::J),
            0x4B => Some(egui::Key::K),
            0x4C => Some(egui::Key::L),
            0x4D => Some(egui::Key::M),
            0x4E => Some(egui::Key::N),
            0x4F => Some(egui::Key::O),
            0x50 => Some(egui::Key::P),
            0x51 => Some(egui::Key::Q),
            0x52 => Some(egui::Key::R),
            0x53 => Some(egui::Key::S),
            0x54 => Some(egui::Key::T),
            0x55 => Some(egui::Key::U),
            0x56 => Some(egui::Key::V),
            0x57 => Some(egui::Key::W),
            0x58 => Some(egui::Key::X),
            0x59 => Some(egui::Key::Y),
            _ => Some(egui::Key::Z),
        },
        0x70..=0x7B => match vk {
            0x70 => Some(egui::Key::F1),
            0x71 => Some(egui::Key::F2),
            0x72 => Some(egui::Key::F3),
            0x73 => Some(egui::Key::F4),
            0x74 => Some(egui::Key::F5),
            0x75 => Some(egui::Key::F6),
            0x76 => Some(egui::Key::F7),
            0x77 => Some(egui::Key::F8),
            0x78 => Some(egui::Key::F9),
            0x79 => Some(egui::Key::F10),
            0x7A => Some(egui::Key::F11),
            _ => Some(egui::Key::F12),
        },
        _ => None,
    }
}

impl WisprFlowGui {
    fn render_sidebar(&mut self, ui: &mut egui::Ui, dictate_key_name: &str, toggle_key_name: &str) {
        ui.vertical(|ui| {
            // App Brand Header
            ui.horizontal(|ui| {
                let (logo_rect, _) = ui.allocate_exact_size(Vec2::new(36.0, 36.0), Sense::hover());
                if let Some(texture) = &self.logo_texture {
                    ui.painter().rect_filled(
                        logo_rect.expand(1.5),
                        Rounding::same(9.0),
                        Color32::from_rgba_premultiplied(99, 102, 241, 45),
                    );
                    ui.painter().image(
                        texture.id(),
                        logo_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    ui.painter().rect_filled(
                        logo_rect,
                        Rounding::same(9.0),
                        Color32::from_rgb(49, 46, 129),
                    );
                    ui.painter().text(
                        logo_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "🎙️",
                        FontId::proportional(16.0),
                        Color32::WHITE,
                    );
                }

                ui.add_space(8.0);

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("WFLOW")
                                .font(FontId::proportional(17.0))
                                .strong()
                                .color(Color32::WHITE),
                        );
                        let rust_badge = ui.label(
                            egui::RichText::new("RUST")
                                .font(FontId::proportional(9.5))
                                .color(Color32::from_rgb(165, 180, 252))
                                .strong(),
                        );
                        ui.painter().rect_filled(
                            rust_badge.rect.expand(2.0),
                            Rounding::same(3.0),
                            Color32::from_rgba_unmultiplied(99, 102, 241, 45),
                        );
                    });
                    ui.label(
                        egui::RichText::new("AI Voice Studio")
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                });
            });

            ui.add_space(14.0);
            ui.separator();
            ui.add_space(10.0);

            // Navigation Menu
            let history_count = lock(&self.state.history).len();
            let dict_count = lock(&self.state.dictionary_entries).len();

            // The Updates entry carries a badge when a new release exists so
            // it can never be missed.
            let update_badge = self.pending_update().is_some();

            let nav_items = [
                (ActiveTab::Studio, "🎙️", "Live Studio", None),
                (ActiveTab::History, "📜", "History", Some(history_count)),
                (ActiveTab::Dictionary, "📖", "Dictionary", Some(dict_count)),
                (
                    ActiveTab::Updates,
                    "🔄",
                    "Updates",
                    if update_badge { Some(1) } else { None },
                ),
                (ActiveTab::Settings, "⚙️", "Settings", None),
            ];

            for (tab, icon, label, count) in nav_items {
                let is_active = self.active_tab == tab;
                let (btn_rect, response) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());

                let is_hovered = response.hovered();
                let bg_color = if is_active {
                    Color32::from_rgb(79, 70, 229)
                } else if is_hovered {
                    Color32::from_rgb(26, 35, 52)
                } else {
                    Color32::TRANSPARENT
                };

                ui.painter()
                    .rect_filled(btn_rect, Rounding::same(8.0), bg_color);

                if is_active {
                    let accent_rect = Rect::from_min_max(
                        btn_rect.min,
                        Pos2::new(btn_rect.min.x + 3.5, btn_rect.max.y),
                    );
                    ui.painter().rect_filled(
                        accent_rect,
                        Rounding::same(2.0),
                        Color32::from_rgb(165, 180, 252),
                    );
                }

                let icon_pos = Pos2::new(btn_rect.min.x + 12.0, btn_rect.center().y);
                ui.painter().text(
                    icon_pos,
                    egui::Align2::LEFT_CENTER,
                    icon,
                    FontId::proportional(15.0),
                    Color32::WHITE,
                );

                let text_pos = Pos2::new(btn_rect.min.x + 38.0, btn_rect.center().y);
                let text_color = if is_active {
                    Color32::WHITE
                } else if is_hovered {
                    Color32::from_rgb(241, 245, 249)
                } else {
                    Color32::from_rgb(160, 174, 192)
                };
                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(13.0),
                    text_color,
                );

                if let Some(c) = count {
                    if c > 0 {
                        let badge_text = format!("{}", c);
                        let badge_pos = Pos2::new(btn_rect.max.x - 14.0, btn_rect.center().y);
                        let badge_bg = if is_active {
                            Color32::from_rgb(99, 102, 241)
                        } else {
                            Color32::from_rgb(33, 44, 66)
                        };
                        let badge_color = if is_active {
                            Color32::WHITE
                        } else {
                            Color32::from_rgb(148, 163, 184)
                        };
                        let badge_rect = Rect::from_center_size(badge_pos, Vec2::new(24.0, 18.0));
                        ui.painter()
                            .rect_filled(badge_rect, Rounding::same(9.0), badge_bg);
                        ui.painter().text(
                            badge_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &badge_text,
                            FontId::proportional(10.5),
                            badge_color,
                        );
                    }
                }

                if response.clicked() {
                    self.active_tab = tab;
                    if tab == ActiveTab::History {
                        let _ = self.event_tx.send(GuiEvent::RefreshHistory);
                    } else if tab == ActiveTab::Dictionary {
                        let _ = self.event_tx.send(GuiEvent::RefreshDictionary);
                    }
                }

                ui.add_space(4.0);
            }

            // Bottom System Status Card
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.add_space(6.0);

                // 1. Explicit Exit Button
                let exit_btn = egui::Button::new(
                    egui::RichText::new(shape_thai("🔴 ปิดโปรแกรม (Exit)"))
                        .font(FontId::proportional(11.5))
                        .color(Color32::from_rgb(252, 165, 165))
                        .strong(),
                )
                .fill(Color32::from_rgb(45, 18, 22))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(185, 28, 28)))
                .rounding(Rounding::same(7.0))
                .min_size(Vec2::new(ui.available_width(), 30.0));

                if ui.add(exit_btn).clicked() {
                    self.show_exit_confirmation = true;
                }

                ui.add_space(4.0);

                // 2. Minimize Button
                let hide_btn = egui::Button::new(
                    egui::RichText::new(shape_thai(&format!("📌 ย่อหน้าต่าง ({})", toggle_key_name)))
                        .font(FontId::proportional(11.5))
                        .color(Color32::from_rgb(203, 213, 225)),
                )
                .fill(Color32::from_rgb(22, 29, 44))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 50, 74)))
                .rounding(Rounding::same(7.0))
                .min_size(Vec2::new(ui.available_width(), 30.0));

                if ui.add(hide_btn).clicked() {
                    #[cfg(target_os = "windows")]
                    crate::platform::windows::PlatformManager::hide_main_window();
                    #[cfg(not(target_os = "windows"))]
                    {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        crate::platform::PlatformManager::hide_main_window();
                    }
                    self.state.window_visible.store(false, Ordering::SeqCst);
                    self.state
                        .set_toast(&format!("Minimized. Press {} to reopen.", toggle_key_name));
                }

                ui.add_space(8.0);

                // Status info card
                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 41, 64)))
                    .rounding(Rounding::same(9.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        let cfg = lock(&self.state.config);
                        let prov_badge = match cfg.stt_provider {
                            SttProvider::WisprFlow => {
                                ("⚡ Wispr Flow", Color32::from_rgb(129, 140, 248))
                            }
                            SttProvider::Groq => {
                                ("🚀 Groq Whisper", Color32::from_rgb(251, 146, 60))
                            }
                            SttProvider::ElevenLabs => {
                                ("🎙️ ElevenLabs", Color32::from_rgb(192, 132, 252))
                            }
                        };

                        let mic_label = if let Some(ref m) = cfg.input_device_name {
                            if m.len() > 18 {
                                format!("{}...", &m[..15])
                            } else {
                                m.clone()
                            }
                        } else {
                            "Default Mic".to_string()
                        };

                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("ENGINE")
                                    .font(FontId::proportional(9.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(prov_badge.0)
                                            .font(FontId::proportional(10.5))
                                            .color(prov_badge.1)
                                            .strong(),
                                    );
                                },
                            );
                        });

                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("MIC")
                                    .font(FontId::proportional(9.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("🎤 {}", mic_label))
                                            .font(FontId::proportional(10.5))
                                            .color(Color32::from_rgb(203, 213, 225)),
                                    );
                                },
                            );
                        });

                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("HOTKEY")
                                    .font(FontId::proportional(9.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("[{}]", dictate_key_name))
                                            .font(FontId::monospace(10.5))
                                            .color(Color32::from_rgb(56, 189, 248))
                                            .strong(),
                                    );
                                },
                            );
                        });

                        ui.add_space(4.0);

                        let pending = self.pending_update();
                        let pending_download = pending.as_ref().map(|i| i.download_url.clone());
                        let pending_latest = pending.as_ref().map(|i| i.latest.to_string());
                        let (version_text, version_color) = match pending_latest.as_ref() {
                            Some(latest) => (
                                format!("v{} ⬆️ v{}", crate::update::current_version(), latest),
                                Color32::from_rgb(74, 222, 128),
                            ),
                            None => (
                                format!("v{}", crate::update::current_version()),
                                Color32::from_rgb(203, 213, 225),
                            ),
                        };

                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("VERSION")
                                    .font(FontId::proportional(9.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let galley = ui.painter().layout_no_wrap(
                                        version_text.clone(),
                                        FontId::monospace(10.5),
                                        version_color,
                                    );
                                    let (rect, response) =
                                        ui.allocate_exact_size(galley.size(), Sense::click());
                                    ui.painter().text(
                                        rect.min,
                                        egui::Align2::LEFT_TOP,
                                        &version_text,
                                        FontId::monospace(10.5),
                                        version_color,
                                    );
                                    if response.clicked() {
                                        if let (Some(url), Some(latest)) =
                                            (pending_download.as_ref(), pending_latest.as_ref())
                                        {
                                            let _ =
                                                self.event_tx.send(GuiEvent::OpenUrl(url.clone()));
                                            self.state.set_toast(&format!(
                                                "กำลังเปิดหน้าดาวน์โหลด v{}",
                                                latest
                                            ));
                                        } else {
                                            let _ = self.event_tx.send(GuiEvent::CheckUpdate);
                                            self.state.set_toast("กำลังตรวจสอบอัปเดต...");
                                        }
                                    }
                                    if response.hovered() {
                                        response.show_tooltip_text("คลิกเพื่อตรวจสอบ/ดาวน์โหลดอัปเดต");
                                    }
                                },
                            );
                        });
                    });

                ui.add_space(8.0);
            });
        });
    }

    fn render_top_bar(
        &mut self,
        ui: &mut egui::Ui,
        status: DictationStatus,
        is_rec: bool,
        dictate_key_name: &str,
        _toggle_key_name: &str,
    ) {
        // The update button never moves to another tab and is never hidden.
        // The decision whether it needs its own row is made from the full
        // width of the panel, *before* the title takes up any space.
        let update_available = self.pending_update();
        let update_running = self.state.update_check_running.load(Ordering::SeqCst);
        let current_version = crate::update::current_version().to_string();
        let (update_label, update_fill, update_stroke, update_text) = update_button_style(
            update_available
                .as_ref()
                .map(|i| i.latest.to_string())
                .as_deref(),
            update_running,
            &current_version,
            self.animation_time,
        );

        let update_label = update_label.clone();
        let build_update_button = || {
            egui::Button::new(
                egui::RichText::new(shape_thai(&update_label))
                    .font(FontId::proportional(12.0))
                    .color(update_text)
                    .strong(),
            )
            .fill(update_fill)
            .stroke(Stroke::new(1.2_f32, update_stroke))
            .rounding(Rounding::same(8.0))
        };

        // The button always gets its own row, right aligned, so that it can
        // never be clipped by the status pills — no matter how narrow the
        // window is.
        //
        // The button sits in its own full-width row, right aligned. The row
        // has to be a `horizontal` layout: a bare `with_layout` in a vertical
        // context would stretch over the whole panel and centre the button
        // vertically, pushing the rest of the UI out of view.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let response = ui.add_enabled(!update_running, build_update_button());
                if response.clicked() {
                    self.trigger_update_action(&update_available);
                }
                if response.hovered() {
                    let tip = if update_available.is_some() {
                        "คลิกเพื่อดาวน์โหลดเวอร์ชันใหม่"
                    } else {
                        "คลิกเพื่อตรวจสอบอัปเดต"
                    };
                    response.show_tooltip_text(tip);
                }
            });
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            let (title, subtitle) = match self.active_tab {
                ActiveTab::Studio => (
                    "🎙️ Live Speech Studio",
                    "Zero-latency real-time voice typing",
                ),
                ActiveTab::History => (
                    "📜 Dictation History",
                    "Search, copy, or re-paste past speech",
                ),
                ActiveTab::Dictionary => (
                    "📖 Custom Vocabulary & Snippets",
                    "Domain terminology and text expansion shortcuts",
                ),
                ActiveTab::Updates => (
                    "🔄 Software Updates",
                    "Check for a new version of Wflow and download it",
                ),
                ActiveTab::Settings => (
                    "⚙️ Settings & Audio Hardware",
                    "Configure microphones, AI models, and keyboard shortcuts",
                ),
            };

            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .font(FontId::proportional(16.5))
                        .strong()
                        .color(Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(subtitle)
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Quick Dictate Action Button
                let quick_btn = if is_rec {
                    egui::Button::new(
                        egui::RichText::new(shape_thai("⏹️ หยุด (Stop)"))
                            .font(FontId::proportional(12.0))
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(185, 28, 28))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(239, 68, 68)))
                    .rounding(Rounding::same(8.0))
                } else {
                    egui::Button::new(
                        egui::RichText::new(shape_thai("🎙️ เริ่มพูด (Dictate)"))
                            .font(FontId::proportional(12.0))
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(37, 99, 235))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(96, 165, 250)))
                    .rounding(Rounding::same(8.0))
                };

                if ui.add(quick_btn).clicked() {
                    if is_rec {
                        let _ = self.event_tx.send(GuiEvent::StopDictation);
                    } else {
                        let _ = self.event_tx.send(GuiEvent::StartDictation);
                    }
                }

                ui.add_space(8.0);
                let (status_text, bg_color, border_color) = match status {
                    DictationStatus::Idle => (
                        format!("● Ready [{}]", dictate_key_name),
                        Color32::from_rgb(6, 78, 59),
                        Color32::from_rgb(16, 185, 129),
                    ),
                    DictationStatus::Listening => (
                        "🔴 Recording...".to_string(),
                        Color32::from_rgb(127, 29, 29),
                        Color32::from_rgb(239, 68, 68),
                    ),
                    DictationStatus::Transcribing => (
                        "✨ AI Processing...".to_string(),
                        Color32::from_rgb(88, 28, 135),
                        Color32::from_rgb(168, 85, 247),
                    ),
                    DictationStatus::Pasting => (
                        "✍️ Pasting Text...".to_string(),
                        Color32::from_rgb(30, 58, 138),
                        Color32::from_rgb(59, 130, 246),
                    ),
                    DictationStatus::Done => (
                        "✔ Inserted!".to_string(),
                        Color32::from_rgb(6, 78, 59),
                        Color32::from_rgb(16, 185, 129),
                    ),
                    DictationStatus::Error => (
                        "⚠️ Audio Error".to_string(),
                        Color32::from_rgb(120, 53, 15),
                        Color32::from_rgb(245, 158, 11),
                    ),
                };

                // Paint the pill background first, then the text on top of it.
                let status_galley = ui.painter().layout_no_wrap(
                    status_text.clone(),
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
                let (status_rect, _) = ui.allocate_exact_size(
                    status_galley.size() + egui::vec2(20.0, 12.0),
                    Sense::hover(),
                );
                ui.painter()
                    .rect_filled(status_rect, Rounding::same(12.0), bg_color);
                ui.painter().rect_stroke(
                    status_rect,
                    Rounding::same(12.0),
                    Stroke::new(1.0_f32, border_color),
                );
                ui.painter().text(
                    status_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    status_text,
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );

                ui.add_space(8.0);

                let active_app = lock(&self.state.active_app).clone();
                let app_display = if active_app.process_name.is_empty() {
                    "Desktop".to_string()
                } else {
                    active_app.process_name.clone()
                };

                let ctx_label = format!("🎯 Focus: {}", app_display);
                let ctx_galley = ui.painter().layout_no_wrap(
                    ctx_label.clone(),
                    FontId::proportional(11.5),
                    Color32::from_rgb(148, 163, 184),
                );
                let (ctx_rect, _) = ui.allocate_exact_size(
                    ctx_galley.size() + egui::vec2(16.0, 10.0),
                    Sense::hover(),
                );
                ui.painter().rect_filled(
                    ctx_rect,
                    Rounding::same(8.0),
                    Color32::from_rgb(22, 29, 44),
                );
                ui.painter().rect_stroke(
                    ctx_rect,
                    Rounding::same(8.0),
                    Stroke::new(1.0_f32, Color32::from_rgb(38, 50, 74)),
                );
                ui.painter().text(
                    ctx_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    ctx_label,
                    FontId::proportional(11.5),
                    Color32::from_rgb(148, 163, 184),
                );
            });
        });
    }

    fn render_studio_tab(
        &mut self,
        ui: &mut egui::Ui,
        is_rec: bool,
        amplitude: f32,
        status: DictationStatus,
        dictate_key_name: &str,
    ) {
        ui.columns(2, |cols| {
            // LEFT COLUMN: Live Audio Visualizer & Controls
            cols[0].vertical(|ui| {
                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(16.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("🎙️  AUDIO CAPTURE & WAVEFORM")
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(148, 163, 184))
                                    .strong(),
                            );

                            // Voice activity indicator: shows whether the
                            // noise gate currently hears speech.
                            if is_rec {
                                let (speaking, noise, threshold) = {
                                    let slot = lock(&self.state.voice_gate).clone();
                                    match slot {
                                        Some(gate) => {
                                            let g = lock(&gate);
                                            (g.is_speaking(), g.noise_floor(), g.threshold())
                                        }
                                        None => (true, 0.0, 0.0),
                                    }
                                };

                                let (label, color) = if speaking {
                                    ("🗣️ กำลังพูด", Color32::from_rgb(74, 222, 128))
                                } else {
                                    ("🤫 เสียงเงียบ", Color32::from_rgb(148, 163, 184))
                                };

                                let pill = ui.label(
                                    egui::RichText::new(format!(
                                        "{}  {:.0}dB / {:.0}dB",
                                        label,
                                        (noise * 1000.0).log10() * 20.0,
                                        (threshold * 1000.0).log10() * 20.0
                                    ))
                                    .font(FontId::monospace(10.5))
                                    .color(color)
                                    .strong(),
                                );
                                ui.painter().rect_filled(
                                    pill.rect.expand(3.0),
                                    Rounding::same(6.0),
                                    Color32::from_rgb(22, 29, 44),
                                );
                                ui.painter().rect_stroke(
                                    pill.rect.expand(3.0),
                                    Rounding::same(6.0),
                                    Stroke::new(1.0_f32, Color32::from_rgb(38, 50, 74)),
                                );
                            }

                            if is_rec {
                                let elapsed = self.recording_start.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
                                let mins = (elapsed / 60.0).floor() as u32;
                                let secs = elapsed % 60.0;
                                let time_str = format!("{:02}:{:04.1}", mins, secs);

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let pill = ui.label(
                                        egui::RichText::new(format!("🔴 LIVE • {}", time_str))
                                            .font(FontId::monospace(11.5))
                                            .color(Color32::from_rgb(254, 202, 202))
                                            .strong(),
                                    );
                                    ui.painter().rect_filled(
                                        pill.rect.expand(3.0),
                                        Rounding::same(6.0),
                                        Color32::from_rgb(153, 27, 27),
                                    );
                                });
                            }
                        });

                        ui.add_space(8.0);

                        // Visualizer Canvas (Audio Waveform)
                        let visualizer_height = 110.0;
                        let (rect, _response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), visualizer_height),
                            Sense::hover(),
                        );

                        // Visualizer Background
                        ui.painter().rect_filled(
                            rect,
                            Rounding::same(10.0),
                            Color32::from_rgb(10, 14, 22),
                        );
                        ui.painter().rect_stroke(
                            rect,
                            Rounding::same(10.0),
                            Stroke::new(1.0_f32, Color32::from_rgb(26, 36, 54)),
                        );

                        let bar_count = 36;
                        let bar_width = (rect.width() - (bar_count as f32 * 3.5)) / bar_count as f32;
                        let center_y = rect.center().y;

                        // Center horizontal guide line
                        ui.painter().line_segment(
                            [
                                Pos2::new(rect.min.x + 8.0, center_y),
                                Pos2::new(rect.max.x - 8.0, center_y),
                            ],
                            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 15)),
                        );

                        for i in 0..bar_count {
                            let x = rect.min.x + (i as f32 * (bar_width + 3.5)) + 4.0;
                            let phase = (i as f32 * 0.32) + (self.animation_time * 4.2);
                            let t = i as f32 / bar_count as f32;

                            let height_factor = if is_rec {
                                let wave = (phase.sin() * 0.45 + 0.55) * (amplitude * 7.5).clamp(0.18, 1.0);
                                wave * (visualizer_height * 0.42)
                            } else {
                                (phase.sin() * 0.2 + 0.3) * 5.0
                            };

                            let bar_color = if is_rec {
                                if t < 0.35 {
                                    Color32::from_rgb(6, 182, 212) // Cyan
                                } else if t < 0.70 {
                                    Color32::from_rgb(99, 102, 241) // Indigo
                                } else {
                                    Color32::from_rgb(236, 72, 153) // Pink
                                }
                            } else {
                                Color32::from_rgb(45, 56, 80)
                            };

                            let bar_rect = Rect::from_min_max(
                                Pos2::new(x, center_y - height_factor),
                                Pos2::new(x + bar_width, center_y + height_factor + 1.0),
                            );

                            ui.painter().rect_filled(bar_rect, Rounding::same(3.0), bar_color);
                        }

                        ui.add_space(14.0);

                        // Hero Action Button
                        let is_tap = self.settings_recording_trigger_mode == "tap_to_toggle";
                        let dictate_btn_text = if is_tap {
                            format!("🎙️  START DICTATING (OR TAP {})", dictate_key_name)
                        } else {
                            format!("🎙️  START DICTATING (OR HOLD {})", dictate_key_name)
                        };
                        let (btn_text, btn_bg, btn_stroke) = if is_rec {
                            ("⏹️  CLICK TO COMMIT & INSERT", Color32::from_rgb(220, 38, 38), Color32::from_rgb(248, 113, 113))
                        } else if status == DictationStatus::Transcribing {
                            ("✨  AI TRANSCRIBING...", Color32::from_rgb(124, 58, 237), Color32::from_rgb(168, 85, 247))
                        } else {
                            (dictate_btn_text.as_str(), Color32::from_rgb(79, 70, 229), Color32::from_rgb(129, 140, 248))
                        };

                        let hero_btn = egui::Button::new(
                            egui::RichText::new(btn_text)
                                .font(FontId::proportional(14.0))
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(btn_bg)
                        .stroke(Stroke::new(1.0_f32, btn_stroke))
                        .rounding(Rounding::same(10.0))
                        .min_size(Vec2::new(ui.available_width(), 46.0));

                        if ui.add(hero_btn).clicked() {
                            if is_rec {
                                let _ = self.event_tx.send(GuiEvent::StopDictation);
                            } else if status == DictationStatus::Idle || status == DictationStatus::Done {
                                let _ = self.event_tx.send(GuiEvent::StartDictation);
                            }
                        }

                        ui.add_space(8.0);
                        let tip_text = if is_tap {
                            format!("💡 Tap {} once to start dictating hands-free, tap again to commit and insert text.", dictate_key_name)
                        } else {
                            format!("💡 Hold {} anywhere across Windows to dictate directly into any active text box.", dictate_key_name)
                        };
                        ui.label(
                            egui::RichText::new(tip_text)
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });

                ui.add_space(10.0);

                // Detected Active Context Card
                let active_app = lock(&self.state.active_app).clone();
                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("🎯  TARGET APPLICATION CONTEXT")
                                .font(FontId::proportional(12.0))
                                .color(Color32::from_rgb(148, 163, 184))
                                .strong(),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let proc_text = if active_app.process_name.is_empty() {
                                "Desktop Cursor".to_string()
                            } else {
                                active_app.process_name.clone()
                            };

                            let tag = ui.label(
                                egui::RichText::new(format!("⚡ {}", proc_text))
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .strong(),
                            );
                            ui.painter().rect_filled(
                                tag.rect.expand(3.0),
                                Rounding::same(6.0),
                                Color32::from_rgba_unmultiplied(14, 165, 233, 35),
                            );

                            if !active_app.window_title.is_empty() {
                                ui.label(
                                    egui::RichText::new(format!("— {}", active_app.window_title))
                                        .font(FontId::proportional(11.0))
                                        .color(Color32::from_rgb(148, 163, 184)),
                                );
                            }
                        });

                        if let Some(before) = &active_app.before_text {
                            if !before.is_empty() {
                                ui.add_space(6.0);
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(12, 16, 26))
                                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(26, 36, 54)))
                                    .rounding(Rounding::same(6.0))
                                    .inner_margin(egui::Margin::same(8.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(format!("Before Cursor: \"{}\"", before))
                                                .font(FontId::proportional(11.0))
                                                .italics()
                                                .color(Color32::from_rgb(203, 213, 225)),
                                        );
                                    });
                            }
                        }
                    });

                ui.add_space(10.0);

                // Active STT Pipeline & Model Status Card
                let (prov_title, model_name, prov_color) = {
                    let cfg = lock(&self.state.config);
                    match cfg.stt_provider {
                        SttProvider::WisprFlow => ("Wispr Flow Cloud".to_string(), "Native Stream (gRPC)".to_string(), Color32::from_rgb(129, 140, 248)),
                        SttProvider::Groq => ("Groq Cloud LPU".to_string(), cfg.groq_model.clone(), Color32::from_rgb(251, 146, 60)),
                        SttProvider::ElevenLabs => ("ElevenLabs".to_string(), cfg.elevenlabs_model.clone(), Color32::from_rgb(192, 132, 252)),
                    }
                };

                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("⚡  ACTIVE AI SPEECH ENGINE & MODEL")
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(148, 163, 184))
                                    .strong(),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("⚙️ Switch Model").clicked() {
                                    self.active_tab = ActiveTab::Settings;
                                }
                            });
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let tag = ui.label(
                                egui::RichText::new(format!("● {}", prov_title))
                                    .font(FontId::proportional(12.0))
                                    .color(prov_color)
                                    .strong(),
                            );
                            ui.painter().rect_filled(
                                tag.rect.expand(3.0),
                                Rounding::same(6.0),
                                Color32::from_rgba_unmultiplied(prov_color.r(), prov_color.g(), prov_color.b(), 35),
                            );

                            ui.label(
                                egui::RichText::new(format!("Model: {}", model_name))
                                    .font(FontId::proportional(12.5))
                                    .color(Color32::WHITE)
                                    .strong(),
                            );
                        });
                    });
            });

            // RIGHT COLUMN: Live Transcript & Quick Actions
            cols[1].vertical(|ui| {
                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(16.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("📝  LATEST TRANSCRIPTION")
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(148, 163, 184))
                                    .strong(),
                            );

                            let latency = *lock(&self.state.last_latency);
                            if latency > 0.0 {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let lat_pill = ui.label(
                                        egui::RichText::new(format!("⚡ {:.0} ms", latency * 1000.0))
                                            .font(FontId::monospace(11.0))
                                            .color(Color32::from_rgb(74, 222, 128))
                                            .strong(),
                                    );
                                    ui.painter().rect_filled(
                                        lat_pill.rect.expand(2.5),
                                        Rounding::same(4.0),
                                        Color32::from_rgba_unmultiplied(34, 197, 94, 30),
                                    );
                                });
                            }
                        });

                        ui.add_space(8.0);

                        let transcript = lock(&self.state.last_transcript).clone();

                        egui::ScrollArea::vertical()
                            .max_height(190.0)
                            .show(ui, |ui| {
                                if transcript.is_empty() {
                                    ui.add_space(35.0);
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            egui::RichText::new("No recent transcript.")
                                                .font(FontId::proportional(13.5))
                                                .color(Color32::from_rgb(100, 116, 139)),
                                        );
                                        let empty_hint = format!("Press {} and speak clearly into your mic.", dictate_key_name);
                                        ui.label(
                                            egui::RichText::new(empty_hint)
                                                .font(FontId::proportional(11.0))
                                                .color(Color32::from_rgb(71, 85, 105)),
                                        );
                                    });
                                    ui.add_space(20.0);
                                } else {
                                    ui.label(
                                        egui::RichText::new(shape_thai(&transcript))
                                            .font(FontId::proportional(15.0))
                                            .color(Color32::from_rgb(248, 250, 252)),
                                    );
                                }
                            });

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            if !transcript.is_empty() {
                                let word_count = transcript.split_whitespace().count();
                                let char_count = transcript.chars().count();
                                ui.label(
                                    egui::RichText::new(format!("{} words • {} chars", word_count, char_count))
                                        .font(FontId::proportional(11.0))
                                        .color(Color32::from_rgb(148, 163, 184)),
                                );

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    // Re-paste Button
                                    let paste_btn = egui::Button::new("✍️ Re-paste to Window")
                                        .fill(Color32::from_rgb(49, 46, 129))
                                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(99, 102, 241)))
                                        .rounding(Rounding::same(6.0));

                                    if ui.add(paste_btn).clicked() {
                                        let _ = PlatformManager::paste_text(&transcript);
                                        self.state.set_toast("Re-pasted transcript to active cursor!");
                                    }

                                    ui.add_space(4.0);

                                    // Copy Button
                                    let copy_btn = egui::Button::new("📋 Copy Text")
                                        .fill(Color32::from_rgb(33, 43, 64))
                                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(51, 65, 85)))
                                        .rounding(Rounding::same(6.0));

                                    if ui.add(copy_btn).clicked() {
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            let _ = clipboard.set_text(&transcript);
                                            self.state.set_toast("Copied transcript to clipboard!");
                                        }
                                    }

                                    ui.add_space(4.0);

                                    // Clear Button
                                    let clear_btn = egui::Button::new("🗑️ Clear")
                                        .fill(Color32::from_rgb(33, 43, 64))
                                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(51, 65, 85)))
                                        .rounding(Rounding::same(6.0));

                                    if ui.add(clear_btn).clicked() {
                                        *lock(&self.state.last_transcript) = String::new();
                                        self.state.set_toast("Transcript cleared");
                                    }
                                });
                            }
                        });
                    });

                ui.add_space(10.0);

                // Quick Stats Card
                let history_count = lock(&self.state.history).len();
                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.columns(3, |stat_cols| {
                            stat_cols[0].vertical(|ui| {
                                ui.label(egui::RichText::new("RECORDED").font(FontId::proportional(10.0)).color(Color32::from_rgb(148, 163, 184)));
                                ui.label(egui::RichText::new(format!("{} Entries", history_count)).font(FontId::proportional(14.0)).strong().color(Color32::from_rgb(167, 139, 250)));
                            });
                            stat_cols[1].vertical(|ui| {
                                ui.label(egui::RichText::new("LATENCY").font(FontId::proportional(10.0)).color(Color32::from_rgb(148, 163, 184)));
                                ui.label(egui::RichText::new("~220 ms").font(FontId::proportional(14.0)).strong().color(Color32::from_rgb(52, 211, 153)));
                            });
                            stat_cols[2].vertical(|ui| {
                                ui.label(egui::RichText::new("STORAGE").font(FontId::proportional(10.0)).color(Color32::from_rgb(148, 163, 184)));
                                ui.label(egui::RichText::new("SQLite Local").font(FontId::proportional(14.0)).strong().color(Color32::from_rgb(56, 189, 248)));
                            });
                        });
                    });

                ui.add_space(10.0);

                // Recent Activity Card (Mini History in Studio)
                let recent_items: Vec<_> = lock(&self.state.history).iter().take(3).cloned().collect();
                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("🕒  RECENT ACTIVITY")
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(148, 163, 184))
                                    .strong(),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("View All ➔").clicked() {
                                    self.active_tab = ActiveTab::History;
                                    let _ = self.event_tx.send(GuiEvent::RefreshHistory);
                                }
                            });
                        });
                        ui.add_space(6.0);

                        if recent_items.is_empty() {
                            ui.label(
                                egui::RichText::new("No recent transcriptions yet. Dictate anywhere to see activity.")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(100, 116, 139)),
                            );
                        } else {
                            for item in recent_items {
                                ui.horizontal(|ui| {
                                    let app_label = if item.app.is_empty() { "Desktop" } else { &item.app };
                                    ui.label(
                                        egui::RichText::new(format!("⚡ {}", app_label))
                                            .font(FontId::proportional(10.5))
                                            .color(Color32::from_rgb(56, 189, 248)),
                                    );
                                    let snippet = if item.formatted_text.chars().count() > 32 {
                                        format!("{}...", item.formatted_text.chars().take(30).collect::<String>())
                                    } else {
                                        item.formatted_text.clone()
                                    };
                                    ui.label(
                                        egui::RichText::new(snippet)
                                            .font(FontId::proportional(11.0))
                                            .color(Color32::from_rgb(203, 213, 225)),
                                    );

                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.small_button("📋").on_hover_text("Copy text").clicked() {
                                            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                                let _ = clipboard.set_text(&item.formatted_text);
                                                self.state.set_toast("Copied to clipboard!");
                                            }
                                        }
                                    });
                                });
                                ui.add_space(2.0);
                            }
                        }
                    });
            });
        });
    }

    fn render_history_tab(&mut self, ui: &mut egui::Ui) {
        let history = lock(&self.state.history).clone();

        // 1. Top Summary Banner
        let total_words: usize = history
            .iter()
            .map(|h| h.formatted_text.split_whitespace().count())
            .sum();
        let total_duration: f64 = history.iter().map(|h| h.duration_secs).sum();

        egui::Frame::none()
            .fill(Color32::from_rgb(17, 23, 36))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                ui.columns(3, |stat_cols| {
                    stat_cols[0].horizontal(|ui| {
                        ui.label("📊");
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Total Transcriptions")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.label(
                                egui::RichText::new(format!("{} recorded", history.len()))
                                    .font(FontId::proportional(14.0))
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                        });
                    });
                    stat_cols[1].horizontal(|ui| {
                        ui.label("📝");
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Dictated Words")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.label(
                                egui::RichText::new(format!("{} words", total_words))
                                    .font(FontId::proportional(14.0))
                                    .strong()
                                    .color(Color32::from_rgb(167, 139, 250)),
                            );
                        });
                    });
                    stat_cols[2].horizontal(|ui| {
                        ui.label("⏱️");
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Total Voice Time")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.label(
                                egui::RichText::new(format!("{:.1} seconds", total_duration))
                                    .font(FontId::proportional(14.0))
                                    .strong()
                                    .color(Color32::from_rgb(52, 211, 153)),
                            );
                        });
                    });
                });
            });

        ui.add_space(8.0);

        // 2. Search Toolbar
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Search transcripts by keywords or application name...")
                    .desired_width(ui.available_width() - 120.0),
            );

            let refresh_btn = egui::Button::new("🔄 Refresh")
                .fill(Color32::from_rgb(33, 44, 66))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(51, 65, 85)))
                .rounding(Rounding::same(6.0));
            if ui.add(refresh_btn).clicked() {
                let _ = self.event_tx.send(GuiEvent::RefreshHistory);
                self.state.set_toast("History records refreshed!");
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        let query = self.search_query.to_lowercase();
        let filtered: Vec<&HistoryRecord> = history
            .iter()
            .filter(|h| {
                if query.is_empty() {
                    true
                } else {
                    h.formatted_text.to_lowercase().contains(&query)
                        || h.app.to_lowercase().contains(&query)
                }
            })
            .collect();

        if filtered.is_empty() {
            ui.add_space(45.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("No matching dictation records found.")
                        .font(FontId::proportional(14.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                );
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for rec in filtered {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(17, 23, 36))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(12.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let app_name = if rec.app.is_empty() {
                                    "Desktop".to_string()
                                } else {
                                    rec.app.clone()
                                };

                                let badge = ui.label(
                                    egui::RichText::new(format!("⚡ {}", app_name))
                                        .font(FontId::proportional(11.0))
                                        .color(Color32::from_rgb(56, 189, 248))
                                        .strong(),
                                );
                                ui.painter().rect_filled(
                                    badge.rect.expand(3.0),
                                    Rounding::same(4.0),
                                    Color32::from_rgba_unmultiplied(14, 165, 233, 30),
                                );

                                if rec.duration_secs > 0.0 {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "⏱️ {:.1}s",
                                            rec.duration_secs
                                        ))
                                        .font(FontId::proportional(11.0))
                                        .color(Color32::from_rgb(148, 163, 184)),
                                    );
                                }

                                if !rec.timestamp.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&rec.timestamp)
                                            .font(FontId::proportional(11.0))
                                            .color(Color32::from_rgb(100, 116, 139)),
                                    );
                                }

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let del_btn = egui::Button::new("🗑️")
                                            .fill(Color32::from_rgb(60, 20, 20))
                                            .rounding(Rounding::same(4.0));
                                        if ui.add(del_btn).clicked() {
                                            let _ = self.event_tx.send(GuiEvent::DeleteHistory(
                                                rec.transcript_id.clone(),
                                            ));
                                            self.state.set_toast("Record deleted");
                                        }

                                        ui.add_space(4.0);

                                        let paste_btn = egui::Button::new("✍️ Paste")
                                            .fill(Color32::from_rgb(49, 46, 129))
                                            .rounding(Rounding::same(4.0));
                                        if ui.add(paste_btn).clicked() {
                                            let _ =
                                                PlatformManager::paste_text(&rec.formatted_text);
                                            self.state.set_toast("Pasted into active window!");
                                        }

                                        ui.add_space(4.0);

                                        let copy_btn = egui::Button::new("📋 Copy")
                                            .fill(Color32::from_rgb(33, 44, 66))
                                            .rounding(Rounding::same(4.0));
                                        if ui.add(copy_btn).clicked() {
                                            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                                let _ = clipboard.set_text(&rec.formatted_text);
                                                self.state.set_toast("Copied to clipboard!");
                                            }
                                        }
                                    },
                                );
                            });

                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(shape_thai(&rec.formatted_text))
                                    .font(FontId::proportional(14.0))
                                    .color(Color32::from_rgb(241, 245, 249)),
                            );
                        });

                    ui.add_space(8.0);
                }
            });
        }
    }

    fn render_dictionary_tab(&mut self, ui: &mut egui::Ui) {
        let entries = lock(&self.state.dictionary_entries).clone();

        egui::ScrollArea::vertical().show(ui, |ui| {
            // CARD 1: Introduction & Add New Entry Form
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("📖  CUSTOM VOCABULARY & TEXT SNIPPET EXPANSIONS")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Teach Wflow custom terminology, names, or define instant trigger snippets. When you speak a trigger phrase, it will automatically be expanded into your configured replacement text.")
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label("Word or Trigger Phrase:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dict_word_input)
                                    .hint_text("e.g. Wflow, myemail, BRB, หมอพร้อม")
                                    .desired_width(240.0),
                            );
                        });

                        ui.add_space(8.0);

                        ui.vertical(|ui| {
                            ui.label("Snippet Replacement (Optional):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dict_replacement_input)
                                    .hint_text("e.g. contact@wflow.ai (leave blank for vocabulary only)")
                                    .desired_width(300.0),
                            );
                        });

                        ui.add_space(8.0);

                        ui.vertical(|ui| {
                            ui.label(" ");
                            let add_btn = egui::Button::new(
                                egui::RichText::new("➕ Add Entry")
                                    .font(FontId::proportional(12.5))
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(Color32::from_rgb(79, 70, 229))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(129, 140, 248)))
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(110.0, 28.0));

                            if ui.add(add_btn).clicked() {
                                let word = self.dict_word_input.trim().to_string();
                                if !word.is_empty() {
                                    let repl = if self.dict_replacement_input.trim().is_empty() {
                                        None
                                    } else {
                                        Some(self.dict_replacement_input.trim().to_string())
                                    };
                                    let _ = self.event_tx.send(GuiEvent::AddDictionaryEntry {
                                        word: word.clone(),
                                        replacement: repl,
                                    });
                                    self.dict_word_input.clear();
                                    self.dict_replacement_input.clear();
                                    self.state.set_toast(&format!("Added '{}' to dictionary", word));
                                }
                            }
                        });
                    });
                });

            ui.add_space(10.0);

            // Filter & Entries List
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("All Entries ({})", entries.len()))
                        .font(FontId::proportional(13.0))
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.dict_search_query)
                            .hint_text("🔍 Filter dictionary...")
                            .desired_width(200.0),
                    );
                });
            });

            ui.add_space(8.0);

            let filter = self.dict_search_query.trim().to_lowercase();
            let filtered_entries: Vec<_> = entries
                .iter()
                .filter(|e| {
                    if filter.is_empty() {
                        true
                    } else {
                        e.word.to_lowercase().contains(&filter)
                            || e.replacement
                                .as_ref()
                                .map(|r| r.to_lowercase().contains(&filter))
                                .unwrap_or(false)
                    }
                })
                .collect();

            if filtered_entries.is_empty() {
                egui::Frame::none()
                    .fill(Color32::from_rgb(15, 20, 32))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(26, 33, 50)))
                    .rounding(Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(24.0))
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new("No dictionary entries found.")
                                    .font(FontId::proportional(13.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.label(
                                egui::RichText::new("Add custom words or text expansion snippets above!")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(100, 116, 139)),
                            );
                        });
                    });
            } else {
                for entry in filtered_entries {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(17, 23, 36))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 41, 64)))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let is_snippet = entry.replacement.as_ref().map(|r| !r.trim().is_empty()).unwrap_or(false);
                                if is_snippet {
                                    let badge = ui.label(
                                        egui::RichText::new("⚡ SNIPPET")
                                            .font(FontId::monospace(10.5))
                                            .color(Color32::from_rgb(192, 132, 252))
                                            .strong(),
                                    );
                                    ui.painter().rect_filled(
                                        badge.rect.expand(2.5),
                                        Rounding::same(4.0),
                                        Color32::from_rgba_unmultiplied(168, 85, 247, 30),
                                    );
                                } else {
                                    let badge = ui.label(
                                        egui::RichText::new("📚 VOCAB")
                                            .font(FontId::monospace(10.5))
                                            .color(Color32::from_rgb(56, 189, 248))
                                            .strong(),
                                    );
                                    ui.painter().rect_filled(
                                        badge.rect.expand(2.5),
                                        Rounding::same(4.0),
                                        Color32::from_rgba_unmultiplied(14, 165, 233, 30),
                                    );
                                }

                                ui.add_space(6.0);

                                ui.label(
                                    egui::RichText::new(shape_thai(&entry.word))
                                        .font(FontId::proportional(14.0))
                                        .color(Color32::WHITE)
                                        .strong(),
                                );

                                if let Some(ref repl) = entry.replacement {
                                    if !repl.trim().is_empty() {
                                        ui.label(
                                            egui::RichText::new("➔")
                                                .font(FontId::proportional(12.0))
                                                .color(Color32::from_rgb(148, 163, 184)),
                                        );
                                        ui.label(
                                            egui::RichText::new(shape_thai(repl))
                                                .font(FontId::proportional(13.5))
                                                .color(Color32::from_rgb(203, 213, 225)),
                                        );
                                    }
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let del_btn = egui::Button::new("🗑️ Delete")
                                        .fill(Color32::from_rgb(45, 25, 30))
                                        .rounding(Rounding::same(4.0));
                                    if ui.add(del_btn).clicked() {
                                        let _ = self.event_tx.send(GuiEvent::DeleteDictionaryEntry(entry.id.clone()));
                                        self.state.set_toast("Entry removed");
                                    }
                                });
                            });
                        });
                    ui.add_space(4.0);
                }
            }

            ui.add_space(14.0);
        });
    }

    fn render_updates_tab(&mut self, ui: &mut egui::Ui) {
        let current = crate::update::current_version();
        let pending = self.pending_update();
        let running = self.state.update_check_running.load(Ordering::SeqCst);
        let error = lock(&self.state.update_error).clone();
        let last_check = lock(&self.state.config).last_update_check.clone();

        egui::Frame::none()
            .fill(Color32::from_rgb(17, 23, 36))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::same(18.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("🔄  Wflow Software Update")
                            .font(FontId::proportional(14.0))
                            .color(Color32::WHITE)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("v{}", current))
                            .font(FontId::monospace(13.0))
                            .color(Color32::from_rgb(56, 189, 248))
                            .strong(),
                    );
                });

                ui.add_space(14.0);

                // --- status line -------------------------------------------------
                let (status_text, status_color, status_fill, status_stroke) = if running {
                    (
                        "⏳ กำลังตรวจสอบอัปเดต...".to_string(),
                        Color32::from_rgb(203, 213, 225),
                        Color32::from_rgb(30, 41, 59),
                        Color32::from_rgb(71, 85, 105),
                    )
                } else if let Some(info) = pending.as_ref() {
                    (
                        format!("🎉 มีเวอร์ชันใหม่ v{}", info.latest),
                        Color32::WHITE,
                        Color32::from_rgb(22, 163, 74),
                        Color32::from_rgb(74, 222, 128),
                    )
                } else if let Some(err) = error.as_ref() {
                    (
                        format!("⚠️ ตรวจสอบไม่สำเร็จ: {}", err),
                        Color32::from_rgb(252, 165, 165),
                        Color32::from_rgb(45, 18, 22),
                        Color32::from_rgb(239, 68, 68),
                    )
                } else {
                    (
                        "✅ เป็นเวอร์ชันล่าสุดแล้ว".to_string(),
                        Color32::from_rgb(134, 239, 172),
                        Color32::from_rgb(12, 38, 28),
                        Color32::from_rgb(34, 197, 94),
                    )
                };

                let status_pill = ui.label(
                    egui::RichText::new(shape_thai(&status_text))
                        .font(FontId::proportional(13.0))
                        .color(status_color)
                        .strong(),
                );
                let pill_rect = status_pill.rect.expand(8.0);
                ui.painter()
                    .rect_filled(pill_rect, Rounding::same(10.0), status_fill);
                ui.painter().rect_stroke(
                    pill_rect,
                    Rounding::same(10.0),
                    Stroke::new(1.0_f32, status_stroke),
                );

                ui.add_space(14.0);

                // --- actions -----------------------------------------------------
                ui.horizontal(|ui| {
                    let check_label = if running {
                        "⏳ กำลังตรวจสอบ..."
                    } else {
                        "🔍 ตรวจสอบอัปเดตเดี๋ยวนี้"
                    };
                    let check_btn = egui::Button::new(
                        egui::RichText::new(shape_thai(check_label))
                            .font(FontId::proportional(12.5))
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(14, 116, 144))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(6, 182, 212)))
                    .rounding(Rounding::same(8.0))
                    .min_size(Vec2::new(190.0, 36.0));

                    if ui.add_enabled(!running, check_btn).clicked() {
                        let _ = self.event_tx.send(GuiEvent::CheckUpdate);
                        self.state.set_toast("กำลังตรวจสอบอัปเดต...");
                    }

                    if let Some(info) = pending.as_ref() {
                        let dl_btn = egui::Button::new(
                            egui::RichText::new(format!("⬇️ ดาวน์โหลด v{}", info.latest))
                                .font(FontId::proportional(12.5))
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(Color32::from_rgb(22, 163, 74))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(74, 222, 128)))
                        .rounding(Rounding::same(8.0))
                        .min_size(Vec2::new(170.0, 36.0));

                        if ui.add(dl_btn).clicked() {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::OpenUrl(info.download_url.clone()));
                        }
                    }

                    if ui
                        .button("🌐 เปิดหน้า Releases")
                        .on_hover_text("เปิดหน้า GitHub Releases ในเบราว์เซอร์")
                        .clicked()
                    {
                        let _ = self.event_tx.send(GuiEvent::OpenUrl(format!(
                            "https://github.com/{}/releases",
                            crate::update::DEFAULT_REPO
                        )));
                    }
                });

                ui.add_space(12.0);

                // --- release notes ------------------------------------------------
                if let Some(info) = pending.as_ref() {
                    ui.label(
                        egui::RichText::new(format!("📝 {}", info.title))
                            .font(FontId::proportional(12.5))
                            .color(Color32::WHITE)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    let notes = if info.notes.trim().is_empty() {
                        "(ไม่มีรายละเอียดเพิ่มเติม)".to_string()
                    } else {
                        info.notes.clone()
                    };
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(notes)
                                    .font(FontId::proportional(11.5))
                                    .color(Color32::from_rgb(203, 213, 225)),
                            );
                        });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("📝 ดู Release Notes บน GitHub").clicked() {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::OpenUrl(info.release_url.clone()));
                        }
                        if ui
                            .button("ข้ามเวอร์ชันนี้")
                            .on_hover_text("ไม่แจ้งเตือนเวอร์ชันนี้อีก")
                            .clicked()
                        {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::SkipUpdateVersion(info.latest.to_string()));
                            self.state.set_toast("ข้ามเวอร์ชันนี้แล้ว");
                        }
                    });
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // --- details ------------------------------------------------------
                ui.label(
                    egui::RichText::new("รายละเอียด")
                        .font(FontId::proportional(12.0))
                        .color(Color32::from_rgb(148, 163, 184))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("• เวอร์ชันปัจจุบัน: v{}", current))
                        .font(FontId::proportional(11.5))
                        .color(Color32::from_rgb(203, 213, 225)),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "• ตรวจสอบล่าสุด: {}",
                        last_check.unwrap_or_else(|| "ยังไม่เคยตรวจสอบ".to_string())
                    ))
                    .font(FontId::proportional(11.5))
                    .color(Color32::from_rgb(203, 213, 225)),
                );
                ui.label(
                    egui::RichText::new(
                        "• ตรวจสอบอัตโนมัติทุก 15 นาที และเมื่อเปิดโปรแกรม (ตั้งค่าได้ใน Settings)",
                    )
                    .font(FontId::proportional(11.5))
                    .color(Color32::from_rgb(203, 213, 225)),
                );
            });
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            // CARD 1: Hotkeys & Trigger Preferences
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("⌨️  GLOBAL KEYBOARD SHORTCUTS")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // 1. Push-To-Talk Dictation Hotkey
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("🎙️ Push-To-Talk Dictation Hotkey:")
                                    .font(FontId::proportional(13.0))
                                    .strong(),
                            );
                            let curr_name = self.settings_hotkey.format_display();
                            let badge = ui.label(
                                egui::RichText::new(format!(" {} ", curr_name))
                                    .font(FontId::monospace(12.0))
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .strong(),
                            );
                            ui.painter().rect_filled(
                                badge.rect.expand(2.0),
                                Rounding::same(4.0),
                                Color32::from_rgba_unmultiplied(14, 165, 233, 40),
                            );
                        });
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new("Hold this key or combination anywhere in Windows to record voice, release to insert text.")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let curr_label = format!("Preset: {}", self.settings_hotkey.format_display());
                            egui::ComboBox::from_id_salt("hotkey_combo")
                                .selected_text(curr_label)
                                .show_ui(ui, |ui| {
                                    for (name, combo) in POPULAR_HOTKEYS {
                                        if ui.selectable_label(self.settings_hotkey == *combo, *name).clicked() {
                                            self.settings_hotkey = *combo;
                                        }
                                    }
                                });

                            let is_rec = self.recording_mode == Some(1);
                            let rec_text = if is_rec {
                                "🔴 Listening for shortcut combination... (Esc to cancel)"
                            } else {
                                "⌨️ Press any key or combination to set"
                            };
                            let rec_fill = if is_rec {
                                Color32::from_rgb(225, 29, 72)
                            } else {
                                Color32::from_rgb(45, 58, 86)
                            };

                            let rec_btn = egui::Button::new(
                                egui::RichText::new(rec_text)
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::WHITE),
                            )
                            .fill(rec_fill)
                            .rounding(Rounding::same(6.0));

                            if ui.add(rec_btn).clicked() {
                                if is_rec {
                                    self.recording_mode = None;
                                    PlatformManager::stop_recording_hotkey();
                                    self.state.set_toast("Key recording cancelled");
                                } else {
                                    self.recording_mode = Some(1);
                                    PlatformManager::start_recording_hotkey(1);
                                    self.state.set_toast("Press any combination (e.g. Ctrl+Space, Alt+Space, Ctrl+Shift+D) to set");
                                }
                            }
                        });
                    });

                    ui.add_space(8.0);

                    // 2. Toggle Window Hotkey
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("🪟 Toggle Window Hotkey:")
                                    .font(FontId::proportional(13.0))
                                    .strong(),
                            );
                            let curr_name = self.settings_toggle.format_display();
                            let badge = ui.label(
                                egui::RichText::new(format!(" {} ", curr_name))
                                    .font(FontId::monospace(12.0))
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .strong(),
                            );
                            ui.painter().rect_filled(
                                badge.rect.expand(2.0),
                                Rounding::same(4.0),
                                Color32::from_rgba_unmultiplied(14, 165, 233, 40),
                            );
                        });
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new("Press this key or combination anywhere to toggle Wflow Studio between screen and system tray.")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let curr_label = format!("Preset: {}", self.settings_toggle.format_display());
                            egui::ComboBox::from_id_salt("toggle_combo")
                                .selected_text(curr_label)
                                .show_ui(ui, |ui| {
                                    for (name, combo) in POPULAR_HOTKEYS {
                                        if ui.selectable_label(self.settings_toggle == *combo, *name).clicked() {
                                            self.settings_toggle = *combo;
                                        }
                                    }
                                });

                            let is_rec = self.recording_mode == Some(2);
                            let rec_text = if is_rec {
                                "🔴 Listening for shortcut combination... (Esc to cancel)"
                            } else {
                                "⌨️ Press any key or combination to set"
                            };
                            let rec_fill = if is_rec {
                                Color32::from_rgb(225, 29, 72)
                            } else {
                                Color32::from_rgb(45, 58, 86)
                            };

                            let rec_btn = egui::Button::new(
                                egui::RichText::new(rec_text)
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::WHITE),
                            )
                            .fill(rec_fill)
                            .rounding(Rounding::same(6.0));

                            if ui.add(rec_btn).clicked() {
                                if is_rec {
                                    self.recording_mode = None;
                                    PlatformManager::stop_recording_hotkey();
                                    self.state.set_toast("Key recording cancelled");
                                } else {
                                    self.recording_mode = Some(2);
                                    PlatformManager::start_recording_hotkey(2);
                                    self.state.set_toast("Press any combination (e.g. Alt+Space, Ctrl+Space, F9) to set");
                                }
                            }
                        });
                    });

                    ui.add_space(8.0);

                    // Dictation Trigger Mode (Push-to-Talk vs Tap-to-Toggle)
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("🎙️ Dictation Trigger Mode:")
                                    .font(FontId::proportional(13.0))
                                    .strong(),
                            );
                            let mode_label = if self.settings_recording_trigger_mode == "tap_to_toggle" {
                                "Tap to Toggle (Hands-Free)"
                            } else {
                                "Push-to-Talk (Hold Key)"
                            };
                            let badge = ui.label(
                                egui::RichText::new(format!(" {} ", mode_label))
                                    .font(FontId::monospace(11.5))
                                    .color(Color32::from_rgb(165, 180, 252))
                                    .strong(),
                            );
                            ui.painter().rect_filled(
                                badge.rect.expand(2.0),
                                Rounding::same(4.0),
                                Color32::from_rgba_unmultiplied(99, 102, 241, 40),
                            );
                        });
                        ui.add_space(3.0);
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut self.settings_recording_trigger_mode,
                                "push_to_talk".to_string(),
                                "🖐️ Hold key to talk (Push-to-Talk)",
                            );
                            ui.selectable_value(
                                &mut self.settings_recording_trigger_mode,
                                "tap_to_toggle".to_string(),
                                "🔄 Tap once to start / tap to stop (Hands-Free)",
                            );
                        });
                        ui.label(
                            egui::RichText::new(if self.settings_recording_trigger_mode == "tap_to_toggle" {
                                "Tap your hotkey once to start dictating freely. Tap again to stop and automatically paste into your active window."
                            } else {
                                "Hold down your hotkey while speaking. Releasing the key immediately commits and types the text."
                            })
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });

                    ui.add_space(10.0);

                    // Auto Paste Checkbox
                    ui.checkbox(
                        &mut self.settings_auto_paste,
                        "Auto-paste transcribed text directly into active cursor position",
                    );

                    ui.add_space(8.0);

                    // Auto Start Checkbox
                    let cb = ui.checkbox(
                        &mut self.settings_auto_start,
                        "Start Wflow automatically on system startup (Run on Startup)",
                    );
                    if cb.changed() {
                        let _ = PlatformManager::set_autostart(self.settings_auto_start);
                        if let Ok(mut lock) = self.state.config.lock() {
                            lock.auto_start = self.settings_auto_start;
                            let _ = lock.save(None);
                        }
                        let msg = if self.settings_auto_start {
                            "Auto-start on Windows startup enabled"
                        } else {
                            "Auto-start on Windows startup disabled"
                        };
                        self.state.set_toast(msg);
                    }
                    ui.label(
                        egui::RichText::new("  └ Launches Wflow automatically when your system boots.")
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(8.0);

                    // Close to Tray Checkbox
                    ui.checkbox(
                        &mut self.settings_close_to_tray,
                        shape_thai("ย่อลง System Tray เมื่อกดปุ่ม [X] (Minimize to tray on close)"),
                    );
                    ui.label(
                        egui::RichText::new(shape_thai("  └ หากไม่เลือก จะแสดงหน้าต่างยืนยันการปิดโปรแกรมเมื่อกดปุ่ม [X]"))
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                });

            ui.add_space(10.0);

            // CARD 1.5: Microphone & Sound Effects Hardware
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("🎤  MICROPHONE HARDWARE & AUDIO SOUND EFFECTS")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // Microphone Device Selector
                    ui.horizontal(|ui| {
                        ui.label("Input Microphone:");
                        let current_dev_label = self
                            .settings_input_device_name
                            .as_deref()
                            .unwrap_or("Default System Microphone");

                        egui::ComboBox::from_id_salt("mic_device_combo")
                            .selected_text(current_dev_label)
                            .width(340.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(self.settings_input_device_name.is_none(), "Default System Microphone").clicked() {
                                    self.settings_input_device_name = None;
                                }
                                for dev in &self.available_devices {
                                    let is_sel = self.settings_input_device_name.as_deref() == Some(dev.as_str());
                                    if ui.selectable_label(is_sel, dev).clicked() {
                                        self.settings_input_device_name = Some(dev.clone());
                                    }
                                }
                            });

                        if ui.button("🔄 Refresh Devices").clicked() {
                            self.available_devices = crate::audio::list_input_devices();
                            self.state.set_toast("Audio input devices refreshed");
                        }
                    });

                    ui.add_space(10.0);

                    // Sound Effects toggle & Test chime button
                    ui.horizontal(|ui| {
                        let se_cb = ui.checkbox(
                            &mut self.settings_sound_effects,
                            "Enable Synthesized Audio Earcons (Start/Finish/Error sound effects)",
                        );
                        if se_cb.changed() {
                            crate::sound::set_sound_enabled(self.settings_sound_effects);
                        }

                        ui.add_space(8.0);
                        let test_sound_btn = egui::Button::new("🔊 Test Chime")
                            .fill(Color32::from_rgb(45, 58, 86))
                            .rounding(Rounding::same(6.0));
                        if ui.add(test_sound_btn).clicked() {
                            crate::sound::play_sound(crate::sound::SoundEffect::StartRecording);
                        }
                    });
                    ui.label(
                        egui::RichText::new("  └ Soft pure-Rust synthesized harmonic chimes provide immediate tactile feedback when dictation starts and finishes.")
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                });

            ui.add_space(10.0);

            // CARD 1.8: AI Post-Processing & Smart Formatting
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("✨  AI POST-PROCESSING & SMART FORMATTING")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    ui.checkbox(
                        &mut self.settings_ai_formatting,
                        "Enable Instant AI Text Formatting & Cleanup (Groq Llama 3.1 8B ~150ms)",
                    );
                    ui.label(
                        egui::RichText::new("  └ Automatically cleans speech disfluencies, filler words (um, uh, อ่า, แบบว่า), fixes punctuation, and adapts writing tone.")
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    if self.settings_ai_formatting {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("Writing Style Tone:");
                            let style_label = match self.settings_writing_style.as_str() {
                                "smart_cleanup" => "Smart Cleanup (Removes stutters, filler words, fixes punctuation)",
                                "casual" => "Casual & Friendly (Chat and messaging tone)",
                                "professional" => "Professional & Executive (Polite corporate emails & reports)",
                                "code" => "Code & CLI Commands (Converts speech to code / terminal syntax)",
                                "bullet_points" => "Bullet Points (Summary in structured markdown bullets)",
                                other => other,
                            };
                            egui::ComboBox::from_id_salt("writing_style_combo")
                                .selected_text(style_label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "smart_cleanup".to_string(),
                                        "Smart Cleanup (Removes stutters, filler words, fixes punctuation) [Recommended]",
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "casual".to_string(),
                                        "Casual & Friendly (Chat and messaging tone)",
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "professional".to_string(),
                                        "Professional & Executive (Polite corporate emails & reports)",
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "code".to_string(),
                                        "Code & CLI Commands (Converts speech to code / terminal syntax)",
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "bullet_points".to_string(),
                                        "Bullet Points (Summary in structured markdown bullets)",
                                    );
                                });
                        });
                    }
                });

            ui.add_space(10.0);

            // CARD 2: STT AI Service Providers
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("🤖  SPEECH-TO-TEXT (STT) AI SERVICE PROVIDER")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // 3 Provider Cards
                    let groq_desc = format!("Model: {}", self.settings_groq_model);
                    let eleven_desc = format!("Model: {}", self.settings_elevenlabs_model);
                    let providers = [
                        (SttProvider::WisprFlow, "⚡ Wispr Flow", "Native gRPC Protobuf streaming".to_string(), Color32::from_rgb(99, 102, 241)),
                        (SttProvider::Groq, "🚀 Groq Whisper", groq_desc, Color32::from_rgb(249, 115, 22)),
                        (SttProvider::ElevenLabs, "🎙️ ElevenLabs", eleven_desc, Color32::from_rgb(168, 85, 247)),
                    ];

                    ui.columns(3, |pcols| {
                        for (idx, (p, title, desc, color)) in providers.iter().enumerate() {
                            let is_sel = self.settings_stt_provider == *p;
                            pcols[idx].vertical(|ui| {
                                let card_bg = if is_sel { Color32::from_rgb(26, 34, 52) } else { Color32::from_rgb(15, 20, 32) };
                                let card_stroke = if is_sel { Stroke::new(1.8_f32, *color) } else { Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)) };
                                let card_resp = egui::Frame::none()
                                    .fill(card_bg)
                                    .stroke(card_stroke)
                                    .rounding(Rounding::same(8.0))
                                    .inner_margin(egui::Margin::same(10.0))
                                    .show(ui, |ui| {
                                        let check = if is_sel { "🔘 " } else { "⚪ " };
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{}{}", check, title))
                                                    .font(FontId::proportional(13.0))
                                                    .color(if is_sel { Color32::WHITE } else { Color32::from_rgb(203, 213, 225) })
                                                    .strong(),
                                            );
                                        });
                                        ui.add_space(4.0);
                                        ui.label(
                                            egui::RichText::new(desc)
                                                .font(FontId::proportional(10.5))
                                                .color(Color32::from_rgb(148, 163, 184)),
                                        );
                                    });

                                if card_resp.response.interact(egui::Sense::click()).clicked() {
                                    self.settings_stt_provider = *p;
                                }
                            });
                        }
                    });

                    ui.add_space(12.0);

                    match self.settings_stt_provider {
                        SttProvider::WisprFlow => {
                            ui.label("API Endpoint URL:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_endpoint)
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);

                            ui.label("Bearer Auth Token (Optional / Session Token):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_token)
                                    .password(true)
                                    .hint_text("Auto-detected from local Wispr Flow session if installed")
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);
                            ui.label("Baseten Gateway API Key (optional, stored securely):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_baseten_api_key)
                                    .password(true)
                                    .hint_text("Api-Key ... or set WFLOW_BASETEN_API_KEY")
                                    .desired_width(ui.available_width()),
                            );
                            ui.label(
                                egui::RichText::new(format!("Model ID: {}", self.settings_baseten_model))
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }
                        SttProvider::Groq => {
                            ui.label("Groq Cloud API Key (get from console.groq.com):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_groq_api_key)
                                    .password(true)
                                    .hint_text("gsk_...")
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);
                            ui.label("Groq Whisper Model:");
                            let groq_selected_label = match self.settings_groq_model.as_str() {
                                "whisper-large-v3" => "whisper-large-v3 (Full accuracy, multilingual)",
                                "whisper-large-v3-turbo" => "whisper-large-v3-turbo (Ultra-fast LPU, multilingual)",
                                "distil-whisper-large-v3-en" => "distil-whisper-large-v3-en (English only, ultra-light)",
                                other => other,
                            };
                            egui::ComboBox::from_id_salt("groq_model_combo")
                                .selected_text(groq_selected_label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.settings_groq_model,
                                        "whisper-large-v3-turbo".to_string(),
                                        "whisper-large-v3-turbo (Ultra-fast LPU, multilingual) [Recommended]",
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_groq_model,
                                        "whisper-large-v3".to_string(),
                                        "whisper-large-v3 (Full accuracy, multilingual)",
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_groq_model,
                                        "distil-whisper-large-v3-en".to_string(),
                                        "distil-whisper-large-v3-en (English only, ultra-light)",
                                    );
                                });
                            ui.label(
                                egui::RichText::new("⚡ Groq executes Whisper on LPUs with near-zero latency (~200ms per utterance).")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }
                        SttProvider::ElevenLabs => {
                            ui.label("ElevenLabs API Key (get from elevenlabs.io):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_elevenlabs_api_key)
                                    .password(true)
                                    .hint_text("xi-api-key")
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);
                            ui.label("ElevenLabs STT Model:");
                            let eleven_selected_label = match self.settings_elevenlabs_model.as_str() {
                                "scribe_v1" => "scribe_v1 (State-of-the-art multilingual STT)",
                                other => other,
                            };
                            egui::ComboBox::from_id_salt("elevenlabs_model_combo")
                                .selected_text(eleven_selected_label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.settings_elevenlabs_model,
                                        "scribe_v1".to_string(),
                                        "scribe_v1 (State-of-the-art multilingual STT) [Recommended]",
                                    );
                                });
                            ui.label(
                                egui::RichText::new("🎙️ ElevenLabs Scribe STT provides unmatched speech recognition accuracy across languages.")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }
                    }

                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let in_progress = self.stt_test_in_progress.load(Ordering::SeqCst);
                    ui.horizontal(|ui| {
                        let btn_label = if in_progress {
                            "⏳ กำลังทดสอบการเชื่อมต่อ STT..."
                        } else {
                            "🧪 ทดสอบการเชื่อมต่อ STT (Test STT Provider)"
                        };
                        let test_btn = egui::Button::new(
                            egui::RichText::new(shape_thai(btn_label))
                                .font(FontId::proportional(12.0))
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(Color32::from_rgb(14, 116, 144))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(6, 182, 212)))
                        .rounding(Rounding::same(6.0))
                        .min_size(Vec2::new(180.0, 32.0));

                        if ui.add_enabled(!in_progress, test_btn).clicked() {
                            self.run_stt_test();
                        }

                        if in_progress {
                            ui.spinner();
                        }
                    });

                    // Show test result banner if available
                    if let Ok(lock) = self.stt_test_status.lock() {
                        if let Some(ref res) = *lock {
                            ui.add_space(8.0);
                            match res {
                                Ok(msg) => {
                                    egui::Frame::none()
                                        .fill(Color32::from_rgb(12, 38, 28))
                                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(34, 197, 94)))
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                                        .show(ui, |ui| {
                                            ui.label(
                                                egui::RichText::new(shape_thai(&format!("✅ {}", msg)))
                                                    .font(FontId::proportional(12.0))
                                                    .color(Color32::from_rgb(134, 239, 172)),
                                            );
                                        });
                                }
                                Err(err_msg) => {
                                    egui::Frame::none()
                                        .fill(Color32::from_rgb(45, 18, 22))
                                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(239, 68, 68)))
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                                        .show(ui, |ui| {
                                            ui.label(
                                                egui::RichText::new(shape_thai(&format!("❌ {}", err_msg)))
                                                    .font(FontId::proportional(12.0))
                                                    .color(Color32::from_rgb(252, 165, 165)),
                                            );
                                        });
                                }
                            }
                        }
                    }
                });

            ui.add_space(10.0);

            // CARD 3: Language & Floating Overlay
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("🌐  LANGUAGE & FLOATING OVERLAY HUD")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // Language Selector
                    ui.horizontal(|ui| {
                        ui.label("Transcription Language:");
                        let current_label = LANGUAGE_OPTIONS
                            .iter()
                            .find(|(_, code)| *code == self.settings_language)
                            .map(|(name, _)| *name)
                            .unwrap_or("English (US)");

                        egui::ComboBox::from_id_salt("lang_combo")
                            .selected_text(current_label)
                            .show_ui(ui, |ui| {
                                for (name, code) in LANGUAGE_OPTIONS {
                                    if ui.selectable_label(self.settings_language == *code, *name).clicked() {
                                        self.settings_language = code.to_string();
                                    }
                                }
                            });
                    });

                    ui.add_space(10.0);

                    // Floating HUD Checkbox
                    ui.checkbox(
                        &mut self.settings_show_floating_hud,
                        "Show floating recording symbol & live waveform HUD (overlay)",
                    );
                    ui.label(
                        egui::RichText::new("  └ Displays an on-screen pill indicator with animated audio waveform bars when dictating without stealing focus.")
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                });

            ui.add_space(14.0);

            // CARD 4: Voice filter & text quality
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("🎚️  VOICE FILTER & TEXT QUALITY")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // --- Silence filter -------------------------------------------------
                    ui.checkbox(
                        &mut self.settings_silence_filter,
                        "กรองเสียงเงียบ (Silence filter / noise gate)",
                    );
                    ui.label(
                        egui::RichText::new(
                            "  └ ตัดเสียงเงียบและเสียงรบกวนออกก่อนส่งให้โมเดล STT ลดการถอดผิดและประหยัดเน็ต",
                        )
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("ความไวต่อเสียงพูด:");
                        ui.add(
                            egui::DragValue::new(&mut self.settings_silence_sensitivity)
                                .range(0.0..=1.0)
                                .speed(0.02)
                                .suffix(""),
                        );
                        ui.label(
                            egui::RichText::new(if self.settings_silence_sensitivity > 0.66 {
                                "ไวมาก"
                            } else if self.settings_silence_sensitivity > 0.33 {
                                "ปกติ"
                            } else {
                                "ไวน้อย (เก็บเสียงพูดเบาๆ)"
                            })
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("หยุดอัตโนมัติเมื่อเงียบนาน:");
                        ui.add(
                            egui::DragValue::new(&mut self.settings_silence_stop_secs)
                                .range(0..=600)
                                .speed(1.0)
                                .suffix(" วิ"),
                        );
                        if self.settings_silence_stop_secs == 0 {
                            ui.label(
                                egui::RichText::new("(0 = ไม่หยุดอัตโนมัติ)")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }
                    });

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // --- Hallucination guard ---------------------------------------------
                    ui.checkbox(
                        &mut self.settings_hallucination_guard,
                        "กันโมเดลพิมพ์มั่ว (Hallucination guard)",
                    );
                    ui.label(
                        egui::RichText::new(
                            "  └ ตรวจจับข้อความปลอมแปลงที่โมเดลมักแต่งขึ้นเอง เช่น \"Thank you.\" \"(upbeat music)\" หรือคำที่ซ้ำยาว ๆ แล้วไม่แทรกลงในแอป",
                        )
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // --- Spelling correction ---------------------------------------------
                    ui.checkbox(
                        &mut self.settings_autocorrect_enabled,
                        "สะกดคำอัตโนมัติตามภาษา (Automatic spell check)",
                    );
                    ui.checkbox(
                        &mut self.settings_mixed_spacing,
                        "เติมช่องว่างระหว่างคำไทยกับอังกฤษ (Mixed-language spacing)",
                    );
                    ui.label(
                        egui::RichText::new(
                            "  └ เช่น \"ทดสอบAPI\" → \"ทดสอบ API\" และแก้คำพิมพ์ผิดที่พบบ่อยในแต่ละภาษา",
                        )
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut self.settings_autocorrect_use_llm,
                        "ใช้โมเดลภาษาช่วยสะกดด้วย (ต้องมี Groq API key)",
                    );

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("ภาษาที่ต้องการตรวจสะกด:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings_autocorrect_languages)
                                .desired_width(160.0)
                                .hint_text("เช่น th,en"),
                        );
                        ui.label(
                            egui::RichText::new("(เว้นว่าง = ตรวจอัตโนมัติตามภาษาของข้อความ)")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });
                });

            ui.add_space(14.0);

            // CARD 5: Privacy & Data
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("🔒  PRIVACY & LOCAL DATA")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    ui.checkbox(
                        &mut self.settings_share_context,
                        "ส่งข้อความรอบเคอร์เซอร์ไปยังผู้ให้บริการ STT (Share surrounding text context)",
                    );
                    ui.label(
                        egui::RichText::new(
                            "  └ ปิดเพื่อความเป็นส่วนตัวสูงสุด: จะไม่ส่ง before/after text หรือ selected text ขึ้น cloud (แนะนำเมื่อพิมพ์ข้อความลับ)",
                        )
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut self.settings_store_textbox_contents,
                        "เก็บเนื้อหาช่องข้อความไว้ในประวัติในเครื่อง (Store textbox contents in local history)",
                    );

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("หมดเวลาคำขอ STT (วินาที):");
                        ui.add(
                            egui::DragValue::new(&mut self.settings_stt_timeout_secs)
                                .range(5..=300)
                                .speed(1.0)
                                .suffix(" s"),
                        );
                    });

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("บันทึกเสียงสูงสุดต่อครั้ง (วินาที):");
                        ui.add(
                            egui::DragValue::new(&mut self.settings_max_recording_secs)
                                .range(10..=3600)
                                .speed(5.0)
                                .suffix(" s"),
                        );
                    });
                    ui.label(
                        egui::RichText::new("  └ ระบบจะหยุดบันทึกอัตโนมัติเมื่อถึงเวลานี้ เพื่อป้องกันหน่วยความจำเต็ม")
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let (label, fill) = if self.clear_history_armed {
                            ("⚠️ กดอีกครั้งเพื่อล้างประวัติทั้งหมด", Color32::from_rgb(185, 28, 28))
                        } else {
                            ("🗑️ ล้างประวัติการพูดทั้งหมด", Color32::from_rgb(51, 65, 85))
                        };
                        let clear_btn = egui::Button::new(
                            egui::RichText::new(shape_thai(label))
                                .font(FontId::proportional(12.0))
                                .color(Color32::WHITE),
                        )
                        .fill(fill)
                        .rounding(Rounding::same(6.0));

                        if ui.add(clear_btn).clicked() {
                            if self.clear_history_armed {
                                self.clear_history_armed = false;
                                let _ = self.event_tx.send(GuiEvent::ClearHistory);
                            } else {
                                self.clear_history_armed = true;
                                self.state.set_toast("กดปุ่มอีกครั้งเพื่อยืนยันการล้างประวัติ");
                            }
                        }
                        if self.clear_history_armed && ui.button("ยกเลิก").clicked() {
                            self.clear_history_armed = false;
                        }
                    });

                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "ข้อมูลถูกเก็บไว้ที่: {}",
                            crate::paths::data_dir().display()
                        ))
                        .font(FontId::proportional(10.5))
                        .color(Color32::from_rgb(100, 116, 139)),
                    );
                });

            ui.add_space(10.0);

            // CARD 5: Updates & Diagnostics
            egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("🔄  SOFTWARE UPDATES & DIAGNOSTICS")
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Wflow v{}",
                                crate::update::current_version()
                            ))
                            .font(FontId::monospace(12.5))
                            .color(Color32::from_rgb(56, 189, 248))
                            .strong(),
                        );
                        ui.label(
                            egui::RichText::new("• ตรวจสอบจาก GitHub Releases")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });

                    ui.add_space(8.0);
                    ui.checkbox(
                        &mut self.settings_check_updates,
                        "ตรวจสอบอัปเดตอัตโนมัติเมื่อเริ่มโปรแกรม",
                    );
                    ui.checkbox(
                        &mut self.settings_include_prerelease,
                        "รวมเวอร์ชันทดลอง (pre-release)",
                    );
                    ui.label(
                        egui::RichText::new(
                            "  └ การเปลี่ยนแปลงในหน้านี้จะมีผลหลังกด Save All Preferences",
                        )
                        .font(FontId::proportional(10.5))
                        .color(Color32::from_rgb(100, 116, 139)),
                    );
                    ui.horizontal(|ui| {
                        ui.label("ตรวจสอบทุก ๆ:");
                        ui.add(
                            egui::DragValue::new(&mut self.settings_update_interval_hours)
                                .range(1..=720)
                                .speed(1.0)
                                .suffix(" ชั่วโมง"),
                        );
                    });

                    ui.add_space(10.0);
                    let running = self.state.update_check_running.load(Ordering::SeqCst);
                    ui.horizontal(|ui| {
                        let check_btn = egui::Button::new(
                            egui::RichText::new(shape_thai(if running {
                                "⏳ กำลังตรวจสอบ..."
                            } else {
                                "🔍 ตรวจสอบอัปเดตเดี๋ยวนี้"
                            }))
                            .font(FontId::proportional(12.0))
                            .color(Color32::WHITE)
                            .strong(),
                        )
                        .fill(Color32::from_rgb(14, 116, 144))
                        .rounding(Rounding::same(6.0));

                        if ui.add_enabled(!running, check_btn).clicked() {
                            let _ = self.event_tx.send(GuiEvent::CheckUpdate);
                        }
                        if ui.button("🌐 เปิดหน้า Releases").clicked() {
                            let _ = self.event_tx.send(GuiEvent::OpenUrl(format!(
                                "https://github.com/{}/releases",
                                crate::update::DEFAULT_REPO
                            )));
                        }
                        if ui.button("📂 เปิดโฟลเดอร์ Log").clicked() {
                            let _ = self.event_tx.send(GuiEvent::OpenLogFolder);
                        }
                    });

                    let update_info = self
                        .state
                        .update_info
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone());
                    let update_error = self
                        .state
                        .update_error
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone());

                    if let Some(info) = update_info {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(shape_thai(&format!(
                                "🎉 มีเวอร์ชันใหม่: v{} (ปัจจุบัน v{})",
                                info.latest, info.current
                            )))
                            .font(FontId::proportional(12.0))
                            .color(Color32::from_rgb(134, 239, 172)),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("⬇️ ดาวน์โหลด").clicked() {
                                let _ = self
                                    .event_tx
                                    .send(GuiEvent::OpenUrl(info.download_url.clone()));
                            }
                            if ui.button("📝 ดู Release Notes").clicked() {
                                let _ = self
                                    .event_tx
                                    .send(GuiEvent::OpenUrl(info.release_url.clone()));
                            }
                        });
                    } else if let Some(err) = update_error {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(shape_thai(&format!("⚠️ ตรวจสอบอัปเดตไม่สำเร็จ: {}", err)))
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(252, 165, 165)),
                        );
                    }

                    let last_check = self
                        .state
                        .config
                        .lock()
                        .ok()
                        .and_then(|cfg| cfg.last_update_check.clone());
                    if let Some(last) = last_check {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("ตรวจสอบล่าสุด: {}", last))
                                .font(FontId::proportional(10.5))
                                .color(Color32::from_rgb(100, 116, 139)),
                        );
                    }
                });

            ui.add_space(14.0);

            // Save Preferences Button
            let save_btn = egui::Button::new(
                egui::RichText::new("💾  Save All Preferences")
                    .font(FontId::proportional(14.0))
                    .color(Color32::WHITE)
                    .strong(),
            )
            .fill(Color32::from_rgb(79, 70, 229))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(129, 140, 248)))
            .rounding(Rounding::same(8.0))
            .min_size(Vec2::new(180.0, 38.0));

            if ui.add(save_btn).clicked() {
                let (
                    user_id,
                    skipped_update_version,
                    last_update_check,
                    client_name,
                    client_version,
                    stt_timeout_secs,
                ) = {
                    let cfg = self.state.config.lock().unwrap_or_else(|e| e.into_inner());
                    (
                        cfg.user_id.clone(),
                        cfg.skipped_update_version.clone(),
                        cfg.last_update_check.clone(),
                        cfg.client_name.clone(),
                        cfg.client_version.clone(),
                        cfg.stt_timeout_secs,
                    )
                };
                let new_config = AppConfig {
                    endpoint_url: self.settings_endpoint.clone(),
                    user_id,
                    hotkey: self.settings_hotkey,
                    toggle_window: self.settings_toggle,
                    hotkey_vk: self.settings_hotkey.key,
                    toggle_window_vk: self.settings_toggle.key,
                    stt_provider: self.settings_stt_provider,
                    groq_api_key: if self.settings_groq_api_key.trim().is_empty() {
                        None
                    } else {
                        Some(self.settings_groq_api_key.trim().to_string())
                    },
                    groq_model: self.settings_groq_model.clone(),
                    elevenlabs_api_key: if self.settings_elevenlabs_api_key.trim().is_empty() {
                        None
                    } else {
                        Some(self.settings_elevenlabs_api_key.trim().to_string())
                    },
                    elevenlabs_model: self.settings_elevenlabs_model.clone(),
                    baseten_api_key: if self.settings_baseten_api_key.trim().is_empty() {
                        None
                    } else {
                        Some(self.settings_baseten_api_key.trim().to_string())
                    },
                    baseten_model_id: self.settings_baseten_model.trim().to_string(),
                    show_floating_hud: self.settings_show_floating_hud,
                    ai_formatting: self.settings_ai_formatting,
                    writing_style: self.settings_writing_style.clone(),
                    sound_effects: self.settings_sound_effects,
                    input_device_name: self.settings_input_device_name.clone(),
                    recording_trigger_mode: self.settings_recording_trigger_mode.clone(),
                    language_code: self.settings_language.clone(),
                    auto_paste: self.settings_auto_paste,
                    auto_start: self.settings_auto_start,
                    close_to_tray: self.settings_close_to_tray,
                    share_context: self.settings_share_context,
                    store_textbox_contents: self.settings_store_textbox_contents,
                    check_updates: self.settings_check_updates,
                    include_prerelease: self.settings_include_prerelease,
                    update_check_interval_hours: self.settings_update_interval_hours.max(1),
                    last_update_check,
                    skipped_update_version,
                    max_recording_secs: self.settings_max_recording_secs.clamp(10, 3600),
                    client_name,
                    client_version,
                    stt_timeout_secs,
                    silence_filter: self.settings_silence_filter,
                    silence_sensitivity: self.settings_silence_sensitivity.clamp(0.0, 1.0),
                    silence_stop_secs: self.settings_silence_stop_secs.clamp(0, 600),
                    hallucination_guard: self.settings_hallucination_guard,
                    autocorrect_enabled: self.settings_autocorrect_enabled,
                    autocorrect_use_llm: self.settings_autocorrect_use_llm,
                    autocorrect_languages: self.settings_autocorrect_languages.trim().to_string(),
                    mixed_spacing: self.settings_mixed_spacing,
                    auth_token: if self.settings_token.is_empty() {
                        None
                    } else {
                        Some(self.settings_token.clone())
                    },
                };

                crate::sound::set_sound_enabled(new_config.sound_effects);

                if let Ok(mut lock) = self.state.config.lock() {
                    *lock = new_config.clone();
                }
                let _ = self.event_tx.send(GuiEvent::SaveConfig(Box::new(new_config)));
                self.state.set_toast("Settings & STT configurations saved successfully!");
            }

            ui.add_space(20.0);
        });
    }

    /// Handles a click on the update button: download the new release when there
    /// is one, otherwise start a check.
    fn trigger_update_action(&self, pending: &Option<UpdateInfo>) {
        if let Some(info) = pending.as_ref() {
            // Open the platform-appropriate download in the browser.
            let _ = self
                .event_tx
                .send(GuiEvent::OpenUrl(info.download_url.clone()));
            self.state
                .set_toast(&format!("กำลังเปิดหน้าดาวน์โหลด v{}", info.latest));
        } else {
            let _ = self.event_tx.send(GuiEvent::CheckUpdate);
            self.state.set_toast("กำลังตรวจสอบอัปเดต...");
        }
    }

    /// Returns the pending update when it is newer than the running build and
    /// has not been skipped by the user.
    fn pending_update(&self) -> Option<UpdateInfo> {
        let info = lock(&self.state.update_info).clone()?;
        if !info.is_newer() {
            return None;
        }
        let latest = info.latest.to_string();
        let skipped =
            lock(&self.state.config).skipped_update_version.as_deref() == Some(latest.as_str());
        if skipped {
            return None;
        }
        Some(info)
    }

    fn render_update_banner(&self, ui: &mut egui::Ui) {
        let Some(info) = self.pending_update() else {
            return;
        };

        let latest = info.latest.to_string();

        ui.add_space(10.0);
        egui::Frame::none()
            .fill(Color32::from_rgb(16, 42, 34))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(34, 197, 94)))
            .rounding(Rounding::same(10.0))
            .inner_margin(egui::Margin::symmetric(14.0, 9.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(shape_thai(&format!(
                            "🎉 Wflow v{} พร้อมใช้งานแล้ว (คุณใช้ v{})",
                            info.latest, info.current
                        )))
                        .font(FontId::proportional(12.5))
                        .color(Color32::from_rgb(187, 247, 208))
                        .strong(),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(shape_thai("ข้ามเวอร์ชันนี้"))
                                        .font(FontId::proportional(11.0))
                                        .color(Color32::from_rgb(148, 163, 184)),
                                )
                                .fill(Color32::from_rgb(20, 30, 42)),
                            )
                            .clicked()
                        {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::SkipUpdateVersion(latest.clone()));
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("⬇️ ดาวน์โหลด")
                                        .font(FontId::proportional(11.5))
                                        .color(Color32::WHITE)
                                        .strong(),
                                )
                                .fill(Color32::from_rgb(22, 163, 74)),
                            )
                            .clicked()
                        {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::OpenUrl(info.download_url.clone()));
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("📝 รายละเอียด")
                                        .font(FontId::proportional(11.0))
                                        .color(Color32::from_rgb(203, 213, 225)),
                                )
                                .fill(Color32::from_rgb(30, 41, 59)),
                            )
                            .clicked()
                        {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::OpenUrl(info.release_url.clone()));
                        }
                    });
                });
            });
    }

    #[cfg(not(target_os = "windows"))]
    fn render_floating_hud(
        &mut self,
        ctx: &egui::Context,
        status: DictationStatus,
        amplitude: f32,
    ) {
        if status == DictationStatus::Idle {
            return;
        }

        let badge = self
            .state
            .config
            .lock()
            .map(|cfg| cfg.stt_provider.short_badge())
            .unwrap_or("Wflow");

        let (label, color) = match status {
            DictationStatus::Listening => ("กำลังฟัง...", Color32::from_rgb(239, 68, 68)),
            DictationStatus::Transcribing => ("กำลังประมวลผล...", Color32::from_rgb(168, 85, 247)),
            DictationStatus::Pasting | DictationStatus::Done => {
                ("แทรกข้อความแล้ว", Color32::from_rgb(16, 185, 129))
            }
            DictationStatus::Error => ("เกิดข้อผิดพลาด", Color32::from_rgb(245, 158, 11)),
            DictationStatus::Idle => ("พร้อมใช้งาน", Color32::from_rgb(148, 163, 184)),
        };

        let viewport_id = egui::ViewportId::from_hash_of("wflow_hud");
        let monitor = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(egui::vec2(1920.0, 1080.0));
        let position = egui::pos2((monitor.x - 300.0) * 0.5, monitor.y - 110.0);
        ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::OuterPosition(position));

        let builder = egui::ViewportBuilder::default()
            .with_title("Wflow HUD")
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_mouse_passthrough(true)
            .with_resizable(false)
            .with_inner_size([300.0, 58.0]);

        let animation_time = self.animation_time;
        let badge = badge.to_string();
        let label = label.to_string();

        ctx.show_viewport_immediate(viewport_id, builder, move |ctx, _class| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::TRANSPARENT))
                .show(ctx, |ui| {
                    let rect = ui.max_rect();
                    let painter = ui.painter();
                    painter.rect_filled(
                        rect.shrink(3.0),
                        Rounding::same(22.0),
                        Color32::from_rgba_unmultiplied(12, 16, 26, 235),
                    );
                    painter.rect_stroke(
                        rect.shrink(3.0),
                        Rounding::same(22.0),
                        Stroke::new(1.0_f32, color.gamma_multiply(0.7)),
                    );

                    painter.circle_filled(
                        Pos2::new(rect.min.x + 26.0, rect.center().y),
                        5.0,
                        color,
                    );

                    painter.text(
                        Pos2::new(rect.min.x + 42.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &badge,
                        FontId::proportional(12.0),
                        Color32::from_rgb(148, 163, 184),
                    );
                    painter.text(
                        Pos2::new(rect.min.x + 100.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        shape_thai(&label),
                        FontId::proportional(12.5),
                        Color32::WHITE,
                    );

                    // Miniature waveform reacts to the live microphone level.
                    let bar_count = 7;
                    let base_x = rect.max.x - 72.0;
                    for index in 0..bar_count {
                        let phase = animation_time * 6.0 + index as f32 * 0.9;
                        let level = (amplitude * 18.0).clamp(0.0, 1.0);
                        let height = 3.0 + (phase.sin().abs() * 16.0 * (0.25 + level));
                        let x = base_x + index as f32 * 8.0;
                        painter.rect_filled(
                            Rect::from_center_size(
                                Pos2::new(x, rect.center().y),
                                Vec2::new(4.0, height),
                            ),
                            Rounding::same(2.0),
                            color.gamma_multiply(0.85),
                        );
                    }
                });

            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        });
    }

    fn render_toast(&self, ui: &mut egui::Ui) {
        if let Ok(mut lock) = self.state.toast.lock() {
            if let Some((msg, time)) = lock.clone() {
                if time.elapsed().as_secs_f32() < 2.5 {
                    let rect = ui.max_rect();
                    let toast_pos = Pos2::new(rect.center().x - 140.0, rect.max.y - 48.0);
                    let toast_rect = Rect::from_min_size(toast_pos, Vec2::new(280.0, 34.0));

                    ui.painter().rect_filled(
                        toast_rect,
                        Rounding::same(8.0),
                        Color32::from_rgb(26, 33, 50),
                    );
                    ui.painter().rect_stroke(
                        toast_rect,
                        Rounding::same(8.0),
                        Stroke::new(1.0_f32, Color32::from_rgb(99, 102, 241)),
                    );
                    ui.painter().text(
                        toast_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        shape_thai(&msg),
                        FontId::proportional(12.5),
                        Color32::WHITE,
                    );
                } else {
                    *lock = None;
                }
            }
        }
    }

    fn run_stt_test(&mut self) {
        if self.stt_test_in_progress.load(Ordering::SeqCst) {
            return;
        }

        self.stt_test_in_progress.store(true, Ordering::SeqCst);
        let in_progress = Arc::clone(&self.stt_test_in_progress);
        let status_lock = Arc::clone(&self.stt_test_status);
        let provider = self.settings_stt_provider;
        let groq_key = self.settings_groq_api_key.clone();
        let eleven_key = self.settings_elevenlabs_api_key.clone();
        let endpoint = self.settings_endpoint.clone();
        let egui_ctx = Arc::clone(&self.state.egui_ctx);

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            if let Ok(rt) = rt {
                rt.block_on(async move {
                    let res = match provider {
                        SttProvider::Groq => crate::stt::test_groq_api(&groq_key).await,
                        SttProvider::ElevenLabs => {
                            crate::stt::test_elevenlabs_api(&eleven_key).await
                        }
                        SttProvider::WisprFlow => {
                            crate::stt::test_wisprflow_endpoint(&endpoint).await
                        }
                    };

                    let mapped_res = match res {
                        Ok(msg) => Ok(msg),
                        Err(err) => Err(err.to_string()),
                    };

                    if let Ok(mut lock) = status_lock.lock() {
                        *lock = Some(mapped_res);
                    }
                    in_progress.store(false, Ordering::SeqCst);

                    if let Ok(lock) = egui_ctx.lock() {
                        if let Some(ctx) = lock.as_ref() {
                            ctx.request_repaint();
                        }
                    }
                });
            } else {
                in_progress.store(false, Ordering::SeqCst);
            }
        });
    }

    fn render_exit_confirmation_modal(&mut self, ctx: &egui::Context) {
        let toggle_key_name = self.settings_toggle.format_display();
        let mut close_modal = false;
        let mut do_exit = false;
        let mut do_tray = false;

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close_modal = true;
        }

        egui::Window::new(shape_thai("⚠️ ยืนยันการปิดโปรแกรม (Confirm Exit)"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([380.0, 150.0])
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(shape_thai("คุณต้องการปิดโปรแกรม Wflow ใช่หรือไม่?"))
                        .font(FontId::proportional(14.5))
                        .color(Color32::WHITE)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(shape_thai(
                        "หากเลือก 'ย่อลง Tray' โปรแกรมจะยังคงรอรับคำสั่งเสียงผ่านปุ่มลัดได้ในพื้นหลัง",
                    ))
                    .font(FontId::proportional(11.5))
                    .color(Color32::from_rgb(148, 163, 184)),
                );
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    // 1. Confirm Exit
                    let exit_btn = egui::Button::new(
                        egui::RichText::new(shape_thai("🔴 ปิดโปรแกรม (Exit)"))
                            .font(FontId::proportional(11.5))
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(185, 28, 28))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(239, 68, 68)))
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(120.0, 32.0));

                    if ui.add(exit_btn).clicked() {
                        do_exit = true;
                    }

                    ui.add_space(6.0);

                    // 2. Minimize to Tray
                    let tray_btn = egui::Button::new(
                        egui::RichText::new(shape_thai("📌 ย่อลง Tray"))
                            .font(FontId::proportional(11.5))
                            .color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(30, 41, 59))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(71, 85, 105)))
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(105.0, 32.0));

                    if ui.add(tray_btn).clicked() {
                        do_tray = true;
                    }

                    ui.add_space(6.0);

                    // 3. Cancel
                    let cancel_btn = egui::Button::new(
                        egui::RichText::new(shape_thai("ยกเลิก (Cancel)"))
                            .font(FontId::proportional(11.5))
                            .color(Color32::from_rgb(203, 213, 225)),
                    )
                    .fill(Color32::from_rgb(22, 29, 44))
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(85.0, 32.0));

                    if ui.add(cancel_btn).clicked() {
                        close_modal = true;
                    }
                });
            });

        if do_exit {
            self.show_exit_confirmation = false;
            self.state.pending_quit.store(true, Ordering::SeqCst);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if do_tray {
            self.show_exit_confirmation = false;
            #[cfg(target_os = "windows")]
            crate::platform::windows::PlatformManager::hide_main_window();
            #[cfg(not(target_os = "windows"))]
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                crate::platform::PlatformManager::hide_main_window();
                self.state.window_visible.store(false, Ordering::SeqCst);
            }
            self.state.set_toast(&format!(
                "ย่อลง Tray แล้ว กด {} หรือคลิกที่ Tray เพื่อเปิดใหม่",
                toggle_key_name
            ));
        } else if close_modal {
            self.show_exit_confirmation = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_button_is_always_labelled() {
        let (label, _, _, _) = update_button_style(None, false, "0.2.1", 0.0);
        assert!(label.contains("อัปเดต"), "idle label: {}", label);
        assert!(label.contains("0.2.1"), "idle label: {}", label);
    }

    #[test]
    fn update_button_highlights_a_new_release() {
        let (label, fill, _, text) = update_button_style(Some("0.3.0"), false, "0.2.1", 0.0);
        assert!(label.contains("0.3.0"), "label: {}", label);
        assert_eq!(fill, Color32::from_rgb(22, 163, 74));
        assert_eq!(text, Color32::WHITE);
    }

    #[test]
    fn update_button_shows_progress() {
        let (label, _, _, _) = update_button_style(None, true, "0.2.1", 0.0);
        assert!(label.contains("กำลังตรวจสอบ"), "label: {}", label);
    }
}
