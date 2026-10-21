//! Guard against garbage output from the speech-to-text / language models.
//!
//! Models hallucinate on silence and background noise. Whisper-style models in
//! particular like to emit "Thank you.", "Subtitles by the Amara.org
//! community" or "(upbeat music)" when they are fed nothing but room tone.
//! Pasting that into the user's editor is worse than pasting nothing at all,
//! so every transcript is validated before it is inserted.

use crate::lang::{self, Languages, Script, TokenKind};

/// Why a transcript was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    Empty,
    /// A known caption/transcript boilerplate phrase.
    Boilerplate,
    /// Only stage directions such as "(music)" or "[BLANK_AUDIO]".
    StageDirection,
    /// The same word repeated over and over.
    WordRepetition,
    /// The same character repeated over and over.
    CharacterRepetition,
    /// Written in a script the user did not ask for.
    ScriptMismatch,
    /// Mostly punctuation, digits and symbols.
    LowWordQuality,
}

impl RejectReason {
    /// A short explanation for the user interface.
    pub fn message(self) -> &'static str {
        match self {
            RejectReason::Empty => "ไม่ได้ยินเสียงพูด",
            RejectReason::Boilerplate => "ตรวจพบข้อความสำเร็จรูปจากโมเดล (ไม่ได้ยินเสียงพูด)",
            RejectReason::StageDirection => "ตรวจพบเฉพาะคำบรรยายเสียง (ไม่ได้ยินเสียงพูด)",
            RejectReason::WordRepetition => "ตรวจพบข้อความซ้ำผิดปกติจากโมเดล",
            RejectReason::CharacterRepetition => "ตรวจพบอักขระซ้ำผิดปกติจากโมเดล",
            RejectReason::ScriptMismatch => "ตรวจพบภาษาที่ไม่ตรงกับที่เลือกไว้",
            RejectReason::LowWordQuality => "ตรวจพบข้อความที่ไม่มีความหมาย",
        }
    }
}

/// Boilerplate that captioning / transcription models like to emit.
const BOILERPLATE: &[&str] = &[
    "thank you",
    "thanks for watching",
    "thanks for watching!",
    "thank you.",
    "thank you!",
    "please subscribe",
    "subtitles by the amara.org community",
    "amara.org",
    "transcribed by",
    "www.",
    "http://",
    "https://",
    "♪♪♪",
    "music",
    "upbeat music",
    "soft music",
    "background music",
    "silence",
    "inaudible",
    "blank audio",
    "foreign",
];

/// Keywords that identify a stage direction inside brackets or parentheses.
const STAGE_KEYWORDS: &[&str] = &[
    "music",
    "silence",
    "blank",
    "inaudible",
    "applause",
    "laughter",
    "upbeat",
    "instrumental",
    "no sound",
    "static",
    "beep",
    "cough",
    "sneeze",
    "thunder",
    "wind",
    "bird",
    "traffic",
];

