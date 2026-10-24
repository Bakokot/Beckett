//! Find/replace in the active file, and search across the project.

use std::path::{Path, PathBuf};

use editor_core::search::{matching_lines, LineMatch, Matcher, Query};
use editor_core::Buffer;
use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, space, text, text_input, Column};
use iced::{Alignment, Element, Font, Length};

use crate::style;

pub const FIND_INPUT: &str = "find-query";
pub const REPLACE_INPUT: &str = "find-replace";
pub const SEARCH_INPUT: &str = "project-search";

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LINES_PER_FILE: usize = 500;
const MAX_TOTAL_LINES: usize = 5000;
/// Text files worth searching in a Godot project.
const SEARCHED: &[&str] = &[
    "gd", "gdshader", "gdshaderinc", "tscn", "tres", "godot", "cfg", "json", "md", "txt", "cs", "csv", "ini",
    "gdextension",
];

#[derive(Debug, Clone)]
pub enum FindMessage {
    Query(String),
    Replace(String),
    ToggleCase,
    ToggleWord,
    ToggleRegex,
    ToggleReplace,
    Next,
    Previous,
    /// Enter in the query box: next, or previous with Shift.
    Submit,
    ReplaceOne,
    ReplaceAll,
    /// Turn every match into a cursor.
    SelectAll,
    Close,
}

/// The find bar of the active tab.
pub struct Find {
    pub query: Query,
    pub replace: String,
    pub show_replace: bool,
    /// Char ranges of matches in the buffer.
    pub matches: Vec<(usize, usize)>,
    pub current: Option<usize>,
    pub error: Option<String>,
    matcher: Option<Matcher>,
}

impl Find {
    pub fn new(query: Query, show_replace: bool) -> Self {
        Self {
            query,
            replace: String::new(),
            show_replace,
            matches: Vec::new(),
            current: None,
            error: None,
            matcher: None,
        }
    }

    pub fn matcher(&self) -> Option<&Matcher> {
        self.matcher.as_ref()
    }

    /// Re-runs the query; the current match becomes the first one at or
    /// after the cursor.
    pub fn recompute(&mut self, buffer: &Buffer) {
        match Matcher::new(&self.query) {
            Ok(matcher) => {
                self.matches = matcher.as_ref().map(|m| buffer.find(m)).unwrap_or_default();
                self.matcher = matcher;
                self.error = None;
            }
            Err(error) => {
                self.matches.clear();
                self.matcher = None;
                self.error = Some(error);
            }
        }
        let from = buffer.primary().start();
        self.current = (!self.matches.is_empty())
            .then(|| self.matches.iter().position(|&(start, _)| start >= from).unwrap_or(0));
    }

    pub fn step(&mut self, forward: bool) {
        let len = self.matches.len();
        self.current = match self.current {
            _ if len == 0 => None,
            None => Some(0),
            Some(i) if forward => Some((i + 1) % len),
            Some(i) => Some((i + len - 1) % len),
        };
    }

    pub fn current_range(&self) -> Option<(usize, usize)> {
        self.matches.get(self.current?).copied()
    }

