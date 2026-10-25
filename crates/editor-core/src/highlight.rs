//! Line lexers for the other files of a Godot project: shaders, scenes and
//! resources (and `project.godot`, `.cfg`, `.import`), JSON and Markdown.
//! Like the GDScript lexer, each works one line at a time with a small
//! state carried to the next line; token ranges are byte offsets.

use crate::gdscript::{Token, TokenKind};

/// What a line inherits from the lines above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Normal,
    /// Inside a shader `/* … */` comment.
    BlockComment,
    /// Inside a resource string that spans lines (embedded scripts).
    String,
    /// Inside a Markdown ``` code block.
    CodeFence,
}

const SHADER_KEYWORDS: &[&str] = &[
    "shader_type", "render_mode", "uniform", "varying", "const", "struct", "in", "out", "inout",
    "void", "global", "instance", "group_uniforms", "lowp", "mediump", "highp", "flat", "smooth",
];
const SHADER_CONTROL: &[&str] =
    &["if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return", "discard"];
const SHADER_TYPES: &[&str] = &[
    "bool", "bvec2", "bvec3", "bvec4", "int", "ivec2", "ivec3", "ivec4", "uint", "uvec2", "uvec3", "uvec4",
    "float", "vec2", "vec3", "vec4", "mat2", "mat3", "mat4", "sampler2D", "isampler2D", "usampler2D",
    "sampler2DArray", "isampler2DArray", "usampler2DArray", "sampler3D", "isampler3D", "usampler3D",
    "samplerCube", "samplerCubeArray", "samplerExternalOES",
];

pub fn lex_shader(line: &str, state: State) -> (Vec<Token>, State) {
    let b = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    if state == State::BlockComment {
        match line.find("*/") {
            Some(end) => {
                tokens.push(token(0, end + 2, TokenKind::Comment));
                i = end + 2;
            }
            None => return (vec![token(0, line.len(), TokenKind::Comment)], State::BlockComment),
        }
    }
    // `#include`, `#define`…: the whole line.
    if line.trim_start().starts_with('#') && i == 0 {
        return (vec![token(0, line.len(), TokenKind::Annotation)], State::Normal);
    }
    let mut after_colon = false;
    while i < b.len() {
        let c = b[i];
        if line[i..].starts_with("//") {
            tokens.push(token(i, line.len(), TokenKind::Comment));
            break;
        }
        if line[i..].starts_with("/*") {
            match line[i + 2..].find("*/") {
                Some(end) => {
                    let end = i + 2 + end + 2;
                    tokens.push(token(i, end, TokenKind::Comment));
                    i = end;
                    continue;
                }
                None => {
                    tokens.push(token(i, line.len(), TokenKind::Comment));
                    return (tokens, State::BlockComment);
                }
            }
        }
        if c == b'"' {
            let end = string_end(b, i + 1, b'"').unwrap_or(b.len());
            tokens.push(token(i, end, TokenKind::String));
            i = end;
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let end = number_end(b, i);
            tokens.push(token(i, end, TokenKind::Number));
            i = end;
            continue;
        }
        if is_ident_start(c) {
            let end = ident_end(b, i);
            let word = &line[i..end];
            let kind = if after_colon {
                // `uniform vec4 color : source_color, hint_range(0, 1);`
                Some(TokenKind::Annotation)
            } else if SHADER_CONTROL.contains(&word) {
                Some(TokenKind::ControlFlow)
            } else if SHADER_KEYWORDS.contains(&word) {
                Some(TokenKind::Keyword)
            } else if SHADER_TYPES.contains(&word) {
                Some(TokenKind::Type)
            } else if matches!(word, "true" | "false") || is_all_caps(word) {
                // Built-ins: VERTEX, UV, ALBEDO, TIME…
                Some(TokenKind::Constant)
            } else if next_non_space(b, end) == Some(b'(') {
                Some(TokenKind::Function)
            } else {
                None
            };
            if let Some(kind) = kind {
                tokens.push(token(i, end, kind));
            }
            i = end;
            continue;
        }
        match c {
            b':' => after_colon = true,
            b';' | b'{' | b'}' | b'=' => after_colon = false,
            _ => {}
        }
        if is_symbol(c) {
            tokens.push(token(i, i + 1, TokenKind::Symbol));
        }
        i += 1;
    }
    (tokens, State::Normal)
}