/// Removes stage directions such as "(upbeat music)" or "[BLANK_AUDIO]".
///
/// Returns the text that is left; if nothing meaningful remains the transcript
/// only contained non-speech annotations.
fn strip_stage_directions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = rest.find(['[', '(']) {
        let ch = rest.as_bytes()[open] as char;
        let close = if ch == '[' { ']' } else { ')' };
        out.push_str(&rest[..open]);

        match rest[open + 1..].find(close) {
            Some(end) => {
                let inner = &rest[open + 1..open + 1 + end];
                let lowered = inner.to_lowercase();
                if STAGE_KEYWORDS.iter().any(|k| lowered.contains(k)) {
                    // Drop the whole stage direction.
                } else {
                    out.push_str(&rest[open..open + 1 + end + 1]);
                }
                rest = &rest[open + 1 + end + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);

    // Music symbols are never speech.
    out.chars()
        .filter(|c| !matches!(c, '♪' | '♫' | '♬' | '♩'))
        .collect()
}

/// How similar the LLM result is to the text that was sent to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmSanity {
    /// The result keeps the original content.
    Good,
    /// The result drifted too far from the original — keep the original.
    Drifted,
}

/// Normalises text for comparisons: lower case, collapse whitespace, strip
/// trailing punctuation.
fn normalize(text: &str) -> String {
    let lowered = text.to_lowercase();
    let collapsed = lang::collapse_duplicate_spaces(&lowered);
    collapsed
        .trim_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .to_string()
}

/// Checks a transcript and returns the reason it should be rejected, if any.
///
/// `expected` is the language the user configured; `None` accepts any script.
pub fn check_transcript(text: &str, expected: Option<Languages>) -> Option<RejectReason> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(RejectReason::Empty);
    }

    let normalized = normalize(trimmed);
    if normalized.is_empty() {
        return Some(RejectReason::Empty);
    }

    // 1. Known boilerplate.
    if BOILERPLATE.iter().any(|b| normalized == *b) {
        return Some(RejectReason::Boilerplate);
    }

    // 2. Only stage directions / non-speech markers.
    let letters = trimmed
        .chars()
        .filter(|c| c.is_alphabetic() || matches!(lang::script_of(*c), Script::Thai))
        .count();
    if letters < 3 {
        return Some(RejectReason::StageDirection);
    }
    let without_markers = strip_stage_directions(trimmed);
    if without_markers.trim().is_empty() {
        return Some(RejectReason::StageDirection);
    }
    // Only a tiny amount of real text left next to the stage directions.
    let remaining_letters = without_markers
        .chars()
        .filter(|c| c.is_alphabetic() || matches!(lang::script_of(*c), Script::Thai))
        .count();
    if remaining_letters == 0 {
        return Some(RejectReason::StageDirection);
    }

    // 3. The same word repeated over and over ("the the the the").
    let words: Vec<String> = lang::tokenize(trimmed)
        .into_iter()
        .filter(|t| t.kind == TokenKind::Word)
        .map(|t| t.text.to_lowercase())
        .collect();
    if words.len() >= 4 {
        let mut run = 1usize;
        let mut worst = 1usize;
        for pair in words.windows(2) {
            if pair[0] == pair[1] {
                run += 1;
                worst = worst.max(run);
            } else {
                run = 1;
            }
        }
        if worst >= 4 {
            return Some(RejectReason::WordRepetition);
        }
    }

    // 4. The same character repeated over and over ("aaaaaaaaaa").
    let mut char_run = 1usize;
    let mut worst_char = 1usize;
    let mut prev: Option<char> = None;
    for ch in trimmed.chars() {
        if Some(ch) == prev {
            char_run += 1;
            worst_char = worst_char.max(char_run);
        } else {
            char_run = 1;
        }
        prev = Some(ch);
    }
    if worst_char >= 12 {
        return Some(RejectReason::CharacterRepetition);
    }

    // 5. The wrong script entirely.
    if let Some(expected) = expected {
        let actual = lang::detect_languages(trimmed);
        let mismatch = matches!(
            (expected, actual),
            (Languages::Thai, Languages::Latin) | (Languages::Latin, Languages::Thai)
        );
        if mismatch {
            return Some(RejectReason::ScriptMismatch);
        }
    }

    // 6. Mostly symbols and digits.
    let total = trimmed.chars().filter(|c| !c.is_whitespace()).count();
    if total > 0 {
        let meaningful = trimmed
            .chars()
            .filter(|c| c.is_alphabetic() || matches!(lang::script_of(*c), Script::Thai))
            .count();
        if (meaningful as f32) < (total as f32) * 0.35 {
            return Some(RejectReason::LowWordQuality);
        }
    }

    None
}

