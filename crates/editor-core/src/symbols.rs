//! The declarations of a GDScript file (`class_name`, inner classes,
//! functions, signals, constants, variables, enums), found without the
//! language server: Godot's has no `workspace/symbol`, so the whole project
//! is searched with this instead.
//!
//! Members only: anything inside a function or another block (`if`, a
//! lambda…) is local and skipped.

use crate::gdscript::{lex_line, LineState, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Class,
    Function,
    Signal,
    Constant,
    Variable,
    Enum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub kind: DeclKind,
    /// Where the name is (zero-based line, column in chars).
    pub line: u32,
    pub column: u32,
    /// The inner classes it is declared in, joined by `.` (empty at the top).
    pub container: String,
}

/// A block opened by a line ending with `:`.
struct Scope {
    indent: usize,
    /// The class name for `class X:`, `None` for any other block.
    class: Option<String>,
}

pub fn declarations(text: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    let mut scopes: Vec<Scope> = Vec::new();
    let mut state = LineState::Normal;
    // Brackets left open by earlier lines, and the indent and class of the
    // statement they belong to (a signature over several lines).
    let mut depth = 0usize;
    let mut statement: Option<(usize, Option<String>)> = None;

    for (number, line) in text.lines().enumerate() {
        let starts_in_string = state != LineState::Normal;
        let (tokens, next) = lex_line(line, state);
        state = next;

        // The line with strings blanked out and the comment cut off, so
        // brackets and colons in them don't count. Same byte offsets.
        let mut code = line.as_bytes().to_vec();
        for token in &tokens {
            match token.kind {
                TokenKind::Comment => code.truncate(token.range.start),
                TokenKind::String | TokenKind::StringName | TokenKind::NodePath => {
                    let end = token.range.end.min(code.len());
                    code[token.range.start.min(end)..end].fill(b' ');
                }
                _ => {}
            }
        }
        let code = String::from_utf8(code).unwrap_or_default();
        let continued = starts_in_string || depth > 0;
        for c in code.bytes() {
            match c {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        let opens_block = depth == 0 && code.trim_end().ends_with(':');

        if continued {
            // The last line of a statement over several lines may open its block.
            if depth == 0 {
                if let Some((indent, class)) = statement.take() {
                    if opens_block {
                        scopes.push(Scope { indent, class });
                    }
                }
            }
            continue;
        }
        let trimmed = code.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let indent = indent_width(&code);
        while scopes.last().is_some_and(|s| indent <= s.indent) {
            scopes.pop();
        }

        let mut class = None;
        if scopes.iter().all(|s| s.class.is_some()) {
            if let Some((kind, name, offset)) = parse(trimmed) {
                let byte = code.len() - trimmed.len() + offset;
                out.push(Declaration {
                    name: name.to_string(),
                    kind,
                    line: number as u32,
                    column: line[..byte].chars().count() as u32,
                    container: scopes.iter().filter_map(|s| s.class.as_deref()).collect::<Vec<_>>().join("."),
                });
                // `class_name` names the file's class; it opens nothing.
                if kind == DeclKind::Class && trimmed.starts_with("class ") {
                    class = Some(name.to_string());
                }
            }
        }
        if opens_block {
            scopes.push(Scope { indent, class });
        } else if depth > 0 {
            statement = Some((indent, class));
        }
    }
    out
}

/// The declaration a (trimmed, comment-free) line starts with: its kind,
/// name and the name's byte offset.
fn parse(line: &str) -> Option<(DeclKind, &str, usize)> {
    let mut rest = line;
    // Annotations: `@export`, `@export_range(0, 10)`, `@onready`…
    while let Some(after) = rest.strip_prefix('@') {
        rest = after.trim_start_matches(is_ident_char).trim_start();
        if rest.starts_with('(') {
            let close = rest.find(')')?;
            rest = rest[close + 1..].trim_start();
        }
    }
    if let Some(after) = rest.strip_prefix("static ") {
        rest = after.trim_start();
    }
    let keyword_len = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
    let kind = match &rest[..keyword_len] {
        "func" => DeclKind::Function,
        "signal" => DeclKind::Signal,
        "const" => DeclKind::Constant,
        "var" => DeclKind::Variable,
        "enum" => DeclKind::Enum,
        "class" | "class_name" => DeclKind::Class,
        _ => return None,
    };
    let after = &rest[keyword_len..];
    let name_start = after.len() - after.trim_start().len();
    if name_start == 0 {
        return None;
    }
    let name = &after[name_start..];
    let name = &name[..name.find(|c: char| !is_ident_char(c)).unwrap_or(name.len())];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None; // `enum { A, B }` has no name.
    }
    let offset = line.len() - after.len() + name_start;
    Some((kind, name, offset))
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Leading whitespace width, tabs counted as 4.
fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<(String, DeclKind, String)> {
        declarations(text).into_iter().map(|d| (d.name, d.kind, d.container)).collect()
    }

    fn decl(name: &str, kind: DeclKind, container: &str) -> (String, DeclKind, String) {
        (name.to_string(), kind, container.to_string())
    }

    #[test]
    fn finds_members_and_skips_locals() {
        let text = "\
class_name Player
extends CharacterBody2D

signal died(cause: String)
const MAX_SPEED := 300.0
@export_range(0, 10) var lives := 3
@onready var sprite: Sprite2D = $Sprite
static var count := 0
enum State { IDLE, CHASE }
enum { A, B }

func _ready() -> void:
\tvar local := 1
\tif local > 0:
\t\tvar deeper := 2

static func create() -> Player:
\treturn null

class Inventory extends RefCounted:
\tvar items := []
\tfunc add_item(i) -> void:
\t\tvar n := 1
\tclass Slot:
\t\tconst SIZE = 4

var after := 1
";
        use DeclKind::*;
        assert_eq!(
            names(text),
            [
                decl("Player", Class, ""),
                decl("died", Signal, ""),
                decl("MAX_SPEED", Constant, ""),
                decl("lives", Variable, ""),
                decl("sprite", Variable, ""),
                decl("count", Variable, ""),
                decl("State", Enum, ""),
                decl("_ready", Function, ""),
                decl("create", Function, ""),
                decl("Inventory", Class, ""),
                decl("items", Variable, "Inventory"),
                decl("add_item", Function, "Inventory"),
                decl("Slot", Class, "Inventory"),
                decl("SIZE", Constant, "Inventory.Slot"),
                decl("after", Variable, ""),
            ]
        );
    }

    #[test]
    fn positions_are_char_columns_of_the_name() {
        let found = declarations("# é\n\t@export var é_speed := 1  # var not_this\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].name.as_str(), found[0].line, found[0].column), ("é_speed", 1, 13));
    }

    #[test]
    fn strings_brackets_and_lambdas_dont_confuse_it() {
        let text = "\
var doc := \"\"\"
func not_a_function():
\"\"\"
var table := {
\t\"key\": 1,
var not_a_member = 2,
}
func long(a: int,
\t\tb: int) -> void:
\tvar local := a
var callback := func():
\tvar inside := 1
func one_line(): return 1
var still_member := 1
";
        let found: Vec<String> = declarations(text).into_iter().map(|d| d.name).collect();
        assert_eq!(found, ["doc", "table", "long", "callback", "one_line", "still_member"]);
    }
}