const RESOURCE_CONSTANTS: &[&str] = &["true", "false", "null", "inf", "nan"];

/// Scenes, resources and Godot's config files (`[section]`, `key = value`).
pub fn lex_resource(line: &str, state: State) -> (Vec<Token>, State) {
    let b = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    if state == State::String {
        match string_end(b, 0, b'"') {
            Some(end) => {
                tokens.push(token(0, end, TokenKind::String));
                i = end;
            }
            None => return (vec![token(0, line.len(), TokenKind::String)], State::String),
        }
    }
    let trimmed_start = line.len() - line.trim_start().len();
    if i == 0 && line[trimmed_start..].starts_with(';') {
        return (vec![token(trimmed_start, line.len(), TokenKind::Comment)], State::Normal);
    }
    // `[node name="Player" type="CharacterBody2D"]`: the tag is a keyword,
    // attribute names are keys.
    let in_header = i == 0 && line[trimmed_start..].starts_with('[');
    let mut at_line_start = i == 0;
    let mut header_tag_done = false;
    while i < b.len() {
        let c = b[i];
        if c == b';' && !in_header {
            tokens.push(token(i, line.len(), TokenKind::Comment));
            break;
        }
        let prefixed = matches!(c, b'&' | b'^') && b.get(i + 1) == Some(&b'"');
        if c == b'"' || prefixed {
            let start = i;
            let open = if prefixed { i + 1 } else { i };
            let kind = match c {
                b'&' => TokenKind::StringName,
                b'^' => TokenKind::NodePath,
                _ => TokenKind::String,
            };
            match string_end(b, open + 1, b'"') {
                Some(end) => {
                    tokens.push(token(start, end, kind));
                    i = end;
                }
                None => {
                    tokens.push(token(start, line.len(), kind));
                    return (tokens, State::String);
                }
            }
            at_line_start = false;
            continue;
        }
        if c.is_ascii_digit() || (c == b'-' && b.get(i + 1).is_some_and(u8::is_ascii_digit) && !prev_is_ident(b, i)) {
            let end = number_end(b, i + usize::from(c == b'-'));
            tokens.push(token(i, end, TokenKind::Number));
            i = end;
            at_line_start = false;
            continue;
        }
        if is_ident_start(c) {
            // Keys may contain `/` (`config/name`, `textures/canvas_textures/…`).
            let mut end = ident_end(b, i);
            while b.get(end) == Some(&b'/') && b.get(end + 1).is_some_and(|&n| is_ident_start(n)) {
                end = ident_end(b, end + 1);
            }
            let word = &line[i..end];
            let next = next_non_space(b, end);
            let kind = if in_header && !header_tag_done {
                header_tag_done = true;
                Some(TokenKind::Keyword)
            } else if (at_line_start || in_header) && next == Some(b'=') {
                Some(TokenKind::Function)
            } else if next == Some(b'(') {
                // Vector2(…), ExtResource("1"), PackedStringArray(…)
                Some(TokenKind::Type)
            } else if RESOURCE_CONSTANTS.contains(&word) {
                Some(TokenKind::Constant)
            } else {
                None
            };
            if let Some(kind) = kind {
                tokens.push(token(i, end, kind));
            }
            i = end;
            at_line_start = false;
            continue;
        }
        if is_symbol(c) {
            tokens.push(token(i, i + 1, TokenKind::Symbol));
        }
        if !c.is_ascii_whitespace() {
            at_line_start = false;
        }
        i += 1;
    }
    (tokens, State::Normal)
}

/// JSON: keys, strings, numbers, `true` / `false` / `null`.
pub fn lex_json(line: &str) -> Vec<Token> {
    let b = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            let end = string_end(b, i + 1, b'"').unwrap_or(b.len());
            let key = next_non_space(b, end) == Some(b':');
            tokens.push(token(i, end, if key { TokenKind::Function } else { TokenKind::String }));
            i = end;
        } else if c.is_ascii_digit() || (c == b'-' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let end = number_end(b, i + usize::from(c == b'-'));
            tokens.push(token(i, end, TokenKind::Number));
            i = end;
        } else if is_ident_start(c) {
            let end = ident_end(b, i);
            if matches!(&line[i..end], "true" | "false" | "null") {
                tokens.push(token(i, end, TokenKind::Constant));
            }
            i = end;
        } else {
            if is_symbol(c) {
                tokens.push(token(i, i + 1, TokenKind::Symbol));
            }
            i += 1;
        }
    }
    tokens
}