/// Checks that a language model's rewrite still resembles the original text.
///
/// A model that "improves" the text into something completely different is
/// usually hallucinating, so the original is kept instead.
pub fn check_llm_result(original: &str, rewritten: &str) -> LlmSanity {
    let original_words: Vec<String> = lang::tokenize(original)
        .into_iter()
        .filter(|t| t.kind == TokenKind::Word)
        .map(|t| t.text.to_lowercase())
        .collect();

    if original_words.is_empty() {
        return LlmSanity::Good;
    }

    let rewritten_words: Vec<String> = lang::tokenize(rewritten)
        .into_iter()
        .filter(|t| t.kind == TokenKind::Word)
        .map(|t| t.text.to_lowercase())
        .collect();

    if rewritten_words.is_empty() {
        return LlmSanity::Drifted;
    }

    // At least half of the original words must survive.
    let kept = original_words
        .iter()
        .filter(|w| rewritten_words.iter().any(|r| r == *w))
        .count();
    let ratio = kept as f32 / original_words.len() as f32;

    // A rewrite that is wildly longer than the original is suspicious too.
    let length_ratio = rewritten_words.len() as f32 / original_words.len() as f32;

    if ratio < 0.5 || length_ratio > 4.0 {
        LlmSanity::Drifted
    } else {
        LlmSanity::Good
    }
}

/// The script the user expects, derived from the configured language code.
pub fn expected_languages(language_code: &str) -> Option<Languages> {
    match language_code.to_lowercase().as_str() {
        "th" | "thai" => Some(Languages::Thai),
        "en" | "en-us" | "english" => Some(Languages::Latin),
        "auto" | "" => None,
        _ => None,
    }
}

/// Convenience helper: the script a language code implies.
pub fn expected_script(language_code: &str) -> Script {
    match expected_languages(language_code) {
        Some(Languages::Thai) => Script::Thai,
        Some(Languages::Latin) => Script::Latin,
        _ => Script::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty() {
        assert_eq!(check_transcript("", None), Some(RejectReason::Empty));
        assert_eq!(check_transcript("   ", None), Some(RejectReason::Empty));
    }

    #[test]
    fn rejects_known_boilerplate() {
        assert_eq!(
            check_transcript("Thank you.", None),
            Some(RejectReason::Boilerplate)
        );
        assert_eq!(
            check_transcript("Subtitles by the Amara.org community", None),
            Some(RejectReason::Boilerplate)
        );
    }

    #[test]
    fn rejects_stage_directions() {
        // Stage directions and caption boilerplate are both rejected; the
        // exact reason depends on which list matches first.
        assert!(check_transcript("(upbeat music)", None).is_some());
        assert_eq!(
            check_transcript("[BLANK_AUDIO]", None),
            Some(RejectReason::StageDirection)
        );
        assert_eq!(
            check_transcript("♪♪♪", None),
            Some(RejectReason::Boilerplate)
        );
    }

    #[test]
    fn rejects_repetition() {
        assert_eq!(
            check_transcript("the the the the the", None),
            Some(RejectReason::WordRepetition)
        );
        assert_eq!(
            check_transcript("aaaaaaaaaaaaaaaa", None),
            Some(RejectReason::CharacterRepetition)
        );
    }

    #[test]
    fn rejects_wrong_script() {
        assert_eq!(
            check_transcript("hello world", Some(Languages::Thai)),
            Some(RejectReason::ScriptMismatch)
        );
        assert_eq!(
            check_transcript("สวัสดีครับ", Some(Languages::Latin)),
            Some(RejectReason::ScriptMismatch)
        );
    }

    #[test]
    fn accepts_real_speech() {
        assert_eq!(check_transcript("สวัสดีครับ วันนี้อากาศดี", None), None);
        assert_eq!(
            check_transcript("Hello there, how are you today?", None),
            None
        );
        assert_eq!(
            check_transcript("ทดสอบใช้ Python กับ Rust", Some(Languages::Mixed)),
            None
        );
    }

    #[test]
    fn llm_result_must_stay_close_to_the_original() {
        assert_eq!(
            check_llm_result("สวัสดีครับ", "สวัสดีครับ ยินดีที่ได้รู้จัก"),
            LlmSanity::Good
        );
        assert_eq!(
            check_llm_result("สวัสดีครับ", "The quick brown fox jumps over the lazy dog"),
            LlmSanity::Drifted
        );
    }

    #[test]
    fn maps_language_codes() {
        assert_eq!(expected_languages("th"), Some(Languages::Thai));
        assert_eq!(expected_languages("en"), Some(Languages::Latin));
        assert_eq!(expected_languages("auto"), None);
    }
}
