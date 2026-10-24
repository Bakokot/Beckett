//! Text buffer: a rope with multiple selections, change sets and grouped undo.
//!
//! Positions are char indices. Every edit is a list of [`Change`]s in
//! pre-edit coordinates, sorted and non-overlapping, so all selections can be
//! mapped through it and it can be inverted for undo.

use std::cell::RefCell;

use ropey::Rope;

use crate::gdscript::{self, LineState, Token};
use crate::search::Matcher;
use crate::Language;

pub const TAB_WIDTH: usize = 4;

/// A zero-based line and char column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    /// Visual column kept across vertical moves.
    goal: Option<usize>,
}

impl Selection {
    pub fn cursor(pos: usize) -> Self {
        Self { anchor: pos, head: pos, goal: None }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head, goal: None }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// Replace `start..end` (pre-edit char indices) with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    /// Toggles between the first non-blank character and column 0.
    Home,
    End,
    PageUp(usize),
    PageDown(usize),
    DocumentStart,
    DocumentEnd,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Move { motion: Motion, extend: bool },
    Click { pos: Pos, extend: bool },
    AddCursorAt(Pos),
    Drag(Pos),
    SelectWordAt(Pos),
    SelectLineAt(Pos),
    SelectAll,
    /// Drops all but the primary selection.
    CollapseSelections,
    AddCursorAbove,
    AddCursorBelow,
    SelectNextOccurrence,
    /// Typed text; single characters get bracket/quote auto-closing.
    Insert(String),
    /// At every cursor: delete `delete` chars before it, insert `text`, then
    /// move back `cursor_back` chars (accepting a completion).
    ReplaceBefore { delete: usize, text: String, cursor_back: usize },
    Paste(String),
    Newline,
    Backspace { word: bool },
    Delete { word: bool },
    DeleteSelection,
    Indent,
    Outdent,
    ToggleComment,
    MoveLinesUp,
    MoveLinesDown,
    DuplicateLines,
    DeleteLines,
    Undo,
    Redo,
    /// Select exact char ranges (e.g. search matches); `primary` indexes them.
    Select { ranges: Vec<(usize, usize)>, primary: usize },
    /// Replace char ranges (sorted, non-overlapping) as one undo step.
    Replace(Vec<(usize, usize, String)>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

struct Entry {
    forward: Vec<Change>,
    inverse: Vec<Change>,
    before: Vec<Selection>,
    after: Vec<Selection>,
}

pub struct Buffer {
    rope: Rope,
    /// Sorted by start, non-overlapping, never empty.
    selections: Vec<Selection>,
    primary: usize,
    undo: Vec<Vec<Entry>>,
    redo: Vec<Vec<Entry>>,
    /// Kind of the last edit, for undo grouping; reset by non-edit commands.
    last_kind: Option<EditKind>,
    version: u64,
    reveal: u64,
    crlf: bool,
    /// The file started with a UTF-8 byte order mark (kept out of the text).
    bom: bool,
    language: Language,
    /// Lexer state at the start of each line, valid for a prefix of lines.
    line_states: RefCell<Vec<LineState>>,
}

const PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\'')];
const BOM: char = '\u{feff}';

impl Buffer {
    /// Builds a buffer from file contents; a leading byte order mark is
    /// stripped and restored by [`Buffer::file_contents`].
    pub fn new(text: &str, language: Language) -> Self {
        let bom = text.starts_with(BOM);
        let text = text.strip_prefix(BOM).unwrap_or(text);
        let crlf = text.contains("\r\n");
        let rope = if crlf {
            Rope::from_str(&text.replace("\r\n", "\n"))
        } else {
            Rope::from_str(text)
        };
        Self {
            rope,
            selections: vec![Selection::cursor(0)],
            primary: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            last_kind: None,
            version: 0,
            reveal: 0,
            crlf,
            bom,
            language,
            line_states: RefCell::new(vec![LineState::Normal]),
        }
    }

    // ---- Queries ----

