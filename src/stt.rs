use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SttProvider {
    WisprFlow,
    Groq,
    ElevenLabs,
}

impl Default for SttProvider {
    fn default() -> Self {
        Self::WisprFlow
    }
}

impl SttProvider {
    #[allow(dead_code)]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::WisprFlow => "Wispr Flow (gRPC Native)",
            Self::Groq => "Groq Cloud (Whisper LPU)",
            Self::ElevenLabs => "ElevenLabs (Scribe STT)",
        }
    }

    pub fn short_badge(&self) -> &'static str {
        match self {
            Self::WisprFlow => "Wispr",
            Self::Groq => "Groq",
            Self::ElevenLabs => "11Labs",
        }
    }
}

/// Converts raw mono 16-bit PCM samples into a standard in-memory WAV byte stream
pub fn pcm_to_wav(pcm_samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .context("Failed to initialize WAV writer")?;
        for &sample in pcm_samples {
            writer.write_sample(sample)?;
        }
        writer.finalize().context("Failed to finalize WAV data")?;
    }
    Ok(cursor.into_inner())
}

#[derive(Deserialize)]
struct GroqTranscriptionResponse {
    text: String,
}

#[derive(Deserialize)]
struct ElevenLabsTranscriptionResponse {
    #[serde(default)]
    text: String,
}

/// Transcribes audio using Groq Cloud Whisper API (ultra-low latency)
pub async fn transcribe_groq(
    api_key: &str,
    model: &str,
    language: &str,
    wav_bytes: Vec<u8>,
) -> Result<String> {
    if api_key.trim().is_empty() {
        return Err(anyhow::anyhow!("Groq API key is missing. Please set it in Settings."));
    }

    info!(
        "Sending {} bytes of WAV audio to Groq Whisper (model: {}, lang: {})",
        wav_bytes.len(),
        model,
        language
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let file_part = Part::bytes(wav_bytes)
        .file_name("dictation.wav")
        .mime_str("audio/wav")?;

    let mut form = Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "json");

    if !language.is_empty() && language != "auto" {
        form = form.text("language", language.to_string());
    }

    let res = client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .multipart(form)
        .send()
        .await
        .context("Failed to send request to Groq API")?;

    let status = res.status();
    let body_text = res.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow::anyhow!("Groq API error ({status}): {body_text}"));
    }

    let parsed: GroqTranscriptionResponse = serde_json::from_str(&body_text)
        .with_context(|| format!("Failed to parse Groq response: {body_text}"))?;

    info!("Groq transcribed text: '{}'", parsed.text.trim());
    Ok(parsed.text.trim().to_string())
}

/// Transcribes audio using ElevenLabs Speech-to-Text API (Scribe model)
pub async fn transcribe_elevenlabs(
    api_key: &str,
    model_id: &str,
    language: &str,
    wav_bytes: Vec<u8>,
) -> Result<String> {
    if api_key.trim().is_empty() {
        return Err(anyhow::anyhow!("ElevenLabs API key is missing. Please set it in Settings."));
    }

    info!(
        "Sending {} bytes of WAV audio to ElevenLabs Scribe STT (model: {}, lang: {})",
        wav_bytes.len(),
        model_id,
        language
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;

    let file_part = Part::bytes(wav_bytes)
        .file_name("dictation.wav")
        .mime_str("audio/wav")?;

    let mut form = Form::new()
        .part("file", file_part)
        .text("model_id", model_id.to_string());

    if !language.is_empty() && language != "auto" {
        form = form.text("language_code", language.to_string());
    }

    let res = client
        .post("https://api.elevenlabs.io/v1/speech-to-text")
        .header("xi-api-key", api_key.trim())
        .multipart(form)
        .send()
        .await
        .context("Failed to send request to ElevenLabs API")?;

    let status = res.status();
    let body_text = res.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow::anyhow!("ElevenLabs API error ({status}): {body_text}"));
    }

    let parsed: ElevenLabsTranscriptionResponse = serde_json::from_str(&body_text)
        .with_context(|| format!("Failed to parse ElevenLabs response: {body_text}"))?;

    info!("ElevenLabs transcribed text: '{}'", parsed.text.trim());
    Ok(parsed.text.trim().to_string())
}
