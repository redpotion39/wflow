use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundEffect {
    StartRecording,
    FinishRecording,
    Error,
}

static SOUND_ENABLED: AtomicBool = AtomicBool::new(true);
static PLAYBACK_WARNED: AtomicBool = AtomicBool::new(false);

pub fn set_sound_enabled(enabled: bool) {
    SOUND_ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn is_sound_enabled() -> bool {
    SOUND_ENABLED.load(Ordering::SeqCst)
}

/// Synthesizes a clean, pleasant PCM WAV buffer in-memory with a fade envelope.
fn generate_sine_wav(sample_rate: u32, notes: &[(f32, f32, f32)]) -> Vec<u8> {
    let total_samples: usize = notes
        .iter()
        .map(|(_, dur, _)| (dur * sample_rate as f32) as usize)
        .sum();

    let mut samples: Vec<i16> = Vec::with_capacity(total_samples);

    for &(freq, dur, amp) in notes {
        let n_samples = (dur * sample_rate as f32) as usize;
        let mut phase = 0.0f32;
        let phase_inc = (2.0 * std::f32::consts::PI * freq) / sample_rate as f32;

        for i in 0..n_samples {
            let t = i as f32 / n_samples as f32;
            let envelope = if t < 0.1 {
                t / 0.1 // Fast attack fade-in
            } else {
                (1.0 - (t - 0.1) / 0.9).powf(1.6) // Smooth exponential fade-out
            };

            let val = phase.sin() * amp * envelope;
            let s = (val.clamp(-1.0, 1.0) * 32767.0) as i16;
            samples.push(s);
            phase += phase_inc;
        }
    }

    let data_len = (samples.len() * 2) as u32;
    let file_len = 36 + data_len;
    let byte_rate = sample_rate * 2;

    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_len.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // subchunk1size (16 for PCM)
    wav.extend_from_slice(&1u16.to_le_bytes()); // AudioFormat (1 = PCM)
    wav.extend_from_slice(&1u16.to_le_bytes()); // NumChannels (1 = Mono)
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes()); // BlockAlign
    wav.extend_from_slice(&16u16.to_le_bytes()); // BitsPerSample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }

    wav
}

/// Builds the WAV payload for an effect (exposed for tests).
pub fn synthesize(effect: SoundEffect) -> Vec<u8> {
    match effect {
        SoundEffect::StartRecording => {
            // Soft ascending two-tone chime (540Hz -> 820Hz, ~80ms)
            generate_sine_wav(22050, &[(540.0, 0.035, 0.25), (820.0, 0.065, 0.35)])
        }
        SoundEffect::FinishRecording => {
            // Warm dual-harmonic completion chord (660Hz -> 1050Hz, ~120ms)
            generate_sine_wav(22050, &[(660.0, 0.045, 0.28), (1050.0, 0.085, 0.38)])
        }
        SoundEffect::Error => {
            // Soft low double-tone warning (280Hz -> 200Hz, ~120ms)
            generate_sine_wav(22050, &[(280.0, 0.06, 0.30), (200.0, 0.08, 0.30)])
        }
    }
}

/// Plays a pleasant synthesized earcon asynchronously without blocking the UI.
pub fn play_sound(effect: SoundEffect) {
    if !is_sound_enabled() {
        return;
    }

    std::thread::spawn(move || {
        let wav = synthesize(effect);

        #[cfg(target_os = "windows")]
        unsafe {
            // SND_SYNC = 0x0000, SND_NODEFAULT = 0x0002, SND_MEMORY = 0x0004
            #[link(name = "winmm")]
            extern "system" {
                fn PlaySoundW(
                    pszSound: *const u8,
                    hmod: *mut std::ffi::c_void,
                    fdwSound: u32,
                ) -> i32;
            }
            let _ = PlaySoundW(wav.as_ptr(), std::ptr::null_mut(), 0x0000 | 0x0002 | 0x0004);
        }

        #[cfg(not(target_os = "windows"))]
        {
            play_wav_file(&wav);
        }
    });
}

/// Writes the WAV to a temporary file and delegates to an installed player.
/// Silently does nothing when no player is available (headless sessions).
#[cfg(not(target_os = "windows"))]
fn play_wav_file(wav: &[u8]) {
    use std::io::Write;

    let path = std::env::temp_dir().join(format!(
        "wflow-earcon-{}-{}.wav",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    let Ok(mut file) = std::fs::File::create(&path) else {
        return;
    };
    if file.write_all(wav).is_err() {
        return;
    }
    drop(file);

    #[cfg(target_os = "macos")]
    let players: &[(&str, &[&str])] = &[("afplay", &[])];
    #[cfg(target_os = "linux")]
    let players: &[(&str, &[&str])] = &[
        ("paplay", &[]),
        ("pw-play", &[]),
        ("aplay", &["-q"]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
    ];

    let mut played = false;
    for (program, args) in players {
        let status = std::process::Command::new(program)
            .args(*args)
            .arg(&path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if status.map(|s| s.success()).unwrap_or(false) {
            played = true;
            break;
        }
    }

    if !played && !PLAYBACK_WARNED.swap(true, Ordering::Relaxed) {
        tracing::info!(
            "No audio player found for earcons (install pulseaudio-utils or alsa-utils)"
        );
    }
    let _ = std::fs::remove_file(&path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_produce_valid_wav() {
        for effect in [
            SoundEffect::StartRecording,
            SoundEffect::FinishRecording,
            SoundEffect::Error,
        ] {
            let wav = synthesize(effect);
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(&wav[8..12], b"WAVE");
            assert!(wav.len() > 44);
        }
    }

    #[test]
    fn toggle_flag_roundtrip() {
        set_sound_enabled(false);
        assert!(!is_sound_enabled());
        set_sound_enabled(true);
        assert!(is_sound_enabled());
    }
}
