//! The texts of the screens: the dictionary of the pages (TUI.md §1), in
//! English or Turkish.

use std::collections::HashMap;

/// The pages' dictionary, `const en = {…}` and `const tr = {…}` of one
/// `key: "text",` line each.
const SOURCE: &str = include_str!("../../apeiron-server/web/i18n.js");

/// The texts of one language, falling back to English.
pub struct Words {
    chosen: HashMap<String, String>,
    english: HashMap<String, String>,
    /// `tr` or `en`.
    pub lang: &'static str,
}

/// The entries of the object literal `const <name> = {` in `source`.
fn dictionary(source: &str, name: &str) -> HashMap<String, String> {
    let start = format!("const {name} = {{");
    let mut words = HashMap::new();
    let Some(at) = source.find(&start) else {
        return words;
    };
    for line in source[at + start.len()..].lines() {
        let line = line.trim();
        if line.starts_with("};") {
            break;
        }
        let Some((key, value)) = line.split_once(": ") else {
            continue;
        };
        let key = key.trim_matches('"');
        let value = value.trim_end_matches(',');
        if let Ok(text) = serde_json::from_str::<String>(value) {
            words.insert(key.to_owned(), text);
        }
    }
    words
}

/// `tr` when the locale variables name Turkish, as the pages follow the
/// system language.
pub fn system_lang() -> &'static str {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        .unwrap_or_default();
    if locale.to_lowercase().starts_with("tr") {
        "tr"
    } else {
        "en"
    }
}

impl Words {
    pub fn new(lang: &str) -> Self {
        let english = dictionary(SOURCE, "en");
        let (chosen, lang) = if lang == "tr" {
            (dictionary(SOURCE, "tr"), "tr")
        } else {
            (HashMap::new(), "en")
        };
        Words {
            chosen,
            english,
            lang,
        }
    }

    /// The text of `key`, or the key itself when there is none.
    pub fn get(&self, key: &str) -> String {
        self.chosen
            .get(key)
            .or_else(|| self.english.get(key))
            .cloned()
            .unwrap_or_else(|| key.to_owned())
    }

    /// Whether `key` has a text.
    pub fn has(&self, key: &str) -> bool {
        self.english.contains_key(key)
    }

    /// The text of `key` with each `{name}` replaced by its value.
    pub fn with(&self, key: &str, values: &[(&str, String)]) -> String {
        let mut text = self.get(key);
        for (name, value) in values {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_are_read_with_their_escapes() {
        let en = Words::new("en");
        let tr = Words::new("tr");
        assert_eq!(en.get("open_button"), "Open");
        assert_eq!(tr.get("open_button"), "Aç");
        assert_eq!(en.get("schedule_do_restore-test"), "restore test");
        assert!(en.get("schedule_example").contains("# [job.documents]\n"));
        assert!(en.get("schedule_example").contains("do = \"backup\""));
        assert_eq!(
            tr.with(
                "review_merge_ask",
                &[("from", "a".into()), ("into", "b".into())]
            ),
            "a, b içine birleştirilsin mi?"
        );
        assert_eq!(en.get("no_such_key"), "no_such_key");
    }

    #[test]
    fn every_text_of_the_terminal_is_in_both_languages() {
        let en = dictionary(SOURCE, "en");
        let tr = dictionary(SOURCE, "tr");
        let missing: Vec<_> = en
            .keys()
            .filter(|k| k.starts_with("tui_") && !tr.contains_key(*k))
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }
}
