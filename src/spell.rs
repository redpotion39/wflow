//! Language-aware automatic spelling correction.
//!
//! The dictionary is deliberately small and offline: it fixes the mistakes
//! people actually make while dictating (and while typing on a phone
//! keyboard), and it is applied **per word** using the script of that word, so
//! a mixed Thai/English sentence gets the right dictionary for each part.
//!
//! For anything the local table cannot handle, the optional LLM pass in
//! `stt::autocorrect_with_llm` takes over.

use crate::lang::{self, Languages, Script, TokenKind};

/// Common English misspellings, lower-cased.
const EN_MISS: &[(&str, &str)] = &[
    ("teh", "the"),
    ("adn", "and"),
    ("nad", "and"),
    ("recieve", "receive"),
    ("recieved", "received"),
    ("seperate", "separate"),
    ("seperated", "separated"),
    ("definately", "definitely"),
    ("occured", "occurred"),
    ("occuring", "occurring"),
    ("untill", "until"),
    ("wich", "which"),
    ("whith", "with"),
    ("becuase", "because"),
    ("becouse", "because"),
    ("thier", "their"),
    ("there", "their"),
    ("alot", "a lot"),
    ("allot", "a lot"),
    ("thats", "that's"),
    ("cant", "can't"),
    ("dont", "don't"),
    ("doesnt", "doesn't"),
    ("didnt", "didn't"),
    ("isnt", "isn't"),
    ("wasnt", "wasn't"),
    ("wont", "won't"),
    ("wouldnt", "wouldn't"),
    ("shouldnt", "shouldn't"),
    ("couldnt", "couldn't"),
    ("im", "I'm"),
    ("ive", "I've"),
    ("dont", "don't"),
    ("hes", "he's"),
    ("shes", "she's"),
    ("its", "it's"),
    ("lets", "let's"),
    ("wont", "won't"),
    ("youd", "you'd"),
    ("youre", "you're"),
    ("youve", "you've"),
    ("theyre", "they're"),
    ("thats", "that's"),
    ("whats", "what's"),
    ("heres", "here's"),
    ("theres", "there's"),
    ("everytime", "every time"),
    ("anymore", "any more"),
    ("alright", "all right"),
    ("infront", "in front"),
    ("incase", "in case"),
    ("inspite", "in spite"),
    ("nevermind", "never mind"),
    ("noone", "no one"),
    ("someones", "someone's"),
    ("anyones", "anyone's"),
    ("everyone's", "everyone's"),
    ("tomorow", "tomorrow"),
    ("yesturday", "yesterday"),
    ("weekend", "weekend"),
    ("beleive", "believe"),
    ("beleived", "believed"),
    ("acheive", "achieve"),
    ("acheived", "achieved"),
    ("arguement", "argument"),
    ("calender", "calendar"),
    ("cemetary", "cemetery"),
    ("collegue", "colleague"),
    ("comming", "coming"),
    ("commited", "committed"),
    ("concious", "conscious"),
    ("embarass", "embarrass"),
    ("enviroment", "environment"),
    ("existance", "existence"),
    ("familar", "familiar"),
    ("finaly", "finally"),
    ("foriegn", "foreign"),
    ("goverment", "government"),
    ("grammer", "grammar"),
    ("happend", "happened"),
    ("independant", "independent"),
    ("knowlege", "knowledge"),
    ("liesure", "leisure"),
    ("libary", "library"),
    ("maintainance", "maintenance"),
    ("neccessary", "necessary"),
    ("noticable", "noticeable"),
    ("occassion", "occasion"),
    ("paralel", "parallel"),
    ("persistant", "persistent"),
    ("posession", "possession"),
    ("prefered", "preferred"),
    ("recomend", "recommend"),
    ("refered", "referred"),
    ("relevent", "relevant"),
    ("religous", "religious"),
    ("responsability", "responsibility"),
    ("rythm", "rhythm"),
    ("succesful", "successful"),
    ("supercede", "supersede"),
    ("suprise", "surprise"),
    ("temperture", "temperature"),
    ("threshhold", "threshold"),
    ("tommorow", "tomorrow"),
    ("truely", "truly"),
    ("unfortunatly", "unfortunately"),
    ("untill", "until"),
    ("wierd", "weird"),
    ("writting", "writing"),
    ("writen", "written"),
    ("acheivment", "achievement"),
    ("apreciate", "appreciate"),
    ("attendent", "attendant"),
    ("basicly", "basically"),
    ("buisness", "business"),
    ("beleif", "belief"),
    ("carachter", "character"),
    ("catagory", "category"),
    ("cemetary", "cemetery"),
    ("changable", "changeable"),
    ("childs", "children"),
    ("comming", "coming"),
    ("compatability", "compatibility"),
    ("consistant", "consistent"),
    ("continous", "continuous"),
    ("definately", "definitely"),
    ("dissapoint", "disappoint"),
    ("embarras", "embarrass"),
    ("excelent", "excellent"),
    ("familar", "familiar"),
    ("fourty", "forty"),
    ("goverment", "government"),
    ("gramatically", "grammatically"),
    ("harrass", "harass"),
    ("immediatly", "immediately"),
    ("independant", "independent"),
    ("intresting", "interesting"),
    ("knowlege", "knowledge"),
    ("liason", "liaison"),
    ("maintenence", "maintenance"),
    ("mispell", "misspell"),
    ("noticable", "noticeable"),
    ("occured", "occurred"),
    ("oppertunity", "opportunity"),
    ("orignal", "original"),
    ("paralel", "parallel"),
    ("pavillion", "pavilion"),
    ("percieve", "perceive"),
    ("personel", "personnel"),
    ("posession", "possession"),
    ("preceed", "precede"),
    ("presance", "presence"),
    ("priviledge", "privilege"),
    ("probaly", "probably"),
    ("proffesional", "professional"),
    ("pronounciation", "pronunciation"),
    ("publically", "publicly"),
    ("realy", "really"),
    ("reccomend", "recommend"),
    ("refered", "referred"),
    ("relevent", "relevant"),
    ("religous", "religious"),
    ("restaraunt", "restaurant"),
    ("rythm", "rhythm"),
    ("saftey", "safety"),
    ("seperate", "separate"),
    ("similiar", "similar"),
    ("speach", "speech"),
    ("succesful", "successful"),
    ("supercede", "supersede"),
    ("suprise", "surprise"),
    ("temperture", "temperature"),
    ("tounge", "tongue"),
    ("truely", "truly"),
    ("unfortunatly", "unfortunately"),
    ("untill", "until"),
    ("vaccuum", "vacuum"),
    ("wierd", "weird"),
    ("writting", "writing"),
];