    /// Full text, with the file's original line endings (no byte order mark).
    pub fn text(&self) -> String {
        let text = self.rope.to_string();
        if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text
        }
    }

    /// What to write back to disk: the text, plus the byte order mark if
    /// the file had one.
    pub fn file_contents(&self) -> String {
        if self.bom {
            format!("{BOM}{}", self.text())
        } else {
            self.text()
        }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// Bumped on every content change.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Bumped whenever the view should scroll the primary cursor into view.
    pub fn reveal_version(&self) -> u64 {
        self.reveal
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Line contents without the line break.
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.len_lines() {
            return String::new();
        }
        let mut text = self.rope.line(line).to_string();
        while text.ends_with(['\n', '\r']) {
            text.pop();
        }
        text
    }

    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.len_lines() {
            return 0;
        }
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        while len > 0 && matches!(slice.char(len - 1), '\n' | '\r') {
            len -= 1;
        }
        len
    }

    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }

    pub fn primary(&self) -> Selection {
        self.selections[self.primary]
    }

    pub fn char_to_pos(&self, index: usize) -> Pos {
        let index = index.min(self.len_chars());
        let line = self.rope.char_to_line(index);
        Pos { line, column: index - self.rope.line_to_char(line) }
    }

    pub fn pos_to_char(&self, pos: Pos) -> usize {
        let line = pos.line.min(self.len_lines() - 1);
        self.rope.line_to_char(line) + pos.column.min(self.line_len(line))
    }

    /// Display column of a char column, with tabs expanded.
    pub fn visual_col(&self, line: usize, column: usize) -> usize {
        if line >= self.len_lines() {
            return 0;
        }
        self.rope
            .line(line)
            .chars()
            .take(column)
            .fold(0, |col, c| col + char_width(c, col))
    }

    /// The last char column whose display column is at most `visual`.
    pub fn column_for_visual(&self, line: usize, visual: usize) -> usize {
        let mut col = 0;
        for (column, c) in self.rope.line(line.min(self.len_lines() - 1)).chars().enumerate() {
            if matches!(c, '\n' | '\r') {
                return column;
            }
            let next = col + char_width(c, col);
            if next > visual {
                return column;
            }
            col = next;
        }
        self.line_len(line)
    }

    /// Text of all selections, one per line.
    pub fn selected_text(&self) -> String {
        self.selections
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| self.rope.slice(s.start()..s.end()).to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Syntax tokens of one line (empty for plain text).
    pub fn tokens(&self, line: usize) -> Vec<Token> {
        if self.language != Language::GDScript || line >= self.len_lines() {
            return Vec::new();
        }
        let mut states = self.line_states.borrow_mut();
        while states.len() <= line {
            let i = states.len() - 1;
            let (_, next) = gdscript::lex_line(&self.line_text(i), states[i]);
            states.push(next);
        }
        gdscript::lex_line(&self.line_text(line), states[line]).0
    }

    /// The bracket pair touching `index` (before or after it), if balanced.
    pub fn matching_bracket(&self, index: usize) -> Option<(usize, usize)> {
        const LIMIT: usize = 20_000;
        let candidates = [index.checked_sub(1), Some(index)];
        for at in candidates.into_iter().flatten() {
            if at >= self.len_chars() {
                continue;
            }
            let c = self.rope.char(at);
            if let Some(&(open, close)) = PAIRS[..3].iter().find(|(o, _)| *o == c) {
                let mut depth = 0usize;
                for (i, ch) in self.rope.chars_at(at).enumerate().take(LIMIT) {
                    if ch == open {
                        depth += 1;
                    } else if ch == close {
                        depth -= 1;
                        if depth == 0 {
                            return Some((at, at + i));
                        }
                    }
                }
            } else if let Some(&(open, close)) = PAIRS[..3].iter().find(|(_, cl)| *cl == c) {
                let mut depth = 0usize;
                let mut chars = self.rope.chars_at(at + 1);
                let mut i = at + 1;
                while let Some(ch) = chars.prev() {
                    i -= 1;
                    if at - i > LIMIT {
                        break;
                    }
                    if ch == close {
                        depth += 1;
                    } else if ch == open {
                        depth -= 1;
                        if depth == 0 {
                            return Some((i, at));
                        }
                    }
                }
            }
        }
        None
    }

    // ---- Commands ----

    /// Places a single cursor (e.g. after go to definition) and reveals it.
    pub fn set_cursor(&mut self, pos: Pos) {
        let index = self.pos_to_char(pos);
        self.set_selections(vec![Selection::cursor(index)], 0);
    }

    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Move { motion, extend } => {
                let selections = self
                    .selections
                    .iter()
                    .map(|&sel| self.moved(sel, motion, extend))
                    .collect();
                self.set_selections(selections, self.primary);
            }
            Command::Click { pos, extend } => {
                let head = self.pos_to_char(pos);
                let anchor = if extend { self.primary().anchor } else { head };
                self.set_selections(vec![Selection::new(anchor, head)], 0);
            }
            Command::AddCursorAt(pos) => {
                let mut selections = self.selections.clone();
                selections.push(Selection::cursor(self.pos_to_char(pos)));
                let primary = selections.len() - 1;
                self.set_selections(selections, primary);
            }
            Command::Drag(pos) => {
                let mut selections = self.selections.clone();
                selections[self.primary].head = self.pos_to_char(pos);
                self.set_selections(selections, self.primary);
            }
            Command::SelectWordAt(pos) => {
                let (start, end) = self.word_at(self.pos_to_char(pos));
                self.set_selections(vec![Selection::new(start, end)], 0);
            }
            Command::SelectLineAt(pos) => {
                let line = pos.line.min(self.len_lines() - 1);
                let start = self.rope.line_to_char(line);
                let end = self.line_start(line + 1);
                self.set_selections(vec![Selection::new(start, end)], 0);
            }
            Command::SelectAll => {
                self.set_selections(vec![Selection::new(0, self.len_chars())], 0);
            }
            Command::CollapseSelections => {
                self.set_selections(vec![self.primary()], 0);
            }
            Command::AddCursorAbove | Command::AddCursorBelow => {
                let up = command == Command::AddCursorAbove;
                let primary = self.primary();
                let pos = self.char_to_pos(primary.head);
                if (up && pos.line == 0) || (!up && pos.line + 1 >= self.len_lines()) {
                    return;
                }
                let goal = primary.goal.unwrap_or(self.visual_col(pos.line, pos.column));
                let line = if up { pos.line - 1 } else { pos.line + 1 };
                let column = self.column_for_visual(line, goal);
                let mut cursor = Selection::cursor(self.pos_to_char(Pos { line, column }));
                cursor.goal = Some(goal);
                let mut selections = self.selections.clone();
                selections.push(cursor);
                let primary = selections.len() - 1;
                self.set_selections(selections, primary);
            }
            Command::SelectNextOccurrence => self.select_next_occurrence(),
            Command::Insert(text) => self.insert(&text),
            Command::ReplaceBefore { delete, text, cursor_back } => {
                let len = text.chars().count();
                let mut changes = Vec::new();
                let mut after = Vec::new();
                let mut delta = 0isize;
                for sel in &self.selections {
                    let end = sel.end();
                    let start = sel.start().saturating_sub(delete);
                    let s = (start as isize + delta) as usize;
                    after.push(Selection::cursor(s + len - cursor_back.min(len)));
                    delta += len as isize - (end - start) as isize;
                    changes.push(change(start, end, &text));
                }
                self.edit(changes, Some(after), EditKind::Other);
            }
            Command::Paste(text) => self.paste(&text.replace("\r\n", "\n")),
            Command::Newline => self.newline(),
            Command::Backspace { word } => self.backspace(word),
            Command::Delete { word } => self.delete_forward(word),
            Command::DeleteSelection => {
                let changes = self
                    .selections
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| change(s.start(), s.end(), ""))
                    .collect();
                self.edit(changes, None, EditKind::Other);
            }
            Command::Indent => self.indent(),
            Command::Outdent => self.outdent(),
            Command::ToggleComment => self.toggle_comment(),
            Command::MoveLinesUp => self.move_lines(true),
            Command::MoveLinesDown => self.move_lines(false),
            Command::DuplicateLines => self.duplicate_lines(),
            Command::DeleteLines => self.delete_lines(),
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::Select { ranges, primary } => {
                if ranges.is_empty() {
                    return;
                }
                let selections = ranges.iter().map(|&(start, end)| Selection::new(start, end)).collect();
                self.set_selections(selections, primary);
            }
            Command::Replace(replacements) => {
                let changes = replacements
                    .iter()
                    .map(|(start, end, text)| change(*start, *end, text))
                    .collect();
                self.edit(changes, None, EditKind::Other);
            }
        }
    }

    /// Char ranges of every match of `matcher`.
    pub fn find(&self, matcher: &Matcher) -> Vec<(usize, usize)> {
        let text = self.rope.to_string();
        matcher
            .find_all(&text)
            .into_iter()
            .map(|(start, end)| (self.rope.byte_to_char(start), self.rope.byte_to_char(end)))
            .collect()
    }

    /// Replacement edits for match `ranges` (char indices), with `$1`-style
    /// groups expanded in regex mode. Feed the result to [`Command::Replace`].
    pub fn replacements(
        &self,
        matcher: &Matcher,
        ranges: &[(usize, usize)],
        template: &str,
    ) -> Vec<(usize, usize, String)> {
        let text = self.rope.to_string();
        ranges
            .iter()
            .map(|&(start, end)| {
                let bytes = (self.rope.char_to_byte(start), self.rope.char_to_byte(end));
                (start, end, matcher.replacement(&text, bytes, template))
            })
            .collect()
    }

    fn moved(&self, sel: Selection, motion: Motion, extend: bool) -> Selection {
        if !extend && !sel.is_empty() {
            match motion {
                Motion::Left => return Selection::cursor(sel.start()),
                Motion::Right => return Selection::cursor(sel.end()),
                _ => {}
            }
        }
        let (head, goal) = self.move_head(sel, motion);
        let anchor = if extend { sel.anchor } else { head };
        Selection { anchor, head, goal }
    }

    fn move_head(&self, sel: Selection, motion: Motion) -> (usize, Option<usize>) {
        let head = sel.head;
        let pos = self.char_to_pos(head);
        let vertical = |lines: isize| {
            let goal = sel.goal.unwrap_or(self.visual_col(pos.line, pos.column));
            let target = pos.line as isize + lines;
            if target < 0 {
                return (0, Some(goal));
            }
            let line = target as usize;
            if line >= self.len_lines() {
                return (self.len_chars(), Some(goal));
            }
            let column = self.column_for_visual(line, goal);
            (self.pos_to_char(Pos { line, column }), Some(goal))
        };
        match motion {
            Motion::Left => (head.saturating_sub(1), None),
            Motion::Right => ((head + 1).min(self.len_chars()), None),
            Motion::Up => vertical(-1),
            Motion::Down => vertical(1),
            Motion::PageUp(n) => vertical(-(n as isize)),
            Motion::PageDown(n) => vertical(n as isize),
            Motion::WordLeft => (self.word_left(head), None),
            Motion::WordRight => (self.word_right(head), None),
            Motion::Home => {
                let indent = self.indent_len(pos.line);
                let column = if pos.column == indent { 0 } else { indent };
                (self.pos_to_char(Pos { line: pos.line, column }), None)
            }
            Motion::End => (self.pos_to_char(Pos { line: pos.line, column: usize::MAX }), None),
            Motion::DocumentStart => (0, None),
            Motion::DocumentEnd => (self.len_chars(), None),
        }
    }

    fn select_next_occurrence(&mut self) {
        let primary = self.primary();
        if primary.is_empty() {
            let (start, end) = self.word_at(primary.head);
            if start == end {
                return;
            }
            let mut selections = self.selections.clone();
            selections[self.primary] = Selection::new(start, end);
            self.set_selections(selections, self.primary);
            return;
        }
        let needle: Vec<char> = self.rope.slice(primary.start()..primary.end()).chars().collect();
        let haystack: Vec<char> = self.rope.chars().collect();
        let from = self.selections.iter().map(Selection::end).max().unwrap_or(0);
        let taken = |start: usize| self.selections.iter().any(|s| s.start() == start);
        let found = (from..haystack.len())
            .chain(0..from)
            .find(|&i| haystack[i..].starts_with(&needle) && !taken(i));
        if let Some(start) = found {
            let mut selections = self.selections.clone();
            selections.push(Selection::new(start, start + needle.len()));
            let primary = selections.len() - 1;
            self.set_selections(selections, primary);
        }
    }

    fn insert(&mut self, text: &str) {
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let mut changes = Vec::new();
        let mut after = Vec::new();
        let mut delta = 0isize;
        for sel in self.selections.clone() {
            let (start, end) = (sel.start(), sel.end());
            let next = self.char_at(end);
            let prev = start.checked_sub(1).map(|i| self.rope.char(i));
            let shift = |i: usize, delta: isize| (i as isize + delta) as usize;

            // Typing a closer that is already there just steps over it.
            if let Some(c) = single {
                if sel.is_empty() && next == Some(c) && PAIRS.iter().any(|&(_, close)| close == c) {
                    after.push(Selection::cursor(shift(start, delta) + 1));
                    continue;
                }
            }
            let pair = single.and_then(|c| PAIRS.iter().find(|&&(open, _)| open == c));
            let (inserted, selection) = match pair {
                // Wrap a selection: `(selected)`, keeping it selected.
                Some(&(open, close)) if !sel.is_empty() => {
                    let inner = self.rope.slice(start..end).to_string();
                    let s = shift(start, delta) + 1;
                    let new = if sel.head >= sel.anchor {
                        Selection::new(s, s + (end - start))
                    } else {
                        Selection::new(s + (end - start), s)
                    };
                    (format!("{open}{inner}{close}"), new)
                }
                Some(&(open, close)) if should_autoclose(open, prev, next) => {
                    (format!("{open}{close}"), Selection::cursor(shift(start, delta) + 1))
                }
                _ => {
                    let len = text.chars().count();
                    (text.to_string(), Selection::cursor(shift(start, delta) + len))
                }
            };
            delta += inserted.chars().count() as isize - (end - start) as isize;
            changes.push(change(start, end, &inserted));
            after.push(selection);
        }
        let kind = if text.contains('\n') { EditKind::Other } else { EditKind::Typing };
        if changes.is_empty() {
            self.set_selections(after, self.primary);
        } else {
            self.edit(changes, Some(after), kind);
        }
    }

    fn paste(&mut self, text: &str) {
        let lines: Vec<&str> = text.split('\n').collect();
        let distribute = self.selections.len() > 1 && lines.len() == self.selections.len();
        let changes = self
            .selections
            .iter()
            .enumerate()
            .map(|(i, s)| change(s.start(), s.end(), if distribute { lines[i] } else { text }))
            .collect();
        self.edit(changes, None, EditKind::Other);
    }

    fn newline(&mut self) {
        let mut changes = Vec::new();
        let mut after = Vec::new();
        let mut delta = 0isize;
        for sel in self.selections.clone() {
            let (start, end) = (sel.start(), sel.end());
            let pos = self.char_to_pos(start);
            let line = self.line_text(pos.line);
            let before: String = line.chars().take(pos.column).collect();
            let indent: String = before.chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
            let opens_block = before.trim_end().ends_with([':', '(', '[', '{']);
            let between_pair = matches!(
                (before.chars().last(), self.char_at(end)),
                (Some('('), Some(')')) | (Some('['), Some(']')) | (Some('{'), Some('}'))
            );

            let inner = if opens_block { format!("{indent}\t") } else { indent.clone() };
            let mut inserted = format!("\n{inner}");
            let cursor = inserted.chars().count();
            if between_pair {
                inserted.push('\n');
                inserted.push_str(&indent);
            }
            let s = (start as isize + delta) as usize;
            after.push(Selection::cursor(s + cursor));
            delta += inserted.chars().count() as isize - (end - start) as isize;
            changes.push(change(start, end, &inserted));
        }
        self.edit(changes, Some(after), EditKind::Other);
    }

    fn backspace(&mut self, word: bool) {
        let changes = self
            .selections
            .iter()
            .filter_map(|s| {
                if !s.is_empty() {
                    return Some(change(s.start(), s.end(), ""));
                }
                let head = s.head;
                if head == 0 {
                    return None;
                }
                if word {
                    return Some(change(self.word_left(head), head, ""));
                }
                let prev = self.rope.char(head - 1);
                let empty_pair = PAIRS
                    .iter()
                    .any(|&(open, close)| open == prev && self.char_at(head) == Some(close));
                let end = if empty_pair { head + 1 } else { head };
                Some(change(head - 1, end, ""))
            })
            .collect();
        self.edit(changes, None, EditKind::Deleting);
    }

    fn delete_forward(&mut self, word: bool) {
        let len = self.len_chars();
        let changes = self
            .selections
            .iter()
            .filter_map(|s| {
                if !s.is_empty() {
                    return Some(change(s.start(), s.end(), ""));
                }
                if s.head >= len {
                    return None;
                }
                let end = if word { self.word_right(s.head) } else { s.head + 1 };
                Some(change(s.head, end, ""))
            })
            .collect();
        self.edit(changes, None, EditKind::Deleting);
    }

    fn indent(&mut self) {
        let multiline = self
            .selections
            .iter()
            .any(|s| self.char_to_pos(s.start()).line != self.char_to_pos(s.end()).line);
        if !multiline {
            self.insert("\t");
            return;
        }
        let changes = self
            .touched_lines()
            .into_iter()
            .filter(|&line| self.line_len(line) > 0)
            .map(|line| {
                let start = self.rope.line_to_char(line);
                change(start, start, "\t")
            })
            .collect();
        self.edit(changes, None, EditKind::Other);
    }

    fn outdent(&mut self) {
        let changes = self
            .touched_lines()
            .into_iter()
            .filter_map(|line| {
                let text = self.line_text(line);
                let remove = if text.starts_with('\t') {
                    1
                } else {
                    text.chars().take(TAB_WIDTH).take_while(|&c| c == ' ').count()
                };
                let start = self.rope.line_to_char(line);
                (remove > 0).then(|| change(start, start + remove, ""))
            })
            .collect();
        self.edit(changes, None, EditKind::Other);
    }

    /// Godot-style: `#` at column 0; removed if every touched line has one.
    fn toggle_comment(&mut self) {
        let lines = self.touched_lines();
        let all_commented = lines.iter().all(|&line| {
            let text = self.line_text(line);
            text.trim().is_empty() || text.trim_start().starts_with('#')
        });
        let changes = lines
            .into_iter()
            .filter_map(|line| {
                let text = self.line_text(line);
                let start = self.rope.line_to_char(line);
                if all_commented {
                    let offset = text.chars().position(|c| c == '#')?;
                    Some(change(start + offset, start + offset + 1, ""))
                } else {
                    Some(change(start, start, "#"))
                }
            })
            .collect();
        self.edit(changes, None, EditKind::Other);
    }

    fn move_lines(&mut self, up: bool) {
        let lines = self.touched_lines();
        let (first, last) = (lines[0], *lines.last().unwrap());
        if (up && first == 0) || (!up && last + 1 >= self.len_lines()) {
            return;
        }
        let block: Vec<String> = (first..=last).map(|l| self.line_text(l)).collect();
        let block = block.join("\n");
        // Swap the block with the neighbouring line.
        let (range_start, range_end, text, shift) = if up {
            let other = self.line_text(first - 1);
            let start = self.rope.line_to_char(first - 1);
            let text = format!("{block}\n{other}");
            (start, self.line_end(last), text, -(other.chars().count() as isize + 1))
        } else {
            let other = self.line_text(last + 1);
            let start = self.rope.line_to_char(first);
            let text = format!("{other}\n{block}");
            (start, self.line_end(last + 1), text, other.chars().count() as isize + 1)
        };
        let after = self.shifted_selections(shift);
        self.edit(vec![change(range_start, range_end, &text)], Some(after), EditKind::Other);
    }

    fn duplicate_lines(&mut self) {
        let lines = self.touched_lines();
        let (first, last) = (lines[0], *lines.last().unwrap());
        let block: Vec<String> = (first..=last).map(|l| self.line_text(l)).collect();
        let block = block.join("\n");
        let at = self.line_end(last);
        let shift = block.chars().count() as isize + 1;
        let after = self.shifted_selections(shift);
        self.edit(vec![change(at, at, &format!("\n{block}"))], Some(after), EditKind::Other);
    }

    fn delete_lines(&mut self) {
        let lines = self.touched_lines();
        let (first, last) = (lines[0], *lines.last().unwrap());
        let (start, end) = if last + 1 < self.len_lines() {
            (self.rope.line_to_char(first), self.rope.line_to_char(last + 1))
        } else if first > 0 {
            (self.line_end(first - 1), self.len_chars())
        } else {
            (0, self.len_chars())
        };
        let cursor = Selection::cursor(start.min(self.len_chars() - (end - start)));
        self.edit(vec![change(start, end, "")], Some(vec![cursor]), EditKind::Other);
    }

    fn undo(&mut self) {
        let Some(group) = self.undo.pop() else {
            return;
        };
        for entry in group.iter().rev() {
            self.apply_raw(&entry.inverse);
        }
        let selections = group[0].before.clone();
        self.redo.push(group);
        self.after_history(selections);
    }

    fn redo(&mut self) {
        let Some(group) = self.redo.pop() else {
            return;
        };
        for entry in &group {
            self.apply_raw(&entry.forward);
        }
        let selections = group.last().unwrap().after.clone();
        self.undo.push(group);
        self.after_history(selections);
    }

    fn after_history(&mut self, selections: Vec<Selection>) {
        self.last_kind = None;
        self.version += 1;
        let primary = selections.len() - 1;
        self.set_selections(selections, primary);
    }

    // ---- Editing machinery ----

    /// Applies `changes`, maps (or replaces) selections, and records undo.
    fn edit(&mut self, changes: Vec<Change>, after: Option<Vec<Selection>>, kind: EditKind) {
        if changes.is_empty() {
            return;
        }
        let before = self.selections.clone();
        let mut inverse = Vec::with_capacity(changes.len());
        let mut delta = 0isize;
        for c in &changes {
            let start = (c.start as isize + delta) as usize;
            let inserted = c.text.chars().count();
            inverse.push(Change {
                start,
                end: start + inserted,
                text: self.rope.slice(c.start..c.end).to_string(),
            });
            delta += inserted as isize - (c.end - c.start) as isize;
        }
        let after = after.unwrap_or_else(|| {
            self.selections
                .iter()
                .map(|s| Selection::new(map_pos(&changes, s.anchor), map_pos(&changes, s.head)))
                .collect()
        });
        self.apply_raw(&changes);

        let entry = Entry { forward: changes, inverse, before, after: after.clone() };
        let merge = kind != EditKind::Other && self.last_kind == Some(kind);
        match self.undo.last_mut() {
            Some(group) if merge => group.push(entry),
            _ => self.undo.push(vec![entry]),
        }
        self.redo.clear();
        self.last_kind = Some(kind);
        self.version += 1;
        let primary = self.primary.min(after.len() - 1);
        self.set_selections_keep_kind(after, primary);
    }

    /// Applies changes to the rope (descending, so indices stay valid).
    fn apply_raw(&mut self, changes: &[Change]) {
        if let Some(first) = changes.first() {
            let line = self.rope.char_to_line(first.start.min(self.len_chars()));
            self.line_states.borrow_mut().truncate(line + 1);
        }
        for c in changes.iter().rev() {
            self.rope.remove(c.start..c.end);
            self.rope.insert(c.start, &c.text);
        }
    }

    fn set_selections(&mut self, selections: Vec<Selection>, primary: usize) {
        self.last_kind = None;
        self.set_selections_keep_kind(selections, primary);
    }

    /// Sorts and merges selections, keeping track of the primary one.
    fn set_selections_keep_kind(&mut self, selections: Vec<Selection>, primary: usize) {
        let len = self.len_chars();
        let clamp = |s: Selection| Selection {
            anchor: s.anchor.min(len),
            head: s.head.min(len),
            goal: s.goal,
        };
        let primary_head = clamp(selections[primary.min(selections.len() - 1)]).head;
        let mut selections: Vec<Selection> = selections.into_iter().map(clamp).collect();
        selections.sort_by_key(|s| (s.start(), s.end()));

        let mut merged: Vec<Selection> = Vec::with_capacity(selections.len());
        for sel in selections {
            match merged.last_mut() {
                Some(last) if sel.start() < last.end() || sel.start() == last.start() => {
                    let (start, end) = (last.start(), last.end().max(sel.end()));
                    *last = if last.head < last.anchor {
                        Selection::new(end, start)
                    } else {
                        Selection::new(start, end)
                    };
                }
                _ => merged.push(sel),
            }
        }
        self.primary = merged
            .iter()
            .position(|s| s.start() <= primary_head && primary_head <= s.end())
            .unwrap_or(merged.len() - 1);
        self.selections = merged;
        self.reveal += 1;
    }

    fn shifted_selections(&self, shift: isize) -> Vec<Selection> {
        let move_by = |i: usize| (i as isize + shift).max(0) as usize;
        self.selections
            .iter()
            .map(|s| Selection::new(move_by(s.anchor), move_by(s.head)))
            .collect()
    }

    /// Lines covered by any selection; a selection ending at column 0
    /// doesn't include that last line.
    fn touched_lines(&self) -> Vec<usize> {
        let mut lines = Vec::new();
        for s in &self.selections {
            let first = self.char_to_pos(s.start()).line;
            let end = self.char_to_pos(s.end());
            let last = if !s.is_empty() && end.column == 0 && end.line > first {
                end.line - 1
            } else {
                end.line
            };
            lines.extend(first..=last);
        }
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    fn char_at(&self, index: usize) -> Option<char> {
        (index < self.len_chars()).then(|| self.rope.char(index))
    }

    fn line_start(&self, line: usize) -> usize {
        if line >= self.len_lines() {
            self.len_chars()
        } else {
            self.rope.line_to_char(line)
        }
    }

    /// Char index of the end of `line`, before its line break.
    fn line_end(&self, line: usize) -> usize {
        self.rope.line_to_char(line) + self.line_len(line)
    }

    fn indent_len(&self, line: usize) -> usize {
        self.line_text(line).chars().take_while(|c| matches!(c, ' ' | '\t')).count()
    }

    fn word_right(&self, pos: usize) -> usize {
        let len = self.len_chars();
        let mut i = pos;
        if self.char_at(i) == Some('\n') {
            return i + 1;
        }
        while matches!(self.char_at(i), Some(' ' | '\t')) {
            i += 1;
        }
        if let Some(class) = self.char_at(i).map(char_class) {
            while self.char_at(i).is_some_and(|c| char_class(c) == class && c != '\n') {
                i += 1;
            }
        }
        if i == pos { (i + 1).min(len) } else { i }
    }

    fn word_left(&self, pos: usize) -> usize {
        let mut i = pos;
        let before = |i: usize| i.checked_sub(1).and_then(|j| self.char_at(j));
        if before(i) == Some('\n') {
            return i - 1;
        }
        while matches!(before(i), Some(' ' | '\t')) {
            i -= 1;
        }
        if let Some(class) = before(i).map(char_class) {
            while before(i).is_some_and(|c| char_class(c) == class && c != '\n') {
                i -= 1;
            }
        }
        if i == pos { i.saturating_sub(1) } else { i }
    }

    /// Start of the identifier that ends at `index` (`index` itself if none).
    pub fn word_start(&self, index: usize) -> usize {
        let mut start = index.min(self.len_chars());
        while start > 0 && char_class(self.rope.char(start - 1)) == CharClass::Word {
            start -= 1;
        }
        start
    }

    /// Text between two char indices.
    pub fn slice(&self, start: usize, end: usize) -> String {
        let end = end.min(self.len_chars());
        self.rope.slice(start.min(end)..end).to_string()
    }

    /// The identifier around `index` (empty range if none).
    pub fn word_range(&self, index: usize) -> (usize, usize) {
        self.word_at(index)
    }

    fn word_at(&self, pos: usize) -> (usize, usize) {
        let is_word = |i: usize| self.char_at(i).is_some_and(|c| char_class(c) == CharClass::Word);
        let anchor = if is_word(pos) {
            pos
        } else if pos > 0 && is_word(pos - 1) {
            pos - 1
        } else {
            return (pos, pos);
        };
        let mut start = anchor;
        while start > 0 && is_word(start - 1) {
            start -= 1;
        }
        let mut end = anchor;
        while is_word(end) {
            end += 1;
        }
        (start, end)
    }
}

