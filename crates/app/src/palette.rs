//! Quick open (Ctrl+P), the command palette (`>`, Ctrl+Shift+P) and go to
//! line (`:`, Ctrl+G), symbols of the file (`@`, Ctrl+Shift+O) or of the
//! project (`#`, Ctrl+T), all in one box. The input's first character
//! picks the mode, as in VS Code.

use std::path::{Path, PathBuf};

use editor_core::fuzzy::fuzzy_match;
use iced::mouse;
use iced::widget::{button, column, container, mouse_area, row, space, text, text_input, Column, Row};
use iced::{Background, Border, Element, Font, Length, Shadow, Vector};

use crate::style;

pub const INPUT: &str = "palette-input";
const SHOWN: usize = 12;
/// Files listed for an empty query (open tabs first).
const MAX_FILES: usize = 20_000;

#[derive(Debug, Clone)]
pub enum PaletteMessage {
    Input(String),
    Move(isize),
    /// Mouse wheel over the list, in rows (positive is down).
    Scroll(isize),
    Accept,
    /// A clicked row, as an index into the visible rows.
    AcceptRow(usize),
    Close,
}

/// What choosing an item does; the app turns it into a message.
#[derive(Debug, Clone)]
pub enum Action<M> {
    Open(PathBuf),
    GotoLine(u32, u32),
    Run(M),
}

#[derive(Debug, Clone)]
pub struct Item<M> {
    pub label: String,
    /// Char indices of `label` that matched the query.
    pub matched: Vec<usize>,
    /// Shown dimmed on the right (folder, shortcut).
    pub detail: String,
    pub action: Action<M>,
}

/// A palette command: name, shortcut hint, and the message it sends.
pub struct CommandEntry<M> {
    pub name: String,
    pub shortcut: String,
    pub message: M,
}

/// One option when the palette asks to pick among a few things.
#[derive(Debug, Clone)]
pub struct Choice<M> {
    pub label: String,
    pub detail: String,
    pub message: M,
}

pub struct Palette<M> {
    pub input: String,
    /// Asking to pick one of these (with this prompt) instead of the
    /// files / commands / line modes.
    choices: Option<(&'static str, Vec<Choice<M>>)>,
    /// Symbols of the current file, for the `@` mode.
    symbols: Vec<Choice<M>>,
    /// Symbols of every script, for the `#` mode; loaded when first needed.
    project_symbols: Option<Vec<Choice<M>>>,
    pub items: Vec<Item<M>>,
    pub selected: usize,
    /// First visible row; the wheel moves it independently of `selected`.
    scroll: usize,
    /// Project files relative to the root, with `/` separators.
    files: Vec<String>,
    root: PathBuf,
    /// Open tabs (relative), listed first for an empty query.
    recent: Vec<String>,
}

impl<M: Clone> Palette<M> {
    pub fn new(root: &Path, open: &[PathBuf], input: String) -> Self {
        let mut files = Vec::new();
        collect(root, root, &mut files);
        files.sort();
        let recent = open.iter().filter_map(|p| relative(root, p)).collect();
        Self { input, choices: None, symbols: Vec::new(), project_symbols: None, items: Vec::new(), selected: 0, scroll: 0, files, root: root.to_path_buf(), recent }
    }

    /// Asks to pick one of `choices`; typing filters them.
    pub fn choose(prompt: &'static str, choices: Vec<Choice<M>>) -> Self {
        Self {
            input: String::new(),
            choices: Some((prompt, choices)),
            symbols: Vec::new(),
            project_symbols: None,
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            files: Vec::new(),
            root: PathBuf::new(),
            recent: Vec::new(),
        }
    }

    /// The current file's symbols, listed in the `@` mode.
    pub fn set_symbols(&mut self, symbols: Vec<Choice<M>>) {
        self.symbols = symbols;
    }

    /// Whether the `#` mode is on and its symbols still need loading
    /// (see [`Self::scripts`] and [`Self::set_project_symbols`]).
    pub fn wants_project_symbols(&self) -> bool {
        self.choices.is_none() && self.input.starts_with('#') && self.project_symbols.is_none()
    }

    /// The project's GDScript files.
    pub fn scripts(&self) -> Vec<PathBuf> {
        self.files.iter().filter(|f| f.ends_with(".gd")).map(|f| self.root.join(f)).collect()
    }

    pub fn set_project_symbols(&mut self, symbols: Vec<Choice<M>>) {
        self.project_symbols = Some(symbols);
    }

