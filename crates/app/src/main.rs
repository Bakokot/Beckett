#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod code_editor;
mod completion;
mod docs;
mod explorer;
mod find;
mod lsp;
mod on_escape;
mod palette;
mod problems;
mod recent;
mod rename;
mod runner;
mod shortcuts;
mod style;
mod terminal_panel;
mod terminal_view;
mod theme_editor;
mod watcher;

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use editor_core::{snippet, Buffer, Command, Language, Pos};
use godot_project::GodotProject;
use iced::widget::{
    button, center, column, container, mouse_area, pick_list, row, scrollable, space, text, tooltip, Row,
};
use iced::{keyboard, Alignment, Element, Length, Subscription, Task};
use lsp_client::{uri, CompletionKind, Event, Position, Severity};

use code_editor::{
    CodeEditor, CompletionAction, CompletionPopup, CompletionRow, EditorEvent, HoverPopup, Mark,
    SignaturePopup,
};
use completion::Completion;
use docs::Doc;
use explorer::Explorer;
use editor_core::search::LineMatch;
use find::{FileResult, Find, FindMessage, ProjectSearch, SearchMessage};
use palette::{CommandEntry, Palette, PaletteMessage};
use lsp::{Lsp, LspEvent};
use problems::Problems;
use rename::{Rename, RenameMessage};
use shortcuts::{Shortcuts, ShortcutsMessage};
use runner::{OutputLine, RunEvent, RunHandle, RunRequest};
use terminal_panel::TerminalEvent;
use terminal_view::TerminalView;
use theme_editor::{ThemeEditor, ThemeMessage};

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
    OpenRecent(PathBuf),
    /// Forget a project in the recent list.
    RemoveRecent(PathBuf),
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
    /// Automatic reconnect attempt for the connection of this generation.
    ReconnectLsp(u64),
    GotoDefinition,
    /// Shift+F12: list every use of the symbol under the cursor.
    FindReferences,
    /// F2: open the rename bar for the symbol under the cursor.
    StartRename,
    Rename(RenameMessage),
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
    /// Paths changed on disk by something other than the IDE.
    FilesChanged(Vec<PathBuf>),
    /// Discard the active tab's edits and reload it from disk.
    ReloadActive,
    Find(FindMessage),
    /// Ctrl+F, or Ctrl+H with the replace row.
    OpenFind { replace: bool },
    Search(SearchMessage),
    /// Ctrl+Shift+F.
    OpenSearch,
    ModifiersChanged(keyboard::Modifiers),
    /// Open the palette with this input ("" files, ">" commands, ":" go to line).
    OpenPalette(String),
    Palette(PaletteMessage),
    OpenThemeEditor,
    /// F1: the keyboard shortcuts page.
    OpenShortcuts,
    Shortcuts(ShortcutsMessage),
    ThemeEditor(ThemeMessage),
    ThemeImportPicked(Option<PathBuf>),
    ThemeExportPicked(Option<PathBuf>),
    ThemeCodePasted(Option<String>),
    /// A file dropped on the window: a theme to import, or a file to open.
    FileDropped(PathBuf),
}

/// Which panel receives keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Editor,
    Terminal,
    /// A text field (find bar, project search) has focus.
    Input,
}

/// Tabs of the bottom panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Problems,
    Output,
    Terminal,
    Search,
}

#[derive(Default)]
struct TerminalState {
    session: Option<std::sync::Arc<terminal::Terminal>>,
    title: Option<String>,
    /// Bumped to start a new shell.
    generation: u64,
}

const MAX_OUTPUT_LINES: usize = 5000;
/// Waits before each automatic language server reconnect, in seconds.
const RECONNECT_DELAYS: [u64; 5] = [1, 2, 5, 10, 20];

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
    /// References to the named symbol.
    References(String),
    /// Whether the symbol `word` at `pos` can be renamed.
    PrepareRename { path: PathBuf, pos: Pos, word: String },
    Rename { from: String, to: String },
}

/// A snippet being filled in: Tab / Shift+Tab move between its stops.
struct SnippetSession {
    /// Char ranges in the active buffer, in visiting order; the last one
    /// is the final cursor.
    stops: Vec<(usize, usize)>,
    current: usize,
}

struct Signature {
    label: String,
    active: Option<Range<usize>>,
    anchor: Pos,
}

struct App {
    project: Option<Project>,
    /// Recently opened projects, most recent first.
    recent: Vec<recent::RecentProject>,
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
    /// Automatic reconnects since the server was last connected.
    lsp_retries: u32,
    pending: HashMap<u64, Pending>,
    problems: Problems,
    /// Open tab of the bottom panel, if it is shown.
    panel: Option<Panel>,
    run: RunState,
    /// Find bar of the active tab.
    find: Option<Find>,
    rename: Option<Rename>,
    search: ProjectSearch,
    modifiers: keyboard::Modifiers,
    palette: Option<Palette<Message>>,
    /// Where each theme was loaded from (`None` for built-in ones).
    theme_paths: Vec<Option<PathBuf>>,
    theme_editor: Option<ThemeEditor>,
    /// The keyboard shortcuts page, shown in place of the editor.
    shortcuts: Option<Shortcuts>,
    // Popups for the active tab.
    completion: Option<Completion>,
    hover: Option<(Pos, Doc)>,
    signature: Option<Signature>,
    snippet: Option<SnippetSession>,
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
    /// The file changed on disk while this tab had unsaved edits.
    disk_changed: bool,
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
        let mut theme_paths = vec![None; themes.len()];
        let mut status = None;
        if let Some(dir) = theme::user_themes_dir() {
            let (user, errors) = theme::load_dir(&dir);
            for (path, theme) in user {
                themes.push(theme);
                theme_paths.push(Some(path));
            }
            if let Some(error) = errors.first() {
                status = Some(format!("Theme error: {error}"));
            }
        }
        let iced_theme = style::iced_theme(&themes[0]);