/// Markdown: headings, code (fenced and inline), quotes, list markers, links.
pub fn lex_markdown(line: &str, state: State) -> (Vec<Token>, State) {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        let next = if state == State::CodeFence { State::Normal } else { State::CodeFence };
        return (vec![token(indent, line.len(), TokenKind::Annotation)], next);
    }
    if state == State::CodeFence {
        return (vec![token(0, line.len(), TokenKind::String)], State::CodeFence);
    }
    if trimmed.starts_with('#') {
        return (vec![token(indent, line.len(), TokenKind::Keyword)], State::Normal);
    }
    if trimmed.starts_with('>') {
        return (vec![token(indent, line.len(), TokenKind::Comment)], State::Normal);
    }
    let b = line.as_bytes();
    let mut tokens = Vec::new();
    // List marker: "- ", "* ", "+ ", "1. ".
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    let marker = if trimmed.len() > 1 && matches!(b[indent], b'-' | b'*' | b'+') && b.get(indent + 1) == Some(&b' ') {
        1
    } else if digits > 0 && trimmed[digits..].starts_with(". ") {
        digits + 1
    } else {
        0
    };
    if marker > 0 {
        tokens.push(token(indent, indent + marker, TokenKind::Symbol));
    }
    let mut i = indent + marker;
    while i < b.len() {
        match b[i] {
            b'`' => {
                let end = line[i + 1..].find('`').map_or(b.len(), |e| i + 1 + e + 1);
                tokens.push(token(i, end, TokenKind::String));
                i = end;
            }
            // [text](url): the url is shown like a path.
            b']' if b.get(i + 1) == Some(&b'(') => {
                let end = line[i + 2..].find(')').map_or(b.len(), |e| i + 2 + e + 1);
                tokens.push(token(i + 1, end, TokenKind::NodePath));
                i = end;
            }
            b'*' | b'_' if b.get(i + 1) == Some(&b[i]) => {
                tokens.push(token(i, i + 2, TokenKind::Symbol));
                i += 2;
            }
            _ => i += 1,
        }
    }
    (tokens, State::Normal)
}

