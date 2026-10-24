#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod code_editor;
mod completion;
mod docs;
mod explorer;
mod lsp;
mod problems;
mod runner;
mod style;
mod terminal_panel;
mod terminal_view;

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use editor_core::{Buffer, Command, Language, Pos};
use godot_project::GodotProject;
use iced::widget::{button, center, column, container, pick_list, row, scrollable, space, text, Row};
use iced::{keyboard, Alignment, Element, Length, Subscription, Task};
use lsp_client::{uri, CompletionKind, Event, Position, Severity};

use code_editor::{
    CodeEditor, CompletionAction, CompletionPopup, CompletionRow, EditorEvent, HoverPopup, Mark,
    SignaturePopup,
};
use completion::Completion;
use docs::Doc;
use explorer::Explorer;
use lsp::{Lsp, LspEvent};
use problems::Problems;
use runner::{OutputLine, RunEvent, RunHandle, RunRequest};
use terminal_panel::TerminalEvent;
use terminal_view::TerminalView;

fn main() -> iced::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    iced::application(move || App::new(initial.clone()), App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .window_size((1280.0, 800.0))
        .run()
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
    Edit(EditorEvent),
    Save,
    ThemeSelected(String),
    Lsp(LspEvent),
    RestartLsp,
    GotoDefinition,
    ToggleProblems,
    /// Open a file and put the cursor at a zero-based line/character.
    JumpTo(PathBuf, u32, u32),
    ToggleTerminal,
    KillTerminal,
    FocusTerminal,
    Terminal(TerminalEvent),
    ShowPanel(Panel),
    ClosePanel,
    /// Run the main scene (F5).
    RunProject,
    /// Run the scene of the active tab (F6).
    RunScene,
    StopRun,
    Run(RunEvent),
    ClearOutput,
    /// A `res://` link in the output: path and 1-based line.
    OpenOutputLink(String, u32),
}

/// Which panel receives keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Editor,
    Terminal,
}

/// Tabs of the bottom panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Problems,
    Output,
    Terminal,
}

#[derive(Default)]
struct TerminalState {
    session: Option<std::sync::Arc<terminal::Terminal>>,
    title: Option<String>,
    /// Bumped to start a new shell.
    generation: u64,
}

const MAX_OUTPUT_LINES: usize = 5000;

#[derive(Default)]
struct RunState {
    /// The current run; its subscription lives until the process exits.
    request: Option<RunRequest>,
    /// Dropping it stops the game.
    handle: Option<std::sync::Arc<RunHandle>>,
    output: Vec<OutputLine>,
    next_id: u64,
    /// Godot binary for running, found on first use.
    godot: Option<PathBuf>,
}

/// What an in-flight LSP request was for.
enum Pending {
    Definition,
    Completion,
    /// Docs for `items[index]` of the current completion.
    Resolve(usize),
    Hover(Pos),
    Signature(Pos),
}

struct Signature {
    label: String,
    active: Option<Range<usize>>,
    anchor: Pos,
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
    /// Open tab of the bottom panel, if it is shown.
    panel: Option<Panel>,
    run: RunState,
    // Popups for the active tab.
    completion: Option<Completion>,
    hover: Option<(Pos, Doc)>,
    signature: Option<Signature>,
    terminal: TerminalState,
    focus: Focus,
}

struct Project {
    root: PathBuf,
    godot: Option<GodotProject>,
    explorer: Explorer,
}

struct Tab {
    path: PathBuf,
    language: Language,
    buffer: Buffer,
    dirty: bool,
    /// LSP document version, bumped on every edit.
    version: i32,
    /// Diagnostic underlines, rebuilt when diagnostics or the theme change.
    marks: Vec<Mark>,
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
            panel: None,
            run: RunState::default(),
            completion: None,
            hover: None,
            signature: None,
            terminal: TerminalState::default(),
            focus: Focus::Editor,
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
        let keys = keyboard::listen().filter_map(shortcut);

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

