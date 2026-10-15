use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::db::HistoryRecord;
use crate::platform::ActiveAppContext;
use crate::ui::DictationStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Studio,
    History,
    Settings,
}

#[derive(Debug, Clone)]
pub enum GuiEvent {
    StartDictation,
    StopDictation,
    RefreshHistory,
    SaveConfig(AppConfig),
    DeleteHistory(String),
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
    pub toast: Arc<Mutex<Option<(String, Instant)>>>,
    pub window_visible: Arc<AtomicBool>,
    #[allow(dead_code)]
    pub pending_toggle_window: Arc<AtomicBool>,
    pub pending_quit: Arc<AtomicBool>,
    pub egui_ctx: Arc<Mutex<Option<egui::Context>>>,
    pub recorded_key: Arc<Mutex<Option<(u8, u16)>>>,
}

impl AppState {
    pub fn new(config: AppConfig, initial_history: Vec<HistoryRecord>) -> Self {
        Self {
            is_recording: Arc::new(AtomicBool::new(false)),
            current_amplitude: Arc::new(Mutex::new(0.0)),
            status: Arc::new(Mutex::new(DictationStatus::Idle)),
            last_transcript: Arc::new(Mutex::new(String::new())),
            last_latency: Arc::new(Mutex::new(0.0)),
            active_app: Arc::new(Mutex::new(ActiveAppContext::default())),
            config: Arc::new(Mutex::new(config)),
            history: Arc::new(Mutex::new(initial_history)),
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
    settings_stt_provider: crate::stt::SttProvider,
    settings_groq_api_key: String,
    settings_groq_model: String,
    settings_elevenlabs_api_key: String,
    settings_elevenlabs_model: String,
    settings_show_floating_hud: bool,
    // Animation tick
    animation_time: f32,
    fonts_initialized: bool,
}

pub const POPULAR_HOTKEYS: &[(&str, u16)] = &[
    ("F8 (Wispr Flow Default)", 0x77),
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
            stt_provider,
            groq_key,
            groq_model,
            elevenlabs_key,
            elevenlabs_model,
            show_hud,
        ) = {
            let cfg = state.config.lock().unwrap();
            (
                cfg.endpoint_url.clone(),
                cfg.auth_token.clone().unwrap_or_default(),
                cfg.language_code.clone(),
                cfg.hotkey_vk,
                cfg.toggle_window_vk,
                cfg.auto_paste,
                cfg.stt_provider,
                cfg.groq_api_key.clone().unwrap_or_default(),
                cfg.groq_model.clone(),
                cfg.elevenlabs_api_key.clone().unwrap_or_default(),
                cfg.elevenlabs_model.clone(),
                cfg.show_floating_hud,
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
            settings_stt_provider: stt_provider,
            settings_groq_api_key: groq_key,
            settings_groq_model: groq_model,
            settings_elevenlabs_api_key: elevenlabs_key,
            settings_elevenlabs_model: elevenlabs_model,
            settings_show_floating_hud: show_hud,
            animation_time: 0.0,
            fonts_initialized: false,
        }
    }

    fn configure_thai_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();

        let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let font_dir = std::path::Path::new(&win_dir).join("Fonts");

        // 1. Thai font candidates (Leelawadee UI is standard modern Windows 10/11 Thai font, Tahoma is fallback)
        let thai_candidates = [
            font_dir.join("LeelawUI.ttf"),
            font_dir.join("leelawad.ttf"),
            font_dir.join("tahoma.ttf"),
        ];

        let mut loaded_thai = false;
        for path in &thai_candidates {
            if path.exists() {
                if let Ok(bytes) = std::fs::read(path) {
                    tracing::info!("Loaded Thai system font from {:?}", path);
                    fonts.font_data.insert(
                        "thai_font".to_owned(),
                        egui::FontData::from_owned(bytes),
                    );

                    // Insert at index 0 for Proportional so English and Thai both render seamlessly
                    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                        family.insert(0, "thai_font".to_owned());
                    }
                    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                        family.push("thai_font".to_owned());
                    }
                    loaded_thai = true;
                    break;
                }
            }
        }

        // 2. Windows Native Emoji font (seguiemj.ttf)
        let emoji_path = font_dir.join("seguiemj.ttf");
        if emoji_path.exists() {
            if let Ok(bytes) = std::fs::read(&emoji_path) {
                fonts.font_data.insert(
                    "segoe_emoji".to_owned(),
                    egui::FontData::from_owned(bytes),
                );
                if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    family.push("segoe_emoji".to_owned());
                }
            }
        }

        if loaded_thai {
            ctx.set_fonts(fonts);
        }
    }

    fn apply_custom_theme(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(241, 245, 249));
        visuals.panel_fill = Color32::from_rgb(15, 17, 24);
        visuals.window_fill = Color32::from_rgb(21, 24, 35);
        
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(24, 28, 41);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(45, 52, 72));
        visuals.widgets.noninteractive.rounding = Rounding::same(10.0);

        visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 36, 52);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(52, 60, 84));
        visuals.widgets.inactive.rounding = Rounding::same(8.0);

        visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 54, 78);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(99, 102, 241));
        visuals.widgets.hovered.rounding = Rounding::same(8.0);

        visuals.widgets.active.bg_fill = Color32::from_rgb(99, 102, 241);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(129, 140, 248));
        visuals.widgets.active.rounding = Rounding::same(8.0);

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

        // Handle recorded key from low-level hook
        if let Ok(mut lock) = self.state.recorded_key.lock() {
            if let Some((mode, vk)) = lock.take() {
                self.recording_mode = None;
                if vk == 0 {
                    self.state.set_toast("Key recording cancelled");
                } else {
                    let key_name = crate::platform::vk_to_string(vk);
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
                crate::platform::PlatformManager::stop_recording_hotkey();
                self.state.set_toast("Key recording cancelled");
            }
        }

        let (dictate_key_name, toggle_key_name) = {
            let cfg = self.state.config.lock().unwrap();
            (
                crate::platform::vk_to_string(cfg.hotkey_vk),
                crate::platform::vk_to_string(cfg.toggle_window_vk),
            )
        };

        // 1. Intercept window [X] close button -> minimize to tray via Win32 SW_HIDE
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            #[cfg(target_os = "windows")]
            crate::platform::windows::PlatformManager::hide_main_window();
            self.state.window_visible.store(false, Ordering::SeqCst);
            self.state.set_toast(&format!("Minimized to Taskbar Tray. Press {} or click tray icon to reopen.", toggle_key_name));
        }

        // 3. Handle tray menu exit
        if self.state.pending_quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        Self::apply_custom_theme(ctx);
        self.animation_time += 0.04;

        let is_rec = self.state.is_recording.load(Ordering::SeqCst);
        let amplitude = *self.state.current_amplitude.lock().unwrap();
        let status = *self.state.status.lock().unwrap();

        // 60 FPS repainting when recording or animating
        if is_rec || status != DictationStatus::Idle || self.recording_mode.is_some() {
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header Bar
            self.render_header(ui, status, is_rec, &dictate_key_name, &toggle_key_name);

            ui.add_space(10.0);

            // Tab Navigation
            self.render_tab_nav(ui);

            ui.add_space(12.0);

            // Active Tab Content
            match self.active_tab {
                ActiveTab::Studio => self.render_studio_tab(ui, is_rec, amplitude, status, &dictate_key_name),
                ActiveTab::History => self.render_history_tab(ui),
                ActiveTab::Settings => self.render_settings_tab(ui),
            }

            // Toast overlay
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
        ui.horizontal(|ui| {
            // Logo & Title
            ui.label(
                egui::RichText::new("🎙️")
                    .font(FontId::proportional(26.0))
            );
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("WISPR FLOW")
                            .font(FontId::proportional(19.0))
                            .strong()
                            .color(Color32::from_rgb(255, 255, 255)),
                    );
                    
                    // Rust Native Badge
                    let badge_rect = ui.label(
                        egui::RichText::new("RUST NATIVE")
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(165, 180, 252))
                            .strong(),
                    );
                    ui.painter().rect_filled(
                        badge_rect.rect.expand(3.0),
                        Rounding::same(4.0),
                        Color32::from_rgba_unmultiplied(99, 102, 241, 45),
                    );

                    // Active STT Provider Badge
                    let current_provider = self.state.config.lock().unwrap().stt_provider;
                    let (p_name, p_color) = match current_provider {
                        crate::stt::SttProvider::WisprFlow => ("⚡ WISPR FLOW", Color32::from_rgb(99, 102, 241)),
                        crate::stt::SttProvider::Groq => ("⚡ GROQ WHISPER", Color32::from_rgb(249, 115, 22)),
                        crate::stt::SttProvider::ElevenLabs => ("⚡ ELEVENLABS", Color32::from_rgb(168, 85, 247)),
                    };
                    let prov_rect = ui.label(
                        egui::RichText::new(p_name)
                            .font(FontId::proportional(11.0))
                            .color(p_color)
                            .strong(),
                    );
                    ui.painter().rect_filled(
                        prov_rect.rect.expand(3.0),
                        Rounding::same(4.0),
                        Color32::from_rgba_unmultiplied(p_color.r(), p_color.g(), p_color.b(), 35),
                    );
                });
                ui.label(
                    egui::RichText::new("Ultra-fast Speech-to-Text • Zero Electron • < 15MB RAM")
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Status Pill
                let ready_text = format!("🟢 Ready ({})", dictate_key_name);
                let (pill_text, pill_bg, pill_fg) = match status {
                    DictationStatus::Idle => (ready_text.as_str(), Color32::from_rgb(22, 101, 52), Color32::from_rgb(187, 247, 208)),
                    DictationStatus::Listening => ("🔴 Streaming Audio...", Color32::from_rgb(153, 27, 27), Color32::from_rgb(254, 202, 202)),
                    DictationStatus::Transcribing => ("🟣 AI Transcribing...", Color32::from_rgb(107, 33, 168), Color32::from_rgb(243, 232, 255)),
                    DictationStatus::Pasting => ("✍️ Pasting Text...", Color32::from_rgb(30, 64, 175), Color32::from_rgb(219, 234, 254)),
                    DictationStatus::Done => ("✅ Inserted", Color32::from_rgb(21, 128, 61), Color32::from_rgb(220, 252, 231)),
                    DictationStatus::Error => ("⚠️ Error", Color32::from_rgb(180, 83, 9), Color32::from_rgb(254, 243, 199)),
                };

                let resp = ui.label(
                    egui::RichText::new(pill_text)
                        .font(FontId::proportional(13.0))
                        .color(pill_fg)
                        .strong(),
                );
                ui.painter().rect_filled(
                    resp.rect.expand(6.0),
                    Rounding::same(12.0),
                    pill_bg,
                );

                ui.add_space(8.0);

                let hide_label = format!("📌 Hide to Tray ({})", toggle_key_name);
                let hide_btn = egui::Button::new(
                    egui::RichText::new(hide_label)
                        .font(FontId::proportional(12.0))
                        .color(Color32::from_rgb(203, 213, 225)),
                )
                .fill(Color32::from_rgb(30, 36, 52))
                .rounding(Rounding::same(6.0));

                if ui.add(hide_btn).clicked() {
                    #[cfg(target_os = "windows")]
                    crate::platform::windows::PlatformManager::hide_main_window();
                    self.state.window_visible.store(false, Ordering::SeqCst);
                    self.state.set_toast(&format!("Minimized to Taskbar Tray. Press {} or click tray icon to reopen.", toggle_key_name));
                }
            });
        });

        ui.add_space(4.0);
        ui.separator();
    }

    fn render_tab_nav(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let tabs = [
                (ActiveTab::Studio, "🎙️ Studio & Live"),
                (ActiveTab::History, "📜 History & Transcripts"),
                (ActiveTab::Settings, "⚙️ Settings & Audio"),
            ];

            for (tab, label) in tabs {
                let is_active = self.active_tab == tab;
                let bg = if is_active {
                    Color32::from_rgb(99, 102, 241)
                } else {
                    Color32::from_rgb(26, 31, 44)
                };
                let fg = if is_active {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(156, 163, 175)
                };

                let btn = egui::Button::new(
                    egui::RichText::new(label)
                        .font(FontId::proportional(13.0))
                        .color(fg)
                        .strong(),
                )
                .fill(bg)
                .rounding(Rounding::same(8.0))
                .min_size(Vec2::new(140.0, 32.0));

                if ui.add(btn).clicked() {
                    self.active_tab = tab;
                    if tab == ActiveTab::History {
                        let _ = self.event_tx.send(GuiEvent::RefreshHistory);
                    }
                }
            }
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
            // LEFT COLUMN: Big Waveform Visualizer & Dictate Hero Card
            cols[0].vertical(|ui| {
                egui::Frame::none()
                    .fill(Color32::from_rgb(20, 24, 35))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(40, 48, 68)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(18.0))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("REAL-TIME AUDIO STUDIO")
                                .font(FontId::proportional(12.0))
                                .color(Color32::from_rgb(148, 163, 184))
                                .strong(),
                        );
                        ui.add_space(8.0);

                        // Visualizer Canvas (Audio Waveform)
                        let visualizer_height = 110.0;
                        let (rect, _response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), visualizer_height),
                            Sense::hover(),
                        );

                        // Draw background for visualizer
                        ui.painter().rect_filled(
                            rect,
                            Rounding::same(10.0),
                            Color32::from_rgb(12, 14, 20),
                        );

                        let bar_count = 32;
                        let bar_width = (rect.width() - (bar_count as f32 * 4.0)) / bar_count as f32;
                        let center_y = rect.center().y;

                        for i in 0..bar_count {
                            let x = rect.min.x + (i as f32 * (bar_width + 4.0)) + 6.0;
                            let phase = (i as f32 * 0.35) + (self.animation_time * 4.0);
                            
                            let height_factor = if is_rec {
                                let wave = (phase.sin() * 0.4 + 0.6) * (amplitude * 6.0).clamp(0.15, 1.0);
                                wave * (visualizer_height * 0.4)
                            } else {
                                (phase.sin() * 0.2 + 0.3) * 6.0
                            };

                            let bar_color = if is_rec {
                                let ratio = (i as f32) / (bar_count as f32);
                                Color32::from_rgb(
                                    6 + (ratio * 50.0) as u8,
                                    182 + (ratio * 30.0) as u8,
                                    212 - (ratio * 40.0) as u8,
                                )
                            } else {
                                Color32::from_rgb(60, 70, 95)
                            };

                            let bar_rect = Rect::from_min_max(
                                Pos2::new(x, center_y - height_factor),
                                Pos2::new(x + bar_width, center_y + height_factor),
                            );

                            ui.painter().rect_filled(bar_rect, Rounding::same(3.0), bar_color);
                        }

                        ui.add_space(14.0);

                        // Hero Action Button
                        let dictate_btn_text = format!("🎙️  START DICTATION (OR HOLD {})", dictate_key_name);
                        let (btn_text, btn_bg) = if is_rec {
                            ("⏹️  CLICK TO STOP & INSERT", Color32::from_rgb(220, 38, 38))
                        } else if status == DictationStatus::Transcribing {
                            ("✨  TRANSCRIBING WITH AI...", Color32::from_rgb(124, 58, 237))
                        } else {
                            (dictate_btn_text.as_str(), Color32::from_rgb(79, 70, 229))
                        };

                        let hero_btn = egui::Button::new(
                            egui::RichText::new(btn_text)
                                .font(FontId::proportional(14.0))
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(btn_bg)
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
                        let tip_text = format!("💡 Tip: Press and hold {} anywhere in Windows to speak into any text box.", dictate_key_name);
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
                    .fill(Color32::from_rgb(20, 24, 35))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(40, 48, 68)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("ACTIVE WINDOW CONTEXT")
                                .font(FontId::proportional(12.0))
                                .color(Color32::from_rgb(148, 163, 184))
                                .strong(),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            ui.label("🎯 Focused App:");
                            ui.label(
                                egui::RichText::new(if active_app.process_name.is_empty() {
                                    "Waiting for focus..."
                                } else {
                                    &active_app.process_name
                                })
                                .color(Color32::from_rgb(56, 189, 248))
                                .strong(),
                            );
                        });

                        if !active_app.window_title.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("Window: {}", active_app.window_title))
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(156, 163, 175)),
                            );
                        }

                        if let Some(before) = &active_app.before_text {
                            if !before.is_empty() {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(format!("Context before cursor: \"{}\"", before))
                                        .font(FontId::proportional(11.0))
                                        .italics()
                                        .color(Color32::from_rgb(203, 213, 225)),
                                );
                            }
                        }
                    });
            });

            // RIGHT COLUMN: Live Transcript & Quick Actions
            cols[1].vertical(|ui| {
                egui::Frame::none()
                    .fill(Color32::from_rgb(20, 24, 35))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(40, 48, 68)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(16.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("LATEST TRANSCRIPTION")
                                    .font(FontId::proportional(12.0))
                                    .color(Color32::from_rgb(148, 163, 184))
                                    .strong(),
                            );

                            let latency = *self.state.last_latency.lock().unwrap();
                            if latency > 0.0 {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("⚡ {:.0} ms", latency * 1000.0))
                                            .font(FontId::proportional(11.0))
                                            .color(Color32::from_rgb(74, 222, 128))
                                            .strong(),
                                    );
                                });
                            }
                        });

                        ui.add_space(8.0);

                        let transcript = self.state.last_transcript.lock().unwrap().clone();

                        egui::ScrollArea::vertical()
                            .max_height(200.0)
                            .show(ui, |ui| {
                                if transcript.is_empty() {
                                    ui.add_space(30.0);
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            egui::RichText::new("No recent transcript.")
                                                .font(FontId::proportional(13.0))
                                                .color(Color32::from_rgb(100, 116, 139)),
                                        );
                                        let empty_hint = format!("Press {} and start speaking.", dictate_key_name);
                                        ui.label(
                                            egui::RichText::new(empty_hint)
                                                .font(FontId::proportional(11.0))
                                                .color(Color32::from_rgb(71, 85, 105)),
                                        );
                                    });
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
                                let copy_btn = egui::Button::new("📋 Copy to Clipboard")
                                    .fill(Color32::from_rgb(45, 55, 78))
                                    .rounding(Rounding::same(6.0));

                                if ui.add(copy_btn).clicked() {
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(&transcript);
                                        self.state.set_toast("Copied to clipboard!");
                                    }
                                }

                                let word_count = transcript.split_whitespace().count();
                                ui.label(
                                    egui::RichText::new(format!("Words: {}", word_count))
                                        .font(FontId::proportional(11.0))
                                        .color(Color32::from_rgb(148, 163, 184)),
                                );
                            }
                        });
                    });

                ui.add_space(10.0);

                // Quick Stats Card
                let history_count = self.state.history.lock().unwrap().len();
                egui::Frame::none()
                    .fill(Color32::from_rgb(20, 24, 35))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(40, 48, 68)))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("📊 Database History:");
                            ui.label(
                                egui::RichText::new(format!("{} transcripts recorded", history_count))
                                    .color(Color32::from_rgb(167, 139, 250))
                                    .strong(),
                            );
                        });
                        ui.label(
                            egui::RichText::new("Stored in %APPDATA%\\Wispr Flow\\flow.sqlite")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(100, 116, 139)),
                        );
                    });
            });
        });
    }

    fn render_history_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Search transcripts...")
                    .desired_width(ui.available_width() - 120.0),
            );

            let refresh_btn = egui::Button::new("🔄 Refresh")
                .fill(Color32::from_rgb(45, 55, 78))
                .rounding(Rounding::same(6.0));
            if ui.add(refresh_btn).clicked() {
                let _ = self.event_tx.send(GuiEvent::RefreshHistory);
                self.state.set_toast("History refreshed!");
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        let history = self.state.history.lock().unwrap().clone();
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
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("No matching dictations found.")
                        .font(FontId::proportional(14.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                );
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for rec in filtered {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(22, 27, 38))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 53, 73)))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(12.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // App badge
                                let app_name = if rec.app.is_empty() {
                                    "Desktop".to_string()
                                } else {
                                    rec.app.clone()
                                };

                                let badge = ui.label(
                                    egui::RichText::new(format!("📱 {}", app_name))
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
                                    let copy_btn = egui::Button::new("📋 Copy")
                                        .fill(Color32::from_rgb(45, 55, 78))
                                        .rounding(Rounding::same(4.0));
                                    if ui.add(copy_btn).clicked() {
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            let _ = clipboard.set_text(&rec.formatted_text);
                                            self.state.set_toast("Copied to clipboard!");
                                        }
                                    }

                                    let del_btn = egui::Button::new("🗑️")
                                        .fill(Color32::from_rgb(60, 20, 20))
                                        .rounding(Rounding::same(4.0));
                                    if ui.add(del_btn).clicked() {
                                        let _ = self.event_tx.send(GuiEvent::DeleteHistory(rec.transcript_id.clone()));
                                        self.state.set_toast("Item deleted");
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

    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::none()
                .fill(Color32::from_rgb(20, 24, 35))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(40, 48, 68)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(18.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("HOTKEY & SHORTCUT PREFERENCES")
                            .font(FontId::proportional(13.0))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(8.0);

                    // 1. Push-To-Talk Dictation Hotkey
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("🎙️ Push-To-Talk Hotkey:")
                                    .font(FontId::proportional(13.0))
                                    .strong(),
                            );
                            let curr_name = crate::platform::vk_to_string(self.settings_hotkey_vk);
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
                            let curr_label = format!("Preset: {}", crate::platform::vk_to_string(self.settings_hotkey_vk));
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
                                "🔴 Press any key... (Esc to cancel)"
                            } else {
                                "⌨️ Press key to set"
                            };
                            let rec_fill = if is_rec {
                                Color32::from_rgb(225, 29, 72)
                            } else {
                                Color32::from_rgb(51, 65, 85)
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
                                    crate::platform::PlatformManager::stop_recording_hotkey();
                                    self.state.set_toast("Key recording cancelled");
                                } else {
                                    self.recording_mode = Some(1);
                                    crate::platform::PlatformManager::start_recording_hotkey(1);
                                    self.state.set_toast("Press any key on your keyboard to set Dictation hotkey... (Esc to cancel)");
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
                            let curr_name = crate::platform::vk_to_string(self.settings_toggle_vk);
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
                            egui::RichText::new("Press this key anywhere to show or hide the Wispr Flow window to system tray.")
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let curr_label = format!("Preset: {}", crate::platform::vk_to_string(self.settings_toggle_vk));
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
                                "🔴 Press any key... (Esc to cancel)"
                            } else {
                                "⌨️ Press key to set"
                            };
                            let rec_fill = if is_rec {
                                Color32::from_rgb(225, 29, 72)
                            } else {
                                Color32::from_rgb(51, 65, 85)
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
                                    crate::platform::PlatformManager::stop_recording_hotkey();
                                    self.state.set_toast("Key recording cancelled");
                                } else {
                                    self.recording_mode = Some(2);
                                    crate::platform::PlatformManager::start_recording_hotkey(2);
                                    self.state.set_toast("Press any key on your keyboard to set Window Toggle hotkey... (Esc to cancel)");
                                }
                            }
                        });
                    });

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

                    // Auto Paste Checkbox
                    ui.checkbox(
                        &mut self.settings_auto_paste,
                        "Auto-paste transcribed text into active cursor position",
                    );

                    ui.add_space(6.0);
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

                    ui.add_space(18.0);
                    ui.separator();
                    ui.add_space(14.0);

                    // Speech-to-Text Provider Selector
                    ui.label(
                        egui::RichText::new("SPEECH-TO-TEXT (STT) AI SERVICE PROVIDER")
                            .font(FontId::proportional(13.0))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        let providers = [
                            (crate::stt::SttProvider::WisprFlow, "⚡ Wispr Flow Cloud"),
                            (crate::stt::SttProvider::Groq, "🚀 Groq Whisper"),
                            (crate::stt::SttProvider::ElevenLabs, "🎙️ ElevenLabs Scribe"),
                        ];
                        for (p, label) in providers {
                            let is_sel = self.settings_stt_provider == p;
                            let btn = egui::Button::new(
                                egui::RichText::new(label)
                                    .font(FontId::proportional(12.5))
                                    .color(if is_sel { Color32::WHITE } else { Color32::from_rgb(156, 163, 175) })
                                    .strong(),
                            )
                            .fill(if is_sel { Color32::from_rgb(79, 70, 229) } else { Color32::from_rgb(30, 36, 52) })
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(130.0, 30.0));

                            if ui.add(btn).clicked() {
                                self.settings_stt_provider = p;
                            }
                        }
                    });

                    ui.add_space(12.0);

                    match self.settings_stt_provider {
                        crate::stt::SttProvider::WisprFlow => {
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
                        crate::stt::SttProvider::Groq => {
                            ui.label("Groq Cloud API Key (get free at console.groq.com):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_groq_api_key)
                                    .password(true)
                                    .hint_text("gsk_...")
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);
                            ui.label("Groq Whisper Model:");
                            egui::ComboBox::from_id_salt("groq_model_combo")
                                .selected_text(&self.settings_groq_model)
                                .show_ui(ui, |ui| {
                                    let models = [
                                        ("whisper-large-v3-turbo", "whisper-large-v3-turbo (Ultra-fast, multilingual) [Recommended]"),
                                        ("whisper-large-v3", "whisper-large-v3 (Full accuracy, multilingual)"),
                                        ("distil-whisper-large-v3-en", "distil-whisper-large-v3-en (English only, ultra-light)"),
                                    ];
                                    for (m, desc) in models {
                                        if ui.selectable_label(self.settings_groq_model == m, desc).clicked() {
                                            self.settings_groq_model = m.to_string();
                                        }
                                    }
                                });
                            ui.label(
                                egui::RichText::new("Groq executes Whisper on LPUs with near-zero latency (~200ms per utterance).")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }
                        crate::stt::SttProvider::ElevenLabs => {
                            ui.label("ElevenLabs API Key (get from elevenlabs.io):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_elevenlabs_api_key)
                                    .password(true)
                                    .hint_text("xi-api-key")
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);
                            ui.label("ElevenLabs STT Model:");
                            egui::ComboBox::from_id_salt("elevenlabs_model_combo")
                                .selected_text(&self.settings_elevenlabs_model)
                                .show_ui(ui, |ui| {
                                    let models = [
                                        ("scribe_v1", "scribe_v1 (State-of-the-art multilingual STT) [Recommended]"),
                                    ];
                                    for (m, desc) in models {
                                        if ui.selectable_label(self.settings_elevenlabs_model == m, desc).clicked() {
                                            self.settings_elevenlabs_model = m.to_string();
                                        }
                                    }
                                });
                            ui.label(
                                egui::RichText::new("ElevenLabs Scribe STT provides unmatched speech recognition accuracy across languages.")
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }
                    }

                    ui.add_space(18.0);
                    ui.separator();
                    ui.add_space(14.0);

                    // Audio Device Info
                    ui.label(
                        egui::RichText::new("AUDIO HARDWARE")
                            .font(FontId::proportional(13.0))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(6.0);
                    ui.label("Capture Device: Default Windows Input Device (WASAPI)");
                    ui.label("Stream Sampling: 16,000 Hz, 1-channel Mono PCM, 100ms frames");

                    ui.add_space(20.0);

                    // Save Button
                    let save_btn = egui::Button::new(
                        egui::RichText::new("💾 Save Preferences")
                            .font(FontId::proportional(14.0))
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(79, 70, 229))
                    .rounding(Rounding::same(8.0))
                    .min_size(Vec2::new(160.0, 36.0));

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
                            writing_style: "default".to_string(),
                            language_code: self.settings_language.clone(),
                            auto_paste: self.settings_auto_paste,
                            auth_token: if self.settings_token.is_empty() {
                                None
                            } else {
                                Some(self.settings_token.clone())
                            },
                        };

                        let _ = self.event_tx.send(GuiEvent::SaveConfig(new_config));
                        self.state.set_toast("Settings, STT provider & hotkeys saved successfully!");
                    }
                });
        });
    }

    fn render_toast(&self, ui: &mut egui::Ui) {
        if let Ok(mut lock) = self.state.toast.lock() {
            if let Some((msg, time)) = lock.clone() {
                if time.elapsed().as_secs_f32() < 2.5 {
                    let rect = ui.max_rect();
                    let toast_pos = Pos2::new(rect.center().x - 120.0, rect.max.y - 45.0);
                    let toast_rect = Rect::from_min_size(toast_pos, Vec2::new(240.0, 32.0));

                    ui.painter().rect_filled(
                        toast_rect,
                        Rounding::same(8.0),
                        Color32::from_rgb(30, 41, 59),
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
                        FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                } else {
                    *lock = None;
                }
            }
        }
    }
}
