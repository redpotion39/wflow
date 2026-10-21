//! Small text helpers shared by the dictation pipeline and the GUI.

/// Case-insensitive substring replacement that respects UTF-8 character
/// boundaries. When the lowercased text changes length (rare, locale specific)
/// it falls back to a plain case-sensitive replacement.
pub fn replace_case_insensitive(text: &str, find: &str, replace_with: &str) -> String {
    if find.is_empty() || text.is_empty() {
        return text.to_string();
    }
    let lower_find: Vec<char> = find.to_lowercase().chars().collect();
    let chars: Vec<char> = text.chars().collect();
    let lower_chars: Vec<char> = text.to_lowercase().chars().collect();

    if chars.len() != lower_chars.len() {
        return text.replace(find, replace_with);
    }

    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + lower_find.len() <= lower_chars.len()
            && lower_chars[i..i + lower_find.len()] == lower_find[..]
        {
            result.push_str(replace_with);
            i += lower_find.len();
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_ascii_case_insensitively() {
        let input = "Hello BRB see you later";
        let output = replace_case_insensitive(input, "brb", "be right back");
        assert_eq!(output, "Hello be right back see you later");
    }

    #[test]
    fn replaces_thai_without_breaking_clusters() {
        let input = "สวัสดีครับทีมงานทุกท่าน";
        let output = replace_case_insensitive(input, "ทีมงาน", "ทีมวิศวกร Wflow");
        assert_eq!(output, "สวัสดีครับทีมวิศวกร Wflowทุกท่าน");
    }

    #[test]
    fn empty_needle_is_a_noop() {
        assert_eq!(replace_case_insensitive("abc", "", "x"), "abc");
    }
}
