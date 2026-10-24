//! Line-based GDScript lexer for syntax highlighting.
//!
//! Lines are lexed one at a time; the only state carried between lines is an
//! open triple-quoted string. Token ranges are byte offsets into the line.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Keyword,
    ControlFlow,
    Type,
    Function,
    Annotation,
    String,
    StringName,
    NodePath,
    Number,
    Comment,
    Constant,
    Symbol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// Lexer state at the start of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineState {
    #[default]
    Normal,
    InTripleString {
        quote: u8,
        kind: TokenKind,
        raw: bool,
    },
}

const CONTROL_FLOW: &[&str] = &[
    "if", "elif", "else", "for", "while", "match", "when", "break", "continue", "pass", "return",
    "await", "breakpoint",
];

const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "class", "class_name", "const", "enum", "extends", "func", "in", "is",
    "not", "or", "preload", "self", "signal", "static", "super", "var", "void", "yield",
];

const CONSTANTS: &[&str] = &["true", "false", "null", "PI", "TAU", "INF", "NAN"];

const BUILTIN_TYPES: &[&str] = &["int", "float", "bool"];

/// Lexes one line, returning its tokens and the state for the next line.
/// Plain identifiers and whitespace produce no token.
pub fn lex_line(line: &str, state: LineState) -> (Vec<Token>, LineState) {
    let b = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    // Whether the previous token can end an expression; disambiguates `%`.
    let mut last_was_value = false;

    if let LineState::InTripleString { quote, kind, raw } = state {
        match find_triple_end(b, 0, quote, raw) {
            Some(end) => {
                push(&mut tokens, 0..end, kind);
                i = end;
                last_was_value = true;
            }
            None => {
                push(&mut tokens, 0..b.len(), kind);
                return (tokens, state);
            }
        }
    }

    while i < b.len() {
        let c = b[i];
        let start = i;

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        if c == b'#' {
            push(&mut tokens, i..b.len(), TokenKind::Comment);
            break;
        }

        // Strings, including &"StringName", ^"NodePath" and r"raw" prefixes.
        let string_prefix = match c {
            b'"' | b'\'' => Some((0, TokenKind::String, false)),
            b'&' if is_quote(b.get(i + 1)) => Some((1, TokenKind::StringName, false)),
            b'^' if is_quote(b.get(i + 1)) => Some((1, TokenKind::NodePath, false)),
            b'r' if is_quote(b.get(i + 1)) => Some((1, TokenKind::String, true)),
            b'$' if is_quote(b.get(i + 1)) => Some((1, TokenKind::NodePath, false)),
            _ => None,
        };
        if let Some((prefix_len, kind, raw)) = string_prefix {
            let (end, next_state) = lex_string(b, i + prefix_len, kind, raw);
            push(&mut tokens, start..end, kind);
            if next_state != LineState::Normal {
                return (tokens, next_state);
            }
            i = end;
            last_was_value = true;
            continue;
        }

        if c == b'@' {
            i = skip_ident(b, i + 1);
            push(&mut tokens, start..i, TokenKind::Annotation);
            last_was_value = false;
            continue;
        }

        // $Node/Path and %UniqueName (but not the `%` modulo operator).
        if c == b'$' || (c == b'%' && !last_was_value && is_ident_start(b.get(i + 1))) {
            i += 1;
            while i < b.len() && (is_ident_byte(b[i]) || b[i] == b'/') {
                i += 1;
            }
            push(&mut tokens, start..i, TokenKind::NodePath);
            last_was_value = true;
            continue;
        }

        if c.is_ascii_digit()
            || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit) && !last_was_value)
        {
            i = skip_number(b, i);
            push(&mut tokens, start..i, TokenKind::Number);
            last_was_value = true;
            continue;
        }

        if is_ident_byte(c) {
            i = skip_ident(b, i);
            let word = &line[start..i];
            let kind = classify(word, &b[i..]);
            if let Some(kind) = kind {
                push(&mut tokens, start..i, kind);
            }
            last_was_value = !matches!(kind, Some(TokenKind::Keyword | TokenKind::ControlFlow));
            continue;
        }

        i += 1;
        if c.is_ascii_punctuation() {
            push(&mut tokens, start..i, TokenKind::Symbol);
        }
        last_was_value = matches!(c, b')' | b']' | b'}');
    }

    (tokens, LineState::Normal)
}

fn push(tokens: &mut Vec<Token>, range: Range<usize>, kind: TokenKind) {
    if !range.is_empty() {
        tokens.push(Token { range, kind });
    }
}

fn classify(word: &str, rest: &[u8]) -> Option<TokenKind> {
    if CONTROL_FLOW.contains(&word) {
        return Some(TokenKind::ControlFlow);
    }
    if KEYWORDS.contains(&word) {
        return Some(TokenKind::Keyword);
    }
    if CONSTANTS.contains(&word) {
        return Some(TokenKind::Constant);
    }
    if BUILTIN_TYPES.contains(&word) {
        return Some(TokenKind::Type);
    }
    if word.starts_with(|c: char| c.is_ascii_uppercase()) {
        let all_caps = word
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        return Some(if all_caps && word.len() > 1 {
            TokenKind::Constant
        } else {
            TokenKind::Type
        });
    }
    let next = rest.iter().find(|c| !c.is_ascii_whitespace());
    if next == Some(&b'(') {
        return Some(TokenKind::Function);
    }
    None
}

