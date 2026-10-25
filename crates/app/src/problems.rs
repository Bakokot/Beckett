//! Diagnostics store and the Problems panel.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use iced::widget::{button, container, row, scrollable, text, Column};
use iced::{Element, Length};
use lsp_client::{uri, Diagnostic, Position, Range, Severity};

use crate::{style, Message};

#[derive(Default)]
pub struct Problems {
    /// Keyed by `uri::path_key` so Godot's path spelling matches ours.
    files: BTreeMap<String, (PathBuf, Vec<Diagnostic>)>,
    /// Errors and warnings from the last run of the game, in the order they
    /// first happened, with how many times each one did.
    runtime: Vec<(PathBuf, Diagnostic, usize)>,
}

/// Runtime problems are kept per line; this end column means "to the end".
pub const WHOLE_LINE: u32 = u32::MAX;

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

    /// Records a problem of the running game; a repeat only bumps its count.
    pub fn add_runtime(&mut self, path: PathBuf, line: u32, message: String, error: bool) {
        let existing = self.runtime.iter_mut().find(|(p, d, _)| {
            d.range.start.line == line && d.message == message && uri::same_path(p, &path)
        });
        if let Some((_, _, count)) = existing {
            *count += 1;
            return;
        }
        let severity = if error { Severity::Error } else { Severity::Warning };
        let range = Range {
            start: Position { line, character: 0 },
            end: Position { line, character: WHOLE_LINE },
        };
        self.runtime.push((path, Diagnostic { range, severity, message }, 1));
    }

    pub fn clear_runtime(&mut self) {
        self.runtime.clear();
    }

    /// The language server's diagnostics for `path`, then the runtime ones.
    pub fn for_path(&self, path: &Path) -> Vec<&Diagnostic> {
        let lsp = self.files.get(&uri::path_key(path)).into_iter().flat_map(|(_, d)| d);
        let runtime = self.runtime.iter().filter(|(p, _, _)| uri::same_path(p, path)).map(|(_, d, _)| d);
        lsp.chain(runtime).collect()
    }

    /// Language server diagnostics only; runtime problems outlive the server.
    pub fn clear(&mut self) {
        self.files.clear();
    }

    pub fn counts(&self) -> (usize, usize) {
        let all = self.files.values().flat_map(|(_, d)| d).chain(self.runtime.iter().map(|(_, d, _)| d));
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
        for (path, d, count) in &self.runtime {
            let (label, color) = match d.severity {
                Severity::Error => ("runtime error", ui.danger),
                _ => ("runtime warning", ui.warning),
            };
            let line = d.range.start.line;
            let repeats = if *count > 1 { format!("  (×{count})") } else { String::new() };
            rows.push(
                button(
                    row![
                        text(label).size(12).color(style::to_color(color)).width(110),
                        text(format!("{}:{}", display_name(path), line + 1)).size(12).color(muted),
                        text(format!("{}{repeats}", d.message)).size(12),
                    ]
                    .spacing(12),
                )
                .on_press(Message::JumpTo(path.clone(), line, 0))
                .style(button::text)
                .width(Length::Fill)
                .padding([2, 8])
                .into(),
            );
        }
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
            // (Runtime rows come first; this is only reached with none at all.)
            rows.push(container(text("No problems").size(12).color(muted)).padding([2, 8]).into());
        }
        scrollable(Column::with_children(rows).width(Length::Fill))
            .height(Length::Fill)
            .into()
    }
}