fn token(start: usize, end: usize, kind: TokenKind) -> Token {
    Token { range: start..end, kind }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn ident_end(b: &[u8], start: usize) -> usize {
    start + b[start..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count()
}

fn prev_is_ident(b: &[u8], i: usize) -> bool {
    i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b')')
}

fn number_end(b: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < b.len() {
        let c = b[i];
        let exponent_sign = matches!(c, b'-' | b'+') && i > start && matches!(b[i - 1], b'e' | b'E');
        if c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || exponent_sign {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// Index just past the closing quote (escapes skipped), if on this line.
fn string_end(b: &[u8], from: usize, quote: u8) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == quote => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn next_non_space(b: &[u8], from: usize) -> Option<u8> {
    b[from.min(b.len())..].iter().copied().find(|c| !c.is_ascii_whitespace())
}

fn is_all_caps(word: &str) -> bool {
    word.len() > 1 && word.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        && word.bytes().any(|c| c.is_ascii_uppercase())
}

fn is_symbol(c: u8) -> bool {
    b"+-*/%=<>!&|^~?:;,.()[]{}".contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (text, kind) of each token.
    fn kinds(line: &str, tokens: &[Token]) -> Vec<(String, TokenKind)> {
        tokens.iter().filter(|t| t.kind != TokenKind::Symbol).map(|t| (line[t.range.clone()].to_string(), t.kind)).collect()
    }

    #[test]
    fn shader_lines() {
        let line = "uniform vec4 tint : source_color = vec4(1.0); // note";
        let (tokens, state) = lex_shader(line, State::Normal);
        assert_eq!(state, State::Normal);
        assert_eq!(
            kinds(line, &tokens),
            [
                ("uniform".into(), TokenKind::Keyword),
                ("vec4".into(), TokenKind::Type),
                ("source_color".into(), TokenKind::Annotation),
                ("vec4".into(), TokenKind::Type),
                ("1.0".into(), TokenKind::Number),
                ("// note".into(), TokenKind::Comment),
            ]
        );
        let line = "\tALBEDO = texture(TEXTURE, UV).rgb; /* start";
        let (tokens, state) = lex_shader(line, State::Normal);
        let k = kinds(line, &tokens);
        assert_eq!(k[0], ("ALBEDO".into(), TokenKind::Constant));
        assert_eq!(k[1], ("texture".into(), TokenKind::Function));
        assert_eq!(state, State::BlockComment);
        let (tokens, state) = lex_shader("still comment */ discard;", State::BlockComment);
        assert_eq!(tokens[0].range, 0..16);
        assert_eq!(tokens[1].kind, TokenKind::ControlFlow);
        assert_eq!(state, State::Normal);
    }

    #[test]
    fn resource_lines() {
        let line = r#"[ext_resource type="Script" path="res://player.gd" id="1_abc"]"#;
        let (tokens, _) = lex_resource(line, State::Normal);
        let k = kinds(line, &tokens);
        assert_eq!(k[0], ("ext_resource".into(), TokenKind::Keyword));
        assert_eq!(k[1], ("type".into(), TokenKind::Function));
        assert_eq!(k[2], ("\"Script\"".into(), TokenKind::String));

        let line = "position = Vector2(-12.5, 3)";
        let k = kinds(line, &lex_resource(line, State::Normal).0);
        assert_eq!(k[0], ("position".into(), TokenKind::Function));
        assert_eq!(k[1], ("Vector2".into(), TokenKind::Type));
        assert_eq!(k[2], ("-12.5".into(), TokenKind::Number));

        let line = "config/name=\"Demo\" ; comment";
        let k = kinds(line, &lex_resource(line, State::Normal).0);
        assert_eq!(k[0], ("config/name".into(), TokenKind::Function));
        assert_eq!(k[2], ("; comment".into(), TokenKind::Comment));

        let line = "groups=[&\"enemies\"] path=^\"../Player\" visible=false";
        let k = kinds(line, &lex_resource(line, State::Normal).0);
        assert!(k.contains(&("&\"enemies\"".into(), TokenKind::StringName)));
        assert!(k.contains(&("^\"../Player\"".into(), TokenKind::NodePath)));
        assert!(k.contains(&("false".into(), TokenKind::Constant)));

        // Embedded script source spans lines.
        let (_, state) = lex_resource("script/source = \"extends Node", State::Normal);
        assert_eq!(state, State::String);
        let (tokens, state) = lex_resource("func f(): pass\"", State::String);
        assert_eq!((tokens[0].kind, state), (TokenKind::String, State::Normal));
    }

    #[test]
    fn json_and_markdown_lines() {
        let line = r#"  "name": "Demo", "size": -2.5e3, "ok": true"#;
        let k = kinds(line, &lex_json(line));
        assert_eq!(k[0], ("\"name\"".into(), TokenKind::Function));
        assert_eq!(k[1], ("\"Demo\"".into(), TokenKind::String));
        assert_eq!(k[3], ("-2.5e3".into(), TokenKind::Number));
        assert_eq!(k[5], ("true".into(), TokenKind::Constant));

        assert_eq!(lex_markdown("## Install", State::Normal).0[0].kind, TokenKind::Keyword);
        let (_, state) = lex_markdown("```gdscript", State::Normal);
        assert_eq!(state, State::CodeFence);
        let (tokens, state) = lex_markdown("# not a heading in code", State::CodeFence);
        assert_eq!((tokens[0].kind, state), (TokenKind::String, State::CodeFence));
        let line = "- see [docs](https://docs.godotengine.org) and `move_and_slide()`";
        let (tokens, _) = lex_markdown(line, State::Normal);
        let k: Vec<_> = tokens.iter().map(|t| (&line[t.range.clone()], t.kind)).collect();
        assert_eq!(k[0], ("-", TokenKind::Symbol));
        assert_eq!(k[1], ("(https://docs.godotengine.org)", TokenKind::NodePath));
        assert_eq!(k[2], ("`move_and_slide()`", TokenKind::String));
    }
}
