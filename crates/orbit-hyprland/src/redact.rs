//! Removes window titles and other user content from recorded IPC output.
//!
//! Each redacted string becomes `<redacted len=N id=XXXXXXXX>`. The id is a
//! keyed hash that is stable within one [`Redactor`], so identical titles can
//! still be correlated inside a single recording, but the key is random per
//! process and never written out, so ids cannot be reversed by guessing.

use std::hash::{BuildHasher, RandomState};

use serde_json::Value;

/// Keys whose string values are user content. Matched case-sensitively at any
/// depth.
pub const SENSITIVE_KEYS: &[&str] = &[
    "title",
    "initialTitle",
    "lastwindowtitle",
    "xdgDescription",
    "serial",
];

pub struct Redactor {
    key: RandomState,
}

impl Default for Redactor {
    fn default() -> Self {
        Self {
            key: RandomState::new(),
        }
    }
}

impl Redactor {
    pub fn redact_str(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let id = self.key.hash_one(text) as u32;
        format!("<redacted len={} id={id:08x}>", text.chars().count())
    }

    /// Redacts sensitive keys in place, at any depth.
    pub fn redact(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, child) in map.iter_mut() {
                    if SENSITIVE_KEYS.contains(&key.as_str())
                        && let Value::String(text) = child
                    {
                        *text = self.redact_str(text);
                    } else {
                        self.redact(child);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.redact(item)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn redacts_titles_at_any_depth_and_keeps_structure() {
        let redactor = Redactor::default();
        let mut value = json!([
            {"address": "0x1", "class": "firefox", "title": "Bank — Statement", "initialTitle": "Firefox"},
            {"address": "0x2", "class": "kitty", "title": "Bank — Statement", "workspace": {"id": 2, "name": "2"}},
            {"id": 1, "lastwindowtitle": "secret.txt", "windows": 3},
        ]);
        redactor.redact(&mut value);
        let text = value.to_string();

        assert!(!text.contains("Bank"));
        assert!(!text.contains("secret"));
        assert_eq!(value[0]["class"], "firefox");
        assert_eq!(value[1]["workspace"]["name"], "2");
        assert_eq!(value[0]["title"], value[1]["title"], "same title, same id");
        assert_ne!(value[0]["title"], value[0]["initialTitle"]);
        assert!(
            value[2]["lastwindowtitle"]
                .as_str()
                .unwrap()
                .starts_with("<redacted len=10 ")
        );
    }

    #[test]
    fn empty_strings_stay_empty() {
        assert_eq!(Redactor::default().redact_str(""), "");
    }
}
