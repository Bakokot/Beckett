//! The outline of the active GDScript file (functions, variables, signals…)
//! from the language server, shown in the sidebar and in the palette (`@`).

use std::path::PathBuf;

use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, text, text_input, Column, Row};
use iced::{Alignment, Background, Border, Element, Font, Length};
use lsp_client::{Symbol, SymbolKind};

use crate::{style, Message};

/// One row of the outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub depth: usize,
    pub name: String,
    pub detail: String,
    pub kind: SymbolKind,
    /// Where the name is (zero-based line and character).
    pub line: u32,
    pub character: u32,
    /// Lines the whole declaration spans.
    pub first_line: u32,
    pub last_line: u32,
}

/// The outline of one version of one file.
pub struct Outline {
    pub path: PathBuf,
    /// The tab version it was computed for.
    pub version: i32,
    pub entries: Vec<Entry>,
}

/// Flattens the symbol tree in document order. Godot wraps everything in a
/// symbol for the file's own class; its members are shown at the top level.
pub fn flatten(symbols: &[Symbol]) -> Vec<Entry> {
    fn walk(symbols: &[Symbol], depth: usize, out: &mut Vec<Entry>) {
        let mut sorted: Vec<&Symbol> = symbols.iter().collect();
        sorted.sort_by_key(|s| s.range.start);
        for s in sorted {
            out.push(Entry {
                depth,
                name: s.name.clone(),
                detail: s.detail.clone(),
                kind: s.kind,
                line: s.selection.start.line,
                character: s.selection.start.character,
                first_line: s.range.start.line,
                last_line: s.range.end.line,
            });
            walk(&s.children, depth + 1, out);
        }
    }
    let file_class = match symbols {
        [root] if root.kind == SymbolKind::Class && root.name.ends_with(".gd") => Some(root),
        _ => None,
    };
    let mut out = Vec::new();
    match file_class {
        Some(root) => walk(&root.children, 0, &mut out),
        None => walk(symbols, 0, &mut out),
    }
    out
}

/// The innermost entry whose declaration contains `line`.
pub fn current(entries: &[Entry], line: u32) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| (e.first_line..=e.last_line).contains(&line))
        .max_by_key(|(_, e)| e.depth)
        .map(|(index, _)| index)
}

pub const INPUT: &str = "outline-filter";
/// Room for the longest tag (`signal`).
const TAG_WIDTH: f32 = 44.0;

/// The kinds the outline can be filtered by, with their chip labels.
pub const KINDS: [(SymbolKind, &str); 6] = [
    (SymbolKind::Class, "class"),
    (SymbolKind::Function, "func"),
    (SymbolKind::Signal, "signal"),
    (SymbolKind::Variable, "var"),
    (SymbolKind::Constant, "const"),
    (SymbolKind::Enum, "enum"),
];

#[derive(Debug, Clone)]
pub enum FilterMessage {
    Text(String),
    /// Show or hide symbols of this kind.
    ToggleKind(SymbolKind),
    /// Show only symbols of this kind (or every kind, when it is the only one shown).
    OnlyKind(SymbolKind),
    Clear,
}

/// What the outline shows: names containing every word of `text`, of the
/// kinds not hidden.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub text: String,
    pub hidden: Vec<SymbolKind>,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || !self.hidden.is_empty()
    }

    pub fn update(&mut self, message: FilterMessage) {
        match message {
            FilterMessage::Text(text) => self.text = text,
            FilterMessage::ToggleKind(kind) => match self.hidden.iter().position(|&k| k == kind) {
                Some(index) => {
                    self.hidden.remove(index);
                }
                None => self.hidden.push(kind),
            },
            FilterMessage::OnlyKind(kind) => {
                let others: Vec<SymbolKind> = KINDS.iter().map(|&(k, _)| k).filter(|&k| k != kind).collect();
                let only_this = others.iter().all(|k| self.hidden.contains(k)) && !self.hidden.contains(&kind);
                self.hidden = if only_this { Vec::new() } else { others };
            }
            FilterMessage::Clear => *self = Filter::default(),
        }
    }

    pub fn shows_kind(&self, kind: SymbolKind) -> bool {
        // Enum members go with their enum.
        let kind = if kind == SymbolKind::EnumMember { SymbolKind::Enum } else { kind };
        !self.hidden.contains(&kind)
    }

    fn matches(&self, entry: &Entry) -> bool {
        let name = entry.name.to_lowercase();
        self.shows_kind(entry.kind) && self.text.split_whitespace().all(|w| name.contains(&w.to_lowercase()))
    }
}

/// A row of the filtered outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    pub index: usize,
    /// `false`: shown only because something inside it matches.
    pub matched: bool,
    /// Nesting among the rows shown (members of a hidden class move out).
    pub depth: usize,
}

