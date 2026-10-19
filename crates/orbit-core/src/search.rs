//! Search text and matching (plan §03 "Keyboard, search, and dismissal").
//!
//! [`Field`] is the overview's text input: a caret and an IME composition
//! range, with the UTF-16 offsets platform text input speaks. [`Query`]
//! ranks windows and workspaces by how well every term matches one of their
//! fields. Both are pure, so ranking and editing are tested here rather
//! than in the UI.

use std::ops::Range;

/// An editable line of text. Offsets are bytes into [`Field::text`] unless a
/// method says UTF-16.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Field {
    text: String,
    caret: usize,
    /// Text an input method is still composing.
    marked: Option<Range<usize>>,
}

impl Field {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// True while an input method is composing: Enter then belongs to it.
    pub fn composing(&self) -> bool {
        self.marked.as_ref().is_some_and(|r| !r.is_empty())
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Typed or committed text, replacing any composition.
    pub fn insert(&mut self, text: &str) {
        let range = self.marked.take().unwrap_or(self.caret..self.caret);
        self.splice(range, text);
    }

    pub fn backspace(&mut self) {
        self.marked = None;
        if let Some(previous) = self.text[..self.caret].chars().next_back() {
            let start = self.caret - previous.len_utf8();
            self.splice(start..self.caret, "");
        }
    }

    pub fn delete(&mut self) {
        self.marked = None;
        if let Some(next) = self.text[self.caret..].chars().next() {
            self.splice(self.caret..self.caret + next.len_utf8(), "");
        }
    }

    pub fn left(&mut self) {
        if let Some(previous) = self.text[..self.caret].chars().next_back() {
            self.caret -= previous.len_utf8();
        }
    }

    pub fn right(&mut self) {
        if let Some(next) = self.text[self.caret..].chars().next() {
            self.caret += next.len_utf8();
        }
    }

    pub fn home(&mut self) {
        self.caret = 0;
    }

    pub fn end(&mut self) {
        self.caret = self.text.len();
    }

    // The platform input handler's view, in UTF-16 code units.

    pub fn len_utf16(&self) -> usize {
        self.text.encode_utf16().count()
    }

    pub fn caret_utf16(&self) -> usize {
        self.utf16_at(self.caret)
    }

    pub fn marked_utf16(&self) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|r| self.utf16_at(r.start)..self.utf16_at(r.end))
    }

    pub fn text_utf16(&self, range: Range<usize>) -> (String, Range<usize>) {
        let range = self.byte_at_utf16(range.start)..self.byte_at_utf16(range.end);
        let text = self.text[range.clone()].to_owned();
        (text, self.utf16_at(range.start)..self.utf16_at(range.end))
    }

    /// Replaces `range` (default: the composition, else the caret) and ends
    /// any composition: the input method committed `text`.
    pub fn replace_utf16(&mut self, range: Option<Range<usize>>, text: &str) {
        let range = self.resolve(range);
        self.marked = None;
        self.splice(range, text);
    }

    /// Replaces `range` like [`Self::replace_utf16`], but `text` stays marked
    /// as an unfinished composition with the caret at `selected` (UTF-16,
    /// relative to `text`), or after it.
    pub fn compose_utf16(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) {
        let range = self.resolve(range);
        let start = range.start;
        self.splice(range, text);
        self.marked = (!text.is_empty()).then(|| start..start + text.len());
        if let Some(selected) = selected {
            let within: usize = text
                .chars()
                .scan(0, |units, c| {
                    *units += c.len_utf16();
                    Some((*units, c.len_utf8()))
                })
                .take_while(|(units, _)| *units <= selected.end)
                .map(|(_, bytes)| bytes)
                .sum();
            self.caret = start + within;
        }
    }

    /// The input method gave up composing; keep what it showed.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    pub fn set_caret_utf16(&mut self, at: usize) {
        self.caret = self.byte_at_utf16(at);
    }

    fn resolve(&self, range: Option<Range<usize>>) -> Range<usize> {
        match range {
            Some(r) => self.byte_at_utf16(r.start)..self.byte_at_utf16(r.end),
            None => self.marked.clone().unwrap_or(self.caret..self.caret),
        }
    }

    fn splice(&mut self, range: Range<usize>, text: &str) {
        let start = range.start.min(self.text.len());
        let end = range.end.clamp(start, self.text.len());
        self.text.replace_range(start..end, text);
        self.caret = start + text.len();
    }

    fn utf16_at(&self, byte: usize) -> usize {
        self.text[..byte.min(self.text.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    /// The byte offset of a UTF-16 offset, rounded down to a character.
    fn byte_at_utf16(&self, units: usize) -> usize {
        let mut seen = 0;
        for (byte, c) in self.text.char_indices() {
            if seen + c.len_utf16() > units {
                return byte;
            }
            seen += c.len_utf16();
        }
        self.text.len()
    }
}

/// Whitespace-separated terms, matched case-insensitively. Every term must
/// match one of a candidate's fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    terms: Vec<String>,
}

impl Query {
    pub fn parse(text: &str) -> Self {
        Self {
            terms: text.split_whitespace().map(str::to_lowercase).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Lower is better; `None` when some term matches no field.
    pub fn score(&self, fields: &[&str]) -> Option<u32> {
        let fields: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
        self.terms.iter().try_fold(0, |total, term| {
            fields
                .iter()
                .filter_map(|field| term_score(field, term))
                .min()
                .map(|score| total + score)
        })
    }
}

/// Whole field, then prefix, then the start of a word, then anywhere.
fn term_score(field: &str, term: &str) -> Option<u32> {
    if field == term {
        return Some(0);
    }
    if field.starts_with(term) {
        return Some(1);
    }
    let word_start = field.match_indices(term).any(|(at, _)| {
        field[..at]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_alphanumeric())
    });
    if word_start {
        return Some(2);
    }
    field.contains(term).then_some(3)
}

/// The name people know an app by: the last part of a reverse-DNS app id
/// (`org.gnome.Nautilus` → `Nautilus`), capitalized.
pub fn app_name(class: &str) -> String {
    let name = class.rsplit('.').find(|s| !s.is_empty()).unwrap_or(class);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_at_the_caret_by_character() {
        let mut field = Field::default();
        field.insert("héllo");
        field.left();
        field.left();
        field.backspace();
        assert_eq!(field.text(), "hélo");
        field.home();
        field.delete();
        assert_eq!(field.text(), "élo");
        field.end();
        field.insert("!");
        assert_eq!(field.text(), "élo!");
        assert_eq!(field.caret(), field.text().len());
    }

    #[test]
    fn composition_is_replaced_by_the_commit() {
        let mut field = Field::default();
        field.insert("a");
        field.compose_utf16(None, "に", None);
        assert!(field.composing());
        assert_eq!(field.marked_utf16(), Some(1..2));
        field.compose_utf16(None, "にほ", Some(2..2));
        assert_eq!(field.text(), "aにほ");
        field.replace_utf16(None, "日本");
        assert!(!field.composing());
        assert_eq!(field.text(), "a日本");
        assert_eq!(field.caret_utf16(), 3);
    }

    #[test]
    fn utf16_offsets_cover_astral_characters() {
        let mut field = Field::default();
        field.insert("a😀b");
        assert_eq!(field.len_utf16(), 4);
        assert_eq!(field.text_utf16(1..3), ("😀".to_owned(), 1..3));
        // An offset inside the surrogate pair rounds down.
        field.set_caret_utf16(2);
        assert_eq!(field.caret(), 1);
        field.replace_utf16(Some(3..4), "c");
        assert_eq!(field.text(), "a😀c");
    }

    #[test]
    fn every_term_must_match_and_better_matches_rank_first() {
        let query = Query::parse("Fire  doc");
        assert_eq!(query.score(&["firefox", "Rust docs"]), Some(1 + 2));
        assert_eq!(query.score(&["firefox", "Rust"]), None);
        let exact = Query::parse("web");
        assert!(exact.score(&["web"]) < exact.score(&["webcam"]));
        assert!(exact.score(&["my web"]) < exact.score(&["cobweb"]));
        assert!(Query::parse("  ").is_empty());
    }

    #[test]
    fn matching_ignores_case_beyond_ascii() {
        assert_eq!(Query::parse("ÉCOLE").score(&["notes école"]), Some(2));
        assert_eq!(Query::parse("日本").score(&["日本語"]), Some(1));
    }

    #[test]
    fn app_names_come_from_app_ids() {
        assert_eq!(app_name("org.gnome.Nautilus"), "Nautilus");
        assert_eq!(app_name("firefox"), "Firefox");
        assert_eq!(app_name("dev.orbit.fixture.tiled-a"), "Tiled-a");
        assert_eq!(app_name(""), "");
    }
}
