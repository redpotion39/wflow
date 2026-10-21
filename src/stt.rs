use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SttProvider {
    #[default]
    WisprFlow,
    Groq,
    ElevenLabs,
}

impl SttProvider {
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

/// Converts raw mono 16-bit PCM samples into a standard in-memory WAV byte stream.
pub fn pcm_to_wav(pcm_samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer =
            hound::WavWriter::new(&mut cursor, spec).context("Failed to initialize WAV writer")?;
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

fn audio_part(wav_bytes: &[u8]) -> Part {
    Part::bytes(wav_bytes.to_vec())
        .file_name("dictation.wav")
        .mime_str("audio/wav")
        .expect("static MIME type is always valid")
}

/// Transcribes audio using the Groq Cloud Whisper API (ultra-low latency).
pub async fn transcribe_groq(
    api_key: &str,
    model: &str,
    language: &str,
    wav_bytes: Vec<u8>,
    timeout_secs: u64,
) -> Result<String> {
    if api_key.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "Groq API key is missing. Please set it in Settings."
        ));
    }

    info!(
        "Sending {} bytes of WAV audio to Groq Whisper (model: {}, lang: {})",
        wav_bytes.len(),
        model,
        language
    );

    let client = crate::http::client();
    let key = api_key.trim().to_string();
    let model = model.to_string();
    let language = language.to_string();

    let res = crate::http::send_with_retry("Groq transcription", 3, || {
        let mut form = Form::new()
            .part("file", audio_part(&wav_bytes))
            .text("model", model.clone())
            .text("response_format", "json");

        if !language.is_empty() && language != "auto" {
            form = form.text("language", language.clone());
        }

        client
            .post("https://api.groq.com/openai/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", key))
            .timeout(Duration::from_secs(timeout_secs.clamp(5, 300)))
            .multipart(form)
    })
    .await?;

    let status = res.status();
    let body_text = res.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow::anyhow!("Groq API error ({status}): {body_text}"));
    }

    let parsed: GroqTranscriptionResponse = serde_json::from_str(&body_text)
        .with_context(|| format!("Failed to parse Groq response: {body_text}"))?;

    info!(
        "Groq transcribed {} characters",
        parsed.text.trim().chars().count()
    );
    Ok(parsed.text.trim().to_string())
}

/// Transcribes audio using the ElevenLabs Speech-to-Text API (Scribe model).
pub async fn transcribe_elevenlabs(
    api_key: &str,
    model_id: &str,
    language: &str,
    wav_bytes: Vec<u8>,
    timeout_secs: u64,
) -> Result<String> {
    if api_key.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "ElevenLabs API key is missing. Please set it in Settings."
        ));
    }

    info!(
        "Sending {} bytes of WAV audio to ElevenLabs Scribe STT (model: {}, lang: {})",
        wav_bytes.len(),
        model_id,
        language
    );

    let client = crate::http::client();
    let key = api_key.trim().to_string();
    let model_id = model_id.to_string();
    let language = language.to_string();

    let res = crate::http::send_with_retry("ElevenLabs transcription", 3, || {
        let mut form = Form::new()
            .part("file", audio_part(&wav_bytes))
            .text("model_id", model_id.clone());

        if !language.is_empty() && language != "auto" {
            form = form.text("language_code", language.clone());
        }

        client
            .post("https://api.elevenlabs.io/v1/speech-to-text")
            .header("xi-api-key", key.clone())
            .timeout(Duration::from_secs(timeout_secs.clamp(5, 300)))
            .multipart(form)
    })
    .await?;

    let status = res.status();
    let body_text = res.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "ElevenLabs API error ({status}): {body_text}"
        ));
    }

    let parsed: ElevenLabsTranscriptionResponse = serde_json::from_str(&body_text)
        .with_context(|| format!("Failed to parse ElevenLabs response: {body_text}"))?;

    info!(
        "ElevenLabs transcribed {} characters",
        parsed.text.trim().chars().count()
    );
    Ok(parsed.text.trim().to_string())
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: String,
}

/// Polishes and formats raw transcribed text using Groq's Llama 3.1 8B (~150 ms).
pub async fn post_process_with_llm(
    api_key: &str,
    raw_text: &str,
    style: &str,
    timeout_secs: u64,
) -> Result<String> {
    if api_key.trim().is_empty() || raw_text.trim().is_empty() || style == "raw" {
        return Ok(raw_text.to_string());
    }

    let system_prompt = match style {
        "smart_cleanup" => {
            "You are an expert real-time voice typing assistant. Clean up the provided transcribed speech: \
             remove stutters, filler words (e.g. um, uh, ah, like, you know, อ่า, เอ่อ, แบบว่า, นะคะ/นะครับซ้ำซ้อน), \
             fix punctuation, capitalization, and grammar while strictly preserving the speaker's original meaning and language (Thai/English). \
             Return ONLY the clean formatted text without preamble, explanations, or quotes."
        }
        "casual" => {
            "You are a friendly messaging assistant. Format the transcribed speech for casual messaging (natural, clear, friendly). \
             Remove stutters and filler words. Return ONLY the formatted message without preamble, explanations, or quotes."
        }
        "professional" => {
            "You are an executive business communication assistant. Refine the transcribed speech into polished, polite, \
             grammatically pristine business language suitable for an email or official document. \
             Return ONLY the refined text without preamble, explanations, or quotes."
        }
        "code" => {
            "You are a programming assistant. Convert the spoken words into code, terminal commands, or technical syntax \
             (camelCase, snake_case, CLI commands, markdown code blocks) as intended by the speaker. \
             Return ONLY the code or command without preamble or explanations."
        }
        "bullet_points" => {
            "You are a summarization assistant. Convert the transcribed speech into clean, well-organized markdown bullet points. \
             Return ONLY the bullet points without preamble or conversational remarks."
        }
        _ => return Ok(raw_text.to_string()),
    };

    info!(
        "Running AI Smart Formatting with style '{}' on Groq Llama 3.1 8B...",
        style
    );

    let client = crate::http::client();
    let key = api_key.trim().to_string();
    let body = ChatCompletionRequest {
        model: "llama-3.1-8b-instant".to_string(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: raw_text.to_string(),
            },
        ],
        temperature: 0.2,
        max_tokens: 1024,
    };

    let res = crate::http::send_with_retry("Groq smart formatting", 3, || {
        client
            .post("https://api.groq.com/openai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", key))
            .timeout(Duration::from_secs(timeout_secs.clamp(3, 120)))
            .json(&body)
    })
    .await?;

    if !res.status().is_success() {
        let err_text = res.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Groq LLM error: {err_text}"));
    }

    let parsed: ChatCompletionResponse = res
        .json()
        .await
        .context("Failed to parse Groq Chat response")?;

    if let Some(first) = parsed.choices.first() {
        let clean = first.message.content.trim().to_string();
        if !clean.is_empty() {
            info!(
                "AI Smart Formatting completed ({} chars)",
                clean.chars().count()
            );
            return Ok(clean);
        }
    }

    Ok(raw_text.to_string())
}

