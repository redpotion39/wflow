use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::db::{DictionaryEntry, HistoryRecord};
use crate::platform::{vk_to_string, ActiveAppContext, PlatformManager};
use crate::stt::SttProvider;
use crate::ui::DictationStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Studio,
    History,
    Dictionary,
    Settings,
}

#[derive(Debug, Clone)]
pub enum GuiEvent {
    StartDictation,
    StopDictation,
    RefreshHistory,
    SaveConfig(AppConfig),
    DeleteHistory(String),
    AddDictionaryEntry { word: String, replacement: Option<String> },
    DeleteDictionaryEntry(String),
    RefreshDictionary,
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
    pub recorded_key: Arc<Mutex<Option<(u8, u16)>>>,
}

impl AppState {
    pub fn new(config: AppConfig, initial_history: Vec<HistoryRecord>, initial_dict: Vec<DictionaryEntry>) -> Self {
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
    settings_hotkey_vk: u16,
    settings_toggle_vk: u16,
    recording_mode: Option<u8>,
    settings_auto_paste: bool,
    settings_auto_start: bool,
    settings_stt_provider: SttProvider,
    settings_groq_api_key: String,
    settings_groq_model: String,
    settings_elevenlabs_api_key: String,
    settings_elevenlabs_model: String,
    settings_show_floating_hud: bool,
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
    window_icon_applied: bool,
}

pub const POPULAR_HOTKEYS: &[(&str, u16)] = &[
    ("F8 (Wflow Default)", 0x77),
    ("Right Control", 0xA3),
    ("Left Control", 0xA2),
    ("Right Alt", 0xA5),
    ("Left Alt", 0xA4),
    ("Caps Lock", 0x14),
    ("F1", 0x70),
    ("F2", 0x71),
    ("F3", 0x72),
    ("F4", 0x73),
    ("F5", 0x74),
    ("F6", 0x75),
    ("F7", 0x76),
    ("F8", 0x77),
    ("F9", 0x78),
    ("F10", 0x79),
    ("F11", 0x7A),
    ("F12", 0x7B),
    ("Scroll Lock", 0x91),
    ("Pause / Break", 0x13),
    ("Insert", 0x2D),
    ("Tilde (~)", 0xC0),
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
    pub fn new(
        state: AppState,
        event_tx: mpsc::UnboundedSender<GuiEvent>,
    ) -> Self {
        let (
            endpoint,
            token,
            language,
            hotkey_vk,
            toggle_vk,
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
        ) = {
            let cfg = state.config.lock().unwrap();
            (
                cfg.endpoint_url.clone(),
                cfg.auth_token.clone().unwrap_or_default(),
                cfg.language_code.clone(),
                cfg.hotkey_vk,
                cfg.toggle_window_vk,
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
            settings_hotkey_vk: hotkey_vk,
            settings_toggle_vk: toggle_vk,
            recording_mode: None,
            settings_auto_paste: auto_paste,
            settings_auto_start: auto_start,
            settings_stt_provider: stt_provider,
            settings_groq_api_key: groq_key,
            settings_groq_model: groq_model,
            settings_elevenlabs_api_key: elevenlabs_key,
            settings_elevenlabs_model: elevenlabs_model,
            settings_show_floating_hud: show_hud,
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
        }
    }

    fn configure_thai_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();

        let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let font_dir = std::path::Path::new(&win_dir).join("Fonts");

        // 1. Primary Segoe UI Variable font for crisp modern typography
        let segoe_candidates = [
            font_dir.join("SegUIVar.ttf"),
            font_dir.join("segoeui.ttf"),
        ];
        let mut loaded_segoe = false;
        for path in &segoe_candidates {
            if path.exists() {
                if let Ok(bytes) = std::fs::read(path) {
                    fonts.font_data.insert(
                        "segoe_ui".to_owned(),
                        egui::FontData::from_owned(bytes),
                    );
                    loaded_segoe = true;
                    break;
                }
            }
        }

        // 2. Thai font candidates (Leelawadee UI is standard Windows Thai font)
        let thai_candidates = [
            font_dir.join("LeelawUI.ttf"),
            font_dir.join("leelawad.ttf"),
            font_dir.join("tahoma.ttf"),
        ];
        let mut loaded_thai = false;
        for path in &thai_candidates {
            if path.exists() {
                if let Ok(bytes) = std::fs::read(path) {
                    fonts.font_data.insert(
                        "thai_font".to_owned(),
                        egui::FontData::from_owned(bytes),
                    );
                    loaded_thai = true;
                    break;
                }
            }
        }

        // 3. Japanese & CJK font candidates (Yu Gothic, MS Gothic, Meiryo, Microsoft YaHei)
        let cjk_candidates = [
            font_dir.join("YuGothM.ttc"),
            font_dir.join("YuGothR.ttc"),
            font_dir.join("msgothic.ttc"),
            font_dir.join("meiryo.ttc"),
            font_dir.join("msyh.ttc"),
        ];
        let mut loaded_cjk = false;
        for path in &cjk_candidates {
            if path.exists() {
                if let Ok(bytes) = std::fs::read(path) {
                    fonts.font_data.insert(
                        "cjk_font".to_owned(),
                        egui::FontData::from_owned(bytes),
                    );
                    loaded_cjk = true;
                    break;
                }
            }
        }

        // 4. Windows Native Emoji font (seguiemj.ttf)
        let emoji_path = font_dir.join("seguiemj.ttf");
        if emoji_path.exists() {
            if let Ok(bytes) = std::fs::read(&emoji_path) {
                fonts.font_data.insert(
                    "segoe_emoji".to_owned(),
                    egui::FontData::from_owned(bytes),
                );
            }
        }

        // Configure font fallback priority: English/Latin -> Thai -> Japanese/CJK -> Emoji
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            if loaded_segoe {
                family.insert(0, "segoe_ui".to_owned());
            }
            if loaded_thai {
                family.push("thai_font".to_owned());
            }
            if loaded_cjk {
                family.push("cjk_font".to_owned());
            }
            family.push("segoe_emoji".to_owned());
        }

        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            if loaded_thai {
                family.push("thai_font".to_owned());
            }
            if loaded_cjk {
                family.push("cjk_font".to_owned());
            }
        }

