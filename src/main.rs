mod audio;
mod config;
mod db;
mod grpc;
mod gui;
mod platform;
mod proto;
mod stt;
mod ui;

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use audio::{AudioRecorder, TARGET_SAMPLE_RATE};
use config::AppConfig;
use db::{Database, HistoryRecord};
use grpc::FlowGrpcClient;
use gui::{AppState, GuiEvent, WisprFlowGui};
use platform::{ActiveAppContext, HotkeyEvent, PlatformManager};
use proto::*;
use stt::{pcm_to_wav, transcribe_elevenlabs, transcribe_groq, SttProvider};
use ui::DictationStatus;

fn main() -> Result<()> {
    // 1. Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("Starting Wispr Flow (Rust Native Edition)...");

    // 2. Load configuration & database
    let config = AppConfig::load_or_default(None);
    let db = Arc::new(Database::open_or_default(None)?);
    let initial_history = db.get_recent_history(50).unwrap_or_default();

    // 3. Create shared thread-safe AppState
    let app_state = AppState::new(config.clone(), initial_history);
    let app_state_worker = app_state.clone();
    let app_state_gui = app_state.clone();

    // 4. Set up communication channels
    let (gui_tx, mut gui_rx) = mpsc::unbounded_channel::<GuiEvent>();
    let (hotkey_tx, mut hotkey_rx) = mpsc::unbounded_channel::<HotkeyEvent>();

    // 5. Start global Win32 low-level keyboard hook
    let _hook_thread = PlatformManager::start_hotkey_listener(
        hotkey_tx,
        config.hotkey_vk,
        config.toggle_window_vk,
    )?;

    // 6. Spawn background Tokio multi-thread runtime for audio, gRPC & DB
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
            run_background_coordinator(
                app_state_worker,
                db_worker,
                &mut hotkey_rx,
                &mut gui_rx,
            )
            .await;
        });
    });

    // 7. Launch modern Native GUI on main OS thread
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([920.0, 640.0])
        .with_min_inner_size([760.0, 500.0])
        .with_title("Wispr Flow (Rust Edition) - AI Voice Studio")
        .with_active(true);

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    info!("Launching Native GUI window...");
    eframe::run_native(
        "Wispr Flow",
        native_options,
        Box::new(move |_cc| Ok(Box::new(WisprFlowGui::new(app_state_gui, gui_tx)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
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

async fn run_background_coordinator(
    state: AppState,
    db: Arc<Database>,
    hotkey_rx: &mut mpsc::UnboundedReceiver<HotkeyEvent>,
    gui_rx: &mut mpsc::UnboundedReceiver<GuiEvent>,
) {
    let mut recorder = AudioRecorder::new();
    let is_dictating = Arc::new(AtomicBool::new(false));

    let mut session_start: Option<Instant> = None;
    let mut session_ctx: Option<ActiveAppContext> = None;
    let mut session_request_tx: Option<mpsc::Sender<Request>> = None;
    let mut session_response_handle: Option<tokio::task::JoinHandle<Option<String>>> = None;
    let session_audio_samples = Arc::new(Mutex::new(Vec::<i16>::new()));

    let dictionary_words = db.get_dictionary_words().unwrap_or_default();

    loop {
        tokio::select! {
            // Hotkey events (from global keyboard hook)
            event = hotkey_rx.recv() => {
                match event {
                    Some(HotkeyEvent::Pressed) => {
                        start_dictation_session(
                            &state,
                            &mut recorder,
                            &is_dictating,
                            &mut session_start,
                            &mut session_ctx,
                            &mut session_request_tx,
                            &mut session_response_handle,
                            &session_audio_samples,
                            &dictionary_words,
                        ).await;
                    }
                    Some(HotkeyEvent::Released) => {
                        finish_dictation_session(
                            &state,
                            &db,
                            &mut recorder,
                            &is_dictating,
                            &mut session_start,
                            &mut session_ctx,
                            &mut session_request_tx,
                            &mut session_response_handle,
                            &session_audio_samples,
                        ).await;
                    }
                    Some(HotkeyEvent::ToggleWindow) => {
                        info!("Received ToggleWindow event from Hotkey or Tray Icon.");
                        #[cfg(target_os = "windows")]
                        PlatformManager::toggle_main_window();
                    }
                    Some(HotkeyEvent::KeyRecorded(mode, vk)) => {
                        info!("Key recorded: mode={}, vk=0x{:02X}", mode, vk);
                        if let Ok(mut lock) = state.recorded_key.lock() {
                            *lock = Some((mode, vk));
                        }
                        if let Ok(lock) = state.egui_ctx.lock() {
                            if let Some(ctx) = lock.as_ref() {
                                ctx.request_repaint();
                            }
                        }
                    }
                    Some(HotkeyEvent::QuitApp) => {
                        info!("Received QuitApp event from Tray menu.");
                        state.pending_quit.store(true, Ordering::SeqCst);
                        if let Ok(lock) = state.egui_ctx.lock() {
                            if let Some(ctx) = lock.as_ref() {
                                ctx.request_repaint();
                            }
                        }
                        break;
                    }
                    None => break,
                }
            }

            // GUI events (from user clicking in egui)
            gui_event = gui_rx.recv() => {
                match gui_event {
                    Some(GuiEvent::StartDictation) => {
                        start_dictation_session(
                            &state,
                            &mut recorder,
                            &is_dictating,
                            &mut session_start,
                            &mut session_ctx,
                            &mut session_request_tx,
                            &mut session_response_handle,
                            &session_audio_samples,
                            &dictionary_words,
                        ).await;
                    }
                    Some(GuiEvent::StopDictation) => {
                        finish_dictation_session(
                            &state,
                            &db,
                            &mut recorder,
                            &is_dictating,
                            &mut session_start,
                            &mut session_ctx,
                            &mut session_request_tx,
                            &mut session_response_handle,
                            &session_audio_samples,
                        ).await;
                    }
                    Some(GuiEvent::RefreshHistory) => {
                        if let Ok(records) = db.get_recent_history(50) {
                            if let Ok(mut lock) = state.history.lock() {
                                *lock = records;
                            }
                        }
                    }
                    Some(GuiEvent::DeleteHistory(id)) => {
                        let _ = db.delete_history(&id);
                        if let Ok(records) = db.get_recent_history(50) {
                            if let Ok(mut lock) = state.history.lock() {
                                *lock = records;
                            }
                        }
                    }
                    Some(GuiEvent::SaveConfig(new_cfg)) => {
                        let _ = new_cfg.save(None);
                        PlatformManager::update_hotkeys(new_cfg.hotkey_vk, new_cfg.toggle_window_vk);
                        if let Ok(mut lock) = state.config.lock() {
                            *lock = new_cfg;
                        }
                    }
                    None => break,
                }
            }

            // Periodic amplitude meter tick & status reset timer
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(30)) => {
                if is_dictating.load(Ordering::SeqCst) {
                    let amp = recorder.get_amplitude();
                    if let Ok(mut lock) = state.current_amplitude.lock() {
                        *lock = amp;
                    }
                    PlatformManager::update_hud_amplitude(amp);
                }
            }
        }
    }
}

async fn start_dictation_session(
    state: &AppState,
    recorder: &mut AudioRecorder,
    is_dictating: &Arc<AtomicBool>,
    session_start: &mut Option<Instant>,
    session_ctx: &mut Option<ActiveAppContext>,
    session_request_tx: &mut Option<mpsc::Sender<Request>>,
    session_response_handle: &mut Option<tokio::task::JoinHandle<Option<String>>>,
    session_audio_samples: &Arc<Mutex<Vec<i16>>>,
    dictionary_words: &[String],
) {
    if is_dictating.load(Ordering::SeqCst) {
        return;
    }
    is_dictating.store(true, Ordering::SeqCst);
    state.is_recording.store(true, Ordering::SeqCst);
    *session_start = Some(Instant::now());

    // Reset audio sample buffer for new session
    if let Ok(mut samples) = session_audio_samples.lock() {
        samples.clear();
    }

    if let Ok(mut lock) = state.status.lock() {
        *lock = DictationStatus::Listening;
    }

    let (endpoint_url, user_id, language_code, _style, auth_token, stt_provider, show_floating_hud) = {
        let cfg = state.config.lock().unwrap();
        (
            cfg.endpoint_url.clone(),
            cfg.user_id.clone(),
            cfg.language_code.clone(),
            cfg.writing_style.clone(),
            cfg.auth_token.clone(),
            cfg.stt_provider,
            cfg.show_floating_hud,
        )
    };

    // Show floating HUD overlay pill & waveform if enabled
    if show_floating_hud {
        PlatformManager::show_floating_hud(stt_provider.short_badge());
        PlatformManager::set_hud_status(DictationStatus::Listening);
    } else {
        PlatformManager::set_tray_recording_state(true);
    }

    // Capture focused window & textbox context
    let app_ctx = PlatformManager::get_active_app_context();
    info!(
        "Active target context: app='{}', title='{}', before_chars={}",
        app_ctx.process_name,
        app_ctx.window_title,
        app_ctx.before_text.as_ref().map(|s| s.len()).unwrap_or(0)
    );
    if let Ok(mut lock) = state.active_app.lock() {
        *lock = app_ctx.clone();
    }
    *session_ctx = Some(app_ctx.clone());

    // Audio stream channel from cpal
    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(100);
    if let Err(e) = recorder.start_recording(audio_tx) {
        error!("Microphone start failed: {}", e);
        if let Ok(mut lock) = state.status.lock() {
            *lock = DictationStatus::Error;
        }
        is_dictating.store(false, Ordering::SeqCst);
        state.is_recording.store(false, Ordering::SeqCst);
        if show_floating_hud {
            PlatformManager::hide_floating_hud();
        }
        return;
    }

    match stt_provider {
        SttProvider::WisprFlow => {
            // gRPC request stream channel
            let (grpc_req_tx, grpc_req_rx) = mpsc::channel::<Request>(100);
            *session_request_tx = Some(grpc_req_tx.clone());

            let session_id = uuid::Uuid::new_v4().to_string();
            let request_id = uuid::Uuid::new_v4().to_string();

            let lang_id = match language_code.to_lowercase().as_str() {
                "th" | "thai" => 34,
                "en" | "en-us" | "english" => 1,
                "zh" | "chinese" => 3,
                "ja" | "japanese" => 10,
                "auto" | "" => 0,
                _ => 0,
            };
            let language_vec = if lang_id == 0 { vec![0] } else { vec![lang_id] };

            // 1. Send Init message
            let init_req = Request {
                init: Some(Init {
                    metadata: Some(Metadata {
                        user_id: user_id.clone(),
                        session_id: session_id.clone(),
                        request_id: request_id.clone(),
                        audio_encoding: AudioEncoding::Wav as i32,
                        environment: Environment::Production as i32,
                        client: Some(Client {
                            name: "Wispr Flow".to_string(),
                            platform: Platform::Win32 as i32,
                            version: Some(Version { major: 1, minor: 6, patch: 1147 }),
                        }),
                        debug: false,
                        referral_code: String::new(),
                        always_wait_scribe: false,
                    }),
                    preferences: Some(Preferences {
                        user: None,
                        language: language_vec,
                        vocabulary: Some(StaticVocabulary {
                            dictionary_personal: dictionary_words.to_vec(),
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
            let _ = grpc_req_tx.send(init_req).await;

            // 2. Send Context message
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
                        contents: app_ctx.full_text.clone(),
                        before_text: app_ctx.before_text.clone(),
                        selected_text: app_ctx.selected_text.clone(),
                        after_text: app_ctx.after_text.clone(),
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
            let _ = grpc_req_tx.send(context_req).await;

            // 3. Spawn audio streaming bridge task
            let bridge_tx = grpc_req_tx.clone();
            let is_dictating_clone = Arc::clone(is_dictating);
            let samples_clone = Arc::clone(session_audio_samples);
            tokio::spawn(async move {
                while is_dictating_clone.load(Ordering::SeqCst) {
                    if let Some(chunk) = audio_rx.recv().await {
                        // Store PCM samples
                        if let Ok(mut buf) = samples_clone.lock() {
                            for p in chunk.chunks_exact(2) {
                                buf.push(i16::from_le_bytes([p[0], p[1]]));
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
                    } else {
                        break;
                    }
                }
            });

            // 4. Spawn gRPC streaming client task
            let resp_handle = tokio::spawn(async move {
                match FlowGrpcClient::connect(&endpoint_url).await {
                    Ok(mut client) => {
                        let stream = tokio_stream::wrappers::ReceiverStream::new(grpc_req_rx);
                        match client.transcribe_stream(stream, auth_token.as_deref()).await {
                            Ok(mut response_stream) => {
                                let mut final_text = None;
                                while let Ok(Some(resp)) = response_stream.message().await {
                                    if let Some(res) = resp.result {
                                        if let Some(out) = res.output {
                                            if !out.plaintext.trim().is_empty() {
                                                info!("Received final speech transcript: '{}'", out.plaintext);
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
                                warn!("gRPC Stream returned error: {}", e);
                                None
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Could not connect to live backend ({}). Local demo mode active.", e);
                        None
                    }
                }
            });

            *session_response_handle = Some(resp_handle);
        }
        SttProvider::Groq | SttProvider::ElevenLabs => {
            // No gRPC streaming channel needed for HTTP REST providers
            *session_request_tx = None;
            *session_response_handle = None;

            // Spawn audio receiver task to accumulate samples
            let is_dictating_clone = Arc::clone(is_dictating);
            let samples_clone = Arc::clone(session_audio_samples);
            tokio::spawn(async move {
                while is_dictating_clone.load(Ordering::SeqCst) {
                    if let Some(chunk) = audio_rx.recv().await {
                        if let Ok(mut buf) = samples_clone.lock() {
                            for p in chunk.chunks_exact(2) {
                                buf.push(i16::from_le_bytes([p[0], p[1]]));
                            }
                        }
                    } else {
                        break;
                    }
                }
            });
        }
    }
}

async fn finish_dictation_session(
    state: &AppState,
    db: &Arc<Database>,
    recorder: &mut AudioRecorder,
    is_dictating: &Arc<AtomicBool>,
    session_start: &mut Option<Instant>,
    session_ctx: &mut Option<ActiveAppContext>,
    session_request_tx: &mut Option<mpsc::Sender<Request>>,
    session_response_handle: &mut Option<tokio::task::JoinHandle<Option<String>>>,
    session_audio_samples: &Arc<Mutex<Vec<i16>>>,
) {
    if !is_dictating.load(Ordering::SeqCst) {
        return;
    }
    is_dictating.store(false, Ordering::SeqCst);
    state.is_recording.store(false, Ordering::SeqCst);
    let duration = session_start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);

    // Stop audio recording
    recorder.stop_recording();

    if let Ok(mut lock) = state.status.lock() {
        *lock = DictationStatus::Transcribing;
    }
    PlatformManager::set_hud_status(DictationStatus::Transcribing);

    let (stt_provider, groq_api_key, groq_model, elevenlabs_api_key, elevenlabs_model, language_code, auto_paste) = {
        let cfg = state.config.lock().unwrap();
        (
            cfg.stt_provider,
            cfg.groq_api_key.clone(),
            cfg.groq_model.clone(),
            cfg.elevenlabs_api_key.clone(),
            cfg.elevenlabs_model.clone(),
            cfg.language_code.clone(),
            cfg.auto_paste,
        )
    };

    let transcription_start = Instant::now();
    let mut result_text: Option<String> = None;

    match stt_provider {
        SttProvider::WisprFlow => {
            // Commit stream
            if let Some(tx) = session_request_tx.take() {
                let commit_req = Request {
                    init: None,
                    context: None,
                    payload: None,
                    commit: Commit::True as i32,
                };
                let _ = tx.send(commit_req).await;
            }

            // Wait for response with timeout to ensure the event loop never locks
            if let Some(handle) = session_response_handle.take() {
                match tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await {
                    Ok(Ok(res)) => {
                        result_text = res;
                    }
                    Ok(Err(join_err)) => {
                        warn!("Wispr transcription task join error: {}", join_err);
                    }
                    Err(_) => {
                        warn!("Wispr transcription timed out after 5s waiting for gRPC response.");
                    }
                }
            }
        }
        SttProvider::Groq => {
            let samples = session_audio_samples.lock().unwrap().clone();
            info!("Groq STT processing {} audio samples ({:.2}s)", samples.len(), duration);

            match groq_api_key {
                Some(key) if !key.trim().is_empty() => {
                    if samples.len() >= 3200 { // at least 0.2s of audio
                        match pcm_to_wav(&samples, TARGET_SAMPLE_RATE) {
                            Ok(wav_bytes) => {
                                match transcribe_groq(&key, &groq_model, &language_code, wav_bytes).await {
                                    Ok(text) => {
                                        result_text = Some(text);
                                    }
                                    Err(err) => {
                                        error!("Groq STT transcription failed: {}", err);
                                        state.set_toast(&format!("Groq Error: {}", err));
                                    }
                                }
                            }
                            Err(err) => {
                                error!("Failed to create WAV byte stream: {}", err);
                            }
                        }
                    } else {
                        info!("Audio too short (< 0.2s) for Groq transcription.");
                    }
                }
                _ => {
                    warn!("Groq API key is not configured.");
                    state.set_toast("Groq API key is missing. Set it in Settings tab.");
                }
            }
        }
        SttProvider::ElevenLabs => {
            let samples = session_audio_samples.lock().unwrap().clone();
            info!("ElevenLabs STT processing {} audio samples ({:.2}s)", samples.len(), duration);

            match elevenlabs_api_key {
                Some(key) if !key.trim().is_empty() => {
                    if samples.len() >= 3200 { // at least 0.2s of audio
                        match pcm_to_wav(&samples, TARGET_SAMPLE_RATE) {
                            Ok(wav_bytes) => {
                                match transcribe_elevenlabs(&key, &elevenlabs_model, &language_code, wav_bytes).await {
                                    Ok(text) => {
                                        result_text = Some(text);
                                    }
                                    Err(err) => {
                                        error!("ElevenLabs STT transcription failed: {}", err);
                                        state.set_toast(&format!("ElevenLabs Error: {}", err));
                                    }
                                }
                            }
                            Err(err) => {
                                error!("Failed to create WAV byte stream: {}", err);
                            }
                        }
                    } else {
                        info!("Audio too short (< 0.2s) for ElevenLabs transcription.");
                    }
                }
                _ => {
                    warn!("ElevenLabs API key is not configured.");
                    state.set_toast("ElevenLabs API key is missing. Set it in Settings tab.");
                }
            }
        }
    }

    let latency = transcription_start.elapsed().as_secs_f64();

    let transcribed_text = match result_text {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            info!("No speech detected for this session (duration: {:.1}s).", duration);
            String::new()
        }
    };

    if transcribed_text.is_empty() {
        if let Ok(mut lock) = state.status.lock() {
            *lock = DictationStatus::Done;
        }
        PlatformManager::set_hud_status(DictationStatus::Done);

        let state_clone = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
            if let Ok(mut lock) = state_clone.status.lock() {
                if *lock == DictationStatus::Done {
                    *lock = DictationStatus::Idle;
                }
            }
            PlatformManager::hide_floating_hud();
        });
        return;
    }

    if let Ok(mut lock) = state.last_transcript.lock() {
        *lock = transcribed_text.clone();
    }
    if let Ok(mut lock) = state.last_latency.lock() {
        *lock = latency;
    }

    if let Ok(mut lock) = state.status.lock() {
        *lock = DictationStatus::Pasting;
    }
    PlatformManager::set_hud_status(DictationStatus::Done);

    // Auto paste if enabled
    if auto_paste {
        if let Err(e) = PlatformManager::paste_text(&transcribed_text) {
            error!("Failed to paste text: {}", e);
        }
    }

    // Save to SQLite
    let history_entry = HistoryRecord {
        transcript_id: uuid::Uuid::new_v4().to_string(),
        asr_text: transcribed_text.clone(),
        formatted_text: transcribed_text.clone(),
        app: session_ctx.as_ref().map(|c| c.process_name.clone()).unwrap_or_default(),
        textbox_contents: session_ctx.as_ref().and_then(|c| c.before_text.clone()),
        duration_secs: duration,
        e2e_latency_secs: latency,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    let _ = db.save_history(&history_entry);

    // Refresh history in GUI state
    if let Ok(records) = db.get_recent_history(50) {
        if let Ok(mut lock) = state.history.lock() {
            *lock = records;
        }
    }

    if let Ok(mut lock) = state.status.lock() {
        *lock = DictationStatus::Done;
    }

    // Reset status and hide floating HUD after brief moment
    let state_clone = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        if let Ok(mut lock) = state_clone.status.lock() {
            if *lock == DictationStatus::Done {
                *lock = DictationStatus::Idle;
            }
        }
        PlatformManager::hide_floating_hud();
    });
}
