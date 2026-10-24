use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use tokio::sync::mpsc;

use crate::config::{AppConfig, KeyCombo};
use crate::db::{DictionaryEntry, HistoryRecord};
use crate::i18n::{tr, Lang};
use crate::icons::Icon;
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

/// The sections of the settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Language,
    Shortcuts,
    Audio,
    Ai,
    Voice,
    Privacy,
    Updates,
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
    /// Download, verify and install the pending update, then restart.
    InstallUpdate,
    OpenUrl(String),
    OpenLogFolder,
    /// Persist a new interface language immediately.
    SetUiLanguage(String),
}

/// Locks a mutex, recovering from poisoning instead of panicking the GUI.
///
/// A panic in any worker thread would otherwise poison the shared state and
/// take the whole interface down.
/// Font for headings, page titles and emphasised numbers.
///
/// egui's `.strong()` only tints text; it does not make it heavier. A real
/// semibold face is registered as the "Heading" family so the interface has a
/// visible hierarchy between titles and body copy.
fn heading(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Heading".into()))
}

/// Muted label colour used by section headings and secondary copy.
const MUTED: Color32 = Color32::from_rgb(148, 163, 184);
/// Card surface and border, shared by every panel on the studio screen.
const SURFACE: Color32 = Color32::from_rgb(19, 24, 38);
const SURFACE_LINE: Color32 = Color32::from_rgb(35, 45, 68);

/// Draws a line icon and a label side by side.
///
/// Replaces the emoji that used to prefix these labels: emoji render with a
/// different colour and shape on every machine and compete with the text for
/// attention, while a monochrome line icon reads as part of the interface.
fn icon_label(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32, text: &str, font: FontId) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        crate::icons::draw(ui.painter(), rect.center(), size, color, icon);
        ui.label(egui::RichText::new(text).font(font).color(color));
    });
}

/// A small pill: rounded background with an icon and text inside.
#[allow(clippy::too_many_arguments)]
fn icon_pill(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    text: &str,
    fg: Color32,
    bg: Color32,
    size: f32,
    font: FontId,
) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), fg);
    let icon_w = if icon.is_some() { size + 6.0 } else { 0.0 };
    let pad_x = 9.0;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(
            galley.size().x + icon_w + pad_x * 2.0,
            galley.size().y + 8.0,
        ),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, Rounding::same(rect.height() / 2.0), bg);
    let mut x = rect.min.x + pad_x;
    if let Some(ic) = icon {
        crate::icons::draw(
            ui.painter(),
            Pos2::new(x + size / 2.0, rect.center().y),
            size,
            fg,
            ic,
        );
        x += size + 6.0;
    }
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        fg,
    );
}

/// A clickable button drawn with a line icon and a label.
///
/// Returns `true` when the button was clicked.
#[allow(clippy::too_many_arguments)]
fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    text: &str,
    height: f32,
    fill: Color32,
    stroke_color: Color32,
    fg: Color32,
    font: FontId,
    icon_size: f32,
    centered: bool,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let rect = rect.shrink(0.5);

    let bg = if response.dragged() {
        fill
    } else if response.hovered() {
        fill.linear_multiply(1.18)
    } else {
        fill
    };
    let rounding = Rounding::same(10.0);
    ui.painter().rect_filled(rect, rounding, bg);
    ui.painter().rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0_f32, if response.hovered() { fg } else { stroke_color }),
    );

    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), fg);
    let group_w = icon_size + 9.0 + galley.size().x;
    let start_x = if centered {
        rect.center().x - group_w / 2.0
    } else {
        rect.min.x + 14.0
    };

    crate::icons::draw(
        ui.painter(),
        Pos2::new(start_x + icon_size / 2.0, rect.center().y),
        icon_size,
        fg,
        icon,
    );
    ui.painter().text(
        Pos2::new(start_x + icon_size + 9.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        fg,
    );

    response.clicked()
}

/// A compact clickable button that is only an icon (row actions, toolbars).
fn icon_button_small(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    fill: Color32,
    stroke_color: Color32,
    fg: Color32,
    tooltip: &str,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let rect = rect.shrink(0.5);
    let bg = if response.hovered() {
        fill.linear_multiply(1.35)
    } else {
        fill
    };
    ui.painter().rect_filled(rect, Rounding::same(7.0), bg);
    ui.painter().rect_stroke(
        rect,
        Rounding::same(7.0),
        Stroke::new(1.0_f32, if response.hovered() { fg } else { stroke_color }),
    );
    crate::icons::draw(ui.painter(), rect.center(), size * 0.52, fg, icon);
    if response.hovered() && !tooltip.is_empty() {
        response.clone().show_tooltip_text(tooltip);
    }
    response.clicked()
}

/// True when a font file carries the WTT 2.0 PUA glyphs (0xF700..0xF71A) that
/// `thai::shape_thai` maps Thai vowels and tone marks onto.
///
/// The shaping only helps fonts that actually own those glyphs — the TLWG
/// family on Linux. Noto Sans Thai, the font `fc-match :lang=th` usually
/// returns, does not have them, and neither does Leelawadee UI or Tahoma on
/// Windows. Asking the real file is the only way to know, because the answer
/// decides whether shaping fixes "สระลอย" or turns the whole interface into
/// tofu boxes.
fn font_has_thai_pua(bytes: &[u8]) -> bool {
    use ab_glyph::Font as _;
    let Ok(font) = ab_glyph::FontArc::try_from_vec(bytes.to_vec()) else {
        return false;
    };
    // The block is either present in full or absent; probing every entry is
    // still cheap (27 lookups) and avoids guessing.
    ('\u{F700}'..='\u{F71A}').all(|c| font.glyph_id(c) != ab_glyph::GlyphId(0))
}

/// Fills the single `{}` of a translated template.
///
/// `format!` only accepts string literals, so translated templates are
/// substituted at runtime instead.
fn tr1(lang: Lang, en: &'static str, value: impl std::fmt::Display) -> String {
    tr(lang, en).replacen("{}", &value.to_string(), 1)
}

/// Fills two `{}` placeholders in a translated template, left to right.
fn tr2(
    lang: Lang,
    en: &'static str,
    first: impl std::fmt::Display,
    second: impl std::fmt::Display,
) -> String {
    tr(lang, en)
        .replacen("{}", &first.to_string(), 1)
        .replacen("{}", &second.to_string(), 1)
}