    /// Rebuilds the list for the current input.
    pub fn refresh(&mut self, commands: &[CommandEntry<M>]) {
        self.selected = 0;
        self.scroll = 0;
        self.items = if let Some((_, choices)) = &self.choices {
            choice_items(self.input.trim(), choices)
        } else if let Some(query) = self.input.strip_prefix('@') {
            choice_items(query.trim(), &self.symbols)
        } else if let Some(query) = self.input.strip_prefix('#') {
            choice_items(query.trim(), self.project_symbols.as_deref().unwrap_or_default())
        } else if let Some(query) = self.input.strip_prefix('>') {
            command_items(query.trim(), commands)
        } else if let Some(target) = self.input.strip_prefix(':') {
            line_items(target.trim())
        } else {
            self.file_items(self.input.trim())
        };
    }

    /// Moves the selection (keyboard), scrolling it into view.
    pub fn move_selection(&mut self, delta: isize) {
        let last = self.items.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + SHOWN {
            self.scroll = self.selected + 1 - SHOWN;
        }
    }

    /// Scrolls the list (mouse wheel) without moving the selection.
    pub fn scroll_by(&mut self, rows: isize) {
        let max = self.items.len().saturating_sub(SHOWN) as isize;
        self.scroll = (self.scroll as isize + rows).clamp(0, max) as usize;
    }

    /// Index of the first visible row.
    pub fn first_shown(&self) -> usize {
        self.scroll
    }

    pub fn placeholder(&self) -> &'static str {
        if let Some((prompt, _)) = &self.choices {
            return prompt;
        }
        match self.input.chars().next() {
            Some('>') => "Type a command",
            Some(':') => "Line number, optionally :column",
            Some('@') => "Go to a function, variable or signal of this file",
            Some('#') => "Go to a function, class, variable or signal of any script",
            _ => "Search files by name  (> commands, @ symbols, # in project, : line)",
        }
    }

    fn file_items(&self, query: &str) -> Vec<Item<M>> {
        let item = |path: &str, matched: Vec<usize>| {
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
            Item {
                label: name.to_string(),
                matched,
                detail: dir.to_string(),
                action: Action::Open(self.root.join(path)),
            }
        };
        if query.is_empty() {
            let rest = self.files.iter().filter(|f| !self.recent.contains(f));
            return self.recent.iter().chain(rest).take(MAX_FILES).map(|f| item(f, Vec::new())).collect();
        }
        let mut scored: Vec<(i64, usize, Item<M>)> = self
            .files
            .iter()
            .filter_map(|path| {
                let name_start = path.rfind('/').map_or(0, |i| i + 1);
                let name = &path[name_start..];
                // A match on the file name alone beats one spread over the path.
                if let Some(m) = fuzzy_match(query, name) {
                    return Some((m.score + 30, path.len(), item(path, m.positions)));
                }
                let m = fuzzy_match(query, path)?;
                let name_chars = path[..name_start].chars().count();
                let in_name = m.positions.iter().filter_map(|&p| p.checked_sub(name_chars)).collect();
                Some((m.score, path.len(), item(path, in_name)))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, item)| item).collect()
    }

    pub fn view<'a>(&'a self, ui: &theme::UiColors) -> Element<'a, PaletteMessage> {
        let muted = style::to_color(ui.text_muted);
        let accent = style::to_color(ui.accent);
        let input = text_input(self.placeholder(), &self.input)
            .id(INPUT)
            .on_input(PaletteMessage::Input)
            .on_submit(PaletteMessage::Accept)
            .size(14)
            .padding([6, 8]);

        let first = self.first_shown();
        let (selected_background, text_color) = (accent.scale_alpha(0.25), style::to_color(ui.text));
        let rows = self.items.iter().enumerate().skip(first).take(SHOWN).map(|(index, item)| {
            // A light accent tint keeps the accent-colored matched letters readable.
            let selected = index == self.selected;
            let style = move |theme: &iced::Theme, status| {
                if selected {
                    button::Style {
                        background: Some(Background::Color(selected_background)),
                        text_color,
                        border: Border { radius: 4.0.into(), ..Border::default() },
                        ..button::Style::default()
                    }
                } else {
                    button::text(theme, status)
                }
            };
            let content = row![
                highlighted(&item.label, &item.matched, accent),
                space::horizontal(),
                text(&item.detail).size(12).color(muted),
            ]
            .spacing(12);
            button(content)
                .on_press(PaletteMessage::AcceptRow(index - first))
                .style(style)
                .width(Length::Fill)
                .padding([3, 8])
                .into()
        });
        let mut list = Column::with_children(rows);
        if self.items.is_empty() {
            list = list.push(container(text("No matches").size(13).color(muted)).padding([3, 8]));
        } else if self.items.len() > SHOWN {
            let last = (first + SHOWN).min(self.items.len());
            let position = format!("{}-{} of {}", first + 1, last, self.items.len());
            list = list.push(container(text(position).size(11).color(muted)).padding([2, 8]));
        }
        // Wheel over the list scrolls it, three rows per notch.
        let list = mouse_area(list).on_scroll(|delta| {
            let rows = match delta {
                mouse::ScrollDelta::Lines { y, .. } => -(y * 3.0),
                mouse::ScrollDelta::Pixels { y, .. } => -(y / 20.0),
            };
            PaletteMessage::Scroll(rows.round() as isize)
        });

        let (background, border) = (style::to_color(ui.surface), style::to_color(ui.border));
        container(column![input, list].spacing(4))
            .width(600)
            .padding(6)
            .style(move |_| container::Style {
                background: Some(Background::Color(background)),
                border: Border { color: border, width: 1.0, radius: 6.0.into() },
                shadow: Shadow { color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4), offset: Vector::new(0.0, 4.0), blur_radius: 16.0 },
                ..container::Style::default()
            })
            .into()
    }
}

