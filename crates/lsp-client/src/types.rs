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
}
