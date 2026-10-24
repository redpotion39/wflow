use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the Thai font the GUI actually loaded contains the WTT 2.0 PUA
/// glyphs (0xF700..0xF71A) that `shape_thai` maps vowels and tone marks onto.
///
/// Only the TLWG fonts (Garuda, Waree, Laksaman, ...) ship those glyphs.
/// Windows Thai faces (Leelawadee UI, Tahoma) do not, and neither does the
/// common Linux default that `fc-match :lang=th` hands back — Noto Sans Thai.
/// Shaping onto a codepoint the font does not have paints a tofu box, which is
/// how the interface ended up in "alien language" in the first place.
///
/// The GUI sets this once, after it has picked a font, by inspecting the real
/// font file. While it is false `shape_thai` leaves the text untouched and the
/// font's own metrics do the work.
static PUA_SHAPING_SUPPORTED: AtomicBool = AtomicBool::new(false);

/// Records whether the active Thai face can render the PUA shaping glyphs.
pub fn set_pua_support(supported: bool) {
    PUA_SHAPING_SUPPORTED.store(supported, Ordering::SeqCst);
}

/// True when `shape_thai` may safely substitute PUA codepoints.
pub fn pua_supported() -> bool {
    PUA_SHAPING_SUPPORTED.load(Ordering::SeqCst)
}

/// Checks if a character is a Thai consonant (ก..ฮ, 0x0E01..=0x0E2E).
#[inline]
pub fn is_thai_consonant(c: char) -> bool {
    ('\u{0E01}'..='\u{0E2E}').contains(&c)
}

/// Checks if a consonant has a tall ascender (ป, ฝ, ฟ, ฬ).
#[inline]
pub fn is_tall_consonant(c: char) -> bool {
    matches!(c, '\u{0E1B}' | '\u{0E1D}' | '\u{0E1F}' | '\u{0E2C}')
}

/// Checks if a consonant has a removable pedestal (ฐ, ญ).
#[inline]
pub fn is_removable_pedestal_consonant(c: char) -> bool {
    matches!(c, '\u{0E10}' | '\u{0E0D}')
}

/// Checks if a consonant has a strict bottom descender (ฎ, ฏ).
#[inline]
pub fn is_strict_descender_consonant(c: char) -> bool {
    matches!(c, '\u{0E0E}' | '\u{0E0F}')
}

/// Checks if a character is an upper vowel / Level 1 above mark:
/// ั (0x0E31), ิ (0x0E34), ี (0x0E35), ึ (0x0E36), ื (0x0E37), ็ (0x0E47), ํ (0x0E4D).
#[inline]
pub fn is_upper_vowel(c: char) -> bool {
    matches!(
        c,
        '\u{0E31}' | '\u{0E34}'..='\u{0E37}' | '\u{0E47}' | '\u{0E4D}'
    )
}

/// Checks if a character is a lower vowel / Level 1 below mark:
/// ุ (0x0E38), ู (0x0E39), ฺ (0x0E3A).
#[inline]
pub fn is_lower_vowel(c: char) -> bool {
    matches!(c, '\u{0E38}'..='\u{0E3A}')
}

/// Checks if a character is a Thai tone mark or Karan diacritic:
/// ่ (0x0E48), ้ (0x0E49), ๊ (0x0E4A), ๋ (0x0E4B), ์ (0x0E4C).
#[inline]
pub fn is_tone_mark(c: char) -> bool {
    matches!(c, '\u{0E48}'..='\u{0E4C}')
}

/// Shifts upper vowel to the left for tall consonants (ป, ฝ, ฟ, ฬ).
#[inline]
fn shift_left_upper_vowel(c: char) -> char {
    match c {
        '\u{0E31}' => '\u{F710}', // MAI HAN-AKAT left
        '\u{0E34}' => '\u{F701}', // SARA I left
        '\u{0E35}' => '\u{F702}', // SARA II left
        '\u{0E36}' => '\u{F703}', // SARA UE left
        '\u{0E37}' => '\u{F704}', // SARA UEE left
        '\u{0E4D}' => '\u{F711}', // NIKHAHIT left
        '\u{0E47}' => '\u{F712}', // MAITAIKHU left
        _ => c,
    }
}

/// Tone mark lowered to Level 1 (when NO upper vowel) for normal consonants.
/// This eliminates floating tone marks ("สระลอย").
#[inline]
fn lower_tone_mark(c: char) -> char {
    match c {
        '\u{0E48}' => '\u{F70A}', // MAI EK low
        '\u{0E49}' => '\u{F70B}', // MAI THO low
        '\u{0E4A}' => '\u{F70C}', // MAI TRI low
        '\u{0E4B}' => '\u{F70D}', // MAI CHATTAWA low
        '\u{0E4C}' => '\u{F70E}', // THANTHAKHAT low
        _ => c,
    }
}