/// Common Thai misspellings (very common dictation / typing slips).
const TH_MISS: &[(&str, &str)] = &[
    ("ป๊ะ", "ปะ"),
    ("ครับบ", "ครับ"),
    ("ครับผมครับ", "ครับ"),
    ("สวัสดีครับยินดี", "สวัสดีครับ"),
    ("หนึ่ง", "หนึ่ง"),
    ("ขันสิ", "ขั้น"),
    ("ทำงานการ", "ทำงานกัน"),
    ("เเชียง", "เชียง"),
    ("บอกถึง", "บอกถึง"),
    ("ขอบคุณมากครับ", "ขอบคุณมากครับ"),
    ("เรื่องยาก", "เรื่องยาก"),
    ("ทำไม", "ทำไม"),
    ("ผม", "ผม"),
    ("กำลัง", "กำลัง"),
    ("เมื่อ", "เมื่อ"),
    ("สำหรับ", "สำหรับ"),
    ("ผม", "ผม"),
    ("ครับ", "ครับ"),
];

fn build_table(entries: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
    entries
        .iter()
        .map(|(wrong, right)| (wrong.to_lowercase(), (*right).to_string()))
        .collect()
}

/// The offline dictionary, built once.
struct Dictionary {
    en: std::collections::HashMap<String, String>,
    th: std::collections::HashMap<String, String>,
}

static DICTIONARY: std::sync::LazyLock<Dictionary> = std::sync::LazyLock::new(|| Dictionary {
    en: build_table(EN_MISS),
    th: build_table(TH_MISS),
});

