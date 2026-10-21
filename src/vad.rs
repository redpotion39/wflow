//! Voice activity detection — the "silence filter".
//!
//! While dictating, the microphone is almost always picking up room tone, a
//! fan or keyboard clicks. Sending that to a speech-to-text provider wastes
//! bandwidth and, worse, makes the model hallucinate ("Thank you.", "♪♪").
//!
//! The gate keeps a running estimate of the ambient noise floor and only
//! forwards chunks that are clearly above it. A short pre-roll buffer is kept
//! so the first syllable of a word is never clipped, and the tail of the
//! recording is trimmed when it is pure silence.

use std::collections::VecDeque;

/// Configuration of the noise gate.
#[derive(Debug, Clone)]
pub struct GateConfig {
    /// Master switch.
    pub enabled: bool,
    /// Manual threshold as an RMS value (0.0 – 1.0). `None` means "adapt to
    /// the ambient noise floor".
    pub manual_threshold: Option<f32>,
    /// How far above the noise floor a chunk has to be to count as speech.
    pub margin: f32,
    /// How long a chunk of silence is kept as pre-roll, in milliseconds.
    pub pre_roll_ms: u64,
    /// Stop the recording after this much continuous silence (0 = never).
    pub stop_after_silence_ms: u64,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            manual_threshold: None,
            margin: 0.004,
            pre_roll_ms: 250,
            stop_after_silence_ms: 0,
        }
    }
}

/// The outcome of feeding one chunk to the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Silence — the chunk was buffered as pre-roll.
    Silence,
    /// Speech — the chunk should be forwarded.
    Speech,
    /// Speech, and the recording should be stopped because the speaker went
    /// quiet for too long.
    SpeechAndStop,
}

/// Adaptive noise gate with a pre-roll buffer.
pub struct VoiceGate {
    config: GateConfig,
    /// Rolling RMS values used to estimate the noise floor.
    levels: VecDeque<f32>,
    /// Estimated ambient noise floor.
    noise_floor: f32,
    /// Currently detected as speaking.
    speaking: bool,
    /// Buffered silence that is replayed when speech starts.
    pre_roll: VecDeque<Vec<u8>>,
    /// Milliseconds of continuous silence while speaking.
    silent_ms: u64,
    /// Total milliseconds of speech detected in this session.
    speech_ms: u64,
    /// Number of chunks that were dropped as silence.
    suppressed: u64,
}

impl VoiceGate {
    pub fn new(config: GateConfig) -> Self {
        Self {
            config,
            levels: VecDeque::with_capacity(64),
            noise_floor: 0.0,
            speaking: false,
            pre_roll: VecDeque::new(),
            silent_ms: 0,
            speech_ms: 0,
            suppressed: 0,
        }
    }

    /// The estimated ambient noise floor.
    pub fn noise_floor(&self) -> f32 {
        self.noise_floor
    }

    /// True while the gate considers the user to be speaking.
    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// Total speech detected so far, in milliseconds.
    pub fn speech_ms(&self) -> u64 {
        self.speech_ms
    }

    /// How many chunks were suppressed as silence.
    pub fn suppressed_chunks(&self) -> u64 {
        self.suppressed
    }

    /// The threshold currently in use.
    ///
    /// The threshold is derived from the measured noise floor: a chunk has to
    /// be at least twice as loud as the room tone, and never below a small
    /// absolute floor so that a perfectly quiet room still works.
    pub fn threshold(&self) -> f32 {
        match self.config.manual_threshold {
            Some(t) if t > 0.0 => t,
            _ => (self.noise_floor * 2.0).max(0.003) + self.config.margin,
        }
    }

    /// Decides whether a chunk contains speech and returns the chunks that
    /// should be forwarded (the chunk itself plus any pre-roll).
    pub fn process(
        &mut self,
        rms: f32,
        chunk: Vec<u8>,
        chunk_ms: u64,
    ) -> (GateDecision, Vec<Vec<u8>>) {
        if !self.config.enabled {
            return (GateDecision::Speech, vec![chunk]);
        }

        // Track every level so the floor can follow a changing room tone; a
        // low percentile keeps a single loud noise from raising it.
        self.levels.push_back(rms);
        while self.levels.len() > 100 {
            self.levels.pop_front();
        }
        let mut sorted: Vec<f32> = self.levels.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let index = (sorted.len() as f64 * 0.1) as usize;
        self.noise_floor = sorted[index.min(sorted.len() - 1)];

        let threshold = self.threshold();

        if rms >= threshold {
            self.speaking = true;
            self.speech_ms += chunk_ms;
            self.silent_ms = 0;

            let mut out = Vec::new();
            // Replay the buffered silence so the first syllable is not clipped.
            out.extend(self.pre_roll.drain(..));
            out.push(chunk);

            if self.config.stop_after_silence_ms > 0 {
                // Not reached: silence is measured below.
            }
            return (GateDecision::Speech, out);
        }

        // Silence.
        self.suppressed += 1;
        if self.speaking {
            self.silent_ms += chunk_ms;
            if self.config.stop_after_silence_ms > 0
                && self.silent_ms >= self.config.stop_after_silence_ms
            {
                self.speaking = false;
                return (GateDecision::SpeechAndStop, Vec::new());
            }
        } else {
            self.pre_roll.push_back(chunk);
            let keep = (self.config.pre_roll_ms as f64 / chunk_ms.max(1) as f64).ceil() as usize;
            while self.pre_roll.len() > keep.max(1) {
                self.pre_roll.pop_front();
            }
        }

        (GateDecision::Silence, Vec::new())
    }