    pub fn view<'a>(&'a self, ui: &theme::UiColors) -> Element<'a, FindMessage> {
        let muted = style::to_color(ui.text_muted);
        let toggle = |label: &'static str, on: bool, message: FindMessage, description: &'static str| {
            style::tip(toggle_button(label, on, message), description, Position::Bottom, ui)
        };
        let action = |label: &'static str, message: FindMessage, description: &'static str| {
            let button = button(text(label).size(12)).on_press(message).style(button::text).padding([2, 6]);
            style::tip(button, description, Position::Bottom, ui)
        };
        let count = match (&self.error, self.current) {
            (Some(error), _) => text(error.clone()).size(12).color(style::to_color(ui.danger)),
            (None, _) if self.query.text.is_empty() => text("").size(12),
            (None, None) => text("No results").size(12).color(muted),
            (None, Some(i)) => text(format!("{} of {}", i + 1, self.matches.len())).size(12).color(muted),
        };

        let expand = if self.show_replace { "v" } else { ">" };
        let find_row = row![
            action(expand, FindMessage::ToggleReplace, "Show or hide Replace (Ctrl+H)"),
            text_input("Find", &self.query.text)
                .id(FIND_INPUT)
                .on_input(FindMessage::Query)
                .on_submit(FindMessage::Submit)
                .size(13)
                .padding([3, 6])
                .width(280),
            toggle("Aa", self.query.case_sensitive, FindMessage::ToggleCase, MATCH_CASE),
            toggle("W", self.query.whole_word, FindMessage::ToggleWord, WHOLE_WORD),
            toggle(".*", self.query.regex, FindMessage::ToggleRegex, REGEX),
            count,
            space::horizontal(),
            action("prev", FindMessage::Previous, "Previous match (Shift+Enter, Shift+F3)"),
            action("next", FindMessage::Next, "Next match (Enter, F3)"),
            action("all", FindMessage::SelectAll, "Select all matches, one cursor each"),
            action("x", FindMessage::Close, "Close (Escape)"),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let mut content = column![find_row].spacing(4);
        if self.show_replace {
            content = content.push(
                row![
                    space().width(26),
                    text_input("Replace", &self.replace)
                        .id(REPLACE_INPUT)
                        .on_input(FindMessage::Replace)
                        .on_submit(FindMessage::ReplaceOne)
                        .size(13)
                        .padding([3, 6])
                        .width(280),
                    action("replace", FindMessage::ReplaceOne, "Replace this match and go to the next (Enter)"),
                    action("replace all", FindMessage::ReplaceAll, "Replace every match (undo with Ctrl+Z)"),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        container(content).padding([4, 8]).width(Length::Fill).style(style::surface(ui)).into()
    }
}

const MATCH_CASE: &str = "Match case: \"Speed\" won't match \"speed\"";
const WHOLE_WORD: &str = "Match whole word: \"hp\" won't match \"max_hp\"";
const REGEX: &str = "Use regular expression ($1 in Replace inserts group 1)";

/// A search option button, highlighted while it is on.
fn toggle_button<'a, Message: Clone + 'a>(label: &'a str, on: bool, message: Message) -> iced::widget::Button<'a, Message> {
    let style: fn(&iced::Theme, button::Status) -> button::Style = if on { button::primary } else { button::text };
    button(text(label).size(12).font(Font::MONOSPACE)).on_press(message).style(style).padding([2, 6])
}

// ---- Project search ----

#[derive(Debug, Clone)]
pub enum SearchMessage {
    Query(String),
    ToggleCase,
    ToggleWord,
    ToggleRegex,
    Run,
    /// Results of search number `id`.
    Done(u64, Result<Vec<FileResult>, String>),
    Open(PathBuf, usize, usize),
}

#[derive(Debug, Clone)]
pub struct FileResult {
    pub path: PathBuf,
    pub lines: Vec<LineMatch>,
}

#[derive(Default)]
pub struct ProjectSearch {
    pub query: Query,
    pub results: Vec<FileResult>,
    pub running: Option<u64>,
    pub next_id: u64,
    pub error: Option<String>,
    /// Query of the shown results, for the summary line.
    pub searched: Option<String>,
}

impl ProjectSearch {
    pub fn view<'a>(&'a self, ui: &theme::UiColors, display: impl Fn(&Path) -> String) -> Element<'a, SearchMessage> {
        let muted = style::to_color(ui.text_muted);
        let toggle = |label: &'static str, on: bool, message: SearchMessage, description: &'static str| {
            style::tip(toggle_button(label, on, message), description, Position::Bottom, ui)
        };
        let total: usize = self.results.iter().map(|f| f.lines.len()).sum();
        let summary = match (&self.error, self.running, &self.searched) {
            (Some(error), _, _) => text(error.clone()).size(12).color(style::to_color(ui.danger)),
            (None, Some(_), _) => text("Searching...").size(12).color(muted),
            (None, None, Some(q)) => {
                let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
                let summary = format!("{} in {} for \"{q}\"", plural(total, "result"), plural(self.results.len(), "file"));
                text(summary).size(12).color(muted)
            }
            (None, None, None) => text("Enter to search all project files").size(12).color(muted),
        };
        let bar = row![
            text_input("Search in project", &self.query.text)
                .id(SEARCH_INPUT)
                .on_input(SearchMessage::Query)
                .on_submit(SearchMessage::Run)
                .size(13)
                .padding([3, 6])
                .width(320),
            toggle("Aa", self.query.case_sensitive, SearchMessage::ToggleCase, MATCH_CASE),
            toggle("W", self.query.whole_word, SearchMessage::ToggleWord, WHOLE_WORD),
            toggle(".*", self.query.regex, SearchMessage::ToggleRegex, REGEX),
            summary,
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .padding([2, 8]);

        let mut rows: Vec<Element<'a, SearchMessage>> = Vec::new();
        for file in &self.results {
            rows.push(
                container(text(format!("{}  ({})", display(&file.path), file.lines.len())).size(12).color(muted))
                    .padding([4, 8])
                    .into(),
            );
            for line in &file.lines {
                rows.push(
                    button(
                        row![
                            text(format!("{:>5}", line.line + 1)).size(12).font(Font::MONOSPACE).color(muted),
                            text(line.text.trim_start()).size(12).font(Font::MONOSPACE),
                        ]
                        .spacing(10),
                    )
                    .on_press(SearchMessage::Open(file.path.clone(), line.line, line.column))
                    .style(button::text)
                    .width(Length::Fill)
                    .padding([0, 8])
                    .into(),
                );
            }
        }
        column![bar, scrollable(Column::with_children(rows).width(Length::Fill)).height(Length::Fill)].into()
    }
}

/// Searches every text file under `root`. `open` holds unsaved buffers,
/// searched instead of their files on disk.
pub fn search_project(root: &Path, query: &Query, open: &[(PathBuf, String)]) -> Result<Vec<FileResult>, String> {
    let Some(matcher) = Matcher::new(query)? else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files.sort();

    let mut results = Vec::new();
    let mut total = 0;
    for path in files {
        let unsaved = open.iter().find(|(p, _)| lsp_client::uri::same_path(p, &path));
        let text = match unsaved {
            Some((_, text)) => text.clone(),
            None => match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(_) => continue,
            },
        };
        let limit = MAX_LINES_PER_FILE.min(MAX_TOTAL_LINES - total);
        let lines = matching_lines(&matcher, &text, limit);
        if lines.is_empty() {
            continue;
        }
        total += lines.len();
        results.push(FileResult { path, lines });
        if total >= MAX_TOTAL_LINES {
            break;
        }
    }
    Ok(results)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !godot_project::is_hidden(&name, true) && !name.starts_with('.') {
                collect_files(&path, out);
            }
        } else if kind.is_file() {
            let searched = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| SEARCHED.contains(&e.to_ascii_lowercase().as_str()));
            let small = entry.metadata().is_ok_and(|m| m.len() <= MAX_FILE_BYTES);
            if searched && small {
                out.push(path);
            }
        }
    }
}
