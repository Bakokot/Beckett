//! Diagnostics store and the Problems panel.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use iced::widget::{button, column, container, row, scrollable, space, text, Column};
use iced::{Alignment, Element, Length};
use lsp_client::{uri, Diagnostic, Severity};

use crate::{style, Message};

#[derive(Default)]
pub struct Problems {
    /// Keyed by `uri::path_key` so Godot's path spelling matches ours.
    files: BTreeMap<String, (PathBuf, Vec<Diagnostic>)>,
}

impl Problems {
    pub fn set(&mut self, path: PathBuf, mut diagnostics: Vec<Diagnostic>) {
        let key = uri::path_key(&path);
        if diagnostics.is_empty() {
            self.files.remove(&key);
        } else {
            diagnostics.sort_by_key(|d| (d.severity, d.range.start));
            self.files.insert(key, (path, diagnostics));
        }
    }

    pub fn clear(&mut self) {
        self.files.clear();
    }

    pub fn counts(&self) -> (usize, usize) {
        let all = self.files.values().flat_map(|(_, d)| d);
        all.fold((0, 0), |(errors, warnings), d| match d.severity {
            Severity::Error => (errors + 1, warnings),
            Severity::Warning => (errors, warnings + 1),
            _ => (errors, warnings),
        })
    }

    pub fn view<'a>(
        &'a self,
        ui: &theme::UiColors,
        display_name: impl Fn(&Path) -> String,
    ) -> Element<'a, Message> {
        let muted = style::to_color(ui.text_muted);
        let mut rows: Vec<Element<'a, Message>> = Vec::new();
        for (path, diagnostics) in self.files.values() {
            let name = display_name(path);
            for d in diagnostics {
                let (label, color) = match d.severity {
                    Severity::Error => ("error", ui.danger),
                    Severity::Warning => ("warning", ui.warning),
                    Severity::Information | Severity::Hint => ("info", ui.text_muted),
                };
                let start = d.range.start;
                rows.push(
                    button(
                        row![
                            text(label).size(12).color(style::to_color(color)).width(56),
                            text(format!("{name}:{}:{}", start.line + 1, start.character + 1))
                                .size(12)
                                .color(muted),
                            text(&d.message).size(12),
                        ]
                        .spacing(12),
                    )
                    .on_press(Message::JumpTo(path.clone(), start.line, start.character))
                    .style(button::text)
                    .width(Length::Fill)
                    .padding([2, 8])
                    .into(),
                );
            }
        }
        if rows.is_empty() {
            rows.push(container(text("No problems").size(12).color(muted)).padding([2, 8]).into());
        }

        let header = row![
            text("PROBLEMS").size(12),
            space::horizontal(),
            button(text("x").size(12))
                .on_press(Message::ToggleProblems)
                .style(button::text)
                .padding([0, 6]),
        ]
        .align_y(Alignment::Center)
        .padding([4, 8]);

        container(column![header, scrollable(Column::with_children(rows)).height(Length::Fill)])
            .width(Length::Fill)
            .height(180)
            .style(style::surface(ui))
            .into()
    }
}