/// Turns the RFC 3339 timestamp stored in the database into the compact
/// `22/10/2026 15:58` form shown in the history list.
fn friendly_timestamp(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(ts) => ts
            .with_timezone(&chrono::Local)
            .format("%d/%m/%Y %H:%M")
            .to_string(),
        Err(_) => raw.to_string(),
    }
}

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
    /// The release version the user chose to skip.
    ///
    /// This lives outside of `config` on purpose: `pending_update()` is called
    /// from the sidebar while the config mutex is already held, and a
    /// `std::sync::Mutex` is not reentrant, so reading the skipped version
    /// from the config there would deadlock the whole interface.
    pub skipped_update_version: Arc<Mutex<Option<String>>>,
    /// Progress of the running self-update (download → verify → install).
    pub update_install: crate::updater::SharedInstallState,
    /// The active noise gate, so the studio can show the voice activity.
    pub voice_gate: Arc<Mutex<Option<Arc<Mutex<crate::vad::VoiceGate>>>>>,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        initial_history: Vec<HistoryRecord>,
        initial_dict: Vec<DictionaryEntry>,
    ) -> Self {
        let skipped_update_version = config.skipped_update_version.clone();
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
            skipped_update_version: Arc::new(Mutex::new(skipped_update_version)),
            update_install: Arc::new(Mutex::new(crate::updater::InstallState::default())),
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
    settings_ui_language: String,
    /// Interface language resolved from `settings_ui_language`.
    ui_lang: Lang,
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
    settings_auto_update: bool,
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
    /// The section of the settings page that is currently shown.
    settings_section: SettingsSection,
    /// The application's long-lived Tokio runtime, used for background work.
    runtime: tokio::runtime::Handle,
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
    pub fn new(
        state: AppState,
        event_tx: mpsc::UnboundedSender<GuiEvent>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let (
            ui_language,
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
            auto_update,
            include_prerelease,
            update_interval_hours,
        ) = {
            let cfg = lock(&state.config);
            (
                cfg.ui_language.clone(),
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
                cfg.auto_update,
                cfg.include_prerelease,
                cfg.update_check_interval_hours,
            )
        };

        Self {
            state,
            event_tx,
            runtime,
            active_tab: ActiveTab::Studio,
            search_query: String::new(),
            settings_ui_language: ui_language.clone(),
            ui_lang: Lang::from_code(&ui_language),
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
            settings_auto_update: auto_update,
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
            settings_section: SettingsSection::Shortcuts,
        }
    }

    fn configure_thai_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();

        let mut loaded_primary = false;
        let mut loaded_thai = false;
        let mut loaded_cjk = false;
        let loaded_emoji = true;
        let mut loaded_bold = false;
        // Set from the font files themselves: Thai PUA shaping is only safe
        // when the face that will draw the text owns those glyphs.
        let mut thai_pua = false;

        // 1. Primary UI Latin font (Segoe UI on Windows, Ubuntu/Liberation/DejaVu on Linux)
        #[cfg(target_os = "windows")]
        {
            let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
            let font_dir = std::path::Path::new(&win_dir).join("Fonts");
            let segoe_candidates = [font_dir.join("SegUIVar.ttf"), font_dir.join("segoeui.ttf")];
            for path in &segoe_candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(path) {
                        thai_pua |= font_has_thai_pua(&bytes);
                        fonts
                            .font_data
                            .insert("primary_ui".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_primary = true;
                        break;
                    }
                }
            }
        }
        // Resolves a fontconfig family to a real file. Asking for a named
        // family (instead of the generic `sans-serif`) matters: on some
        // desktops the generic alias points at a display or monospace face,
        // which makes the whole UI look wrong.
        #[cfg(not(target_os = "windows"))]
        let fc_match = |family: &str| -> Option<std::path::PathBuf> {
            let out = std::process::Command::new("fc-match")
                .args(["-f", "%{file}", family])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if path.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(path))
            }
        };

        #[cfg(not(target_os = "windows"))]
        {
            let mut candidates: Vec<std::path::PathBuf> = Vec::new();
            for family in [
                "Inter",
                "Noto Sans",
                "Ubuntu",
                "Cantarell",
                "Liberation Sans",
                "DejaVu Sans",
            ] {
                if let Some(path) = fc_match(family) {
                    candidates.push(path);
                }
            }
            candidates.extend(vec![
                std::path::PathBuf::from(
                    "/usr/share/fonts/truetype/inter-zorin-os/Inter-Regular.ttf",
                ),
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
                        thai_pua |= font_has_thai_pua(&bytes);
                        fonts
                            .font_data
                            .insert("primary_ui".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_primary = true;
                        tracing::info!("Loaded primary UI font from {:?}", path);
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
                        // Leelawadee UI and Tahoma have no PUA block, so this
                        // is the check that stops shaping on Windows.
                        thai_pua |= font_has_thai_pua(&bytes);
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
                        // Garuda has a deeper ascent than Inter, which made
                        // Thai text sit visually lower than the Latin text on
                        // the same line. A small scale + upward shift lines
                        // the two scripts up.
                        thai_pua |= font_has_thai_pua(&bytes);
                        let thai = egui::FontData::from_owned(bytes).tweak(egui::FontTweak {
                            scale: 0.96,
                            y_offset_factor: -0.08,
                            y_offset: 0.0,
                            baseline_offset_factor: 0.0,
                        });
                        fonts.font_data.insert("thai_font".to_owned(), thai);
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

        // 4. Emoji / pictogram font.
        //
        // The UI uses emoji as small icons. Colour emoji fonts (Noto Color
        // Emoji, Segoe UI Emoji) store bitmap glyphs that egui cannot render
        // as outlines, which is exactly what produced the empty tofu boxes.
        // A monochrome outline font is bundled instead so every glyph is
        // available on every platform.
        fonts.font_data.insert(
            "emoji_font".to_owned(),
            egui::FontData::from_static(include_bytes!("../assets/NotoEmoji-Regular.ttf")),
        );
        // 5. A real semibold face for headings. egui's `.strong()` only tints
        // text, so a heavier cut is loaded to give the UI a clear hierarchy.
        #[cfg(target_os = "windows")]
        {
            let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
            let font_dir = std::path::Path::new(&win_dir).join("Fonts");
            let bold_candidates = [font_dir.join("segoeuib.ttf"), font_dir.join("seguisb.ttf")];
            for path in &bold_candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(path) {
                        fonts
                            .font_data
                            .insert("bold_ui".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_bold = true;
                        break;
                    }
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut bold_candidates: Vec<std::path::PathBuf> = Vec::new();
            if let Some(path) = fc_match("Inter:style=SemiBold") {
                bold_candidates.push(path);
            }
            bold_candidates.extend(vec![
                std::path::PathBuf::from(
                    "/usr/share/fonts/truetype/inter-zorin-os/Inter-SemiBold.ttf",
                ),
                std::path::PathBuf::from("/usr/share/fonts/truetype/ubuntu/Ubuntu-M.ttf"),
                std::path::PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
            ]);
            for path in bold_candidates {
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(&path) {
                        fonts
                            .font_data
                            .insert("bold_ui".to_owned(), egui::FontData::from_owned(bytes));
                        loaded_bold = true;
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

        // Heading family: the same fallback chain, but with the semibold face
        // first so headings actually look heavier than body text.
        {
            let mut heading: Vec<String> = fonts
                .families
                .get(&egui::FontFamily::Proportional)
                .cloned()
                .unwrap_or_default();
            if loaded_bold {
                heading.insert(0, "bold_ui".to_owned());
            }
            fonts
                .families
                .insert(egui::FontFamily::Name("Heading".into()), heading);
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

        // Decide once, from the fonts that are actually installed, whether the
        // PUA shaping is safe. Getting this wrong is what produced tofu boxes
        // for every Thai vowel on machines without a TLWG font.
        crate::thai::set_pua_support(thai_pua);
        tracing::info!("Thai PUA shaping enabled: {}", thai_pua);

        if loaded_primary || loaded_thai || loaded_cjk {
            ctx.set_fonts(fonts);
            tracing::info!(
                "Successfully applied typography: primary={}, thai={}, cjk={}, emoji={}, bold={}",
                loaded_primary,
                loaded_thai,
                loaded_cjk,
                loaded_emoji,
                loaded_bold
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
            // Font definitions are swapped in at the start of the next pass.
            // Drawing this frame would still use the default families and
            // panic on the custom "Heading" family, so skip one frame.
            ctx.request_repaint();
            return;
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
                    self.state
                        .set_toast(tr(self.ui_lang, "Key recording cancelled"));
                } else {
                    let key_name = combo.format_display();
                    if mode == 1 {
                        self.settings_hotkey = combo;
                        self.state.set_toast(&tr1(
                            self.ui_lang,
                            "Dictation hotkey changed to: {}",
                            key_name,
                        ));
                    } else if mode == 2 {
                        self.settings_toggle = combo;
                        self.state.set_toast(&tr1(
                            self.ui_lang,
                            "Window toggle hotkey changed to: {}",
                            key_name,
                        ));
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
                self.state
                    .set_toast(tr(self.ui_lang, "Key recording cancelled"));
            }
        }

        let (dictate_key_name, toggle_key_name) = {
            let cfg = lock(&self.state.config);
            (
                cfg.hotkey.format_display(),
                cfg.toggle_window.format_display(),
            )
        };

        if self.state.pending_quit.load(Ordering::SeqCst) || crate::platform::PlatformManager::is_quitting() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Window close [X] -> minimize to tray or show exit confirmation modal
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.state.pending_quit.load(Ordering::SeqCst) || crate::platform::PlatformManager::is_quitting() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
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
        self.ui_lang = Lang::from_code(&self.settings_ui_language);
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

        // In-window fallback for the dictation hotkey.
        //
        // On X11 and Windows a system-wide hook already delivers the key, so
        // running this as well would toggle dictation twice per press (start,
        // then an immediate stop). It is only used where no global hook exists,
        // for example a Wayland session.
        let hotkey_vk = self
            .state
            .config
            .lock()
            .map(|c| c.hotkey.key)
            .unwrap_or(0x77);
        if !PlatformManager::has_global_hotkeys()
            && self.recording_mode.is_none()
            && egui_key_from_vk(hotkey_vk)
                .map(|key| {
                    ctx.input(|i| {
                        // Ignore auto-repeat, which would otherwise toggle on
                        // every repeat while the key is held down.
                        i.events.iter().any(|event| {
                            matches!(
                                event,
                                egui::Event::Key {
                                    key: pressed_key,
                                    pressed: true,
                                    repeat: false,
                                    ..
                                } if *pressed_key == key
                            )
                        })
                    })
                })
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
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                self.render_studio_tab(
                                    ui,
                                    is_rec,
                                    amplitude,
                                    status,
                                    &dictate_key_name,
                                );
                            });
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
    lang: Lang,
    latest: Option<&str>,
    running: bool,
    current_version: &str,
    animation_time: f32,
) -> (String, Color32, Color32, Color32) {
    if let Some(latest) = latest {
        // Gentle pulse so the action is impossible to miss.
        let pulse = 0.5 + 0.5 * (animation_time * 3.0).sin();
        return (
            format!("↑  {}", tr1(lang, "New update v{}", latest)),
            Color32::from_rgb(22, 163, 74),
            Color32::from_rgb(74, 222, 128).gamma_multiply(0.65 + 0.35 * pulse),
            Color32::WHITE,
        );
    }
    if running {
        return (
            format!("·  {}", tr(lang, "Checking...")),
            Color32::from_rgb(30, 41, 59),
            Color32::from_rgb(100, 116, 139),
            Color32::from_rgb(203, 213, 225),
        );
    }
    (
        format!("↻  {}", tr1(lang, "Update · v{}", current_version)),
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
        let lang = self.ui_lang;
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
                    crate::icons::draw(
                        ui.painter(),
                        logo_rect.center(),
                        17.0,
                        Color32::WHITE,
                        Icon::Mic,
                    );
                }

                ui.add_space(8.0);

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("WFLOW")
                                .font(heading(17.0))
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
                        egui::RichText::new(tr(lang, "AI Voice Studio"))
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
                (ActiveTab::Studio, Icon::Mic, tr(lang, "Live Studio"), None),
                (
                    ActiveTab::History,
                    Icon::History,
                    tr(lang, "History"),
                    Some(history_count),
                ),
                (
                    ActiveTab::Dictionary,
                    Icon::Book,
                    tr(lang, "Dictionary"),
                    Some(dict_count),
                ),
                (
                    ActiveTab::Updates,
                    Icon::Download,
                    tr(lang, "Updates"),
                    if update_badge { Some(1) } else { None },
                ),
                (ActiveTab::Settings, Icon::Gear, tr(lang, "Settings"), None),
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

                let icon_pos = Pos2::new(btn_rect.min.x + 19.0, btn_rect.center().y);
                crate::icons::draw(
                    ui.painter(),
                    icon_pos,
                    16.0,
                    if is_active {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(148, 163, 184)
                    },
                    icon,
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
                if icon_button(
                    ui,
                    Icon::Power,
                    shape_thai(tr(lang, "Exit")).as_ref(),
                    30.0,
                    Color32::from_rgb(45, 18, 22),
                    Color32::from_rgb(185, 28, 28),
                    Color32::from_rgb(252, 165, 165),
                    FontId::proportional(11.5),
                    13.0,
                    false,
                ) {
                    self.show_exit_confirmation = true;
                }

                ui.add_space(4.0);

                // 2. Minimize Button
                let minimize_label =
                    shape_thai(&tr1(lang, "Minimize ({})", toggle_key_name)).to_string();
                if icon_button(
                    ui,
                    Icon::Collapse,
                    &minimize_label,
                    34.0,
                    Color32::from_rgb(22, 29, 44),
                    Color32::from_rgb(38, 50, 74),
                    Color32::from_rgb(203, 213, 225),
                    FontId::proportional(11.5),
                    13.0,
                    false,
                ) {
                    #[cfg(target_os = "windows")]
                    crate::platform::windows::PlatformManager::hide_main_window();
                    #[cfg(not(target_os = "windows"))]
                    {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        crate::platform::PlatformManager::hide_main_window();
                    }
                    self.state.window_visible.store(false, Ordering::SeqCst);
                    self.state.set_toast(&tr1(
                        lang,
                        "Minimized to Tray. Press {} to reopen.",
                        toggle_key_name,
                    ));
                }

                ui.add_space(8.0);

                // Status info card
                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 41, 64)))
                    .rounding(Rounding::same(9.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        // The sidebar footer uses a bottom-up layout, which
                        // would reverse these rows. Give the card its own
                        // normal top-down flow.
                        ui.vertical(|ui| {
                            // Read the few settings the card needs and
                            // release the config lock right away: holding it
                            // across the whole card would block the settings
                            // page and the update checker.
                            let (prov_badge, mic_label) = {
                                let cfg = lock(&self.state.config);
                                let prov_badge = match cfg.stt_provider {
                                    SttProvider::WisprFlow => {
                                        (Icon::Bolt, "Wispr Flow", Color32::from_rgb(129, 140, 248))
                                    }
                                    SttProvider::Groq => (
                                        Icon::Activity,
                                        "Groq Whisper",
                                        Color32::from_rgb(251, 146, 60),
                                    ),
                                    SttProvider::ElevenLabs => (
                                        Icon::Waveform,
                                        "ElevenLabs",
                                        Color32::from_rgb(192, 132, 252),
                                    ),
                                };

                                let mic_label = if let Some(ref m) = cfg.input_device_name {
                                    if m.len() > 18 {
                                        format!("{}...", &m[..15])
                                    } else {
                                        m.clone()
                                    }
                                } else {
                                    tr(lang, "Default Mic").to_string()
                                };
                                (prov_badge, mic_label)
                            };

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(tr(lang, "ENGINE"))
                                        .font(FontId::proportional(9.0))
                                        .color(Color32::from_rgb(148, 163, 184)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(prov_badge.1)
                                                .font(FontId::proportional(10.5))
                                                .color(prov_badge.2)
                                                .strong(),
                                        );
                                        let (icon_rect, _) = ui
                                            .allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                                        crate::icons::draw(
                                            ui.painter(),
                                            icon_rect.center(),
                                            12.0,
                                            prov_badge.2,
                                            prov_badge.0,
                                        );
                                    },
                                );
                            });

                            ui.add_space(4.0);

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(tr(lang, "MIC"))
                                        .font(FontId::proportional(9.0))
                                        .color(Color32::from_rgb(148, 163, 184)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(&mic_label)
                                                .font(FontId::proportional(10.5))
                                                .color(Color32::from_rgb(203, 213, 225)),
                                        );
                                        let (icon_rect, _) = ui
                                            .allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                                        crate::icons::draw(
                                            ui.painter(),
                                            icon_rect.center(),
                                            12.0,
                                            Color32::from_rgb(148, 163, 184),
                                            Icon::Mic,
                                        );
                                    },
                                );
                            });

                            ui.add_space(4.0);

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(tr(lang, "HOTKEY"))
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
                                    format!("v{} ↑ v{}", crate::update::current_version(), latest),
                                    Color32::from_rgb(74, 222, 128),
                                ),
                                None => (
                                    format!("v{}", crate::update::current_version()),
                                    Color32::from_rgb(203, 213, 225),
                                ),
                            };

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(tr(lang, "VERSION"))
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
                                                let _ = self
                                                    .event_tx
                                                    .send(GuiEvent::OpenUrl(url.clone()));
                                                self.state.set_toast(&tr1(
                                                    lang,
                                                    "Opening the download page for v{}",
                                                    latest,
                                                ));
                                            } else {
                                                let _ = self.event_tx.send(GuiEvent::CheckUpdate);
                                                self.state
                                                    .set_toast(tr(lang, "Checking for updates..."));
                                            }
                                        }
                                        if response.hovered() {
                                            response.show_tooltip_text(tr(
                                                lang,
                                                "Click to check for or download an update",
                                            ));
                                        }
                                    },
                                );
                            });
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
        let lang = self.ui_lang;
        // The update button never moves to another tab and is never hidden.
        // The decision whether it needs its own row is made from the full
        // width of the panel, *before* the title takes up any space.
        let update_available = self.pending_update();
        let update_running = self.state.update_check_running.load(Ordering::SeqCst);
        let current_version = crate::update::current_version().to_string();
        let (update_label, update_fill, update_stroke, update_text) = update_button_style(
            self.ui_lang,
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
                        tr(self.ui_lang, "Download the new version")
                    } else {
                        tr(self.ui_lang, "Check for updates")
                    };
                    response.show_tooltip_text(tip);
                }
            });
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            // Each page carries the same glyph as its sidebar entry, so the
            // icon doubles as a "you are here" cue.
            let (title_icon, title, subtitle) = match self.active_tab {
                ActiveTab::Studio => (
                    Icon::Mic,
                    tr(lang, "Live Speech Studio"),
                    tr(lang, "Zero-latency real-time voice typing"),
                ),
                ActiveTab::History => (
                    Icon::History,
                    tr(lang, "Dictation History"),
                    tr(lang, "Search, copy, or re-paste past speech"),
                ),
                ActiveTab::Dictionary => (
                    Icon::Book,
                    tr(lang, "Custom Vocabulary & Snippets"),
                    tr(lang, "Domain terminology and text expansion shortcuts"),
                ),
                ActiveTab::Updates => (
                    Icon::Download,
                    tr(lang, "Software Updates"),
                    tr(lang, "Check for a new version of Wflow and download it"),
                ),
                ActiveTab::Settings => (
                    Icon::Gear,
                    tr(lang, "Settings & Audio Hardware"),
                    tr(
                        lang,
                        "Configure microphones, AI models, and keyboard shortcuts",
                    ),
                ),
            };

            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 9.0;
                    let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    crate::icons::draw(
                        ui.painter(),
                        Pos2::new(icon_rect.center().x, icon_rect.min.y + 10.0),
                        18.0,
                        Color32::from_rgb(167, 139, 250),
                        title_icon,
                    );
                    ui.label(
                        egui::RichText::new(title)
                            .font(heading(16.5))
                            .strong()
                            .color(Color32::WHITE),
                    );
                });
                ui.label(
                    egui::RichText::new(subtitle)
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Quick Dictate Action Button
                let (q_icon, q_text, q_fill, q_stroke) = if is_rec {
                    (
                        Icon::Stop,
                        tr(lang, "Stop"),
                        Color32::from_rgb(185, 28, 28),
                        Color32::from_rgb(239, 68, 68),
                    )
                } else {
                    (
                        Icon::Mic,
                        tr(lang, "Dictate"),
                        Color32::from_rgb(79, 70, 229),
                        Color32::from_rgb(129, 140, 248),
                    )
                };
                let q_label = shape_thai(q_text).to_string();
                let q_galley = ui.painter().layout_no_wrap(
                    q_label.clone(),
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
                let q_w = q_galley.size().x + 14.0 + 8.0 + 22.0;
                let (q_rect, q_resp) = ui.allocate_exact_size(Vec2::new(q_w, 30.0), Sense::click());
                let q_rect = q_rect.shrink(0.5);
                ui.painter().rect_filled(
                    q_rect,
                    Rounding::same(8.0),
                    if q_resp.hovered() {
                        q_fill.linear_multiply(1.18)
                    } else {
                        q_fill
                    },
                );
                ui.painter().rect_stroke(
                    q_rect,
                    Rounding::same(8.0),
                    Stroke::new(1.0_f32, q_stroke),
                );
                let q_group = 14.0 + 8.0 + q_galley.size().x;
                let q_start = q_rect.center().x - q_group / 2.0;
                crate::icons::draw(
                    ui.painter(),
                    Pos2::new(q_start + 7.0, q_rect.center().y),
                    14.0,
                    Color32::WHITE,
                    q_icon,
                );
                ui.painter().text(
                    Pos2::new(q_start + 22.0, q_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    q_label,
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
                if q_resp.clicked() {
                    if is_rec {
                        let _ = self.event_tx.send(GuiEvent::StopDictation);
                    } else {
                        let _ = self.event_tx.send(GuiEvent::StartDictation);
                    }
                }

                ui.add_space(8.0);
                let (status_icon, status_text, fg_color, bg_color, border_color) = match status {
                    DictationStatus::Idle => (
                        Icon::Check,
                        tr1(lang, "Ready [{}]", dictate_key_name),
                        Color32::from_rgb(110, 231, 183),
                        Color32::from_rgb(6, 78, 59),
                        Color32::from_rgb(16, 185, 129),
                    ),
                    DictationStatus::Listening => (
                        Icon::Dot,
                        tr(lang, "Recording...").to_string(),
                        Color32::from_rgb(254, 202, 202),
                        Color32::from_rgb(127, 29, 29),
                        Color32::from_rgb(239, 68, 68),
                    ),
                    DictationStatus::Transcribing => (
                        Icon::Sparkle,
                        tr(lang, "AI Processing...").to_string(),
                        Color32::from_rgb(216, 180, 254),
                        Color32::from_rgb(88, 28, 135),
                        Color32::from_rgb(168, 85, 247),
                    ),
                    DictationStatus::Pasting => (
                        Icon::Paste,
                        tr(lang, "Pasting Text...").to_string(),
                        Color32::from_rgb(191, 219, 254),
                        Color32::from_rgb(30, 58, 138),
                        Color32::from_rgb(59, 130, 246),
                    ),
                    DictationStatus::Done => (
                        Icon::Check,
                        tr(lang, "Inserted!").to_string(),
                        Color32::from_rgb(110, 231, 183),
                        Color32::from_rgb(6, 78, 59),
                        Color32::from_rgb(16, 185, 129),
                    ),
                    DictationStatus::Error => (
                        Icon::Alert,
                        tr(lang, "Audio Error").to_string(),
                        Color32::from_rgb(253, 230, 138),
                        Color32::from_rgb(120, 53, 15),
                        Color32::from_rgb(245, 158, 11),
                    ),
                };

                // Paint the pill background first, then the icon and text.
                let status_label = shape_thai(&status_text).to_string();
                let status_galley = ui.painter().layout_no_wrap(
                    status_label.clone(),
                    FontId::proportional(12.0),
                    fg_color,
                );
                let (status_rect, _) = ui.allocate_exact_size(
                    status_galley.size() + egui::vec2(34.0, 12.0),
                    Sense::hover(),
                );
                ui.painter()
                    .rect_filled(status_rect, Rounding::same(12.0), bg_color);
                ui.painter().rect_stroke(
                    status_rect,
                    Rounding::same(12.0),
                    Stroke::new(1.0_f32, border_color),
                );
                crate::icons::draw(
                    ui.painter(),
                    Pos2::new(status_rect.min.x + 16.0, status_rect.center().y),
                    12.0,
                    fg_color,
                    status_icon,
                );
                ui.painter().text(
                    Pos2::new(status_rect.min.x + 26.0, status_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    status_label,
                    FontId::proportional(12.0),
                    fg_color,
                );

                ui.add_space(8.0);

                let active_app = lock(&self.state.active_app).clone();
                let app_display = if active_app.process_name.is_empty() {
                    "Desktop".to_string()
                } else {
                    active_app.process_name.clone()
                };

                let ctx_label = tr1(lang, "Focus: {}", app_display);
                let ctx_galley = ui.painter().layout_no_wrap(
                    ctx_label.clone(),
                    FontId::proportional(11.5),
                    Color32::from_rgb(148, 163, 184),
                );
                let (ctx_rect, _) = ui.allocate_exact_size(
                    ctx_galley.size() + egui::vec2(34.0, 10.0),
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
                crate::icons::draw(
                    ui.painter(),
                    Pos2::new(ctx_rect.min.x + 14.0, ctx_rect.center().y),
                    12.0,
                    Color32::from_rgb(148, 163, 184),
                    Icon::Target,
                );
                ui.painter().text(
                    Pos2::new(ctx_rect.min.x + 24.0, ctx_rect.center().y),
                    egui::Align2::LEFT_CENTER,
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
        let lang = self.ui_lang;

        // One column, ordered the way the feature is actually used: speak,
        // read what came out, then check the numbers. The previous two-column
        // split let the transcript card float over the record button and the
        // waveform whenever the window was short.
        //
        // The scroll bar is hidden: on a short window the page still scrolls
        // with the wheel or by dragging, but a permanent bar down the side of
        // the only screen the user actually looks at made the layout feel
        // cramped and broken.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                // -------------------- 1. Audio capture --------------------
                egui::Frame::none()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0_f32, SURFACE_LINE))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(16.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(
                                ui,
                                Icon::Waveform,
                                13.0,
                                MUTED,
                                tr(lang, "AUDIO CAPTURE & WAVEFORM"),
                                FontId::proportional(11.0),
                            );

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if !is_rec {
                                        return;
                                    }

                                    let elapsed = self
                                        .recording_start
                                        .map(|s| s.elapsed().as_secs_f32())
                                        .unwrap_or(0.0);
                                    let mins = (elapsed / 60.0).floor() as u32;
                                    let time_str =
                                        format!("{:02}:{:04.1}", mins, elapsed % 60.0);
                                    icon_pill(
                                        ui,
                                        Some(Icon::Dot),
                                        &tr1(lang, "LIVE • {}", time_str),
                                        Color32::from_rgb(254, 202, 202),
                                        Color32::from_rgb(127, 29, 29),
                                        8.0,
                                        FontId::monospace(11.0),
                                    );
                                    ui.add_space(6.0);

                                    // Voice-activity readout: is the gate
                                    // hearing speech or just the room?
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
                                    let (label, fg, act_icon) = if speaking {
                                        (
                                            tr(lang, "Speaking"),
                                            Color32::from_rgb(74, 222, 128),
                                            Icon::Activity,
                                        )
                                    } else {
                                        (tr(lang, "Silent"), MUTED, Icon::Flatline)
                                    };
                                    icon_pill(
                                        ui,
                                        Some(act_icon),
                                        &format!(
                                            "{}  {:.0}dB / {:.0}dB",
                                            label,
                                            (noise * 1000.0).log10() * 20.0,
                                            (threshold * 1000.0).log10() * 20.0
                                        ),
                                        fg,
                                        Color32::from_rgb(22, 29, 44),
                                        11.0,
                                        FontId::monospace(10.5),
                                    );
                                },
                            );
                        });

                        ui.add_space(10.0);

                        // Waveform. Deliberately shallow: the tall, spiky
                        // version competed with the controls below it.
                        let visualizer_height = 74.0;
                        let (rect, _response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), visualizer_height),
                            Sense::hover(),
                        );
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

                        let bar_count = 44;
                        let gap = 3.0;
                        let bar_width =
                            (rect.width() - 16.0 - (bar_count as f32 * gap)) / bar_count as f32;
                        let center_y = rect.center().y;

                        ui.painter().line_segment(
                            [
                                Pos2::new(rect.min.x + 8.0, center_y),
                                Pos2::new(rect.max.x - 8.0, center_y),
                            ],
                            Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(255, 255, 255, 15),
                            ),
                        );

                        for i in 0..bar_count {
                            let x = rect.min.x + 8.0 + (i as f32 * (bar_width + gap));
                            let t = i as f32 / bar_count as f32;
                            let phase = (i as f32 * 0.32) + (self.animation_time * 3.0);

                            let height_factor = if is_rec {
                                let wave = (phase.sin() * 0.35 + 0.55)
                                    * (amplitude * 6.0).clamp(0.3, 1.0);
                                wave * (visualizer_height * 0.19)
                            } else {
                                (phase.sin() * 0.15 + 0.25) * 3.0
                            };

                            // One cyan -> purple ramp instead of three
                            // competing hues.
                            let bar_color = if is_rec {
                                Color32::from_rgb(
                                    (6.0 + 118.0 * t) as u8,
                                    (182.0 - 74.0 * t) as u8,
                                    (212.0 + 28.0 * t).min(255.0) as u8,
                                )
                            } else {
                                Color32::from_rgb(45, 56, 80)
                            };

                            ui.painter().rect_filled(
                                Rect::from_min_max(
                                    Pos2::new(x, center_y - height_factor),
                                    Pos2::new(x + bar_width, center_y + height_factor + 1.0),
                                ),
                                Rounding::same(2.0),
                                bar_color,
                            );
                        }

                        ui.add_space(12.0);

                        // The single primary action, placed below the
                        // waveform instead of painted on top of it.
                        let is_tap = self.settings_recording_trigger_mode == "tap_to_toggle";
                        let (btn_icon, btn_text, fill, border) = if is_rec {
                            (
                                Icon::Stop,
                                tr(lang, "CLICK TO COMMIT & INSERT").to_string(),
                                Color32::from_rgb(190, 30, 36),
                                Color32::from_rgb(239, 68, 68),
                            )
                        } else if status == DictationStatus::Transcribing {
                            (
                                Icon::Sparkle,
                                tr(lang, "AI TRANSCRIBING...").to_string(),
                                Color32::from_rgb(76, 42, 158),
                                Color32::from_rgb(168, 85, 247),
                            )
                        } else {
                            (
                                Icon::Mic,
                                if is_tap {
                                    tr1(
                                        lang,
                                        "START DICTATING (OR TAP {})",
                                        dictate_key_name,
                                    )
                                } else {
                                    tr1(
                                        lang,
                                        "START DICTATING (OR HOLD {})",
                                        dictate_key_name,
                                    )
                                },
                                Color32::from_rgb(79, 70, 229),
                                Color32::from_rgb(129, 140, 248),
                            )
                        };

                        let hero_clicked = icon_button(
                            ui,
                            btn_icon,
                            &btn_text,
                            44.0,
                            fill,
                            border,
                            Color32::WHITE,
                            heading(13.5),
                            17.0,
                            true,
                        );
                        if hero_clicked {
                            if is_rec {
                                let _ = self.event_tx.send(GuiEvent::StopDictation);
                            } else if status == DictationStatus::Idle
                                || status == DictationStatus::Done
                            {
                                let _ = self.event_tx.send(GuiEvent::StartDictation);
                            }
                        }

                        ui.add_space(8.0);
                        let tip = if is_tap {
                            tr1(
                                lang,
                                "Tap {} once to start dictating hands-free, tap again to commit and insert text.",
                                dictate_key_name,
                            )
                        } else {
                            tr1(
                                lang,
                                "Hold {} while speaking to dictate directly into any text box.",
                                dictate_key_name,
                            )
                        };
                        icon_label(
                            ui,
                            Icon::Keyboard,
                            13.0,
                            Color32::from_rgb(100, 116, 139),
                            &tip,
                            FontId::proportional(11.0),
                        );
                    });

                ui.add_space(12.0);

                // -------------------- 2. Latest transcription --------------------
                let transcript = lock(&self.state.last_transcript).clone();
                let latency = *lock(&self.state.last_latency);

                egui::Frame::none()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0_f32, SURFACE_LINE))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(16.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(
                                ui,
                                Icon::FileText,
                                13.0,
                                MUTED,
                                tr(lang, "LATEST TRANSCRIPTION"),
                                FontId::proportional(11.0),
                            );
                            if latency > 0.0 {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        icon_pill(
                                            ui,
                                            Some(Icon::Bolt),
                                            &format!("{:.0} ms", latency * 1000.0),
                                            Color32::from_rgb(74, 222, 128),
                                            Color32::from_rgba_unmultiplied(34, 197, 94, 30),
                                            11.0,
                                            FontId::monospace(11.0),
                                        );
                                    },
                                );
                            }
                        });

                        ui.add_space(8.0);

                        // Shrinks to fit when there is nothing to show, so an
                        // empty transcript card does not leave a tall hole.
                        egui::ScrollArea::vertical()
                            .max_height(120.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                if transcript.is_empty() {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(6.0);
                                        ui.label(
                                            egui::RichText::new(tr(
                                                lang,
                                                "No recent transcript.",
                                            ))
                                            .font(FontId::proportional(13.0))
                                            .color(Color32::from_rgb(100, 116, 139)),
                                        );
                                        ui.label(
                                            egui::RichText::new(tr1(
                                                lang,
                                                "Press {} and speak clearly into your mic.",
                                                dictate_key_name,
                                            ))
                                            .font(FontId::proportional(11.0))
                                            .color(Color32::from_rgb(71, 85, 105)),
                                        );
                                        ui.add_space(6.0);
                                    });
                                } else {
                                    ui.label(
                                        egui::RichText::new(shape_thai(&transcript))
                                            .font(FontId::proportional(14.5))
                                            .color(Color32::from_rgb(248, 250, 252)),
                                    );
                                }
                            });

                        if !transcript.is_empty() {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                let word_count = transcript.split_whitespace().count();
                                let char_count = transcript.chars().count();
                                ui.label(
                                    egui::RichText::new(tr2(
                                        lang,
                                        "{} words • {} chars",
                                        word_count,
                                        char_count,
                                    ))
                                    .font(FontId::proportional(11.0))
                                    .color(MUTED),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let surface_btn = Color32::from_rgb(33, 43, 64);
                                        let surface_line = Color32::from_rgb(51, 65, 85);

                                        if icon_button_small(
                                            ui,
                                            Icon::Trash,
                                            26.0,
                                            surface_btn,
                                            surface_line,
                                            Color32::from_rgb(252, 165, 165),
                                            tr(lang, "Clear"),
                                        ) {
                                            *lock(&self.state.last_transcript) = String::new();
                                            self.state.set_toast(tr(lang, "Transcript cleared"));
                                        }
                                        ui.add_space(6.0);

                                        if icon_button_small(
                                            ui,
                                            Icon::Copy,
                                            26.0,
                                            surface_btn,
                                            surface_line,
                                            Color32::from_rgb(203, 213, 225),
                                            tr(lang, "Copy"),
                                        ) {
                                            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                                let _ = clipboard.set_text(&transcript);
                                                self.state.set_toast(tr(
                                                    lang,
                                                    "Copied transcript to clipboard!",
                                                ));
                                            }
                                        }
                                        ui.add_space(6.0);

                                        if icon_button_small(
                                            ui,
                                            Icon::Paste,
                                            26.0,
                                            Color32::from_rgb(49, 46, 129),
                                            Color32::from_rgb(99, 102, 241),
                                            Color32::WHITE,
                                            tr(lang, "Re-paste"),
                                        ) {
                                            let _ = PlatformManager::paste_text(&transcript);
                                            self.state.set_toast(tr(
                                                lang,
                                                "Re-pasted transcript to active cursor!",
                                            ));
                                        }
                                    },
                                );
                            });
                        }
                    });

                ui.add_space(12.0);

                // -------------------- 3. Quick stats --------------------
                let history_count = lock(&self.state.history).len();
                let latency_text = if latency > 0.0 {
                    format!("{:.0} ms", latency * 1000.0)
                } else {
                    "—".to_string()
                };

                egui::Frame::none()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0_f32, SURFACE_LINE))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.columns(3, |cols| {
                            let stats = [
                                (
                                    Icon::History,
                                    tr(lang, "RECORDED").to_string(),
                                    tr1(lang, "{} Entries", history_count),
                                    Color32::from_rgb(167, 139, 250),
                                ),
                                (
                                    Icon::Clock,
                                    tr(lang, "LATENCY").to_string(),
                                    latency_text.clone(),
                                    Color32::from_rgb(52, 211, 153),
                                ),
                                (
                                    Icon::Server,
                                    tr(lang, "STORAGE").to_string(),
                                    tr(lang, "SQLite Local").to_string(),
                                    Color32::from_rgb(56, 189, 248),
                                ),
                            ];
                            for (col, (icon, label, value, color)) in
                                cols.iter_mut().zip(stats.iter())
                            {
                                icon_label(col, *icon, 12.0, MUTED, label, FontId::proportional(10.0));
                                col.add_space(4.0);
                                col.label(
                                    egui::RichText::new(value)
                                        .font(heading(14.0))
                                        .strong()
                                        .color(*color),
                                );
                            }
                        });
                    });

                ui.add_space(12.0);

                // -------------------- 4. Context & engine --------------------
                let active_app = lock(&self.state.active_app).clone();
                let (prov_title, model_name, prov_color) = {
                    let cfg = lock(&self.state.config);
                    match cfg.stt_provider {
                        SttProvider::WisprFlow => (
                            "Wispr Flow Cloud".to_string(),
                            "Native Stream (gRPC)".to_string(),
                            Color32::from_rgb(129, 140, 248),
                        ),
                        SttProvider::Groq => (
                            "Groq Cloud LPU".to_string(),
                            cfg.groq_model.clone(),
                            Color32::from_rgb(251, 146, 60),
                        ),
                        SttProvider::ElevenLabs => (
                            "ElevenLabs".to_string(),
                            cfg.elevenlabs_model.clone(),
                            Color32::from_rgb(192, 132, 252),
                        ),
                    }
                };

                ui.columns(2, |cols| {
                    // Where the text will land.
                    egui::Frame::none()
                        .fill(SURFACE)
                        .stroke(Stroke::new(1.0_f32, SURFACE_LINE))
                        .rounding(Rounding::same(14.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(&mut cols[0], |ui| {
                            ui.horizontal(|ui| {
                                let (icon_rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                                ui.painter().rect_filled(
                                    icon_rect,
                                    Rounding::same(9.0),
                                    Color32::from_rgba_unmultiplied(14, 165, 233, 30),
                                );
                                crate::icons::draw(
                                    ui.painter(),
                                    icon_rect.center(),
                                    16.0,
                                    Color32::from_rgb(56, 189, 248),
                                    Icon::Target,
                                );

                                ui.add_space(4.0);
                                ui.vertical(|ui| {
                                    let proc_text = if active_app.process_name.is_empty() {
                                        tr(lang, "Desktop Cursor").to_string()
                                    } else {
                                        active_app.process_name.clone()
                                    };
                                    ui.label(
                                        egui::RichText::new(proc_text)
                                            .font(FontId::proportional(12.5))
                                            .strong()
                                            .color(Color32::WHITE),
                                    );
                                    let sub = if active_app.window_title.is_empty() {
                                        tr(lang, "TARGET APPLICATION CONTEXT").to_string()
                                    } else {
                                        active_app.window_title.clone()
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(sub)
                                                .font(FontId::proportional(11.0))
                                                .color(MUTED),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                        });

                    // Which model is doing the work.
                    egui::Frame::none()
                        .fill(SURFACE)
                        .stroke(Stroke::new(1.0_f32, SURFACE_LINE))
                        .rounding(Rounding::same(14.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(&mut cols[1], |ui| {
                            ui.horizontal(|ui| {
                                let (icon_rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                                ui.painter().rect_filled(
                                    icon_rect,
                                    Rounding::same(9.0),
                                    Color32::from_rgba_unmultiplied(124, 108, 240, 34),
                                );
                                crate::icons::draw(
                                    ui.painter(),
                                    icon_rect.center(),
                                    16.0,
                                    prov_color,
                                    Icon::Bolt,
                                );

                                ui.add_space(4.0);
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(&prov_title)
                                            .font(FontId::proportional(12.5))
                                            .strong()
                                            .color(Color32::WHITE),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(tr1(
                                                lang,
                                                "Model: {}",
                                                model_name.clone(),
                                            ))
                                            .font(FontId::proportional(11.0))
                                            .color(MUTED),
                                        )
                                        .truncate(),
                                    );
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let (rect, response) = ui.allocate_exact_size(
                                            Vec2::new(28.0, 28.0),
                                            Sense::click(),
                                        );
                                        let rect = rect.shrink(0.5);
                                        ui.painter().rect_filled(
                                            rect,
                                            Rounding::same(8.0),
                                            if response.hovered() {
                                                Color32::from_rgb(40, 52, 78)
                                            } else {
                                                Color32::from_rgb(26, 33, 50)
                                            },
                                        );
                                        ui.painter().rect_stroke(
                                            rect,
                                            Rounding::same(8.0),
                                            Stroke::new(1.0_f32, SURFACE_LINE),
                                        );
                                        crate::icons::draw(
                                            ui.painter(),
                                            rect.center(),
                                            15.0,
                                            MUTED,
                                            Icon::Swap,
                                        );
                                        if response.clicked() {
                                            self.active_tab = ActiveTab::Settings;
                                        }
                                        let _ = response.on_hover_text(tr(lang, "Switch Model"));
                                    },
                                );
                            });
                        });
                });

                ui.add_space(12.0);

                // -------------------- 5. Recent activity --------------------
                let recent_items: Vec<_> = lock(&self.state.history)
                    .iter()
                    .take(3)
                    .cloned()
                    .collect();

                egui::Frame::none()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0_f32, SURFACE_LINE))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(
                                ui,
                                Icon::Clock,
                                13.0,
                                MUTED,
                                tr(lang, "RECENT ACTIVITY"),
                                FontId::proportional(11.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(tr(lang, "View All →"))
                                        .clicked()
                                    {
                                        self.active_tab = ActiveTab::History;
                                        let _ = self.event_tx.send(GuiEvent::RefreshHistory);
                                    }
                                },
                            );
                        });

                        ui.add_space(6.0);

                        if recent_items.is_empty() {
                            icon_label(
                                ui,
                                Icon::Flatline,
                                12.0,
                                Color32::from_rgb(100, 116, 139),
                                tr(
                                    lang,
                                    "No recent transcriptions yet. Dictate anywhere to see activity.",
                                ),
                                FontId::proportional(11.0),
                            );
                        } else {
                            for item in recent_items {
                                ui.horizontal(|ui| {
                                    crate::icons::draw(
                                        ui.painter(),
                                        Pos2::new(
                                            ui.cursor().min.x + 6.0,
                                            ui.cursor().min.y + 9.0,
                                        ),
                                        12.0,
                                        Color32::from_rgb(124, 108, 240),
                                        Icon::Bolt,
                                    );
                                    ui.add_space(12.0);

                                    let app_label = if item.app.is_empty() {
                                        tr(lang, "Desktop").to_string()
                                    } else {
                                        item.app.as_str().to_string()
                                    };
                                    ui.label(
                                        egui::RichText::new(app_label)
                                            .font(FontId::proportional(10.5))
                                            .strong()
                                            .color(Color32::from_rgb(56, 189, 248)),
                                    );

                                    let snippet = if item.formatted_text.chars().count() > 32 {
                                        format!(
                                            "{}...",
                                            item.formatted_text.chars().take(30).collect::<String>()
                                        )
                                    } else {
                                        item.formatted_text.clone()
                                    };

                                    // Reserve room for the copy button so a
                                    // long snippet cannot be painted under it.
                                    let text_width = (ui.available_width() - 34.0).max(40.0);
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(text_width, 18.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(shape_thai(&snippet))
                                                        .font(FontId::proportional(11.0))
                                                        .color(Color32::from_rgb(203, 213, 225)),
                                                )
                                                .truncate(),
                                            );
                                        },
                                    );

                                    if icon_button_small(
                                        ui,
                                        Icon::Copy,
                                        22.0,
                                        Color32::TRANSPARENT,
                                        SURFACE_LINE,
                                        MUTED,
                                        tr(lang, "Copy"),
                                    ) {
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            let _ = clipboard.set_text(&item.formatted_text);
                                            self.state
                                                .set_toast(tr(lang, "Copied to clipboard!"));
                                        }
                                    }
                                });
                                ui.add_space(2.0);
                            }
                        }
                    });
            });
    }

    fn render_history_tab(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
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
                                egui::RichText::new(tr(lang, "Total Transcriptions"))
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.label(
                                egui::RichText::new(tr1(lang, "{} recorded", history.len()))
                                    .font(heading(14.0))
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                        });
                    });
                    stat_cols[1].horizontal(|ui| {
                        ui.label("📝");
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(tr(lang, "Dictated Words"))
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.label(
                                egui::RichText::new(tr1(lang, "{} words", total_words))
                                    .font(heading(14.0))
                                    .strong()
                                    .color(Color32::from_rgb(167, 139, 250)),
                            );
                        });
                    });
                    stat_cols[2].horizontal(|ui| {
                        ui.label("⏱️");
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(tr(lang, "Total Voice Time"))
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.label(
                                egui::RichText::new(tr1(
                                    lang,
                                    "{} seconds",
                                    format!("{:.1}", total_duration),
                                ))
                                .font(heading(14.0))
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
                    .hint_text(tr(
                        lang,
                        "Search transcripts by keywords or application name...",
                    ))
                    .desired_width(ui.available_width() - 120.0),
            );

            let refresh_btn = egui::Button::new(format!("🔄 {}", tr(lang, "Refresh")))
                .fill(Color32::from_rgb(33, 44, 66))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(51, 65, 85)))
                .rounding(Rounding::same(6.0));
            if ui.add(refresh_btn).clicked() {
                let _ = self.event_tx.send(GuiEvent::RefreshHistory);
                self.state.set_toast(tr(lang, "History records refreshed!"));
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
                    egui::RichText::new(tr(lang, "No matching dictation records found."))
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
                                    tr(lang, "Desktop").to_string()
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
                                        egui::RichText::new(friendly_timestamp(&rec.timestamp))
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
                                            self.state.set_toast(tr(lang, "Record deleted"));
                                        }

                                        ui.add_space(4.0);

                                        let paste_btn =
                                            egui::Button::new(format!("✍️ {}", tr(lang, "Paste")))
                                                .fill(Color32::from_rgb(49, 46, 129))
                                                .rounding(Rounding::same(4.0));
                                        if ui.add(paste_btn).clicked() {
                                            let _ =
                                                PlatformManager::paste_text(&rec.formatted_text);
                                            self.state
                                                .set_toast(tr(lang, "Pasted into active window!"));
                                        }

                                        ui.add_space(4.0);

                                        let copy_btn =
                                            egui::Button::new(format!("📋 {}", tr(lang, "Copy")))
                                                .fill(Color32::from_rgb(33, 44, 66))
                                                .rounding(Rounding::same(4.0));
                                        if ui.add(copy_btn).clicked() {
                                            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                                let _ = clipboard.set_text(&rec.formatted_text);
                                                self.state
                                                    .set_toast(tr(lang, "Copied to clipboard!"));
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
        let lang = self.ui_lang;
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
                        egui::RichText::new(format!(
                            "📖  {}",
                            tr(lang, "CUSTOM VOCABULARY & TEXT SNIPPET EXPANSIONS")
                        ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(tr(
                            lang,
                            "Teach Wflow custom terminology, names, or define instant trigger snippets. When you speak a trigger phrase, it will automatically be expanded into your configured replacement text.",
                        ))
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                    ui.add_space(12.0);

                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.label(tr(lang, "Word or Trigger Phrase:"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dict_word_input)
                                    .hint_text(tr(lang, "e.g. Wflow, myemail, BRB, หมอพร้อม"))
                                    .desired_width(240.0),
                            );
                        });

                        ui.add_space(8.0);

                        ui.vertical(|ui| {
                            ui.label(tr(lang, "Snippet Replacement (Optional):"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dict_replacement_input)
                                    .hint_text(tr(
                                        lang,
                                        "e.g. contact@wflow.ai (leave blank for vocabulary only)",
                                    ))
                                    .desired_width(300.0),
                            );
                        });

                        ui.add_space(8.0);

                        ui.vertical(|ui| {
                            ui.label(" ");
                            let add_btn = egui::Button::new(
                                egui::RichText::new(format!("➕ {}", tr(lang, "Add Entry")))
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
                                    self.state.set_toast(&tr1(lang, "Added '{}' to dictionary", &word));
                                }
                            }
                        });
                    });
                });

            ui.add_space(10.0);

            // Filter & Entries List
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(tr1(
                        lang,
                        "All Entries ({})",
                        entries.len(),
                    ))
                        .font(heading(13.0))
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.dict_search_query)
                            .hint_text(tr(lang, "🔍 Filter dictionary..."))
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
                                egui::RichText::new(tr(lang, "No dictionary entries found."))
                                    .font(FontId::proportional(13.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                            ui.label(
                                egui::RichText::new(tr(
                                    lang,
                                    "Add custom words or text expansion snippets above!",
                                ))
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
                                        egui::RichText::new(tr(lang, "SNIPPET"))
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
                                        egui::RichText::new(tr(lang, "VOCAB"))
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
                                            egui::RichText::new("→")
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
                                    let del_btn = egui::Button::new(format!("🗑️ {}", tr(lang, "Delete")))
                                        .fill(Color32::from_rgb(45, 25, 30))
                                        .rounding(Rounding::same(4.0));
                                    if ui.add(del_btn).clicked() {
                                        let _ = self.event_tx.send(GuiEvent::DeleteDictionaryEntry(entry.id.clone()));
                                        self.state.set_toast(tr(lang, "Entry removed"));
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
        let lang = self.ui_lang;
        let current = crate::update::current_version();
        let pending = self.pending_update();
        let running = self.state.update_check_running.load(Ordering::SeqCst);
        let error = lock(&self.state.update_error).clone();
        let last_check = lock(&self.state.config).last_update_check.clone();
        let install_running = lock(&self.state.update_install).is_running();

        egui::Frame::none()
            .fill(Color32::from_rgb(17, 23, 36))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::same(18.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("🔄  {}", tr(lang, "Wflow Software Update")))
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
                        format!("⏳ {}", tr(lang, "Checking for updates...")),
                        Color32::from_rgb(203, 213, 225),
                        Color32::from_rgb(30, 41, 59),
                        Color32::from_rgb(71, 85, 105),
                    )
                } else if let Some(info) = pending.as_ref() {
                    (
                        format!(
                            "🎉 {}",
                            tr1(lang, "New version v{} available", &info.latest)
                        ),
                        Color32::WHITE,
                        Color32::from_rgb(22, 163, 74),
                        Color32::from_rgb(74, 222, 128),
                    )
                } else if let Some(err) = error.as_ref() {
                    (
                        format!("⚠️ {}", tr1(lang, "Update check failed: {}", err)),
                        Color32::from_rgb(252, 165, 165),
                        Color32::from_rgb(45, 18, 22),
                        Color32::from_rgb(239, 68, 68),
                    )
                } else {
                    (
                        format!("✅ {}", tr(lang, "You are on the latest version")),
                        Color32::from_rgb(134, 239, 172),
                        Color32::from_rgb(12, 38, 28),
                        Color32::from_rgb(34, 197, 94),
                    )
                };

                // Paint the background first, then the text on top of it;
                // drawing the filled rect after the label hid the message.
                let galley = ui.painter().layout_no_wrap(
                    shape_thai(&status_text).to_string(),
                    FontId::proportional(13.0),
                    status_color,
                );
                let pill_size = galley.size() + egui::vec2(22.0, 12.0);
                let (pill_rect, _) = ui.allocate_exact_size(pill_size, egui::Sense::hover());
                ui.painter()
                    .rect_filled(pill_rect, Rounding::same(10.0), status_fill);
                ui.painter().rect_stroke(
                    pill_rect,
                    Rounding::same(10.0),
                    Stroke::new(1.0_f32, status_stroke),
                );
                ui.painter().galley(
                    pill_rect.center() - galley.size() / 2.0,
                    galley,
                    status_color,
                );

                ui.add_space(14.0);

                // --- actions -----------------------------------------------------
                ui.horizontal(|ui| {
                    let check_label = if running {
                        format!("⏳ {}", tr(lang, "Checking for updates..."))
                    } else {
                        format!("🔍 {}", tr(lang, "Check now"))
                    };
                    let check_btn = egui::Button::new(
                        egui::RichText::new(shape_thai(&check_label))
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
                        self.state.set_toast(tr(lang, "Checking for updates..."));
                    }

                    if let Some(info) = pending.as_ref() {
                        // The primary action installs the update inside the
                        // app: download → checksum → replace → restart.
                        let install_label = if install_running {
                            format!("⏳ {}", tr1(lang, "Installing v{}...", &info.latest))
                        } else {
                            format!("⬇️ {}", tr1(lang, "Install v{}", &info.latest))
                        };
                        let install_btn = egui::Button::new(
                            egui::RichText::new(shape_thai(&install_label))
                                .font(FontId::proportional(12.5))
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(Color32::from_rgb(22, 163, 74))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(74, 222, 128)))
                        .rounding(Rounding::same(8.0))
                        .min_size(Vec2::new(190.0, 36.0));

                        if ui.add_enabled(!install_running, install_btn).clicked() {
                            let _ = self.event_tx.send(GuiEvent::InstallUpdate);
                        }
                    }

                    if ui
                        .button(format!("🌐 {}", tr(lang, "Open Releases page")))
                        .on_hover_text(tr(lang, "Open the GitHub Releases page in your browser"))
                        .clicked()
                    {
                        let _ = self.event_tx.send(GuiEvent::OpenUrl(format!(
                            "https://github.com/{}/releases",
                            crate::update::DEFAULT_REPO
                        )));
                    }
                });

                // Manual download, for the rare case where the in-app
                // installation is not possible (no archive for this platform).
                if let Some(info) = pending.as_ref() {
                    ui.horizontal(|ui| {
                        if ui
                            .button(format!("📥 {}", tr(lang, "Download manually")))
                            .on_hover_text(tr(
                                lang,
                                "Download the installer and install it yourself",
                            ))
                            .clicked()
                        {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::OpenUrl(info.download_url.clone()));
                        }
                    });
                }

                // --- progress of the running installation --------------------
                let install_state = lock(&self.state.update_install).clone();
                if install_state.is_running() {
                    ui.add_space(10.0);
                    let label = match install_state.stage {
                        crate::updater::InstallStage::Downloading => {
                            tr(lang, "Downloading update...")
                        }
                        crate::updater::InstallStage::Verifying => {
                            tr(lang, "Verifying checksum...")
                        }
                        crate::updater::InstallStage::Installing => {
                            tr(lang, "Installing update...")
                        }
                        crate::updater::InstallStage::Restarting => tr(lang, "Restarting Wflow..."),
                        _ => tr(lang, "Working..."),
                    };
                    ui.label(
                        egui::RichText::new(shape_thai(label))
                            .font(FontId::proportional(11.5))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                    ui.add(
                        egui::ProgressBar::new(install_state.percent as f32 / 100.0)
                            .desired_width(ui.available_width())
                            .fill(Color32::from_rgb(34, 197, 94)),
                    );
                }

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
                        tr(lang, "(no further details)").to_string()
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
                        if ui
                            .button(format!("📝 {}", tr(lang, "View Release Notes on GitHub")))
                            .clicked()
                        {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::OpenUrl(info.release_url.clone()));
                        }
                        if ui
                            .button(tr(lang, "Skip this version"))
                            .on_hover_text(tr(lang, "Do not notify about this version again"))
                            .clicked()
                        {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::SkipUpdateVersion(info.latest.to_string()));
                            self.state.set_toast(tr(lang, "Version skipped"));
                        }
                    });
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // --- details ------------------------------------------------------
                ui.label(
                    egui::RichText::new(tr(lang, "Details"))
                        .font(FontId::proportional(12.0))
                        .color(Color32::from_rgb(148, 163, 184))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "• {}",
                        tr1(lang, "Current version: v{}", current)
                    ))
                    .font(FontId::proportional(11.5))
                    .color(Color32::from_rgb(203, 213, 225)),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "• {}",
                        tr1(
                            lang,
                            "Last checked: {}",
                            last_check
                                .map(|v| friendly_timestamp(&v))
                                .unwrap_or_else(|| tr(lang, "never").to_string())
                        )
                    ))
                    .font(FontId::proportional(11.5))
                    .color(Color32::from_rgb(203, 213, 225)),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "• {}",
                        tr(
                            lang,
                            "Checked automatically at startup and at the interval set in Settings",
                        )
                    ))
                    .font(FontId::proportional(11.5))
                    .color(Color32::from_rgb(203, 213, 225)),
                );
            });
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        // A compact section switcher keeps the settings readable: instead of
        // one very long page, every topic gets its own short page.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (section, icon, label) in [
                (SettingsSection::Language, "🌐", "Language"),
                (SettingsSection::Shortcuts, "⌨️", "Shortcuts"),
                (SettingsSection::Audio, "🎙️", "Audio"),
                (SettingsSection::Ai, "🤖", "AI & Models"),
                (SettingsSection::Voice, "🎚️", "Voice & Text"),
                (SettingsSection::Privacy, "🔒", "Privacy"),
                (SettingsSection::Updates, "🔄", "Updates"),
            ] {
                let selected = self.settings_section == section;
                let text = egui::RichText::new(format!("{} {}", icon, tr(lang, label)))
                    .font(FontId::proportional(12.0))
                    .color(if selected {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(148, 163, 184)
                    })
                    .strong();

                let btn = egui::Button::new(text)
                    .fill(if selected {
                        Color32::from_rgb(79, 70, 229)
                    } else {
                        Color32::from_rgb(22, 29, 44)
                    })
                    .stroke(Stroke::new(
                        1.0_f32,
                        if selected {
                            Color32::from_rgb(129, 140, 248)
                        } else {
                            Color32::from_rgb(51, 65, 85)
                        },
                    ))
                    .rounding(Rounding::same(8.0));

                if ui.add(btn).clicked() {
                    self.settings_section = section;
                }
            }
        });

        ui.add_space(10.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            match self.settings_section {
                SettingsSection::Language => self.render_settings_language(ui),
                SettingsSection::Shortcuts => self.render_settings_shortcuts(ui),
                SettingsSection::Audio => self.render_settings_audio(ui),
                SettingsSection::Ai => self.render_settings_ai(ui),
                SettingsSection::Voice => self.render_settings_voice(ui),
                SettingsSection::Privacy => self.render_settings_privacy(ui),
                SettingsSection::Updates => self.render_settings_updates(ui),
            }

            ui.add_space(16.0);
            self.render_settings_save_button(ui);
        });
    }

    /// Settings ▸ Language: switches the interface between Thai and English.
    ///
    /// The change applies instantly and is persisted right away, so the user
    /// never has to find the save button in a language they cannot read.
    fn render_settings_language(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        ui.add_space(4.0);

        egui::Frame::none()
            .fill(Color32::from_rgb(17, 23, 36))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(format!("🌐  {}", tr(lang, "INTERFACE LANGUAGE")))
                        .font(heading(12.5))
                        .color(Color32::from_rgb(148, 163, 184))
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(tr(
                        lang,
                        "Choose the language of the Wflow interface. The language used for speech transcription is configured separately under Audio.",
                    ))
                    .font(FontId::proportional(11.5))
                    .color(Color32::from_rgb(148, 163, 184)),
                );

                ui.add_space(14.0);

                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
                    for choice in [Lang::Th, Lang::En] {
                        let selected = self.settings_ui_language == choice.code();
                        let fill = if selected {
                            Color32::from_rgb(55, 48, 163)
                        } else {
                            Color32::from_rgb(22, 29, 44)
                        };
                        let stroke = if selected {
                            Color32::from_rgb(129, 140, 248)
                        } else {
                            Color32::from_rgb(51, 65, 85)
                        };

                        let response = ui.add(
                            egui::Button::new(
                                egui::RichText::new(format!(
                                    "{}  {}",
                                    if selected { "●" } else { "○" },
                                    choice.native_name()
                                ))
                                .font(heading(14.0))
                                .color(Color32::WHITE),
                            )
                            .fill(fill)
                            .stroke(Stroke::new(1.4_f32, stroke))
                            .rounding(Rounding::same(10.0))
                            .min_size(Vec2::new(170.0, 46.0)),
                        );

                        if response.clicked() && !selected {
                            self.settings_ui_language = choice.code().to_string();
                            self.ui_lang = choice;
                            // Persist immediately: the rest of the settings
                            // page is now in a different language.
                            let _ = self
                                .event_tx
                                .send(GuiEvent::SetUiLanguage(choice.code().to_string()));
                        }
                    }
                });

                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(tr(
                        lang,
                        "The interface language only affects labels and messages. Your dictation hotkeys, history and vocabulary are unchanged.",
                    ))
                    .font(FontId::proportional(11.0))
                    .color(Color32::from_rgb(100, 116, 139)),
                );
            });

        ui.add_space(12.0);
    }

    fn render_settings_shortcuts(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        ui.add_space(4.0);
        // CARD 1: Hotkeys & Trigger Preferences
        egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(format!(
                            "⌨️  {}",
                            tr(lang, "GLOBAL KEYBOARD SHORTCUTS")
                        ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // 1. Push-To-Talk Dictation Hotkey
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "🎙️ {}",
                                    tr(lang, "Push-To-Talk Dictation Hotkey:")
                                ))
                                    .font(heading(13.0))
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
                            egui::RichText::new(tr(
                                    lang,
                                    "Hold this key or combination to record voice, release to insert text.",
                                ))
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let curr_label = tr1(
                                lang,
                                "Preset: {}",
                                self.settings_hotkey.format_display(),
                            );
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
                                format!(
                                    "🔴 {}",
                                    tr(lang, "Listening for shortcut combination... (Esc to cancel)")
                                )
                            } else {
                                format!(
                                    "⌨️ {}",
                                    tr(lang, "Press any key or combination to set")
                                )
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
                                    self.state
                                        .set_toast(tr(lang, "Key recording cancelled"));
                                } else {
                                    self.recording_mode = Some(1);
                                    PlatformManager::start_recording_hotkey(1);
                                    self.state.set_toast(tr(
                                        lang,
                                        "Press any combination (e.g. Ctrl+Space, Alt+Space, Ctrl+Shift+D) to set",
                                    ));
                                }
                            }
                        });
                    });

                    ui.add_space(8.0);

                    // 2. Toggle Window Hotkey
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "🪟 {}",
                                    tr(lang, "Toggle Window Hotkey:")
                                ))
                                    .font(heading(13.0))
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
                            egui::RichText::new(tr(
                                    lang,
                                    "Press this key or combination anywhere to toggle Wflow Studio between screen and system tray.",
                                ))
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            let curr_label = tr1(
                                lang,
                                "Preset: {}",
                                self.settings_toggle.format_display(),
                            );
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
                                format!(
                                    "🔴 {}",
                                    tr(lang, "Listening for shortcut combination... (Esc to cancel)")
                                )
                            } else {
                                format!(
                                    "⌨️ {}",
                                    tr(lang, "Press any key or combination to set")
                                )
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
                                    self.state.set_toast(tr(
                                        lang,
                                        "Press any combination (e.g. Alt+Space, Ctrl+Space, F9) to set",
                                    ));
                                }
                            }
                        });
                    });

                    ui.add_space(8.0);

                    // Dictation Trigger Mode (Push-to-Talk vs Tap-to-Toggle)
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "🎙️ {}",
                                    tr(lang, "Dictation Trigger Mode:")
                                ))
                                    .font(heading(13.0))
                                    .strong(),
                            );
                            let mode_label = if self.settings_recording_trigger_mode == "tap_to_toggle" {
                                tr(lang, "Tap to Toggle (Hands-Free)")
                            } else {
                                tr(lang, "Push-to-Talk (Hold Key)")
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
                                format!("🖐️ {}", tr(lang, "Hold key to talk (Push-to-Talk)")),
                            );
                            ui.selectable_value(
                                &mut self.settings_recording_trigger_mode,
                                "tap_to_toggle".to_string(),
                                format!(
                                    "🔄 {}",
                                    tr(lang, "Tap once to start / tap to stop (Hands-Free)")
                                ),
                            );
                        });
                        ui.label(
                            egui::RichText::new(if self.settings_recording_trigger_mode == "tap_to_toggle" {
                                tr(
                                    lang,
                                    "Tap your hotkey once to start dictating freely. Tap again to stop and automatically paste into your active window.",
                                )
                            } else {
                                tr(
                                    lang,
                                    "Hold down your hotkey while speaking. Releasing the key immediately commits and types the text.",
                                )
                            })
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });

                    ui.add_space(10.0);

                    // Auto Paste Checkbox
                    ui.checkbox(
                        &mut self.settings_auto_paste,
                        tr(
                            lang,
                            "Auto-paste transcribed text directly into active cursor position",
                        ),
                    );

                    ui.add_space(8.0);

                    // Auto Start Checkbox
                    let cb = ui.checkbox(
                        &mut self.settings_auto_start,
                        tr(
                            lang,
                            "Start Wflow automatically on system startup (Run on Startup)",
                        ),
                    );
                    if cb.changed() {
                        let _ = PlatformManager::set_autostart(self.settings_auto_start);
                        if let Ok(mut lock) = self.state.config.lock() {
                            lock.auto_start = self.settings_auto_start;
                            let _ = lock.save(None);
                        }
                        let msg = if self.settings_auto_start {
                            tr(lang, "Auto-start on system startup enabled")
                        } else {
                            tr(lang, "Auto-start on system startup disabled")
                        };
                        self.state.set_toast(msg);
                    }
                    ui.label(
                        egui::RichText::new(format!(
                        "  └ {}",
                        tr(lang, "Launches Wflow automatically when your system boots.")
                    ))
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(8.0);

                    // Close to Tray Checkbox
                    ui.checkbox(
                        &mut self.settings_close_to_tray,
                        shape_thai(tr(lang, "Minimize to tray on close")),
                    );
                    ui.label(
                        egui::RichText::new(shape_thai(&format!(
                        "  └ {}",
                        tr(lang, "If unchecked, a confirmation dialog appears when you press [X].")
                    )))
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                });
    }

    fn render_settings_audio(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        ui.add_space(4.0);
        // CARD 1.5: Microphone & Sound Effects Hardware
        egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(format!(
                            "🎤  {}",
                            tr(lang, "MICROPHONE HARDWARE & AUDIO SOUND EFFECTS")
                        ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // Microphone Device Selector
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "Input Microphone:"));
                        let current_dev_label = self
                            .settings_input_device_name
                            .as_deref()
                            .unwrap_or(tr(lang, "Default System Microphone"));

                        egui::ComboBox::from_id_salt("mic_device_combo")
                            .selected_text(current_dev_label)
                            .width(340.0)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        self.settings_input_device_name.is_none(),
                                        tr(lang, "Default System Microphone"),
                                    )
                                    .clicked()
                                {
                                    self.settings_input_device_name = None;
                                }
                                for dev in &self.available_devices {
                                    let is_sel = self.settings_input_device_name.as_deref() == Some(dev.as_str());
                                    if ui.selectable_label(is_sel, dev).clicked() {
                                        self.settings_input_device_name = Some(dev.clone());
                                    }
                                }
                            });

                        if ui
                            .button(format!("🔄 {}", tr(lang, "Refresh Devices")))
                            .clicked()
                        {
                            self.available_devices = crate::audio::list_input_devices();
                            self.state.set_toast(tr(lang, "Audio input devices refreshed"));
                        }
                    });

                    ui.add_space(10.0);

                    // Sound Effects toggle & Test chime button
                    ui.horizontal(|ui| {
                        let se_cb = ui.checkbox(
                            &mut self.settings_sound_effects,
                            tr(
                                lang,
                                "Enable Synthesized Audio Earcons (Start/Finish/Error sound effects)",
                            ),
                        );
                        if se_cb.changed() {
                            crate::sound::set_sound_enabled(self.settings_sound_effects);
                        }

                        ui.add_space(8.0);
                        let test_sound_btn = egui::Button::new(format!("🔊 {}", tr(lang, "Test Chime")))
                            .fill(Color32::from_rgb(45, 58, 86))
                            .rounding(Rounding::same(6.0));
                        if ui.add(test_sound_btn).clicked() {
                            crate::sound::play_sound(crate::sound::SoundEffect::StartRecording);
                        }
                    });
                    ui.label(
                        egui::RichText::new(format!(
                            "  └ {}",
                            tr(
                                lang,
                                "Soft pure-Rust synthesized harmonic chimes provide immediate tactile feedback when dictation starts and finishes.",
                            )
                        ))
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                });

        // CARD 3: Language & Floating Overlay
        egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(format!(
                            "🌐  {}",
                            tr(lang, "LANGUAGE & FLOATING OVERLAY HUD")
                        ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // Language Selector
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "Transcription Language:"));
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
                        tr(
                            lang,
                            "Show floating recording symbol & live waveform HUD (overlay)",
                        ),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "  └ {}",
                            tr(
                                lang,
                                "Displays an on-screen pill indicator with animated audio waveform bars when dictating without stealing focus.",
                            )
                        ))
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                });
    }

    fn render_settings_ai(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        ui.add_space(4.0);
        // CARD 1.8: AI Post-Processing & Smart Formatting
        egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(format!(
                            "✨  {}",
                            tr(lang, "AI POST-PROCESSING & SMART FORMATTING")
                        ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    ui.checkbox(
                        &mut self.settings_ai_formatting,
                        tr(
                            lang,
                            "Enable Instant AI Text Formatting & Cleanup (Groq Llama 3.1 8B ~150ms)",
                        ),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                        "  └ {}",
                        tr(
                            lang,
                            "Automatically cleans speech disfluencies, filler words (um, uh, อ่า, แบบว่า), fixes punctuation, and adapts writing tone.",
                        )
                    ))
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    if self.settings_ai_formatting {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(tr(lang, "Writing Style Tone:"));
                            let style_label = match self.settings_writing_style.as_str() {
                                "smart_cleanup" => tr(lang, "Smart Cleanup (Removes stutters, filler words, fixes punctuation)"),
                                "casual" => tr(lang, "Casual & Friendly (Chat and messaging tone)"),
                                "professional" => tr(lang, "Professional & Executive (Polite corporate emails & reports)"),
                                "code" => tr(lang, "Code & CLI Commands (Converts speech to code / terminal syntax)"),
                                "bullet_points" => tr(lang, "Bullet Points (Summary in structured markdown bullets)"),
                                other => other,
                            };
                            egui::ComboBox::from_id_salt("writing_style_combo")
                                .selected_text(style_label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "smart_cleanup".to_string(),
                                        tr(lang, "Smart Cleanup (Removes stutters, filler words, fixes punctuation) [Recommended]"),
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "casual".to_string(),
                                        tr(lang, "Casual & Friendly (Chat and messaging tone)"),
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "professional".to_string(),
                                        tr(lang, "Professional & Executive (Polite corporate emails & reports)"),
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "code".to_string(),
                                        tr(lang, "Code & CLI Commands (Converts speech to code / terminal syntax)"),
                                    );
                                    ui.selectable_value(
                                        &mut self.settings_writing_style,
                                        "bullet_points".to_string(),
                                        tr(lang, "Bullet Points (Summary in structured markdown bullets)"),
                                    );
                                });
                        });
                    }
                });

        // CARD 2: STT AI Service Providers
        egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(format!(
                            "🤖  {}",
                            tr(lang, "SPEECH-TO-TEXT (STT) AI SERVICE PROVIDER")
                        ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // 3 Provider Cards
                    let groq_desc = format!("Model: {}", self.settings_groq_model);
                    let eleven_desc = format!("Model: {}", self.settings_elevenlabs_model);
                    let providers = [
                        (SttProvider::WisprFlow, "⚡ Wispr Flow", tr(lang, "Native gRPC Protobuf streaming").to_string(), Color32::from_rgb(99, 102, 241)),
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
                                        ui.set_width(ui.available_width());
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
                            ui.label(tr(lang, "API Endpoint URL:"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_endpoint)
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);

                            ui.label(tr(lang, "Bearer Auth Token (Optional / Session Token):"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_token)
                                    .password(true)
                                    .hint_text(tr(
                                        lang,
                                        "Auto-detected from local Wispr Flow session if installed",
                                    ))
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);
                            ui.label(tr(lang, "Baseten Gateway API Key (optional, stored securely):"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_baseten_api_key)
                                    .password(true)
                                    .hint_text("Api-Key ... or set WFLOW_BASETEN_API_KEY")
                                    .desired_width(ui.available_width()),
                            );
                            ui.label(
                                egui::RichText::new(tr1(
                                    lang,
                                    "Model ID: {}",
                                    &self.settings_baseten_model,
                                ))
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }
                        SttProvider::Groq => {
                            ui.label(tr(lang, "Groq Cloud API Key (get from console.groq.com):"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_groq_api_key)
                                    .password(true)
                                    .hint_text("gsk_...")
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);
                            ui.label(tr(lang, "Groq Whisper Model:"));
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
                                egui::RichText::new(tr(
                                    lang,
                                    "Groq executes Whisper on LPUs with near-zero latency (~200ms per utterance).",
                                ))
                                    .font(FontId::proportional(11.0))
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }
                        SttProvider::ElevenLabs => {
                            ui.label(tr(lang, "ElevenLabs API Key (get from elevenlabs.io):"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_elevenlabs_api_key)
                                    .password(true)
                                    .hint_text("xi-api-key")
                                    .desired_width(ui.available_width()),
                            );

                            ui.add_space(8.0);
                            ui.label(tr(lang, "ElevenLabs STT Model:"));
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
                                egui::RichText::new(tr(
                                    lang,
                                    "ElevenLabs Scribe STT provides unmatched speech recognition accuracy across languages.",
                                ))
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
                            format!("⏳ {}", tr(lang, "Testing the STT connection..."))
                        } else {
                            format!("🧪 {}", tr(lang, "Test STT Provider"))
                        };
                        let test_btn = egui::Button::new(
                            egui::RichText::new(shape_thai(&btn_label))
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
                                            ui.set_width(ui.available_width());
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
                                            ui.set_width(ui.available_width());
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
    }

    fn render_settings_voice(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        ui.add_space(4.0);
        // CARD 4: Voice filter & text quality
        egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(format!(
                            "🎚️  {}",
                            tr(lang, "VOICE FILTER & TEXT QUALITY")
                        ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // --- Silence filter -------------------------------------------------
                    ui.checkbox(
                        &mut self.settings_silence_filter,
                        tr(lang, "Silence filter (noise gate)"),
                    );
                    ui.label(
                        egui::RichText::new(
                            format!(
                                "  └ {}",
                                tr(
                                    lang,
                                    "Trims silence and background noise before audio reaches the STT model, reducing errors and saving bandwidth.",
                                )
                            ),
                        )
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "Speech sensitivity:"));
                        ui.add(
                            egui::DragValue::new(&mut self.settings_silence_sensitivity)
                                .range(0.0..=1.0)
                                .speed(0.02)
                                .suffix(""),
                        );
                        ui.label(
                            egui::RichText::new(if self.settings_silence_sensitivity > 0.66 {
                                tr(lang, "Very sensitive")
                            } else if self.settings_silence_sensitivity > 0.33 {
                                tr(lang, "Normal")
                            } else {
                                tr(lang, "Less sensitive (keeps quiet speech)")
                            })
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "Stop automatically after silence:"));
                        ui.add(
                            egui::DragValue::new(&mut self.settings_silence_stop_secs)
                                .range(0..=600)
                                .speed(1.0)
                                .suffix(tr(lang, " sec")),
                        );
                        if self.settings_silence_stop_secs == 0 {
                            ui.label(
                                egui::RichText::new(tr(lang, "(0 = never stop automatically)"))
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
                        tr(lang, "Hallucination guard"),
                    );
                    ui.label(
                        egui::RichText::new(
                            format!(
                                "  └ {}",
                                tr(
                                    lang,
                                    "Detects invented text the model tends to add on its own, such as \"Thank you.\" or \"(upbeat music)\", and long repeated words, then drops it instead of typing it.",
                                )
                            ),
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
                        tr(lang, "Automatic spell check"),
                    );
                    ui.checkbox(
                        &mut self.settings_mixed_spacing,
                        tr(lang, "Mixed-language spacing"),
                    );
                    ui.label(
                        egui::RichText::new(
                            format!(
                                "  └ {}",
                                tr(
                                    lang,
                                    "For example \"ทดสอบAPI\" → \"ทดสอบ API\". Also fixes common misspellings in each language.",
                                )
                            ),
                        )
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut self.settings_autocorrect_use_llm,
                        tr(lang, "Use a language model for spelling (requires a Groq API key)"),
                    );

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "Languages to spell check:"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings_autocorrect_languages)
                                .desired_width(160.0)
                                .hint_text(tr(lang, "e.g. th,en")),
                        );
                        ui.label(
                            egui::RichText::new(tr(
                                lang,
                                "(leave blank = detect from the text automatically)",
                            ))
                                .font(FontId::proportional(11.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });
                });
    }

    fn render_settings_privacy(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        ui.add_space(4.0);
        // CARD 5: Privacy & Data
        egui::Frame::none()
                .fill(Color32::from_rgb(17, 23, 36))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
                .rounding(Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(format!(
                            "🔒  {}",
                            tr(lang, "PRIVACY & LOCAL DATA")
                        ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(10.0);

                    ui.checkbox(
                        &mut self.settings_share_context,
                        tr(lang, "Share surrounding text context with the STT provider"),
                    );
                    ui.label(
                        egui::RichText::new(
                            format!(
                                "  └ {}",
                                tr(
                                    lang,
                                    "Turn off for maximum privacy: before/after text and selected text are never uploaded (recommended for sensitive text).",
                                )
                            ),
                        )
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut self.settings_store_textbox_contents,
                        tr(lang, "Store textbox contents in the local history"),
                    );

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "STT request timeout (seconds):"));
                        ui.add(
                            egui::DragValue::new(&mut self.settings_stt_timeout_secs)
                                .range(5..=300)
                                .speed(1.0)
                                .suffix(" s"),
                        );
                    });

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "Maximum recording length (seconds):"));
                        ui.add(
                            egui::DragValue::new(&mut self.settings_max_recording_secs)
                                .range(10..=3600)
                                .speed(5.0)
                                .suffix(" s"),
                        );
                    });
                    ui.label(
                        egui::RichText::new(format!(
                            "  └ {}",
                            tr(
                                lang,
                                "Recording stops automatically at this limit to protect memory.",
                            )
                        ))
                            .font(FontId::proportional(11.0))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let (label, fill) = if self.clear_history_armed {
                            (
                                format!("⚠️ {}", tr(lang, "Click again to clear all history")),
                                Color32::from_rgb(185, 28, 28),
                            )
                        } else {
                            (
                                format!("🗑️ {}", tr(lang, "Clear all dictation history")),
                                Color32::from_rgb(51, 65, 85),
                            )
                        };
                        let clear_btn = egui::Button::new(
                            egui::RichText::new(shape_thai(&label))
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
                                self.state.set_toast(tr(
                                    lang,
                                    "Click the button again to confirm clearing history",
                                ));
                            }
                        }
                        if self.clear_history_armed && ui.button(tr(lang, "Cancel")).clicked() {
                            self.clear_history_armed = false;
                        }
                    });

                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(tr1(
                            lang,
                            "Data is stored in: {}",
                            crate::paths::data_dir().display(),
                        ))
                        .font(FontId::proportional(10.5))
                        .color(Color32::from_rgb(100, 116, 139)),
                    );
                });
    }

    fn render_settings_updates(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        ui.add_space(4.0);
        // CARD 5: Updates & Diagnostics
        egui::Frame::none()
            .fill(Color32::from_rgb(17, 23, 36))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(33, 44, 66)))
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(format!(
                        "🔄  {}",
                        tr(lang, "SOFTWARE UPDATES & DIAGNOSTICS")
                    ))
                    .font(FontId::proportional(12.5))
                    .color(Color32::from_rgb(148, 163, 184))
                    .strong(),
                );
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Wflow v{}", crate::update::current_version()))
                            .font(FontId::monospace(12.5))
                            .color(Color32::from_rgb(56, 189, 248))
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "• {}",
                            tr(lang, "Checked against GitHub Releases")
                        ))
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                    );
                });

                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.settings_check_updates,
                    tr(lang, "Check for new versions in the background"),
                );
                ui.checkbox(
                    &mut self.settings_auto_update,
                    tr(lang, "Install new versions automatically"),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "  └ {}",
                        tr(
                            lang,
                            "Downloads the release for this platform, verifies its checksum, then restarts Wflow. It waits until you are not dictating."
                        )
                    ))
                    .font(FontId::proportional(10.5))
                    .color(Color32::from_rgb(100, 116, 139)),
                );
                ui.checkbox(
                    &mut self.settings_include_prerelease,
                    tr(lang, "Include pre-release versions"),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "  └ {}",
                        tr(
                            lang,
                            "Changes on this page apply after pressing Save All Preferences"
                        )
                    ))
                    .font(FontId::proportional(10.5))
                    .color(Color32::from_rgb(100, 116, 139)),
                );
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Check every:"));
                    ui.add(
                        egui::DragValue::new(&mut self.settings_update_interval_hours)
                            .range(1..=720)
                            .speed(1.0)
                            .suffix(tr(lang, " hours")),
                    );
                });

                ui.add_space(10.0);
                let running = self.state.update_check_running.load(Ordering::SeqCst);
                ui.horizontal(|ui| {
                    let check_btn = egui::Button::new(
                        egui::RichText::new(shape_thai(&if running {
                            format!("⏳ {}", tr(lang, "Checking for updates..."))
                        } else {
                            format!("🔍 {}", tr(lang, "Check now"))
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
                    if ui
                        .button(format!("🌐 {}", tr(lang, "Open Releases page")))
                        .clicked()
                    {
                        let _ = self.event_tx.send(GuiEvent::OpenUrl(format!(
                            "https://github.com/{}/releases",
                            crate::update::DEFAULT_REPO
                        )));
                    }
                    if ui
                        .button(format!("📂 {}", tr(lang, "Open Log Folder")))
                        .clicked()
                    {
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
                            "🎉 {}",
                            tr2(
                                lang,
                                "New version available: v{} (current v{})",
                                &info.latest,
                                &info.current
                            )
                        )))
                        .font(FontId::proportional(12.0))
                        .color(Color32::from_rgb(134, 239, 172)),
                    );
                    let install_running = lock(&self.state.update_install).is_running();
                    ui.horizontal(|ui| {
                        let install_label = if install_running {
                            format!("⏳ {}", tr1(lang, "Installing v{}...", &info.latest))
                        } else {
                            format!("⬇️ {}", tr1(lang, "Install v{}", &info.latest))
                        };
                        if ui
                            .add_enabled(
                                !install_running,
                                egui::Button::new(
                                    egui::RichText::new(shape_thai(&install_label))
                                        .font(FontId::proportional(12.0))
                                        .color(Color32::WHITE)
                                        .strong(),
                                )
                                .fill(Color32::from_rgb(22, 163, 74))
                                .rounding(Rounding::same(6.0)),
                            )
                            .clicked()
                        {
                            let _ = self.event_tx.send(GuiEvent::InstallUpdate);
                        }
                        if ui
                            .button(format!("📝 {}", tr(lang, "View Release Notes")))
                            .clicked()
                        {
                            let _ = self
                                .event_tx
                                .send(GuiEvent::OpenUrl(info.release_url.clone()));
                        }
                    });

                    // Progress of the running installation.
                    let install_state = lock(&self.state.update_install).clone();
                    if install_state.is_running() {
                        ui.add(
                            egui::ProgressBar::new(install_state.percent as f32 / 100.0)
                                .desired_width(ui.available_width())
                                .fill(Color32::from_rgb(34, 197, 94)),
                        );
                        ui.label(
                            egui::RichText::new(shape_thai(&install_state.message))
                                .font(FontId::proportional(10.5))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    }
                } else if let Some(err) = update_error {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(shape_thai(&format!(
                            "⚠️ {}",
                            tr1(lang, "Update check failed: {}", err)
                        )))
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
                        egui::RichText::new(tr1(
                            lang,
                            "Last checked: {}",
                            friendly_timestamp(&last),
                        ))
                        .font(FontId::proportional(10.5))
                        .color(Color32::from_rgb(100, 116, 139)),
                    );
                }
            });
    }

    /// The "Save all preferences" button, shared by every settings section.
    fn render_settings_save_button(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang;
        ui.add_space(8.0);
        let dirty = self.settings_dirty();

        if dirty {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("●  {}", tr(lang, "You have unsaved changes")))
                        .font(FontId::proportional(12.0))
                        .color(Color32::from_rgb(251, 191, 36))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(tr(lang, "(press Save All Preferences to apply)"))
                        .font(FontId::proportional(11.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                );
            });
            ui.add_space(4.0);
        }

        let save_label = if dirty {
            format!("💾  {}  •", tr(lang, "Save All Preferences"))
        } else {
            format!("💾  {}", tr(lang, "Save All Preferences"))
        };
        let save_btn = egui::Button::new(
            egui::RichText::new(save_label)
                .font(FontId::proportional(14.0))
                .color(Color32::WHITE)
                .strong(),
        )
        .fill(if dirty {
            Color32::from_rgb(202, 138, 4)
        } else {
            Color32::from_rgb(79, 70, 229)
        })
        .stroke(Stroke::new(
            1.0_f32,
            if dirty {
                Color32::from_rgb(251, 191, 36)
            } else {
                Color32::from_rgb(129, 140, 248)
            },
        ))
        .rounding(Rounding::same(8.0))
        .min_size(Vec2::new(240.0, 42.0));

        if ui.add(save_btn).clicked() {
            self.save_all_settings();
        }
    }

    /// Writes every setting back to disk and notifies the background worker.
    fn save_all_settings(&mut self) {
        let new_config = self.build_settings_config();

        crate::sound::set_sound_enabled(new_config.sound_effects);

        if let Ok(mut lock) = self.state.config.lock() {
            *lock = new_config.clone();
        }
        let _ = self
            .event_tx
            .send(GuiEvent::SaveConfig(Box::new(new_config)));
        self.state.set_toast(tr(
            self.ui_lang,
            "Settings & STT configurations saved successfully!",
        ));
    }

    /// Builds the configuration from the current state of the settings form.
    fn build_settings_config(&self) -> AppConfig {
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
            auto_update: self.settings_auto_update,
            include_prerelease: self.settings_include_prerelease,
            update_check_interval_hours: self.settings_update_interval_hours.max(1),
            last_update_check,
            skipped_update_version,
            max_recording_secs: self.settings_max_recording_secs.clamp(10, 3600),
            client_name,
            client_version,
            stt_timeout_secs,
            ui_language: self.settings_ui_language.clone(),
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

        new_config
    }

    /// True when the settings form differs from what is stored on disk.
    fn settings_dirty(&self) -> bool {
        let mut candidate = self.build_settings_config();
        let saved = lock(&self.state.config).clone();

        // Fields that the settings form does not own must not influence the
        // comparison, otherwise the page would always look "unsaved".
        candidate.user_id = saved.user_id.clone();
        candidate.last_update_check = saved.last_update_check.clone();
        candidate.skipped_update_version = saved.skipped_update_version.clone();
        candidate.hotkey_vk = saved.hotkey_vk;
        candidate.toggle_window_vk = saved.toggle_window_vk;
        candidate.auto_start = saved.auto_start;

        candidate != saved
    }

    /// Handles a click on the update button: download the new release when
    /// there is one, otherwise start a check.
    fn trigger_update_action(&self, pending: &Option<UpdateInfo>) {
        if let Some(info) = pending.as_ref() {
            // Install it right here: download → checksum → replace → restart.
            let _ = self.event_tx.send(GuiEvent::InstallUpdate);
            self.state
                .set_toast(&format!("กำลังติดตั้งอัปเดต v{}", info.latest));
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
        // The skipped version is kept outside of the config mutex: this
        // function runs while the sidebar still holds the config lock, and
        // locking a `std::sync::Mutex` twice on the same thread deadlocks.
        let skipped = lock(&self.state.skipped_update_version)
            .as_deref()
            .map(|v| v == latest)
            .unwrap_or(false);
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
                                    egui::RichText::new(shape_thai("⬇️ ติดตั้ง"))
                                        .font(FontId::proportional(11.5))
                                        .color(Color32::WHITE)
                                        .strong(),
                                )
                                .fill(Color32::from_rgb(22, 163, 74)),
                            )
                            .clicked()
                        {
                            let _ = self.event_tx.send(GuiEvent::InstallUpdate);
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

        let lang = self.ui_lang;
        let badge = self
            .state
            .config
            .lock()
            .map(|cfg| cfg.stt_provider.short_badge())
            .unwrap_or("Wflow");

        let (label, color) = match status {
            DictationStatus::Listening => {
                (tr(lang, "Listening..."), Color32::from_rgb(239, 68, 68))
            }
            DictationStatus::Transcribing => {
                (tr(lang, "Processing..."), Color32::from_rgb(168, 85, 247))
            }
            DictationStatus::Pasting | DictationStatus::Done => {
                (tr(lang, "Text inserted"), Color32::from_rgb(16, 185, 129))
            }
            DictationStatus::Error => (
                tr(lang, "Something went wrong"),
                Color32::from_rgb(245, 158, 11),
            ),
            DictationStatus::Idle => (tr(lang, "Ready"), Color32::from_rgb(148, 163, 184)),
        };

        let viewport_id = egui::ViewportId::from_hash_of("wflow_hud");
        let monitor = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(egui::vec2(1920.0, 1080.0));
        let hud_width = 320.0;
        let position = egui::pos2((monitor.x - hud_width) * 0.5, monitor.y - 110.0);
        ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::OuterPosition(position));

        let builder = egui::ViewportBuilder::default()
            .with_title("Wflow HUD")
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            // The HUD carries a real stop button, so it must receive clicks.
            .with_mouse_passthrough(false)
            .with_resizable(false)
            .with_inner_size([hud_width, 58.0]);

        let animation_time = self.animation_time;
        let badge = badge.to_string();
        let label = label.to_string();
        let event_tx = self.event_tx.clone();
        // Only the active states offer a stop button; a finished status just
        // fades away by itself.
        let can_stop = matches!(
            status,
            DictationStatus::Listening | DictationStatus::Transcribing
        );

        ctx.show_viewport_immediate(viewport_id, builder, move |ctx, _class| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::TRANSPARENT))
                .show(ctx, |ui| {
                    let rect = ui.max_rect();
                    let painter = ui.painter();
                    let pill = rect.shrink(3.0);
                    painter.rect_filled(
                        pill,
                        Rounding::same(22.0),
                        Color32::from_rgba_unmultiplied(12, 16, 26, 238),
                    );
                    painter.rect_stroke(
                        pill,
                        Rounding::same(22.0),
                        Stroke::new(1.0_f32, color.gamma_multiply(0.7)),
                    );

                    painter.circle_filled(
                        Pos2::new(pill.min.x + 22.0, pill.center().y),
                        5.0,
                        color,
                    );

                    // Measure instead of hard-coding offsets: the provider
                    // badge and the status label are both translated, so a
                    // fixed x used to let the two collide in Thai, and let the
                    // label run under the waveform on the right.
                    let badge_galley = painter.layout_no_wrap(
                        badge.clone(),
                        FontId::proportional(11.5),
                        Color32::from_rgb(148, 163, 184),
                    );
                    let badge_x = pill.min.x + 36.0;
                    // A galley is drawn from its top-left corner, so shift it
                    // up by half its height to sit on the pill's centre line.
                    painter.galley(
                        Pos2::new(badge_x, pill.center().y - badge_galley.size().y / 2.0),
                        badge_galley.clone(),
                        Color32::from_rgb(148, 163, 184),
                    );

                    let label_x = badge_x + badge_galley.size().x + 10.0;
                    // Everything left of the waveform / stop button.
                    let right_reserve = if can_stop { 58.0 } else { 62.0 };
                    let room = (pill.max.x - right_reserve - label_x).max(24.0);
                    let label_galley = painter.layout_no_wrap(
                        shape_thai(&label).to_string(),
                        FontId::proportional(12.5),
                        Color32::WHITE,
                    );
                    let label_galley = if label_galley.size().x > room {
                        let mut job = egui::text::LayoutJob::simple(
                            shape_thai(&label).to_string(),
                            FontId::proportional(12.5),
                            Color32::WHITE,
                            room,
                        );
                        // Thai is written without spaces between words, so
                        // wrapping on word boundaries alone would overflow.
                        job.wrap.break_anywhere = true;
                        painter.layout_job(job)
                    } else {
                        label_galley
                    };
                    painter.galley(
                        Pos2::new(label_x, pill.center().y - label_galley.size().y / 2.0),
                        label_galley,
                        Color32::WHITE,
                    );

                    // Miniature waveform reacts to the live microphone level
                    // and shrinks while the stop button takes the right edge.
                    let bar_count = 6;
                    let base_x = pill.max.x - 92.0;
                    for index in 0..bar_count {
                        let phase = animation_time * 6.0 + index as f32 * 0.9;
                        let level = (amplitude * 18.0).clamp(0.0, 1.0);
                        let height = 3.0 + (phase.sin().abs() * 14.0 * (0.25 + level));
                        let x = base_x + index as f32 * 7.0;
                        painter.rect_filled(
                            Rect::from_center_size(
                                Pos2::new(x, pill.center().y),
                                Vec2::new(3.5, height),
                            ),
                            Rounding::same(2.0),
                            color.gamma_multiply(0.85),
                        );
                    }

                    if can_stop {
                        let button_rect = Rect::from_center_size(
                            Pos2::new(pill.max.x - 26.0, pill.center().y),
                            Vec2::splat(30.0),
                        );
                        let response = ui
                            .interact(button_rect, ui.id().with("hud_stop"), egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(tr(lang, "Stop recording"));

                        let hovered = response.hovered();
                        painter.circle_filled(
                            button_rect.center(),
                            14.0,
                            if hovered {
                                Color32::from_rgb(248, 113, 113)
                            } else {
                                Color32::from_rgb(220, 38, 38)
                            },
                        );
                        painter.rect_filled(
                            Rect::from_center_size(button_rect.center(), Vec2::splat(9.0)),
                            Rounding::same(2.0),
                            Color32::WHITE,
                        );

                        if response.clicked() {
                            let _ = event_tx.send(GuiEvent::StopDictation);
                        }
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

        // Run the test on the application's runtime: the shared HTTP client is
        // bound to it and would stop working on a throw-away runtime.
        let runtime = self.runtime.clone();
        runtime.spawn(async move {
            let res = match provider {
                SttProvider::Groq => crate::stt::test_groq_api(&groq_key).await,
                SttProvider::ElevenLabs => crate::stt::test_elevenlabs_api(&eleven_key).await,
                SttProvider::WisprFlow => crate::stt::test_wisprflow_endpoint(&endpoint).await,
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
    }

    fn render_exit_confirmation_modal(&mut self, ctx: &egui::Context) {
        let lang = self.ui_lang;
        let toggle_key_name = self.settings_toggle.format_display();
        let mut close_modal = false;
        let mut do_exit = false;
        let mut do_tray = false;

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close_modal = true;
        }

        egui::Window::new(shape_thai(&format!(
            "⚠️ {}",
            tr(lang, "Confirm Exit")
        )))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([380.0, 150.0])
            // The theme's window fill is translucent; the dialog needs to be
            // fully opaque so the page behind it does not show through.
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(Color32::from_rgb(17, 23, 36))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(51, 65, 85))),
            )
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(shape_thai(tr(
                        lang,
                        "Do you really want to quit Wflow?",
                    )))
                        .font(FontId::proportional(14.5))
                        .color(Color32::WHITE)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(shape_thai(tr(
                        lang,
                        "If you minimize to the tray, Wflow keeps listening for your dictation hotkey in the background.",
                    )))
                    .font(FontId::proportional(11.5))
                    .color(Color32::from_rgb(148, 163, 184)),
                );
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    // 1. Confirm Exit
                    let exit_btn = egui::Button::new(
                        egui::RichText::new(shape_thai(&format!(
                            "🔴 {}",
                            tr(lang, "Exit")
                        )))
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
                        egui::RichText::new(shape_thai(&format!(
                            "📌 {}",
                            tr(lang, "Minimize to Tray")
                        )))
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
                        egui::RichText::new(shape_thai(tr(lang, "Cancel")))
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
            crate::platform::PlatformManager::set_quitting(true);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(2000));
                std::process::exit(0);
            });
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
            self.state.set_toast(&tr1(
                self.ui_lang,
                "Minimized to the tray. Press {} or click the tray icon to reopen.",
                toggle_key_name,
            ));
        } else if close_modal {
            self.show_exit_confirmation = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The PUA probe has to answer "no" for a font that genuinely lacks the
    /// WTT block — that is the case for Noto Sans Thai, Leelawadee UI and
    /// Tahoma, and shaping onto their missing glyphs is what painted the
    /// interface in tofu boxes.
    #[test]
    fn font_without_the_pua_block_disables_shaping() {
        assert!(!font_has_thai_pua(include_bytes!(
            "../assets/NotoEmoji-Regular.ttf"
        )));
        assert!(!font_has_thai_pua(b"not a font at all"));
    }

    /// Positive control for the same lookup: if the probe cannot see a glyph
    /// that really is in the file, it would also miss the PUA block on a TLWG
    /// font and silently disable shaping for every Linux user.
    #[test]
    fn glyph_probe_reports_glyphs_that_are_present() {
        use ab_glyph::Font as _;
        let font = ab_glyph::FontArc::try_from_vec(
            include_bytes!("../assets/NotoEmoji-Regular.ttf").to_vec(),
        )
        .expect("bundled emoji font should parse");
        assert_ne!(
            font.glyph_id('\u{1F600}'),
            ab_glyph::GlyphId(0),
            "U+1F600 is in Noto Emoji, so the probe must find it"
        );
    }

    #[test]
    fn update_button_is_always_labelled() {
        let (label, _, _, _) = update_button_style(Lang::Th, None, false, "0.2.1", 0.0);
        assert!(label.contains("อัปเดต"), "idle label: {}", label);
        assert!(label.contains("0.2.1"), "idle label: {}", label);
    }

    #[test]
    fn update_button_highlights_a_new_release() {
        let (label, fill, _, text) =
            update_button_style(Lang::Th, Some("0.3.0"), false, "0.2.1", 0.0);
        assert!(label.contains("0.3.0"), "label: {}", label);
        assert_eq!(fill, Color32::from_rgb(22, 163, 74));
        assert_eq!(text, Color32::WHITE);
    }

    #[test]
    fn update_button_shows_progress() {
        let (label, _, _, _) = update_button_style(Lang::Th, None, true, "0.2.1", 0.0);
        assert!(label.contains("กำลังตรวจสอบ"), "label: {}", label);
    }

    /// The sidebar renders while the config mutex is held, and it asks for the
    /// pending update. Reading the skipped version from the config there would
    /// lock a `std::sync::Mutex` twice on the same thread and freeze the whole
    /// interface as soon as a new release is available.
    #[tokio::test]
    async fn pending_update_does_not_lock_the_config() {
        let state = AppState::new(AppConfig::default(), Vec::new(), Vec::new());
        let gui = WisprFlowGui::new(
            state.clone(),
            mpsc::unbounded_channel().0,
            tokio::runtime::Handle::current(),
        );

        // Simulate a new release.
        if let Ok(mut lock) = state.update_info.lock() {
            *lock = Some(UpdateInfo {
                current: semver::Version::new(0, 3, 4),
                latest: semver::Version::new(0, 3, 5),
                title: "v0.3.5".into(),
                notes: String::new(),
                release_url: "https://example.test/release".into(),
                download_url: "https://example.test/deb".into(),
                archive_url: Some("https://example.test/tar.gz".into()),
                deb_url: Some("https://example.test/deb".into()),
                windows_setup_url: None,
                checksums_url: Some("https://example.test/SHA256SUMS.txt".into()),
                published_at: None,
                prerelease: false,
            });
        }

        // Hold the config lock, like the sidebar does, and ask for the update.
        let _cfg = lock(&state.config);
        let pending = gui.pending_update();
        assert!(pending.is_some(), "the new release must be visible");
    }

    /// A version the user skipped must not be offered again.
    #[tokio::test]
    async fn skipped_version_is_not_offered() {
        let state = AppState::new(AppConfig::default(), Vec::new(), Vec::new());
        let gui = WisprFlowGui::new(
            state.clone(),
            mpsc::unbounded_channel().0,
            tokio::runtime::Handle::current(),
        );

        if let Ok(mut lock) = state.update_info.lock() {
            *lock = Some(UpdateInfo {
                current: semver::Version::new(0, 3, 4),
                latest: semver::Version::new(0, 3, 5),
                title: "v0.3.5".into(),
                notes: String::new(),
                release_url: "https://example.test/release".into(),
                download_url: "https://example.test/deb".into(),
                archive_url: Some("https://example.test/tar.gz".into()),
                deb_url: Some("https://example.test/deb".into()),
                windows_setup_url: None,
                checksums_url: Some("https://example.test/SHA256SUMS.txt".into()),
                published_at: None,
                prerelease: false,
            });
        }
        if let Ok(mut skipped) = state.skipped_update_version.lock() {
            *skipped = Some("0.3.5".to_string());
        }

        assert!(gui.pending_update().is_none());
    }
}