        let mut app = Self {
            project: None,
            recent: recent::load(),
            tabs: Vec::new(),
            active: None,
            themes,
            theme_index: 0,
            iced_theme,
            pending_close: None,
            status,
            lsp: Lsp::Off,
            lsp_generation: 0,
            lsp_retries: 0,
            pending: HashMap::new(),
            problems: Problems::default(),
            panel: None,
            run: RunState::default(),
            find: None,
            rename: None,
            search: ProjectSearch::default(),
            modifiers: keyboard::Modifiers::default(),
            palette: None,
            theme_paths,
            theme_editor: None,
            shortcuts: None,
            completion: None,
            hover: None,
            signature: None,
            snippet: None,
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

    /// The theme in effect: the theme editor's draft while it is open.
    fn current_theme(&self) -> &theme::Theme {
        match &self.theme_editor {
            Some(editor) => &editor.draft,
            None => &self.themes[self.theme_index],
        }
    }

    /// Re-applies the current theme after it changed.
    fn apply_theme(&mut self) {
        self.iced_theme = style::iced_theme(self.current_theme());
        self.refresh_marks();
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

        let files = self
            .project
            .as_ref()
            .map(|project| watcher::watch(project.root.clone()).map(Message::FilesChanged))
            .unwrap_or_else(Subscription::none);

        let dropped = iced::event::listen_with(|event, _status, _window| match event {
            iced::Event::Window(iced::window::Event::FileDropped(path)) => Some(Message::FileDropped(path)),
            _ => None,
        });

        Subscription::batch([keys, lsp, terminal, run, files, dropped])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.find_key();
        let task = self.handle(message);
        // Keep find matches in step with whichever buffer is active.
        if self.find.is_some() && self.find_key() != before {
            self.recompute_find();
        }
        task
    }

    /// Identifies the active buffer's contents.
    fn find_key(&self) -> Option<(PathBuf, u64)> {
        self.active_tab().map(|tab| (tab.path.clone(), tab.buffer.version()))
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenFolderDialog => {
                return Task::perform(pick_folder(), Message::FolderPicked);
            }
            Message::FolderPicked(Some(path)) => self.open_path(path),
            Message::FolderPicked(None) => {}
            Message::OpenRecent(root) => {
                if root.is_dir() {
                    self.open_path(root);
                } else {
                    self.status = Some(format!("{} no longer exists", root.display()));
                }
            }
            Message::RemoveRecent(root) => self.recent = recent::remove(&root),
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
                    self.shortcuts = None;
                }
            }
            Message::OpenShortcuts => {
                // Keep the filter when it is already open.
                self.shortcuts.get_or_insert_with(Shortcuts::default);
                self.palette = None;
                self.focus = Focus::Input;
                self.dismiss_popups();
                return Task::batch([
                    iced::widget::operation::focus(shortcuts::INPUT),
                    iced::widget::operation::select_all(shortcuts::INPUT),
                ]);
            }
            Message::Shortcuts(ShortcutsMessage::Filter(filter)) => {
                if let Some(page) = &mut self.shortcuts {
                    page.filter = filter;
                }
            }
            Message::Shortcuts(ShortcutsMessage::Close) => {
                self.shortcuts = None;
                self.focus = Focus::Editor;
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
                    // While editing, picking a theme starts editing that one.
                    let builtin = self.theme_paths[index].is_none();
                    if let Some(editor) = &mut self.theme_editor {
                        editor.reset(self.themes[index].clone(), index, builtin);
                    }
                    self.apply_theme();
                }
            }
            Message::OpenThemeEditor => {
                let index = self.theme_index;
                let builtin = self.theme_paths[index].is_none();
                self.theme_editor = Some(ThemeEditor::new(self.themes[index].clone(), index, builtin));
                self.focus = Focus::Input;
                self.dismiss_popups();
                self.apply_theme();
            }
            Message::ThemeEditor(message) => return self.on_theme_editor(message),
            Message::ThemeImportPicked(Some(path)) => self.import_theme_file(&path),
            Message::ThemeImportPicked(None) | Message::ThemeExportPicked(None) => {}
            Message::ThemeExportPicked(Some(path)) => {
                let theme = self.current_theme().clone();
                self.status = Some(match theme.to_toml().map_err(|e| e.to_string()).and_then(|t| {
                    std::fs::write(&path, t).map_err(|e| e.to_string())
                }) {
                    Ok(()) => format!("Exported theme to {}", path.display()),
                    Err(error) => format!("Cannot export theme: {error}"),
                });
            }
            Message::ThemeCodePasted(text) => match theme::from_share_code(text.as_deref().unwrap_or("")) {
                Ok(theme) => self.install_theme(theme, "share code"),
                Err(error) => self.status = Some(error),
            },
            Message::FileDropped(path) => {
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml")) {
                    self.import_theme_file(&path);
                } else if path.is_file() {
                    self.dismiss_popups();
                    self.open_file(path);
                }
            }
            Message::Lsp(event) => return self.on_lsp(event),
            Message::ReconnectLsp(generation) => {
                // Skipped if the user restarted it or opened another project meanwhile.
                let waiting = matches!(self.lsp, Lsp::Disconnected { retry_in: Some(_) });
                if waiting && generation == self.lsp_generation {
                    self.lsp_retries += 1;
                    self.reset_lsp(Lsp::Starting);
                }
            }
            Message::RestartLsp => {
                self.lsp_retries = 0;
                if self.project.as_ref().is_some_and(|p| p.godot.is_some()) {
                    self.reset_lsp(Lsp::Starting);
                }
            }
            Message::GotoDefinition => self.goto_definition(),
            Message::FindReferences => self.find_references(),
            Message::StartRename => self.start_rename(),
            Message::Rename(message) => return self.on_rename(message),
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
            Message::FilesChanged(paths) => self.on_files_changed(&paths),
            Message::ReloadActive => {
                if let Some(index) = self.active {
                    self.reload_tab(index);
                }
            }
            Message::OpenFind { replace } => return self.open_find(replace),
            Message::Find(message) => return self.on_find(message),
            Message::OpenSearch => return self.open_search(),
            Message::Search(message) => return self.on_search(message),
            Message::ModifiersChanged(modifiers) => self.modifiers = modifiers,
            Message::OpenPalette(input) => return self.open_palette(input),
            Message::Palette(message) => return self.on_palette(message),
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
        self.focus = match panel {
            Panel::Terminal => Focus::Terminal,
            Panel::Search => Focus::Input,
            Panel::Problems | Panel::Output => Focus::Editor,
        };
        self.dismiss_popups();
    }

    /// A single-line selection, to prefill a search box.
    fn selection_query(&self) -> Option<String> {
        let selected = self.active_tab()?.buffer.selected_text();
        (!selected.is_empty() && !selected.contains('\n')).then_some(selected)
    }

    fn open_find(&mut self, replace: bool) -> Task<Message> {
        if self.active_tab().is_none() {
            return Task::none();
        }
        let prefill = self.selection_query();
        let find = self
            .find
            .get_or_insert_with(|| Find::new(editor_core::search::Query::default(), replace));
        if let Some(text) = prefill {
            find.query.text = text;
        }
        find.show_replace |= replace;
        self.recompute_find();
        self.focus = Focus::Input;
        self.dismiss_popups();
        let input = if replace { find::REPLACE_INPUT } else { find::FIND_INPUT };
        Task::batch([
            iced::widget::operation::focus(input),
            iced::widget::operation::select_all(input),
        ])
    }

    fn recompute_find(&mut self) {
        if let (Some(find), Some(tab)) = (&mut self.find, self.active.and_then(|i| self.tabs.get(i))) {
            find.recompute(&tab.buffer);
        }
    }

    /// Selects the current match in the editor, which also scrolls to it.
    fn select_current_match(&mut self) {
        if let Some(range) = self.find.as_ref().and_then(Find::current_range) {
            self.apply_command(Command::Select { ranges: vec![range], primary: 0 });
        }
    }

    fn on_find(&mut self, message: FindMessage) -> Task<Message> {
        let Some(find) = &mut self.find else {
            return Task::none();
        };
        match message {
            FindMessage::Query(text) => {
                find.query.text = text;
                self.recompute_find();
                self.select_current_match();
            }
            FindMessage::Replace(text) => find.replace = text,
            FindMessage::ToggleCase | FindMessage::ToggleWord | FindMessage::ToggleRegex => {
                let q = &mut find.query;
                match message {
                    FindMessage::ToggleCase => q.case_sensitive = !q.case_sensitive,
                    FindMessage::ToggleWord => q.whole_word = !q.whole_word,
                    _ => q.regex = !q.regex,
                }
                self.recompute_find();
                self.select_current_match();
            }
            FindMessage::ToggleReplace => find.show_replace = !find.show_replace,
            FindMessage::Next | FindMessage::Previous | FindMessage::Submit => {
                let forward = match message {
                    FindMessage::Previous => false,
                    FindMessage::Submit => !self.modifiers.shift(),
                    _ => true,
                };
                // Step from the match under the cursor, not from a stale index.
                self.recompute_find();
                let selection = self.active_tab().map(|tab| tab.buffer.primary());
                if let Some(find) = &mut self.find {
                    let on_match = selection
                        .zip(find.current_range())
                        .is_some_and(|(sel, range)| (sel.start(), sel.end()) == range);
                    if on_match || !forward {
                        find.step(forward);
                    }
                }
                self.select_current_match();
            }
            FindMessage::ReplaceOne => {
                let Some(range) = find.current_range() else {
                    return Task::none();
                };
                let selected = self.active_tab().map(|t| t.buffer.primary()).is_some_and(|s| (s.start(), s.end()) == range);
                if selected {
                    let edits = self.find_replacements(&[range]);
                    self.apply_command(Command::Replace(edits));
                    self.recompute_find();
                }
                self.select_current_match();
            }
            FindMessage::ReplaceAll => {
                let matches = find.matches.clone();
                let count = matches.len();
                if count > 0 {
                    let edits = self.find_replacements(&matches);
                    self.apply_command(Command::Replace(edits));
                    self.recompute_find();
                    self.status = Some(format!("Replaced {count} occurrence{}", if count == 1 { "" } else { "s" }));
                }
            }
            FindMessage::SelectAll => {
                let ranges = find.matches.clone();
                let primary = find.current.unwrap_or(0);
                self.find = None;
                self.focus = Focus::Editor;
                self.apply_command(Command::Select { ranges, primary });
            }
            FindMessage::Close => {
                self.find = None;
                self.focus = Focus::Editor;
            }
        }
        Task::none()
    }

    fn find_replacements(&self, ranges: &[(usize, usize)]) -> Vec<(usize, usize, String)> {
        match (&self.find, self.active_tab()) {
            (Some(find), Some(tab)) => match find.matcher() {
                Some(matcher) => tab.buffer.replacements(matcher, ranges, &find.replace),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Color edits need the editor open; import, export and share codes work
    /// on the current theme either way (they are also palette commands).
    fn on_theme_editor(&mut self, message: ThemeMessage) -> Task<Message> {
        if let Some(editor) = &mut self.theme_editor {
            if editor.apply(&message) {
                self.apply_theme();
                return Task::none();
            }
        }
        match message {
            ThemeMessage::Save => self.save_theme(),
            ThemeMessage::Revert => {
                if let Some(editor) = &mut self.theme_editor {
                    let base = editor.base;
                    editor.reset(self.themes[base].clone(), base, self.theme_paths[base].is_none());
                    self.apply_theme();
                }
            }
            ThemeMessage::Close => {
                self.theme_editor = None;
                self.focus = Focus::Editor;
                self.apply_theme();
            }
            ThemeMessage::Import => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import theme")
                            .add_filter("Godot IDE theme", &["toml"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::ThemeImportPicked,
                );
            }
            ThemeMessage::Export => {
                let name = theme::file_name(&self.current_theme().name);
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Export theme")
                            .add_filter("Godot IDE theme", &["toml"])
                            .set_file_name(name)
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::ThemeExportPicked,
                );
            }
            ThemeMessage::CopyCode => {
                let code = theme::to_share_code(self.current_theme());
                self.status = Some(format!("Theme code copied ({} characters)", code.len()));
                return iced::clipboard::write(code);
            }
            ThemeMessage::PasteCode => return iced::clipboard::read().map(Message::ThemeCodePasted),
            _ => {}
        }
        Task::none()
    }

    /// Writes the editor's draft to the user themes folder. A built-in theme
    /// is never overwritten: its edited version is saved as a copy.
    fn save_theme(&mut self) {
        let Some(editor) = &mut self.theme_editor else {
            return;
        };
        let mut theme = editor.draft.clone();
        theme.name = theme.name.trim().to_string();
        if theme.name.is_empty() {
            theme.name = "My Theme".to_string();
        }
        let clashes_with_builtin = self
            .themes
            .iter()
            .zip(&self.theme_paths)
            .any(|(t, path)| path.is_none() && t.name == theme.name);
        if clashes_with_builtin {
            theme.name = format!("{} (custom)", theme.name);
        }
        match self.write_user_theme(&theme) {
            Ok(path) => {
                let index = self.add_theme(theme.clone(), path.clone());
                self.theme_index = index;
                if let Some(editor) = &mut self.theme_editor {
                    editor.reset(theme, index, false);
                }
                self.status = Some(format!("Saved theme to {}", path.display()));
                self.apply_theme();
            }
            Err(error) => self.status = Some(format!("Cannot save theme: {error}")),
        }
    }

    fn write_user_theme(&self, theme: &theme::Theme) -> Result<PathBuf, String> {
        let dir = theme::user_themes_dir().ok_or("No user config folder")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(theme::file_name(&theme.name));
        let text = theme.to_toml().map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Adds a theme to the list, replacing a user theme of the same name.
    fn add_theme(&mut self, theme: theme::Theme, path: PathBuf) -> usize {
        let existing = self
            .themes
            .iter()
            .zip(&self.theme_paths)
            .position(|(t, p)| p.is_some() && t.name == theme.name);
        match existing {
            Some(index) => {
                self.themes[index] = theme;
                self.theme_paths[index] = Some(path);
                index
            }
            None => {
                self.themes.push(theme);
                self.theme_paths.push(Some(path));
                self.themes.len() - 1
            }
        }
    }

    fn import_theme_file(&mut self, path: &Path) {
        let result = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| theme::Theme::from_toml(&text).map_err(|e| e.to_string()));
        match result {
            Ok(theme) => self.install_theme(theme, &path.display().to_string()),
            Err(error) => self.status = Some(format!("Cannot import theme: {error}")),
        }
    }

    /// Copies a theme into the user themes folder and switches to it.
    fn install_theme(&mut self, mut theme: theme::Theme, source: &str) {
        if self.themes.iter().zip(&self.theme_paths).any(|(t, p)| p.is_none() && t.name == theme.name) {
            theme.name = format!("{} (imported)", theme.name);
        }
        match self.write_user_theme(&theme) {
            Ok(path) => {
                let index = self.add_theme(theme.clone(), path);
                self.theme_index = index;
                if let Some(editor) = &mut self.theme_editor {
                    editor.reset(theme.clone(), index, false);
                }
                self.status = Some(format!("Imported theme \"{}\" from {source}", theme.name));
                self.apply_theme();
            }
            Err(error) => self.status = Some(format!("Cannot import theme: {error}")),
        }
    }

    fn open_palette(&mut self, input: String) -> Task<Message> {
        let Some(project) = &self.project else {
            return Task::none();
        };
        let open: Vec<PathBuf> = self.tabs.iter().map(|t| t.path.clone()).collect();
        let mut palette = Palette::new(&project.root, &open, input);
        palette.refresh(&self.commands());
        self.palette = Some(palette);
        self.focus = Focus::Input;
        self.dismiss_popups();
        Task::batch([
            iced::widget::operation::focus(palette::INPUT),
            iced::widget::operation::move_cursor_to_end(palette::INPUT),
        ])
    }

    fn on_palette(&mut self, message: PaletteMessage) -> Task<Message> {
        let commands = self.commands();
        let Some(palette) = &mut self.palette else {
            return Task::none();
        };
        let item = match message {
            PaletteMessage::Input(input) => {
                palette.input = input;
                palette.refresh(&commands);
                return Task::none();
            }
            PaletteMessage::Move(delta) => {
                palette.move_selection(delta);
                return Task::none();
            }
            PaletteMessage::Scroll(rows) => {
                palette.scroll_by(rows);
                return Task::none();
            }
            PaletteMessage::Accept => palette.selected_item().cloned(),
            PaletteMessage::AcceptRow(row) => palette.row(row).cloned(),
            PaletteMessage::Close => None,
        };
        self.palette = None;
        self.focus = Focus::Editor;
        match item.map(|item| item.action) {
            Some(palette::Action::Open(path)) => {
                self.dismiss_popups();
                self.open_file(path);
            }
            Some(palette::Action::GotoLine(line, column)) => {
                if let Some(path) = self.active_tab().map(|t| t.path.clone()) {
                    self.jump_to(path, line, column);
                }
            }
            Some(palette::Action::Run(message)) => return self.handle(message),
            None => {}
        }
        Task::none()
    }

    /// Everything the command palette offers.
    fn commands(&self) -> Vec<CommandEntry<Message>> {
        let edit = |command: Command| Message::Edit(EditorEvent::Command(command));
        let mut commands: Vec<(&str, &'static str, Message)> = vec![
            ("File: Save", "Ctrl+S", Message::Save),
            ("File: Open Folder", "Ctrl+O", Message::OpenFolderDialog),
            ("File: Close Tab", "Ctrl+W", Message::CloseActiveTab),
            ("File: Reload from Disk", "", Message::ReloadActive),
            ("Go to File", "Ctrl+P", Message::OpenPalette(String::new())),
            ("Go to Line", "Ctrl+G", Message::OpenPalette(":".to_string())),
            ("Go to Definition", "F12", Message::GotoDefinition),
            ("Find All References", "Shift+F12", Message::FindReferences),
            ("Rename Symbol", "F2", Message::StartRename),
            ("Find", "Ctrl+F", Message::OpenFind { replace: false }),
            ("Replace", "Ctrl+H", Message::OpenFind { replace: true }),
            ("Search in Project", "Ctrl+Shift+F", Message::OpenSearch),
            ("Run: Project", "F5", Message::RunProject),
            ("Run: Current Scene", "F6", Message::RunScene),
            ("Run: Stop", "F8", Message::StopRun),
            ("View: Toggle Terminal", "Ctrl+J", Message::ToggleTerminal),
            ("View: Problems", "", Message::ShowPanel(Panel::Problems)),
            ("View: Output", "", Message::ShowPanel(Panel::Output)),
            ("View: Search Results", "", Message::ShowPanel(Panel::Search)),
            ("Terminal: Kill", "", Message::KillTerminal),
            ("Output: Clear", "", Message::ClearOutput),
            ("Language Server: Restart", "", Message::RestartLsp),
            ("Edit: Undo", "Ctrl+Z", edit(Command::Undo)),
            ("Edit: Redo", "Ctrl+Y", edit(Command::Redo)),
            ("Edit: Select All", "Ctrl+A", edit(Command::SelectAll)),
            ("Edit: Toggle Comment", "Ctrl+K", edit(Command::ToggleComment)),
            ("Edit: Duplicate Line", "Shift+Alt+Down", edit(Command::DuplicateLines)),
            ("Edit: Delete Line", "Ctrl+Shift+K", edit(Command::DeleteLines)),
            ("Edit: Move Line Up", "Alt+Up", edit(Command::MoveLinesUp)),
            ("Edit: Move Line Down", "Alt+Down", edit(Command::MoveLinesDown)),
            ("Edit: Indent", "Tab", edit(Command::Indent)),
            ("Edit: Outdent", "Shift+Tab", edit(Command::Outdent)),
            ("Edit: Select Next Occurrence", "Ctrl+D", edit(Command::SelectNextOccurrence)),
            ("Edit: Add Cursor Above", "Ctrl+Alt+Up", edit(Command::AddCursorAbove)),
            ("Edit: Add Cursor Below", "Ctrl+Alt+Down", edit(Command::AddCursorBelow)),
            ("Edit: Trigger Completion", "Ctrl+Space", Message::Edit(EditorEvent::TriggerCompletion)),
            ("Help: Keyboard Shortcuts", "F1", Message::OpenShortcuts),
            ("Theme: Edit Colors", "", Message::OpenThemeEditor),
            ("Theme: Import File...", "", Message::ThemeEditor(ThemeMessage::Import)),
            ("Theme: Export File...", "", Message::ThemeEditor(ThemeMessage::Export)),
            ("Theme: Copy Share Code", "", Message::ThemeEditor(ThemeMessage::CopyCode)),
            ("Theme: Paste Share Code", "", Message::ThemeEditor(ThemeMessage::PasteCode)),
        ];
        let themes: Vec<(String, Message)> = self
            .themes
            .iter()
            .map(|t| (format!("Theme: {}", t.name), Message::ThemeSelected(t.name.clone())))
            .collect();
        let mut entries: Vec<CommandEntry<Message>> = commands
            .drain(..)
            .map(|(name, shortcut, message)| CommandEntry { name: name.to_string(), shortcut, message })
            .collect();
        entries.extend(themes.into_iter().map(|(name, message)| CommandEntry { name, shortcut: "", message }));
        let current = self.project.as_ref().map(|p| p.root.as_path());
        entries.extend(
            self.recent
                .iter()
                .filter(|r| !r.missing && current.is_none_or(|c| !uri::same_path(c, &r.root)))
                .map(|r| CommandEntry {
                    name: format!("Open Recent: {} ({})", r.name, r.root.display()),
                    shortcut: "",
                    message: Message::OpenRecent(r.root.clone()),
                }),
        );
        entries
    }

    fn open_search(&mut self) -> Task<Message> {
        if self.project.is_none() {
            return Task::none();
        }
        if let Some(text) = self.selection_query() {
            self.search.query.text = text;
        }
        self.show_panel(Panel::Search);
        Task::batch([
            iced::widget::operation::focus(find::SEARCH_INPUT),
            iced::widget::operation::select_all(find::SEARCH_INPUT),
        ])
    }

    fn on_search(&mut self, message: SearchMessage) -> Task<Message> {
        let query = &mut self.search.query;
        match message {
            SearchMessage::Query(text) => query.text = text,
            SearchMessage::ToggleCase => query.case_sensitive = !query.case_sensitive,
            SearchMessage::ToggleWord => query.whole_word = !query.whole_word,
            SearchMessage::ToggleRegex => query.regex = !query.regex,
            SearchMessage::Run => {
                let Some(project) = &self.project else {
                    return Task::none();
                };
                self.search.next_id += 1;
                let id = self.search.next_id;
                self.search.running = Some(id);
                self.search.error = None;
                self.search.references = None;
                let root = project.root.clone();
                let query = self.search.query.clone();
                // Unsaved edits are searched instead of the files on disk.
                let open: Vec<(PathBuf, String)> = self
                    .tabs
                    .iter()
                    .filter(|t| t.dirty)
                    .map(|t| (t.path.clone(), t.buffer.text()))
                    .collect();
                return Task::perform(
                    async move { find::search_project(&root, &query, &open) },
                    move |result| Message::Search(SearchMessage::Done(id, result)),
                );
            }
            SearchMessage::Done(id, result) => {
                if self.search.running != Some(id) {
                    return Task::none();
                }
                self.search.running = None;
                match result {
                    Ok(results) => {
                        self.search.results = results;
                        self.search.searched = Some(self.search.query.text.clone());
                    }
                    Err(error) => self.search.error = Some(error),
                }
            }
            SearchMessage::Open(path, line, column) => {
                self.dismiss_popups();
                self.focus = Focus::Editor;
                self.jump_to(path, line as u32, column as u32);
            }
        }
        Task::none()
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

    fn on_files_changed(&mut self, paths: &[PathBuf]) {
        // Files appeared or vanished: refresh their folders in the explorer.
        if let Some(project) = &mut self.project {
            let dirs: std::collections::BTreeSet<&Path> = paths.iter().filter_map(|p| p.parent()).collect();
            for dir in dirs {
                project.explorer.refresh(dir);
            }
        }

        for index in 0..self.tabs.len() {
            let tab = &self.tabs[index];
            if !paths.iter().any(|p| uri::same_path(p, &tab.path)) {
                continue;
            }
            let Ok(disk) = std::fs::read_to_string(&tab.path) else {
                if !tab.path.exists() {
                    self.status = Some(format!("{} was deleted on disk", tab.title()));
                    self.tabs[index].disk_changed = true;
                }
                continue;
            };
            // Our own saves come back as events too.
            if disk == tab.buffer.file_contents() {
                self.tabs[index].disk_changed = false;
                continue;
            }
            if tab.dirty {
                self.status = Some(format!(
                    "{} changed on disk (save keeps yours)",
                    tab.title()
                ));
                self.tabs[index].disk_changed = true;
            } else {
                self.reload_tab(index);
            }
        }
    }

    /// Replaces a tab's contents with the file on disk, keeping the cursor
    /// line and column where possible.
    fn reload_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let text = match std::fs::read_to_string(&tab.path) {
            Ok(text) => text,
            Err(error) => {
                self.status = Some(format!("Cannot reload {}: {error}", tab.title()));
                return;
            }
        };
        let cursor = tab.buffer.char_to_pos(tab.buffer.primary().head);
        tab.buffer = Buffer::new(&text, tab.language);
        tab.buffer.set_cursor(cursor);
        tab.dirty = false;
        tab.disk_changed = false;
        tab.version += 1;
        if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
            let _ = client.did_change(&tab.path, tab.version, &tab.buffer.text());
        }
        self.status = Some(format!("Reloaded {} from disk", tab.title()));
        if Some(index) == self.active {
            self.dismiss_popups();
        }
        self.refresh_marks();
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
                // Escape closes the popups first, then ends the snippet.
                CompletionAction::Dismiss => {
                    if self.completion.is_some() || self.hover.is_some() || self.signature.is_some() {
                        self.completion = None;
                        self.hover = None;
                        self.signature = None;
                    } else {
                        self.snippet = None;
                    }
                }
            },
            EditorEvent::TriggerCompletion => self.request_completion(None),
            EditorEvent::NextTabStop | EditorEvent::PreviousTabStop => {
                self.completion = None;
                self.signature = None;
                if let Some(session) = &self.snippet {
                    let index = match event {
                        EditorEvent::NextTabStop => session.current + 1,
                        _ => session.current.saturating_sub(1),
                    };
                    self.goto_tab_stop(index);
                }
            }
            EditorEvent::Hover(pos) => self.request_hover(pos),
            EditorEvent::HoverEnd => self.hover = None,
            EditorEvent::Focused => {
                self.focus = Focus::Editor;
                self.palette = None;
            }
        }
    }

    /// Applies an edit to the active tab and syncs it to the language server.
    fn apply_command(&mut self, command: Command) {
        if let Some(index) = self.active {
            let len_before = self.tabs[index].buffer.len_chars();
            self.edit_tab(index, command);
            self.track_snippet(len_before);
        }
    }

    /// Applies an edit to any tab and syncs it to the language server.
    fn edit_tab(&mut self, index: usize, command: Command) {
        let Some(tab) = self.tabs.get_mut(index) else {
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
            if self.find.is_some() && Some(index) == self.active {
                self.recompute_find();
            }
        }
    }

    /// The identifier under the active tab's cursor, with the tab's path
    /// and the cursor position.
    fn symbol_at_cursor(&self) -> Option<(PathBuf, Pos, String)> {
        let tab = self.active_tab()?;
        let head = tab.buffer.primary().head;
        let (start, end) = tab.buffer.word_range(head);
        let word = tab.buffer.slice(start, end);
        let is_word = word.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
        is_word.then(|| (tab.path.clone(), tab.buffer.char_to_pos(head), word))
    }

    fn find_references(&mut self) {
        let Some((client, _, _)) = self.lsp_target() else {
            self.status = Some("Find references needs the language server and a GDScript file".to_string());
            return;
        };
        let Some((path, pos, word)) = self.symbol_at_cursor() else {
            self.status = Some("No symbol under the cursor".to_string());
            return;
        };
        match client.references(&path, to_lsp(pos)) {
            Ok(id) => {
                self.pending.insert(id, Pending::References(word));
            }
            Err(error) => self.status = Some(format!("Find references failed: {error}")),
        }
    }

    /// Lists reference locations in the search panel, grouped by file.
    fn show_references(&mut self, name: String, locations: Vec<lsp_client::Location>) {
        if locations.is_empty() {
            self.status = Some(format!("No references to {name}"));
            return;
        }
        let mut results: Vec<FileResult> = Vec::new();
        for location in locations {
            let index = match results.iter().position(|f| uri::same_path(&f.path, &location.path)) {
                Some(index) => index,
                None => {
                    results.push(FileResult { path: location.path.clone(), lines: Vec::new() });
                    results.len() - 1
                }
            };
            let start = location.range.start;
            results[index].lines.push(LineMatch {
                line: start.line as usize,
                column: start.character as usize,
                text: String::new(),
            });
        }
        // Line text from the open buffer, else from disk.
        for file in &mut results {
            file.lines.sort_by_key(|l| (l.line, l.column));
            let text = match self.tabs.iter().find(|t| uri::same_path(&t.path, &file.path)) {
                Some(tab) => tab.buffer.text(),
                None => std::fs::read_to_string(&file.path).unwrap_or_default(),
            };
            let lines: Vec<&str> = text.lines().collect();
            for line in &mut file.lines {
                line.text = lines.get(line.line).map(|l| l.trim_end().to_string()).unwrap_or_default();
            }
        }
        self.search.results = results;
        self.search.references = Some(name);
        self.search.running = None;
        self.search.error = None;
        self.show_panel(Panel::Search);
        self.focus = Focus::Editor;
    }

    fn start_rename(&mut self) {
        let Some((client, _, _)) = self.lsp_target() else {
            self.status = Some("Rename needs the language server and a GDScript file".to_string());
            return;
        };
        let Some((path, pos, word)) = self.symbol_at_cursor() else {
            self.status = Some("No symbol under the cursor".to_string());
            return;
        };
        match client.prepare_rename(&path, to_lsp(pos)) {
            Ok(id) => {
                self.pending.insert(id, Pending::PrepareRename { path, pos, word });
            }
            Err(error) => self.status = Some(format!("Rename failed: {error}")),
        }
    }

    fn open_rename(&mut self, path: PathBuf, pos: Pos, word: String) -> Task<Message> {
        self.rename = Some(Rename::new(path, pos, word));
        self.find = None;
        self.focus = Focus::Input;
        self.dismiss_popups();
        Task::batch([
            iced::widget::operation::focus(rename::INPUT),
            iced::widget::operation::select_all(rename::INPUT),
        ])
    }

    fn on_rename(&mut self, message: RenameMessage) -> Task<Message> {
        let Some(rename) = &mut self.rename else {
            return Task::none();
        };
        match message {
            RenameMessage::Input(name) => rename.name = name,
            RenameMessage::Submit => {
                let Some(to) = rename.valid_name().map(str::to_string) else {
                    if rename.name.trim() == rename.original {
                        self.close_rename();
                    }
                    return Task::none();
                };
                let Some(client) = self.lsp.client() else {
                    self.status = Some("Language server is not connected".to_string());
                    return Task::none();
                };
                match client.rename(&rename.path, to_lsp(rename.pos), &to) {
                    Ok(id) => {
                        let from = rename.original.clone();
                        self.pending.insert(id, Pending::Rename { from, to });
                    }
                    Err(error) => self.status = Some(format!("Rename failed: {error}")),
                }
                self.close_rename();
            }
            RenameMessage::Close => self.close_rename(),
        }
        Task::none()
    }

    fn close_rename(&mut self) {
        self.rename = None;
        self.focus = Focus::Editor;
    }

    /// Applies a language server edit: open tabs are edited in place (one
    /// undo step each, left unsaved), other files are rewritten on disk.
    fn apply_workspace_edit(&mut self, from: &str, to: &str, files: Vec<(PathBuf, Vec<lsp_client::TextEdit>)>) {
        let char_edits = |buffer: &Buffer, edits: &[lsp_client::TextEdit]| -> Vec<(usize, usize, String)> {
            let at = |p: Position| buffer.pos_to_char(Pos { line: p.line as usize, column: p.character as usize });
            edits.iter().map(|e| (at(e.range.start), at(e.range.end), e.new_text.clone())).collect()
        };
        let (mut count, mut file_count, mut errors) = (0, 0, Vec::new());
        for (path, edits) in files {
            match self.tabs.iter().position(|t| uri::same_path(&t.path, &path)) {
                Some(index) => {
                    let changes = char_edits(&self.tabs[index].buffer, &edits);
                    self.edit_tab(index, Command::Replace(changes));
                }
                None => {
                    let result = std::fs::read_to_string(&path).and_then(|text| {
                        let mut buffer = Buffer::new(&text, Language::from_path(&path));
                        buffer.apply(Command::Replace(char_edits(&buffer, &edits)));
                        std::fs::write(&path, buffer.file_contents())
                    });
                    if let Err(error) = result {
                        errors.push(format!("{}: {error}", self.display_path(&path)));
                        continue;
                    }
                }
            }
            count += edits.len();
            file_count += 1;
        }
        self.refresh_marks();
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        self.status = Some(match errors.first() {
            Some(error) => format!("Rename incomplete, cannot write {error}"),
            None if count == 0 => format!("Nothing to rename for {from}"),
            None => format!(
                "Renamed {from} to {to}: {} in {}",
                plural(count, "occurrence"),
                plural(file_count, "file")
            ),
        });
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
        self.snippet = None;
    }

    /// The active GDScript tab with a connected language server.
    fn lsp_target(&self) -> Option<(lsp_client::Client, &Tab, Pos)> {
        let tab = self.active_tab().filter(|t| t.language == Language::GDScript)?;
        let client = self.lsp.client()?.clone();
        let pos = tab.buffer.char_to_pos(tab.buffer.primary().head);
        Some((client, tab, pos))
    }

    /// Asks the server for completions; snippets are offered too, and alone
    /// when the server is not connected.
    fn request_completion(&mut self, trigger: Option<char>) {
        let Some(tab) = self.active_tab().filter(|t| t.language == Language::GDScript) else {
            return;
        };
        let anchor = tab.buffer.word_start(tab.buffer.primary().head);
        let with_snippets = trigger.is_none() && snippets_allowed(&tab.buffer, anchor);
        let request = self.lsp_target().and_then(|(client, tab, pos)| {
            client.completion(&tab.path, to_lsp(pos), trigger).ok()
        });
        if let Some(id) = request {
            self.pending.insert(id, Pending::Completion);
        } else if !with_snippets {
            return;
        }
        self.completion = Some(Completion::new(request, anchor, with_snippets));
        if request.is_none() {
            self.refilter_completion();
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
        // Snippets only on a prefix match: fuzzy matches would pop them up
        // for almost any word.
        let lower = prefix.to_lowercase();
        let filtered = completion::rank(&c.items, &prefix)
            .into_iter()
            .filter(|&i| !c.is_snippet(i) || c.items[i].label.to_lowercase().starts_with(&lower))
            .collect();
        c.filtered = filtered;
        if c.filtered.is_empty() {
            self.completion = None;
            return;
        }
        let colors = code_editor::Colors::from_theme(&self.themes[self.theme_index]);
        c.rows = c.filtered.iter().map(|&i| completion_row(&c.items[i], c.is_snippet(i), &colors)).collect();
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
        let Some(&index) = c.filtered.get(row.unwrap_or(c.selected)) else {
            return;
        };
        let item = &c.items[index];
        let Some(tab) = self.active_tab() else {
            return;
        };
        let head = tab.buffer.primary().head;
        let Some(delete) = head.checked_sub(c.anchor) else {
            return;
        };
        if c.is_snippet(index) {
            self.insert_snippet(&item.insert_text, c.anchor, delete);
            return;
        }
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

    /// Replaces the `delete` chars typed before the cursor (starting at
    /// `start`) with an expanded snippet, then selects its first tab stop.
    /// With several cursors it is inserted at each, without tab stops.
    fn insert_snippet(&mut self, body: &str, start: usize, delete: usize) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let line = tab.buffer.line_text(tab.buffer.char_to_pos(start).line);
        let indent: String = line.chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
        let single = tab.buffer.selections().len() == 1;
        let expansion = snippet::expand(body, &indent);
        self.apply_command(Command::ReplaceBefore { delete, text: expansion.text, cursor_back: 0 });
        if single {
            let stops = expansion.stops.iter().map(|&(s, e)| (start + s, start + e)).collect();
            self.snippet = Some(SnippetSession { stops, current: 0 });
            self.goto_tab_stop(0);
        }
    }

    /// Selects tab stop `index` of the snippet being filled in; reaching the
    /// last one (the final cursor) ends it.
    fn goto_tab_stop(&mut self, index: usize) {
        let Some(session) = &mut self.snippet else {
            return;
        };
        let Some(&stop) = session.stops.get(index) else {
            return;
        };
        session.current = index;
        let last = index + 1 == session.stops.len();
        self.apply_command(Command::Select { ranges: vec![stop], primary: 0 });
        if last {
            self.snippet = None;
        }
    }

    /// Keeps tab stops in place as the user types in the current one; an
    /// edit elsewhere, or moving out of the snippet, ends it.
    fn track_snippet(&mut self, len_before: usize) {
        let (Some(session), Some(tab)) = (&mut self.snippet, self.active.and_then(|i| self.tabs.get(i))) else {
            return;
        };
        let buffer = &tab.buffer;
        let head = buffer.primary().head;
        let delta = buffer.len_chars() as isize - len_before as isize;
        let (start, end) = session.stops[session.current];
        let inside = if delta == 0 {
            let first = session.stops.iter().map(|s| s.0).min().unwrap_or(start);
            let last = session.stops.iter().map(|s| s.1).max().unwrap_or(end);
            (first..=last).contains(&head)
        } else {
            let new_end = end as isize + delta;
            let ok = new_end >= start as isize && (start as isize..=new_end).contains(&(head as isize));
            if ok {
                let current = session.current;
                for (index, stop) in session.stops.iter_mut().enumerate() {
                    if index == current {
                        stop.1 = new_end as usize;
                    } else if stop.0 >= end {
                        stop.0 = (stop.0 as isize + delta) as usize;
                        stop.1 = (stop.1 as isize + delta) as usize;
                    }
                }
            }
            ok
        };
        if !inside || buffer.selections().len() > 1 {
            self.snippet = None;
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

    fn on_response(&mut self, pending: Pending, id: u64, value: serde_json::Value) -> Task<Message> {
        match pending {
            Pending::References(name) => self.show_references(name, lsp_client::locations(&value)),
            Pending::PrepareRename { path, pos, word } => {
                // A range means the symbol can be renamed; null means it can't
                // (e.g. a built-in class or method).
                if lsp_client::range(&value).is_some() {
                    return self.open_rename(path, pos, word);
                }
                self.status = Some(format!("{word} cannot be renamed"));
            }
            Pending::Rename { from, to } => {
                self.apply_workspace_edit(&from, &to, lsp_client::workspace_edit(&value));
            }
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
                        c.set_items(lsp_client::completion_items(&value));
                        c.request = None;
                        self.refilter_completion();
                    }
                }
            }
            Pending::Resolve(index) => {
                let Some(resolved) = lsp_client::completion_item(&value) else {
                    return Task::none();
                };
                let Some(c) = &mut self.completion else {
                    return Task::none();
                };
                let Some(item) = c.items.get_mut(index).filter(|i| i.label == resolved.label) else {
                    return Task::none();
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
        Task::none()
    }

    fn on_lsp(&mut self, event: LspEvent) -> Task<Message> {
        match event {
            LspEvent::Connected { client, label, warning } => {
                for tab in self.tabs.iter().filter(|t| t.language == Language::GDScript) {
                    let _ = client.did_open(&tab.path, tab.version, &tab.buffer.text());
                }
                if warning.is_some() {
                    self.status = warning.clone();
                } else if self.lsp_retries > 0 {
                    self.status = Some("Language server reconnected".to_string());
                }
                self.lsp_retries = 0;
                self.lsp = Lsp::Ready { client, label, warning };
            }
            LspEvent::Failed(error) => {
                self.status = Some(error);
                // A failed reconnect (e.g. Godot still restarting) tries again.
                if self.lsp_retries > 0 {
                    return self.schedule_reconnect();
                }
                self.lsp = Lsp::Failed;
            }
            LspEvent::Event(Event::Diagnostics { path, diagnostics }) => {
                self.problems.set(path, diagnostics);
                self.refresh_marks();
            }
            LspEvent::Event(Event::Response { id, result }) => {
                let Some(pending) = self.pending.remove(&id) else {
                    return Task::none();
                };
                match (result, pending) {
                    (Ok(value), pending) => return self.on_response(pending, id, value),
                    // A server without prepareRename: let it judge the rename itself.
                    (Err(_), Pending::PrepareRename { path, pos, word }) => return self.open_rename(path, pos, word),
                    (Err(error), Pending::Definition) => self.status = Some(format!("Go to definition failed: {error}")),
                    (Err(error), Pending::References(_)) => self.status = Some(format!("Find references failed: {error}")),
                    (Err(error), Pending::Rename { .. }) => self.status = Some(format!("Rename failed: {error}")),
                    (Err(_), _) => {}
                }
            }
            LspEvent::Event(Event::Disconnected(reason)) => {
                if let Some(reason) = reason {
                    self.status = Some(format!("Language server disconnected: {reason}"));
                }
                self.pending.clear();
                self.problems.clear();
                self.refresh_marks();
                return self.schedule_reconnect();
            }
            LspEvent::Event(Event::Notification { .. }) => {}
        }
        Task::none()
    }

    /// Restarts the language server after a growing delay, or gives up
    /// (leaving the manual retry) once every delay has been tried.
    fn schedule_reconnect(&mut self) -> Task<Message> {
        let Some(&seconds) = RECONNECT_DELAYS.get(self.lsp_retries as usize) else {
            self.lsp = Lsp::Disconnected { retry_in: None };
            return Task::none();
        };
        self.lsp = Lsp::Disconnected { retry_in: Some(seconds) };
        let generation = self.lsp_generation;
        Task::perform(sleep(std::time::Duration::from_secs(seconds)), move |()| {
            Message::ReconnectLsp(generation)
        })
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
                        message: Some(d.message.split_whitespace().collect::<Vec<_>>().join(" ")),
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
        self.lsp_retries = 0;
        self.reset_lsp(if godot.is_some() { Lsp::Starting } else { Lsp::Off });
        self.recent = recent::add(&root);
        self.project = Some(Project {
            explorer: Explorer::new(root.clone()),
            root,
            godot,
        });
        self.tabs.clear();
        self.active = None;
        self.rename = None;
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
        // Opening a file shows it, even from the shortcuts page.
        self.shortcuts = None;
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
                    disk_changed: false,
                };
                if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
                    let _ = client.did_open(&tab.path, tab.version, &tab.buffer.text());
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
        match std::fs::write(&tab.path, tab.buffer.file_contents()) {
            Ok(()) => {
                tab.dirty = false;
                tab.disk_changed = false;
                if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
                    let _ = client.did_save(&tab.path, &tab.buffer.text());
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
            // Pages that don't need a project replace the welcome screen.
            None if self.theme_editor.is_some() || self.shortcuts.is_some() => self.editor_area(),
            None => self.welcome(),
            Some(project) => {
                let mut main = column![self.editor_area()];
                if let Some(panel) = self.panel {
                    main = main.push(self.bottom_panel(panel));
                }
                let body = row![self.sidebar(project), main];
                match &self.palette {
                    // Drawn over everything, centered near the top. A
                    // backdrop under it catches clicks outside the box, which
                    // close it (and don't reach the editor or panels).
                    Some(palette) => {
                        let ui = &self.current_theme().ui;
                        let close = Message::Palette(PaletteMessage::Close);
                        let backdrop = mouse_area(
                            container(space())
                                .width(Length::Fill)
                                .height(Length::Fill)
                                .style(|_| container::Style {
                                    background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.15).into()),
                                    ..container::Style::default()
                                }),
                        )
                        .on_press(close.clone());
                        let palette = palette.view(ui).map(Message::Palette);
                        let palette = on_escape::on_escape(palette, close);
                        let layer = container(palette)
                            .center_x(Length::Fill)
                            .padding(iced::Padding { top: 48.0, ..iced::Padding::ZERO });
                        iced::widget::stack![body, backdrop, layer].into()
                    }
                    None => body.into(),
                }
            }
        };
        column![container(body).height(Length::Fill), self.status_bar()].into()
    }

    fn shortcuts_page(&self) -> Element<'_, Message> {
        let Some(page) = &self.shortcuts else {
            return space().into();
        };
        let ui = &self.current_theme().ui;
        let view = page.view(ui).map(Message::Shortcuts);
        let view = on_escape::on_escape(view, Message::Shortcuts(ShortcutsMessage::Close));
        container(view).width(Length::Fill).height(Length::Fill).style(style::surface(ui)).into()
    }

    fn welcome(&self) -> Element<'_, Message> {
        let ui = &self.current_theme().ui;
        let muted = style::to_color(ui.text_muted);
        let mut content = column![
            text("Godot IDE").size(28),
            text("Open a folder containing project.godot").color(muted),
            button(text("Open Folder…")).on_press(Message::OpenFolderDialog),
        ]
        .spacing(12)
        .align_x(Alignment::Center);

        if !self.recent.is_empty() {
            let rows = self.recent.iter().map(|project| {
                let mut title = row![text(&project.name).size(14)].spacing(8).align_y(Alignment::Center);
                if let Some(version) = &project.godot_version {
                    title = title.push(text(format!("Godot {version}")).size(11).color(muted));
                }
                if project.missing {
                    title = title.push(text("not found").size(11).color(style::to_color(ui.danger)));
                }
                let path = text(project.root.display().to_string()).size(11).color(muted);
                let open = button(column![title, path].spacing(2))
                    .on_press_maybe((!project.missing).then(|| Message::OpenRecent(project.root.clone())))
                    .style(button::text)
                    .width(Length::Fill)
                    .padding([4, 8]);
                let forget = button(text("×").size(13))
                    .on_press(Message::RemoveRecent(project.root.clone()))
                    .style(button::text)
                    .padding([4, 8]);
                row![open, style::tip(forget, "Remove from this list", tooltip::Position::Left, ui)]
                    .align_y(Alignment::Center)
                    .into()
            });
            let list = scrollable(iced::widget::Column::with_children(rows).width(Length::Fill)).height(Length::Shrink);
            content = content.push(space().height(12)).push(
                column![text("RECENT PROJECTS").size(12).color(muted), container(list).max_height(420)]
                    .spacing(6)
                    .width(520),
            );
        }
        center(content).into()
    }

    fn sidebar<'a>(&'a self, project: &'a Project) -> Element<'a, Message> {
        let header = row![
            text(self.project_name(project).to_uppercase()).size(12),
            space::horizontal(),
            style::tip(
                button(text("Open…").size(12))
                    .on_press(Message::OpenFolderDialog)
                    .style(button::text)
                    .padding([2, 6]),
                "Open a project folder (Ctrl+O)",
                tooltip::Position::Bottom,
                &self.current_theme().ui,
            ),
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
        if let Some(editor) = &self.theme_editor {
            return editor.view().map(Message::ThemeEditor);
        }
        if self.shortcuts.is_some() {
            return self.shortcuts_page();
        }
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
            .focused(self.focus == Focus::Editor)
            .tab_stops(self.snippet.is_some());

        let mut area = column![self.tab_bar()];
        if let Some(rename) = &self.rename {
            let bar = rename.view(&self.current_theme().ui).map(Message::Rename);
            area = area.push(on_escape::on_escape(bar, Message::Rename(RenameMessage::Close)));
        }
        let editor = match &self.find {
            Some(find) => {
                let bar = find.view(&self.current_theme().ui).map(Message::Find);
                area = area.push(on_escape::on_escape(bar, Message::Find(FindMessage::Close)));
                editor.highlights(&find.matches, find.current)
            }
            None => editor,
        };
        area.push(editor).into()
    }

    /// The bottom panel: Problems / Output / Terminal tabs and the open one.
    fn bottom_panel(&self, panel: Panel) -> Element<'_, Message> {
        let theme = self.current_theme();
        let ui = &theme.ui;
        let muted = style::to_color(ui.text_muted);
        let action = |label, message, description: &str| {
            let button = button(text(label).size(12)).on_press(message).style(button::text).padding([0, 6]);
            style::tip(button, description, tooltip::Position::Top, ui)
        };

        let (errors, warnings) = self.problems.counts();
        let tabs = [
            (Panel::Problems, format!("PROBLEMS  {errors}/{warnings}")),
            (Panel::Output, if self.is_running() { "OUTPUT (running)".to_string() } else { "OUTPUT".to_string() }),
            (Panel::Terminal, "TERMINAL".to_string()),
            (Panel::Search, "SEARCH".to_string()),
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
            Panel::Output if self.is_running() => header = header.push(action("stop", Message::StopRun, "Stop the game (F8)")),
            Panel::Output => header = header.push(action("clear", Message::ClearOutput, "Clear the output")),
            Panel::Terminal => header = header.push(action("kill", Message::KillTerminal, "End the shell; the next toggle starts a new one")),
            Panel::Problems | Panel::Search => {}
        }
        header = header.push(action("x", Message::ClosePanel, "Close the panel"));

        let body: Element<'_, Message> = match panel {
            Panel::Problems => self.problems.view(ui, |p| self.display_path(p)),
            Panel::Search => self.search.view(ui, |p| self.display_path(p)).map(Message::Search),
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
            let marker = match (tab.dirty, tab.disk_changed) {
                (_, true) => " (changed on disk)",
                (true, false) => " *",
                (false, false) => "",
            };
            let label = format!("{}{marker}", tab.title());
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
                style::tip(
                    button(text("×").size(13))
                        .on_press(Message::CloseTab(index))
                        .style(button::text)
                        .padding([4, 6]),
                    "Close (Ctrl+W)",
                    tooltip::Position::Bottom,
                    &self.current_theme().ui,
                ),
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
                let counts = button(
                    row![
                        text(format!("E {errors}")).size(12).color(style::to_color(ui.danger)),
                        text(format!("W {warnings}")).size(12).color(style::to_color(ui.warning)),
                    ]
                    .spacing(8),
                )
                .on_press(Message::ToggleProblems)
                .style(button::text)
                .padding([0, 4]);
                let description = format!("{errors} errors, {warnings} warnings found by Godot. Click to list them");
                left.push(style::tip(counts, description, tooltip::Position::Top, ui));
                let mut label = text(self.lsp.label()).size(12);
                if self.lsp.warning().is_some() {
                    label = label.color(style::to_color(ui.warning));
                }
                let lsp = button(label).on_press(Message::RestartLsp).style(button::text).padding([0, 4]);
                let mut description =
                    "Godot's language server (completion, errors, go to definition). Click to restart it".to_string();
                if let Some(warning) = self.lsp.warning() {
                    description = format!("{warning}.\n{description}");
                }
                left.push(style::tip(lsp, description, tooltip::Position::Top, ui));
            }
        }
        if self.project.is_some() {
            let terminal = button(text("Terminal").size(12))
                .on_press(Message::ToggleTerminal)
                .style(button::text)
                .padding([0, 4]);
            left.push(style::tip(terminal, "Show or hide the terminal (Ctrl+J)", tooltip::Position::Top, ui));
            let (label, message, color, description) = if self.is_running() {
                ("Stop (F8)", Message::StopRun, ui.danger, "Stop the running game")
            } else {
                ("Run (F5)", Message::RunProject, ui.success, "Run the main scene. F6 runs the current scene")
            };
            let run = button(text(label).size(12).color(style::to_color(color)))
                .on_press(message)
                .style(button::text)
                .padding([0, 4]);
            left.push(style::tip(run, description, tooltip::Position::Top, ui));
        }
        if self.active_tab().is_some_and(|t| t.disk_changed) {
            let reload = button(text("Reload from disk").size(12).color(style::to_color(ui.warning)))
                .on_press(Message::ReloadActive)
                .style(button::text)
                .padding([0, 4]);
            let description = "Take the version on disk and discard your unsaved edits. Ctrl+S keeps yours";
            left.push(style::tip(reload, description, tooltip::Position::Top, ui));
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
                Some(self.themes[self.theme_index].name.clone()),
                Message::ThemeSelected,
            )
            .text_size(12)
            .padding([2, 6])
            .into(),
        );
        let edit = button(text("edit").size(12))
            .on_press(Message::OpenThemeEditor)
            .style(button::text)
            .padding([0, 4]);
        right.push(style::tip(edit, "Edit, import or share themes", tooltip::Position::Top, ui));
        let help = button(text("?").size(12))
            .on_press(Message::OpenShortcuts)
            .style(button::text)
            .padding([0, 4]);
        right.push(style::tip(help, "Keyboard shortcuts (F1)", tooltip::Position::Top, ui));

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
    let (key, modifiers) = match event {
        keyboard::Event::KeyPressed { key, modifiers, .. } => (key, modifiers),
        keyboard::Event::ModifiersChanged(modifiers) => return Some(Message::ModifiersChanged(modifiers)),
        keyboard::Event::KeyReleased { .. } => return None,
    };
    match key.as_ref() {
        keyboard::Key::Named(keyboard::key::Named::F12) if modifiers.shift() => Some(Message::FindReferences),
        keyboard::Key::Named(keyboard::key::Named::F12) => Some(Message::GotoDefinition),
        keyboard::Key::Named(keyboard::key::Named::F2) => Some(Message::StartRename),
        keyboard::Key::Named(keyboard::key::Named::F1) => Some(Message::OpenShortcuts),
        keyboard::Key::Named(keyboard::key::Named::F3) if modifiers.shift() => {
            Some(Message::Find(FindMessage::Previous))
        }
        keyboard::Key::Named(keyboard::key::Named::F3) => Some(Message::Find(FindMessage::Next)),
        // Palette list navigation; ignored while it is closed. (Text inputs
        // leave these keys alone, and the editor captures them itself.)
        keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some(Message::Palette(PaletteMessage::Move(-1))),
        keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some(Message::Palette(PaletteMessage::Move(1))),
        keyboard::Key::Named(keyboard::key::Named::PageUp) => Some(Message::Palette(PaletteMessage::Move(-12))),
        keyboard::Key::Named(keyboard::key::Named::PageDown) => Some(Message::Palette(PaletteMessage::Move(12))),
        keyboard::Key::Character("p" | "P") if modifiers.command() && modifiers.shift() => {
            Some(Message::OpenPalette(">".to_string()))
        }
        keyboard::Key::Character("p") if modifiers.command() => Some(Message::OpenPalette(String::new())),
        keyboard::Key::Character("g") if modifiers.command() => Some(Message::OpenPalette(":".to_string())),
        keyboard::Key::Character("f" | "F") if modifiers.command() && modifiers.shift() => Some(Message::OpenSearch),
        keyboard::Key::Character("f") if modifiers.command() => Some(Message::OpenFind { replace: false }),
        keyboard::Key::Character("h") if modifiers.command() => Some(Message::OpenFind { replace: true }),
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

/// Completes after `duration`, without an async runtime timer.
async fn sleep(duration: std::time::Duration) {
    let (done, wait) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        let _ = done.send(());
    });
    let _ = wait.await;
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

