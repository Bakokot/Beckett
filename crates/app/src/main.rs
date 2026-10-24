#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod explorer;
mod highlighter;
mod style;

use std::path::{Path, PathBuf};

use editor_core::Language;
use godot_project::GodotProject;
use iced::widget::{
    button, center, column, container, horizontal_space, pick_list, row, scrollable, text,
    text_editor, Row,
};
use iced::{keyboard, Alignment, Element, Font, Length, Subscription, Task};

use explorer::Explorer;
use highlighter::GdHighlighter;

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
        keyboard::on_key_press(|key, modifiers| {
            if !modifiers.command() {
                return None;
            }
            match key.as_ref() {
                keyboard::Key::Character("s") => Some(Message::Save),
                keyboard::Key::Character("o") => Some(Message::OpenFolderDialog),
                keyboard::Key::Character("w") => Some(Message::CloseActiveTab),
                _ => None,
            }
        })
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
            Message::OpenFile(path) => self.open_file(path),
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
                    if action.is_edit() {
                        tab.dirty = true;
                    }
                    tab.content.perform(action);
                }
            }
            Message::Save => self.save_active(),
            Message::ThemeSelected(name) => {
                if let Some(index) = self.themes.iter().position(|t| t.name == name) {
                    self.theme_index = index;
                    self.iced_theme = style::iced_theme(&self.themes[index]);
                }
            }
        }
        Task::none()
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

    fn open_file(&mut self, path: PathBuf) {
        if let Some(index) = self.tabs.iter().position(|t| t.path == path) {
            self.active = Some(index);
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.tabs.push(Tab {
                    language: Language::from_path(&path),
                    content: text_editor::Content::with_text(&text),
                    path,
                    dirty: false,
                });
                self.active = Some(self.tabs.len() - 1);
            }
            Err(error) => {
                self.status = Some(format!("Cannot open {}: {error}", path.display()));
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
        match std::fs::write(&tab.path, tab.content.text()) {
            Ok(()) => {
                tab.dirty = false;
                self.status = Some(format!("Saved {}", tab.title()));
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

    fn view(&self) -> Element<'_, Message> {
        let body: Element<'_, Message> = match &self.project {
            None => self.welcome(),
            Some(project) => row![self.sidebar(project), self.editor_area()].into(),
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
        let mut left: Vec<Element<'_, Message>> = Vec::new();
        if let Some(project) = &self.project {
            left.push(text(self.project_name(project)).size(12).into());
            if let Some(version) = project.godot.as_ref().and_then(GodotProject::godot_version) {
                left.push(text(format!("Godot {version}")).size(12).into());
            }
        }
        if let Some(status) = &self.status {
            left.push(text(status).size(12).into());
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
        .style(style::surface(&self.current_theme().ui))
        .into()
    }
}

async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Open Godot project")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}
