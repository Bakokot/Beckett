//! Completion session state and client-side ranking. Godot returns the full
//! candidate list once; narrowing it as the user types happens here.

use lsp_client::CompletionItem;

use crate::code_editor::CompletionRow;
use crate::docs::Doc;

pub struct Completion {
    /// In-flight `textDocument/completion` request.
    pub request: Option<u64>,
    /// Char index where the word being completed starts.
    pub anchor: usize,
    pub items: Vec<CompletionItem>,
    /// Indices into `items`, best first.
    pub filtered: Vec<usize>,
    pub rows: Vec<CompletionRow>,
    pub selected: usize,
    /// Docs of the selected item, once resolved.
    pub doc: Option<Doc>,
}

impl Completion {
    pub fn new(request: u64, anchor: usize) -> Self {
        Self {
            request: Some(request),
            anchor,
            items: Vec::new(),
            filtered: Vec::new(),
            rows: Vec::new(),
            selected: 0,
            doc: None,
        }
    }

    pub fn selected_item(&self) -> Option<(usize, &CompletionItem)> {
        let index = *self.filtered.get(self.selected)?;
        Some((index, &self.items[index]))
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
    use lsp_client::CompletionKind;
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
    fn filter_text_drops_parameters() {
        assert_eq!(filter_text("angle_to(…)"), "angle_to");
        assert_eq!(filter_text("x"), "x");
    }
}