        if loaded_thai || loaded_segoe || loaded_cjk {
            ctx.set_fonts(fonts);
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
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66));
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
                self.logo_texture = Some(ctx.load_texture("wispr_flow_app_logo", color_image, egui::TextureOptions::LINEAR));
            }
        }

        #[cfg(target_os = "windows")]
        if !self.window_icon_applied {
            crate::platform::windows::PlatformManager::apply_window_icon(
                crate::platform::windows::PlatformManager::get_app_icon(1)
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
            if let Some((mode, vk)) = lock.take() {
                self.recording_mode = None;
                if vk == 0 {
                    self.state.set_toast("Key recording cancelled");
                } else {
                    let key_name = vk_to_string(vk);
                    if mode == 1 {
                        self.settings_hotkey_vk = vk;
                        self.state.set_toast(&format!("Dictation hotkey changed to: {}", key_name));
                    } else if mode == 2 {
                        self.settings_toggle_vk = vk;
                        self.state.set_toast(&format!("Window Toggle hotkey changed to: {}", key_name));
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
            let cfg = self.state.config.lock().unwrap();
            (
                vk_to_string(cfg.hotkey_vk),
                vk_to_string(cfg.toggle_window_vk),
            )
        };

        // Window close [X] -> minimize to tray
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            #[cfg(target_os = "windows")]
            crate::platform::windows::PlatformManager::hide_main_window();
            self.state.window_visible.store(false, Ordering::SeqCst);
            self.state.set_toast(&format!("Minimized to Taskbar Tray. Press {} or click tray icon to reopen.", toggle_key_name));
        }

        // Tray menu exit
        if self.state.pending_quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        Self::apply_custom_theme(ctx);
        self.animation_time += 0.04;

        let is_rec = self.state.is_recording.load(Ordering::SeqCst);
        let amplitude = *self.state.current_amplitude.lock().unwrap();
        let status = *self.state.status.lock().unwrap();

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

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header Bar
            self.render_header(ui, status, is_rec, &dictate_key_name, &toggle_key_name);

            ui.add_space(8.0);

            // Tab Navigation
            self.render_tab_nav(ui);

            ui.add_space(10.0);

            // Active Tab Content
            match self.active_tab {
                ActiveTab::Studio => self.render_studio_tab(ui, is_rec, amplitude, status, &dictate_key_name),
                ActiveTab::History => self.render_history_tab(ui),
                ActiveTab::Dictionary => self.render_dictionary_tab(ui),
                ActiveTab::Settings => self.render_settings_tab(ui),
            }

            // Toast notification overlay
            self.render_toast(ui);
        });
    }
}