/// Whether snippets fit where a word starts at `anchor`: not after `.`,
/// `$`, `@` and the like, and not inside a string or comment.
fn snippets_allowed(buffer: &Buffer, anchor: usize) -> bool {
    use editor_core::gdscript::TokenKind;
    let before = anchor.checked_sub(1).map(|i| buffer.slice(i, anchor));
    if before.is_some_and(|c| matches!(c.as_str(), "." | "$" | "%" | "@" | "&" | "^" | "\"" | "'")) {
        return false;
    }
    let pos = buffer.char_to_pos(anchor);
    let line = buffer.line_text(pos.line);
    let byte = line.char_indices().nth(pos.column).map_or(line.len(), |(b, _)| b);
    !buffer.tokens(pos.line).iter().any(|t| {
        let quoted = matches!(t.kind, TokenKind::String | TokenKind::StringName | TokenKind::NodePath | TokenKind::Comment);
        quoted && t.range.start < byte && byte <= t.range.end
    })
}

fn completion_row(item: &lsp_client::CompletionItem, snippet: bool, colors: &code_editor::Colors) -> CompletionRow {
    let syntax = &colors.syntax;
    if snippet {
        return CompletionRow { label: item.label.clone(), tag: "snip", color: style::to_color(syntax.annotation) };
    }
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
