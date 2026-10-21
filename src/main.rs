#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use tracing_subscriber::prelude::*;

use wflow::audio::{AudioRecorder, TARGET_SAMPLE_RATE};
use wflow::config::AppConfig;
use wflow::db::{Database, HistoryRecord};
use wflow::grpc::FlowGrpcClient;
use wflow::gui::{AppState, GuiEvent, WisprFlowGui};
use wflow::platform::{ActiveAppContext, HotkeyEvent, PlatformManager};
use wflow::proto::*;
use wflow::stt::{pcm_to_wav, transcribe_elevenlabs, transcribe_groq, SttProvider};
use wflow::text::replace_case_insensitive;
use wflow::ui::DictationStatus;

const TICK_INTERVAL: Duration = Duration::from_millis(30);
const TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(15);

fn main() -> Result<()> {
    // Handle --version / --help before any logging or side effects.
    match parse_cli() {
        Cli::Version => {
            println!("wflow {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Cli::Help => {
            print_help();
            return Ok(());
        }
        Cli::CheckUpdates => {
            let _log_guard = init_logging();
            return run_update_check_cli();
        }
        Cli::Run => {}
    }

    let _log_guard = init_logging();
    install_panic_hook();
    info!("Starting Wflow (Rust Native Edition)...");

    // Single instance: a second copy would duplicate hooks and tray icons.
    let _instance = match wflow::instance::InstanceGuard::acquire(&wflow::paths::config_dir())? {
        Some(guard) => guard,
        None => {
            warn!("Another Wflow instance is already running; exiting.");
            return Ok(());
        }
    };

    let config = AppConfig::load_or_default(None);
    wflow::sound::set_sound_enabled(config.sound_effects);

    let db = Arc::new(Database::open_or_default(None)?);
    let initial_history = db.get_recent_history(50).unwrap_or_default();
    let initial_dict = db.get_all_dictionary_entries().unwrap_or_default();

    let app_state = AppState::new(config.clone(), initial_history, initial_dict);
    let app_state_worker = app_state.clone();
    let app_state_gui = app_state.clone();

    let (gui_tx, mut gui_rx) = mpsc::unbounded_channel::<GuiEvent>();
    let (hotkey_tx, mut hotkey_rx) = mpsc::unbounded_channel::<HotkeyEvent>();

    let _hook_thread =
        PlatformManager::start_hotkey_listener(hotkey_tx, config.hotkey, config.toggle_window)?;

    if config.check_updates
        && wflow::update::should_check(
            config.last_update_check.as_deref(),
            config.update_check_interval_hours,
        )
    {
        spawn_update_check(app_state.clone(), config.include_prerelease);
    }

    let db_worker = Arc::clone(&db);
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to create Tokio runtime: {}", e);
                return;
            }
        };

        rt.block_on(async move {
            run_background_coordinator(app_state_worker, db_worker, &mut hotkey_rx, &mut gui_rx)
                .await;
        });
    });

    let is_startup =
        std::env::args().any(|arg| arg == "--startup" || arg == "--tray" || arg == "--minimized");
    if is_startup {
        info!("Launched with startup/minimized flag, starting in tray...");
        app_state.window_visible.store(false, Ordering::SeqCst);
    }

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([980.0, 660.0])
        .with_min_inner_size([820.0, 540.0])
        .with_title("Wflow - AI Voice Studio")
        .with_active(!is_startup)
        .with_visible(!is_startup);

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    info!("Launching Native GUI window...");
    let run_result = eframe::run_native(
        "Wflow",
        native_options,
        Box::new(move |_cc| Ok(Box::new(WisprFlowGui::new(app_state_gui, gui_tx)))),
    );

    if let Err(err) = run_result {
        error!("GUI terminated with an error: {:?}", err);
        return Err(anyhow::anyhow!("GUI terminated: {:?}", err));
    }

    info!("Wflow shut down cleanly.");
    Ok(())
}

enum Cli {
    Run,
    Version,
    Help,
    CheckUpdates,
}

fn parse_cli() -> Cli {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-V" | "--version" => return Cli::Version,
            "--check-updates" => return Cli::CheckUpdates,
            "-h" | "--help" => return Cli::Help,
            _ => {}
        }
    }
    Cli::Run
}

fn print_help() {
    println!(
        "Wflow {}\n\nUSAGE:\n    wflow [OPTIONS]\n\nOPTIONS:\n    --startup          Start minimized in the tray\n    --check-updates    Check for updates and exit\n    -V, --version      Print version\n    -h, --help         Show this help\n\nENVIRONMENT:\n    RUST_LOG                  Log filter (default: info)\n    WFLOW_AUTH_TOKEN          Account JWT (overrides stored secrets)\n    WFLOW_GROQ_API_KEY        Groq API key\n    WFLOW_ELEVENLABS_API_KEY  ElevenLabs API key\n    WFLOW_BASETEN_API_KEY     Baseten gateway key\n    WFLOW_UPDATE_REPO         GitHub repo used by the update checker",
        env!("CARGO_PKG_VERSION")
    );
}

fn run_update_check_cli() -> Result<()> {
    let config = AppConfig::load_or_default(None);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = rt.block_on(wflow::update::check(config.include_prerelease));
    match result {
        Ok(Some(info)) => {
            println!("Update available: {} -> {}", info.current, info.latest);
            println!("Download: {}", info.download_url);
            println!("Release:  {}", info.release_url);
            if !info.notes.trim().is_empty() {
                println!("\n{}", info.notes.trim());
            }
        }
        Ok(None) => println!("Wflow {} is up to date.", wflow::update::current_version()),
        Err(err) => {
            eprintln!("Update check failed: {err:#}");
            std::process::exit(1);
        }
    }
    Ok(())
}

fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let log_dir = wflow::paths::ensure_dir(wflow::paths::log_dir());
    let file_appender = tracing_appender::rolling::daily(&log_dir, "wflow.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    #[cfg(target_os = "windows")]
    {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(non_blocking),
            )
            .init();
    }
    #[cfg(not(target_os = "windows"))]
    {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(true),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(non_blocking),
            )
            .init();
    }

    info!("Logging to {:?}", log_dir);
    Some(guard)
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        error!("PANIC: {}", info);
        default_hook(info);
    }));
}

fn load_app_icon() -> Option<eframe::egui::IconData> {
    let icon_bytes = include_bytes!("../assets/icon_256.png");
    let img = image::load_from_memory(icon_bytes).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(eframe::egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

/// Runs an update check in the background and publishes the result to the GUI.
fn spawn_update_check(state: AppState, include_prerelease: bool) {
    if state.update_check_running.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        match rt {
            Ok(rt) => {
                let result = rt.block_on(wflow::update::check(include_prerelease));
                match result {
                    Ok(info) => {
                        if let Ok(mut lock) = state.update_info.lock() {
                            *lock = info;
                        }
                        if let Ok(mut lock) = state.update_error.lock() {
                            *lock = None;
                        }
                    }
                    Err(err) => {
                        warn!("Update check failed: {err:#}");
                        if let Ok(mut lock) = state.update_error.lock() {
                            *lock = Some(format!("{err:#}"));
                        }
                    }
                }
            }
            Err(err) => {
                if let Ok(mut lock) = state.update_error.lock() {
                    *lock = Some(format!("Could not start update checker: {err}"));
                }
            }
        }

        if let Ok(mut cfg) = state.config.lock() {
            cfg.last_update_check = Some(wflow::update::now_rfc3339());
            if let Err(err) = cfg.save(None) {
                warn!("Could not persist last update check: {}", err);
            }
        }

        state.update_check_running.store(false, Ordering::SeqCst);
        if let Ok(lock) = state.egui_ctx.lock() {
            if let Some(ctx) = lock.as_ref() {
                ctx.request_repaint();
            }
        }
    });
}

struct DictationSession {
    recorder: AudioRecorder,
    is_dictating: Arc<AtomicBool>,
    start: Option<Instant>,
    ctx: Option<ActiveAppContext>,
    request_tx: Option<mpsc::Sender<Request>>,
    response_handle: Option<tokio::task::JoinHandle<Option<String>>>,
    samples: Arc<Mutex<Vec<i16>>>,
}

