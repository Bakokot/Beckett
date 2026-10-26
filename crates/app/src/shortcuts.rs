//! The keyboard shortcuts page (F1): the commands with their (changeable)
//! shortcuts, then the other keys and mouse gestures, all filterable.

use iced::widget::tooltip::Position;
use iced::widget::{button, column, container, row, scrollable, space, text, text_input, Column};
use iced::{Alignment, Background, Border, Element, Font, Length};

use crate::keymap::{Keymap, Preset, COMMANDS};
use crate::style;

pub const INPUT: &str = "shortcuts-filter";

/// Keys and gestures that are not commands (fixed). Keys of one binding
/// are joined by `+`; alternatives are separated by ` / `.
pub const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Editing",
        &[
            ("Ctrl+X / Ctrl+C / Ctrl+V", "Cut, copy, paste (with several cursors, pasted lines are spread)"),
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
        "Mouse",
        &[
            ("Shift+Click", "Extend the selection to the click"),
            ("Double-click / Triple-click", "Select word / line"),
            ("Alt+Click", "Add a cursor"),
            ("Alt+Drag", "Column (box) selection, one cursor per line"),
            ("Ctrl+Click", "Go to definition, or open the res:// or uid:// path in a string"),
            ("Ctrl (hold)", "Underline what Ctrl+Click can open under the mouse"),
            ("Right-click", "Menu of the file, tab or code under the mouse"),
            ("Click the gutter", "Fold or unfold the block (indentation or #region)"),
            ("Mouse rest", "Documentation of the word under the mouse"),
            ("Escape", "Back to a single cursor"),
        ],
    ),
    (
        "Popups and snippets",
        &[
            ("Up / Down / PageUp / PageDown", "Move in the completion list"),
            ("Enter / Tab", "Accept the completion"),
            ("Escape", "Close the completion, hover or signature popup"),
            ("Tab / Shift+Tab", "Next / previous placeholder of a snippet (tagged snip in completions)"),
            ("Enter / Shift+Enter", "Next / previous match (in the find box)"),
        ],
    ),
    (
        "Terminal",
        &[
            ("Ctrl+C", "Copy the selection, or interrupt when nothing is selected"),
            ("Ctrl+V", "Paste"),
            ("Double-click / Triple-click", "Select word / line"),
            ("Shift+Drag", "Select text while a program uses the mouse"),
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
            ("@ prefix", "Go to a symbol of this file"),
            (": prefix", "Go to line"),
        ],
    ),
];

