//! The keyboard shortcuts page (F1): every binding, grouped and filterable.

use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, space, text, text_input, Column};
use iced::{Alignment, Background, Border, Element, Font, Length};

use crate::style;

pub const INPUT: &str = "shortcuts-filter";

/// Sections of (keys, what they do). Keys of one binding are joined by
/// `+`; alternatives are separated by ` / `.
pub const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "General",
        &[
            ("Ctrl+O", "Open a project folder"),
            ("Ctrl+S", "Save the current file"),
            ("Ctrl+W", "Close the current tab (twice to discard unsaved edits)"),
            ("Ctrl+P", "Go to file (fuzzy search)"),
            ("Ctrl+Shift+P", "Command palette"),
            ("Ctrl+G", "Go to line (type :line or :line:column)"),
            ("F1", "Keyboard shortcuts (this page)"),
            ("Ctrl+= / Ctrl+- / Ctrl+0", "Zoom the interface in / out / back to 100%"),
        ],
    ),
    (
        "Editing",
        &[
            ("Ctrl+Z", "Undo"),
            ("Ctrl+Y / Ctrl+Shift+Z", "Redo"),
            ("Ctrl+X / Ctrl+C / Ctrl+V", "Cut, copy, paste (with several cursors, pasted lines are spread)"),
            ("Ctrl+A", "Select all"),
            ("Ctrl+K / Ctrl+/", "Toggle comment"),
            ("Ctrl+Shift+K", "Delete line"),
            ("Alt+Up / Alt+Down", "Move line up / down"),
            ("Shift+Alt+Up / Shift+Alt+Down", "Duplicate line"),
            ("Tab / Shift+Tab", "Indent / outdent"),
            ("Enter", "New line, keeping indentation (indents after : and brackets)"),
            ("Ctrl+Backspace / Ctrl+Delete", "Delete the previous / next word"),
        ],
    ),
    (
        "Moving around",
        &[
            ("Ctrl+Left / Ctrl+Right", "Previous / next word"),
            ("Home", "First non-blank character, then start of line"),
            ("End", "End of line"),
            ("Ctrl+Home / Ctrl+End", "Start / end of the file"),
            ("PageUp / PageDown", "Scroll a page"),
            ("Shift+any move", "Extend the selection"),
        ],
    ),
    (
        "Selection and multiple cursors",
        &[
            ("Shift+Click", "Extend the selection to the click"),
            ("Double-click / Triple-click", "Select word / line"),
            ("Alt+Click", "Add a cursor"),
            ("Alt+Drag", "Column (box) selection, one cursor per line"),
            ("Ctrl+Alt+Up / Ctrl+Alt+Down", "Add a cursor above / below"),
            ("Ctrl+D", "Select the next occurrence of the selection"),
            ("Escape", "Back to a single cursor"),
        ],
    ),
    (
        "Code intelligence (GDScript)",
        &[
            ("Ctrl+Space", "Show completions"),
            ("Up / Down / PageUp / PageDown", "Move in the completion list"),
            ("Enter / Tab", "Accept the completion"),
            ("Escape", "Close the completion, hover or signature popup"),
            ("Tab / Shift+Tab", "Next / previous placeholder of a snippet (tagged snip in completions)"),
            ("F12 / Ctrl+Click", "Go to definition, or open the res:// or uid:// path in a string"),
            ("Ctrl (hold)", "Underline what Ctrl+Click can open under the mouse"),
            ("Shift+F12", "Find all references"),
            ("F2", "Rename symbol in every file"),
            ("Mouse rest", "Documentation of the word under the mouse"),
        ],
    ),
    (
        "Find and search",
        &[
            ("Ctrl+F", "Find in the file (the selection fills the box)"),
            ("Ctrl+H", "Find and replace"),
            ("Enter / Shift+Enter", "Next / previous match (in the find box)"),
            ("F3 / Shift+F3", "Next / previous match"),
            ("Escape", "Close the find bar"),
            ("Ctrl+Shift+F", "Search in the whole project (Enter to run)"),
        ],
    ),
    (
        "Run",
        &[
            ("F5", "Run the project's main scene"),
            ("F6", "Run the current scene, or the scene using this script (asks once when several do)"),
            ("F8 / Shift+F5", "Stop the game"),
        ],
    ),
    (
        "Terminal",
        &[
            ("Ctrl+J / Ctrl+` / Ctrl+²", "Show or hide the terminal"),
            ("Ctrl+C", "Copy the selection, or interrupt when nothing is selected"),
            ("Ctrl+V", "Paste"),
            ("Double-click / Triple-click", "Select word / line"),
            ("Mouse wheel", "Scroll back through the output"),
        ],
    ),
    (
        "Command palette",
        &[
            ("Up / Down / PageUp / PageDown", "Move in the list"),
            ("Enter", "Open the file or run the command"),
            ("Escape / Click outside", "Close"),
            ("> prefix", "Search commands instead of files"),
            (": prefix", "Go to line"),
        ],
    ),
];