/// Tone mark lowered and shifted left (when NO upper vowel) for tall consonants (ป, ฝ, ฟ, ฬ).
#[inline]
fn lower_and_shift_left_tone_mark(c: char) -> char {
    match c {
        '\u{0E48}' => '\u{F705}', // MAI EK low left
        '\u{0E49}' => '\u{F706}', // MAI THO low left
        '\u{0E4A}' => '\u{F707}', // MAI TRI low left
        '\u{0E4B}' => '\u{F708}', // MAI CHATTAWA low left
        '\u{0E4C}' => '\u{F709}', // THANTHAKHAT low left
        _ => c,
    }
}

/// Tone mark shifted left at Level 2 (when upper vowel IS present) for tall consonants (ป, ฝ, ฟ, ฬ).
#[inline]
fn shift_left_level2_tone_mark(c: char) -> char {
    match c {
        '\u{0E48}' => '\u{F713}', // MAI EK left
        '\u{0E49}' => '\u{F714}', // MAI THO left
        '\u{0E4A}' => '\u{F715}', // MAI TRI left
        '\u{0E4B}' => '\u{F716}', // MAI CHATTAWA left
        '\u{0E4C}' => '\u{F717}', // THANTHAKHAT left
        _ => c,
    }
}

/// Lower vowel lowered below ฎ, ฏ.
#[inline]
fn lower_lower_vowel(c: char) -> char {
    match c {
        '\u{0E38}' => '\u{F718}', // SARA U low
        '\u{0E39}' => '\u{F719}', // SARA UU low
        '\u{0E3A}' => '\u{F71A}', // PHINTHU low
        _ => c,
    }
}

/// Shapes Thai text by mapping combining tone marks, vowels, and base consonants
/// to WTT 2.0 / Windows PUA codepoints (0xF700..0xF71A).
///
/// This eliminates floating tone marks ("สระลอย"), ascender collisions with
/// tall consonants (ป, ฝ, ฟ, ฬ), and pedestal/descender collisions (ฐ, ญ, ฎ, ฏ)
/// when rendering in graphical toolkits lacking HarfBuzz OpenType shaping (like egui).
///
/// The substitution is only applied when the font that is actually drawing the
/// text owns those glyphs. Where it does not — Noto Sans Thai, Leelawadee UI,
/// Tahoma — the raw Unicode text is returned untouched, because mapping a
/// vowel onto a missing glyph paints a tofu box and loses the word entirely.
pub fn shape_thai(text: &str) -> Cow<'_, str> {
    if pua_supported() {
        shape_thai_unchecked(text)
    } else {
        Cow::Borrowed(text)
    }
}