/// Lexes a string whose opening quote is at `q`. Returns the end offset and
/// the state for the next line (non-normal if a triple string stays open).
fn lex_string(b: &[u8], q: usize, kind: TokenKind, raw: bool) -> (usize, LineState) {
    let quote = b[q];
    if b.get(q + 1) == Some(&quote) && b.get(q + 2) == Some(&quote) {
        return match find_triple_end(b, q + 3, quote, raw) {
            Some(end) => (end, LineState::Normal),
            None => (b.len(), LineState::InTripleString { quote, kind, raw }),
        };
    }
    let mut i = q + 1;
    while i < b.len() {
        if b[i] == b'\\' && !raw {
            i += 2;
            continue;
        }
        if b[i] == quote {
            return (i + 1, LineState::Normal);
        }
        i += 1;
    }
    // Unterminated: highlight to end of line.
    (b.len(), LineState::Normal)
}

fn find_triple_end(b: &[u8], from: usize, quote: u8, raw: bool) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        if b[i] == b'\\' && !raw {
            i += 2;
            continue;
        }
        if b[i] == quote && b.get(i + 1) == Some(&quote) && b.get(i + 2) == Some(&quote) {
            return Some(i + 3);
        }
        i += 1;
    }
    None
}

fn skip_ident(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_ident_byte(b[i]) {
        i += 1;
    }
    i
}

fn skip_number(b: &[u8], mut i: usize) -> usize {
    let hex = b.get(i) == Some(&b'0') && matches!(b.get(i + 1), Some(b'x' | b'X'));
    while i < b.len() {
        let c = b[i];
        let exponent_sign =
            matches!(c, b'+' | b'-') && !hex && matches!(b.get(i.wrapping_sub(1)), Some(b'e' | b'E'));
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || exponent_sign {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// Identifier bytes; non-ASCII bytes count so Unicode identifiers stay whole.
fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

fn is_ident_start(c: Option<&u8>) -> bool {
    c.is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_' || c >= 0x80)
}

fn is_quote(c: Option<&u8>) -> bool {
    matches!(c, Some(b'"' | b'\''))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<(&str, TokenKind)> {
        lex_line(line, LineState::Normal)
            .0
            .into_iter()
            .map(|t| (&line[t.range], t.kind))
            .collect()
    }

    #[test]
    fn function_definition() {
        assert_eq!(
            kinds("func _ready() -> void:"),
            vec![
                ("func", TokenKind::Keyword),
                ("_ready", TokenKind::Function),
                ("(", TokenKind::Symbol),
                (")", TokenKind::Symbol),
                ("-", TokenKind::Symbol),
                (">", TokenKind::Symbol),
                ("void", TokenKind::Keyword),
                (":", TokenKind::Symbol),
            ]
        );
    }

    #[test]
    fn annotations_types_and_numbers() {
        assert_eq!(
            kinds("@export var speed: float = 1.5e+3"),
            vec![
                ("@export", TokenKind::Annotation),
                ("var", TokenKind::Keyword),
                (":", TokenKind::Symbol),
                ("float", TokenKind::Type),
                ("=", TokenKind::Symbol),
                ("1.5e+3", TokenKind::Number),
            ]
        );
    }

    #[test]
    fn strings_and_comments() {
        assert_eq!(
            kinds(r#"print("a \"b\" # c") # note"#),
            vec![
                ("print", TokenKind::Function),
                ("(", TokenKind::Symbol),
                (r#""a \"b\" # c""#, TokenKind::String),
                (")", TokenKind::Symbol),
                ("# note", TokenKind::Comment),
            ]
        );
    }

    #[test]
    fn prefixed_strings() {
        assert_eq!(
            kinds(r#"&"name" ^"path" $"../Node""#),
            vec![
                (r#"&"name""#, TokenKind::StringName),
                (r#"^"path""#, TokenKind::NodePath),
                (r#"$"../Node""#, TokenKind::NodePath),
            ]
        );
    }

    #[test]
    fn node_paths_vs_modulo() {
        assert_eq!(
            kinds("$Player/Sprite2D"),
            vec![("$Player/Sprite2D", TokenKind::NodePath)]
        );
        assert_eq!(
            kinds("x = %Health"),
            vec![("=", TokenKind::Symbol), ("%Health", TokenKind::NodePath)]
        );
        assert_eq!(
            kinds("a % b"),
            vec![("%", TokenKind::Symbol)]
        );
    }

    #[test]
    fn identifier_classification() {
        assert_eq!(
            kinds("const MAX_HP = Vector2.ZERO"),
            vec![
                ("const", TokenKind::Keyword),
                ("MAX_HP", TokenKind::Constant),
                ("=", TokenKind::Symbol),
                ("Vector2", TokenKind::Type),
                (".", TokenKind::Symbol),
                ("ZERO", TokenKind::Constant),
            ]
        );
        assert_eq!(kinds("return null"), vec![
            ("return", TokenKind::ControlFlow),
            ("null", TokenKind::Constant),
        ]);
    }

    #[test]
    fn triple_string_spans_lines() {
        let (tokens, state) = lex_line(r#"var doc = """first"#, LineState::Normal);
        assert_eq!(tokens.last().unwrap().kind, TokenKind::String);
        assert!(matches!(state, LineState::InTripleString { quote: b'"', .. }));

        let (tokens, state) = lex_line("middle # not a comment", state);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::String);

        let line = r#"end""" + x"#;
        let (tokens, state) = lex_line(line, state);
        assert_eq!(&line[tokens[0].range.clone()], r#"end""""#);
        assert_eq!(state, LineState::Normal);
    }

    #[test]
    fn unicode_identifiers_keep_char_boundaries() {
        let line = "var vitesse_é = 1";
        for token in lex_line(line, LineState::Normal).0 {
            assert!(line.is_char_boundary(token.range.start));
            assert!(line.is_char_boundary(token.range.end));
        }
    }
}