impl<M: Clone> Palette<M> {
    /// The visible row's item.
    pub fn row(&self, row: usize) -> Option<&Item<M>> {
        self.items.get(self.first_shown() + row)
    }

    pub fn selected_item(&self) -> Option<&Item<M>> {
        self.items.get(self.selected)
    }
}

fn command_items<M: Clone>(query: &str, commands: &[CommandEntry<M>]) -> Vec<Item<M>> {
    let mut scored: Vec<(i64, usize, Item<M>)> = commands
        .iter()
        .enumerate()
        .filter_map(|(order, command)| {
            let m = fuzzy_match(query, &command.name)?;
            let item = Item {
                label: command.name.clone(),
                matched: m.positions,
                detail: command.shortcut.clone(),
                action: Action::Run(command.message.clone()),
            };
            Some((m.score, order, item))
        })
        .collect();
    // Empty query: keep the declared order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

fn choice_items<M: Clone>(query: &str, choices: &[Choice<M>]) -> Vec<Item<M>> {
    let mut scored: Vec<(i64, usize, Item<M>)> = choices
        .iter()
        .enumerate()
        .filter_map(|(order, choice)| {
            let m = fuzzy_match(query, &choice.label)?;
            let item = Item {
                label: choice.label.clone(),
                matched: m.positions,
                detail: choice.detail.clone(),
                action: Action::Run(choice.message.clone()),
            };
            Some((m.score, order, item))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

fn line_items<M>(target: &str) -> Vec<Item<M>> {
    let mut parts = target.split(':');
    let line = parts.next().and_then(|l| l.trim().parse::<u32>().ok()).filter(|&l| l > 0);
    let column = parts.next().and_then(|c| c.trim().parse::<u32>().ok()).filter(|&c| c > 0);
    match line {
        Some(line) => {
            let label = match column {
                Some(column) => format!("Go to line {line}, column {column}"),
                None => format!("Go to line {line}"),
            };
            vec![Item {
                label,
                matched: Vec::new(),
                detail: String::new(),
                action: Action::GotoLine(line - 1, column.map_or(0, |c| c - 1)),
            }]
        }
        None => Vec::new(),
    }
}

/// `label` with matched characters in the accent color.
fn highlighted<'a, Message: 'a>(label: &str, matched: &[usize], accent: iced::Color) -> Element<'a, Message> {
    let mut parts: Vec<Element<'a, Message>> = Vec::new();
    let mut run = String::new();
    let mut run_matched = false;
    let flush = |parts: &mut Vec<Element<'a, Message>>, run: &mut String, is_match: bool| {
        if run.is_empty() {
            return;
        }
        let piece = text(std::mem::take(run)).size(13).font(Font::MONOSPACE);
        parts.push(if is_match { piece.color(accent).into() } else { piece.into() });
    };
    for (i, c) in label.chars().enumerate() {
        let is_match = matched.contains(&i);
        if is_match != run_matched {
            flush(&mut parts, &mut run, run_matched);
            run_matched = is_match;
        }
        run.push(c);
    }
    flush(&mut parts, &mut run, run_matched);
    Row::with_children(parts).into()
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if godot_project::is_hidden(&name, kind.is_dir()) || name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect(root, &path, out);
        } else if let Some(rel) = relative(root, &path) {
            out.push(rel);
            if out.len() >= MAX_FILES {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(files: &[&str]) -> Palette<u8> {
        Palette {
            input: String::new(),
            choices: None,
            symbols: Vec::new(),
            project_symbols: None,
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            files: files.iter().map(|f| f.to_string()).collect(),
            root: PathBuf::from("/game"),
            recent: vec!["scenes/main.gd".into()],
        }
    }

    #[test]
    fn files_rank_name_matches_first_and_recent_lead_empty_queries() {
        let mut p = palette(&["player/ui.gd", "scenes/main.gd", "scenes/player.gd"]);
        p.input = "player".into();
        p.refresh(&[]);
        assert_eq!(p.items[0].label, "player.gd");
        assert_eq!(p.items[0].detail, "scenes");
        assert_eq!(p.items[0].matched, (0..6).collect::<Vec<_>>());

        p.input = String::new();
        p.refresh(&[]);
        assert_eq!(p.items[0].label, "main.gd");
        assert_eq!(p.items.len(), 3);
    }

    #[test]
    fn at_lists_the_file_symbols() {
        let mut p = palette(&["a.gd"]);
        let choice = |label: &str, message| Choice { label: label.into(), detail: String::new(), message };
        p.set_symbols(vec![choice("speed", 1), choice("_physics_process", 2), choice("jump", 3)]);
        p.input = "@".into();
        p.refresh(&[]);
        assert_eq!(p.items.len(), 3);
        p.input = "@phys".into();
        p.refresh(&[]);
        assert!(matches!(p.items[0].action, Action::Run(2)));
    }

    #[test]
    fn hash_lists_the_project_symbols_once_loaded() {
        let mut p = palette(&["a.gd", "b.tscn", "enemies/slime.gd"]);
        p.input = "#".into();
        assert!(p.wants_project_symbols());
        assert_eq!(p.scripts(), [PathBuf::from("/game").join("a.gd"), PathBuf::from("/game").join("enemies/slime.gd")]);
        let choice = |label: &str, message| Choice { label: label.into(), detail: String::new(), message };
        p.set_project_symbols(vec![choice("take_damage", 1), choice("jump", 2)]);
        assert!(!p.wants_project_symbols());
        p.input = "#jmp".into();
        p.refresh(&[]);
        assert_eq!(p.items.len(), 1);
        assert!(matches!(p.items[0].action, Action::Run(2)));
    }

    #[test]
    fn choices_ignore_prefixes_and_filter_by_label() {
        let choice = |label: &str, message| Choice { label: label.into(), detail: String::new(), message };
        let mut p = Palette::choose("Pick", vec![choice("res://levels/forest.tscn", 1), choice("res://levels/cave.tscn", 2)]);
        p.refresh(&[]);
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.placeholder(), "Pick");
        p.input = "cave".into();
        p.refresh(&[]);
        assert_eq!(p.items.len(), 1);
        assert!(matches!(p.items[0].action, Action::Run(2)));
        // ">" and ":" are just text here, not mode switches.
        p.input = ":".into();
        p.refresh(&[]);
        assert!(p.items.iter().all(|i| matches!(i.action, Action::Run(_))));
    }

    #[test]
    fn wheel_scrolls_without_moving_the_selection_and_keys_scroll_into_view() {
        let files: Vec<String> = (0..40).map(|i| format!("f{i:02}.gd")).collect();
        let mut p = palette(&files.iter().map(String::as_str).collect::<Vec<_>>());
        p.recent.clear();
        p.refresh(&[]);
        assert_eq!(p.items.len(), 40);

        p.scroll_by(5);
        assert_eq!((p.first_shown(), p.selected), (5, 0));
        p.scroll_by(100);
        assert_eq!(p.first_shown(), 40 - SHOWN);
        p.scroll_by(-100);
        assert_eq!(p.first_shown(), 0);

        p.move_selection(SHOWN as isize);
        assert_eq!(p.selected, SHOWN);
        assert_eq!(p.first_shown(), 1);
        assert_eq!(p.row(SHOWN - 1).unwrap().label, p.items[SHOWN].label);
        p.move_selection(-(SHOWN as isize));
        assert_eq!(p.first_shown(), 0);
    }

    #[test]
    fn modes_follow_the_prefix() {
        let commands = [
            CommandEntry { name: "Save".into(), shortcut: "Ctrl+S".into(), message: 1 },
            CommandEntry { name: "Toggle Terminal".into(), shortcut: "Ctrl+J".into(), message: 2 },
        ];
        let mut p = palette(&[]);
        p.input = ">term".into();
        p.refresh(&commands);
        assert_eq!(p.items.len(), 1);
        assert!(matches!(p.items[0].action, Action::Run(2)));

        p.input = ":12:4".into();
        p.refresh(&commands);
        assert!(matches!(p.items[0].action, Action::GotoLine(11, 3)));
        p.input = ":abc".into();
        p.refresh(&commands);
        assert!(p.items.is_empty());
    }
}
