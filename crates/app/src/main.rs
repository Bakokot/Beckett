#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod explorer;
mod highlighter;
mod lsp;
mod problems;
mod style;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use editor_core::Language;
use godot_project::GodotProject;
use iced::widget::{
    button, center, column, container, horizontal_space, pick_list, row, scrollable, text,
    text_editor, Row,
};
use iced::{keyboard, Alignment, Element, Font, Length, Subscription, Task};
use lsp_client::{uri, Event, Position};

use explorer::Explorer;
use highlighter::GdHighlighter;
use lsp::{Lsp, LspEvent};
use problems::Problems;

fn main() -> iced::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    iced::application(App::title, App::update, App::view)
        .theme(App::theme)
        .subscription(App::subscription)
        .window_size((1280.0, 800.0))
        .run_with(move || App::new(initial))
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenFolderDialog,
    FolderPicked(Option<PathBuf>),
    ToggleDir(PathBuf),
    OpenFile(PathBuf),
    SelectTab(usize),
    CloseTab(usize),
    CloseActiveTab,
    Edit(text_editor::Action),
    Save,
    ThemeSelected(String),
    Lsp(LspEvent),
    RestartLsp,
    GotoDefinition,
    ToggleProblems,
    /// Open a file and put the cursor at a zero-based line/character.
    JumpTo(PathBuf, u32, u32),
}

/// What an in-flight LSP request was for.
enum Pending {
    Definition,
}

struct App {
    project: Option<Project>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    themes: Vec<theme::Theme>,
    theme_index: usize,
    iced_theme: iced::Theme,
    /// Dirty tab the user already tried to close once; a second close discards it.
    pending_close: Option<PathBuf>,
    status: Option<String>,
    lsp: Lsp,
    /// Bumped to restart the language server subscription.
    lsp_generation: u64,
    pending: HashMap<u64, Pending>,
    problems: Problems,
    show_problems: bool,
}

struct Project {
    root: PathBuf,
    godot: Option<GodotProject>,
    explorer: Explorer,
}

struct Tab {
    path: PathBuf,
    language: Language,
    content: text_editor::Content,
    dirty: bool,
    /// LSP document version, bumped on every edit.
    version: i32,
}

