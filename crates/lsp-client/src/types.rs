//! The few LSP structures the IDE consumes, decoded from raw JSON.

use std::path::PathBuf;

use serde_json::Value;

use crate::uri::uri_to_path;

/// Zero-based line and character (Godot counts characters, not UTF-16 units).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub path: PathBuf,
    pub range: Range,
}

impl Position {
    pub fn to_json(self) -> Value {
        serde_json::json!({ "line": self.line, "character": self.character })
    }
}

/// LSP `CompletionItemKind` values the IDE distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Function,
    Variable,
    Class,
    Constant,
    Keyword,
    Signal,
    File,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub insert_text: String,
    pub kind: CompletionKind,
    /// Markdown; Godot fills it in on `completionItem/resolve`.
    pub documentation: Option<String>,
    /// The item as received, sent back verbatim to resolve it.
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignatureHelp {
    pub label: String,
    pub documentation: Option<String>,
    /// Byte range of the active parameter inside `label`.
    pub active_parameter: Option<std::ops::Range<usize>>,
}

/// Decodes a completion result: `CompletionItem[]` or `CompletionList`.
pub fn completion_items(result: &Value) -> Vec<CompletionItem> {
    let items = result.get("items").unwrap_or(result);
    items
        .as_array()
        .map(|items| items.iter().filter_map(completion_item).collect())
        .unwrap_or_default()
}

/// Decodes one item (also the answer to `completionItem/resolve`).
pub fn completion_item(item: &Value) -> Option<CompletionItem> {
    let label = item.get("label")?.as_str()?.to_string();
    let insert_text = item
        .get("insertText")
        .and_then(Value::as_str)
        .map_or_else(|| label.clone(), str::to_string);
    let kind = match item.get("kind").and_then(Value::as_f64).map(|k| k as u64) {
        Some(2..=4) => CompletionKind::Function,
        Some(5 | 6 | 10) => CompletionKind::Variable,
        Some(7 | 8 | 22) => CompletionKind::Class,
        Some(13 | 20 | 21) => CompletionKind::Constant,
        Some(14) => CompletionKind::Keyword,
        Some(23) => CompletionKind::Signal,
        Some(17 | 19) => CompletionKind::File,
        _ => CompletionKind::Other,
    };
    Some(CompletionItem {
        label,
        insert_text,
        kind,
        documentation: item.get("documentation").and_then(markup),
        raw: item.clone(),
    })
}

/// Hover contents as Markdown, if any.
pub fn hover_text(result: &Value) -> Option<String> {
    markup(result.get("contents")?).filter(|text| !text.trim().is_empty())
}

pub fn signature_help(result: &Value) -> Option<SignatureHelp> {
    let signatures = result.get("signatures")?.as_array()?;
    let index = result.get("activeSignature").and_then(Value::as_u64).unwrap_or(0) as usize;
    let signature = signatures.get(index).or(signatures.first())?;
    let label = signature.get("label")?.as_str()?.to_string();
    let active = signature
        .get("activeParameter")
        .or(result.get("activeParameter"))
        .and_then(Value::as_u64)
        .map(|i| i as usize);
    let active_parameter = active.and_then(|i| {
        let parameter = signature.get("parameters")?.as_array()?.get(i)?.get("label")?;
        match parameter {
            Value::String(name) => label.find(name.as_str()).map(|start| start..start + name.len()),
            // [start, end] in UTF-16 units; labels are ASCII in practice.
            Value::Array(bounds) => {
                let start = bounds.first()?.as_u64()? as usize;
                let end = bounds.get(1)?.as_u64()? as usize;
                (end <= label.len() && label.is_char_boundary(start) && label.is_char_boundary(end))
                    .then_some(start..end)
            }
            _ => None,
        }
    });
    Some(SignatureHelp {
        label,
        documentation: signature.get("documentation").and_then(markup),
        active_parameter,
    })
}

/// `MarkupContent`, `MarkedString`, a plain string, or an array of those.
fn markup(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(_) => value.get("value")?.as_str().map(str::to_string),
        Value::Array(parts) => {
            let parts: Vec<String> = parts.iter().filter_map(markup).collect();
            (!parts.is_empty()).then(|| parts.join("\n\n"))
        }
        _ => None,
    }
}

/// Decodes `textDocument/publishDiagnostics` params.
pub(crate) fn diagnostics(params: &Value) -> Option<(PathBuf, Vec<Diagnostic>)> {
    let path = uri_to_path(params.get("uri")?.as_str()?)?;
    let diagnostics = params
        .get("diagnostics")?
        .as_array()?
        .iter()
        .filter_map(|d| {
            Some(Diagnostic {
                range: range(d.get("range")?)?,
                severity: match d.get("severity").and_then(Value::as_u64) {
                    Some(2) => Severity::Warning,
                    Some(3) => Severity::Information,
                    Some(4) => Severity::Hint,
                    _ => Severity::Error,
                },
                message: d.get("message")?.as_str()?.to_string(),
            })
        })
        .collect();
    Some((path, diagnostics))
}

