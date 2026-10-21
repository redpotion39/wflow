//! Script detection and mixed-language (code-switching) handling.
//!
//! Thai and Latin words are frequently mixed inside one sentence
//! ("รันด้วย Python ชัดมาก"). Thai is written without spaces between words,
//! so the boundary between a Thai run and a Latin run has to be detected and
//! normalised explicitly. Knowing the script of every token also lets the
//! spell checker apply the right dictionary to the right word.

/// The writing system a character belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Thai,
    Latin,
    Digit,
    Other,
}

/// What kind of token a run of characters is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Word,
    Space,
    Punct,
}

/// A token with its script and kind resolved.
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub script: Script,
    pub text: String,
}

/// Returns the script a character belongs to.
pub fn script_of(ch: char) -> Script {
    match ch {
        '\u{0E00}'..='\u{0E7F}' => Script::Thai,
        'a'..='z' | 'A'..='Z' => Script::Latin,
        '0'..='9' => Script::Digit,
        _ => Script::Other,
    }
}

/// True when the character can be part of a word.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || matches!(script_of(ch), Script::Thai)
}

/// Splits text into word / space / punctuation tokens.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut current = String::new();
    let mut current_kind: Option<TokenKind> = None;
    let mut current_script = Script::Other;

    let flush = |tokens: &mut Vec<Token>,
                 current: &mut String,
                 kind: &mut Option<TokenKind>,
                 script: Script| {
        if !current.is_empty() {
            tokens.push(Token {
                kind: kind.unwrap_or(TokenKind::Word),
                script,
                text: std::mem::take(current),
            });
            *kind = None;
        }
    };

    for ch in text.chars() {
        let kind = if ch.is_whitespace() {
            TokenKind::Space
        } else if is_word_char(ch) {
            TokenKind::Word
        } else {
            TokenKind::Punct
        };

        match current_kind {
            Some(k) if k == kind => {
                current.push(ch);
                if kind == TokenKind::Word {
                    current_script = script_of(ch);
                }
            }
            _ => {
                flush(&mut tokens, &mut current, &mut current_kind, current_script);
                current.push(ch);
                current_kind = Some(kind);
                current_script = script_of(ch);
            }
        }
    }
    flush(&mut tokens, &mut current, &mut current_kind, current_script);

    tokens
}

/// The dominant script of a piece of text.
pub fn dominant_script(text: &str) -> Script {
    let mut thai = 0usize;
    let mut latin = 0usize;

    for ch in text.chars() {
        match script_of(ch) {
            Script::Thai => thai += 1,
            Script::Latin => latin += 1,
            _ => {}
        }
    }

    match (thai, latin) {
        (0, 0) => Script::Other,
        (_, 0) => Script::Thai,
        (0, _) => Script::Latin,
        (t, l) if t >= l => Script::Thai,
        _ => Script::Latin,
    }
}

/// The languages present in a piece of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Languages {
    Thai,
    Latin,
    Mixed,
    None,
}

impl Languages {
    /// Language codes that should be spell-checked for this text.
    pub fn codes(self) -> Vec<&'static str> {
        match self {
            Languages::Thai => vec!["th"],
            Languages::Latin => vec!["en"],
            Languages::Mixed => vec!["th", "en"],
            Languages::None => vec![],
        }
    }
}

/// Detects which languages occur in a piece of text.
pub fn detect_languages(text: &str) -> Languages {
    let mut thai = 0usize;
    let mut latin = 0usize;

    for ch in text.chars() {
        match script_of(ch) {
            Script::Thai => thai += 1,
            Script::Latin => latin += 1,
            _ => {}
        }
    }

    match (thai, latin) {
        (0, 0) => Languages::None,
        (_, 0) => Languages::Thai,
        (0, _) => Languages::Latin,
        _ => Languages::Mixed,
    }
}

/// Inserts a missing space at every Thai ↔ Latin/digit boundary.
///
/// Thai does not use spaces between words, so `ทดสอบAPI` has to become
/// `ทดสอบ API` for the text to read correctly. Boundaries next to punctuation
/// are left alone.
pub fn normalize_mixed_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev: Option<char> = None;

    for ch in text.chars() {
        if let Some(p) = prev {
            let boundary = matches!(
                (script_of(p), script_of(ch)),
                (Script::Thai, Script::Latin)
                    | (Script::Thai, Script::Digit)
                    | (Script::Latin, Script::Thai)
                    | (Script::Digit, Script::Thai)
            );
            if boundary {
                out.push(' ');
            }
        }
        out.push(ch);
        prev = Some(ch);
    }

    out
}

/// Removes spaces that were inserted between two words of the same script.
///
/// Used when re-joining text after spell correction so that Thai words do not
/// end up with a stray space in the middle.
pub fn collapse_duplicate_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = false;

    for ch in text.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }

    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_scripts() {
        assert_eq!(script_of('ก'), Script::Thai);
        assert_eq!(script_of('A'), Script::Latin);
        assert_eq!(script_of('7'), Script::Digit);
        assert_eq!(script_of('!'), Script::Other);
    }

    #[test]
    fn detects_languages() {
        assert_eq!(detect_languages("สวัสดีครับ"), Languages::Thai);
        assert_eq!(detect_languages("hello world"), Languages::Latin);
        assert_eq!(detect_languages("ใช้ Python ครับ"), Languages::Mixed);
        assert_eq!(detect_languages("123 456"), Languages::None);
    }

    #[test]
    fn inserts_missing_space_between_scripts() {
        assert_eq!(normalize_mixed_spacing("ทดสอบAPI"), "ทดสอบ API");
        assert_eq!(normalize_mixed_spacing("ใช้Pythonแล้ว"), "ใช้ Python แล้ว");
        // A Latin word followed by a digit stays together ("Python2", "COVID19").
        assert_eq!(normalize_mixed_spacing("Python2"), "Python2");
        assert_eq!(normalize_mixed_spacing("ใช้ Python2 ได้"), "ใช้ Python2 ได้");
    }

    #[test]
    fn keeps_same_script_text_untouched() {
        assert_eq!(normalize_mixed_spacing("สวัสดีครับ"), "สวัสดีครับ");
        assert_eq!(normalize_mixed_spacing("hello world"), "hello world");
        assert_eq!(normalize_mixed_spacing("a, b"), "a, b");
    }

    #[test]
    fn does_not_break_punctuation() {
        assert_eq!(normalize_mixed_spacing("ข้อความ,"), "ข้อความ,");
        assert_eq!(normalize_mixed_spacing("(test)"), "(test)");
    }

    #[test]
    fn tokenizes_words_spaces_and_punctuation() {
        let tokens = tokenize("hi ทดสอบ, ok");
        assert_eq!(tokens.len(), 6);
        assert_eq!(tokens[0].kind, TokenKind::Word);
        assert_eq!(tokens[0].script, Script::Latin);
        assert_eq!(tokens[1].kind, TokenKind::Space);
        assert_eq!(tokens[2].kind, TokenKind::Word);
        assert_eq!(tokens[2].script, Script::Thai);
        assert_eq!(tokens[3].kind, TokenKind::Punct);
        assert_eq!(tokens[5].kind, TokenKind::Word);
    }

    #[test]
    fn collapses_duplicate_spaces() {
        assert_eq!(collapse_duplicate_spaces("a   b  c "), "a b c");
    }
}
