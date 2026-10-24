//! Text search for find/replace: plain, whole-word or regex, with or
//! without case. Plain queries are escaped so all modes share one engine.

use regex::{Regex, RegexBuilder};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// A compiled query.
#[derive(Debug, Clone)]
pub struct Matcher {
    regex: Regex,
    /// Replacement templates expand `$1` only in regex mode.
    expand: bool,
}

impl Matcher {
    /// `Ok(None)` for an empty query; `Err` explains an invalid regex.
    pub fn new(query: &Query) -> Result<Option<Self>, String> {
        if query.text.is_empty() {
            return Ok(None);
        }
        let mut pattern = if query.regex {
            query.text.clone()
        } else {
            regex::escape(&query.text)
        };
        if query.whole_word {
            pattern = format!(r"\b(?:{pattern})\b");
        }
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!query.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| match e {
                regex::Error::Syntax(message) => message.lines().last().unwrap_or("invalid regex").trim().to_string(),
                other => other.to_string(),
            })?;
        Ok(Some(Self { regex, expand: query.regex }))
    }

    /// Byte ranges of all non-empty matches in `text`.
    pub fn find_all(&self, text: &str) -> Vec<(usize, usize)> {
        self.regex
            .find_iter(text)
            .filter(|m| !m.is_empty())
            .map(|m| (m.start(), m.end()))
            .collect()
    }

    /// The replacement for the match at `range` (byte offsets in `text`).
    pub fn replacement(&self, text: &str, range: (usize, usize), template: &str) -> String {
        if !self.expand {
            return template.to_string();
        }
        match self.regex.captures_at(text, range.0) {
            Some(captures) if captures.get(0).is_some_and(|m| m.start() == range.0) => {
                let mut out = String::new();
                captures.expand(template, &mut out);
                out
            }
            _ => template.to_string(),
        }
    }
}

/// One matching line, for project-wide results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// Zero-based line.
    pub line: usize,
    /// Char column of the first match on the line.
    pub column: usize,
    pub text: String,
}

/// Matching lines of `text`, at most `limit`.
pub fn matching_lines(matcher: &Matcher, text: &str, limit: usize) -> Vec<LineMatch> {
    text.lines()
        .enumerate()
        .filter_map(|(line, content)| {
            let (start, _) = matcher.find_all(content).into_iter().next()?;
            Some(LineMatch {
                line,
                column: content[..start].chars().count(),
                text: content.trim_end().to_string(),
            })
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> Query {
        Query { text: text.into(), ..Query::default() }
    }

    #[test]
    fn plain_search_is_escaped_and_case_insensitive_by_default() {
        let m = Matcher::new(&q("a.b")).unwrap().unwrap();
        assert_eq!(m.find_all("A.B axb a.b"), vec![(0, 3), (8, 11)]);
        let cased = Matcher::new(&Query { case_sensitive: true, ..q("a.b") }).unwrap().unwrap();
        assert_eq!(cased.find_all("A.B a.b"), vec![(4, 7)]);
        assert!(Matcher::new(&q("")).unwrap().is_none());
    }

    #[test]
    fn whole_word_and_regex() {
        let word = Matcher::new(&Query { whole_word: true, ..q("hp") }).unwrap().unwrap();
        assert_eq!(word.find_all("hp max_hp hp2 (hp)"), vec![(0, 2), (15, 17)]);

        let re = Matcher::new(&Query { regex: true, ..q(r"var (\w+)") }).unwrap().unwrap();
        let text = "var speed = 1";
        let found = re.find_all(text);
        assert_eq!(found, vec![(0, 9)]);
        assert_eq!(re.replacement(text, found[0], "const $1"), "const speed");

        let err = Matcher::new(&Query { regex: true, ..q("(") }).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn plain_replacement_is_literal() {
        let m = Matcher::new(&q("x")).unwrap().unwrap();
        assert_eq!(m.replacement("x", (0, 1), "$1"), "$1");
    }

    #[test]
    fn lists_matching_lines_with_char_columns() {
        let m = Matcher::new(&q("speed")).unwrap().unwrap();
        let lines = matching_lines(&m, "var é = 1\nvar éé speed\nnope\n", 10);
        assert_eq!(lines, vec![LineMatch { line: 1, column: 7, text: "var éé speed".into() }]);
    }
}
