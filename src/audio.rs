use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub const TARGET_SAMPLE_RATE: u32 = 16_000;
pub const CHUNK_SAMPLES: usize = 1600; // 100ms at 16kHz

pub struct AudioRecorder {
    is_recording: Arc<AtomicBool>,
    active_stream: Option<cpal::Stream>,
    current_amplitude: Arc<Mutex<f32>>,
}

/// Enumerates all available audio input devices on the system
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    if let Ok(devices) = host.input_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                names.push(name);
            }
        }
    }
    names
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            is_recording: Arc::new(AtomicBool::new(false)),
            active_stream: None,
            current_amplitude: Arc::new(Mutex::new(0.0)),
        }
    }

    #[allow(dead_code)]
    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }

    pub fn get_amplitude(&self) -> f32 {
        *self.current_amplitude.lock().unwrap()
    }

    /// Starts streaming 16kHz 16-bit PCM audio chunks over an async channel
    pub fn start_recording(&mut self, tx: mpsc::Sender<Vec<u8>>, target_device_name: Option<&str>) -> Result<()> {
        let host = cpal::default_host();
        let device = if let Some(target) = target_device_name {
            let mut matched = None;
            if let Ok(devices) = host.input_devices() {
                for d in devices {
                    if let Ok(name) = d.name() {
                        if name == target {
                            matched = Some(d);
                            break;
                        }
                    }
                }
            }
            matched.or_else(|| host.default_input_device())
        } else {
            host.default_input_device()
        }.context("No audio input device found")?;

        let device_name = device.name().unwrap_or_else(|_| "Unknown Device".to_string());
        info!("Opening audio input device: {}", device_name);

        let default_config = device.default_input_config().context("Failed to get default input config")?;
        let sample_rate = default_config.sample_rate().0;
        let channels = default_config.channels();

        info!(
            "Native audio config: {} channels, {} Hz sample rate, format {:?}",
            channels,
            sample_rate,
            default_config.sample_format()
        );

        let is_recording = self.is_recording.clone();
        is_recording.store(true, Ordering::SeqCst);

        let current_amp = self.current_amplitude.clone();
        let mut sample_accumulator: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES);

        // Simple resampler / downmixer to 16kHz mono i16
        let resample_ratio = sample_rate as f64 / TARGET_SAMPLE_RATE as f64;
        let mut resample_counter = 0.0f64;

        let err_fn = |err| {
            error!("Audio input stream error: {}", err);
        };

        let stream = match default_config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &default_config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !is_recording.load(Ordering::SeqCst) {
                        return;
                    }

                    let mut sum_sq = 0.0f32;
                    let num_frames = data.len() / channels as usize;

                    for frame_idx in 0..num_frames {
                        // Average all channels to mono
                        let mut mono_sample = 0.0f32;
                        for ch in 0..channels as usize {
                            mono_sample += data[frame_idx * channels as usize + ch];
                        }
                        mono_sample /= channels as f32;

                        sum_sq += mono_sample * mono_sample;

                        // Downsample to 16kHz
                        resample_counter += 1.0;
                        if resample_counter >= resample_ratio {
                            resample_counter -= resample_ratio;

                            let clamped = mono_sample.clamp(-1.0, 1.0);
                            let i16_val = (clamped * 32767.0) as i16;
                            sample_accumulator.push(i16_val);

                            if sample_accumulator.len() >= CHUNK_SAMPLES {
                                let mut bytes = Vec::with_capacity(sample_accumulator.len() * 2);
                                for s in &sample_accumulator {
                                    bytes.extend_from_slice(&s.to_le_bytes());
                                }
                                sample_accumulator.clear();

                                let _ = tx.try_send(bytes);
                            }
                        }
                    }

                    let rms = (sum_sq / num_frames.max(1) as f32).sqrt();
                    if let Ok(mut amp) = current_amp.lock() {
                        *amp = rms;
                    }
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &default_config.into(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if !is_recording.load(Ordering::SeqCst) {
                        return;
                    }

                    let mut sum_sq = 0.0f32;
                    let num_frames = data.len() / channels as usize;

                    for frame_idx in 0..num_frames {
                        let mut mono_val = 0i32;
                        for ch in 0..channels as usize {
                            mono_val += data[frame_idx * channels as usize + ch] as i32;
                        }
                        let mono_sample = (mono_val / channels as i32) as i16;

                        let norm = mono_sample as f32 / 32768.0;
                        sum_sq += norm * norm;

                        resample_counter += 1.0;
                        if resample_counter >= resample_ratio {
                            resample_counter -= resample_ratio;
                            sample_accumulator.push(mono_sample);

                            if sample_accumulator.len() >= CHUNK_SAMPLES {
                                let mut bytes = Vec::with_capacity(sample_accumulator.len() * 2);
                                for s in &sample_accumulator {
                                    bytes.extend_from_slice(&s.to_le_bytes());
                                }
                                sample_accumulator.clear();

                                let _ = tx.try_send(bytes);
                            }
                        }
                    }

                    let rms = (sum_sq / num_frames.max(1) as f32).sqrt();
                    if let Ok(mut amp) = current_amp.lock() {
                        *amp = rms;
                    }
                },
                err_fn,
                None,
            )?,
            sample_format => {
                warn!("Unsupported audio sample format: {:?}", sample_format);
                return Err(anyhow::anyhow!("Unsupported sample format: {:?}", sample_format));
            }
        };

        stream.play().context("Failed to start audio stream")?;
        self.active_stream = Some(stream);

        Ok(())
    }

    /// Stops audio capture
    pub fn stop_recording(&mut self) {
        self.is_recording.store(false, Ordering::SeqCst);
        if let Some(stream) = self.active_stream.take() {
            let _ = stream.pause();
        }
        if let Ok(mut amp) = self.current_amplitude.lock() {
            *amp = 0.0;
        }
    }
}