/// Decodes a definition-style result: `Location`, `Location[]` or `LocationLink[]`.
pub fn locations(result: &Value) -> Vec<Location> {
    let items = match result {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![result],
        _ => Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|item| {
            let (uri, r) = match item.get("targetUri") {
                Some(uri) => (uri, item.get("targetSelectionRange")?),
                None => (item.get("uri")?, item.get("range")?),
            };
            Some(Location {
                path: uri_to_path(uri.as_str()?)?,
                range: range(r)?,
            })
        })
        .collect()
}

fn range(v: &Value) -> Option<Range> {
    Some(Range {
        start: position(v.get("start")?)?,
        end: position(v.get("end")?)?,
    })
}

fn position(v: &Value) -> Option<Position> {
    Some(Position {
        line: u32::try_from(v.get("line")?.as_u64()?).ok()?,
        character: u32::try_from(v.get("character")?.as_u64()?).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Shapes captured from Godot 4.7.1.
    #[test]
    fn decodes_diagnostics() {
        let params = json!({
            "uri": "file:///C%3A/game/player.gd",
            "diagnostics": [
                {"code": 0, "message": "Expected parameter name.", "severity": 1, "source": "gdscript",
                 "range": {"start": {"line": 21, "character": 11}, "end": {"line": 21, "character": 12}}},
                {"code": 0, "message": "(UNUSED_SIGNAL): ...", "severity": 2, "source": "gdscript",
                 "range": {"start": {"line": 8, "character": 7}, "end": {"line": 8, "character": 11}}}
            ]
        });
        let (path, diagnostics) = diagnostics(&params).unwrap();
        assert!(path.to_string_lossy().ends_with("player.gd"));
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].range.start, Position { line: 21, character: 11 });
        assert_eq!(diagnostics[1].severity, Severity::Warning);
    }

    #[test]
    fn decodes_location_shapes() {
        let loc = json!({"uri": "file:///C%3A/game/a.gd",
            "range": {"start": {"line": 6, "character": 6}, "end": {"line": 6, "character": 12}}});
        assert_eq!(locations(&json!([loc.clone()])).len(), 1);
        assert_eq!(locations(&loc)[0].range.start.line, 6);

        let link = json!([{"targetUri": "file:///C%3A/game/b.gd",
            "targetRange": {"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 0}},
            "targetSelectionRange": {"start": {"line": 3, "character": 5}, "end": {"line": 3, "character": 9}}}]);
        assert_eq!(locations(&link)[0].range.start, Position { line: 3, character: 5 });
        assert!(locations(&Value::Null).is_empty());
    }

    #[test]
    fn decodes_completion_items() {
        let result = json!([
            {"label": "angle_to(…)", "insertText": "angle_to", "kind": 2, "data": {}},
            {"label": "x", "insertText": "x", "kind": 10},
            {"label": "died", "kind": 23}
        ]);
        let items = completion_items(&result);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].insert_text, "angle_to");
        assert_eq!(items[0].kind, CompletionKind::Function);
        assert_eq!(items[1].kind, CompletionKind::Variable);
        assert_eq!(items[2].insert_text, "died");
        assert_eq!(items[2].kind, CompletionKind::Signal);
        assert_eq!(completion_items(&json!({"items": [{"label": "a"}]})).len(), 1);

        // Resolve answers come back with float numbers.
        let resolved = json!({"label": "length()", "kind": 2.0, "documentation":
            {"kind": "markdown", "value": "\tfunc Vector2.length() -> float"}});
        let item = completion_item(&resolved).unwrap();
        assert_eq!(item.kind, CompletionKind::Function);
        assert!(item.documentation.unwrap().contains("Vector2.length"));
    }

    #[test]
    fn decodes_hover_and_signature() {
        let hover = json!({"contents": {"kind": "markdown", "value": "\tfunc move_and_slide() -> bool"}});
        assert!(hover_text(&hover).unwrap().contains("move_and_slide"));
        assert_eq!(hover_text(&json!({"contents": ""})), None);

        let help = json!({"activeParameter": 1, "activeSignature": 0, "signatures": [{
            "label": "func lerp(from: float, to: float, weight: float) -> float",
            "parameters": [{"label": "from: float"}, {"label": "to: float"}, {"label": "weight: float"}]
        }]});
        let help = signature_help(&help).unwrap();
        let active = help.active_parameter.clone().unwrap();
        assert_eq!(&help.label[active], "to: float");

        let vararg = json!({"activeParameter": 1, "signatures": [{"label": "func print(...) -> void", "parameters": []}]});
        assert_eq!(signature_help(&vararg).unwrap().active_parameter, None);
    }
}
