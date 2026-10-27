//! Links in terminal output: URLs, `res://` paths and file paths, with an
//! optional `:line:column` (or `(line,column)`) after them. Ctrl+click opens
//! them; holding Ctrl underlines the one under the mouse.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermLink {
    /// An `http://` or `https://` address.
    Url(String),
    /// A `res://`, absolute or relative path, with a one-based line and
    /// column when given.
    Path { path: String, line: Option<u32>, column: Option<u32> },
}

/// The link covering `column` of `line` (one char per terminal column),
/// with the columns it spans.
pub fn link_at(line: &[char], column: usize) -> Option<(Range<usize>, TermLink)> {
    if column >= line.len() || is_separator(line[column]) {
        return None;
    }
    // The "word" around the column: up to whitespace, quotes and brackets.
    let start = line[..column].iter().rposition(|&c| is_separator(c)).map_or(0, |i| i + 1);
    let end = line[column..].iter().position(|&c| is_separator(c)).map_or(line.len(), |i| column + i);
    let mut word: String = line[start..end].iter().collect();
    let mut end = end;
    // `file.gd(12,5)`: the position in parentheses (C#, MSBuild).
    let mut position = None;
    if line.get(end) == Some(&'(') {
        let close = line[end..].iter().position(|&c| c == ')').map(|i| end + i);
        if let Some(close) = close {
            let inside: String = line[end + 1..close].iter().collect();
            let mut numbers = inside.split(',').map(|n| n.trim().parse::<u32>());
            if let Some(Ok(l)) = numbers.next() {
                position = Some((l, numbers.next().and_then(Result::ok)));
                end = close + 1;
            }
        }
    }
    // Sentence punctuation after a link isn't part of it.
    while word.ends_with(['.', ',', ';', '!', '?', ':']) {
        word.pop();
        if position.is_none() {
            end -= 1;
        }
    }
    let span = start..end;
    if word.starts_with("http://") || word.starts_with("https://") {
        return (word.len() > "https://".len()).then(|| (span, TermLink::Url(word)));
    }
    let (path, line_number, column_number) = match position {
        Some((l, c)) => (word.as_str(), Some(l), c),
        None => split_position(&word),
    };
    looks_like_path(path).then(|| {
        (span, TermLink::Path { path: path.to_string(), line: line_number, column: column_number })
    })
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '|' | '\0')
}

/// `path:12:5` → (`path`, 12, 5); a Windows drive's `C:` is part of the path.
fn split_position(word: &str) -> (&str, Option<u32>, Option<u32>) {
    let number = |s: &str| s.parse::<u32>().ok();
    let Some((head, last)) = word.rsplit_once(':').filter(|(head, last)| !head.is_empty() && number(last).is_some()) else {
        return (word, None, None);
    };
    match head.rsplit_once(':') {
        Some((path, line)) if !path.is_empty() && number(line).is_some() => (path, number(line), number(last)),
        _ => (head, number(last), None),
    }
}

/// A `res://` path, or something with a folder separator or a file
/// extension: worth checking on disk.
fn looks_like_path(path: &str) -> bool {
    if path.starts_with("res://") {
        return path.len() > "res://".len();
    }
    if path.contains("://") || path.is_empty() {
        return false;
    }
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let has_extension = file.rsplit_once('.').is_some_and(|(stem, ext)| {
        // `4.5` is a version, not a file: extensions start with a letter.
        !stem.is_empty()
            && ext.len() <= 10
            && ext.starts_with(|c: char| c.is_ascii_alphabetic())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    });
    has_extension || path.contains('/') || path.contains('\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The link under the `|` marker.
    fn at(text: &str) -> Option<(String, TermLink)> {
        let index = text.chars().position(|c| c == '|').unwrap();
        let line: Vec<char> = text.replace('|', "").chars().collect();
        link_at(&line, index).map(|(span, link)| (line[span].iter().collect(), link))
    }

    fn path(path: &str, line: Option<u32>, column: Option<u32>) -> TermLink {
        TermLink::Path { path: path.to_string(), line, column }
    }

    #[test]
    fn finds_godot_and_compiler_locations() {
        assert_eq!(
            at("   at: _ready (res://pl|ayer/player.gd:12)"),
            Some(("res://player/player.gd:12".into(), path("res://player/player.gd", Some(12), None)))
        );
        assert_eq!(at("SCRIPT ERROR: res://a.gd:3:7|: oops").unwrap().1, path("res://a.gd", Some(3), Some(7)));
        assert_eq!(at(r"error in C:\game\ma|in.gd:40").unwrap().1, path(r"C:\game\main.gd", Some(40), None));
        assert_eq!(at("src/Player.c|s(12,5): error CS0103").unwrap(), ("src/Player.cs(12,5)".into(), path("src/Player.cs", Some(12), Some(5))));
        assert_eq!(at("see ./docs/READ|ME.md.").unwrap(), ("./docs/README.md".into(), path("./docs/README.md", None, None)));
        assert_eq!(at("C:/Godot/Godot_v4|.5.exe").unwrap().1, path("C:/Godot/Godot_v4.5.exe", None, None));
    }

    #[test]
    fn finds_urls() {
        assert_eq!(
            at("Docs: https://docs.godot|engine.org/en/stable/, then"),
            Some(("https://docs.godotengine.org/en/stable/".into(), TermLink::Url("https://docs.godotengine.org/en/stable/".into())))
        );
        assert_eq!(at("<http://local|host:8080/x>").unwrap().1, TermLink::Url("http://localhost:8080/x".into()));
    }

    #[test]
    fn ignores_plain_words() {
        for text in ["hel|lo world", "PS C:\\> |", "version 4|.5", "ftp://ho|st/x", "12:3|4:56", "res:/|/"] {
            assert_eq!(at(text), None, "{text}");
        }
    }
}