    /// Drops the buffered pre-roll, e.g. when the recording is cancelled.
    pub fn reset(&mut self) {
        self.pre_roll.clear();
        self.speaking = false;
        self.silent_ms = 0;
        self.speech_ms = 0;
        self.suppressed = 0;
        self.levels.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(tag: u8) -> Vec<u8> {
        vec![tag; 320]
    }

    #[test]
    fn forwards_speech() {
        let mut gate = VoiceGate::new(GateConfig {
            manual_threshold: Some(0.05),
            ..Default::default()
        });
        let (decision, out) = gate.process(0.5, chunk(1), 100);
        assert_eq!(decision, GateDecision::Speech);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn suppresses_silence_but_keeps_pre_roll() {
        let mut gate = VoiceGate::new(GateConfig {
            manual_threshold: Some(0.05),
            pre_roll_ms: 300,
            ..Default::default()
        });
        for _ in 0..5 {
            let (decision, out) = gate.process(0.001, chunk(0), 100);
            assert_eq!(decision, GateDecision::Silence);
            assert!(out.is_empty());
        }
        let (_, out) = gate.process(0.5, chunk(1), 100);
        // The buffered silence is replayed together with the speech chunk.
        assert!(out.len() >= 4, "pre-roll not kept: {}", out.len());
    }

    #[test]
    fn disabled_gate_forwards_everything() {
        let mut gate = VoiceGate::new(GateConfig {
            enabled: false,
            ..Default::default()
        });
        let (decision, out) = gate.process(0.0, chunk(0), 100);
        assert_eq!(decision, GateDecision::Speech);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn stops_after_long_silence() {
        let mut gate = VoiceGate::new(GateConfig {
            manual_threshold: Some(0.05),
            stop_after_silence_ms: 300,
            ..Default::default()
        });
        assert_eq!(gate.process(0.5, chunk(1), 100).0, GateDecision::Speech);
        assert_eq!(gate.process(0.0, chunk(0), 100).0, GateDecision::Silence);
        assert_eq!(gate.process(0.0, chunk(0), 100).0, GateDecision::Silence);
        assert_eq!(
            gate.process(0.0, chunk(0), 100).0,
            GateDecision::SpeechAndStop
        );
    }

    #[test]
    fn noise_floor_adapts_to_the_room() {
        // A noisy room raises the floor.
        let mut gate = VoiceGate::new(GateConfig {
            manual_threshold: None,
            ..Default::default()
        });
        for _ in 0..40 {
            gate.process(0.02, chunk(0), 100);
        }
        assert!(gate.noise_floor() > 0.005, "floor: {}", gate.noise_floor());
        assert!(gate.threshold() > 0.02, "threshold: {}", gate.threshold());

        // A quiet room keeps it low.
        let mut quiet = VoiceGate::new(GateConfig {
            manual_threshold: None,
            ..Default::default()
        });
        for _ in 0..40 {
            quiet.process(0.0005, chunk(0), 100);
        }
        assert!(
            quiet.noise_floor() < 0.005,
            "floor: {}",
            quiet.noise_floor()
        );
        assert!(quiet.threshold() < 0.02, "threshold: {}", quiet.threshold());
    }

    #[test]
    fn speech_above_the_floor_passes_the_gate() {
        let mut gate = VoiceGate::new(GateConfig {
            manual_threshold: None,
            ..Default::default()
        });
        // Room tone first, so the gate learns the floor.
        for _ in 0..30 {
            gate.process(0.004, chunk(0), 100);
        }
        let (decision, out) = gate.process(0.08, chunk(1), 100);
        assert_eq!(decision, GateDecision::Speech);
        assert!(!out.is_empty());
    }

    #[test]
    fn counts_suppressed_chunks() {
        let mut gate = VoiceGate::new(GateConfig {
            manual_threshold: Some(0.05),
            ..Default::default()
        });
        for _ in 0..3 {
            gate.process(0.0, chunk(0), 100);
        }
        assert_eq!(gate.suppressed_chunks(), 3);
    }
}