/// The entries to show: those matching the filter, and the ones they are
/// nested in, unless their kind is hidden.
pub fn visible(entries: &[Entry], filter: &Filter) -> Vec<Shown> {
    if !filter.is_active() {
        return entries.iter().enumerate().map(|(index, e)| Shown { index, matched: true, depth: e.depth }).collect();
    }
    let mut shown = vec![None; entries.len()];
    // Indices of the entries enclosing the current one, outermost first.
    let mut enclosing: Vec<usize> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        enclosing.truncate(entry.depth);
        if filter.matches(entry) {
            shown[index] = Some(true);
            for &parent in enclosing.iter().filter(|&&p| filter.shows_kind(entries[p].kind)) {
                shown[parent].get_or_insert(false);
            }
        }
        enclosing.push(index);
    }
    let mut rows = Vec::new();
    enclosing.clear();
    for (index, entry) in entries.iter().enumerate() {
        enclosing.truncate(entry.depth);
        if let Some(matched) = shown[index] {
            let depth = enclosing.iter().filter(|&&p| shown[p].is_some()).count();
            rows.push(Shown { index, matched, depth });
        }
        enclosing.push(index);
    }
    rows
}

/// Short tag (the GDScript keyword) and syntax color for a kind.
pub fn tag(kind: SymbolKind, syntax: &theme::SyntaxColors) -> (&'static str, iced::Color) {
    let (tag, color) = match kind {
        SymbolKind::Class => ("class", syntax.r#type),
        SymbolKind::Function => ("func", syntax.function),
        SymbolKind::Variable => ("var", syntax.text),
        SymbolKind::Constant => ("const", syntax.constant),
        SymbolKind::Enum => ("enum", syntax.r#type),
        SymbolKind::EnumMember => ("", syntax.constant),
        SymbolKind::Signal => ("signal", syntax.annotation),
        SymbolKind::Other => ("", syntax.text),
    };
    (tag, style::to_color(color))
}

pub fn view<'a>(
    outline: Option<&'a Outline>,
    filter: &'a Filter,
    cursor_line: Option<u32>,
    theme: &theme::Theme,
) -> Element<'a, Message> {
    let ui = &theme.ui;
    let muted = style::to_color(ui.text_muted);
    let Some(outline) = outline else {
        let note = "Open a GDScript file to see its functions, variables and signals (needs the language server)";
        return container(text(note).size(12).color(muted)).padding([6, 12]).into();
    };
    if outline.entries.is_empty() {
        return container(text("No symbols in this file").size(12).color(muted)).padding([6, 12]).into();
    }

    let message = |m: FilterMessage| Message::OutlineFilter(m);
    let mut search = row![text_input("Filter by name", &filter.text)
        .id(INPUT)
        .on_input(move |t| message(FilterMessage::Text(t)))
        .size(12)
        .padding([3, 6])]
    .spacing(4)
    .align_y(Alignment::Center);
    if filter.is_active() {
        let clear = button(text("x").size(12)).on_press(message(FilterMessage::Clear)).style(button::text).padding([2, 6]);
        search = search.push(style::tip(clear, "Clear the filter", Position::Bottom, ui));
    }
    // One chip per kind, in its syntax color; faded when hidden.
    let chips = KINDS.iter().map(|&(kind, label)| {
        let shown = filter.shows_kind(kind);
        let color = tag(kind, &theme.syntax).1;
        let chip = button(text(label).size(11).font(Font::MONOSPACE).color(if shown { color } else { color.scale_alpha(0.35) }))
            .on_press(message(FilterMessage::ToggleKind(kind)))
            .style(move |theme: &iced::Theme, status| {
                let base = button::text(theme, status);
                if !shown {
                    return base;
                }
                // Shown: tinted and outlined in the kind's color.
                let tint = if status == button::Status::Hovered { 0.25 } else { 0.12 };
                button::Style {
                    background: Some(Background::Color(color.scale_alpha(tint))),
                    border: Border { color: color.scale_alpha(0.6), width: 1.0, radius: 4.0.into() },
                    ..base
                }
            })
            .padding([1, 6]);
        let chip = iced::widget::mouse_area(chip).on_right_press(message(FilterMessage::OnlyKind(kind)));
        style::tip(chip, "Click: show or hide this kind. Right-click: only this kind", Position::Bottom, ui)
    });
    let chips = Row::with_children(chips).spacing(4).wrap().vertical_spacing(4);
    let header = column![search, chips].spacing(6).padding([0, 8]);

    let here = cursor_line.and_then(|line| current(&outline.entries, line));
    let highlight = style::to_color(ui.accent).scale_alpha(0.2);
    let visible = visible(&outline.entries, filter);
    if visible.is_empty() {
        let note = container(text("No symbol matches the filter").size(12).color(muted)).padding([6, 12]);
        return column![header, note].spacing(4).into();
    }
    let rows = visible.into_iter().map(|Shown { index, matched, depth }| {
        let entry = &outline.entries[index];
        let (tag, color) = tag(entry.kind, &theme.syntax);
        // Shown only because something inside it matches: faded.
        let (color, name_color) = if matched { (color, None) } else { (color.scale_alpha(0.5), Some(muted)) };
        // Tags in one column; only the names are indented.
        let content = row![
            text(tag).size(11).font(Font::MONOSPACE).color(color).width(TAG_WIDTH),
            iced::widget::space().width(14.0 * depth as f32),
            text(entry.name.as_str()).size(13).color_maybe(name_color),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        let selected = here == Some(index);
        button(content)
            .on_press(Message::JumpTo(outline.path.clone(), entry.line, entry.character))
            .width(Length::Fill)
            .padding([2, 8])
            .style(move |theme: &iced::Theme, status| {
                let base = button::text(theme, status);
                if selected {
                    button::Style {
                        background: Some(Background::Color(highlight)),
                        border: Border { radius: 3.0.into(), ..Border::default() },
                        ..base
                    }
                } else {
                    base
                }
            })
            .into()
    });
    let list = scrollable(Column::with_children(rows).width(Length::Fill)).height(Length::Fill);
    column![header, list].spacing(6).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_client::{Position, Range};

    fn symbol(name: &str, kind: SymbolKind, lines: (u32, u32), children: Vec<Symbol>) -> Symbol {
        let range = Range {
            start: Position { line: lines.0, character: 0 },
            end: Position { line: lines.1, character: 0 },
        };
        Symbol { name: name.into(), detail: String::new(), kind, range, selection: range, children }
    }

    #[test]
    fn unwraps_the_file_class_and_finds_the_innermost_entry() {
        let inner = symbol("Inner", SymbolKind::Class, (7, 10), vec![symbol("f", SymbolKind::Function, (9, 10), vec![])]);
        let root = symbol(
            "player.gd",
            SymbolKind::Class,
            (0, 14),
            // Out of order on purpose: shown in document order.
            vec![symbol("_ready", SymbolKind::Function, (12, 13), vec![]), symbol("died", SymbolKind::Signal, (2, 2), vec![]), inner],
        );
        let entries = flatten(&[root]);
        let names: Vec<(usize, &str)> = entries.iter().map(|e| (e.depth, e.name.as_str())).collect();
        assert_eq!(names, [(0, "died"), (0, "Inner"), (1, "f"), (0, "_ready")]);
        assert_eq!(current(&entries, 10).map(|i| entries[i].name.as_str()), Some("f"));
        assert_eq!(current(&entries, 8).map(|i| entries[i].name.as_str()), Some("Inner"));
        assert_eq!(current(&entries, 5), None);
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    fn entry(depth: usize, name: &str, kind: SymbolKind) -> Entry {
        Entry { depth, name: name.into(), detail: String::new(), kind, line: 0, character: 0, first_line: 0, last_line: 0 }
    }

    fn sample() -> Vec<Entry> {
        vec![
            entry(0, "died", SymbolKind::Signal),
            entry(0, "SPEED", SymbolKind::Constant),
            entry(0, "State", SymbolKind::Enum),
            entry(1, "IDLE", SymbolKind::EnumMember),
            entry(0, "Inner", SymbolKind::Class),
            entry(1, "jump_height", SymbolKind::Variable),
            entry(1, "jump", SymbolKind::Function),
            entry(0, "_ready", SymbolKind::Function),
        ]
    }

    fn names(entries: &[Entry], filter: &Filter) -> Vec<(String, bool)> {
        visible(entries, filter).into_iter().map(|s| (entries[s.index].name.clone(), s.matched)).collect()
    }

    #[test]
    fn names_match_case_insensitively_with_their_parents_for_context() {
        let entries = sample();
        assert_eq!(visible(&entries, &Filter::default()).len(), entries.len());
        let filter = Filter { text: "JUMP".into(), ..Filter::default() };
        let shown = names(&entries, &filter);
        assert_eq!(shown, [("Inner".into(), false), ("jump_height".into(), true), ("jump".into(), true)]);
        let depths: Vec<usize> = visible(&entries, &filter).iter().map(|s| s.depth).collect();
        assert_eq!(depths, [0, 1, 1]);
        let filter = Filter { text: "jump h".into(), ..Filter::default() };
        assert_eq!(names(&entries, &filter).last().unwrap().0, "jump_height");
    }

    #[test]
    fn kinds_can_be_hidden_or_shown_alone() {
        let entries = sample();
        let mut filter = Filter::default();
        filter.update(FilterMessage::OnlyKind(SymbolKind::Function));
        // Classes are hidden too, even around a matching method, which moves out.
        let shown = names(&entries, &filter);
        assert_eq!(shown, [("jump".into(), true), ("_ready".into(), true)]);
        assert!(visible(&entries, &filter).iter().all(|s| s.depth == 0));
        // Again: every kind back.
        filter.update(FilterMessage::OnlyKind(SymbolKind::Function));
        assert!(!filter.is_active());

        // Hiding enums hides their members too.
        filter.update(FilterMessage::ToggleKind(SymbolKind::Enum));
        assert!(!names(&entries, &filter).iter().any(|(n, _)| n == "State" || n == "IDLE"));
        filter.update(FilterMessage::ToggleKind(SymbolKind::Enum));
        assert!(!filter.is_active());
    }
}