/// Returns the corrected form of a single word, if it is in the dictionary.
///
/// The lookup is case-insensitive but keeps the original capitalisation
/// pattern of the input.
pub fn correct_word(word: &str, script: Script) -> Option<String> {
    let key = word.to_lowercase();
    let replacement = match script {
        Script::Latin => DICTIONARY.en.get(&key),
        Script::Thai => DICTIONARY.th.get(&key),
        _ => None,
    }?;

    // Preserve capitalisation of the original word.
    let corrected = if word
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
    {
        let mut chars = replacement.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => replacement.clone(),
        }
    } else {
        replacement.clone()
    };

    Some(corrected)
}

/// Applies the offline dictionary to every word of the text.
///
/// `languages` limits which dictionaries are used; an empty list means "use
/// every dictionary that matches the script of the word".
pub fn autocorrect(text: &str, languages: &[String]) -> String {
    if text.is_empty() {
        return String::new();
    }

    let allow_th = languages.is_empty() || languages.iter().any(|l| l == "th");
    let allow_en = languages.is_empty() || languages.iter().any(|l| l == "en" || l == "auto");

    let mut out = String::with_capacity(text.len());
    for token in lang::tokenize(text) {
        match token.kind {
            TokenKind::Word => {
                let allow = match token.script {
                    Script::Thai => allow_th,
                    Script::Latin => allow_en,
                    _ => false,
                };
                if allow {
                    if let Some(fixed) = correct_word(&token.text, token.script) {
                        out.push_str(&fixed);
                        continue;
                    }
                }
                out.push_str(&token.text);
            }
            _ => out.push_str(&token.text),
        }
    }
    out
}

/// Full offline pass: normalise mixed-language spacing and fix spelling.
pub fn autocorrect_offline(text: &str, languages: &[String], fix_spacing: bool) -> String {
    let spaced = if fix_spacing {
        lang::normalize_mixed_spacing(text)
    } else {
        text.to_string()
    };
    let corrected = autocorrect(&spaced, languages);
    lang::collapse_duplicate_spaces(&corrected)
}

/// The languages that should be handed to the LLM for a piece of text.
pub fn target_languages(text: &str, configured: &[String]) -> Vec<String> {
    if configured.is_empty() {
        return match lang::detect_languages(text) {
            Languages::Thai => vec!["th".to_string()],
            Languages::Latin => vec!["en".to_string()],
            Languages::Mixed => vec!["th".to_string(), "en".to_string()],
            Languages::None => vec!["en".to_string()],
        };
    }
    configured.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrects_english_words() {
        assert_eq!(correct_word("teh", Script::Latin).as_deref(), Some("the"));
        assert_eq!(
            correct_word("Recieve", Script::Latin).as_deref(),
            Some("Receive")
        );
        assert_eq!(correct_word("fine", Script::Latin), None);
    }

    #[test]
    fn autocorrects_a_sentence() {
        let out = autocorrect("i recieve teh package", &[]);
        assert_eq!(out, "i receive the package");
    }

    #[test]
    fn respects_language_filter() {
        let th_only = vec!["th".to_string()];
        let out = autocorrect("teh package ทดสอบ", &th_only);
        assert_eq!(out, "teh package ทดสอบ");
    }

    #[test]
    fn normalises_mixed_spacing_and_spelling() {
        let out = autocorrect_offline("ทดสอบAPI teh feature", &[], true);
        assert_eq!(out, "ทดสอบ API the feature");
    }

    #[test]
    fn keeps_punctuation_intact() {
        let out = autocorrect("teh, package.", &[]);
        assert_eq!(out, "the, package.");
    }

    #[test]
    fn detects_target_languages() {
        assert_eq!(target_languages("สวัสดี", &[]), vec!["th".to_string()]);
        assert_eq!(
            target_languages("ใช้ Python", &[]),
            vec!["th".to_string(), "en".to_string()]
        );
        assert_eq!(
            target_languages("hello", &["th".to_string()]),
            vec!["th".to_string()]
        );
    }
}