/// Tests connection and validity of the Groq API key.
pub async fn test_groq_api(api_key: &str) -> Result<String> {
    if api_key.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "ยังไม่ได้ระบุ Groq API Key! กรุณากรอก API Key ก่อนทดสอบ"
        ));
    }
    let client = crate::http::client();
    let key = api_key.trim().to_string();
    let res = crate::http::send_with_retry("Groq API test", 2, || {
        client
            .get("https://api.groq.com/openai/v1/models")
            .header("Authorization", format!("Bearer {}", key))
    })
    .await
    .context("ไม่สามารถเชื่อมต่อไปยัง api.groq.com ได้ กรุณาตรวจสอบการเชื่อมต่ออินเทอร์เน็ต")?;

    let status = res.status();
    if status.is_success() {
        Ok("เชื่อมต่อ Groq API สำเร็จ! API Key ถูกต้อง และโมเดล Whisper พร้อมใช้งาน 🚀".to_string())
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        Err(anyhow::anyhow!("Groq API Key ไม่ถูกต้อง (401 Unauthorized)"))
    } else {
        let body = res.text().await.unwrap_or_default();
        Err(anyhow::anyhow!(
            "Groq API เกิดข้อผิดพลาด: HTTP {} - {}",
            status,
            body
        ))
    }
}

/// Tests connection and validity of the ElevenLabs API key.
pub async fn test_elevenlabs_api(api_key: &str) -> Result<String> {
    if api_key.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "ยังไม่ได้ระบุ ElevenLabs API Key! กรุณากรอก API Key ก่อนทดสอบ"
        ));
    }
    let client = crate::http::client();
    let key = api_key.trim().to_string();
    let res = crate::http::send_with_retry("ElevenLabs API test", 2, || {
        client
            .get("https://api.elevenlabs.io/v1/user")
            .header("xi-api-key", key.clone())
    })
    .await
    .context("ไม่สามารถเชื่อมต่อไปยัง api.elevenlabs.io ได้ กรุณาตรวจสอบการเชื่อมต่ออินเทอร์เน็ต")?;

    let status = res.status();
    if status.is_success() {
        Ok("เชื่อมต่อ ElevenLabs API สำเร็จ! API Key ถูกต้อง และบริการพร้อมใช้งาน 🎙️".to_string())
    } else if status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
    {
        Err(anyhow::anyhow!(
            "ElevenLabs API Key ไม่ถูกต้อง (401 Unauthorized / 403 Forbidden)"
        ))
    } else {
        let body = res.text().await.unwrap_or_default();
        Err(anyhow::anyhow!(
            "ElevenLabs API เกิดข้อผิดพลาด: HTTP {} - {}",
            status,
            body
        ))
    }
}

/// Tests connection to the configured gRPC endpoint.
pub async fn test_wisprflow_endpoint(endpoint_url: &str) -> Result<String> {
    if endpoint_url.trim().is_empty() {
        return Err(anyhow::anyhow!("ยังไม่ได้ระบุ Endpoint URL"));
    }
    let client = crate::http::client();
    let url = endpoint_url.trim().to_string();
    match crate::http::send_with_retry("Endpoint test", 2, || client.get(&url)).await {
        // gRPC endpoints reject a raw HTTP GET, but any HTTP response proves
        // the host is reachable.
        Ok(_) => Ok("เชื่อมต่อเซิร์ฟเวอร์ Endpoint สำเร็จ ⚡".to_string()),
        Err(e) => Err(anyhow::anyhow!(
            "ไม่สามารถเชื่อมต่อ Endpoint ({}): {:#}",
            endpoint_url,
            e
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_valid() {
        let samples = vec![0i16; 1600];
        let wav = pcm_to_wav(&samples, 16_000).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + samples.len() * 2);
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap());
        assert_eq!(data_len as usize, samples.len() * 2);
    }

    #[test]
    fn provider_badges_are_stable() {
        assert_eq!(SttProvider::WisprFlow.short_badge(), "Wispr");
        assert_eq!(SttProvider::Groq.short_badge(), "Groq");
        assert_eq!(SttProvider::ElevenLabs.short_badge(), "11Labs");
    }
}
