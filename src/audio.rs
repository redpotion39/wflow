use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::vad::{GateDecision, VoiceGate};

pub const TARGET_SAMPLE_RATE: u32 = 16_000;
pub const CHUNK_SAMPLES: usize = 1600; // 100 ms at 16 kHz

/// Milliseconds of audio in one chunk.
pub const CHUNK_MS: u64 = (CHUNK_SAMPLES as u64 * 1000) / TARGET_SAMPLE_RATE as u64;

pub struct AudioRecorder {
    is_recording: Arc<AtomicBool>,
    active_stream: Option<cpal::Stream>,
    current_amplitude: Arc<Mutex<f32>>,
    dropped_chunks: Arc<AtomicU64>,
    /// Shared noise gate, so the main thread can inspect what was detected.
    gate: Option<Arc<Mutex<VoiceGate>>>,
    /// Set by the audio thread when the speaker went quiet for too long.
    stop_requested: Arc<AtomicBool>,
}

impl Default for AudioRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// Box-filter downsampler. Averaging the input samples that fall inside one
/// output period removes most aliasing that naive decimation would introduce.
struct Resampler {
    phase: f64,
    sum: f32,
    count: u32,
    input_rate: f64,
}

impl Resampler {
    fn new(input_rate: u32) -> Self {
        Self {
            phase: 0.0,
            sum: 0.0,
            count: 0,
            input_rate: input_rate.max(1) as f64,
        }
    }

    fn push(&mut self, sample: f32, out: &mut Vec<i16>) {
        self.sum += sample.clamp(-1.0, 1.0);
        self.count += 1;
        self.phase += TARGET_SAMPLE_RATE as f64;
        while self.phase >= self.input_rate {
            self.phase -= self.input_rate;
            let avg = if self.count > 0 {
                self.sum / self.count as f32
            } else {
                0.0
            };
            out.push((avg.clamp(-1.0, 1.0) * 32767.0) as i16);
            self.sum = 0.0;
            self.count = 0;
            // When upsampling (input < 16 kHz) avoid an infinite loop; one
            // output sample per input sample is the best we can do.
            if self.input_rate < TARGET_SAMPLE_RATE as f64 {
                break;
            }
        }
    }
}