#[derive(Debug, Clone)]
pub enum ShortcutsMessage {
    Filter(String),
    /// Wait for the keys of this command's new shortcut.
    Record(&'static str),
    /// Back to the preset's shortcut.
    Reset(&'static str),
    /// Start over from a preset (drops the changes).
    Preset(Preset),
    Close,
}

#[derive(Default)]
pub struct Shortcuts {
    pub filter: String,
    /// The command whose new shortcut the next key press sets.
    pub recording: Option<&'static str>,
}

impl Shortcuts {
    fn words(&self) -> Vec<String> {
        self.filter.split_whitespace().map(str::to_lowercase).collect()
    }

    /// Commands matching the filter (title, id or keys).
    pub fn matching_commands(&self, keymap: &Keymap) -> Vec<&'static str> {
        let words = self.words();
        COMMANDS
            .iter()
            .filter(|c| {
                let keys: Vec<String> = keymap.chords(c.id).iter().map(ToString::to_string).collect();
                let haystack = format!("{} {} {}", c.title, c.id, keys.join(" ")).to_lowercase();
                words.iter().all(|w| haystack.contains(w.as_str()))
            })
            .map(|c| c.id)
            .collect()
    }

    /// Reference sections with their entries matching the filter (in the
    /// keys, the description or the section name); empty ones left out.
    pub fn matching(&self) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
        let words = self.words();
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

    pub fn view<'a>(&'a self, ui: &theme::UiColors, keymap: &'a Keymap) -> Element<'a, ShortcutsMessage> {
        let muted = style::to_color(ui.text_muted);
        let accent = style::to_color(ui.accent);
        let (cap_background, cap_border, cap_text) =
            (style::to_color(ui.surface), style::to_color(ui.border), style::to_color(ui.text));
        let cap = move |label: String| {
            container(text(label).size(12).font(Font::MONOSPACE).color(cap_text))
                .padding([1, 6])
                .style(move |_| container::Style {
                    background: Some(Background::Color(cap_background)),
                    border: Border { color: cap_border, width: 1.0, radius: 4.0.into() },
                    ..container::Style::default()
                })
        };
        // One key cap per binding; alternatives in between.
        let keycaps = move |keys: Vec<String>| {
            let mut caps = row![].spacing(4).align_y(Alignment::Center);
            for (index, binding) in keys.into_iter().enumerate() {
                if index > 0 {
                    caps = caps.push(text("or").size(11).color(muted));
                }
                caps = caps.push(cap(binding));
            }
            caps
        };

        let close = button(text("x").size(12)).on_press(ShortcutsMessage::Close).style(button::text).padding([2, 6]);
        let mut presets = row![text("Preset").size(12).color(muted)].spacing(4).align_y(Alignment::Center);
        for preset in Preset::ALL {
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if keymap.preset() == preset { button::secondary } else { button::text };
            let choose = button(text(preset.name()).size(12)).on_press(ShortcutsMessage::Preset(preset)).style(style).padding([2, 8]);
            presets = presets.push(style::tip(choose, "Use these shortcuts (your changes are dropped)", Position::Bottom, ui));
        }
        let header = row![
            text("Keyboard Shortcuts").size(20),
            space::horizontal(),
            presets,
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

        let mut list: Vec<Element<'a, ShortcutsMessage>> = Vec::new();
        let section = |title: String| -> Element<'a, ShortcutsMessage> {
            container(text(title).size(12).color(accent)).padding([12, 0]).into()
        };

        let commands = self.matching_commands(keymap);
        if !commands.is_empty() {
            list.push(section("COMMANDS  (click a shortcut to change it)".to_string()));
        }
        for id in commands {
            let title = COMMANDS.iter().find(|c| c.id == id).map_or("", |c| c.title);
            let keys: Vec<String> = keymap.chords(id).iter().map(ToString::to_string).collect();
            let shortcut: Element<'a, ShortcutsMessage> = if self.recording == Some(id) {
                text("Press the new keys…  (Escape cancels, Backspace removes)").size(12).color(accent).into()
            } else if keys.is_empty() {
                text("none").size(12).color(muted).into()
            } else {
                keycaps(keys).into()
            };
            let change = button(shortcut).on_press(ShortcutsMessage::Record(id)).style(button::text).padding([1, 2]);
            let mut line = row![container(change).width(380), text(title).size(13)].spacing(16).align_y(Alignment::Center);
            if !keymap.is_default(id) {
                let reset = button(text("reset").size(11)).on_press(ShortcutsMessage::Reset(id)).style(button::text).padding([0, 6]);
                line = line.push(style::tip(reset, "Back to the preset's shortcut", Position::Top, ui));
            }
            list.push(line.padding([1, 0]).into());
        }

        let sections = self.matching();
        for (title, entries) in sections {
            list.push(section(title.to_uppercase()));
            for (keys, description) in entries {
                let keys = keys.split(" / ").map(str::to_string).collect();
                list.push(
                    row![container(keycaps(keys)).width(380), text(description).size(13)]
                        .spacing(16)
                        .align_y(Alignment::Center)
                        .padding([3, 0])
                        .into(),
                );
            }
        }
        if list.is_empty() {
            list.push(text("No shortcut matches the filter").size(13).color(muted).into());
        }
        // Right padding keeps the scrollbar off the descriptions.
        let list = Column::with_children(list).padding(iced::Padding { left: 4.0, right: 24.0, ..iced::Padding::ZERO });
        let body = scrollable(list).height(Length::Fill);

        container(column![header, body].spacing(12).max_width(1000))
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
    fn filter_matches_commands_by_title_id_or_keys() {
        let keymap = Keymap::defaults();
        let mut page = Shortcuts::default();
        assert_eq!(page.matching_commands(&keymap).len(), COMMANDS.len());

        page.filter = "f12".to_string();
        assert_eq!(page.matching_commands(&keymap), ["go.definition", "go.references"]);

        page.filter = "run stop".to_string();
        assert_eq!(page.matching_commands(&keymap), ["run.stop"]);

        page.filter = "nothing like this".to_string();
        assert!(page.matching_commands(&keymap).is_empty() && page.matching().is_empty());
    }

    #[test]
    fn reference_lists_only_fixed_keys() {
        let keymap = Keymap::defaults();
        for (section, entries) in SECTIONS {
            assert!(!entries.is_empty(), "{section}");
            for (keys, description) in *entries {
                assert!(!keys.trim().is_empty() && !description.trim().is_empty(), "{section}: {keys}");
                // A key listed here must not also be a command's shortcut.
                for key in keys.split(" / ") {
                    if let Some(chord) = crate::keymap::Chord::parse(key) {
                        assert_eq!(keymap.lookup(&chord), None, "{key} is a command: list it there");
                    }
                }
            }
        }
    }
}