impl Tab {
    fn title(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

impl App {
    fn new(initial: Option<PathBuf>) -> (Self, Task<Message>) {
        let mut themes = theme::builtin();
        let mut status = None;
        if let Some(dir) = theme::user_themes_dir() {
            let (user, errors) = theme::load_dir(&dir);
            themes.extend(user);
            if let Some(error) = errors.first() {
                status = Some(format!("Theme error: {error}"));
            }
        }
        let iced_theme = style::iced_theme(&themes[0]);

        let mut app = Self {
            project: None,
            tabs: Vec::new(),
            active: None,
            themes,
            theme_index: 0,
            iced_theme,
            pending_close: None,
            status,
            lsp: Lsp::Off,
            lsp_generation: 0,
            pending: HashMap::new(),
            problems: Problems::default(),
            show_problems: false,
        };
        if let Some(path) = initial {
            app.open_path(path);
        }
        (app, Task::none())
    }

    fn title(&self) -> String {
        let mut parts = Vec::new();
        if let Some(tab) = self.active_tab() {
            parts.push(format!("{}{}", tab.title(), if tab.dirty { " ●" } else { "" }));
        }
        if let Some(project) = &self.project {
            parts.push(self.project_name(project));
        }
        parts.push("Godot IDE".to_string());
        parts.join(" — ")
    }

    fn theme(&self) -> iced::Theme {
        self.iced_theme.clone()
    }

    fn current_theme(&self) -> &theme::Theme {
        &self.themes[self.theme_index]
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = keyboard::on_key_press(|key, modifiers| match key.as_ref() {
            keyboard::Key::Named(keyboard::key::Named::F12) => Some(Message::GotoDefinition),
            keyboard::Key::Character("s") if modifiers.command() => Some(Message::Save),
            keyboard::Key::Character("o") if modifiers.command() => Some(Message::OpenFolderDialog),
            keyboard::Key::Character("w") if modifiers.command() => Some(Message::CloseActiveTab),
            _ => None,
        });

        let lsp = self
            .project
            .as_ref()
            .and_then(|project| {
                let godot = project.godot.as_ref()?;
                let connection =
                    lsp::connect(project.root.clone(), godot.godot_version(), self.lsp_generation);
                Some(connection.map(Message::Lsp))
            })
            .unwrap_or_else(Subscription::none);

        Subscription::batch([keys, lsp])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenFolderDialog => {
                return Task::perform(pick_folder(), Message::FolderPicked);
            }
            Message::FolderPicked(Some(path)) => self.open_path(path),
            Message::FolderPicked(None) => {}
            Message::ToggleDir(path) => {
                if let Some(project) = &mut self.project {
                    project.explorer.toggle(&path);
                }
            }
            Message::OpenFile(path) => {
                self.open_file(path);
            }
            Message::SelectTab(index) => {
                if index < self.tabs.len() {
                    self.active = Some(index);
                }
            }
            Message::CloseTab(index) => self.close_tab(index),
            Message::CloseActiveTab => {
                if let Some(index) = self.active {
                    self.close_tab(index);
                }
            }
            Message::Edit(action) => {
                if let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
                    let is_edit = action.is_edit();
                    tab.content.perform(action);
                    if is_edit {
                        tab.dirty = true;
                        tab.version += 1;
                        if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
                            let _ = client.did_change(&tab.path, tab.version, &tab.content.text());
                        }
                    }
                }
            }
            Message::Save => self.save_active(),
            Message::ThemeSelected(name) => {
                if let Some(index) = self.themes.iter().position(|t| t.name == name) {
                    self.theme_index = index;
                    self.iced_theme = style::iced_theme(&self.themes[index]);
                }
            }
            Message::Lsp(event) => self.on_lsp(event),
            Message::RestartLsp => {
                if self.project.as_ref().is_some_and(|p| p.godot.is_some()) {
                    self.reset_lsp(Lsp::Starting);
                }
            }
            Message::GotoDefinition => self.goto_definition(),
            Message::ToggleProblems => self.show_problems = !self.show_problems,
            Message::JumpTo(path, line, character) => self.jump_to(path, line, character),
        }
        Task::none()
    }

    fn on_lsp(&mut self, event: LspEvent) {
        match event {
            LspEvent::Connected { client, label } => {
                for tab in self.tabs.iter().filter(|t| t.language == Language::GDScript) {
                    let _ = client.did_open(&tab.path, tab.version, &tab.content.text());
                }
                self.lsp = Lsp::Ready { client, label };
            }
            LspEvent::Failed(error) => {
                self.status = Some(error);
                self.lsp = Lsp::Failed;
            }
            LspEvent::Event(Event::Diagnostics { path, diagnostics }) => {
                self.problems.set(path, diagnostics);
            }
            LspEvent::Event(Event::Response { id, result }) => match self.pending.remove(&id) {
                Some(Pending::Definition) => match result {
                    Ok(value) => match lsp_client::locations(&value).into_iter().next() {
                        Some(location) => {
                            let start = location.range.start;
                            self.jump_to(location.path, start.line, start.character);
                        }
                        None => self.status = Some("No definition found".to_string()),
                    },
                    Err(error) => self.status = Some(format!("Go to definition failed: {error}")),
                },
                None => {}
            },
            LspEvent::Event(Event::Disconnected(reason)) => {
                if let Some(reason) = reason {
                    self.status = Some(format!("Language server disconnected: {reason}"));
                }
                self.lsp = Lsp::Disconnected;
                self.pending.clear();
                self.problems.clear();
            }
            LspEvent::Event(Event::Notification { .. }) => {}
        }
    }

    /// Drops the current connection (stopping a spawned Godot) and lets the
    /// subscription start over.
    fn reset_lsp(&mut self, state: Lsp) {
        self.lsp = state;
        self.lsp_generation += 1;
        self.pending.clear();
        self.problems.clear();
    }

    fn goto_definition(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        if tab.language != Language::GDScript {
            return;
        }
        let Some(client) = self.lsp.client() else {
            self.status = Some("Language server is not connected".to_string());
            return;
        };
        let (line, column) = tab.content.cursor_position();
        let position = Position {
            line: line as u32,
            character: column as u32,
        };
        match client.definition(&tab.path, position) {
            Ok(id) => {
                self.pending.insert(id, Pending::Definition);
            }
            Err(error) => self.status = Some(format!("Go to definition failed: {error}")),
        }
    }

    fn jump_to(&mut self, path: PathBuf, line: u32, character: u32) {
        if !self.open_file(path) {
            return;
        }
        if let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
            move_cursor(&mut tab.content, line, character);
        }
    }

    /// Opens a project folder, or a file together with its project.
    fn open_path(&mut self, path: PathBuf) {
        let (dir, file) = if path.is_file() {
            (path.parent().map(Path::to_path_buf).unwrap_or_default(), Some(path))
        } else {
            (path, None)
        };
        let root = godot_project::find_root(&dir).unwrap_or(dir);
        let godot = GodotProject::load(&root).ok();
        self.status = godot
            .is_none()
            .then(|| "No project.godot found in this folder".to_string());
        self.reset_lsp(if godot.is_some() { Lsp::Starting } else { Lsp::Off });
        self.project = Some(Project {
            explorer: Explorer::new(root.clone()),
            root,
            godot,
        });
        self.tabs.clear();
        self.active = None;
        if let Some(file) = file {
            self.open_file(file);
        }
    }

    /// Opens (or focuses) a file tab. Returns whether it is now active.
    fn open_file(&mut self, path: PathBuf) -> bool {
        if let Some(index) = self.tabs.iter().position(|t| uri::same_path(&t.path, &path)) {
            self.active = Some(index);
            return true;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let tab = Tab {
                    language: Language::from_path(&path),
                    content: text_editor::Content::with_text(&text),
                    path,
                    dirty: false,
                    version: 1,
                };
                if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
                    let _ = client.did_open(&tab.path, tab.version, &text);
                }
                self.tabs.push(tab);
                self.active = Some(self.tabs.len() - 1);
                true
            }
            Err(error) => {
                self.status = Some(format!("Cannot open {}: {error}", path.display()));
                false
            }
        }
    }

    fn close_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if tab.dirty && self.pending_close.as_ref() != Some(&tab.path) {
            self.pending_close = Some(tab.path.clone());
            self.status = Some(format!(
                "{} has unsaved changes. Close again to discard them.",
                tab.title()
            ));
            return;
        }
        if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
            let _ = client.did_close(&tab.path);
        }
        self.pending_close = None;
        self.tabs.remove(index);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) => Some(active.min(self.tabs.len() - 1)),
            None => None,
        };
    }

    fn save_active(&mut self) {
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else {
            return;
        };
        let text = tab.content.text();
        match std::fs::write(&tab.path, &text) {
            Ok(()) => {
                tab.dirty = false;
                self.status = Some(format!("Saved {}", tab.title()));
                if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
                    let _ = client.did_save(&tab.path, &text);
                }
            }
            Err(error) => {
                self.status = Some(format!("Cannot save {}: {error}", tab.title()));
            }
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|i| self.tabs.get(i))
    }

    fn project_name(&self, project: &Project) -> String {
        project
            .godot
            .as_ref()
            .and_then(GodotProject::name)
            .or_else(|| project.root.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| project.root.display().to_string())
    }

    /// `res://` path when inside the project, file name otherwise.
    fn display_path(&self, path: &Path) -> String {
        self.project
            .as_ref()
            .and_then(|p| p.godot.as_ref())
            .and_then(|g| g.to_res_path(path))
            .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| path.display().to_string())
    }

    fn view(&self) -> Element<'_, Message> {
        let body: Element<'_, Message> = match &self.project {
            None => self.welcome(),
            Some(project) => {
                let mut main = column![self.editor_area()];
                if self.show_problems {
                    let ui = &self.current_theme().ui;
                    main = main.push(self.problems.view(ui, |p| self.display_path(p)));
                }
                row![self.sidebar(project), main].into()
            }
        };
        column![container(body).height(Length::Fill), self.status_bar()].into()
    }

    fn welcome(&self) -> Element<'_, Message> {
        let muted = style::to_color(self.current_theme().ui.text_muted);
        center(
            column![
                text("Godot IDE").size(28),
                text("Open a folder containing project.godot").color(muted),
                button(text("Open Folder…")).on_press(Message::OpenFolderDialog),
            ]
            .spacing(12)
            .align_x(Alignment::Center),
        )
        .into()
    }

    fn sidebar<'a>(&'a self, project: &'a Project) -> Element<'a, Message> {
        let header = row![
            text(self.project_name(project).to_uppercase()).size(12),
            horizontal_space(),
            button(text("Open…").size(12))
                .on_press(Message::OpenFolderDialog)
                .style(button::text)
                .padding([2, 6]),
        ]
        .align_y(Alignment::Center)
        .padding([6, 8]);

        let active = self.active_tab().map(|t| t.path.as_path());
        let tree = scrollable(project.explorer.view(active)).height(Length::Fill);

        container(column![header, tree])
            .width(260)
            .height(Length::Fill)
            .style(style::surface(&self.current_theme().ui))
            .into()
    }

    fn editor_area(&self) -> Element<'_, Message> {
        let Some(tab) = self.active_tab() else {
            let muted = style::to_color(self.current_theme().ui.text_muted);
            return center(text("Open a file from the explorer").color(muted)).into();
        };

        let editor = text_editor(&tab.content)
            .on_action(Message::Edit)
            .font(Font::MONOSPACE)
            .size(14)
            .padding(8)
            .height(Length::Fill);
        let editor: Element<'_, Message> = match tab.language {
            Language::GDScript => editor
                .highlight_with::<GdHighlighter>(
                    self.current_theme().syntax,
                    highlighter::to_format,
                )
                .into(),
            Language::Plain => editor.into(),
        };

        column![self.tab_bar(), editor].into()
    }

    fn tab_bar(&self) -> Element<'_, Message> {
        let tabs = self.tabs.iter().enumerate().map(|(index, tab)| {
            let label = format!("{}{}", tab.title(), if tab.dirty { " *" } else { "" });
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if self.active == Some(index) {
                    button::secondary
                } else {
                    button::text
                };
            row![
                button(text(label).size(13))
                    .on_press(Message::SelectTab(index))
                    .style(style)
                    .padding([4, 10]),
                button(text("×").size(13))
                    .on_press(Message::CloseTab(index))
                    .style(button::text)
                    .padding([4, 6]),
            ]
            .into()
        });

        container(Row::with_children(tabs).spacing(2))
            .width(Length::Fill)
            .style(style::surface(&self.current_theme().ui))
            .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let ui = &self.current_theme().ui;
        let mut left: Vec<Element<'_, Message>> = Vec::new();
        if let Some(project) = &self.project {
            left.push(text(self.project_name(project)).size(12).into());
            if let Some(version) = project.godot.as_ref().and_then(GodotProject::godot_version) {
                left.push(text(format!("Godot {version}")).size(12).into());
            }
            if project.godot.is_some() {
                let (errors, warnings) = self.problems.counts();
                left.push(
                    button(
                        row![
                            text(format!("E {errors}")).size(12).color(style::to_color(ui.danger)),
                            text(format!("W {warnings}")).size(12).color(style::to_color(ui.warning)),
                        ]
                        .spacing(8),
                    )
                    .on_press(Message::ToggleProblems)
                    .style(button::text)
                    .padding([0, 4])
                    .into(),
                );
                left.push(
                    button(text(self.lsp.label()).size(12))
                        .on_press(Message::RestartLsp)
                        .style(button::text)
                        .padding([0, 4])
                        .into(),
                );
            }
        }
        if let Some(status) = &self.status {
            left.push(text(status).size(12).color(style::to_color(ui.text_muted)).into());
        }

        let mut right: Vec<Element<'_, Message>> = Vec::new();
        if let Some(tab) = self.active_tab() {
            let (line, column) = tab.content.cursor_position();
            right.push(text(format!("Ln {}, Col {}", line + 1, column + 1)).size(12).into());
            right.push(text(tab.language.name()).size(12).into());
        }
        let names: Vec<String> = self.themes.iter().map(|t| t.name.clone()).collect();
        right.push(
            pick_list(
                names,
                Some(self.current_theme().name.clone()),
                Message::ThemeSelected,
            )
            .text_size(12)
            .padding([2, 6])
            .into(),
        );

        container(
            row![
                Row::with_children(left).spacing(16).align_y(Alignment::Center),
                horizontal_space(),
                Row::with_children(right).spacing(16).align_y(Alignment::Center),
            ]
            .align_y(Alignment::Center)
            .padding([2, 8]),
        )
        .width(Length::Fill)
        .style(style::surface(ui))
        .into()
    }
}

/// Places the cursor at a zero-based line/character using relative motions
/// (iced 0.13's editor has no absolute cursor setter).
fn move_cursor(content: &mut text_editor::Content, line: u32, character: u32) {
    use text_editor::{Action, Motion};
    content.perform(Action::Move(Motion::DocumentStart));
    for _ in 0..line {
        content.perform(Action::Move(Motion::Down));
    }
    for _ in 0..character {
        content.perform(Action::Move(Motion::Right));
    }
}

async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Open Godot project")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}