/// Flushes a full chunk to the async channel without blocking the audio thread.
///
/// When a noise gate is configured the chunk is first classified as speech or
/// silence; silence is buffered as pre-roll instead of being sent to the
/// speech-to-text provider.
fn emit_chunk(
    accumulator: &mut Vec<i16>,
    tx: &mpsc::Sender<Vec<u8>>,
    dropped: &AtomicU64,
    gate: &Option<Arc<Mutex<VoiceGate>>>,
    stop_requested: &AtomicBool,
) -> Option<GateDecision> {
    if accumulator.len() < CHUNK_SAMPLES {
        return None;
    }

    // RMS of the chunk itself, which is what the gate compares against.
    let sum_sq: f64 = accumulator
        .iter()
        .map(|s| {
            let v = *s as f64 / 32768.0;
            v * v
        })
        .sum();
    let rms = (sum_sq / accumulator.len() as f64).sqrt() as f32;

    let mut bytes = Vec::with_capacity(accumulator.len() * 2);
    for sample in accumulator.iter() {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    accumulator.clear();

    let Some(gate) = gate else {
        if tx.try_send(bytes).is_err() {
            // The transcriber is behind; drop this chunk but keep the counter so
            // the user can be warned instead of silently losing audio.
            dropped.fetch_add(1, Ordering::Relaxed);
        }
        return None;
    };

    let decision = {
        let mut g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let (decision, chunks) = g.process(rms, bytes, CHUNK_MS);
        if decision == GateDecision::SpeechAndStop {
            stop_requested.store(true, Ordering::SeqCst);
        }
        for chunk in chunks {
            if tx.try_send(chunk).is_err() {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        decision
    };

    Some(decision)
}

/// Enumerates all available audio input devices on the system.
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
            dropped_chunks: Arc::new(AtomicU64::new(0)),
            gate: None,
            stop_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Milliseconds of speech the gate has detected in this session.
    pub fn detected_speech_ms(&self) -> u64 {
        self.gate
            .as_ref()
            .map(|g| g.lock().unwrap_or_else(|e| e.into_inner()).speech_ms())
            .unwrap_or(0)
    }

    /// True when the user is currently speaking.
    pub fn is_speaking(&self) -> bool {
        self.gate
            .as_ref()
            .map(|g| g.lock().unwrap_or_else(|e| e.into_inner()).is_speaking())
            .unwrap_or(false)
    }

    /// True when the gate asked the recorder to stop (long silence).
    pub fn stop_requested(&self) -> bool {
        self.stop_requested.load(Ordering::SeqCst)
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }

    pub fn get_amplitude(&self) -> f32 {
        *self
            .current_amplitude
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Starts streaming 16 kHz 16-bit PCM audio chunks over an async channel.
    pub fn start_recording(
        &mut self,
        tx: mpsc::Sender<Vec<u8>>,
        target_device_name: Option<&str>,
        gate: Option<Arc<Mutex<VoiceGate>>>,
    ) -> Result<()> {
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
        }
        .context("No audio input device found")?;

        let device_name = device
            .name()
            .unwrap_or_else(|_| "Unknown Device".to_string());
        info!("Opening audio input device: {}", device_name);

        let default_config = device
            .default_input_config()
            .context("Failed to get default input config")?;
        let sample_rate = default_config.sample_rate().0;
        let channels = default_config.channels().max(1);
        let sample_format = default_config.sample_format();

        info!(
            "Native audio config: {} channels, {} Hz sample rate, format {:?}",
            channels, sample_rate, sample_format
        );

        let is_recording = self.is_recording.clone();
        is_recording.store(true, Ordering::SeqCst);

        let current_amp = self.current_amplitude.clone();
        let dropped = self.dropped_chunks.clone();
        dropped.store(0, Ordering::Relaxed);
        let stop_requested = self.stop_requested.clone();
        stop_requested.store(false, Ordering::SeqCst);
        self.gate = gate.clone();

        let err_fn = |err| {
            error!("Audio input stream error: {}", err);
        };

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let mut resampler = Resampler::new(sample_rate);
                let mut accumulator: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES);
                let tx = tx.clone();
                let dropped = dropped.clone();
                let gate = gate.clone();
                let stop_requested = stop_requested.clone();
                device.build_input_stream(
                    &default_config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if !is_recording.load(Ordering::SeqCst) {
                            return;
                        }
                        let frames = data.len() / channels as usize;
                        if frames == 0 {
                            return;
                        }
                        let mut sum_sq = 0.0f32;
                        for frame in data.chunks_exact(channels as usize) {
                            let mono = frame.iter().sum::<f32>() / channels as f32;
                            resampler.push(mono, &mut accumulator);
                            sum_sq += mono * mono;
                            emit_chunk(&mut accumulator, &tx, &dropped, &gate, &stop_requested);
                        }
                        if let Ok(mut amp) = current_amp.lock() {
                            *amp = (sum_sq / frames as f32).sqrt();
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let mut resampler = Resampler::new(sample_rate);
                let mut accumulator: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES);
                let tx = tx.clone();
                let dropped = dropped.clone();
                let gate = gate.clone();
                let stop_requested = stop_requested.clone();
                device.build_input_stream(
                    &default_config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if !is_recording.load(Ordering::SeqCst) {
                            return;
                        }
                        let frames = data.len() / channels as usize;
                        if frames == 0 {
                            return;
                        }
                        let mut sum_sq = 0.0f32;
                        for frame in data.chunks_exact(channels as usize) {
                            let sum: i32 = frame.iter().map(|s| *s as i32).sum();
                            let mono = (sum as f32 / channels as f32) / 32768.0;
                            resampler.push(mono, &mut accumulator);
                            sum_sq += mono * mono;
                            emit_chunk(&mut accumulator, &tx, &dropped, &gate, &stop_requested);
                        }
                        if let Ok(mut amp) = current_amp.lock() {
                            *amp = (sum_sq / frames as f32).sqrt();
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let mut resampler = Resampler::new(sample_rate);
                let mut accumulator: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES);
                let tx = tx.clone();
                let dropped = dropped.clone();
                let gate = gate.clone();
                let stop_requested = stop_requested.clone();
                device.build_input_stream(
                    &default_config.into(),
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        if !is_recording.load(Ordering::SeqCst) {
                            return;
                        }
                        let frames = data.len() / channels as usize;
                        if frames == 0 {
                            return;
                        }
                        let mut sum_sq = 0.0f32;
                        for frame in data.chunks_exact(channels as usize) {
                            let sum: u32 = frame.iter().map(|s| *s as u32).sum();
                            let avg = sum as f32 / channels as f32;
                            let mono = (avg - 32768.0) / 32768.0;
                            resampler.push(mono, &mut accumulator);
                            sum_sq += mono * mono;
                            emit_chunk(&mut accumulator, &tx, &dropped, &gate, &stop_requested);
                        }
                        if let Ok(mut amp) = current_amp.lock() {
                            *amp = (sum_sq / frames as f32).sqrt();
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            other => {
                warn!("Unsupported audio sample format: {:?}", other);
                return Err(anyhow::anyhow!("Unsupported sample format: {:?}", other));
            }
        };

        stream.play().context("Failed to start audio stream")?;
        self.active_stream = Some(stream);

        Ok(())
    }

    /// Stops audio capture.
    pub fn stop_recording(&mut self) {
        self.is_recording.store(false, Ordering::SeqCst);
        if let Some(stream) = self.active_stream.take() {
            let _ = stream.pause();
        }
        if let Ok(mut amp) = self.current_amplitude.lock() {
            *amp = 0.0;
        }
        let dropped = self.dropped_chunks.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            warn!(
                "Dropped {} audio chunk(s) because transcription could not keep up",
                dropped
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsamples_48k_to_16k() {
        let mut resampler = Resampler::new(48_000);
        let mut out = Vec::new();
        for _ in 0..48_000 {
            resampler.push(0.25, &mut out);
        }
        // One second of input at 48 kHz must produce ~16k output samples.
        assert!((out.len() as i64 - 16_000).abs() <= 1, "got {}", out.len());
        let expected = (0.25 * 32767.0) as i16;
        assert!((out[0] - expected).abs() <= 1);
    }

    #[test]
    fn upsamples_without_hanging() {
        let mut resampler = Resampler::new(8_000);
        let mut out = Vec::new();
        for _ in 0..8_000 {
            resampler.push(0.0, &mut out);
        }
        assert!(out.len() >= 8_000);
    }
}
