//! The outline of the active GDScript file (functions, variables, signals…)
//! from the language server, shown in the sidebar and in the palette (`@`).

use std::path::PathBuf;

use iced::widget::{button, container, row, scrollable, text, Column};
use iced::{Background, Border, Element, Font, Length};
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

/// Short tag and syntax color for a kind, as in the completion list.
pub fn tag(kind: SymbolKind, syntax: &theme::SyntaxColors) -> (&'static str, iced::Color) {
    let (tag, color) = match kind {
        SymbolKind::Class => ("cls", syntax.r#type),
        SymbolKind::Function => ("fn", syntax.function),
        SymbolKind::Variable => ("var", syntax.text),
        SymbolKind::Constant => ("const", syntax.constant),
        SymbolKind::Enum => ("enum", syntax.r#type),
        SymbolKind::EnumMember => ("", syntax.constant),
        SymbolKind::Signal => ("sig", syntax.annotation),
        SymbolKind::Other => ("", syntax.text),
    };
    (tag, style::to_color(color))
}

pub fn view<'a>(outline: Option<&'a Outline>, cursor_line: Option<u32>, theme: &theme::Theme) -> Element<'a, Message> {
    let ui = &theme.ui;
    let muted = style::to_color(ui.text_muted);
    let Some(outline) = outline else {
        let note = "Open a GDScript file to see its functions, variables and signals (needs the language server)";
        return container(text(note).size(12).color(muted)).padding([6, 12]).into();
    };
    if outline.entries.is_empty() {
        return container(text("No symbols in this file").size(12).color(muted)).padding([6, 12]).into();
    }
    let here = cursor_line.and_then(|line| current(&outline.entries, line));
    let highlight = style::to_color(ui.accent).scale_alpha(0.2);
    let rows = outline.entries.iter().enumerate().map(|(index, entry)| {
        let (tag, color) = tag(entry.kind, &theme.syntax);
        let content = row![
            text(tag).size(11).font(Font::MONOSPACE).color(color).width(36),
            text(entry.name.as_str()).size(13),
        ]
        .spacing(4);
        let selected = here == Some(index);
        button(content)
            .on_press(Message::JumpTo(outline.path.clone(), entry.line, entry.character))
            .width(Length::Fill)
            .padding(iced::Padding { left: 8.0 + 14.0 * entry.depth as f32, right: 8.0, top: 2.0, bottom: 2.0 })
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
    scrollable(Column::with_children(rows).width(Length::Fill)).height(Length::Fill).into()
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