/// The shaping itself, with no capability check.
///
/// Kept separate so the mapping tables can be unit-tested without mutating
/// process-wide state, which would make the suite order-dependent.
fn shape_thai_unchecked(text: &str) -> Cow<'_, str> {
    if !text.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c)) {
        return Cow::Borrowed(text);
    }

    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0;

    while i < n {
        let c = chars[i];
        if is_thai_consonant(c) {
            let base_cons = c;
            let mut lower_v = None;
            let mut upper_v = None;
            let mut tone_m = None;
            let mut has_sara_am = false;

            let mut j = i + 1;
            while j < n {
                let next_c = chars[j];
                if is_lower_vowel(next_c) && lower_v.is_none() {
                    lower_v = Some(next_c);
                    j += 1;
                } else if is_upper_vowel(next_c) && upper_v.is_none() {
                    upper_v = Some(next_c);
                    j += 1;
                } else if is_tone_mark(next_c) && tone_m.is_none() {
                    tone_m = Some(next_c);
                    j += 1;
                } else if next_c == '\u{0E33}' {
                    has_sara_am = true;
                    j += 1;
                    break;
                } else {
                    break;
                }
            }

            // 1. Base consonant adjustment (remove pedestal for ฐ, ญ when followed by lower vowel)
            let out_cons = if lower_v.is_some() && is_removable_pedestal_consonant(base_cons) {
                if base_cons == '\u{0E10}' {
                    '\u{F700}' // descless ฐ
                } else {
                    '\u{F70F}' // descless ญ
                }
            } else {
                base_cons
            };
            out.push(out_cons);

            // 2. Lower vowel adjustment (shift down below ฎ, ฏ)
            if let Some(lv) = lower_v {
                let out_lv = if is_strict_descender_consonant(base_cons) {
                    lower_lower_vowel(lv)
                } else {
                    lv
                };
                out.push(out_lv);
            }

            // 3. Sara Am (0x0E33) handling
            if has_sara_am {
                if tone_m.is_none() && !is_tall_consonant(base_cons) {
                    // Simple consonant + Sara Am (e.g. คำ, ทำ)
                    out.push('\u{0E33}');
                } else {
                    // Decompose into Nikhahit + tone mark + Sara Aa
                    let nikhahit = if is_tall_consonant(base_cons) {
                        '\u{F711}' // NIKHAHIT left
                    } else {
                        '\u{0E4D}' // standard NIKHAHIT
                    };
                    out.push(nikhahit);

                    if let Some(tm) = tone_m {
                        let out_tm = if is_tall_consonant(base_cons) {
                            shift_left_level2_tone_mark(tm)
                        } else {
                            tm // standard Level 2 tone mark
                        };
                        out.push(out_tm);
                    }
                    out.push('\u{0E32}'); // SARA AA
                }
            } else {
                // 4. Upper vowel adjustment
                if let Some(uv) = upper_v {
                    let out_uv = if is_tall_consonant(base_cons) {
                        shift_left_upper_vowel(uv)
                    } else {
                        uv
                    };
                    out.push(out_uv);
                }

                // 5. Tone mark adjustment
                if let Some(tm) = tone_m {
                    let out_tm = if upper_v.is_some() {
                        // Level 2 tone mark (above upper vowel)
                        if is_tall_consonant(base_cons) {
                            shift_left_level2_tone_mark(tm)
                        } else {
                            tm // standard Level 2
                        }
                    } else {
                        // Level 1 tone mark (NO upper vowel -> fixes "สระลอย")
                        if is_tall_consonant(base_cons) {
                            lower_and_shift_left_tone_mark(tm)
                        } else {
                            lower_tone_mark(tm)
                        }
                    };
                    out.push(out_tm);
                }
            }

            i = j;
        } else {
            out.push(c);
            i += 1;
        }
    }

    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mapping tables are exercised through `shape_thai_unchecked`, so no
    /// test has to mutate the process-wide capability flag.
    fn shaped(text: &str) -> Cow<'_, str> {
        shape_thai_unchecked(text)
    }

    #[test]
    fn test_non_thai_passthrough() {
        assert_eq!(shaped("Hello World 123!"), "Hello World 123!");
        assert_eq!(shaped("日本語テスト"), "日本語テスト");
    }

    #[test]
    fn shaping_is_skipped_when_the_font_lacks_the_glyphs() {
        // This gate is what keeps Noto Sans Thai and Leelawadee UI readable:
        // without it every vowel and tone mark turns into a tofu box.
        set_pua_support(false);
        assert_eq!(shape_thai("ก่"), "ก่");

        set_pua_support(true);
        assert_ne!(shape_thai("ก่"), "ก่");
        set_pua_support(false);
    }

    #[test]
    fn test_lowered_tone_no_floating() {
        // ก่ -> ก + ่ (lowered to 0xF70A)
        let out = shaped("ก่");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(codepoints, vec![0x0E01, 0xF70A]);
    }

    #[test]
    fn test_tall_consonant_lowered_left() {
        // ป่ -> ป + ่ (lowered and shifted left to 0xF705)
        let out = shaped("ป่");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(codepoints, vec![0x0E1B, 0xF705]);
    }

    #[test]
    fn test_upper_vowel_and_level2_tone() {
        // กิ่ง -> ก + ิ + ่ (level 2) + ง
        let out = shaped("กิ่ง");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(codepoints, vec![0x0E01, 0x0E34, 0x0E48, 0x0E07]);
    }

    #[test]
    fn test_tall_upper_vowel_and_tone() {
        // ปิ้ง -> ป + ิ(left) + ้(left level 2) + ง
        let out = shaped("ปิ้ง");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(codepoints, vec![0x0E1B, 0xF701, 0xF714, 0x0E07]);
    }

    #[test]
    fn test_phu_yai() {
        // ผู้ใหญ่ -> ผ + ู + ้ (lowered) + ใ + ห + ญ + ่ (lowered)
        let out = shaped("ผู้ใหญ่");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(
            codepoints,
            vec![0x0E1C, 0x0E39, 0xF70B, 0x0E43, 0x0E2B, 0x0E0D, 0xF70A]
        );
    }

    #[test]
    fn test_sara_am_decomposition() {
        // น้ำ -> น + ํ (0x0E4D) + ้ (0x0E49) + า (0x0E32)
        let out = shaped("น้ำ");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(codepoints, vec![0x0E19, 0x0E4D, 0x0E49, 0x0E32]);
    }

    #[test]
    fn test_pedestal_removal() {
        // ฐุ -> ฐ(descless 0xF700) + ุ
        let out = shaped("ฐุ");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(codepoints, vec![0xF700, 0x0E38]);

        // ญุ -> ญ(descless 0xF70F) + ุ
        let out = shaped("ญุ");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(codepoints, vec![0xF70F, 0x0E38]);
    }

    #[test]
    fn test_strict_descender() {
        // ฎุ -> ฎ + ุ(lowered 0xF718)
        let out = shaped("ฎุ");
        let codepoints: Vec<u32> = out.chars().map(|c| c as u32).collect();
        assert_eq!(codepoints, vec![0x0E0E, 0xF718]);
    }
}
