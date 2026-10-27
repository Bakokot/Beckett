//! Completion session state and client-side ranking. Godot returns the full
//! candidate list once; narrowing it as the user types happens here.

use editor_core::snippet::{self, Snippet};
use lsp_client::{CompletionItem, CompletionKind};

use crate::code_editor::CompletionRow;
use crate::docs::Doc;

/// What is completed inside a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InString {
    /// `res://` paths.
    Paths,
    /// Input Map action names.
    Actions,
}

pub struct Completion {
    /// In-flight `textDocument/completion` request.
    pub request: Option<u64>,
    /// Char index where the word being completed starts.
    pub anchor: usize,
    pub items: Vec<CompletionItem>,
    /// `items[snippets_from..]` are snippets, listed after the server's items.
    pub snippets_from: usize,
    /// Whether snippets are offered here (not after `.`, in strings, …).
    pub with_snippets: bool,
    /// Completing inside a string (`anchor` is the string's start), of
    /// what; the typed text may contain anything but quotes.
    pub in_string: Option<InString>,
    /// Indices into `items`, best first.
    pub filtered: Vec<usize>,
    pub rows: Vec<CompletionRow>,
    pub selected: usize,
    /// Docs of the selected item, once resolved.
    pub doc: Option<Doc>,
}

impl Completion {
    /// `request` is the pending server request, if any; without one the
    /// list holds only snippets.
    pub fn new(request: Option<u64>, anchor: usize, with_snippets: bool) -> Self {
        let mut completion = Self {
            request,
            anchor,
            items: Vec::new(),
            snippets_from: 0,
            with_snippets,
            in_string: None,
            filtered: Vec::new(),
            rows: Vec::new(),
            selected: 0,
            doc: None,
        };
        completion.set_items(Vec::new());
        completion
    }

    /// Completion of the `res://` paths in `files` for a string starting
    /// at `anchor`.
    pub fn paths(anchor: usize, files: Vec<String>) -> Self {
        let mut completion = Self::new(None, anchor, false);
        completion.in_string = Some(InString::Paths);
        completion.set_items(
            files
                .into_iter()
                .map(|path| CompletionItem {
                    label: path.clone(),
                    insert_text: path,
                    kind: CompletionKind::File,
                    // Known to have no docs, so none are requested.
                    documentation: Some(String::new()),
                    raw: serde_json::Value::Null,
                })
                .collect(),
        );
        completion
    }

    /// Completion of Input Map action names (with their events as docs) for
    /// a string starting at `anchor`.
    pub fn actions(anchor: usize, actions: Vec<(String, String)>) -> Self {
        let mut completion = Self::new(None, anchor, false);
        completion.in_string = Some(InString::Actions);
        completion.set_items(
            actions
                .into_iter()
                .map(|(name, events)| CompletionItem {
                    label: name.clone(),
                    insert_text: name,
                    kind: CompletionKind::Constant,
                    documentation: Some(events),
                    raw: serde_json::Value::Null,
                })
                .collect(),
        );
        completion
    }

    /// Sets the server's items, followed by the snippets.
    pub fn set_items(&mut self, mut items: Vec<CompletionItem>) {
        if self.with_snippets {
            // The `for`, `if`, `match`… snippets replace the bare keywords.
            items.retain(|i| i.kind != CompletionKind::Keyword || !snippet::GDSCRIPT.iter().any(|s| s.prefix == i.label));
        }
        self.snippets_from = items.len();
        if self.with_snippets {
            items.extend(snippet::GDSCRIPT.iter().map(snippet_item));
        }
        self.items = items;
    }

    pub fn is_snippet(&self, index: usize) -> bool {
        index >= self.snippets_from
    }

    pub fn selected_item(&self) -> Option<(usize, &CompletionItem)> {
        let index = *self.filtered.get(self.selected)?;
        Some((index, &self.items[index]))
    }
}

/// A snippet as a completion item: its body is the insert text, and its
/// docs show the first expanded line (as the signature) and description.
fn snippet_item(snippet: &Snippet) -> CompletionItem {
    let expanded = snippet::expand(snippet.body, "");
    let first_line = expanded.text.lines().next().unwrap_or_default();
    CompletionItem {
        label: snippet.prefix.to_string(),
        insert_text: snippet.body.to_string(),
        kind: CompletionKind::Other,
        documentation: Some(format!("\t{first_line}\n\n{}", snippet.description)),
        raw: serde_json::Value::Null,
    }
}

/// Text used for matching: the label up to its parameter list.
pub fn filter_text(label: &str) -> &str {
    label.split('(').next().unwrap_or(label).trim()
}

/// Ranks items against the typed prefix: exact-case prefix, prefix,
/// substring, then subsequence matches; ties keep the server's order.
pub fn rank(items: &[CompletionItem], prefix: &str) -> Vec<usize> {
    let needle = prefix.to_lowercase();
    let mut scored: Vec<(u8, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let text = filter_text(&item.label);
            let lower = text.to_lowercase();
            let score = if needle.is_empty() || text.starts_with(prefix) {
                0
            } else if lower.starts_with(&needle) {
                1
            } else if lower.contains(&needle) {
                2
            } else if is_subsequence(&needle, &lower) {
                3
            } else {
                return None;
            };
            Some((score, index))
        })
        .collect();
    scored.sort_by_key(|&(score, index)| (score, index));
    scored.into_iter().map(|(_, index)| index).collect()
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|n| chars.any(|h| h == n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn item(label: &str) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            insert_text: label.to_string(),
            kind: CompletionKind::Other,
            documentation: None,
            raw: Value::Null,
        }
    }

    #[test]
    fn ranks_prefix_then_substring_then_subsequence() {
        let items: Vec<_> = ["move_toward(…)", "length()", "length_squared()", "is_normalized()", "Length"]
            .into_iter()
            .map(item)
            .collect();
        let labels = |prefix| -> Vec<&str> {
            rank(&items, prefix).into_iter().map(|i| items[i].label.as_str()).collect()
        };
        assert_eq!(labels("len"), ["length()", "length_squared()", "Length"]);
        assert_eq!(labels("sq"), ["length_squared()"]);
        assert_eq!(labels("mvt"), ["move_toward(…)"]);
        assert_eq!(labels("").len(), 5);
    }

    #[test]
    fn snippets_follow_server_items_and_replace_their_keywords() {
        let keyword = CompletionItem { kind: CompletionKind::Keyword, ..item("for") };
        let mut c = Completion::new(Some(1), 0, true);
        c.set_items(vec![item("format"), keyword]);
        assert_eq!(c.snippets_from, 1);
        assert_eq!(c.items[0].label, "format");
        assert!(c.is_snippet(1) && c.items.iter().filter(|i| i.label == "for").count() == 1);

        let mut plain = Completion::new(Some(1), 0, false);
        plain.set_items(vec![item("format")]);
        assert_eq!(plain.items.len(), 1);
    }

    #[test]
    fn filter_text_drops_parameters() {
        assert_eq!(filter_text("angle_to(…)"), "angle_to");
        assert_eq!(filter_text("x"), "x");
    }
}