impl DictationSession {
    fn new() -> Self {
        Self {
            recorder: AudioRecorder::new(),
            is_dictating: Arc::new(AtomicBool::new(false)),
            start: None,
            ctx: None,
            request_tx: None,
            response_handle: None,
            samples: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn is_active(&self) -> bool {
        self.is_dictating.load(Ordering::SeqCst)
    }

    async fn start(&mut self, state: &AppState, db: &Arc<Database>) {
        if self.is_active() {
            return;
        }
        self.is_dictating.store(true, Ordering::SeqCst);
        state.is_recording.store(true, Ordering::SeqCst);
        self.start = Some(Instant::now());

        if let Ok(mut samples) = self.samples.lock() {
            samples.clear();
        }
        if let Ok(mut lock) = state.status.lock() {
            *lock = DictationStatus::Listening;
        }

        let (
            endpoint_url,
            user_id,
            language_code,
            auth_token,
            stt_provider,
            show_floating_hud,
            sound_effects,
            input_device,
            share_context,
            baseten_key,
            baseten_model,
            client_name,
            client_version,
            gate_config,
        ) = {
            let cfg = state.config.lock().unwrap_or_else(|e| e.into_inner());
            (
                cfg.endpoint_url.clone(),
                cfg.user_id.clone(),
                cfg.language_code.clone(),
                cfg.auth_token.clone(),
                cfg.stt_provider,
                cfg.show_floating_hud,
                cfg.sound_effects,
                cfg.input_device_name.clone(),
                cfg.share_context,
                cfg.baseten_api_key.clone(),
                cfg.baseten_model_id.clone(),
                cfg.client_name.clone(),
                parse_client_version(&cfg.client_version),
                wflow::vad::GateConfig {
                    enabled: cfg.silence_filter,
                    manual_threshold: None,
                    // A higher sensitivity means a smaller margin above the
                    // measured noise floor.
                    margin: 0.02 - (cfg.silence_sensitivity.clamp(0.0, 1.0) * 0.019),
                    pre_roll_ms: 250,
                    stop_after_silence_ms: cfg.silence_stop_secs * 1000,
                },
            )
        };

        wflow::sound::set_sound_enabled(sound_effects);
        wflow::sound::play_sound(wflow::sound::SoundEffect::StartRecording);

        if show_floating_hud {
            PlatformManager::show_floating_hud(stt_provider.short_badge());
            PlatformManager::set_hud_status(DictationStatus::Listening);
        } else {
            PlatformManager::set_tray_recording_state(true);
        }

        let app_ctx = PlatformManager::get_active_app_context();
        info!(
            "Active target context: app='{}', title='{}', before_chars={}, share_context={}",
            app_ctx.process_name,
            app_ctx.window_title,
            app_ctx.before_text.as_ref().map(|s| s.len()).unwrap_or(0),
            share_context
        );
        if let Ok(mut lock) = state.active_app.lock() {
            *lock = app_ctx.clone();
        }
        self.ctx = Some(app_ctx.clone());

        let dictionary_words = db.get_dictionary_words().unwrap_or_default();
        let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(200);

        // Noise gate: only speech is forwarded to the speech-to-text provider.
        let gate = if gate_config.enabled {
            Some(Arc::new(Mutex::new(wflow::vad::VoiceGate::new(
                gate_config,
            ))))
        } else {
            None
        };

        if let Err(e) = self
            .recorder
            .start_recording(audio_tx, input_device.as_deref(), gate)
        {
            error!("Microphone start failed: {}", e);
            wflow::sound::play_sound(wflow::sound::SoundEffect::Error);
            if let Ok(mut lock) = state.status.lock() {
                *lock = DictationStatus::Error;
            }
            self.is_dictating.store(false, Ordering::SeqCst);
            state.is_recording.store(false, Ordering::SeqCst);
            if show_floating_hud {
                PlatformManager::hide_floating_hud();
            }
            state.set_toast("ไม่สามารถเปิดไมโครโฟนได้ กรุณาตรวจสอบอุปกรณ์เสียง");
            return;
        }

        match stt_provider {
            SttProvider::WisprFlow => {
                let (grpc_req_tx, grpc_req_rx) = mpsc::channel::<Request>(100);
                self.request_tx = Some(grpc_req_tx.clone());

                let session_id = uuid::Uuid::new_v4().to_string();
                let request_id = uuid::Uuid::new_v4().to_string();

                let lang_id = match language_code.to_lowercase().as_str() {
                    "th" | "thai" => 34,
                    "en" | "en-us" | "english" => 1,
                    "zh" | "chinese" => 3,
                    "ja" | "japanese" => 10,
                    _ => 0,
                };
                let language_vec = if lang_id == 0 { vec![0] } else { vec![lang_id] };

                let init_req = Request {
                    init: Some(Init {
                        metadata: Some(Metadata {
                            user_id: user_id.clone(),
                            session_id,
                            request_id,
                            audio_encoding: AudioEncoding::Wav as i32,
                            environment: Environment::Production as i32,
                            client: Some(Client {
                                name: client_name,
                                platform: client_platform() as i32,
                                version: Some(client_version),
                            }),
                            debug: false,
                            referral_code: String::new(),
                            always_wait_scribe: false,
                        }),
                        preferences: Some(Preferences {
                            user: None,
                            language: language_vec,
                            vocabulary: Some(StaticVocabulary {
                                dictionary_personal: dictionary_words.clone(),
                                dictionary_team: Vec::new(),
                                dictionary_personal_starred: Vec::new(),
                                dictionary_team_starred: Vec::new(),
                            }),
                            replacements: None,
                            style_config: None,
                        }),
                        state: None,
                    }),
                    context: None,
                    payload: None,
                    commit: Commit::Unspecified as i32,
                };
                // (sent later, once speech has been detected)

                // Privacy: only send clipboard-like context when the user opted in.
                let (before_text, selected_text, after_text, full_text) = if share_context {
                    (
                        app_ctx.before_text.clone(),
                        app_ctx.selected_text.clone(),
                        app_ctx.after_text.clone(),
                        app_ctx.full_text.clone(),
                    )
                } else {
                    (None, None, None, None)
                };

                let context_req = Request {
                    init: None,
                    context: Some(Context {
                        app: Some(App {
                            name: app_ctx.process_name.clone(),
                            bundle_id: app_ctx.process_name.clone(),
                            url: String::new(),
                            r#type: AppType::Other as i32,
                        }),
                        textbox: Some(Textbox {
                            contents: full_text,
                            before_text,
                            selected_text,
                            after_text,
                        }),
                        vocabulary: None,
                        user_identifier: user_id.clone(),
                        content_text: String::new(),
                        content_html: String::new(),
                        session_dictation_apps: Vec::new(),
                        screenshot: Vec::new(),
                    }),
                    payload: None,
                    commit: Commit::Unspecified as i32,
                };
                // (sent later, once speech has been detected)

                // The Init/Context messages are only sent once the noise gate
                // has forwarded the first chunk of speech, so a recording that
                // contains nothing but silence never opens a session with the
                // backend.
                let bridge_tx = grpc_req_tx.clone();
                let is_dictating = Arc::clone(&self.is_dictating);
                let samples = Arc::clone(&self.samples);
                let (speech_tx, speech_rx) = tokio::sync::oneshot::channel::<()>();
                let speech_tx = std::sync::Mutex::new(Some(speech_tx));
                tokio::spawn(async move {
                    let mut started = false;
                    while is_dictating.load(Ordering::SeqCst) {
                        let Some(chunk) = audio_rx.recv().await else {
                            break;
                        };
                        if let Ok(mut buf) = samples.lock() {
                            for pair in chunk.as_chunks::<2>().0 {
                                buf.push(i16::from_le_bytes(*pair));
                            }
                        }

                        if !started {
                            started = true;
                            let _ = bridge_tx.send(init_req.clone()).await;
                            let _ = bridge_tx.send(context_req.clone()).await;
                            // Tell the gRPC task that there is something to
                            // transcribe; until then it stays offline.
                            if let Ok(mut slot) = speech_tx.lock() {
                                if let Some(tx) = slot.take() {
                                    let _ = tx.send(());
                                }
                            }
                        }

                        let payload_req = Request {
                            init: None,
                            context: None,
                            payload: Some(Payload {
                                audio: Some(payload::Audio::AudioPackets(AudioPackets {
                                    items: vec![chunk],
                                })),
                            }),
                            commit: Commit::Unspecified as i32,
                        };
                        if bridge_tx.send(payload_req).await.is_err() {
                            break;
                        }
                    }
                });

                let handle = tokio::spawn(async move {
                    // Wait for the first chunk of speech before connecting.
                    let _ = speech_rx.await;
                    match FlowGrpcClient::connect(&endpoint_url).await {
                        Ok(mut client) => {
                            let stream = tokio_stream::wrappers::ReceiverStream::new(grpc_req_rx);
                            match client
                                .transcribe_stream(
                                    stream,
                                    auth_token.as_deref(),
                                    baseten_key.as_deref(),
                                    &baseten_model,
                                )
                                .await
                            {
                                Ok(mut response_stream) => {
                                    let mut final_text = None;
                                    while let Ok(Some(resp)) = response_stream.message().await {
                                        if let Some(res) = resp.result {
                                            if let Some(out) = res.output {
                                                if !out.plaintext.trim().is_empty() {
                                                    info!(
                                                        "Received final transcript ({} chars)",
                                                        out.plaintext.chars().count()
                                                    );
                                                    final_text = Some(out.plaintext);
                                                    break;
                                                }
                                            }
                                        }
                                        if let Some(st) = resp.state {
                                            if let Some(ft) = st.formatted_text {
                                                if !ft.content.trim().is_empty() {
                                                    final_text = Some(ft.content);
                                                }
                                            }
                                        }
                                    }
                                    final_text
                                }
                                Err(e) => {
                                    warn!("gRPC stream returned error: {}", e);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Could not connect to transcription backend: {}", e);
                            None
                        }
                    }
                });

                self.response_handle = Some(handle);
            }
            SttProvider::Groq | SttProvider::ElevenLabs => {
                self.request_tx = None;
                self.response_handle = None;

                let is_dictating = Arc::clone(&self.is_dictating);
                let samples = Arc::clone(&self.samples);
                tokio::spawn(async move {
                    while is_dictating.load(Ordering::SeqCst) {
                        let Some(chunk) = audio_rx.recv().await else {
                            break;
                        };
                        if let Ok(mut buf) = samples.lock() {
                            for pair in chunk.as_chunks::<2>().0 {
                                buf.push(i16::from_le_bytes(*pair));
                            }
                        }
                    }
                });
            }
        }
    }

    async fn finish(&mut self, state: &AppState, db: &Arc<Database>) {
        if !self.is_active() {
            return;
        }
        self.is_dictating.store(false, Ordering::SeqCst);
        state.is_recording.store(false, Ordering::SeqCst);
        let duration = self.start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        self.start = None;

        self.recorder.stop_recording();

        if let Ok(mut lock) = state.status.lock() {
            *lock = DictationStatus::Transcribing;
        }
        PlatformManager::set_hud_status(DictationStatus::Transcribing);

        let (
            stt_provider,
            groq_api_key,
            groq_model,
            elevenlabs_api_key,
            elevenlabs_model,
            language_code,
            auto_paste,
            store_textbox_contents,
            stt_timeout_secs,
            silence_filter,
        ) = {
            let cfg = state.config.lock().unwrap_or_else(|e| e.into_inner());
            (
                cfg.stt_provider,
                cfg.groq_api_key.clone(),
                cfg.groq_model.clone(),
                cfg.elevenlabs_api_key.clone(),
                cfg.elevenlabs_model.clone(),
                cfg.language_code.clone(),
                cfg.auto_paste,
                cfg.store_textbox_contents,
                cfg.stt_timeout_secs,
                cfg.silence_filter,
            )
        };

        // The noise gate knows whether anything was actually said. If not, the
        // backend is never contacted at all — no upload, no gRPC session, no
        // request, and therefore no chance of a hallucinated transcript.
        let speech_ms = self.recorder.detected_speech_ms();
        if !should_transcribe(silence_filter, speech_ms) {
            info!("Noise gate detected no speech; skipping the transcription entirely.");
            if let Some(handle) = self.response_handle.take() {
                handle.abort();
            }
            self.request_tx = None;
            self.samples
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();

            wflow::sound::play_sound(wflow::sound::SoundEffect::Error);
            state
                .set_toast("ไม่ได้ยินเสียงพูด — ไม่ได้ส่งให้โมเดลประมวลผล (เพิ่มความไวต่อเสียงพูดได้ใน Settings)");
            if let Ok(mut lock) = state.status.lock() {
                *lock = DictationStatus::Idle;
            }
            PlatformManager::set_hud_status(DictationStatus::Idle);
            PlatformManager::hide_floating_hud();
            schedule_hud_reset(state.clone(), 400);
            return;
        }

        let transcription_start = Instant::now();
        let mut result_text: Option<String> = None;

        match stt_provider {
            SttProvider::WisprFlow => {
                if let Some(tx) = self.request_tx.take() {
                    let _ = tx
                        .send(Request {
                            init: None,
                            context: None,
                            payload: None,
                            commit: Commit::True as i32,
                        })
                        .await;
                }

                if let Some(mut handle) = self.response_handle.take() {
                    let mut failed = false;
                    match tokio::time::timeout(TRANSCRIPTION_TIMEOUT, &mut handle).await {
                        Ok(Ok(res)) => result_text = res,
                        Ok(Err(join_err)) => {
                            warn!("Transcription task join error: {}", join_err);
                            failed = true;
                        }
                        Err(_) => {
                            warn!(
                                "Transcription timed out after {}s; aborting the stream.",
                                TRANSCRIPTION_TIMEOUT.as_secs()
                            );
                            handle.abort();
                            state.set_toast("การถอดเสียงใช้เวลานานเกินไป กรุณาลองอีกครั้ง");
                            failed = true;
                        }
                    }

                    if failed {
                        if let Ok(mut lock) = state.status.lock() {
                            *lock = DictationStatus::Error;
                        }
                        PlatformManager::set_hud_status(DictationStatus::Error);
                        schedule_hud_reset(state.clone(), 2500);
                        return;
                    }
                }
            }
            SttProvider::Groq | SttProvider::ElevenLabs => {
                // Give the collector a moment to drain the final audio chunk.
                tokio::time::sleep(Duration::from_millis(120)).await;
                let samples = self
                    .samples
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                info!(
                    "{:?} STT processing {} samples ({:.2}s)",
                    stt_provider,
                    samples.len(),
                    duration
                );

                let (api_key, model, provider) = match stt_provider {
                    SttProvider::Groq => (groq_api_key, groq_model, "Groq"),
                    _ => (elevenlabs_api_key, elevenlabs_model, "ElevenLabs"),
                };

                match api_key {
                    Some(key) if !key.trim().is_empty() => {
                        if samples.len() >= 1600 {
                            match pcm_to_wav(&samples, TARGET_SAMPLE_RATE) {
                                Ok(wav_bytes) => {
                                    let transcription = if provider == "Groq" {
                                        transcribe_groq(
                                            &key,
                                            &model,
                                            &language_code,
                                            wav_bytes,
                                            stt_timeout_secs,
                                        )
                                        .await
                                    } else {
                                        transcribe_elevenlabs(
                                            &key,
                                            &model,
                                            &language_code,
                                            wav_bytes,
                                            stt_timeout_secs,
                                        )
                                        .await
                                    };

                                    match transcription {
                                        Ok(text) => result_text = Some(text),
                                        Err(err) => {
                                            error!("{} STT failed: {}", provider, err);
                                            wflow::sound::play_sound(
                                                wflow::sound::SoundEffect::Error,
                                            );
                                            state
                                                .set_toast(&format!("{} Error: {}", provider, err));
                                        }
                                    }
                                }
                                Err(err) => {
                                    error!("Failed to create WAV byte stream: {}", err);
                                    wflow::sound::play_sound(wflow::sound::SoundEffect::Error);
                                    if let Ok(mut lock) = state.status.lock() {
                                        *lock = DictationStatus::Error;
                                    }
                                    PlatformManager::set_hud_status(DictationStatus::Error);
                                    schedule_hud_reset(state.clone(), 2500);
                                    return;
                                }
                            }
                        } else {
                            info!("Audio too short (< 0.1s) for transcription.");
                            state.set_toast("⚠️ สัญญาณเสียงสั้นเกินไป กรุณากดพูดค้างไว้อย่างน้อย 1 วินาที");
                        }
                    }
                    _ => {
                        warn!("{} API key is not configured.", provider);
                        wflow::sound::play_sound(wflow::sound::SoundEffect::Error);
                        state.set_toast(&format!(
                            "⚠️ {} API key ยังไม่ได้ตั้งค่า (ไปที่แท็บ Settings)",
                            provider
                        ));
                        if let Ok(mut lock) = state.status.lock() {
                            *lock = DictationStatus::Error;
                        }
                        PlatformManager::set_hud_status(DictationStatus::Error);
                        schedule_hud_reset(state.clone(), 2500);
                        return;
                    }
                }
            }
        }

        let latency = transcription_start.elapsed().as_secs_f64();
        let transcribed_text = match result_text {
            Some(t) if !t.trim().is_empty() => t,
            _ => {
                info!(
                    "No speech detected for this session (duration: {:.1}s).",
                    duration
                );
                String::new()
            }
        };

        if transcribed_text.is_empty() {
            info!("The model returned no text for a session with {speech_ms} ms of speech.");
            state.set_toast("ไม่ได้ยินเสียงพูดที่ชัดเจน — ไม่ได้แทรกข้อความ");
            if let Ok(mut lock) = state.status.lock() {
                *lock = DictationStatus::Done;
            }
            PlatformManager::set_hud_status(DictationStatus::Done);
            schedule_hud_reset(state.clone(), 800);
            return;
        }

        let (
            ai_formatting,
            writing_style,
            groq_key,
            llm_timeout_secs,
            hallucination_guard,
            autocorrect_enabled,
            autocorrect_use_llm,
            autocorrect_languages,
            mixed_spacing,
        ) = {
            let cfg = state.config.lock().unwrap_or_else(|e| e.into_inner());
            (
                cfg.ai_formatting,
                cfg.writing_style.clone(),
                cfg.groq_api_key.clone(),
                cfg.stt_timeout_secs.clamp(3, 60),
                cfg.hallucination_guard,
                cfg.autocorrect_enabled,
                cfg.autocorrect_use_llm,
                cfg.autocorrect_languages
                    .split(',')
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<String>>(),
                cfg.mixed_spacing,
            )
        };

        // 0. Guard against model hallucinations ("Thank you.", "(music)", a
        //    word repeated 50 times). Such output must never be pasted.
        if hallucination_guard {
            let expected = wflow::quality::expected_languages(&language_code);
            if let Some(reason) = wflow::quality::check_transcript(&transcribed_text, expected) {
                info!(
                    "Rejected transcript ({}): {:?}",
                    reason.message(),
                    transcribed_text
                );
                wflow::sound::play_sound(wflow::sound::SoundEffect::Error);
                state.set_toast(&format!("🚫 {} — ไม่ได้แทรกข้อความลงในแอป", reason.message()));
                if let Ok(mut lock) = state.status.lock() {
                    *lock = DictationStatus::Error;
                }
                PlatformManager::set_hud_status(DictationStatus::Error);
                schedule_hud_reset(state.clone(), 2500);
                return;
            }
        }

        // 1. Custom snippets (trigger phrase -> replacement)
        let mut final_text = transcribed_text.clone();
        if let Ok(snippets) = db.get_snippets() {
            for (trigger, replacement) in snippets {
                if !trigger.trim().is_empty() && !replacement.trim().is_empty() {
                    final_text = replace_case_insensitive(&final_text, &trigger, &replacement);
                }
            }
        }

        // 2. Offline spelling correction + mixed Thai/Latin spacing
        if autocorrect_enabled {
            final_text = wflow::spell::autocorrect_offline(
                &final_text,
                &autocorrect_languages,
                mixed_spacing,
            );
        } else if mixed_spacing {
            final_text = wflow::lang::collapse_duplicate_spaces(
                &wflow::lang::normalize_mixed_spacing(&final_text),
            );
        }

        // 3. Optional LLM pass: spelling correction and/or style formatting.
        let llm_languages = wflow::spell::target_languages(&final_text, &autocorrect_languages);
        let want_llm = (ai_formatting && writing_style != "raw")
            || (autocorrect_use_llm && autocorrect_enabled);
        if want_llm && !final_text.is_empty() {
            if let Some(ref key) = groq_key {
                if !key.trim().is_empty() {
                    let options = wflow::stt::PostProcess {
                        style: if ai_formatting {
                            writing_style.clone()
                        } else {
                            "raw".to_string()
                        },
                        languages: llm_languages.clone(),
                        fix_spelling: autocorrect_enabled && autocorrect_use_llm,
                        fix_mixed_spacing: mixed_spacing && autocorrect_use_llm,
                    };
                    match wflow::stt::post_process_with_llm(
                        key,
                        &final_text,
                        &options,
                        llm_timeout_secs,
                    )
                    .await
                    {
                        Ok(cleaned) if !cleaned.trim().is_empty() => {
                            // Keep the original if the model drifted away from it.
                            match wflow::quality::check_llm_result(&final_text, &cleaned) {
                                wflow::quality::LlmSanity::Good => final_text = cleaned,
                                wflow::quality::LlmSanity::Drifted => {
                                    warn!("LLM result drifted from the original; keeping it");
                                    state.set_toast("โมเดลแก้ข้อความคลาดเคลื่อน จึงใช้ข้อความต้นฉบับ");
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(err) => warn!("LLM post-processing error: {}", err),
                    }
                }
            }
        }

        wflow::sound::play_sound(wflow::sound::SoundEffect::FinishRecording);

        if let Ok(mut lock) = state.last_transcript.lock() {
            *lock = final_text.clone();
        }
        if let Ok(mut lock) = state.last_latency.lock() {
            *lock = latency;
        }
        if let Ok(mut lock) = state.status.lock() {
            *lock = DictationStatus::Pasting;
        }
        PlatformManager::set_hud_status(DictationStatus::Done);

        if auto_paste {
            if let Err(e) = PlatformManager::paste_text(&final_text) {
                error!("Failed to paste text: {}", e);
                state.set_toast(&format!("วางข้อความไม่สำเร็จ: {}", e));
            }
        }

        let textbox_contents = if store_textbox_contents {
            self.ctx.as_ref().and_then(|c| c.before_text.clone())
        } else {
            None
        };

        let history_entry = HistoryRecord {
            transcript_id: uuid::Uuid::new_v4().to_string(),
            asr_text: transcribed_text,
            formatted_text: final_text.clone(),
            app: self
                .ctx
                .as_ref()
                .map(|c| c.process_name.clone())
                .unwrap_or_default(),
            textbox_contents,
            duration_secs: duration,
            e2e_latency_secs: latency,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(err) = db.save_history(&history_entry) {
            warn!("Could not save history entry: {}", err);
        }

        if let Ok(records) = db.get_recent_history(50) {
            if let Ok(mut lock) = state.history.lock() {
                *lock = records;
            }
        }

        if let Ok(mut lock) = state.status.lock() {
            *lock = DictationStatus::Done;
        }
        schedule_hud_reset(state.clone(), 1500);
    }
}

/// Parses a "major.minor.patch" string, falling back to the protocol default.
fn parse_client_version(raw: &str) -> Version {
    let mut parts = raw
        .trim()
        .split('.')
        .map(|p| p.trim().parse::<u32>().unwrap_or(0));
    Version {
        major: parts.next().unwrap_or(1),
        minor: parts.next().unwrap_or(6),
        patch: parts.next().unwrap_or(1147),
    }
}

/// The platform announced to the gRPC backend.
fn client_platform() -> Platform {
    #[cfg(target_os = "windows")]
    {
        Platform::Win32
    }
    #[cfg(target_os = "macos")]
    {
        Platform::Macos
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Platform::Api
    }
}

/// Decides whether a recording has to be sent to the speech-to-text backend.
///
/// With the silence filter enabled a recording in which the gate never
/// detected any speech is dropped locally: no upload, no gRPC session and no
/// request — which also means the model cannot hallucinate on silence.
fn should_transcribe(silence_filter: bool, speech_ms: u64) -> bool {
    !(silence_filter && speech_ms == 0)
}

fn schedule_hud_reset(state: AppState, delay_ms: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if let Ok(mut lock) = state.status.lock() {
            if *lock == DictationStatus::Done {
                *lock = DictationStatus::Idle;
            }
        }
        PlatformManager::hide_floating_hud();
        if let Ok(lock) = state.egui_ctx.lock() {
            if let Some(ctx) = lock.as_ref() {
                ctx.request_repaint();
            }
        }
    });
}

async fn run_background_coordinator(
    state: AppState,
    db: Arc<Database>,
    hotkey_rx: &mut mpsc::UnboundedReceiver<HotkeyEvent>,
    gui_rx: &mut mpsc::UnboundedReceiver<GuiEvent>,
) {
    let mut session = DictationSession::new();
    let mut last_update_probe = Duration::ZERO;

    loop {
        tokio::select! {
            event = hotkey_rx.recv() => {
                match event {
                    Some(HotkeyEvent::Pressed) => {
                        let mode = state.config.lock().map(|c| c.recording_trigger_mode.clone()).unwrap_or_else(|_| "push_to_talk".to_string());
                        if mode == "tap_to_toggle" && session.is_active() {
                            session.finish(&state, &db).await;
                        } else {
                            session.start(&state, &db).await;
                        }
                    }
                    Some(HotkeyEvent::Released) => {
                        let mode = state.config.lock().map(|c| c.recording_trigger_mode.clone()).unwrap_or_else(|_| "push_to_talk".to_string());
                        if mode != "tap_to_toggle" {
                            session.finish(&state, &db).await;
                        }
                    }
                    Some(HotkeyEvent::ToggleWindow) => {
                        info!("Received ToggleWindow event from hotkey or tray.");
                        #[cfg(target_os = "windows")]
                        PlatformManager::toggle_main_window();
                        #[cfg(not(target_os = "windows"))]
                        {
                            let curr = state.window_visible.load(Ordering::SeqCst);
                            state.window_visible.store(!curr, Ordering::SeqCst);
                            if let Ok(lock) = state.egui_ctx.lock() {
                                if let Some(ctx) = lock.as_ref() {
                                    if curr {
                                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(true));
                                    } else {
                                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(false));
                                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
                                    }
                                    ctx.request_repaint();
                                }
                            }
                            if !curr {
                                PlatformManager::show_main_window();
                            } else {
                                PlatformManager::hide_main_window();
                            }
                        }
                    }
                    Some(HotkeyEvent::KeyRecorded(mode, combo)) => {
                        info!("Key recorded: mode={}, combo={}", mode, combo.format_display());
                        if let Ok(mut lock) = state.recorded_key.lock() {
                            *lock = Some((mode, combo));
                        }
                        request_repaint(&state);
                    }
                    Some(HotkeyEvent::AutostartToggled(enabled)) => {
                        info!("Auto-start toggled from tray menu: {}", enabled);
                        if let Ok(mut lock) = state.config.lock() {
                            lock.auto_start = enabled;
                            let _ = lock.save(None);
                        }
                        state.set_toast(if enabled {
                            "Auto-start enabled (Wflow will start with the system)"
                        } else {
                            "Auto-start disabled"
                        });
                        request_repaint(&state);
                    }
                    Some(HotkeyEvent::CheckUpdate) => {
                        info!("Update check requested from the tray menu.");
                        let include_prerelease = state
                            .config
                            .lock()
                            .map(|c| c.include_prerelease)
                            .unwrap_or(false);
                        spawn_update_check(state.clone(), include_prerelease);
                        state.set_toast("กำลังตรวจสอบอัปเดต...");
                        request_repaint(&state);
                    }
                    Some(HotkeyEvent::QuitApp) => {
                        info!("Quit requested from tray/menu; shutting down gracefully.");
                        state.pending_quit.store(true, Ordering::SeqCst);
                        request_repaint(&state);
                    }
                    None => break,
                }
            }

            gui_event = gui_rx.recv() => {
                match gui_event {
                    Some(GuiEvent::StartDictation) => {
                        session.start(&state, &db).await;
                    }
                    Some(GuiEvent::StopDictation) => {
                        session.finish(&state, &db).await;
                    }
                    Some(GuiEvent::RefreshHistory) => {
                        refresh_history(&state, &db);
                    }
                    Some(GuiEvent::ClearHistory) => {
                        match db.clear_history() {
                            Ok(removed) => {
                                state.set_toast(&format!("ล้างประวัติแล้ว {} รายการ", removed));
                                refresh_history(&state, &db);
                            }
                            Err(err) => {
                                warn!("Could not clear history: {}", err);
                                state.set_toast(&format!("ล้างประวัติไม่สำเร็จ: {}", err));
                            }
                        }
                    }
                    Some(GuiEvent::DeleteHistory(id)) => {
                        if let Err(err) = db.delete_history(&id) {
                            warn!("Could not delete history row: {}", err);
                        }
                        refresh_history(&state, &db);
                    }
                    Some(GuiEvent::AddDictionaryEntry { word, replacement }) => {
                        if let Err(err) = db.add_dictionary_entry(&word, replacement.as_deref()) {
                            warn!("Could not add dictionary entry: {}", err);
                        }
                        refresh_dictionary(&state, &db);
                    }
                    Some(GuiEvent::DeleteDictionaryEntry(id)) => {
                        if let Err(err) = db.delete_dictionary_entry(&id) {
                            warn!("Could not delete dictionary entry: {}", err);
                        }
                        refresh_dictionary(&state, &db);
                    }
                    Some(GuiEvent::RefreshDictionary) => {
                        refresh_dictionary(&state, &db);
                    }
                    Some(GuiEvent::SaveConfig(new_cfg)) => {
                        let new_cfg = *new_cfg;
                        if let Err(err) = new_cfg.save(None) {
                            warn!("Could not save settings: {}", err);
                            state.set_toast(&format!("บันทึกการตั้งค่าไม่สำเร็จ: {}", err));
                        }
                        PlatformManager::update_hotkeys(new_cfg.hotkey, new_cfg.toggle_window);
                        if let Err(err) = PlatformManager::set_autostart(new_cfg.auto_start) {
                            warn!("Could not update autostart: {}", err);
                        }
                        wflow::sound::set_sound_enabled(new_cfg.sound_effects);
                        if let Ok(mut lock) = state.config.lock() {
                            *lock = new_cfg;
                        }
                    }
                    Some(GuiEvent::CheckUpdate) => {
                        let include_prerelease = state
                            .config
                            .lock()
                            .map(|c| c.include_prerelease)
                            .unwrap_or(false);
                        spawn_update_check(state.clone(), include_prerelease);
                        state.set_toast("กำลังตรวจสอบอัปเดต...");
                        request_repaint(&state);
                    }
                    Some(GuiEvent::SkipUpdateVersion(version)) => {
                        if let Ok(mut cfg) = state.config.lock() {
                            cfg.skipped_update_version = Some(version);
                            if let Err(err) = cfg.save(None) {
                                warn!("Could not save skipped update: {}", err);
                            }
                        }
                        request_repaint(&state);
                    }
                    Some(GuiEvent::OpenUrl(url)) => {
                        if let Err(err) = PlatformManager::open_url(&url) {
                            warn!("Could not open URL: {}", err);
                            state.set_toast(&format!("เปิดลิงก์ไม่สำเร็จ: {}", err));
                        }
                    }
                    Some(GuiEvent::OpenLogFolder) => {
                        let dir = wflow::paths::ensure_dir(wflow::paths::log_dir());
                        if let Err(err) = PlatformManager::open_url(&dir.display().to_string()) {
                            warn!("Could not open log folder: {}", err);
                        }
                    }
                    None => break,
                }
            }

            _ = tokio::time::sleep(TICK_INTERVAL) => {
                // Re-check for updates periodically so a long-running session
                // still learns about new releases.
                last_update_probe += TICK_INTERVAL;
                if last_update_probe >= Duration::from_secs(15 * 60) {
                    last_update_probe = Duration::ZERO;
                    let (enabled, interval, prerelease, last_check) = {
                        let cfg = state.config.lock().unwrap_or_else(|e| e.into_inner());
                        (
                            cfg.check_updates,
                            cfg.update_check_interval_hours,
                            cfg.include_prerelease,
                            cfg.last_update_check.clone(),
                        )
                    };
                    if enabled && wflow::update::should_check(last_check.as_deref(), interval) {
                        spawn_update_check(state.clone(), prerelease);
                    }
                }

                if session.is_active() {
                    let amp = session.recorder.get_amplitude();
                    if let Ok(mut lock) = state.current_amplitude.lock() {
                        *lock = amp;
                    }
                    PlatformManager::update_hud_amplitude(amp);

                    // The noise gate can ask us to stop when the speaker went
                    // quiet for a long time.
                    if session.recorder.stop_requested() {
                        warn!("Stopping after a long silence.");
                        state.set_toast("หยุดอัตโนมัติ เพราะไม่มีเสียงพูด");
                        session.finish(&state, &db).await;
                    }

                    let max_secs = state
                        .config
                        .lock()
                        .map(|c| c.max_recording_secs)
                        .unwrap_or(300);
                    if let Some(start) = session.start {
                        if start.elapsed().as_secs() >= max_secs {
                            warn!("Maximum recording length ({}s) reached; stopping.", max_secs);
                            state.set_toast("ถึงเวลาบันทึกสูงสุดแล้ว ระบบหยุดให้อัตโนมัติ");
                            session.finish(&state, &db).await;
                        }
                    }
                }
            }
        }
    }
}

fn request_repaint(state: &AppState) {
    if let Ok(lock) = state.egui_ctx.lock() {
        if let Some(ctx) = lock.as_ref() {
            ctx.request_repaint();
        }
    }
}

fn refresh_history(state: &AppState, db: &Arc<Database>) {
    match db.get_recent_history(50) {
        Ok(records) => {
            if let Ok(mut lock) = state.history.lock() {
                *lock = records;
            }
        }
        Err(err) => warn!("Could not refresh history: {}", err),
    }
}

fn refresh_dictionary(state: &AppState, db: &Arc<Database>) {
    match db.get_all_dictionary_entries() {
        Ok(entries) => {
            if let Ok(mut lock) = state.dictionary_entries.lock() {
                *lock = entries;
            }
        }
        Err(err) => warn!("Could not refresh dictionary: {}", err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_sent_when_the_gate_heard_no_speech() {
        // With the filter on and no detected speech nothing reaches the
        // backend, so the model can never hallucinate on silence.
        assert!(!should_transcribe(true, 0));
        // Any detected speech is transcribed.
        assert!(should_transcribe(true, 120));
        // With the filter off everything is sent.
        assert!(should_transcribe(false, 0));
    }
}