impl WisprFlowGui {
    fn render_header(
        &mut self,
        ui: &mut egui::Ui,
        status: DictationStatus,
        _is_rec: bool,
        dictate_key_name: &str,
        toggle_key_name: &str,
    ) {
        egui::Frame::none()
            .fill(Color32::from_rgb(15, 20, 32))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 41, 64)))
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::symmetric(14.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Logo Icon: Render the stunning custom Wispr Flow icon artwork
                    let (logo_rect, _) = ui.allocate_exact_size(Vec2::new(38.0, 38.0), Sense::hover());
                    if let Some(texture) = &self.logo_texture {
                        // Soft glow container around icon
                        ui.painter().rect_filled(
                            logo_rect.expand(1.5),
                            Rounding::same(10.0),
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
                            Rounding::same(10.0),
                            Color32::from_rgb(49, 46, 129),
                        );
                        ui.painter().rect_stroke(
                            logo_rect,
                            Rounding::same(10.0),
                            Stroke::new(1.5_f32, Color32::from_rgb(99, 102, 241)),
                        );
                        ui.painter().text(
                            logo_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "🎙️",
                            FontId::proportional(18.0),
                            Color32::WHITE,
                        );
                    }

                    ui.add_space(6.0);

                    // App Title & Badges
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("WFLOW")
                                    .font(FontId::proportional(18.0))
                                    .strong()
                                    .color(Color32::WHITE),
                            );

                            // Native Rust Badge
                            let rust_badge = ui.label(
                                egui::RichText::new("NATIVE RUST")
                                    .font(FontId::proportional(10.5))
                                    .color(Color32::from_rgb(165, 180, 252))
                                    .strong(),
                            );
                            ui.painter().rect_filled(
                                rust_badge.rect.expand(2.5),
                                Rounding::same(4.0),
                                Color32::from_rgba_unmultiplied(99, 102, 241, 40),
                            );

                            // Active STT Engine & Model Badge
                            let cfg = self.state.config.lock().unwrap();
                            let current_provider = cfg.stt_provider;
                            let (p_icon, p_name, p_model, p_color) = match current_provider {
                                SttProvider::WisprFlow => ("⚡", "Wispr Flow", "Cloud Native (gRPC)", Color32::from_rgb(129, 140, 248)),
                                SttProvider::Groq => ("🚀", "Groq LPU", cfg.groq_model.as_str(), Color32::from_rgb(251, 146, 60)),
                                SttProvider::ElevenLabs => ("🎙️", "ElevenLabs", cfg.elevenlabs_model.as_str(), Color32::from_rgb(192, 132, 252)),
                            };
                            let prov_badge = ui.label(
                                egui::RichText::new(format!("{} {} • {}", p_icon, p_name, p_model))
                                    .font(FontId::proportional(11.0))
                                    .color(p_color)
                                    .strong(),
                            );
                            ui.painter().rect_filled(
                                prov_badge.rect.expand(2.5),
                                Rounding::same(4.0),
                                Color32::from_rgba_unmultiplied(p_color.r(), p_color.g(), p_color.b(), 35),
                            );
                            ui.painter().rect_stroke(
                                prov_badge.rect.expand(2.5),
                                Rounding::same(4.0),
                                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(p_color.r(), p_color.g(), p_color.b(), 80)),
                            );
                        });

                        ui.label(
                            egui::RichText::new("Zero Electron • Ultra-Low Latency • Direct Win32 Native Pipeline")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Minimize to Tray button
                        let hide_btn = egui::Button::new(
                            egui::RichText::new(format!("📌 Hide to Tray ({})", toggle_key_name))
                                .font(FontId::proportional(12.0))
                                .color(Color32::from_rgb(203, 213, 225)),
                        )
                        .fill(Color32::from_rgb(26, 33, 50))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 58, 86)))
                        .rounding(Rounding::same(8.0))
                        .min_size(Vec2::new(132.0, 32.0));

                        if ui.add(hide_btn).clicked() {
                            #[cfg(target_os = "windows")]
                            crate::platform::windows::PlatformManager::hide_main_window();
                            self.state.window_visible.store(false, Ordering::SeqCst);
                            self.state.set_toast(&format!("Minimized to Taskbar Tray. Press {} to reopen.", toggle_key_name));
                        }

                        ui.add_space(8.0);

                        // Dynamic Status Pill with pulsing glow
                        let (status_text, bg_color, border_color) = match status {
                            DictationStatus::Idle => (
                                format!("Ready [{}]", dictate_key_name),
                                Color32::from_rgb(6, 78, 59),
                                Color32::from_rgb(16, 185, 129),
                            ),
                            DictationStatus::Listening => (
                                "Recording...".to_string(),
                                Color32::from_rgb(127, 29, 29),
                                Color32::from_rgb(239, 68, 68),
                            ),
                            DictationStatus::Transcribing => (
                                "AI Transcribing...".to_string(),
                                Color32::from_rgb(88, 28, 135),
                                Color32::from_rgb(168, 85, 247),
                            ),
                            DictationStatus::Pasting => (
                                "Pasting Text...".to_string(),
                                Color32::from_rgb(30, 58, 138),
                                Color32::from_rgb(59, 130, 246),
                            ),
                            DictationStatus::Done => (
                                "Inserted!".to_string(),
                                Color32::from_rgb(6, 78, 59),
                                Color32::from_rgb(16, 185, 129),
                            ),
                            DictationStatus::Error => (
                                "Error".to_string(),
                                Color32::from_rgb(120, 53, 15),
                                Color32::from_rgb(245, 158, 11),
                            ),
                        };

                        let status_pill = ui.label(
                            egui::RichText::new(format!("● {}", status_text))
                                .font(FontId::proportional(12.5))
                                .color(Color32::WHITE)
                                .strong(),
                        );
                        ui.painter().rect_filled(
                            status_pill.rect.expand(5.0),
                            Rounding::same(12.0),
                            bg_color,
                        );
                        ui.painter().rect_stroke(
                            status_pill.rect.expand(5.0),
                            Rounding::same(12.0),
                            Stroke::new(1.0_f32, border_color),
                        );
                    });
                });
            });
    }

    fn render_tab_nav(&mut self, ui: &mut egui::Ui) {
        let history_count = self.state.history.lock().unwrap().len();
        let dict_count = self.state.dictionary_entries.lock().unwrap().len();

        egui::Frame::none()
            .fill(Color32::from_rgb(15, 20, 32))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 41, 64)))
            .rounding(Rounding::same(10.0))
            .inner_margin(egui::Margin::symmetric(4.0, 4.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let tabs = [
                        (ActiveTab::Studio, "🎙️ Live Studio".to_string()),
                        (ActiveTab::History, format!("📜 History ({})", history_count)),
                        (ActiveTab::Dictionary, format!("📖 Dictionary & Snippets ({})", dict_count)),
                        (ActiveTab::Settings, "⚙️ Settings & Audio".to_string()),
                    ];

                    for (tab, label) in tabs {
                        let is_active = self.active_tab == tab;
                        let btn = egui::Button::new(
                            egui::RichText::new(label)
                                .font(FontId::proportional(12.5))
                                .color(if is_active { Color32::WHITE } else { Color32::from_rgb(148, 163, 184) })
                                .strong(),
                        )
                        .fill(if is_active { Color32::from_rgb(79, 70, 229) } else { Color32::TRANSPARENT })
                        .stroke(if is_active { Stroke::new(1.0_f32, Color32::from_rgb(129, 140, 248)) } else { Stroke::NONE })
                        .rounding(Rounding::same(7.0))
                        .min_size(Vec2::new(150.0, 32.0));

                        if ui.add(btn).clicked() {
                            self.active_tab = tab;
                            if tab == ActiveTab::History {
                                let _ = self.event_tx.send(GuiEvent::RefreshHistory);
                            } else if tab == ActiveTab::Dictionary {
                                let _ = self.event_tx.send(GuiEvent::RefreshDictionary);
                            }
                        }
                    }
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
                let active_app = self.state.active_app.lock().unwrap().clone();
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
                    let cfg = self.state.config.lock().unwrap();
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

                            let latency = *self.state.last_latency.lock().unwrap();
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

                        let transcript = self.state.last_transcript.lock().unwrap().clone();

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
                                        egui::RichText::new(&transcript)
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
                                });
                            }
                        });
                    });

                ui.add_space(10.0);

                // Quick Stats Card
                let history_count = self.state.history.lock().unwrap().len();
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
            });
        });
    }

    fn render_history_tab(&mut self, ui: &mut egui::Ui) {
        let history = self.state.history.lock().unwrap().clone();

        // 1. Top Summary Banner
        let total_words: usize = history.iter().map(|h| h.formatted_text.split_whitespace().count()).sum();
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
                            ui.label(egui::RichText::new("Total Transcriptions").font(FontId::proportional(11.0)).color(Color32::from_rgb(148, 163, 184)));
                            ui.label(egui::RichText::new(format!("{} recorded", history.len())).font(FontId::proportional(14.0)).strong().color(Color32::WHITE));
                        });
                    });
                    stat_cols[1].horizontal(|ui| {
                        ui.label("📝");
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("Dictated Words").font(FontId::proportional(11.0)).color(Color32::from_rgb(148, 163, 184)));
                            ui.label(egui::RichText::new(format!("{} words", total_words)).font(FontId::proportional(14.0)).strong().color(Color32::from_rgb(167, 139, 250)));
                        });
                    });
                    stat_cols[2].horizontal(|ui| {
                        ui.label("⏱️");
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("Total Voice Time").font(FontId::proportional(11.0)).color(Color32::from_rgb(148, 163, 184)));
                            ui.label(egui::RichText::new(format!("{:.1} seconds", total_duration)).font(FontId::proportional(14.0)).strong().color(Color32::from_rgb(52, 211, 153)));
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
                                        egui::RichText::new(format!("⏱️ {:.1}s", rec.duration_secs))
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

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let del_btn = egui::Button::new("🗑️")
                                        .fill(Color32::from_rgb(60, 20, 20))
                                        .rounding(Rounding::same(4.0));
                                    if ui.add(del_btn).clicked() {
                                        let _ = self.event_tx.send(GuiEvent::DeleteHistory(rec.transcript_id.clone()));
                                        self.state.set_toast("Record deleted");
                                    }

                                    ui.add_space(4.0);

                                    let paste_btn = egui::Button::new("✍️ Paste")
                                        .fill(Color32::from_rgb(49, 46, 129))
                                        .rounding(Rounding::same(4.0));
                                    if ui.add(paste_btn).clicked() {
                                        let _ = PlatformManager::paste_text(&rec.formatted_text);
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
                                });
                            });

                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(&rec.formatted_text)
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
        let entries = self.state.dictionary_entries.lock().unwrap().clone();

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
                                    egui::RichText::new(&entry.word)
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
                                            egui::RichText::new(repl)
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
                            let curr_name = vk_to_string(self.settings_hotkey_vk);
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
                            egui::RichText::new("Hold this key anywhere in Windows to record voice, release to insert text.")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let curr_label = format!("Preset: {}", vk_to_string(self.settings_hotkey_vk));
                            egui::ComboBox::from_id_salt("hotkey_combo")
                                .selected_text(curr_label)
                                .show_ui(ui, |ui| {
                                    for (name, vk) in POPULAR_HOTKEYS {
                                        if ui.selectable_label(self.settings_hotkey_vk == *vk, *name).clicked() {
                                            self.settings_hotkey_vk = *vk;
                                        }
                                    }
                                });

                            let is_rec = self.recording_mode == Some(1);
                            let rec_text = if is_rec {
                                "🔴 Listening for key... (Esc to cancel)"
                            } else {
                                "⌨️ Press any key to set"
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
                                    self.state.set_toast("Press any key to assign Dictation shortcut (Esc to cancel)");
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
                            let curr_name = vk_to_string(self.settings_toggle_vk);
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
                            egui::RichText::new("Press this key anywhere to toggle Wispr Flow Studio between screen and system tray.")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let curr_label = format!("Preset: {}", vk_to_string(self.settings_toggle_vk));
                            egui::ComboBox::from_id_salt("toggle_combo")
                                .selected_text(curr_label)
                                .show_ui(ui, |ui| {
                                    for (name, vk) in POPULAR_HOTKEYS {
                                        if ui.selectable_label(self.settings_toggle_vk == *vk, *name).clicked() {
                                            self.settings_toggle_vk = *vk;
                                        }
                                    }
                                });

                            let is_rec = self.recording_mode == Some(2);
                            let rec_text = if is_rec {
                                "🔴 Listening for key... (Esc to cancel)"
                            } else {
                                "⌨️ Press any key to set"
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
                                    self.state.set_toast("Press any key to assign Window Toggle shortcut (Esc to cancel)");
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
                        egui::RichText::new("  └ Launches Wflow minimized in the taskbar notification tray when your PC boots.")
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
                                egui::Frame::none()
                                    .fill(if is_sel { Color32::from_rgb(26, 34, 52) } else { Color32::from_rgb(15, 20, 32) })
                                    .stroke(if is_sel { Stroke::new(1.5_f32, *color) } else { Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)) })
                                    .rounding(Rounding::same(8.0))
                                    .inner_margin(egui::Margin::same(10.0))
                                    .show(ui, |ui| {
                                        let check = if is_sel { "✔ " } else { "" };
                                        if ui.add(
                                            egui::Button::new(
                                                egui::RichText::new(format!("{}{}", check, title))
                                                    .font(FontId::proportional(13.0))
                                                    .color(if is_sel { Color32::WHITE } else { Color32::from_rgb(203, 213, 225) })
                                                    .strong(),
                                            )
                                            .fill(Color32::TRANSPARENT),
                                        ).clicked() {
                                            self.settings_stt_provider = *p;
                                        }
                                        ui.label(
                                            egui::RichText::new(desc)
                                                .font(FontId::proportional(10.5))
                                                .color(Color32::from_rgb(148, 163, 184)),
                                        );
                                    });
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
                let new_config = AppConfig {
                    endpoint_url: self.settings_endpoint.clone(),
                    user_id: self.state.config.lock().unwrap().user_id.clone(),
                    hotkey_vk: self.settings_hotkey_vk,
                    toggle_window_vk: self.settings_toggle_vk,
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
                    show_floating_hud: self.settings_show_floating_hud,
                    ai_formatting: self.settings_ai_formatting,
                    writing_style: self.settings_writing_style.clone(),
                    sound_effects: self.settings_sound_effects,
                    input_device_name: self.settings_input_device_name.clone(),
                    recording_trigger_mode: self.settings_recording_trigger_mode.clone(),
                    language_code: self.settings_language.clone(),
                    auto_paste: self.settings_auto_paste,
                    auto_start: self.settings_auto_start,
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
                let _ = self.event_tx.send(GuiEvent::SaveConfig(new_config));
                self.state.set_toast("Settings & STT configurations saved successfully!");
            }

            ui.add_space(20.0);
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
                        &msg,
                        FontId::proportional(12.5),
                        Color32::WHITE,
                    );
                } else {
                    *lock = None;
                }
            }
        }
    }
}