fn change(start: usize, end: usize, text: &str) -> Change {
    Change { start, end, text: text.to_string() }
}

/// Maps a pre-edit position through sorted changes. Positions inside a
/// replaced range (or at an insertion point) end up after the new text.
fn map_pos(changes: &[Change], pos: usize) -> usize {
    let mut delta = 0isize;
    for c in changes {
        let inserted = c.text.chars().count() as isize;
        if pos < c.start {
            break;
        }
        if pos >= c.end {
            delta += inserted - (c.end - c.start) as isize;
            continue;
        }
        return (c.start as isize + delta + inserted) as usize;
    }
    (pos as isize + delta) as usize
}

fn should_autoclose(open: char, prev: Option<char>, next: Option<char>) -> bool {
    let next_ok = next.is_none_or(|c| c.is_whitespace() || matches!(c, ')' | ']' | '}' | ',' | ':' | ';'));
    if open == '"' || open == '\'' {
        // Not inside a word or right after a quote: `don't`, `""` stays manual.
        let prev_ok = prev.is_none_or(|c| !c.is_alphanumeric() && c != '"' && c != '\'');
        next_ok && prev_ok
    } else {
        next_ok
    }
}

fn char_width(c: char, col: usize) -> usize {
    if c == '\t' {
        TAB_WIDTH - col % TAB_WIDTH
    } else {
        1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Space,
    Word,
    Punct,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        Buffer::new(text, Language::GDScript)
    }

    fn at(b: &mut Buffer, line: usize, column: usize) {
        b.set_cursor(Pos { line, column });
    }

    fn type_str(b: &mut Buffer, s: &str) {
        for c in s.chars() {
            b.apply(Command::Insert(c.to_string()));
        }
    }

    #[test]
    fn typing_groups_into_one_undo() {
        let mut b = buf("");
        type_str(&mut b, "var x");
        assert_eq!(b.text(), "var x");
        b.apply(Command::Undo);
        assert_eq!(b.text(), "");
        b.apply(Command::Redo);
        assert_eq!(b.text(), "var x");
    }

    #[test]
    fn moving_breaks_undo_group() {
        let mut b = buf("");
        type_str(&mut b, "ab");
        b.apply(Command::Move { motion: Motion::Left, extend: false });
        type_str(&mut b, "X");
        assert_eq!(b.text(), "aXb");
        b.apply(Command::Undo);
        assert_eq!(b.text(), "ab");
    }

    #[test]
    fn multi_cursor_insert_and_undo() {
        let mut b = buf("a\nb\nc");
        at(&mut b, 0, 1);
        b.apply(Command::AddCursorBelow);
        b.apply(Command::AddCursorBelow);
        assert_eq!(b.selections().len(), 3);
        type_str(&mut b, "!");
        assert_eq!(b.text(), "a!\nb!\nc!");
        b.apply(Command::Backspace { word: false });
        assert_eq!(b.text(), "a\nb\nc");
        b.apply(Command::Undo);
        assert_eq!(b.text(), "a!\nb!\nc!");
    }

    #[test]
    fn auto_close_and_step_over() {
        let mut b = buf("");
        type_str(&mut b, "f(");
        assert_eq!(b.text(), "f()");
        type_str(&mut b, "x)");
        assert_eq!(b.text(), "f(x)");
        assert_eq!(b.primary().head, 4);

        let mut b = buf("");
        type_str(&mut b, "[");
        b.apply(Command::Backspace { word: false });
        assert_eq!(b.text(), "");

        let mut b = buf("don");
        at(&mut b, 0, 3);
        type_str(&mut b, "'");
        assert_eq!(b.text(), "don'");
    }

    #[test]
    fn wrap_selection_in_brackets() {
        let mut b = buf("value");
        b.apply(Command::SelectAll);
        type_str(&mut b, "(");
        assert_eq!(b.text(), "(value)");
        assert_eq!((b.primary().start(), b.primary().end()), (1, 6));
    }

    #[test]
    fn newline_keeps_and_adds_indent() {
        let mut b = buf("\tif x:");
        at(&mut b, 0, 6);
        b.apply(Command::Newline);
        assert_eq!(b.text(), "\tif x:\n\t\t");
        b.apply(Command::Newline);
        assert_eq!(b.text(), "\tif x:\n\t\t\n\t\t");

        let mut b = buf("f()");
        at(&mut b, 0, 2);
        b.apply(Command::Newline);
        assert_eq!(b.text(), "f(\n\t\n)");
        assert_eq!(b.char_to_pos(b.primary().head), Pos { line: 1, column: 1 });
    }

    #[test]
    fn indent_outdent_and_comment_lines() {
        let mut b = buf("a\nb\n");
        b.apply(Command::SelectAll);
        b.apply(Command::Indent);
        assert_eq!(b.text(), "\ta\n\tb\n");
        b.apply(Command::Outdent);
        assert_eq!(b.text(), "a\nb\n");
        b.apply(Command::ToggleComment);
        assert_eq!(b.text(), "#a\n#b\n");
        b.apply(Command::ToggleComment);
        assert_eq!(b.text(), "a\nb\n");
    }

    #[test]
    fn move_duplicate_delete_lines() {
        let mut b = buf("one\ntwo\nthree");
        at(&mut b, 1, 1);
        b.apply(Command::MoveLinesUp);
        assert_eq!(b.text(), "two\none\nthree");
        assert_eq!(b.char_to_pos(b.primary().head), Pos { line: 0, column: 1 });
        b.apply(Command::MoveLinesDown);
        b.apply(Command::MoveLinesDown);
        assert_eq!(b.text(), "one\nthree\ntwo");
        b.apply(Command::DuplicateLines);
        assert_eq!(b.text(), "one\nthree\ntwo\ntwo");
        assert_eq!(b.char_to_pos(b.primary().head).line, 3);
        b.apply(Command::DeleteLines);
        assert_eq!(b.text(), "one\nthree\ntwo");
    }

    #[test]
    fn select_next_occurrence_then_type() {
        let mut b = buf("hp = hp + max_hp");
        at(&mut b, 0, 0);
        b.apply(Command::SelectNextOccurrence);
        b.apply(Command::SelectNextOccurrence);
        assert_eq!(b.selections().len(), 2);
        type_str(&mut b, "health");
        assert_eq!(b.text(), "health = health + max_hp");
    }

    #[test]
    fn vertical_motion_keeps_goal_across_tabs() {
        let mut b = buf("\tabc\nx\n\tabc");
        at(&mut b, 0, 3); // visual column 6
        b.apply(Command::Move { motion: Motion::Down, extend: false });
        assert_eq!(b.char_to_pos(b.primary().head), Pos { line: 1, column: 1 });
        b.apply(Command::Move { motion: Motion::Down, extend: false });
        assert_eq!(b.char_to_pos(b.primary().head), Pos { line: 2, column: 3 });
    }

    #[test]
    fn word_motions_and_smart_home() {
        let mut b = buf("\tvar speed := 1");
        at(&mut b, 0, 1);
        b.apply(Command::Move { motion: Motion::WordRight, extend: false });
        assert_eq!(b.primary().head, 4);
        b.apply(Command::Move { motion: Motion::WordRight, extend: false });
        assert_eq!(b.primary().head, 10);
        b.apply(Command::Move { motion: Motion::WordLeft, extend: false });
        assert_eq!(b.primary().head, 5);
        b.apply(Command::Move { motion: Motion::Home, extend: false });
        assert_eq!(b.primary().head, 1);
        b.apply(Command::Move { motion: Motion::Home, extend: false });
        assert_eq!(b.primary().head, 0);
    }

    #[test]
    fn paste_distributes_lines_over_cursors() {
        let mut b = buf("a\nb");
        at(&mut b, 0, 1);
        b.apply(Command::AddCursorBelow);
        b.apply(Command::Paste("1\n2".into()));
        assert_eq!(b.text(), "a1\nb2");
    }

    #[test]
    fn replace_before_accepts_completion_at_every_cursor() {
        let mut b = buf("v.len\nw.len");
        at(&mut b, 0, 5);
        b.apply(Command::AddCursorBelow);
        assert_eq!(b.word_start(b.primary().head) , b.pos_to_char(Pos { line: 1, column: 2 }));
        b.apply(Command::ReplaceBefore { delete: 3, text: "length()".into(), cursor_back: 1 });
        assert_eq!(b.text(), "v.length()\nw.length()");
        assert_eq!(b.char_to_pos(b.primary().head), Pos { line: 1, column: 9 });
        b.apply(Command::Undo);
        assert_eq!(b.text(), "v.len\nw.len");
    }

    #[test]
    fn find_and_replace_all_in_one_undo_step() {
        use crate::search::Query;
        let mut b = buf("var é = hp\nhp += 1");
        let query = Query { text: "hp".into(), whole_word: true, ..Query::default() };
        let matcher = Matcher::new(&query).unwrap().unwrap();
        let found = b.find(&matcher);
        assert_eq!(found, vec![(8, 10), (11, 13)]);

        b.apply(Command::Select { ranges: found.clone(), primary: 1 });
        assert_eq!(b.selections().len(), 2);
        assert_eq!(b.primary().start(), 11);

        let edits = b.replacements(&matcher, &found, "health");
        b.apply(Command::Replace(edits));
        assert_eq!(b.text(), "var é = health\nhealth += 1");
        b.apply(Command::Undo);
        assert_eq!(b.text(), "var é = hp\nhp += 1");
    }

    #[test]
    fn byte_order_mark_is_kept_out_of_the_text() {
        let b = buf("\u{feff}extends Node");
        assert_eq!(b.text(), "extends Node");
        assert_eq!(b.line_text(0), "extends Node");
        assert_eq!(b.file_contents(), "\u{feff}extends Node");
        assert_eq!(buf("plain").file_contents(), "plain");
    }

    #[test]
    fn crlf_is_preserved() {
        let mut b = buf("a\r\nb");
        assert_eq!(b.len_lines(), 2);
        at(&mut b, 1, 1);
        type_str(&mut b, "c");
        assert_eq!(b.text(), "a\r\nbc");
    }

    #[test]
    fn tokens_follow_multiline_strings_through_edits() {
        let mut b = buf("x\n\"\"\"\nvar\n\"\"\"");
        assert_eq!(b.tokens(2)[0].kind, gdscript::TokenKind::String);
        at(&mut b, 1, 0);
        b.apply(Command::DeleteLines);
        assert_eq!(b.tokens(1)[0].kind, gdscript::TokenKind::Keyword);
    }

    #[test]
    fn brackets_match_both_ways() {
        let b = buf("f(a[1])");
        assert_eq!(b.matching_bracket(2), Some((1, 6)));
        assert_eq!(b.matching_bracket(7), Some((1, 6)));
        assert_eq!(b.matching_bracket(4), Some((3, 5)));
    }
}