#[derive(Debug, Clone)]
pub enum ShortcutsMessage {
    Filter(String),
    Close,
}

#[derive(Default)]
pub struct Shortcuts {
    pub filter: String,
}

impl Shortcuts {
    /// Sections with their entries matching the filter (in the keys, the
    /// description or the section name); empty sections are left out.
    pub fn matching(&self) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
        let words: Vec<String> = self.filter.split_whitespace().map(str::to_lowercase).collect();
        SECTIONS
            .iter()
            .map(|&(section, entries)| {
                let entries = entries
                    .iter()
                    .copied()
                    .filter(|(keys, description)| {
                        let haystack = format!("{section} {keys} {description}").to_lowercase();
                        words.iter().all(|w| haystack.contains(w.as_str()))
                    })
                    .collect::<Vec<_>>();
                (section, entries)
            })
            .filter(|(_, entries)| !entries.is_empty())
            .collect()
    }

    pub fn view<'a>(&'a self, ui: &theme::UiColors) -> Element<'a, ShortcutsMessage> {
        let muted = style::to_color(ui.text_muted);
        let accent = style::to_color(ui.accent);
        let (cap_background, cap_border, cap_text) =
            (style::to_color(ui.surface), style::to_color(ui.border), style::to_color(ui.text));
        // One bordered "key cap" per binding; alternatives in between.
        let keycaps = move |keys: &'static str| {
            let mut caps = row![].spacing(4).align_y(Alignment::Center);
            for (index, binding) in keys.split(" / ").enumerate() {
                if index > 0 {
                    caps = caps.push(text("or").size(11).color(muted));
                }
                let cap = container(text(binding).size(12).font(Font::MONOSPACE).color(cap_text))
                    .padding([1, 6])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(cap_background)),
                        border: Border { color: cap_border, width: 1.0, radius: 4.0.into() },
                        ..container::Style::default()
                    });
                caps = caps.push(cap);
            }
            caps
        };

        let close = button(text("x").size(12)).on_press(ShortcutsMessage::Close).style(button::text).padding([2, 6]);
        let header = row![
            text("Keyboard Shortcuts").size(20),
            space::horizontal(),
            text_input("Filter (e.g. \"cursor\" or \"F5\")", &self.filter)
                .id(INPUT)
                .on_input(ShortcutsMessage::Filter)
                .size(13)
                .padding([3, 6])
                .width(280),
            style::tip(close, "Close (Escape)", Position::Bottom, ui),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let sections = self.matching();
        let mut list: Vec<Element<'a, ShortcutsMessage>> = Vec::new();
        if sections.is_empty() {
            list.push(text("No shortcut matches the filter").size(13).color(muted).into());
        }
        for (section, entries) in sections {
            list.push(container(text(section.to_uppercase()).size(12).color(accent)).padding([12, 0]).into());
            for (keys, description) in entries {
                list.push(
                    row![container(keycaps(keys)).width(320), text(description).size(13)]
                        .spacing(16)
                        .align_y(Alignment::Center)
                        .padding([3, 0])
                        .into(),
                );
            }
        }
        // Right padding keeps the scrollbar off the descriptions.
        let list = Column::with_children(list).padding(iced::Padding { left: 4.0, right: 24.0, ..iced::Padding::ZERO });
        let body = scrollable(list).height(Length::Fill);

        container(column![header, body].spacing(12).max_width(900))
            .padding([16, 24])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_matches_keys_descriptions_and_sections() {
        let mut page = Shortcuts::default();
        let total: usize = SECTIONS.iter().map(|(_, e)| e.len()).sum();
        assert_eq!(page.matching().iter().map(|(_, e)| e.len()).sum::<usize>(), total);

        page.filter = "f12".to_string();
        let keys: Vec<&str> = page.matching().iter().flat_map(|(_, e)| e.iter().map(|(k, _)| *k)).collect();
        assert_eq!(keys, ["F12 / Ctrl+Click", "Shift+F12"]);

        page.filter = "RUN stop".to_string();
        let found = page.matching();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, [("F8 / Shift+F5", "Stop the game")]);

        page.filter = "nothing like this".to_string();
        assert!(page.matching().is_empty());
    }

    #[test]
    fn entries_are_filled_in() {
        for (section, entries) in SECTIONS {
            assert!(!entries.is_empty(), "{section}");
            for (keys, description) in *entries {
                assert!(!keys.trim().is_empty() && !description.trim().is_empty(), "{section}: {keys}");
            }
        }
    }
}