        // The shell keeps running while its panel is hidden.
        let terminal = self
            .project
            .as_ref()
            .filter(|_| self.panel == Some(Panel::Terminal) || self.terminal.session.is_some())
            .map(|project| {
                let version = project.godot.as_ref().and_then(GodotProject::godot_version);
                terminal_panel::spawn(self.terminal.generation, project.root.clone(), version)
                    .map(Message::Terminal)
            })
            .unwrap_or_else(Subscription::none);

        let run = self
            .run
            .request
            .clone()
            .map(|request| runner::run(request).map(Message::Run))
            .unwrap_or_else(Subscription::none);

        Subscription::batch([keys, lsp, terminal, run])
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
                self.dismiss_popups();
                self.open_file(path);
            }
            Message::SelectTab(index) => {
                if index < self.tabs.len() {
                    self.dismiss_popups();
                    self.active = Some(index);
                }
            }
            Message::CloseTab(index) => {
                self.dismiss_popups();
                self.close_tab(index);
            }
            Message::CloseActiveTab => {
                if let Some(index) = self.active {
                    self.dismiss_popups();
                    self.close_tab(index);
                }
            }
            Message::Edit(event) => self.on_editor(event),
            Message::Save => self.save_active(),
            Message::ThemeSelected(name) => {
                if let Some(index) = self.themes.iter().position(|t| t.name == name) {
                    self.theme_index = index;
                    self.iced_theme = style::iced_theme(&self.themes[index]);
                    self.refresh_marks();
                }
            }
            Message::Lsp(event) => self.on_lsp(event),
            Message::RestartLsp => {
                if self.project.as_ref().is_some_and(|p| p.godot.is_some()) {
                    self.reset_lsp(Lsp::Starting);
                }
            }
            Message::GotoDefinition => self.goto_definition(),
            Message::ToggleProblems => self.toggle_panel(Panel::Problems),
            Message::JumpTo(path, line, character) => {
                self.dismiss_popups();
                self.jump_to(path, line, character);
            }
            Message::ToggleTerminal => {
                if self.project.is_some() {
                    self.toggle_panel(Panel::Terminal);
                }
            }
            Message::ShowPanel(panel) => self.show_panel(panel),
            Message::ClosePanel => {
                self.panel = None;
                self.focus = Focus::Editor;
            }
            Message::RunProject => self.start_run(None),
            Message::RunScene => match self.current_scene() {
                Ok(scene) => self.start_run(Some(scene)),
                Err(reason) => self.status = Some(reason),
            },
            Message::StopRun => {
                if self.run.handle.take().is_some() {
                    self.push_output(OutputLine::info("Stopping..."));
                }
            }
            Message::Run(event) => self.on_run(event),
            Message::ClearOutput => self.run.output.clear(),
            Message::OpenOutputLink(res_path, line) => {
                let path = self
                    .project
                    .as_ref()
                    .and_then(|p| p.godot.as_ref())
                    .and_then(|g| g.resolve(&res_path));
                if let Some(path) = path {
                    self.dismiss_popups();
                    self.focus = Focus::Editor;
                    self.jump_to(path, line.saturating_sub(1), 0);
                }
            }
            Message::KillTerminal => self.reset_terminal(),
            Message::FocusTerminal => {
                self.focus = Focus::Terminal;
                self.dismiss_popups();
            }
            Message::Terminal(event) => match event {
                TerminalEvent::Ready(session) => self.terminal.session = Some(session),
                // Nothing to update: the view redraws from the live grid.
                TerminalEvent::Wakeup => {}
                TerminalEvent::Title(title) => self.terminal.title = Some(title),
                TerminalEvent::Exit => self.reset_terminal(),
                TerminalEvent::Failed(error) => {
                    self.status = Some(format!("Cannot start terminal: {error}"));
                    self.reset_terminal();
                }
            },
        }
        Task::none()
    }

    fn show_panel(&mut self, panel: Panel) {
        self.panel = Some(panel);
        self.focus = if panel == Panel::Terminal { Focus::Terminal } else { Focus::Editor };
        self.dismiss_popups();
    }

    fn toggle_panel(&mut self, panel: Panel) {
        if self.panel == Some(panel) {
            self.panel = None;
            self.focus = Focus::Editor;
        } else {
            self.show_panel(panel);
        }
    }

    /// The `res://` scene to run for the active tab: the scene itself, or
    /// a scene its script is attached to.
    fn current_scene(&self) -> Result<String, String> {
        let godot = self
            .project
            .as_ref()
            .and_then(|p| p.godot.as_ref())
            .ok_or("Not a Godot project")?;
        let tab = self.active_tab().ok_or("No file open")?;
        let res_path = godot.to_res_path(&tab.path).ok_or("File is outside the project")?;
        if res_path.ends_with(".tscn") {
            return Ok(res_path);
        }
        let scenes = godot.scenes_using(&res_path);
        let scene = scenes.first().ok_or_else(|| format!("No scene uses {res_path}"))?;
        godot.to_res_path(scene).ok_or_else(|| "Scene is outside the project".to_string())
    }

    fn start_run(&mut self, scene: Option<String>) {
        let Some(project) = &self.project else {
            return;
        };
        let Some(godot) = &project.godot else {
            self.status = Some("Not a Godot project (no project.godot)".to_string());
            return;
        };
        if self.run.godot.is_none() {
            let installs = lsp_client::godot::discover();
            let wanted = godot.godot_version();
            self.run.godot = lsp_client::godot::pick(&installs, wanted.as_deref())
                .map(|install| lsp_client::godot::console_variant(&install.path));
        }
        let Some(exe) = self.run.godot.clone() else {
            self.status = Some("No Godot executable found (set GODOT_BIN)".to_string());
            return;
        };
        let root = project.root.clone();
        self.save_all();

        // Replacing the handle stops a game that is still running.
        self.run.handle = None;
        self.run.next_id += 1;
        let request = RunRequest { id: self.run.next_id, godot: exe, root, scene };
        self.run.output.clear();
        self.push_output(OutputLine::info(format!("> {}", request.command_line())));
        self.run.request = Some(request);
        self.show_panel(Panel::Output);
    }

    fn on_run(&mut self, event: RunEvent) {
        match event {
            RunEvent::Started(handle) => self.run.handle = Some(handle),
            RunEvent::Line(line) => self.push_output(line),
            RunEvent::Exited(code) => {
                let message = match code {
                    Some(code) => format!("Process exited with code {code}"),
                    None => "Process stopped".to_string(),
                };
                self.push_output(OutputLine::info(message));
                self.run.handle = None;
                self.run.request = None;
            }
            RunEvent::Failed(error) => {
                self.push_output(OutputLine { kind: runner::LineKind::Error, ..OutputLine::info(error) });
                self.run.request = None;
            }
        }
    }

    fn push_output(&mut self, line: OutputLine) {
        let output = &mut self.run.output;
        output.push(line);
        if output.len() > MAX_OUTPUT_LINES {
            output.drain(..output.len() - MAX_OUTPUT_LINES);
        }
    }

    fn is_running(&self) -> bool {
        self.run.handle.is_some()
    }

    /// Ends the shell (if any) and hides its panel; the next toggle starts
    /// a fresh one.
    fn reset_terminal(&mut self) {
        let generation = self.terminal.generation + 1;
        self.terminal = TerminalState { generation, ..TerminalState::default() };
        if self.panel == Some(Panel::Terminal) {
            self.panel = None;
        }
        self.focus = Focus::Editor;
    }

    fn on_editor(&mut self, event: EditorEvent) {
        match event {
            EditorEvent::Command(command) => {
                let typed = match &command {
                    Command::Insert(text) => text.chars().last(),
                    _ => None,
                };
                let backspace = matches!(command, Command::Backspace { .. });
                self.apply_command(command);
                self.hover = None;
                match typed {
                    Some(c) => self.after_typing(c),
                    None if backspace => self.refilter_completion(),
                    None => {
                        self.completion = None;
                        self.signature = None;
                    }
                }
            }
            EditorEvent::Completion(action) => match action {
                CompletionAction::Move(delta) => {
                    if let Some(c) = &mut self.completion {
                        let last = c.filtered.len().saturating_sub(1) as isize;
                        c.selected = (c.selected as isize + delta).clamp(0, last) as usize;
                        c.doc = None;
                    }
                    self.resolve_selected();
                }
                CompletionAction::Accept => self.accept_completion(None),
                CompletionAction::AcceptIndex(index) => self.accept_completion(Some(index)),
                CompletionAction::Dismiss => self.dismiss_popups(),
            },
            EditorEvent::TriggerCompletion => self.request_completion(None),
            EditorEvent::Hover(pos) => self.request_hover(pos),
            EditorEvent::HoverEnd => self.hover = None,
            EditorEvent::Focused => self.focus = Focus::Editor,
        }
    }

    /// Applies an edit to the active tab and syncs it to the language server.
    fn apply_command(&mut self, command: Command) {
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else {
            return;
        };
        let before = tab.buffer.version();
        tab.buffer.apply(command);
        if tab.buffer.version() != before {
            tab.dirty = true;
            tab.version += 1;
            if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
                let _ = client.did_change(&tab.path, tab.version, &tab.buffer.text());
            }
        }
    }

    fn after_typing(&mut self, c: char) {
        if c.is_alphanumeric() || c == '_' {
            if self.completion.is_some() {
                self.refilter_completion();
            } else {
                self.request_completion(None);
            }
        } else if matches!(c, '.' | '$' | '"' | '\'') {
            self.request_completion(Some(c));
        } else {
            self.completion = None;
        }
        match c {
            '(' | ',' => self.request_signature(),
            ')' => self.signature = None,
            _ => {}
        }
    }

    fn dismiss_popups(&mut self) {
        self.completion = None;
        self.hover = None;
        self.signature = None;
    }

    /// The active GDScript tab with a connected language server.
    fn lsp_target(&self) -> Option<(lsp_client::Client, &Tab, Pos)> {
        let tab = self.active_tab().filter(|t| t.language == Language::GDScript)?;
        let client = self.lsp.client()?.clone();
        let pos = tab.buffer.char_to_pos(tab.buffer.primary().head);
        Some((client, tab, pos))
    }

    fn request_completion(&mut self, trigger: Option<char>) {
        let Some((client, tab, pos)) = self.lsp_target() else {
            return;
        };
        let anchor = tab.buffer.word_start(tab.buffer.primary().head);
        if let Ok(id) = client.completion(&tab.path, to_lsp(pos), trigger) {
            self.pending.insert(id, Pending::Completion);
            self.completion = Some(Completion::new(id, anchor));
        }
    }

    /// Narrows the completion list to the word typed since it opened.
    fn refilter_completion(&mut self) {
        let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) else {
            return;
        };
        let Some(c) = &mut self.completion else {
            return;
        };
        let head = tab.buffer.primary().head;
        let same_line = tab.buffer.char_to_pos(head).line == tab.buffer.char_to_pos(c.anchor).line;
        let prefix = tab.buffer.slice(c.anchor, head);
        if head < c.anchor || !same_line || !prefix.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
            self.completion = None;
            return;
        }
        if c.request.is_some() {
            return; // Narrowed when the answer arrives.
        }
        c.filtered = completion::rank(&c.items, &prefix);
        if c.filtered.is_empty() {
            self.completion = None;
            return;
        }
        let colors = code_editor::Colors::from_theme(&self.themes[self.theme_index]);
        c.rows = c.filtered.iter().map(|&i| completion_row(&c.items[i], &colors)).collect();
        c.selected = 0;
        c.doc = None;
        self.resolve_selected();
    }

    /// Shows the selected item's docs, asking the server if needed.
    fn resolve_selected(&mut self) {
        let client = self.lsp.client().cloned();
        let Some(c) = &mut self.completion else {
            return;
        };
        let Some((index, item)) = c.selected_item() else {
            return;
        };
        match &item.documentation {
            Some(markdown) => c.doc = Some(Doc::from_markdown(markdown)),
            None => {
                if let Some(Ok(id)) = client.map(|client| client.resolve_completion(item)) {
                    self.pending.insert(id, Pending::Resolve(index));
                }
            }
        }
    }

    fn accept_completion(&mut self, row: Option<usize>) {
        let Some(c) = self.completion.take() else {
            return;
        };
        let Some(item) = c.filtered.get(row.unwrap_or(c.selected)).map(|&i| &c.items[i]) else {
            return;
        };
        let Some(tab) = self.active_tab() else {
            return;
        };
        let head = tab.buffer.primary().head;
        let Some(delete) = head.checked_sub(c.anchor) else {
            return;
        };
        let mut text = item.insert_text.clone();
        // Functions without arguments come with "()"; add parens for the rest
        // and put the cursor inside.
        let callable = item.kind == CompletionKind::Function && !text.contains('(') && !text.ends_with(':');
        if callable {
            text.push_str("()");
        }
        let cursor_back = usize::from(callable);
        self.apply_command(Command::ReplaceBefore { delete, text, cursor_back });
        if callable {
            self.request_signature();
        }
    }

    fn request_hover(&mut self, pos: Pos) {
        let Some((client, tab, _)) = self.lsp_target() else {
            return;
        };
        if let Ok(id) = client.hover(&tab.path, to_lsp(pos)) {
            self.pending.insert(id, Pending::Hover(pos));
        }
    }

    fn request_signature(&mut self) {
        let Some((client, tab, pos)) = self.lsp_target() else {
            return;
        };
        if let Ok(id) = client.signature_help(&tab.path, to_lsp(pos)) {
            self.pending.insert(id, Pending::Signature(pos));
        }
    }

    fn on_response(&mut self, pending: Pending, id: u64, value: serde_json::Value) {
        match pending {
            Pending::Definition => match lsp_client::locations(&value).into_iter().next() {
                Some(location) => {
                    let start = location.range.start;
                    self.dismiss_popups();
                    self.jump_to(location.path, start.line, start.character);
                }
                None => self.status = Some("No definition found".to_string()),
            },
            Pending::Completion => {
                if let Some(c) = &mut self.completion {
                    if c.request == Some(id) {
                        c.items = lsp_client::completion_items(&value);
                        c.request = None;
                        self.refilter_completion();
                    }
                }
            }
            Pending::Resolve(index) => {
                let Some(resolved) = lsp_client::completion_item(&value) else {
                    return;
                };
                let Some(c) = &mut self.completion else {
                    return;
                };
                let Some(item) = c.items.get_mut(index).filter(|i| i.label == resolved.label) else {
                    return;
                };
                // Remember "no docs" too, so it isn't requested again.
                item.documentation = Some(resolved.documentation.unwrap_or_default());
                if c.selected_item().is_some_and(|(i, _)| i == index) {
                    c.doc = c.items[index].documentation.as_deref().map(Doc::from_markdown);
                }
            }
            Pending::Hover(pos) => {
                self.hover = lsp_client::hover_text(&value).map(|text| (pos, Doc::from_markdown(&text)));
            }
            Pending::Signature(anchor) => {
                self.signature = lsp_client::signature_help(&value).map(|help| Signature {
                    label: help.label.replace('\t', " "),
                    active: help.active_parameter,
                    anchor,
                });
            }
        }
    }

    fn on_lsp(&mut self, event: LspEvent) {
        match event {
            LspEvent::Connected { client, label } => {
                for tab in self.tabs.iter().filter(|t| t.language == Language::GDScript) {
                    let _ = client.did_open(&tab.path, tab.version, &tab.buffer.text());
                }
                self.lsp = Lsp::Ready { client, label };
            }
            LspEvent::Failed(error) => {
                self.status = Some(error);
                self.lsp = Lsp::Failed;
            }
            LspEvent::Event(Event::Diagnostics { path, diagnostics }) => {
                self.problems.set(path, diagnostics);
                self.refresh_marks();
            }
            LspEvent::Event(Event::Response { id, result }) => {
                let Some(pending) = self.pending.remove(&id) else {
                    return;
                };
                match result {
                    Ok(value) => self.on_response(pending, id, value),
                    Err(error) => {
                        if matches!(pending, Pending::Definition) {
                            self.status = Some(format!("Go to definition failed: {error}"));
                        }
                    }
                }
            }
            LspEvent::Event(Event::Disconnected(reason)) => {
                if let Some(reason) = reason {
                    self.status = Some(format!("Language server disconnected: {reason}"));
                }
                self.lsp = Lsp::Disconnected;
                self.pending.clear();
                self.problems.clear();
                self.refresh_marks();
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
        self.refresh_marks();
    }

    /// Rebuilds every tab's diagnostic underlines.
    fn refresh_marks(&mut self) {
        let ui = self.themes[self.theme_index].ui;
        for tab in &mut self.tabs {
            tab.marks = self
                .problems
                .for_path(&tab.path)
                .iter()
                .map(|d| {
                    let color = match d.severity {
                        Severity::Error => ui.danger,
                        Severity::Warning => ui.warning,
                        Severity::Information | Severity::Hint => ui.text_muted,
                    };
                    let (start, end) = (d.range.start, d.range.end);
                    let line = start.line as usize;
                    let end_column = if end.line == start.line { end.character } else { u32::MAX };
                    Mark {
                        line,
                        start: start.character as usize,
                        end: (end_column as usize).min(tab.buffer.line_len(line)),
                        color: style::to_color(color),
                    }
                })
                .collect();
        }
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
        let pos = tab.buffer.char_to_pos(tab.buffer.primary().head);
        match client.definition(&tab.path, to_lsp(pos)) {
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
            tab.buffer.set_cursor(Pos {
                line: line as usize,
                column: character as usize,
            });
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
        // A shell or game started for the previous project is stale.
        self.reset_terminal();
        self.run = RunState::default();
        self.panel = None;
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
                let language = Language::from_path(&path);
                let tab = Tab {
                    language,
                    buffer: Buffer::new(&text, language),
                    path,
                    dirty: false,
                    version: 1,
                    marks: Vec::new(),
                };
                if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
                    let _ = client.did_open(&tab.path, tab.version, &text);
                }
                self.tabs.push(tab);
                self.active = Some(self.tabs.len() - 1);
                self.refresh_marks();
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
        if let Some(index) = self.active {
            if self.save_tab(index) {
                self.status = Some(format!("Saved {}", self.tabs[index].title()));
            }
        }
    }

    /// Saves every unsaved tab (Godot runs from the files on disk).
    fn save_all(&mut self) {
        for index in 0..self.tabs.len() {
            if self.tabs[index].dirty {
                self.save_tab(index);
            }
        }
    }

    fn save_tab(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        let text = tab.buffer.text();
        match std::fs::write(&tab.path, &text) {
            Ok(()) => {
                tab.dirty = false;
                if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
                    let _ = client.did_save(&tab.path, &text);
                }
                true
            }
            Err(error) => {
                self.status = Some(format!("Cannot save {}: {error}", tab.title()));
                false
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
                if let Some(panel) = self.panel {
                    main = main.push(self.bottom_panel(panel));
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
            space::horizontal(),
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

        let colors = code_editor::Colors::from_theme(self.current_theme());
        let completion = self.completion.as_ref().map(|c| CompletionPopup {
            rows: &c.rows,
            selected: c.selected,
            anchor: tab.buffer.char_to_pos(c.anchor),
            doc: c.doc.as_ref(),
        });
        let hover = self.hover.as_ref().map(|(pos, doc)| HoverPopup { pos: *pos, doc });
        let signature = self.signature.as_ref().map(|s| SignaturePopup {
            label: &s.label,
            active: s.active.clone(),
            anchor: s.anchor,
        });
        let editor = CodeEditor::new(&tab.buffer, colors, Message::Edit)
            .marks(&tab.marks)
            .completion(completion)
            .hover(hover)
            .signature(signature)
            .focused(self.focus == Focus::Editor);

        column![self.tab_bar(), editor].into()
    }

    /// The bottom panel: Problems / Output / Terminal tabs and the open one.
    fn bottom_panel(&self, panel: Panel) -> Element<'_, Message> {
        let theme = self.current_theme();
        let ui = &theme.ui;
        let muted = style::to_color(ui.text_muted);
        let action = |label, message| {
            button(text(label).size(12)).on_press(message).style(button::text).padding([0, 6])
        };

        let (errors, warnings) = self.problems.counts();
        let tabs = [
            (Panel::Problems, format!("PROBLEMS  {errors}/{warnings}")),
            (Panel::Output, if self.is_running() { "OUTPUT (running)".to_string() } else { "OUTPUT".to_string() }),
            (Panel::Terminal, "TERMINAL".to_string()),
        ]
        .into_iter()
        .map(|(tab, label)| {
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if tab == panel { button::secondary } else { button::text };
            button(text(label).size(12)).on_press(Message::ShowPanel(tab)).style(style).padding([2, 8]).into()
        });

        let mut header = Row::with_children(tabs).spacing(4).align_y(Alignment::Center);
        if panel == Panel::Terminal {
            let title = self.terminal.title.as_deref().unwrap_or("");
            header = header.push(text(title).size(12).color(muted));
        }
        header = header.push(space::horizontal());
        match panel {
            Panel::Output if self.is_running() => header = header.push(action("stop", Message::StopRun)),
            Panel::Output => header = header.push(action("clear", Message::ClearOutput)),
            Panel::Terminal => header = header.push(action("kill", Message::KillTerminal)),
            Panel::Problems => {}
        }
        header = header.push(action("x", Message::ClosePanel));

        let body: Element<'_, Message> = match panel {
            Panel::Problems => self.problems.view(ui, |p| self.display_path(p)),
            Panel::Output => self.output_view(),
            Panel::Terminal => match &self.terminal.session {
                Some(session) => TerminalView::new(session, terminal_panel::palette(theme), Message::FocusTerminal)
                    .focused(self.focus == Focus::Terminal)
                    .selection_color(style::to_color(ui.accent).scale_alpha(0.4))
                    .into(),
                None => center(text("Starting shell...").size(12).color(muted)).into(),
            },
        };

        container(column![header.padding([4, 8]), body])
            .width(Length::Fill)
            .height(260)
            .style(style::surface(ui))
            .into()
    }

    fn output_view(&self) -> Element<'_, Message> {
        const SHOWN: usize = 1000;
        let ui = &self.current_theme().ui;
        let skip = self.run.output.len().saturating_sub(SHOWN);
        let lines = self.run.output[skip..].iter().map(|line| {
            let color = match line.kind {
                runner::LineKind::Error => ui.danger,
                runner::LineKind::Warning => ui.warning,
                runner::LineKind::Info => ui.text_muted,
                runner::LineKind::Normal => ui.text,
            };
            let label = text(&line.text).size(12).font(iced::Font::MONOSPACE).color(style::to_color(color));
            match &line.link {
                Some((path, number)) => button(label)
                    .on_press(Message::OpenOutputLink(path.clone(), *number))
                    .style(button::text)
                    .padding([0, 8])
                    .into(),
                None => container(label).padding([0, 8]).into(),
            }
        });
        let body: Element<'_, Message> = if self.run.output.is_empty() {
            center(text("Press F5 to run the project, F6 to run the current scene.").size(12).color(style::to_color(ui.text_muted))).into()
        } else {
            scrollable(iced::widget::Column::with_children(lines).width(Length::Fill))
                .anchor_bottom()
                .height(Length::Fill)
                .into()
        };
        body
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
        if self.project.is_some() {
            left.push(
                button(text("Terminal").size(12))
                    .on_press(Message::ToggleTerminal)
                    .style(button::text)
                    .padding([0, 4])
                    .into(),
            );
            let (label, message, color) = if self.is_running() {
                ("Stop (F8)", Message::StopRun, ui.danger)
            } else {
                ("Run (F5)", Message::RunProject, ui.success)
            };
            left.push(
                button(text(label).size(12).color(style::to_color(color)))
                    .on_press(message)
                    .style(button::text)
                    .padding([0, 4])
                    .into(),
            );
        }
        if let Some(status) = &self.status {
            left.push(text(status).size(12).color(style::to_color(ui.text_muted)).into());
        }

        let mut right: Vec<Element<'_, Message>> = Vec::new();
        if let Some(tab) = self.active_tab() {
            let pos = tab.buffer.char_to_pos(tab.buffer.primary().head);
            let mut label = format!("Ln {}, Col {}", pos.line + 1, pos.column + 1);
            let cursors = tab.buffer.selections().len();
            if cursors > 1 {
                label.push_str(&format!(" ({cursors} cursors)"));
            }
            right.push(text(label).size(12).into());
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
                space::horizontal(),
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

/// Global shortcuts, for key presses no widget handled.
fn shortcut(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Named(keyboard::key::Named::F12) => Some(Message::GotoDefinition),
        // Godot editor bindings: F5 run, F6 run current scene, F8 stop.
        keyboard::Key::Named(keyboard::key::Named::F5) if modifiers.shift() => Some(Message::StopRun),
        keyboard::Key::Named(keyboard::key::Named::F5) => Some(Message::RunProject),
        keyboard::Key::Named(keyboard::key::Named::F6) => Some(Message::RunScene),
        keyboard::Key::Named(keyboard::key::Named::F8) => Some(Message::StopRun),
        keyboard::Key::Character("s") if modifiers.command() => Some(Message::Save),
        keyboard::Key::Character("o") if modifiers.command() => Some(Message::OpenFolderDialog),
        keyboard::Key::Character("w") if modifiers.command() => Some(Message::CloseActiveTab),
        // Ctrl+J (VS Code), Ctrl+` and Ctrl+² (the key left of 1 on AZERTY).
        keyboard::Key::Character("j" | "`" | "²") if modifiers.command() => Some(Message::ToggleTerminal),
        _ => None,
    }
}

async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Open Godot project")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

fn to_lsp(pos: Pos) -> Position {
    Position {
        line: pos.line as u32,
        character: pos.column as u32,
    }
}

fn completion_row(item: &lsp_client::CompletionItem, colors: &code_editor::Colors) -> CompletionRow {
    let syntax = &colors.syntax;
    let (tag, color) = match item.kind {
        CompletionKind::Function => ("fn", style::to_color(syntax.function)),
        CompletionKind::Variable => ("var", colors.text),
        CompletionKind::Class => ("cls", style::to_color(syntax.r#type)),
        CompletionKind::Constant => ("const", style::to_color(syntax.constant)),
        CompletionKind::Keyword => ("kw", style::to_color(syntax.keyword)),
        CompletionKind::Signal => ("sig", style::to_color(syntax.annotation)),
        CompletionKind::File => ("file", style::to_color(syntax.string)),
        CompletionKind::Other => ("", colors.muted),
    };
    CompletionRow {
        label: item.label.clone(),
        tag,
        color,
    }
}
