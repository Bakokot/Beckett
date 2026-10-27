#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod changes;
mod class_ref;
mod code_editor;
mod completion;
mod context_menu;
mod drag;
mod debugger;
mod docs;
mod explorer;
mod file_icons;
mod file_ops;
mod find;
mod format;
mod git;
mod keymap;
mod lsp;
mod new_script;
mod numpad;
mod on_escape;
mod outline;
mod palette;
mod paths;
mod problems;
mod project_panel;
mod project_symbols;
mod recent;
mod rename;
mod runner;
mod session;
mod settings;
mod shortcuts;
mod split;
mod splitter;
mod style;
mod terminal_links;
mod terminal_panel;
mod terminal_view;
mod theme_editor;
mod watcher;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use editor_core::diff::{self, Hunk, LineChange};
use editor_core::gdscript::TokenKind;
use editor_core::{snippet, Buffer, Command, Language, Pos};
use godot_project::GodotProject;
use iced::widget::{
    button, center, column, container, mouse_area, pick_list, responsive, row, scrollable, space, text, tooltip,
    Row,
};
use iced::{keyboard, Alignment, Element, Length, Subscription, Task};
use lsp_client::{uri, CompletionKind, Event, Position, Severity};

use context_menu::{context_area, fallback_context_area};
use code_editor::{
    CodeEditor, CompletionAction, CompletionPopup, CompletionRow, EditorEvent, HoverPopup, Mark,
    SignaturePopup,
};
use completion::{Completion, InString};
use docs::Doc;
use explorer::Explorer;
use editor_core::search::LineMatch;
use find::{FileResult, Find, FindMessage, ProjectSearch, SearchMessage};
use palette::{CommandEntry, Palette, PaletteMessage};
use lsp::{Lsp, LspEvent};
use problems::Problems;
use project_panel::ProjectInfo;
use file_ops::{FileOp, FileOpBar, FileOpMessage};
use new_script::{NewScript, NewScriptMessage};
use rename::{Rename, RenameMessage};
use class_ref::{ClassRef, ClassRefMessage};
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
        .scale_factor(App::scale_factor)
        .exit_on_close_request(false)
        .subscription(App::subscription)
        .window_size(WINDOW_SIZE)
        .run()
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenFolderDialog,
    /// Back to the welcome screen (the session is kept).
    CloseFolder,
    /// Periodic session save, so a crash loses little.
    AutosaveSession,
    /// Git status of this repository, and the staged text of open files.
    GitRefreshed(PathBuf, Option<Box<git::Snapshot>>),
    /// The window came to the front (git may have changed meanwhile).
    WindowFocused,
    /// Add Godot's `.gitignore` / `.gitattributes` lines to the project.
    AddGodotIgnores,
    /// Read the git status again now.
    RefreshGit,
    /// Open the CHANGES panel (at this file only, when given).
    ShowChanges(Option<PathBuf>),
    /// Put back the staged lines of this hunk, in the file's tab.
    RevertHunk(PathBuf, Hunk),
    /// The window is closing: save the session, then close.
    CloseRequested(iced::window::Id),
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
    /// An event of the editor on one side of the split.
    EditIn(usize, EditorEvent),
    /// Ctrl+\: move the current tab to the other side (splitting the view).
    SplitMove,
    FocusOtherSide,
    Save,
    ThemeSelected(String),
    /// List the VS Code themes installed here, to import one.
    ChooseVsCodeTheme,
    /// List the Godot editor settings found here, to import their colors.
    ChooseGodotEditorTheme,
    /// Import this theme file (and switch to it).
    ImportThemeFile(PathBuf),
    /// Import all these theme files, keeping the current theme.
    ImportThemeFiles(Vec<PathBuf>),
    /// The OS is (or went) dark or light.
    SystemTheme(iced::theme::Mode),
    /// Turn following the OS light/dark mode on or off.
    ToggleFollowOsTheme,
    /// An event of the language server connection of this generation.
    Lsp(u64, LspEvent),
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
    /// An event of the shell of this terminal tab.
    Terminal(u64, TerminalEvent),
    /// Start another shell in a new terminal tab.
    NewTerminal,
    SelectTerminal(u64),
    CloseTerminal(u64),
    ShowPanel(Panel),
    ShowSidebar(SidebarView),
    /// List every use of this word in the project (whole word, case
    /// sensitive) in the SEARCH tab.
    SearchProject(String),
    ClosePanel,
    /// The splitter above the bottom panel moved by this many pixels.
    ResizePanel(f32),
    /// The bar between the two sides moved by this many pixels, in an
    /// editor area this wide.
    ResizeSplit(f32, f32),
    WindowResized(iced::Size),
    /// Zoom the interface in (positive) or out by this many steps.
    Zoom(i32),
    ZoomReset,
    /// Run the main scene (F5).
    RunProject,
    /// Run under the debugger: the main scene, or the current one.
    StartDebug { scene: bool },
    /// F9: add or remove a breakpoint on the cursor's line.
    ToggleBreakpoint,
    /// An event of the debug session of this generation.
    Debugger(u64, debugger::DebugEvent),
    Debug(debugger::DebugMessage),
    /// Ask again for variables Godot wasn't ready to give (session, reference, attempt).
    DebugRetry(u64, i64, u32),
    /// Run the scene of the active tab (F6).
    RunScene,
    /// Pick which installed Godot the project uses.
    ChooseGodot,
    /// Use this Godot for the project (`None`: the best match found).
    GodotChosen(Option<PathBuf>),
    /// Pick which scene F6 runs for the current script.
    ChooseScene,
    /// Run this scene for this script, and remember the choice.
    RunChosenScene(PathBuf, String),
    /// Open the project, at the current scene, in the Godot editor.
    OpenInGodot,
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
    /// A key event no widget used (shortcuts, palette navigation).
    Key(keyboard::Event),
    /// Open the palette with this input ("" files, ">" commands, ":" go to line).
    OpenPalette(String),
    Palette(PaletteMessage),
    OpenThemeEditor,
    /// F1: the keyboard shortcuts page.
    OpenShortcuts,
    Shortcuts(ShortcutsMessage),
    /// The class reference, at this class (and member) when given.
    OpenClassRef(Option<String>, Option<String>),
    /// Bring the class reference tab to the front.
    ShowClassRef,
    ClassRef(ClassRefMessage),
    ThemeEditor(ThemeMessage),
    ThemeImportPicked(Option<PathBuf>),
    ThemeExportPicked(Option<PathBuf>),
    ThemeCodePasted(Option<String>),
    /// A file dropped on the window: a theme to import, or a file to open.
    FileDropped(PathBuf),
    /// Right-click on something: open its menu at this window position.
    ContextMenu(ContextTarget, iced::Point),
    CloseMenu,
    /// A menu item was chosen: close the menu, then do this.
    MenuPick(Box<Message>),
    /// Open a file on the other side of the split view.
    OpenToSide(PathBuf),
    CopyText(String),
    /// A file is being dragged from the explorer (`None`: the drag ended).
    FileDrag(Option<PathBuf>),
    /// Open the New Script bar for this folder (a file: its folder; `None`:
    /// the active file's folder).
    OpenNewScript(Option<PathBuf>),
    NewScript(NewScriptMessage),
    /// Open the bar for a new folder, rename or delete in the explorer.
    OpenFileOp(FileOp),
    FileOp(FileOpMessage),
    /// Show a file or folder in the system file manager.
    Reveal(PathBuf),
    /// Ctrl+click on a path or URL in the terminal.
    TerminalLink(terminal_links::TermLink),
    /// Close every tab but this file (`None`: but the class reference).
    CloseOtherTabs(Option<usize>),
    /// Move this file (`None`: the class reference) to the other side.
    MoveTabToOtherSide(Option<usize>),
    EditorCopy,
    EditorCut,
    EditorPaste,
    PasteText(Option<String>),
}

/// What a right-click menu is about.
#[derive(Debug, Clone)]
pub enum ContextTarget {
    File(PathBuf),
    Folder(PathBuf),
    /// A tab, by index.
    Tab(usize),
    /// The class reference tab.
    ClassRef,
    /// The code of the focused editor.
    Editor,
}

/// What the sidebar shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarView {
    #[default]
    Files,
    Outline,
    /// Main scene, autoloads and Input Map.
    Project,
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
    /// Files changed since the staged text, with their hunks.
    Changes,
    /// The debug session: call stack and variables.
    Debug,
}

/// The shells of the terminal panel, one per tab.
#[derive(Default)]
struct Terminals {
    tabs: Vec<TerminalTab>,
    /// Index of the tab shown.
    active: usize,
    /// Ids are never reused: each one keys its shell's subscription.
    next_id: u64,
}

struct TerminalTab {
    id: u64,
    /// `None` until the shell has started.
    session: Option<std::sync::Arc<terminal::Terminal>>,
    /// Set by the shell (the running program or folder).
    title: Option<String>,
}

impl Terminals {
    /// Starts a new shell in a new tab and shows it.
    fn add(&mut self) {
        self.next_id += 1;
        self.tabs.push(TerminalTab { id: self.next_id, session: None, title: None });
        self.active = self.tabs.len() - 1;
    }

    fn active(&self) -> Option<&TerminalTab> {
        self.tabs.get(self.active)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut TerminalTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Ends the shell of tab `id` (dropping its subscription stops it).
    fn remove(&mut self, id: u64) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
    }
}

const MAX_OUTPUT_LINES: usize = 5000;
const WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);
const DEFAULT_PANEL_HEIGHT: f32 = 260.0;
const MIN_PANEL_HEIGHT: f32 = 100.0;
/// Room the bottom panel always leaves for the tabs, editor and status bar.
const MIN_EDITOR_HEIGHT: f32 = 200.0;
/// Waits before each automatic language server reconnect, in seconds.
const RECONNECT_DELAYS: [u64; 5] = [1, 2, 5, 10, 20];

#[derive(Default)]
struct RunState {
    /// The current run; its subscription lives until the process exits.
    request: Option<RunRequest>,
    /// Dropping it stops the game.
    handle: Option<std::sync::Arc<RunHandle>>,
    output: Vec<OutputLine>,
    /// Picks runtime errors out of the output for the Problems panel.
    problems: runner::RuntimeParser,
    next_id: u64,
    /// Godot binary for running, found on first use.
    godot: Option<PathBuf>,
}

/// What an in-flight LSP request was for.
enum Pending {
    Definition,
    Completion,
    /// Hover asked after an empty definition: an engine symbol opens its docs.
    NativeDefinition,
    /// The docs of this engine class.
    ClassDoc(String),
    /// Docs for `items[index]` of the current completion.
    Resolve(usize),
    Hover(Pos),
    Signature(Pos),
    /// References to the named symbol.
    References(String),
    /// Whether the symbol `word` at `pos` can be renamed.
    PrepareRename { path: PathBuf, pos: Pos, word: String },
    Rename { from: String, to: String },
    /// The outline of this file at this tab version.
    Outline(PathBuf, i32),
    /// Whether the word at this char range has a definition (Ctrl+hover).
    LinkProbe((usize, usize)),
    /// The same word without a definition: whether its hover names an
    /// engine symbol (Ctrl+click opens the class reference then).
    NativeLinkProbe((usize, usize)),
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
    /// The session as last written, to skip rewriting an unchanged one.
    saved_session: Option<session::Session>,
    settings: settings::Settings,
    /// Shortcuts in effect (defaults with the user's changes).
    keymap: keymap::Keymap,
    tabs: Vec<Tab>,
    /// The side of the split with keyboard focus (0 left, 1 right); `active`
    /// is the tab it shows.
    focused_group: usize,
    /// The tab shown on each side.
    group_active: [Option<usize>; 2],
    active: Option<usize>,
    themes: Vec<theme::Theme>,
    theme_index: usize,
    /// Whether the OS is in dark or light mode, once known.
    os_theme: Option<theme::ThemeKind>,
    iced_theme: iced::Theme,
    /// Dirty tab the user already tried to close once; a second close discards it.
    pending_close: Option<PathBuf>,
    status: Option<String>,
    lsp: Lsp,
    /// Bumped to restart the language server subscription.
    lsp_generation: u64,
    /// Automatic reconnects since the server was last connected.
    lsp_retries: u32,
    /// This connection waits for a Godot editor the IDE just started and
    /// never spawns a server. Part of the connection's identity, so it is
    /// only reset together with `lsp_generation`.
    lsp_wait_editor: bool,
    pending: HashMap<u64, Pending>,
    problems: Problems,
    /// Height of the bottom panel, set with the splitter above it.
    panel_height: f32,
    /// Share of the editor area's width the left side takes when split.
    split_ratio: f32,
    window_height: f32,
    window_width: f32,
    /// The open right-click menu.
    context_menu: Option<context_menu::Menu<Message>>,
    /// File being dragged from the explorer, droppable into code.
    file_drag: Option<PathBuf>,
    /// The New Script bar, and why its last submit failed.
    new_script: Option<(NewScript, Option<String>)>,
    /// The new folder / rename / delete bar.
    file_op: Option<FileOpBar>,
    git_refresh: GitRefresh,
    /// The CHANGES panel shows only this file.
    changes_focus: Option<PathBuf>,
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
    /// The class reference tab (one at most), on side `class_ref_side`.
    class_ref: Option<ClassRef>,
    class_ref_side: usize,
    /// Whether that side shows it rather than one of its files.
    class_ref_shown: bool,
    /// Whether it is the active tab (shown, on the focused side); `active`
    /// then keeps the file its side showed before, if any.
    class_ref_focused: bool,
    /// The class reference as it was closed, to reopen at the same class.
    hidden_class_ref: Option<ClassRef>,
    /// Engine classes, from the language server.
    native_classes: Vec<lsp_client::NativeClass>,
    // Popups for the active tab.
    completion: Option<Completion>,
    hover: Option<(Pos, Doc)>,
    signature: Option<Signature>,
    snippet: Option<SnippetSession>,
    sidebar_view: SidebarView,
    /// Outline of the active GDScript file, from the language server.
    outline: Option<outline::Outline>,
    outline_request: Option<u64>,
    /// Ctrl+click target under the mouse, underlined while Ctrl is held.
    link: Option<(usize, usize)>,
    /// Definition request checking whether the hovered word is a target.
    link_probe: Option<u64>,
    /// `uid://` lookups (they scan the project), cleared when files change.
    uid_cache: HashMap<String, Option<PathBuf>>,
    /// Scene F6 runs for a script used by several scenes, once picked.
    scene_choice: HashMap<PathBuf, String>,
    /// The Godot executable chosen for this project (`None`: the best
    /// installed match for its version). Kept in the session.
    godot_choice: Option<PathBuf>,
    terminals: Terminals,
    /// Breakpoint lines (zero-based) by file; kept in the session.
    breakpoints: std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<usize>>,
    /// The debug session, while one runs.
    debug: Option<debugger::Session>,
    /// Bumped for each session, so a finished one's events are ignored.
    debug_generation: u64,
    /// Watch expressions, evaluated at each pause; kept in the session.
    watches: Vec<String>,
    focus: Focus,
}

struct Project {
    root: PathBuf,
    godot: Option<GodotProject>,
    /// Autoloads and Input Map, read from `project.godot` with `godot`.
    info: ProjectInfo,
    /// The git repository holding the project, if any.
    repo: Option<PathBuf>,
    git_status: git::Status,
    /// Changed files of the repository, for the CHANGES panel.
    git_files: Vec<git::ChangedFile>,
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
    /// Side of the split it is on: 0 left, 1 right.
    group: usize,
    /// Change markers against the file's staged text.
    git: GitMarks,
}

/// Reading git status and staged texts runs in the background, one run at
/// a time; asking again during a run starts another after it.
#[derive(Default)]
struct GitRefresh {
    wanted: bool,
    running: bool,
}

/// A tab's lines changed since the text staged in git, recomputed when the
/// buffer or the staged text changes.
#[derive(Default)]
struct GitMarks {
    /// The staged text; `None` outside a repository or for untracked files.
    base: Option<String>,
    /// Bumped when `base` changes.
    base_id: u64,
    /// (buffer version, base id) the diff was computed for, its hunks and
    /// their gutter markers.
    cache: RefCell<Option<GitDiff>>,
}

struct GitDiff {
    key: (u64, u64),
    hunks: Rc<Vec<Hunk>>,
    changes: Rc<Vec<LineChange>>,
}

impl GitMarks {
    fn set_base(&mut self, base: Option<String>) {
        if self.base != base {
            self.base = base;
            self.base_id += 1;
        }
    }

    fn changes(&self, buffer: &Buffer) -> Rc<Vec<LineChange>> {
        self.diff(buffer, |d| Rc::clone(&d.changes))
    }

    fn hunks(&self, buffer: &Buffer) -> Rc<Vec<Hunk>> {
        self.diff(buffer, |d| Rc::clone(&d.hunks))
    }

    /// Reads the diff of the buffer's current version, computing it if needed.
    fn diff<T: Default>(&self, buffer: &Buffer, read: impl Fn(&GitDiff) -> T) -> T {
        let Some(base) = &self.base else {
            return T::default();
        };
        let key = (buffer.version(), self.base_id);
        if let Some(diff) = self.cache.borrow().as_ref().filter(|d| d.key == key) {
            return read(diff);
        }
        let hunks = diff::hunks(base, &buffer.text());
        let changes = hunks.iter().map(Hunk::change).collect();
        let diff = GitDiff { key, hunks: Rc::new(hunks), changes: Rc::new(changes) };
        let value = read(&diff);
        *self.cache.borrow_mut() = Some(diff);
        value
    }
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
        let settings = settings::Settings::load();
        let theme_index = settings.theme.as_ref().and_then(|name| themes.iter().position(|t| &t.name == name)).unwrap_or(0);
        let iced_theme = style::iced_theme(&themes[theme_index]);
        let sidebar_view = settings.sidebar;

        let mut app = Self {
            project: None,
            recent: recent::load(),
            saved_session: None,
            settings,
            keymap: keymap::Keymap::load(),
            tabs: Vec::new(),
            focused_group: 0,
            group_active: [None, None],
            active: None,
            themes,
            theme_index,
            os_theme: None,
            iced_theme,
            pending_close: None,
            status,
            lsp: Lsp::Off,
            lsp_generation: 0,
            lsp_retries: 0,
            lsp_wait_editor: false,
            pending: HashMap::new(),
            problems: Problems::default(),
            panel: None,
            panel_height: DEFAULT_PANEL_HEIGHT,
            split_ratio: 0.5,
            window_height: WINDOW_SIZE.1,
            window_width: WINDOW_SIZE.0,
            context_menu: None,
            file_drag: None,
            new_script: None,
            file_op: None,
            git_refresh: GitRefresh::default(),
            changes_focus: None,
            run: RunState::default(),
            find: None,
            rename: None,
            search: ProjectSearch::default(),
            modifiers: keyboard::Modifiers::default(),
            palette: None,
            theme_paths,
            theme_editor: None,
            shortcuts: None,
            class_ref: None,
            class_ref_side: 0,
            class_ref_shown: false,
            class_ref_focused: false,
            hidden_class_ref: None,
            native_classes: Vec::new(),
            completion: None,
            hover: None,
            signature: None,
            snippet: None,
            sidebar_view,
            outline: None,
            outline_request: None,
            link: None,
            link_probe: None,
            uid_cache: HashMap::new(),
            scene_choice: HashMap::new(),
            godot_choice: None,
            terminals: Terminals::default(),
            breakpoints: std::collections::BTreeMap::new(),
            debug: None,
            debug_generation: 0,
            watches: Vec::new(),
            focus: Focus::Editor,
        };
        // Without a folder to open, pick up where the last session left off.
        let last = app.recent.first().filter(|r| !r.missing).map(|r| r.root.clone());
        if let Some(path) = initial.or(last) {
            app.open_path(path);
        }
        (app, Task::batch([autosave_later(), iced::system::theme().map(Message::SystemTheme)]))
    }

    fn title(&self) -> String {
        let mut parts = Vec::new();
        if let Some(tab) = self.active_tab() {
            parts.push(format!("{}{}", tab.title(), if tab.dirty { " ●" } else { "" }));
        }
        if let Some(project) = &self.project {
            parts.push(self.project_name(project));
        }
        parts.push("Beckett".to_string());
        parts.join(" — ")
    }

    fn theme(&self) -> iced::Theme {
        self.iced_theme.clone()
    }

    fn scale_factor(&self) -> f32 {
        self.settings.ui_scale
    }

    /// Changes the interface zoom and saves it.
    fn set_zoom(&mut self, change: impl FnOnce(&mut settings::Settings)) {
        change(&mut self.settings);
        let percent = (self.settings.ui_scale * 100.0).round();
        self.status = Some(match self.settings.save() {
            Ok(()) => format!("Zoom {percent}% (Ctrl+0 resets)"),
            Err(error) => format!("Zoom {percent}%, not saved: {error}"),
        });
    }

    /// The theme in effect: the theme editor's draft while it is open.
    fn current_theme(&self) -> &theme::Theme {
        match &self.theme_editor {
            Some(editor) => &editor.draft,
            None => &self.themes[self.theme_index],
        }
    }

    /// Re-applies the current theme after it changed.
    /// Following the OS: shows the theme chosen for its mode (not while a
    /// theme is being edited).
    fn apply_os_theme(&mut self) {
        let (true, Some(kind), None) = (self.settings.follow_os_theme, self.os_theme, &self.theme_editor) else {
            return;
        };
        let chosen = match kind {
            theme::ThemeKind::Dark => self.settings.dark_theme.as_deref(),
            theme::ThemeKind::Light => self.settings.light_theme.as_deref(),
        };
        let index = theme_for_mode(&self.themes, kind, chosen);
        if let Some(index) = index.filter(|&i| i != self.theme_index) {
            self.theme_index = index;
            self.apply_theme();
        }
    }

    fn apply_theme(&mut self) {
        self.iced_theme = style::iced_theme(self.current_theme());
        self.refresh_marks();
        // Remember the selected theme (not the editor's draft) for next start.
        let name = &self.themes[self.theme_index].name;
        if self.settings.theme.as_ref() != Some(name) {
            self.settings.theme = Some(name.clone());
            let _ = self.settings.save();
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = keyboard::listen().map(Message::Key);

        let lsp = self
            .project
            .as_ref()
            .and_then(|project| {
                let godot = project.godot.as_ref()?;
                let connection = lsp::connect(
                    project.root.clone(),
                    godot.godot_version(),
                    self.godot_choice.clone(),
                    self.lsp_generation,
                    self.lsp_wait_editor,
                );
                Some(connection.map(|(generation, event)| Message::Lsp(generation, event)))
            })
            .unwrap_or_else(Subscription::none);

        // One shell per terminal tab; they keep running while the panel is
        // hidden. Each is keyed by its tab's id.
        let terminal = match &self.project {
            Some(project) => {
                let version = project.godot.as_ref().and_then(GodotProject::godot_version);
                Subscription::batch(self.terminals.tabs.iter().map(|tab| {
                    let id = tab.id;
                    terminal_panel::spawn(id, project.root.clone(), version.clone(), self.godot_choice.clone())
                        .with(id)
                        .map(|(id, event)| Message::Terminal(id, event))
                }))
            }
            None => Subscription::none(),
        };

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
            iced::Event::Window(iced::window::Event::Resized(size)) => Some(Message::WindowResized(size)),
            iced::Event::Window(iced::window::Event::Focused) => Some(Message::WindowFocused),
            _ => None,
        });

        let close = iced::window::close_requests().map(Message::CloseRequested);
        let os_theme = iced::system::theme_changes().map(Message::SystemTheme);

        let debug = match &self.debug {
            Some(session) => debugger::connect(session.generation, session.port)
                .map(|(generation, event)| Message::Debugger(generation, event)),
            None => Subscription::none(),
        };

        Subscription::batch([keys, lsp, terminal, run, files, dropped, close, debug, os_theme])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.find_key();
        let task = self.handle(message);
        // Keep find matches in step with whichever buffer is active.
        if self.find.is_some() && self.find_key() != before {
            self.recompute_find();
        }
        self.sync_groups();
        self.refresh_outline();
        match self.start_git_refresh() {
            Some(git) => Task::batch([task, git]),
            None => task,
        }
    }

    /// Reads the git status and the staged text of every open file in the
    /// background, when asked for and none is running.
    fn start_git_refresh(&mut self) -> Option<Task<Message>> {
        let refresh = &mut self.git_refresh;
        if !refresh.wanted || refresh.running {
            return None;
        }
        refresh.wanted = false;
        let repo = self.project.as_ref()?.repo.clone()?;
        refresh.running = true;
        let open: Vec<PathBuf> = self.tabs.iter().map(|t| t.path.clone()).collect();
        let task = Task::perform(
            {
                let repo = repo.clone();
                async move { git::snapshot(&repo, &open).map(Box::new) }
            },
            move |snapshot| Message::GitRefreshed(repo.clone(), snapshot),
        );
        Some(task)
    }

    fn on_git_refreshed(&mut self, repo: PathBuf, snapshot: Option<Box<git::Snapshot>>) {
        self.git_refresh.running = false;
        let Some(project) = self.project.as_mut().filter(|p| p.repo.as_ref() == Some(&repo)) else {
            return;
        };
        let snapshot = snapshot.map(|s| *s).unwrap_or_default();
        project.git_status = snapshot.status;
        project.git_files = snapshot.files;
        for (path, base) in snapshot.bases {
            for tab in self.tabs.iter_mut().filter(|t| t.path == path) {
                tab.git.set_base(base.clone());
            }
        }
    }

    /// Keeps the split consistent after any change: the active tab decides
    /// which side has focus, each side shows one of its own tabs, and a side
    /// left without tabs closes the split. The class reference tab counts
    /// as one more tab, after the files.
    fn sync_groups(&mut self) {
        let docs = self.tabs.len();
        let mut groups: Vec<usize> = self.tabs.iter().map(|t| t.group).collect();
        let mut shown = self.group_active;
        let mut active = self.active;
        if self.class_ref.is_some() {
            groups.push(self.class_ref_side);
            if self.class_ref_shown {
                shown[self.class_ref_side] = Some(docs);
            }
            if self.class_ref_focused {
                active = Some(docs);
            }
        }
        let mut split = split::Split { active, shown, focused: self.focused_group };
        split.sync(&mut groups);
        for (tab, group) in self.tabs.iter_mut().zip(&groups) {
            tab.group = *group;
        }
        for side in 0..2 {
            self.group_active[side] = match split.shown[side] {
                // Behind the class reference: keep the file shown before it.
                Some(i) if i == docs => {
                    let kept = self.group_active[side].filter(|&i| i < docs && groups[i] == side);
                    kept.or_else(|| groups[..docs].iter().rposition(|&g| g == side))
                }
                other => other,
            };
        }
        self.focused_group = split.focused;
        if self.class_ref.is_some() {
            self.class_ref_side = groups[docs];
            self.class_ref_shown = split.shown[self.class_ref_side] == Some(docs);
            self.class_ref_focused = split.active == Some(docs);
        }
        self.active = match split.active {
            Some(i) if i == docs => self.group_active[self.focused_group],
            other => other,
        };
    }

    /// Makes a file tab the active one (in front of the class reference
    /// when that was active).
    fn focus_tab(&mut self, index: usize) {
        self.active = Some(index);
        if self.class_ref_focused {
            self.class_ref_focused = false;
            self.focus = Focus::Editor;
        }
    }

    /// An event from the editor on one side: clicking or typing there moves
    /// the focus to that side; hovering the other side does nothing.
    fn on_editor_in(&mut self, group: usize, event: EditorEvent) -> Task<Message> {
        if group != self.focused_group {
            if matches!(event, EditorEvent::Hover(_) | EditorEvent::HoverEnd | EditorEvent::LinkHover(_)) {
                return Task::none();
            }
            let Some(index) = self.group_active[group] else {
                return Task::none();
            };
            self.dismiss_popups();
            self.find = None;
            self.rename = None;
            self.focus_tab(index);
            self.sync_groups();
        }
        self.on_editor(event)
    }

    /// Moves the current tab to the other side, splitting the view (or
    /// closing the split when its side is left empty).
    fn split_move(&mut self) {
        let docs = self.class_ref_active();
        let from = match self.active {
            _ if docs => self.class_ref_side,
            Some(active) => self.tabs[active].group,
            None => return,
        };
        let to = 1 - from;
        let count = |side: usize| {
            let docs = self.class_ref.is_some() && self.class_ref_side == side;
            self.tabs.iter().filter(|t| t.group == side).count() + usize::from(docs)
        };
        if count(from) == 1 && count(to) == 0 {
            self.status = Some("Split moves the current tab to the other side: open a second tab first".to_string());
            return;
        }
        self.dismiss_popups();
        self.find = None;
        self.rename = None;
        match self.active {
            _ if docs => self.class_ref_side = to,
            Some(active) => self.tabs[active].group = to,
            None => {}
        }
    }

    /// Asks for the active file's outline while it is shown (sidebar or the
    /// palette's `@` mode) and out of date; one request at a time, so fast
    /// typing doesn't queue one per keystroke.
    fn refresh_outline(&mut self) {
        let in_palette = self.palette.as_ref().is_some_and(|p| p.input.starts_with('@'));
        if self.sidebar_view != SidebarView::Outline && !in_palette {
            return;
        }
        if self.outline_request.is_some_and(|id| self.pending.contains_key(&id)) {
            return;
        }
        let Some((client, tab, _)) = self.lsp_target() else {
            return;
        };
        let fresh = self.outline.as_ref().is_some_and(|o| uri::same_path(&o.path, &tab.path) && o.version == tab.version);
        if fresh {
            return;
        }
        let (path, version) = (tab.path.clone(), tab.version);
        if let Ok(id) = client.document_symbols(&path) {
            self.pending.insert(id, Pending::Outline(path, version));
            self.outline_request = Some(id);
        }
    }

    /// The outline as palette choices (`@`), when it is the active file's.
    fn symbol_choices(&self) -> Vec<palette::Choice<Message>> {
        let (Some(outline), Some(tab)) = (&self.outline, self.active_tab()) else {
            return Vec::new();
        };
        if !uri::same_path(&outline.path, &tab.path) {
            return Vec::new();
        }
        let syntax = &self.current_theme().syntax;
        outline
            .entries
            .iter()
            .map(|e| palette::Choice {
                label: format!("{}{}", "  ".repeat(e.depth), e.name),
                detail: format!("{}  {}", outline::tag(e.kind, syntax).0, e.line + 1),
                message: Message::JumpTo(outline.path.clone(), e.line, e.character),
            })
            .collect()
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
            Message::CloseFolder => self.close_folder(),
            Message::GitRefreshed(repo, snapshot) => self.on_git_refreshed(repo, snapshot),
            Message::WindowFocused | Message::RefreshGit => self.git_refresh.wanted = true,
            Message::AddGodotIgnores => {
                if let Some(project) = &self.project {
                    self.status = Some(match git::add_godot_ignores(&project.root) {
                        Ok(files) if files.is_empty() => "The project already has Godot's .gitignore and .gitattributes lines".to_string(),
                        Ok(files) => format!("Added Godot's lines to {}", files.join(" and ")),
                        Err(error) => error,
                    });
                }
            }
            Message::ShowChanges(path) => {
                self.changes_focus = path;
                self.git_refresh.wanted = true;
                self.show_panel(Panel::Changes);
            }
            Message::RevertHunk(path, hunk) => self.revert_hunk(&path, &hunk),
            Message::AutosaveSession => {
                // Also catches commits made in the terminal.
                self.git_refresh.wanted = true;
                self.save_session();
                return autosave_later();
            }
            Message::CloseRequested(window) => {
                // Hot exit: unsaved edits are kept in the session, not lost.
                self.save_session();
                return iced::window::close(window);
            }
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
                    self.focus_tab(index);
                    self.shortcuts = None;
                    if self.tabs[index].group == self.class_ref_side {
                        self.class_ref_shown = false;
                    }
                }
            }
            Message::OpenClassRef(class, member) => return self.open_class_ref(class, member),
            Message::ShowClassRef => self.show_class_ref(),
            Message::ClassRef(message) => return self.on_class_ref(message),
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
            Message::Shortcuts(ShortcutsMessage::Record(id)) => {
                if let Some(page) = &mut self.shortcuts {
                    page.recording = Some(id);
                }
                self.status = Some(format!("Press the new shortcut for \"{}\"", keymap::title(id)));
                // Take focus from the filter box so the keys reach the recorder.
                return iced::widget::operation::focus("shortcuts-recording");
            }
            Message::Shortcuts(ShortcutsMessage::Reset(id)) => {
                self.keymap.reset(id);
                self.status = Some(match self.keymap.save() {
                    Ok(()) => format!("{}: back to {}", keymap::title(id), self.keymap.label(id)),
                    Err(error) => format!("Cannot save shortcuts: {error}"),
                });
            }
            Message::Shortcuts(ShortcutsMessage::Preset(preset)) => {
                if let Some(page) = &mut self.shortcuts {
                    page.recording = None;
                }
                let dropped = self.keymap.set_preset(preset);
                self.status = Some(match self.keymap.save() {
                    Ok(()) if dropped > 0 => format!("{} shortcuts ({dropped} changed shortcuts reset)", preset.name()),
                    Ok(()) => format!("{} shortcuts", preset.name()),
                    Err(error) => format!("Cannot save shortcuts: {error}"),
                });
            }
            Message::Shortcuts(ShortcutsMessage::Close) => {
                // Escape while recording cancels the recording, not the page.
                if let Some(page) = self.shortcuts.as_mut().filter(|p| p.recording.is_some()) {
                    page.recording = None;
                    self.status = Some("Shortcut unchanged".to_string());
                } else {
                    self.shortcuts = None;
                    self.focus = Focus::Editor;
                }
            }
            Message::CloseTab(index) => {
                self.dismiss_popups();
                self.close_tab(index);
            }
            Message::CloseActiveTab => {
                if self.class_ref_active() {
                    self.close_class_ref();
                } else if let Some(index) = self.active {
                    self.dismiss_popups();
                    self.close_tab(index);
                }
            }
            Message::Edit(event) => return self.on_editor(event),
            Message::EditIn(group, event) => return self.on_editor_in(group, event),
            Message::SplitMove => self.split_move(),
            Message::FocusOtherSide => {
                let other = 1 - self.focused_group;
                if self.class_ref.is_some() && self.class_ref_shown && self.class_ref_side == other {
                    self.show_class_ref();
                } else if let Some(index) = self.group_active[other].filter(|_| self.is_split()) {
                    self.dismiss_popups();
                    self.focus_tab(index);
                }
            }
            Message::Save => self.save_active(),
            Message::ChooseVsCodeTheme => return self.choose_vscode_theme(),
            Message::ChooseGodotEditorTheme => return self.choose_godot_editor_theme(),
            Message::ImportThemeFile(path) => self.import_theme_file(&path),
            Message::ImportThemeFiles(paths) => self.import_theme_files(&paths),
            Message::SystemTheme(mode) => {
                self.os_theme = match mode {
                    iced::theme::Mode::Dark => Some(theme::ThemeKind::Dark),
                    iced::theme::Mode::Light => Some(theme::ThemeKind::Light),
                    iced::theme::Mode::None => None,
                };
                self.apply_os_theme();
            }
            Message::ToggleFollowOsTheme => {
                self.settings.follow_os_theme = !self.settings.follow_os_theme;
                let _ = self.settings.save();
                self.status = Some(if self.settings.follow_os_theme {
                    "The theme now follows the OS light/dark mode (picking a theme sets the one for its mode)".to_string()
                } else {
                    "The theme no longer follows the OS".to_string()
                });
                self.apply_os_theme();
            }
            Message::ThemeSelected(name) => {
                if let Some(index) = self.themes.iter().position(|t| t.name == name) {
                    // Following the OS: the picked theme becomes the one for its mode.
                    if self.settings.follow_os_theme {
                        match self.themes[index].kind {
                            theme::ThemeKind::Dark => self.settings.dark_theme = Some(name.clone()),
                            theme::ThemeKind::Light => self.settings.light_theme = Some(name.clone()),
                        }
                        let _ = self.settings.save();
                    }
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
            Message::Lsp(generation, event) => {
                if generation == self.lsp_generation {
                    return self.on_lsp(event);
                }
            }
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
            Message::ShowSidebar(view) => {
                self.sidebar_view = view;
                self.settings.sidebar = view;
                let _ = self.settings.save();
            }
            Message::SearchProject(word) => {
                if self.project.is_none() {
                    return Task::none();
                }
                let query = &mut self.search.query;
                query.text = word;
                query.case_sensitive = true;
                query.whole_word = true;
                query.regex = false;
                self.show_panel(Panel::Search);
                return self.on_search(SearchMessage::Run);
            }
            // Dragging the splitter up makes the panel taller.
            Message::ResizePanel(delta) => self.panel_height = (self.panel_height() - delta).max(MIN_PANEL_HEIGHT),
            Message::ResizeSplit(delta, width) => {
                let left = split::left_width(self.split_ratio, width) + delta;
                self.split_ratio = (left / width).clamp(0.0, 1.0);
            }
            Message::WindowResized(size) => {
                self.window_height = size.height;
                self.window_width = size.width;
            }
            Message::ContextMenu(target, position) => {
                self.dismiss_popups();
                let entries = self.context_entries(&target);
                self.context_menu = (!entries.is_empty()).then_some(context_menu::Menu { position, entries });
            }
            Message::CloseMenu => self.context_menu = None,
            Message::MenuPick(message) => {
                self.context_menu = None;
                return self.handle(*message);
            }
            Message::OpenToSide(path) => self.open_to_side(path),
            Message::CopyText(text) => {
                self.status = Some(format!("Copied {text}"));
                return iced::clipboard::write(text);
            }
            Message::FileDrag(path) => self.file_drag = path,
            Message::TerminalLink(link) => self.open_terminal_link(link),
            Message::OpenNewScript(path) => return self.open_new_script(path),
            Message::NewScript(message) => self.on_new_script(message),
            Message::OpenFileOp(op) => return self.open_file_op(op),
            Message::FileOp(message) => return self.on_file_op(message),
            Message::Reveal(path) => {
                if let Err(error) = reveal_in_file_manager(&path) {
                    self.status = Some(format!("Cannot show {}: {error}", path.display()));
                }
            }
            Message::CloseOtherTabs(keep) => self.close_other_tabs(keep),
            Message::MoveTabToOtherSide(Some(index)) => {
                if index < self.tabs.len() {
                    self.focus_tab(index);
                    self.sync_groups();
                    self.split_move();
                }
            }
            Message::MoveTabToOtherSide(None) => {
                if self.class_ref.is_some() {
                    self.show_class_ref();
                    self.sync_groups();
                    self.split_move();
                }
            }
            Message::EditorCopy => return self.copy_selection(false),
            Message::EditorCut => return self.copy_selection(true),
            Message::EditorPaste => return iced::clipboard::read().map(Message::PasteText),
            Message::PasteText(Some(text)) => self.apply_command(Command::Paste(text)),
            Message::PasteText(None) => {}
            Message::Zoom(steps) => self.set_zoom(|s| s.zoom(steps)),
            Message::ZoomReset => self.set_zoom(|s| s.ui_scale = 1.0),
            Message::ClosePanel => {
                self.panel = None;
                self.focus = Focus::Editor;
            }
            // F5 while paused in the debugger continues.
            Message::RunProject if self.debug.as_ref().is_some_and(debugger::Session::paused) => {
                self.on_debug(debugger::DebugMessage::Step("continue"));
            }
            Message::RunProject => self.start_run(None),
            Message::StartDebug { scene } => self.start_debug(scene),
            Message::ToggleBreakpoint => self.toggle_breakpoint_at_cursor(),
            Message::Debugger(generation, event) => return self.on_debugger(generation, event),
            Message::Debug(message) => self.on_debug(message),
            Message::DebugRetry(generation, reference, attempt) => {
                if let Some(session) = self.debug.as_mut().filter(|s| s.generation == generation) {
                    session.request_variables(reference, attempt);
                }
            }
            Message::OpenInGodot => self.open_in_godot(),
            Message::RunScene => return self.run_scene(false),
            Message::ChooseScene => return self.run_scene(true),
            Message::ChooseGodot => return self.choose_godot(),
            Message::GodotChosen(path) => self.set_godot(path),
            Message::RunChosenScene(script, scene) => {
                self.scene_choice.insert(script, scene.clone());
                self.start_run(Some(scene));
            }
            Message::StopRun => {
                if self.debug.is_some() {
                    self.on_debug(debugger::DebugMessage::Stop);
                }
                if self.run.handle.take().is_some() {
                    self.push_output(OutputLine::info("Stopping..."));
                }
            }
            Message::Run(event) => self.on_run(event),
            Message::ClearOutput => self.run.output.clear(),
            Message::FilesChanged(paths) => {
                self.uid_cache.clear();
                self.on_files_changed(&paths);
            }
            Message::ReloadActive => {
                if let Some(index) = self.active_index() {
                    self.reload_tab(index);
                }
            }
            Message::OpenFind { replace } => return self.open_find(replace),
            Message::Find(message) => return self.on_find(message),
            Message::OpenSearch => return self.open_search(),
            Message::Search(message) => return self.on_search(message),
            Message::Key(event) => return self.on_key(event),
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
            Message::KillTerminal => {
                if let Some(id) = self.terminals.active().map(|t| t.id) {
                    self.close_terminal(id);
                }
            }
            Message::NewTerminal => {
                self.terminals.add();
                self.show_panel(Panel::Terminal);
            }
            Message::SelectTerminal(id) => {
                if let Some(index) = self.terminals.tabs.iter().position(|t| t.id == id) {
                    self.terminals.active = index;
                    self.focus = Focus::Terminal;
                }
            }
            Message::CloseTerminal(id) => self.close_terminal(id),
            Message::FocusTerminal => {
                self.focus = Focus::Terminal;
                self.dismiss_popups();
            }
            Message::Terminal(id, event) => match event {
                TerminalEvent::Ready(session) => {
                    if let Some(tab) = self.terminals.get_mut(id) {
                        tab.session = Some(session);
                    }
                }
                // Nothing to update: the view redraws from the live grid.
                TerminalEvent::Wakeup => {}
                TerminalEvent::Title(title) => {
                    if let Some(tab) = self.terminals.get_mut(id) {
                        tab.title = Some(title);
                    }
                }
                TerminalEvent::Exit => self.close_terminal(id),
                TerminalEvent::Failed(error) => {
                    self.status = Some(format!("Cannot start terminal: {error}"));
                    self.close_terminal(id);
                }
            },
        }
        Task::none()
    }

    fn show_panel(&mut self, panel: Panel) {
        self.panel = Some(panel);
        // Showing the terminal starts a shell when there is none.
        if panel == Panel::Terminal && self.terminals.tabs.is_empty() && self.project.is_some() {
            self.terminals.add();
        }
        self.focus = match panel {
            Panel::Terminal => Focus::Terminal,
            Panel::Search => Focus::Input,
            Panel::Problems | Panel::Output | Panel::Changes | Panel::Debug => Focus::Editor,
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
                // The OS may have changed mode while editing.
                self.apply_os_theme();
            }
            ThemeMessage::Import => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import theme")
                            .add_filter("Beckett or VS Code theme", &["toml", "json", "jsonc"])
                            .add_filter("Beckett theme", &["toml"])
                            .add_filter("VS Code theme", &["json", "jsonc"])
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
                            .add_filter("Beckett theme", &["toml"])
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

    /// Lists the themes of the VS Code installed here in the palette.
    fn choose_vscode_theme(&mut self) -> Task<Message> {
        let installed = theme::vscode::installed();
        if installed.is_empty() {
            self.status = Some("No VS Code themes found (VS Code's extensions folder): import a theme .json file instead".to_string());
            return Task::none();
        }
        let all = palette::Choice {
            label: format!("Import all {} themes", installed.len()),
            detail: "keeps the current theme".to_string(),
            message: Message::ImportThemeFiles(installed.iter().map(|found| found.path.clone()).collect()),
        };
        let choices = std::iter::once(all)
            .chain(installed.into_iter().map(|found| palette::Choice {
                detail: found.path.display().to_string(),
                message: Message::ImportThemeFile(found.path),
                label: found.label,
            }))
            .collect();
        let mut palette = Palette::choose("Import which VS Code theme?", choices);
        palette.refresh(&[]);
        self.palette = Some(palette);
        self.focus = Focus::Input;
        self.dismiss_popups();
        iced::widget::operation::focus(palette::INPUT)
    }

    /// Lists the Godot editor settings files (one per Godot version) to
    /// import their text editor colors.
    fn choose_godot_editor_theme(&mut self) -> Task<Message> {
        let files = theme::godot_editor::settings_files();
        if files.is_empty() {
            self.status = Some("No Godot editor settings found (editor_settings-4.x.tres in Godot's config folder)".to_string());
            return Task::none();
        }
        let all = palette::Choice {
            label: format!("Import all {} Godot versions' colors", files.len()),
            detail: "keeps the current theme".to_string(),
            message: Message::ImportThemeFiles(files.clone()),
        };
        let choices = std::iter::once(all)
            .chain(files.into_iter().map(|path| palette::Choice {
                label: match theme::godot_editor::version_of(&path) {
                    Some((major, minor)) => format!("Godot {major}.{minor} editor colors"),
                    None => "Godot editor colors".to_string(),
                },
                detail: path.display().to_string(),
                message: Message::ImportThemeFile(path),
            }))
            .collect();
        let mut palette = Palette::choose("Import the colors of which Godot editor?", choices);
        palette.refresh(&[]);
        self.palette = Some(palette);
        self.focus = Focus::Input;
        self.dismiss_popups();
        iced::widget::operation::focus(palette::INPUT)
    }

    fn import_theme_file(&mut self, path: &Path) {
        match read_theme_file(path) {
            Ok(theme) => self.install_theme(theme, &path.display().to_string()),
            Err(error) => self.status = Some(format!("Cannot import theme: {error}")),
        }
    }

    /// Imports every one of these theme files, keeping the current theme.
    fn import_theme_files(&mut self, paths: &[PathBuf]) {
        let mut imported = 0;
        let mut failed = Vec::new();
        for path in paths {
            match read_theme_file(path).and_then(|theme| self.store_theme(theme)) {
                Ok(_) => imported += 1,
                Err(error) => failed.push(error),
            }
        }
        let plural = if imported == 1 { "" } else { "s" };
        self.status = Some(match failed.first() {
            None => format!("Imported {imported} theme{plural} (pick them from the theme list)"),
            Some(error) => format!("Imported {imported} theme{plural}; {} failed, e.g. {error}", failed.len()),
        });
    }

    /// Adds a theme to the user themes folder (a built-in one's name gets
    /// "(imported)") and to the list; returns its index.
    fn store_theme(&mut self, mut theme: theme::Theme) -> Result<usize, String> {
        if self.themes.iter().zip(&self.theme_paths).any(|(t, p)| p.is_none() && t.name == theme.name) {
            theme.name = format!("{} (imported)", theme.name);
        }
        let path = self.write_user_theme(&theme)?;
        Ok(self.add_theme(theme, path))
    }

    /// Copies a theme into the user themes folder and switches to it.
    fn install_theme(&mut self, theme: theme::Theme, source: &str) {
        match self.store_theme(theme) {
            Ok(index) => {
                let theme = self.themes[index].clone();
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
        palette.set_symbols(self.symbol_choices());
        self.palette = Some(palette);
        self.load_project_symbols();
        let commands = self.commands();
        if let Some(palette) = &mut self.palette {
            palette.refresh(&commands);
        }
        self.focus = Focus::Input;
        self.dismiss_popups();
        Task::batch([
            iced::widget::operation::focus(palette::INPUT),
            iced::widget::operation::move_cursor_to_end(palette::INPUT),
        ])
    }

    /// Reads the symbols of every script once the palette's `#` mode needs them.
    fn load_project_symbols(&mut self) {
        let (Some(project), Some(palette)) = (&self.project, &self.palette) else {
            return;
        };
        if !palette.wants_project_symbols() {
            return;
        }
        let open: Vec<(PathBuf, String)> = self
            .tabs
            .iter()
            .filter(|t| t.path.extension().is_some_and(|e| e == "gd"))
            .map(|t| (t.path.clone(), t.buffer.text()))
            .collect();
        let symbols = project_symbols::choices(&project.root, &palette.scripts(), &open, &self.current_theme().syntax);
        if let Some(palette) = &mut self.palette {
            palette.set_project_symbols(symbols);
        }
    }

    fn on_palette(&mut self, message: PaletteMessage) -> Task<Message> {
        let commands = self.commands();
        let Some(palette) = &mut self.palette else {
            return Task::none();
        };
        let item = match message {
            PaletteMessage::Input(input) => {
                palette.input = input;
                self.load_project_symbols();
                if let Some(palette) = &mut self.palette {
                    palette.refresh(&commands);
                }
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
            ("File: Save", "file.save", Message::Save),
            ("Format Document", "edit.format", Message::Edit(EditorEvent::FormatDocument)),
            ("File: Open Folder", "file.openFolder", Message::OpenFolderDialog),
            ("File: Close Folder", "", Message::CloseFolder),
            ("File: Close Tab", "file.closeTab", Message::CloseActiveTab),
            ("File: Reload from Disk", "", Message::ReloadActive),
            ("Go to File", "file.quickOpen", Message::OpenPalette(String::new())),
            ("Go to Line", "go.line", Message::OpenPalette(":".to_string())),
            ("Go to Symbol in File", "go.symbol", Message::OpenPalette("@".to_string())),
            ("Go to Symbol in Project", "go.projectSymbol", Message::OpenPalette("#".to_string())),
            ("View: Outline", "", Message::ShowSidebar(SidebarView::Outline)),
            ("View: Files", "", Message::ShowSidebar(SidebarView::Files)),
            ("View: Project (Autoloads, Input Map)", "", Message::ShowSidebar(SidebarView::Project)),
            ("File: New Script...", "", Message::OpenNewScript(None)),
            ("Git: Show Changes", "", Message::ShowChanges(None)),
            ("Terminal: New Terminal", "", Message::NewTerminal),
            ("Git: Add Godot .gitignore and .gitattributes", "", Message::AddGodotIgnores),
            ("Git: Show Changes in This File", "", Message::ShowChanges(self.active_tab().map(|t| t.path.clone()))),
            ("Go to Definition", "go.definition", Message::GotoDefinition),
            ("Find All References", "go.references", Message::FindReferences),
            ("Rename Symbol", "refactor.rename", Message::StartRename),
            ("Find", "find.find", Message::OpenFind { replace: false }),
            ("Replace", "find.replace", Message::OpenFind { replace: true }),
            ("Search in Project", "search.project", Message::OpenSearch),
            ("Run: Project", "run.project", Message::RunProject),
            ("Run: Current Scene", "run.scene", Message::RunScene),
            ("Run: Choose Scene for This Script...", "", Message::ChooseScene),
            ("Debug: Start Debugging (Main Scene)", "debug.start", Message::StartDebug { scene: false }),
            ("Debug: Start Debugging (Current Scene)", "debug.startScene", Message::StartDebug { scene: true }),
            ("Debug: Toggle Breakpoint", "debug.toggleBreakpoint", Message::ToggleBreakpoint),
            ("Debug: Show Debug Panel", "", Message::ShowPanel(Panel::Debug)),
            ("Godot: Choose Version...", "", Message::ChooseGodot),
            ("Run: Stop", "run.stop", Message::StopRun),
            ("Godot: Open in Editor", "", Message::OpenInGodot),
            ("View: Toggle Terminal", "view.terminal", Message::ToggleTerminal),
            ("View: Split Editor (Move Tab to the Other Side)", "view.split", Message::SplitMove),
            ("View: Focus the Other Side", "", Message::FocusOtherSide),
            ("View: Zoom In", "view.zoomIn", Message::Zoom(1)),
            ("View: Zoom Out", "view.zoomOut", Message::Zoom(-1)),
            ("View: Reset Zoom", "view.zoomReset", Message::ZoomReset),
            ("View: Problems", "", Message::ShowPanel(Panel::Problems)),
            ("View: Output", "", Message::ShowPanel(Panel::Output)),
            ("View: Search Results", "", Message::ShowPanel(Panel::Search)),
            ("Terminal: Kill", "", Message::KillTerminal),
            ("Output: Clear", "", Message::ClearOutput),
            ("Language Server: Restart", "", Message::RestartLsp),
            ("Edit: Undo", "edit.undo", edit(Command::Undo)),
            ("Edit: Redo", "edit.redo", edit(Command::Redo)),
            ("Edit: Select All", "edit.selectAll", edit(Command::SelectAll)),
            ("Edit: Toggle Comment", "edit.toggleComment", edit(Command::ToggleComment)),
            ("Fold / Unfold", "edit.fold", edit(Command::ToggleFoldAtCursor)),
            ("Fold", "", edit(Command::Fold)),
            ("Unfold", "", edit(Command::Unfold)),
            ("Fold All", "", edit(Command::FoldAll)),
            ("Unfold All", "", edit(Command::UnfoldAll)),
            ("Edit: Duplicate Line", "edit.duplicateLine", edit(Command::DuplicateLines)),
            ("Edit: Delete Line", "edit.deleteLine", edit(Command::DeleteLines)),
            ("Edit: Move Line Up", "edit.moveLineUp", edit(Command::MoveLinesUp)),
            ("Edit: Move Line Down", "edit.moveLineDown", edit(Command::MoveLinesDown)),
            ("Edit: Indent", "Tab", edit(Command::Indent)),
            ("Edit: Outdent", "Shift+Tab", edit(Command::Outdent)),
            ("Edit: Select Next Occurrence", "edit.selectNextOccurrence", edit(Command::SelectNextOccurrence)),
            ("Edit: Add Cursor Above", "edit.cursorAbove", edit(Command::AddCursorAbove)),
            ("Edit: Add Cursor Below", "edit.cursorBelow", edit(Command::AddCursorBelow)),
            ("Edit: Trigger Completion", "edit.completion", Message::Edit(EditorEvent::TriggerCompletion)),
            ("Help: Keyboard Shortcuts", "view.shortcuts", Message::OpenShortcuts),
            ("Help: Class Reference", "help.classReference", Message::OpenClassRef(None, None)),
            ("Theme: Edit Colors", "", Message::OpenThemeEditor),
            (
                if self.settings.follow_os_theme { "Theme: Stop Following the OS Light/Dark Mode" } else { "Theme: Follow the OS Light/Dark Mode" },
                "",
                Message::ToggleFollowOsTheme,
            ),
            ("Theme: Import File...", "", Message::ThemeEditor(ThemeMessage::Import)),
            ("Theme: Import from VS Code...", "", Message::ChooseVsCodeTheme),
            ("Theme: Import from the Godot Editor...", "", Message::ChooseGodotEditorTheme),
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
            .map(|(name, shortcut, message)| {
                // A command id shows its current key; anything else is literal.
                let shortcut = if shortcut.contains('.') { self.keymap.label(shortcut) } else { shortcut.to_string() };
                CommandEntry { name: name.to_string(), shortcut, message }
            })
            .collect();
        entries.extend(themes.into_iter().map(|(name, message)| CommandEntry { name, shortcut: String::new(), message }));
        entries.extend(keymap::Preset::ALL.into_iter().map(|preset| CommandEntry {
            name: format!("Keyboard Shortcuts: Use {} Preset", preset.name()),
            shortcut: String::new(),
            message: Message::Shortcuts(ShortcutsMessage::Preset(preset)),
        }));
        let current = self.project.as_ref().map(|p| p.root.as_path());
        entries.extend(
            self.recent
                .iter()
                .filter(|r| !r.missing && current.is_none_or(|c| !uri::same_path(c, &r.root)))
                .map(|r| CommandEntry {
                    name: format!("Open Recent: {} ({})", r.name, r.root.display()),
                    shortcut: String::new(),
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
        // Any change to what would be replaced cancels a pending confirmation.
        if !matches!(message, SearchMessage::ConfirmReplace | SearchMessage::Done(..) | SearchMessage::Open(..)) {
            self.search.confirm = None;
        }
        let query = &mut self.search.query;
        match message {
            SearchMessage::Replace(text) => self.search.replace = text,
            SearchMessage::CancelReplace => {}
            SearchMessage::ReplaceAll => match self.project_replacements() {
                Ok(files) => {
                    let count = files.iter().map(|f| f.edits.len()).sum();
                    self.search.confirm = Some((count, files.len()));
                }
                Err(error) => self.search.error = Some(error),
            },
            SearchMessage::ConfirmReplace => {
                self.search.confirm = None;
                match self.project_replacements() {
                    Ok(files) => self.apply_project_replace(files),
                    Err(error) => self.search.error = Some(error),
                }
                // Show what the project looks like now.
                return self.on_search(SearchMessage::Run);
            }
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

    fn project_replacements(&self) -> Result<Vec<find::FileEdits>, String> {
        let Some(project) = &self.project else {
            return Ok(Vec::new());
        };
        let open: Vec<(&Path, &Buffer)> = self.tabs.iter().map(|t| (t.path.as_path(), &t.buffer)).collect();
        find::project_replacements(&project.root, &self.search.query, &self.search.replace, &open)
    }

    /// Replace across the project: open tabs are edited in place (one undo
    /// step each, left unsaved), other files are rewritten on disk.
    fn apply_project_replace(&mut self, files: Vec<find::FileEdits>) {
        let (mut count, mut file_count, mut errors) = (0, 0, Vec::new());
        for file in files {
            let n = file.edits.len();
            match self.tabs.iter().position(|t| uri::same_path(&t.path, &file.path)) {
                Some(index) => self.edit_tab(index, Command::Replace(file.edits)),
                None => {
                    let result = std::fs::read_to_string(&file.path).and_then(|text| {
                        let mut buffer = Buffer::new(&text, Language::from_path(&file.path));
                        buffer.apply(Command::Replace(file.edits));
                        std::fs::write(&file.path, buffer.file_contents())
                    });
                    if let Err(error) = result {
                        errors.push(format!("{}: {error}", self.display_path(&file.path)));
                        continue;
                    }
                }
            }
            count += n;
            file_count += 1;
        }
        self.refresh_marks();
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        self.status = Some(match errors.first() {
            Some(error) => format!("Replace incomplete, cannot write {error}"),
            None => format!("Replaced {} in {}", plural(count, "occurrence"), plural(file_count, "file")),
        });
    }

    /// The bottom panel's height, kept within what the window allows.
    fn panel_height(&self) -> f32 {
        let max = (self.window_height - MIN_EDITOR_HEIGHT).max(MIN_PANEL_HEIGHT);
        self.panel_height.clamp(MIN_PANEL_HEIGHT, max)
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
        let (path, scenes) = self.scenes_for_active()?;
        match self.scene_choice.get(&path).filter(|s| scenes.contains(s)) {
            Some(chosen) => Ok(chosen.clone()),
            None => Ok(scenes[0].clone()),
        }
    }

    /// The active tab's path and the `res://` scenes it can run as: itself
    /// for a scene, the scenes it is attached to for a script (never empty).
    fn scenes_for_active(&self) -> Result<(PathBuf, Vec<String>), String> {
        let godot = self
            .project
            .as_ref()
            .and_then(|p| p.godot.as_ref())
            .ok_or("Not a Godot project")?;
        let tab = self.active_tab().ok_or("No file open")?;
        let res_path = godot.to_res_path(&tab.path).ok_or("File is outside the project")?;
        if res_path.ends_with(".tscn") {
            return Ok((tab.path.clone(), vec![res_path]));
        }
        let scenes: Vec<String> =
            godot.scenes_using(&res_path).iter().filter_map(|scene| godot.to_res_path(scene)).collect();
        if scenes.is_empty() {
            return Err(format!("No scene uses {res_path}"));
        }
        Ok((tab.path.clone(), scenes))
    }

    /// F6: runs the current scene, asking which one first when several
    /// scenes use the script and none was picked yet (or `ask` is set).
    fn run_scene(&mut self, ask: bool) -> Task<Message> {
        let (path, scenes) = match self.scenes_for_active() {
            Ok(found) => found,
            Err(reason) => {
                self.status = Some(reason);
                return Task::none();
            }
        };
        let chosen = self.scene_choice.get(&path).filter(|s| scenes.contains(s)).cloned();
        if scenes.len() == 1 || (chosen.is_some() && !ask) {
            let scene = chosen.unwrap_or_else(|| scenes[0].clone());
            self.start_run(Some(scene));
            return Task::none();
        }
        let choices = scenes
            .into_iter()
            .map(|scene| palette::Choice {
                detail: if chosen.as_ref() == Some(&scene) { "last run".to_string() } else { String::new() },
                message: Message::RunChosenScene(path.clone(), scene.clone()),
                label: scene,
            })
            .collect();
        let mut palette = Palette::choose("Several scenes use this script: run which one?", choices);
        palette.refresh(&[]);
        self.palette = Some(palette);
        self.focus = Focus::Input;
        self.dismiss_popups();
        iced::widget::operation::focus(palette::INPUT)
    }

    /// Starts the Godot editor on the project, opening the current scene
    /// (the scene itself, or the one the current script is attached to).
    fn open_in_godot(&mut self) {
        let Some(project) = &self.project else {
            return;
        };
        let Some(godot) = &project.godot else {
            self.status = Some("Not a Godot project (no project.godot)".to_string());
            return;
        };
        let attached = self.lsp.client().is_some_and(|c| matches!(c.server(), lsp_client::Server::Attached { .. }));
        if attached {
            self.status = Some("The Godot editor is already open on this project".to_string());
            return;
        }
        let version = godot.godot_version();
        let root = project.root.clone();
        let Some(install) = lsp_client::godot::resolve(self.godot_choice.as_deref(), version.as_deref()) else {
            self.status = Some("No Godot executable found (set GODOT_BIN)".to_string());
            return;
        };
        if lsp_client::godot::older_than(&install, version.as_deref()) {
            self.status = Some(format!(
                "This project is for Godot {}, the newest installed is {}: opening it there would convert it",
                version.unwrap_or_default(),
                install.version_string().unwrap_or_default()
            ));
            return;
        }
        let scene = self.current_scene().ok();
        let mut command = std::process::Command::new(&install.path);
        command.arg("--editor").arg("--path").arg(&root);
        if let Some(scene) = &scene {
            command.arg(scene);
        }

        // Only one Godot may have the project open (they would fight over
        // GDExtension libraries and the import cache): stop the IDE's
        // headless server first, then use the editor's language server.
        self.reset_lsp(Lsp::Starting);
        self.lsp_wait_editor = true;
        self.status = Some(match command.spawn() {
            Ok(_) => match scene {
                Some(scene) => format!("Opening {scene} in the Godot editor"),
                None => "Opening the project in the Godot editor".to_string(),
            },
            Err(error) => {
                self.reset_lsp(Lsp::Starting);
                format!("Cannot start Godot: {error}")
            }
        });
    }

    /// Runs the game under the debugger (the main scene, or the current
    /// one), through the debug adapter of the Godot serving the language
    /// server: breakpoints stop it, so do runtime errors.
    fn start_debug(&mut self, current_scene: bool) {
        if self.debug.is_some() {
            self.status = Some("Already debugging (Shift+F5 stops)".to_string());
            return;
        }
        if self.project.as_ref().is_none_or(|p| p.godot.is_none()) {
            self.status = Some("Not a Godot project (no project.godot)".to_string());
            return;
        }
        let Some(client) = self.lsp.client() else {
            self.status = Some("The debugger runs in the Godot that serves the language server: wait until it is connected".to_string());
            return;
        };
        let port = client.server().dap_port();
        let scene = if current_scene {
            match self.current_scene() {
                Ok(scene) => Some(scene),
                Err(reason) => {
                    self.status = Some(reason);
                    return;
                }
            }
        } else {
            None
        };
        // One game at a time.
        self.run.handle = None;
        self.save_all();
        self.run.output.clear();
        self.run.problems = runner::RuntimeParser::default();
        self.problems.clear_runtime();
        self.refresh_marks();
        let what = scene.clone().unwrap_or_else(|| "the main scene".to_string());
        self.push_output(OutputLine::info(format!("> Debugging {what}")));
        self.debug_generation += 1;
        self.debug = Some(debugger::Session::new(self.debug_generation, port, scene, self.watches.clone()));
        self.show_panel(Panel::Debug);
    }

    fn on_debugger(&mut self, generation: u64, event: debugger::DebugEvent) -> Task<Message> {
        let root = self.project.as_ref().map(|p| p.root.clone()).unwrap_or_default();
        let Some(session) = self.debug.as_mut().filter(|s| s.generation == generation) else {
            return Task::none();
        };
        let effects = match event {
            debugger::DebugEvent::Connected(client) => {
                session.on_connected(client, &root);
                Vec::new()
            }
            debugger::DebugEvent::Message(message) => session.on_message(message),
            debugger::DebugEvent::Failed(error) => vec![debugger::Effect::Ended(Some(error))],
            debugger::DebugEvent::Closed => vec![debugger::Effect::Ended(None)],
        };
        self.apply_debug_effects(effects)
    }

    fn on_debug(&mut self, message: debugger::DebugMessage) {
        let Some(session) = &mut self.debug else {
            return;
        };
        let effects = match message {
            debugger::DebugMessage::SelectFrame(index) => session.select_frame(index),
            debugger::DebugMessage::Toggle(path, reference) => {
                session.toggle(path, reference);
                Vec::new()
            }
            debugger::DebugMessage::Step(command) => {
                session.step(command);
                Vec::new()
            }
            debugger::DebugMessage::WatchInput(text) => {
                session.watch_input = text;
                Vec::new()
            }
            debugger::DebugMessage::AddWatch => {
                let expression = std::mem::take(&mut session.watch_input).trim().to_string();
                if !expression.is_empty() && !session.watches.contains(&expression) {
                    session.watches.push(expression.clone());
                    session.evaluate_watch(expression);
                    self.watches = session.watches.clone();
                }
                Vec::new()
            }
            debugger::DebugMessage::RemoveWatch(index) => {
                if index < session.watches.len() {
                    session.watches.remove(index);
                    self.watches = session.watches.clone();
                }
                Vec::new()
            }
            debugger::DebugMessage::Stop if session.client.is_none() => vec![debugger::Effect::Ended(None)],
            debugger::DebugMessage::Stop => {
                session.stop();
                Vec::new()
            }
        };
        // Frames and toggles only jump: nothing to wait for.
        let _ = self.apply_debug_effects(effects);
    }

    fn apply_debug_effects(&mut self, effects: Vec<debugger::Effect>) -> Task<Message> {
        let mut tasks = Vec::new();
        for effect in effects {
            match effect {
                debugger::Effect::Configure => {
                    if let Some(client) = self.debug.as_ref().and_then(|s| s.client.clone()) {
                        debugger::send_breakpoints(&client, &self.all_breakpoints());
                        let _ = client.configuration_done();
                    }
                }
                debugger::Effect::Jump(path, line) => {
                    self.dismiss_popups();
                    self.focus = Focus::Editor;
                    self.jump_to(path, line as u32, 0);
                }
                debugger::Effect::Output(text, error) => {
                    for line in text.lines() {
                        let previous = self.run.output.last().map_or(runner::LineKind::Normal, |l| l.kind);
                        let mut line = runner::classify(line.to_string(), previous);
                        if error && line.kind == runner::LineKind::Normal {
                            line.kind = runner::LineKind::Error;
                        }
                        self.on_run(RunEvent::Line(line));
                    }
                }
                debugger::Effect::Retry(reference, attempt) => {
                    let generation = self.debug_generation;
                    tasks.push(Task::perform(sleep(std::time::Duration::from_millis(100)), move |()| {
                        Message::DebugRetry(generation, reference, attempt)
                    }));
                }
                debugger::Effect::HoverValue(pos, expression, value) => {
                    let text = format!("`{expression}` = `{}`", value.replace('`', "'"));
                    self.hover = Some((pos, Doc::from_markdown(&text)));
                }
                debugger::Effect::Ended(reason) => {
                    self.debug = None;
                    self.push_output(OutputLine::info("Debugging ended"));
                    self.status = Some(reason.unwrap_or_else(|| "Debugging ended".to_string()));
                }
            }
        }
        Task::batch(tasks)
    }

    /// A tab's breakpoints; `None` for files that can't have any.
    fn breakpoints_of(&self, tab: &Tab) -> Option<Vec<usize>> {
        (tab.language == Language::GDScript).then(|| tab.buffer.line_marks())
    }

    /// Every breakpoint: open files' from their buffers (where they follow
    /// edits), the others' as last known. Files without any are kept, so
    /// Godot is told to forget their last one.
    fn all_breakpoints(&self) -> std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<usize>> {
        let mut all = self.breakpoints.clone();
        for tab in self.tabs.iter().filter(|t| t.language == Language::GDScript) {
            let marks: std::collections::BTreeSet<usize> = tab.buffer.line_marks().into_iter().collect();
            if !marks.is_empty() || all.contains_key(&tab.path) {
                all.insert(tab.path.clone(), marks);
            }
        }
        all
    }

    /// Sends one file's breakpoints to a running debug session.
    fn send_file_breakpoints(&self, path: &Path) {
        let Some(client) = self.debug.as_ref().and_then(|s| s.client.as_ref()) else {
            return;
        };
        let lines = match self.tabs.iter().find(|t| t.path == path) {
            Some(tab) => tab.buffer.line_marks(),
            None => self.breakpoints.get(path).map(|l| l.iter().copied().collect()).unwrap_or_default(),
        };
        let one_based: Vec<usize> = lines.iter().map(|l| l + 1).collect();
        let _ = client.set_breakpoints(path, &one_based);
    }

    /// The (zero-based) line of `path` the debugger is paused on.
    fn paused_line_in(&self, path: &Path) -> Option<usize> {
        let session = self.debug.as_ref().filter(|s| s.paused())?;
        let (file, line) = session.location()?;
        uri::same_path(file, path).then(|| line.saturating_sub(1))
    }

    /// F9: a breakpoint on the cursor's line of the active GDScript tab.
    fn toggle_breakpoint_at_cursor(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let line = tab.buffer.char_to_pos(tab.buffer.primary().head).line;
        let path = tab.path.clone();
        self.toggle_breakpoint(path, line);
    }

    fn toggle_breakpoint(&mut self, path: PathBuf, line: usize) {
        if Language::from_path(&path) != Language::GDScript {
            self.status = Some("Breakpoints go in GDScript files".to_string());
            return;
        }
        let tab = self.tabs.iter_mut().find(|t| t.path == path);
        let mut lines: std::collections::BTreeSet<usize> = match &tab {
            Some(tab) => tab.buffer.line_marks().into_iter().collect(),
            None => self.breakpoints.get(&path).cloned().unwrap_or_default(),
        };
        if !lines.remove(&line) {
            lines.insert(line);
        }
        if let Some(tab) = tab {
            tab.buffer.set_line_marks(lines.iter().copied());
        }
        self.breakpoints.insert(path.clone(), lines);
        // A running session gets the file's new list at once (an empty one
        // too, so Godot forgets the last).
        self.send_file_breakpoints(&path);
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
            let wanted = godot.godot_version();
            self.run.godot = lsp_client::godot::resolve(self.godot_choice.as_deref(), wanted.as_deref())
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
        self.run.problems = runner::RuntimeParser::default();
        self.problems.clear_runtime();
        self.refresh_marks();
        self.push_output(OutputLine::info(format!("> {}", request.command_line())));
        self.run.request = Some(request);
        self.show_panel(Panel::Output);
    }

    fn on_run(&mut self, event: RunEvent) {
        match event {
            RunEvent::Started(handle) => self.run.handle = Some(handle),
            RunEvent::Line(line) => {
                if let Some(problem) = self.run.problems.push(&line) {
                    self.add_runtime_problem(problem);
                }
                self.push_output(line);
            }
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

    fn add_runtime_problem(&mut self, problem: runner::RuntimeProblem) {
        let path = self.project.as_ref().and_then(|p| p.godot.as_ref()).and_then(|g| g.resolve(&problem.res_path));
        let Some(path) = path else {
            return;
        };
        self.problems.add_runtime(path, problem.line.saturating_sub(1), problem.message, problem.error);
        self.refresh_marks();
    }

    fn on_files_changed(&mut self, paths: &[PathBuf]) {
        // Our own saves come back here too: the markers and statuses follow.
        self.git_refresh.wanted = true;
        // Files appeared or vanished: refresh their folders in the explorer.
        if let Some(project) = &mut self.project {
            let dirs: std::collections::BTreeSet<&Path> = paths.iter().filter_map(|p| p.parent()).collect();
            for dir in dirs {
                project.explorer.refresh(dir);
            }
            // Godot rewrites project.godot when autoloads or actions change.
            let changed = project.godot.as_ref().is_some_and(|g| paths.iter().any(|p| project_panel::is_project_file(g, p)));
            if changed {
                // A half-written file keeps the previous settings.
                if let Ok(godot) = GodotProject::load(&project.root) {
                    project.info = ProjectInfo::read(Some(&godot));
                    project.godot = Some(godot);
                }
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
        let marks = tab.buffer.line_marks();
        tab.buffer = Buffer::new(&text, tab.language);
        tab.buffer.set_line_marks(marks);
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

    /// Ends one shell; when none is left, hides the panel (the next toggle
    /// starts a fresh one).
    fn close_terminal(&mut self, id: u64) {
        self.terminals.remove(id);
        if self.terminals.tabs.is_empty() {
            self.reset_terminal();
        }
    }

    /// Ends every shell and hides their panel; the next toggle starts a
    /// fresh one.
    fn reset_terminal(&mut self) {
        self.terminals.tabs.clear();
        self.terminals.active = 0;
        if self.panel == Some(Panel::Terminal) {
            self.panel = None;
        }
        self.focus = Focus::Editor;
    }

    fn on_editor(&mut self, event: EditorEvent) -> Task<Message> {
        // Keys meant for a file do nothing while the class reference is in front.
        if self.class_ref_active() {
            return Task::none();
        }
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
            EditorEvent::GotoDefinition => self.goto_definition(),
            EditorEvent::FormatDocument => self.format_active(),
            EditorEvent::ContextMenu(position) => {
                self.dismiss_popups();
                let entries = self.context_entries(&ContextTarget::Editor);
                self.context_menu = (!entries.is_empty()).then_some(context_menu::Menu { position, entries });
            }
            EditorEvent::LinkHover(index) => self.on_link_hover(index),
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
            EditorEvent::DropFile { path, pos, ctrl } => self.drop_file(&path, pos, ctrl),
            EditorEvent::ToggleBreakpoint(line) => {
                if let Some(path) = self.active_tab().map(|t| t.path.clone()) {
                    self.toggle_breakpoint(path, line);
                }
            }
            EditorEvent::GitMarker => {
                let path = self.active_tab().map(|t| t.path.clone());
                return self.update(Message::ShowChanges(path));
            }
            EditorEvent::Hover(pos) => self.request_hover(pos),
            EditorEvent::HoverEnd => self.hover = None,
            EditorEvent::Focused => {
                self.focus = Focus::Editor;
                self.palette = None;
            }
            EditorEvent::Shortcut(id) => return self.run_command(id),
        }
        Task::none()
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
        // Char ranges no longer match once the text changes.
        self.link = None;
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
        // Inside a `res://` or action-name string, every typed char narrows
        // the list.
        if let Some((anchor, kind)) = self.string_completion_anchor() {
            match &self.completion {
                Some(completion) if completion.in_string == Some(kind) && completion.anchor == anchor => {
                    self.refilter_completion()
                }
                _ => self.open_string_completion(anchor, kind),
            }
            return;
        }
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
        self.link = None;
        self.link_probe = None;
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
        if let Some((anchor, kind)) = self.string_completion_anchor() {
            return self.open_string_completion(anchor, kind);
        }
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

    /// Where completion inside a string starts for the cursor of the active
    /// GDScript tab, and of what: a `res://` path, or an action name passed
    /// to `Input.is_action_pressed(` and the like.
    fn string_completion_anchor(&self) -> Option<(usize, InString)> {
        let tab = self.active_tab().filter(|t| t.language == Language::GDScript)?;
        if tab.buffer.selections().len() != 1 {
            return None;
        }
        let head = tab.buffer.primary().head;
        paths::path_completion_anchor(&tab.buffer, head)
            .map(|anchor| (anchor, InString::Paths))
            .or_else(|| project_panel::action_completion_anchor(&tab.buffer, head).map(|anchor| (anchor, InString::Actions)))
    }

    fn open_string_completion(&mut self, anchor: usize, kind: InString) {
        let Some(project) = &self.project else {
            return;
        };
        self.completion = Some(match kind {
            InString::Paths => {
                let Some(godot) = &project.godot else {
                    return;
                };
                let files = godot.resource_files().iter().filter_map(|f| godot.to_res_path(f)).collect();
                Completion::paths(anchor, files)
            }
            InString::Actions => Completion::actions(anchor, project.info.action_names()),
        });
        self.refilter_completion();
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
        let valid = if c.in_string.is_some() {
            !prefix.contains(['"', '\''])
        } else {
            prefix.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
        };
        if head < c.anchor || !same_line || !valid {
            self.completion = None;
            return;
        }
        if c.request.is_some() {
            return; // Narrowed when the answer arrives.
        }
        if c.in_string == Some(InString::Paths) {
            let labels: Vec<String> = c.items.iter().map(|i| i.label.clone()).collect();
            c.filtered = paths::rank_paths(&labels, &prefix);
        } else {
            // Snippets only on a prefix match: fuzzy matches would pop them
            // up for almost any word.
            let lower = prefix.to_lowercase();
            let filtered = completion::rank(&c.items, &prefix)
                .into_iter()
                .filter(|&i| !c.is_snippet(i) || c.items[i].label.to_lowercase().starts_with(&lower))
                .collect();
            c.filtered = filtered;
        }
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
        // No docs for words in comments and strings.
        if paths::in_prose(&tab.buffer, tab.buffer.pos_to_char(pos)) {
            return;
        }
        // Paused in the debugger: the value of what is hovered instead.
        if self.debug.as_ref().is_some_and(debugger::Session::paused) {
            // Keywords and literals have no value to show (`self` does).
            let index = tab.buffer.pos_to_char(pos);
            let (start, end) = tab.buffer.word_range(index);
            let word = tab.buffer.slice(start, end);
            let kind = paths::token_at(&tab.buffer, index);
            if word != "self" && matches!(kind, Some(TokenKind::Keyword | TokenKind::ControlFlow | TokenKind::Annotation | TokenKind::Number)) {
                return;
            }
            let line = tab.buffer.line_text(pos.line);
            let expression = debugger::hover_expression(line.trim_end(), pos.column);
            if let (Some(expression), Some(session)) = (expression, self.debug.as_mut()) {
                session.hover(pos, expression);
            }
            return;
        }
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
            Pending::Outline(path, version) => {
                if self.outline_request == Some(id) {
                    self.outline_request = None;
                }
                let entries = outline::flatten(&lsp_client::document_symbols(&value));
                self.outline = Some(outline::Outline { path, version, entries });
                let symbols = self.symbol_choices();
                if let Some(palette) = &mut self.palette {
                    palette.set_symbols(symbols);
                    palette.refresh(&[]);
                }
            }
            Pending::LinkProbe(range) => {
                if self.link_probe == Some(id) {
                    self.link_probe = None;
                    if !lsp_client::locations(&value).is_empty() {
                        self.link = Some(range);
                    } else {
                        self.probe_native_link(range);
                    }
                }
            }
            Pending::NativeLinkProbe(range) => {
                if self.link_probe == Some(id) {
                    self.link_probe = None;
                    let hover = lsp_client::hover_text(&value);
                    if hover.is_some_and(|h| class_ref::hover_target(&h, &self.native_classes).is_some()) {
                        self.link = Some(range);
                    }
                }
            }
            Pending::References(name) =>self.show_references(name, lsp_client::locations(&value)),
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
                // Engine symbols have no file: their docs, found from the hover.
                None => self.request_native_definition(),
            },
            Pending::NativeDefinition => {
                let target = lsp_client::hover_text(&value).and_then(|hover| class_ref::hover_target(&hover, &self.native_classes));
                match target {
                    Some((class, member)) => return self.open_class_ref(Some(class), member),
                    None => self.status = Some("No definition found".to_string()),
                }
            }
            Pending::ClassDoc(class) => {
                let page = self.class_ref.as_mut().filter(|p| p.loading.as_deref() == Some(class.as_str()));
                if let (Some(page), Some(symbol)) = (page, lsp_client::native_symbol(&value)) {
                    page.loading = None;
                    page.page = Some(class_ref::Page::new(symbol));
                    return iced::widget::operation::snap_to(class_ref::PAGE, iced::widget::scrollable::RelativeOffset::START);
                }
            }
            Pending::Completion => {
                if let Some(c) = &mut self.completion {
                    if c.request == Some(id) {
                        let mut items = lsp_client::completion_items(&value);
                        // Godot's path items replace the whole string, quotes
                        // included (a range we don't apply); paths come from
                        // our own `res://` completion instead.
                        items.retain(|i| i.kind != CompletionKind::File);
                        c.set_items(items);
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
                    (Err(error), Pending::ClassDoc(class)) => {
                        if let Some(page) = self.class_ref.as_mut().filter(|p| p.loading.as_deref() == Some(class.as_str())) {
                            page.loading = None;
                        }
                        self.status = Some(format!("No documentation for {class}: {error}"));
                    }
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
            LspEvent::Event(Event::NativeClasses(mut classes)) => {
                classes.sort_by(|a, b| a.name.cmp(&b.name));
                self.native_classes = classes;
            }
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

    /// Lists the installed Godots in the palette to pick the project's.
    fn choose_godot(&mut self) -> Task<Message> {
        let Some(godot) = self.project.as_ref().and_then(|p| p.godot.as_ref()) else {
            self.status = Some("Not a Godot project (no project.godot)".to_string());
            return Task::none();
        };
        let wanted = godot.godot_version();
        let mut installs = lsp_client::godot::discover();
        // A chosen file outside the usual folders is listed too.
        if let Some(chosen) = self.godot_choice.as_ref().filter(|p| p.is_file()) {
            if !installs.iter().any(|i| uri::same_path(&i.path, chosen)) {
                installs.push(lsp_client::godot::GodotInstall::from_path(chosen.clone()));
            }
        }
        installs.sort_by(|a, b| b.version.cmp(&a.version));
        let best = lsp_client::godot::pick(&installs, wanted.as_deref()).map(|i| i.path.clone());
        let project = wanted.as_deref().unwrap_or("?");
        let automatic = palette::Choice {
            label: "Automatic".to_string(),
            detail: match &best {
                Some(path) => format!("newest install matching the project ({project}): {}", path.display()),
                None => "no Godot found (set GODOT_BIN)".to_string(),
            },
            message: Message::GodotChosen(None),
        };
        let choices = std::iter::once(automatic)
            .chain(installs.into_iter().map(|install| {
                let fit = if lsp_client::godot::older_than(&install, wanted.as_deref()) {
                    "older than the project: opening it there would convert it"
                } else if lsp_client::godot::matches(&install, wanted.as_deref()) {
                    "matches the project"
                } else {
                    "newer than the project"
                };
                let chosen = self.godot_choice.as_ref().is_some_and(|p| uri::same_path(p, &install.path));
                palette::Choice {
                    label: format!(
                        "Godot {}{}",
                        install.version_string().unwrap_or_else(|| "(unknown version)".to_string()),
                        if chosen { "  (in use)" } else { "" }
                    ),
                    detail: format!("{fit} - {}", install.path.display()),
                    message: Message::GodotChosen(Some(install.path)),
                }
            }))
            .collect();
        let mut palette = Palette::choose("Which Godot does this project use?", choices);
        palette.refresh(&[]);
        self.palette = Some(palette);
        self.focus = Focus::Input;
        self.dismiss_popups();
        iced::widget::operation::focus(palette::INPUT)
    }

    /// Uses `path` (or the best match) for the language server, Run, Open
    /// in Godot and new terminals.
    fn set_godot(&mut self, path: Option<PathBuf>) {
        if self.godot_choice == path {
            return;
        }
        self.godot_choice = path;
        self.run.godot = None;
        self.save_session();
        self.status = Some(match &self.godot_choice {
            Some(path) => format!("Using {} (new terminals too)", path.display()),
            None => "Using the newest Godot matching the project".to_string(),
        });
        // The connection is keyed by the choice: restart it with the new one.
        self.lsp_retries = 0;
        self.reset_lsp(Lsp::Starting);
    }

    /// Drops the current connection (stopping a spawned Godot) and lets the
    /// subscription start over.
    fn reset_lsp(&mut self, state: Lsp) {
        self.lsp = state;
        self.lsp_generation += 1;
        self.lsp_wait_editor = false;
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
                .into_iter()
                // Runtime lines may be stale after edits.
                .filter(|d| (d.range.start.line as usize) < tab.buffer.len_lines())
                .map(|d| {
                    let color = match d.severity {
                        Severity::Error => ui.danger,
                        Severity::Warning => ui.warning,
                        Severity::Information | Severity::Hint => ui.text_muted,
                    };
                    let (start, end) = (d.range.start, d.range.end);
                    let line = start.line as usize;
                    let end_column = if end.line == start.line { end.character } else { u32::MAX };
                    // A whole-line (runtime) problem is underlined from the indentation on.
                    let start_column = if end.character == problems::WHOLE_LINE {
                        tab.buffer.line_text(line).chars().take_while(|c| matches!(c, ' ' | '\t')).count()
                    } else {
                        start.character as usize
                    };
                    Mark {
                        line,
                        start: start_column,
                        end: (end_column as usize).min(tab.buffer.line_len(line)),
                        color: style::to_color(color),
                        message: Some(d.message.split_whitespace().collect::<Vec<_>>().join(" ")),
                    }
                })
                .collect();
        }
    }

    /// Opens a Ctrl+clicked terminal link: a URL in the browser; a file at
    /// its line (relative paths from the project folder, where the shell
    /// starts); a folder in the file manager.
    fn open_terminal_link(&mut self, link: terminal_links::TermLink) {
        let (path, line, column) = match link {
            terminal_links::TermLink::Url(url) => {
                if let Err(error) = open_external(Path::new(&url)) {
                    self.status = Some(format!("Cannot open {url}: {error}"));
                }
                return;
            }
            terminal_links::TermLink::Path { path, line, column } => (path, line, column),
        };
        let Some(project) = &self.project else {
            return;
        };
        let target = if path.starts_with("res://") {
            project.godot.as_ref().and_then(|g| g.resolve(&path))
        } else {
            let given = PathBuf::from(&path);
            Some(if given.is_absolute() { given } else { project.root.join(given) })
        };
        match target {
            Some(target) if target.is_file() => {
                self.dismiss_popups();
                self.focus = Focus::Editor;
                let line = line.map_or(0, |l| l.saturating_sub(1));
                self.jump_to(target, line, column.map_or(0, |c| c.saturating_sub(1)));
            }
            Some(target) if target.is_dir() => {
                if let Err(error) = reveal_in_file_manager(&target) {
                    self.status = Some(format!("Cannot show {path}: {error}"));
                }
            }
            _ => self.status = Some(format!("{path} not found (relative paths are read from the project folder)")),
        }
    }

    /// Opens the New Script bar for `path`'s folder (or the active file's,
    /// or the project root), with the file name selected.
    fn open_new_script(&mut self, path: Option<PathBuf>) -> Task<Message> {
        let Some(godot) = self.project.as_ref().and_then(|p| p.godot.as_ref()) else {
            self.status = Some("New scripts need a Godot project (project.godot)".to_string());
            return Task::none();
        };
        let folder = path
            .or_else(|| self.active_tab().map(|t| t.path.clone()))
            .map(|p| if p.is_dir() { p } else { p.parent().map(Path::to_path_buf).unwrap_or(p) })
            .and_then(|dir| godot.to_res_path(&dir))
            .unwrap_or_else(|| "res://".to_string());
        let script = NewScript::new(godot, &folder);
        let end = script.path.chars().count() - ".gd".len();
        let start = end - new_script::DEFAULT_NAME.len();
        self.new_script = Some((script, None));
        self.file_op = None;
        self.focus = Focus::Input;
        Task::batch([
            iced::widget::operation::focus(new_script::PATH_INPUT),
            iced::widget::operation::select_range(new_script::PATH_INPUT, start, end),
        ])
    }

    fn on_new_script(&mut self, message: NewScriptMessage) {
        let Some((script, error)) = &mut self.new_script else {
            return;
        };
        match message {
            NewScriptMessage::Close => {
                self.new_script = None;
                self.focus = Focus::Editor;
            }
            NewScriptMessage::Submit => {
                let Some(project) = &mut self.project else {
                    return;
                };
                let Some(godot) = &project.godot else {
                    return;
                };
                let (target, text) = match script.validate(godot) {
                    Ok(created) => created,
                    Err(message) => {
                        *error = Some(message);
                        return;
                    }
                };
                let written = target
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&target, text));
                if let Err(e) = written {
                    *error = Some(format!("Cannot write the file: {e}"));
                    return;
                }
                self.status = Some(format!("Created {}", script.path.trim()));
                if let Some(dir) = target.parent() {
                    project.explorer.refresh(dir);
                }
                self.new_script = None;
                self.focus = Focus::Editor;
                self.open_file(target);
            }
            other => {
                script.update(other);
                *error = None;
            }
        }
    }

    /// Opens the new folder / rename / delete bar, with the name selected.
    fn open_file_op(&mut self, op: FileOp) -> Task<Message> {
        let Some(project) = &self.project else {
            return Task::none();
        };
        let target = match &op {
            FileOp::NewFolder(path) | FileOp::Rename(path) | FileOp::Delete(path) => path,
        };
        let label = project
            .godot
            .as_ref()
            .and_then(|g| g.to_res_path(target))
            .unwrap_or_else(|| target.display().to_string());
        let bar = FileOpBar::new(op, label);
        let (start, end) = bar.initial_selection();
        let typed = !matches!(bar.op, FileOp::Delete(_));
        self.file_op = Some(bar);
        self.new_script = None;
        if !typed {
            return Task::none();
        }
        self.focus = Focus::Input;
        Task::batch([
            iced::widget::operation::focus(file_ops::INPUT),
            iced::widget::operation::select_range(file_ops::INPUT, start, end),
        ])
    }

    fn on_file_op(&mut self, message: FileOpMessage) -> Task<Message> {
        let Some(bar) = &mut self.file_op else {
            return Task::none();
        };
        match message {
            FileOpMessage::Input(name) => {
                bar.name = name;
                bar.error = None;
            }
            FileOpMessage::Close => {
                self.file_op = None;
                self.focus = Focus::Editor;
            }
            FileOpMessage::Submit => {
                let op = bar.op.clone();
                let result = match &op {
                    FileOp::NewFolder(parent) => file_ops::validate_name(&bar.name).and_then(|n| file_ops::new_folder(parent, n)),
                    FileOp::Rename(path) => file_ops::validate_name(&bar.name).and_then(|n| file_ops::rename(path, n)),
                    FileOp::Delete(path) => file_ops::delete(path).map(|()| path.clone()),
                };
                let done = match result {
                    Ok(done) => done,
                    Err(error) => {
                        bar.error = Some(error);
                        return Task::none();
                    }
                };
                self.file_op = None;
                self.focus = Focus::Editor;
                return match op {
                    FileOp::NewFolder(parent) => {
                        if let Some(project) = &mut self.project {
                            project.explorer.refresh(&parent);
                        }
                        self.status = Some(format!("Created {}", done.display()));
                        Task::none()
                    }
                    FileOp::Rename(from) => self.after_rename(&from, &done),
                    FileOp::Delete(path) => self.after_delete(&path),
                };
            }
        }
        Task::none()
    }

    /// Open tabs follow a renamed file or folder; files still using its old
    /// `res://` path are listed in the SEARCH tab.
    fn after_rename(&mut self, from: &Path, to: &Path) -> Task<Message> {
        // Breakpoints of closed files follow too (open ones live in their tab).
        let moved: Vec<(PathBuf, PathBuf)> =
            self.breakpoints.keys().filter_map(|p| file_ops::moved(p, from, to).map(|n| (p.clone(), n))).collect();
        for (old, new) in moved {
            if let Some(lines) = self.breakpoints.remove(&old) {
                self.breakpoints.insert(new, lines);
            }
        }
        let client = self.lsp.client().cloned();
        for tab in &mut self.tabs {
            let Some(path) = file_ops::moved(&tab.path, from, to) else {
                continue;
            };
            if let (Language::GDScript, Some(client)) = (tab.language, &client) {
                let _ = client.did_close(&tab.path);
            }
            let language = Language::from_path(&path);
            if language != tab.language {
                tab.language = language;
                tab.buffer.set_language(language);
            }
            tab.path = path;
            if let (Language::GDScript, Some(client)) = (tab.language, &client) {
                let _ = client.did_open(&tab.path, tab.version, &tab.buffer.text());
            }
        }
        let Some(project) = &mut self.project else {
            return Task::none();
        };
        if let Some(parent) = to.parent() {
            project.explorer.refresh(parent);
        }
        let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.status = Some(format!("Renamed to {name}"));
        let Some(godot) = &project.godot else {
            return Task::none();
        };
        let Some(mut old) = godot.to_res_path(from) else {
            return Task::none();
        };
        if to.is_dir() {
            old.push('/');
        }
        let users = file_ops::files_mentioning(&godot.resource_files(), &old);
        if users == 0 {
            return Task::none();
        }
        let files = if users == 1 { "1 file still uses".to_string() } else { format!("{users} files still use") };
        self.status = Some(format!("Renamed to {name}. {files} {old}: listed in SEARCH"));
        self.update(Message::SearchProject(old))
    }

    /// Tabs of a deleted file (or of files in a deleted folder) close.
    fn after_delete(&mut self, path: &Path) -> Task<Message> {
        for index in (0..self.tabs.len()).rev() {
            if file_ops::moved(&self.tabs[index].path, path, path).is_some() {
                self.pending_close = Some(self.tabs[index].path.clone());
                self.close_tab(index);
            }
        }
        self.sync_groups();
        // After closing its tabs, which hand their breakpoints back.
        self.breakpoints.retain(|p, _| file_ops::moved(p, path, path).is_none());
        let Some(project) = &mut self.project else {
            return Task::none();
        };
        if let Some(parent) = path.parent() {
            project.explorer.refresh(parent);
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.status = Some(format!("Moved {name} to the Recycle Bin"));
        Task::none()
    }

    /// Inserts a file dragged from the explorer where it was dropped in the
    /// active tab: `preload("res://…")`, or the bare path in a string.
    /// Puts back the staged lines of `hunk` in `path`'s tab (opening it),
    /// as one undo step, left unsaved.
    fn revert_hunk(&mut self, path: &Path, hunk: &Hunk) {
        let found = |tabs: &[Tab]| tabs.iter().position(|t| uri::same_path(&t.path, path));
        if found(&self.tabs).is_none() && !self.open_file(path.to_path_buf()) {
            return;
        }
        let Some(index) = found(&self.tabs) else {
            return;
        };
        self.focus_tab(index);
        // A tab just opened gets its staged text with the next refresh; the
        // last refresh already read it.
        if self.tabs[index].git.base.is_none() {
            let base = self.project.as_ref().and_then(|p| p.git_files.iter().find(|f| uri::same_path(&f.path, path)));
            if let Some(base) = base.and_then(|f| f.base.clone()) {
                self.tabs[index].git.set_base(Some(base));
            }
        }
        let tab = &self.tabs[index];
        let Some(base) = tab.git.base.as_deref() else {
            self.status = Some(format!("{} has no staged version to go back to", tab.title()));
            return;
        };
        // The text may have changed since the panel was drawn.
        if !tab.git.hunks(&tab.buffer).contains(hunk) {
            self.status = Some("That change moved meanwhile; try again from the CHANGES panel".to_string());
            return;
        }
        let (range, text) = diff::revert(base, &tab.buffer.text(), hunk);
        self.apply_command(Command::Replace(vec![(range.start, range.end, text)]));
        self.status = Some(format!("Reverted a change in {} (unsaved; Ctrl+Z undoes it)", self.tabs[index].title()));
    }

    fn drop_file(&mut self, path: &Path, pos: Pos, ctrl: bool) {
        self.file_drag = None;
        let Some(res_path) = self.project.as_ref().and_then(|p| p.godot.as_ref()).and_then(|g| g.to_res_path(path)) else {
            return;
        };
        let Some(tab) = self.active_tab() else {
            return;
        };
        let text = paths::drop_text(&tab.buffer, tab.buffer.pos_to_char(pos), &res_path, ctrl);
        self.dismiss_popups();
        self.focus = Focus::Editor;
        self.apply_command(Command::Click { pos, extend: false });
        self.apply_command(Command::Insert(text));
    }

    fn goto_definition(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        // A `res://` / `uid://` path in a string opens that file.
        if let Some(link) = paths::link_at(&tab.buffer, tab.buffer.primary().head) {
            self.open_link(&link);
            return;
        }
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

    /// After an empty definition: asks for the hover, whose signature names
    /// the engine class of a built-in symbol.
    fn request_native_definition(&mut self) {
        let Some((client, tab, pos)) = self.lsp_target() else {
            return;
        };
        match client.hover(&tab.path, to_lsp(pos)) {
            Ok(id) => {
                self.pending.insert(id, Pending::NativeDefinition);
            }
            Err(_) => self.status = Some("No definition found".to_string()),
        }
    }

    /// Shows the class reference (as it was left), at `class` and filtered
    /// to `member` when given.
    fn open_class_ref(&mut self, class: Option<String>, member: Option<String>) -> Task<Message> {
        self.shortcuts = None;
        self.palette = None;
        self.dismiss_popups();
        self.focus = Focus::Input;
        if self.class_ref.is_none() {
            self.class_ref = Some(self.hidden_class_ref.take().unwrap_or_default());
            self.class_ref_side = self.focused_group;
        }
        self.show_class_ref();
        let Some(page) = &mut self.class_ref else {
            return Task::none();
        };
        let Some(class) = class else {
            return Task::batch([
                iced::widget::operation::focus(class_ref::FILTER),
                iced::widget::operation::select_all(class_ref::FILTER),
            ]);
        };
        page.member_filter = member.unwrap_or_default();
        if page.page.as_ref().is_some_and(|p| p.name == class) && page.loading.is_none() {
            return Task::none();
        }
        self.load_class(class, true)
    }

    /// Asks for a class's docs; `remember` puts the current class in the
    /// Back history.
    fn load_class(&mut self, class: String, remember: bool) -> Task<Message> {
        let Some(client) = self.lsp.client().cloned() else {
            self.status = Some("The class reference needs the language server".to_string());
            return Task::none();
        };
        match client.native_symbol(&class, &class) {
            Ok(id) => {
                self.pending.insert(id, Pending::ClassDoc(class.clone()));
                if let Some(page) = &mut self.class_ref {
                    if let Some(current) = page.page.as_ref().filter(|_| remember) {
                        page.history.push(current.name.clone());
                    }
                    page.loading = Some(class);
                }
            }
            Err(error) => self.status = Some(format!("Cannot load {class}: {error}")),
        }
        Task::none()
    }

    fn on_class_ref(&mut self, message: ClassRefMessage) -> Task<Message> {
        // Using the page (typing in its filters too) makes it the active tab.
        if !matches!(message, ClassRefMessage::Close) && !self.class_ref_focused {
            self.show_class_ref();
        }
        let Some(page) = &mut self.class_ref else {
            return Task::none();
        };
        match message {
            ClassRefMessage::Filter(filter) => page.filter = filter,
            ClassRefMessage::MemberFilter(filter) => page.member_filter = filter,
            ClassRefMessage::Open(class_ref::Link::Class(class)) => return self.open_class_ref(Some(class), None),
            ClassRefMessage::Open(class_ref::Link::Member(class, member)) => return self.open_class_ref(Some(class), Some(member)),
            ClassRefMessage::Back => {
                if let Some(previous) = page.history.pop() {
                    page.member_filter.clear();
                    return self.load_class(previous, false);
                }
            }
            ClassRefMessage::Close => self.close_class_ref(),
        }
        Task::none()
    }

    /// Closes the class reference tab, keeping its state to reopen as it was.
    fn close_class_ref(&mut self) {
        if let Some(page) = self.class_ref.take() {
            self.hidden_class_ref = Some(page);
        }
        self.class_ref_shown = false;
        self.class_ref_focused = false;
        self.focus = Focus::Editor;
    }

    /// Makes the class reference the active tab: in front of its side, and
    /// that side focused.
    fn show_class_ref(&mut self) {
        if self.class_ref.is_none() {
            return;
        }
        self.class_ref_shown = true;
        self.class_ref_focused = true;
        self.dismiss_popups();
        self.find = None;
        self.rename = None;
        self.focus = Focus::Input;
    }

    /// Whether the class reference is the active tab: file commands (save,
    /// edits) then have no target.
    fn class_ref_active(&self) -> bool {
        self.class_ref.is_some() && self.class_ref_focused
    }

    /// The existing file a `res://` or `uid://` path points to.
    fn resolve_link(&mut self, link: &str) -> Option<PathBuf> {
        let godot = self.project.as_ref()?.godot.as_ref()?;
        let path = if link.starts_with("uid://") {
            self.uid_cache.entry(link.to_string()).or_insert_with(|| godot.find_uid(link)).clone()
        } else {
            godot.resolve(link)
        };
        path.filter(|p| p.is_file())
    }

    /// Ctrl is held over char `index` (or released): finds whether a Ctrl+click
    /// there would go somewhere, to underline it. Paths are checked here;
    /// words ask the language server for a definition.
    fn on_link_hover(&mut self, index: Option<usize>) {
        let Some(index) = index else {
            self.link = None;
            self.link_probe = None;
            return;
        };
        if self.link.is_some_and(|(start, end)| (start..end).contains(&index)) {
            return;
        }
        self.link = None;
        self.link_probe = None;
        let Some(tab) = self.active_tab() else {
            return;
        };
        if let Some(link) = paths::link_at(&tab.buffer, index) {
            let range = paths::string_at(&tab.buffer, index).map(|s| (s.content_start, s.content_end));
            if self.resolve_link(&link).is_some() {
                self.link = range;
            }
            return;
        }
        let (start, end) = tab.buffer.word_range(index);
        let is_word = tab.buffer.slice(start, end).starts_with(|c: char| c.is_alphabetic() || c == '_')
            && !paths::in_prose(&tab.buffer, index);
        let Some((client, tab, _)) = self.lsp_target().filter(|_| is_word) else {
            return;
        };
        let pos = tab.buffer.char_to_pos(index);
        if let Ok(id) = client.definition(&tab.path, to_lsp(pos)) {
            self.pending.insert(id, Pending::LinkProbe((start, end)));
            self.link_probe = Some(id);
        }
    }

    /// Asks for the hover of the word at `range` (which has no definition),
    /// to underline it when it is an engine symbol.
    fn probe_native_link(&mut self, range: (usize, usize)) {
        let Some((client, tab, _)) = self.lsp_target() else {
            return;
        };
        let pos = tab.buffer.char_to_pos(range.0);
        if let Ok(id) = client.hover(&tab.path, to_lsp(pos)) {
            self.pending.insert(id, Pending::NativeLinkProbe(range));
            self.link_probe = Some(id);
        }
    }

    /// Opens the file a `res://` or `uid://` path points to: text files in a
    /// tab, others (images, sounds, models) with the system's default app.
    fn open_link(&mut self, link: &str) {
        if self.project.as_ref().is_none_or(|p| p.godot.is_none()) {
            return;
        }
        let Some(path) = self.resolve_link(link) else {
            self.status = Some(format!("{link} not found in the project"));
            return;
        };
        self.dismiss_popups();
        if is_text_file(&path) {
            self.open_file(path);
        } else {
            self.status = Some(match open_external(&path) {
                Ok(()) => format!("Opened {} with the default app", self.display_path(&path)),
                Err(error) => format!("Cannot open {}: {error}", path.display()),
            });
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
        // Keep the session (and unsaved edits) of the project being left.
        self.save_session();
        self.saved_session = None;
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
            info: ProjectInfo::read(godot.as_ref()),
            repo: git::repo_root(&root),
            git_status: git::Status::default(),
            git_files: Vec::new(),
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
        self.restore_session();
        self.git_refresh.wanted = true;
        if let Some(file) = file {
            self.open_file(file);
        }
    }

    fn format_active(&mut self) {
        let Some(index) = self.active_index() else {
            return;
        };
        self.status = Some(match self.format_tab(index) {
            Ok(None) => "Already formatted".to_string(),
            Ok(Some(tool)) => format!("Formatted with {tool}"),
            Err(error) => error,
        });
    }

    /// Formats a tab as one undo step: gdformat for GDScript when it is
    /// installed, else trailing whitespace and final newline tidied.
    /// Returns the tool used, or `None` if nothing changed.
    fn format_tab(&mut self, index: usize) -> Result<Option<&'static str>, String> {
        let Some(tab) = self.tabs.get(index) else {
            return Ok(None);
        };
        // Trailing double spaces are line breaks in Markdown.
        if tab.language == Language::Markdown {
            return Err("Markdown is not formatted".to_string());
        }
        let text = tab.buffer.text();
        let gdformat = (tab.language == Language::GDScript).then(format::gdformat_path).flatten();
        let (formatted, tool) = match gdformat {
            Some(exe) => (format::gdformat(&exe, &text)?, "gdformat"),
            None => (format::tidy(&text), "the built-in tidy (install gdtoolkit for gdformat)"),
        };
        match format::minimal_edit(&text, &formatted) {
            Some(edit) => {
                self.edit_tab(index, Command::Replace(vec![edit]));
                Ok(Some(tool))
            }
            None => Ok(None),
        }
    }

    /// Keys the terminal leaves to the IDE (all others go to the shell).
    fn terminal_passthrough(&self) -> Vec<keymap::Chord> {
        keymap::TERMINAL_PASSTHROUGH.iter().flat_map(|id| self.keymap.chords(id).iter().cloned()).collect()
    }

    /// Runs a key-mapped command.
    fn run_command(&mut self, id: &str) -> Task<Message> {
        match command_message(id) {
            Some(message) => self.handle(message),
            None => Task::none(),
        }
    }

    /// A key press that no widget used: shortcut recording on the shortcuts
    /// page, palette navigation, then the key map.
    fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        let (key, modifiers) = match numpad::fix_key_event(event) {
            keyboard::Event::KeyPressed { key, modifiers, .. } => (key, modifiers),
            keyboard::Event::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                return Task::none();
            }
            keyboard::Event::KeyReleased { .. } => return Task::none(),
        };
        if let Some(id) = self.shortcuts.as_ref().and_then(|page| page.recording) {
            self.record_shortcut(id, &key, modifiers);
            return Task::none();
        }
        use keyboard::key::Named;
        // Palette list navigation (text inputs leave these keys alone).
        let palette_move = match key.as_ref() {
            keyboard::Key::Named(Named::ArrowUp) => Some(-1),
            keyboard::Key::Named(Named::ArrowDown) => Some(1),
            keyboard::Key::Named(Named::PageUp) => Some(-12),
            keyboard::Key::Named(Named::PageDown) => Some(12),
            _ => None,
        };
        if let (Some(delta), Some(_)) = (palette_move, &self.palette) {
            return self.handle(Message::Palette(PaletteMessage::Move(delta)));
        }
        match keymap::Chord::from_key(&key, modifiers).and_then(|chord| self.keymap.lookup(&chord)) {
            Some(id) => self.run_command(id),
            None => Task::none(),
        }
    }

    /// The next key pressed on the shortcuts page becomes `id`'s shortcut.
    fn record_shortcut(&mut self, id: &'static str, key: &keyboard::Key, modifiers: keyboard::Modifiers) {
        use keyboard::key::Named;
        let done = |app: &mut Self| {
            if let Some(page) = &mut app.shortcuts {
                page.recording = None;
            }
        };
        match key.as_ref() {
            keyboard::Key::Named(Named::Escape) => {
                done(self);
                self.status = Some("Shortcut unchanged".to_string());
                return;
            }
            keyboard::Key::Named(Named::Backspace | Named::Delete) if modifiers.is_empty() => {
                done(self);
                self.keymap.unbind(id);
                self.status = Some(format!("{}: no shortcut", keymap::title(id)));
            }
            _ => {
                // Lone modifier presses arrive as named keys: wait for more.
                let Some(chord) = keymap::Chord::from_key(key, modifiers) else {
                    if matches!(key, keyboard::Key::Character(_)) || matches!(key.as_ref(), keyboard::Key::Named(Named::Enter | Named::Tab | Named::Space)) {
                        self.status = Some("Shortcuts need Ctrl or Alt (or a function key). Escape cancels".to_string());
                    }
                    return;
                };
                done(self);
                let taken = self.keymap.rebind(id, chord.clone());
                let mut status = format!("{}: {chord}", keymap::title(id));
                if !taken.is_empty() {
                    let titles: Vec<&str> = taken.iter().map(|other| keymap::title(other)).collect();
                    status.push_str(&format!(" (removed from {})", titles.join(", ")));
                }
                self.status = Some(status);
            }
        }
        if let Err(error) = self.keymap.save() {
            self.status = Some(format!("Cannot save shortcuts: {error}"));
        }
    }

    /// Copies the selection to the clipboard (the right-click menu's Copy /
    /// Cut; the keys are handled in the editor itself).
    fn copy_selection(&mut self, cut: bool) -> Task<Message> {
        let selected = self.active_tab().map(|tab| tab.buffer.selected_text()).unwrap_or_default();
        if selected.is_empty() {
            return Task::none();
        }
        if cut {
            self.apply_command(Command::DeleteSelection);
        }
        iced::clipboard::write(selected)
    }

    /// Opens a file on the other side of the split (making one if needed).
    fn open_to_side(&mut self, path: PathBuf) {
        let target = if self.is_split() { 1 - self.focused_group } else { 1 };
        if !self.open_file(path) {
            return;
        }
        if let Some(active) = self.active {
            self.tabs[active].group = target;
        }
        self.dismiss_popups();
        self.sync_groups();
    }

    /// Closes every tab but `keep` (`None`: but the class reference).
    fn close_other_tabs(&mut self, keep: Option<usize>) {
        let path = match keep {
            Some(keep) => match self.tabs.get(keep) {
                Some(tab) => Some(tab.path.clone()),
                None => return,
            },
            None if self.class_ref.is_some() => None,
            None => return,
        };
        // Back to front, so indices stay valid; unsaved tabs stay open.
        for index in (0..self.tabs.len()).rev() {
            if Some(index) != keep && !self.tabs[index].dirty {
                self.close_tab(index);
            }
        }
        match &path {
            Some(path) => {
                if self.class_ref.is_some() {
                    self.close_class_ref();
                }
                if let Some(index) = self.tabs.iter().position(|t| &t.path == path) {
                    self.focus_tab(index);
                }
            }
            None => self.show_class_ref(),
        }
        if self.tabs.iter().any(|t| t.dirty && Some(&t.path) != path.as_ref()) {
            self.status = Some("Tabs with unsaved changes were kept open".to_string());
        }
    }

    /// The right-click menu for a file, folder, tab or the editor.
    fn context_entries(&self, target: &ContextTarget) -> Vec<context_menu::Entry<Message>> {
        use context_menu::Entry;
        let pick = |message: Message| Message::MenuPick(Box::new(message));
        // Current keys of a command, as the menus show them.
        let key = |id: &str| self.keymap.label(id);
        let godot = self.project.as_ref().and_then(|p| p.godot.as_ref());
        let paths = |path: &Path| -> Vec<Entry<Message>> {
            let mut entries = vec![Entry::item("Copy Path", pick(Message::CopyText(path.display().to_string())))];
            if let Some(res) = godot.and_then(|g| g.to_res_path(path)) {
                entries.push(Entry::item("Copy res:// Path", pick(Message::CopyText(res))));
            }
            entries.push(Entry::item("Reveal in File Explorer", pick(Message::Reveal(path.to_path_buf()))));
            entries
        };
        match target {
            ContextTarget::File(path) => {
                let mut entries = vec![
                    Entry::item("Open", pick(Message::OpenFile(path.clone()))),
                    Entry::with_shortcut("Open to the Side", "split view", Some(pick(Message::OpenToSide(path.clone())))),
                    Entry::Separator,
                    Entry::item("New Script Here...", pick(Message::OpenNewScript(Some(path.clone())))),
                    Entry::Separator,
                    Entry::item("Rename...", pick(Message::OpenFileOp(FileOp::Rename(path.clone())))),
                    Entry::item("Delete", pick(Message::OpenFileOp(FileOp::Delete(path.clone())))),
                    Entry::Separator,
                ];
                entries.extend(paths(path));
                entries
            }
            ContextTarget::Folder(path) => {
                let mut entries = vec![
                    Entry::item("New Script...", pick(Message::OpenNewScript(Some(path.clone())))),
                    Entry::item("New Folder...", pick(Message::OpenFileOp(FileOp::NewFolder(path.clone())))),
                    Entry::Separator,
                ];
                // The project root itself can't be renamed or deleted from here.
                if self.project.as_ref().is_none_or(|p| p.root != *path) {
                    entries.extend([
                        Entry::item("Rename...", pick(Message::OpenFileOp(FileOp::Rename(path.clone())))),
                        Entry::item("Delete", pick(Message::OpenFileOp(FileOp::Delete(path.clone())))),
                        Entry::Separator,
                    ]);
                }
                entries.extend(paths(path));
                entries
            }
            ContextTarget::Tab(index) => {
                let Some(tab) = self.tabs.get(*index) else {
                    return Vec::new();
                };
                let side = if self.is_split() { "Move to the Other Side" } else { "Move to the Right Side" };
                let mut entries = vec![
                    Entry::with_shortcut("Close", key("file.closeTab"), Some(pick(Message::CloseTab(*index)))),
                    Entry::item("Close Others", pick(Message::CloseOtherTabs(Some(*index)))),
                    Entry::with_shortcut(side, key("view.split"), Some(pick(Message::MoveTabToOtherSide(Some(*index))))),
                    Entry::Separator,
                ];
                entries.extend(paths(&tab.path));
                entries
            }
            ContextTarget::ClassRef => {
                let side = if self.is_split() { "Move to the Other Side" } else { "Move to the Right Side" };
                vec![
                    Entry::with_shortcut("Close", key("file.closeTab"), Some(pick(Message::ClassRef(ClassRefMessage::Close)))),
                    Entry::item("Close Others", pick(Message::CloseOtherTabs(None))),
                    Entry::with_shortcut(side, key("view.split"), Some(pick(Message::MoveTabToOtherSide(None)))),
                ]
            }
            ContextTarget::Editor => {
                let Some(tab) = self.active_tab() else {
                    return Vec::new();
                };
                let code = tab.language == Language::GDScript && self.lsp.client().is_some();
                let selection = !tab.buffer.primary().is_empty();
                let when = |enabled: bool, message: Message| enabled.then(|| pick(message));
                vec![
                    Entry::with_shortcut("Go to Definition", key("go.definition"), when(code, Message::GotoDefinition)),
                    Entry::with_shortcut("Find All References", key("go.references"), when(code, Message::FindReferences)),
                    Entry::with_shortcut("Rename Symbol", key("refactor.rename"), when(code, Message::StartRename)),
                    Entry::Separator,
                    Entry::with_shortcut("Cut", "Ctrl+X", when(selection, Message::EditorCut)),
                    Entry::with_shortcut("Copy", "Ctrl+C", when(selection, Message::EditorCopy)),
                    Entry::with_shortcut("Paste", "Ctrl+V", Some(pick(Message::EditorPaste))),
                    Entry::Separator,
                    Entry::with_shortcut("Toggle Comment", key("edit.toggleComment"), Some(pick(Message::Edit(EditorEvent::Command(Command::ToggleComment))))),
                    Entry::with_shortcut("Fold / Unfold", key("edit.fold"), Some(pick(Message::Edit(EditorEvent::Command(Command::ToggleFoldAtCursor))))),
                    Entry::with_shortcut("Format Document", key("edit.format"), Some(pick(Message::Edit(EditorEvent::FormatDocument)))),
                    Entry::Separator,
                    Entry::with_shortcut("Move to the Other Side", key("view.split"), Some(pick(Message::SplitMove))),
                ]
            }
        }
    }

    /// Leaves the project for the welcome screen, keeping its session.
    fn close_folder(&mut self) {
        self.save_session();
        self.saved_session = None;
        self.project = None;
        self.tabs.clear();
        self.active = None;
        self.dismiss_popups();
        self.find = None;
        self.rename = None;
        self.outline = None;
        self.reset_lsp(Lsp::Off);
        self.reset_terminal();
        self.run = RunState::default();
        self.panel = None;
        self.recent = recent::load();
    }

    /// Opens (or focuses) a file tab. Returns whether it is now active.
    fn open_file(&mut self, path: PathBuf) -> bool {
        // Opening a file shows it, even from the shortcuts page.
        self.shortcuts = None;
        // A file on the class reference's side takes its place.
        let group = self.tabs.iter().find(|t| uri::same_path(&t.path, &path)).map_or(self.focused_group, |t| t.group);
        if group == self.class_ref_side {
            self.class_ref_shown = false;
        }
        self.link = None;
        if let Some(index) = self.tabs.iter().position(|t| uri::same_path(&t.path, &path)) {
            self.focus_tab(index);
            return true;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.add_tab(path, &text, false);
                true
            }
            Err(error) => {
                self.status = Some(format!("Cannot open {}: {error}", path.display()));
                false
            }
        }
    }

    /// Adds and activates a tab showing `text` (the file's contents, or
    /// unsaved edits restored from the session).
    fn add_tab(&mut self, path: PathBuf, text: &str, dirty: bool) {
        let language = Language::from_path(&path);
        let mut tab = Tab {
            language,
            buffer: Buffer::new(text, language),
            path,
            dirty,
            version: 1,
            marks: Vec::new(),
            disk_changed: false,
            group: self.focused_group,
            git: GitMarks::default(),
        };
        if let Some(lines) = self.breakpoints.get(&tab.path) {
            tab.buffer.set_line_marks(lines.iter().copied());
        }
        // The staged text from the last refresh shows the markers at once.
        if let Some(file) = self.project.as_ref().and_then(|p| p.git_files.iter().find(|f| uri::same_path(&f.path, &tab.path))) {
            tab.git.set_base(file.base.clone());
        }
        self.git_refresh.wanted = true;
        if let (Language::GDScript, Some(client)) = (tab.language, self.lsp.client()) {
            let _ = client.did_open(&tab.path, tab.version, &tab.buffer.text());
        }
        self.tabs.push(tab);
        self.focus_tab(self.tabs.len() - 1);
        self.refresh_marks();
    }

    /// What to restore next time this project opens.
    fn session_snapshot(&self) -> session::Session {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                let pos = tab.buffer.char_to_pos(tab.buffer.primary().head);
                let disk = || std::fs::read_to_string(&tab.path).ok().map(|text| session::content_hash(&text));
                session::TabState {
                    path: tab.path.clone(),
                    line: pos.line,
                    column: pos.column,
                    group: tab.group,
                    folds: tab.buffer.folded().iter().map(|&(header, _)| header).collect(),
                    unsaved: tab.dirty.then(|| tab.buffer.file_contents()),
                    disk_hash: if tab.dirty { disk() } else { None },
                }
            })
            .collect();
        let split_ratio = Some(self.split_ratio).filter(|&r| r != 0.5);
        let breakpoints = self
            .all_breakpoints()
            .iter()
            .filter(|(_, lines)| !lines.is_empty())
            .map(|(path, lines)| (path.clone(), lines.iter().copied().collect()))
            .collect();
        session::Session {
            tabs,
            active: self.active,
            split_ratio,
            godot: self.godot_choice.clone(),
            breakpoints,
            watches: self.watches.clone(),
        }
    }

    /// Saves the project's session if it changed since the last save.
    fn save_session(&mut self) {
        let Some(project) = &self.project else {
            return;
        };
        let snapshot = self.session_snapshot();
        if self.saved_session.as_ref() == Some(&snapshot) {
            return;
        }
        match session::save(&project.root, &snapshot) {
            Ok(()) => self.saved_session = Some(snapshot),
            Err(error) => self.status = Some(format!("Cannot save the session: {error}")),
        }
    }

    /// Reopens the tabs of the project's last session, unsaved edits included.
    fn restore_session(&mut self) {
        let Some(project) = &self.project else {
            return;
        };
        let saved = session::load(&project.root);
        self.split_ratio = saved.split_ratio.unwrap_or(0.5).clamp(0.0, 1.0);
        self.godot_choice = saved.godot.clone();
        self.watches = saved.watches.clone();
        self.breakpoints = saved.breakpoints.iter().map(|(path, lines)| (path.clone(), lines.iter().copied().collect())).collect();
        for state in &saved.tabs {
            if self.tabs.iter().any(|t| uri::same_path(&t.path, &state.path)) {
                continue;
            }
            match &state.unsaved {
                Some(text) => {
                    self.add_tab(state.path.clone(), text, true);
                    // The file changed (or vanished) since the edits were kept.
                    let disk = std::fs::read_to_string(&state.path).ok().map(|d| session::content_hash(&d));
                    if disk != state.disk_hash {
                        if let Some(tab) = self.tabs.last_mut() {
                            tab.disk_changed = true;
                        }
                    }
                }
                None if state.path.is_file() => {
                    if !self.open_file(state.path.clone()) {
                        continue;
                    }
                }
                None => continue,
            }
            if let Some(tab) = self.tabs.last_mut() {
                tab.group = state.group.min(1);
                for &header in &state.folds {
                    tab.buffer.apply(Command::ToggleFold(header));
                }
                tab.buffer.set_cursor(Pos { line: state.line, column: state.column });
            }
        }
        if let Some(active) = saved.active.filter(|&i| i < self.tabs.len()) {
            self.active = Some(active);
        }
        // Restored before the first update: put the split in order now.
        self.sync_groups();
        self.saved_session = Some(saved);
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
        // Its breakpoints, as moved by edits, stay known once it is closed.
        if tab.language == Language::GDScript {
            let marks: std::collections::BTreeSet<usize> = tab.buffer.line_marks().into_iter().collect();
            if !marks.is_empty() || self.breakpoints.contains_key(&tab.path) {
                self.breakpoints.insert(tab.path.clone(), marks);
            }
        }
        self.pending_close = None;
        self.tabs.remove(index);
        // Indices after the removed tab shift down; sync_groups refills gaps.
        for shown in &mut self.group_active {
            *shown = match *shown {
                Some(i) if i == index => None,
                Some(i) if i > index => Some(i - 1),
                other => other,
            };
        }
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) => Some(active.min(self.tabs.len() - 1)),
            None => None,
        };
    }

    fn save_active(&mut self) {
        if let Some(index) = self.active_index() {
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
        let formats = self.tabs.get(index).is_some_and(|t| matches!(t.language, Language::GDScript | Language::Shader));
        if self.settings.format_on_save && formats {
            if let Err(error) = self.format_tab(index) {
                // Saved as typed: a formatter error must not block saving.
                self.status = Some(error);
            }
        }
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
                // Godot reads the saved file: its breakpoints may have moved with edits.
                if tab.language == Language::GDScript {
                    let path = tab.path.clone();
                    self.send_file_breakpoints(&path);
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
        self.active_index().and_then(|i| self.tabs.get(i))
    }

    /// The active file tab: none while the class reference is active.
    fn active_index(&self) -> Option<usize> {
        self.active.filter(|_| !self.class_ref_active())
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
                let mut main = column![];
                if let Some((script, error)) = &self.new_script {
                    let bar = script.view(&self.current_theme().ui, error.as_deref()).map(Message::NewScript);
                    main = main.push(on_escape::on_escape(bar, Message::NewScript(NewScriptMessage::Close)));
                }
                if let Some(bar) = &self.file_op {
                    let bar = bar.view(&self.current_theme().ui).map(Message::FileOp);
                    main = main.push(on_escape::on_escape(bar, Message::FileOp(FileOpMessage::Close)));
                }
                main = main.push(self.editor_area());
                if let Some(panel) = self.panel {
                    let ui = &self.current_theme().ui;
                    let (line, active) = (style::to_color(ui.border), style::to_color(ui.accent));
                    main = main.push(splitter::splitter(line, active, Message::ResizePanel));
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
        let window: Element<'_, Message> = column![container(body).height(Length::Fill), self.status_bar()].into();
        // A right-click menu sits over the whole window: a transparent
        // backdrop closes it on any click elsewhere, Escape too.
        let window = match &self.context_menu {
            Some(menu) => {
                let backdrop = mouse_area(container(space()).width(Length::Fill).height(Length::Fill))
                    .on_press(Message::CloseMenu)
                    .on_right_press(Message::CloseMenu);
                let size = iced::Size::new(self.window_width, self.window_height);
                let menu = menu.view(&self.current_theme().ui, size, |message| message);
                let menu = on_escape::on_escape(menu, Message::CloseMenu);
                iced::widget::stack![window, backdrop, menu].into()
            }
            None => window,
        };
        numpad::numpad_fix(window).into()
    }

    fn class_ref_page(&self) -> Element<'_, Message> {
        let Some(page) = &self.class_ref else {
            return space().into();
        };
        let theme = self.current_theme();
        let view = page.view(&self.native_classes, theme).map(Message::ClassRef);
        container(view).width(Length::Fill).height(Length::Fill).style(style::surface(&theme.ui)).into()
    }

    fn shortcuts_page(&self) -> Element<'_, Message> {
        let Some(page) = &self.shortcuts else {
            return space().into();
        };
        let ui = &self.current_theme().ui;
        let view = page.view(ui, &self.keymap).map(Message::Shortcuts);
        let view = on_escape::on_escape(view, Message::Shortcuts(ShortcutsMessage::Close));
        container(view).width(Length::Fill).height(Length::Fill).style(style::surface(ui)).into()
    }

    fn welcome(&self) -> Element<'_, Message> {
        let ui = &self.current_theme().ui;
        let muted = style::to_color(ui.text_muted);
        let mut content = column![
            text("Beckett").size(28),
            text("An IDE for Godot").color(muted),
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

        let ui = &self.current_theme().ui;
        let view_tab = |label: &'static str, view: SidebarView, description: &'static str| {
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if self.sidebar_view == view { button::secondary } else { button::text };
            let tab = button(text(label).size(11)).on_press(Message::ShowSidebar(view)).style(style).padding([2, 8]);
            style::tip(tab, description, tooltip::Position::Bottom, ui)
        };
        let tabs = row![
            view_tab("FILES", SidebarView::Files, "Project files"),
            view_tab("OUTLINE", SidebarView::Outline, "Functions, variables and signals of the current file (Ctrl+Shift+O to search them)"),
            view_tab("PROJECT", SidebarView::Project, "Main scene, autoloads and Input Map of project.godot"),
        ]
        .spacing(4)
        .padding([0, 8]);

        let body: Element<'a, Message> = match self.sidebar_view {
            SidebarView::Files => {
                let active = self.active_tab().map(|t| t.path.as_path());
                // Right-click below or between the rows: the project root's menu.
                let root = project.root.clone();
                let ui = &self.current_theme().ui;
                let git = explorer::GitLook {
                    status: &project.git_status,
                    modified: style::to_color(ui.warning),
                    added: style::to_color(ui.success),
                    conflicted: style::to_color(ui.danger),
                };
                let files = scrollable(project.explorer.view(active, &git)).height(Length::Fill);
                fallback_context_area(files, move |point| Message::ContextMenu(ContextTarget::Folder(root.clone()), point)).into()
            }
            SidebarView::Outline => {
                let tab = self.active_tab();
                let outline = self.outline.as_ref().filter(|o| tab.is_some_and(|t| uri::same_path(&t.path, &o.path)));
                let line = tab.map(|t| t.buffer.char_to_pos(t.buffer.primary().head).line as u32);
                outline::view(outline, line, self.current_theme())
            }
            SidebarView::Project => project_panel::view(&project.info, project.godot.as_ref(), self.current_theme()),
        };

        container(column![header, tabs, body].spacing(4))
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
        if !self.is_split() {
            return self.pane(0);
        }
        let ui = &self.current_theme().ui;
        let (line, active) = (style::to_color(ui.border), style::to_color(ui.accent));
        // Laid out at the area's width, which turns drags into a share of it.
        responsive(move |size| {
            let width = (size.width - splitter::THICKNESS).max(1.0);
            row![
                container(self.pane(0)).width(split::left_width(self.split_ratio, width)),
                splitter::vertical(line, active, move |delta| Message::ResizeSplit(delta, width)),
                container(self.pane(1)).width(Length::Fill),
            ]
            .into()
        })
        .into()
    }

    /// Whether the editor area is split in two sides.
    fn is_split(&self) -> bool {
        self.tabs.iter().any(|t| t.group == 1) || (self.class_ref.is_some() && self.class_ref_side == 1)
    }

    /// One side of the editor area: its tabs and the file it shows. Only
    /// the focused side has the find / rename bars and the popups.
    fn pane(&self, group: usize) -> Element<'_, Message> {
        let focused = group == self.focused_group;
        if self.class_ref.is_some() && self.class_ref_shown && group == self.class_ref_side {
            // A click on the page's background focuses its side too.
            let page = mouse_area(self.class_ref_page()).on_press(Message::ShowClassRef);
            return column![self.tab_bar(group), page].into();
        }
        let Some(tab) = self.group_active[group].and_then(|i| self.tabs.get(i)) else {
            let muted = style::to_color(self.current_theme().ui.text_muted);
            return center(text("Open a file from the explorer").color(muted)).into();
        };

        let colors = code_editor::Colors::from_theme(self.current_theme());
        let mut editor = CodeEditor::new(&tab.buffer, colors, move |event| Message::EditIn(group, event))
            .font(code_editor::CodeFont::from_settings(&self.current_theme().font))
            .keymap(&self.keymap)
            .marks(&tab.marks)
            .focused(focused && self.focus == Focus::Editor)
            .dropping(self.file_drag.as_deref())
            .git_changes(tab.git.changes(&tab.buffer))
            .breakpoints(self.breakpoints_of(tab), self.paused_line_in(&tab.path));
        let mut area = column![self.tab_bar(group)];
        if !focused {
            return area.push(editor).into();
        }

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
        editor = editor
            .completion(completion)
            .hover(hover)
            .signature(signature)
            .tab_stops(self.snippet.is_some())
            .link(self.link);
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
            (Panel::Changes, match self.project.as_ref().map_or(0, |p| p.git_files.len()) {
                0 => "CHANGES".to_string(),
                n => format!("CHANGES  {n}"),
            }),
            (Panel::Debug, match &self.debug {
                Some(session) if session.paused() => "DEBUG (paused)".to_string(),
                Some(_) => "DEBUG (running)".to_string(),
                None => "DEBUG".to_string(),
            }),
        ]
        .into_iter()
        .map(|(tab, label)| {
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if tab == panel { button::secondary } else { button::text };
            button(text(label).size(12)).on_press(Message::ShowPanel(tab)).style(style).padding([2, 8]).into()
        });

        let mut header = Row::with_children(tabs).spacing(4).align_y(Alignment::Center);
        header = header.push(space::horizontal());
        match panel {
            Panel::Output if self.is_running() => header = header.push(action("stop", Message::StopRun, "Stop the game (F8)")),
            Panel::Output => header = header.push(action("clear", Message::ClearOutput, "Clear the output")),
            Panel::Terminal => header = header.push(action("kill", Message::KillTerminal, "End this shell (the last one closes the panel)")),
            Panel::Changes => {
                if let Some(path) = &self.changes_focus {
                    header = header.push(text(self.display_path(path)).size(12).color(muted));
                    header = header.push(action("show all", Message::ShowChanges(None), "List every changed file"));
                }
                header = header.push(action("refresh", Message::RefreshGit, "Read the git status again"));
            }
            Panel::Debug => {
                if let Some(session) = &self.debug {
                    let key = |id: &str| self.keymap.label(id);
                    for control in debugger::controls(session, ui, key) {
                        header = header.push(control.map(Message::Debug));
                    }
                } else {
                    let start = format!("Debug the main scene ({})", self.keymap.label("debug.start"));
                    header = header.push(action("start", Message::StartDebug { scene: false }, &start));
                }
            }
            Panel::Problems | Panel::Search => {}
        }
        header = header.push(action("x", Message::ClosePanel, "Close the panel"));

        let body: Element<'_, Message> = match panel {
            Panel::Problems => self.problems.view(ui, |p| self.display_path(p)),
            Panel::Search => self.search.view(ui, |p| self.display_path(p)).map(Message::Search),
            Panel::Output => self.output_view(),
            Panel::Changes => self.changes_view(),
            Panel::Debug => self.debug_view(),
            Panel::Terminal => match self.terminals.active().and_then(|t| t.session.as_ref()) {
                Some(session) => TerminalView::new(session, terminal_panel::palette(theme), Message::FocusTerminal)
                    .font(code_editor::CodeFont::from_settings(&theme.font))
                    .passthrough(self.terminal_passthrough())
                    .focused(self.focus == Focus::Terminal)
                    .selection_color(style::to_color(ui.accent).scale_alpha(0.4))
                    .on_link(style::to_color(ui.accent), Message::TerminalLink)
                    .into(),
                None => center(text("Starting shell...").size(12).color(muted)).into(),
            },
        };

        let mut content = column![header.padding([4, 8])];
        // The shells' tabs, on their own row under the panel's tabs.
        if panel == Panel::Terminal {
            content = content.push(container(self.terminal_tabs()).padding(iced::Padding { top: 0.0, bottom: 4.0, left: 8.0, right: 8.0 }));
        }
        container(content.push(body))
            .width(Length::Fill)
            .height(self.panel_height())
            .style(style::surface(ui))
            .into()
    }

    /// The DEBUG panel: the session's state, call stack and variables, or
    /// how to start one.
    fn debug_view(&self) -> Element<'_, Message> {
        let theme = self.current_theme();
        let ui = &theme.ui;
        let Some(session) = &self.debug else {
            let note = format!(
                "{} toggles a breakpoint on the cursor's line (or click a line number). {} debugs the main scene, {} the current one; F5 continues, {} / {} / {} step over / into / out. Runtime errors stop at their line too. While paused, hover a name for its value; watch expressions are added in this panel.",
                self.keymap.label("debug.toggleBreakpoint"),
                self.keymap.label("debug.start"),
                self.keymap.label("debug.startScene"),
                self.keymap.label("debug.stepOver"),
                self.keymap.label("debug.stepInto"),
                self.keymap.label("debug.stepOut"),
            );
            return container(text(note).size(12).color(style::to_color(ui.text_muted))).padding([6, 12]).into();
        };
        let look = debugger::Look {
            text: style::to_color(theme.syntax.text),
            muted: style::to_color(ui.text_muted),
            accent: style::to_color(ui.accent),
            error: style::to_color(ui.danger),
            highlight: style::to_color(ui.accent).scale_alpha(0.2),
        };
        session.view(&look, |p| self.display_path(p)).map(Message::Debug)
    }

    /// The CHANGES panel: open files show their tab's text (unsaved edits
    /// included), the others the text read on the last git refresh.
    fn changes_view(&self) -> Element<'_, Message> {
        let theme = self.current_theme();
        let ui = &theme.ui;
        let look = changes::Look {
            text: style::to_color(theme.syntax.text),
            muted: style::to_color(ui.text_muted),
            added: style::to_color(ui.success),
            removed: style::to_color(ui.danger),
            modified: style::to_color(ui.warning),
            conflicted: style::to_color(ui.danger),
        };
        let Some(project) = &self.project else {
            return changes::view(Vec::new(), &look, ui);
        };
        let files = project
            .git_files
            .iter()
            .filter(|f| self.changes_focus.as_ref().is_none_or(|p| uri::same_path(p, &f.path)))
            .map(|file| {
                let tab = self.tabs.iter().find(|t| uri::same_path(&t.path, &file.path));
                let (new, hunks, unsaved) = match (tab, &file.current) {
                    (Some(tab), _) => {
                        let hunks = tab.git.hunks(&tab.buffer).to_vec();
                        (Some(Cow::Owned(tab.buffer.text())), Cow::Owned(hunks), tab.dirty)
                    }
                    (None, Some((text, hunks))) => (Some(Cow::Borrowed(text.as_str())), Cow::Borrowed(hunks.as_slice()), false),
                    (None, None) => (None, Cow::Borrowed(&[][..]), false),
                };
                changes::FileView {
                    path: &file.path,
                    label: self.display_path(&file.path),
                    status: file.status,
                    old: file.base.as_deref(),
                    new,
                    hunks,
                    unsaved,
                }
            })
            // Changes already all staged: nothing left against the index.
            .filter(|f| !(f.status == git::FileStatus::Modified && f.new.is_some() && f.hunks.is_empty()))
            .collect();
        changes::view(files, &look, ui)
    }

    /// The terminal tabs in the panel header: one per shell (with its title,
    /// and × to end it), then + for another.
    fn terminal_tabs(&self) -> Element<'_, Message> {
        let ui = &self.current_theme().ui;
        let muted = style::to_color(ui.text_muted);
        let single = self.terminals.tabs.len() == 1;
        let mut tabs = Row::new().spacing(2).align_y(Alignment::Center);
        for (index, tab) in self.terminals.tabs.iter().enumerate() {
            let title = tab.title.as_deref().unwrap_or("shell");
            // Long titles (full paths) are cut from the left: the end says most.
            let short: String = match title.chars().count() {
                n if n > 28 => format!("…{}", title.chars().skip(n - 27).collect::<String>()),
                _ => title.to_string(),
            };
            if single {
                // One shell: just its title, as before tabs existed.
                tabs = tabs.push(text(short).size(12).color(muted));
                continue;
            }
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if index == self.terminals.active { button::secondary } else { button::text };
            let select = button(text(format!("{} {short}", index + 1)).size(12))
                .on_press(Message::SelectTerminal(tab.id))
                .style(style)
                .padding([2, 6]);
            let close = button(text("×").size(12)).on_press(Message::CloseTerminal(tab.id)).style(button::text).padding([2, 4]);
            let chip = row![select, style::tip(close, "End this shell", tooltip::Position::Top, ui)];
            tabs = tabs.push(mouse_area(chip).on_middle_release(Message::CloseTerminal(tab.id)));
        }
        let add = button(text("+").size(12)).on_press(Message::NewTerminal).style(button::text).padding([2, 6]);
        tabs.push(style::tip(add, "Another shell, in a new tab", tooltip::Position::Top, ui)).into()
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

    /// The tabs of one side of the split.
    fn tab_bar(&self, group: usize) -> Element<'_, Message> {
        let has_class_ref = self.class_ref.is_some() && group == self.class_ref_side;
        let class_ref_shown = has_class_ref && self.class_ref_shown;
        let shown = self.group_active[group].filter(|_| !class_ref_shown);
        let tabs = self.tabs.iter().enumerate().filter(|(_, tab)| tab.group == group).map(|(index, tab)| {
            let marker = match (tab.dirty, tab.disk_changed) {
                (_, true) => " (changed on disk)",
                (true, false) => " *",
                (false, false) => "",
            };
            let label = format!("{}{marker}", tab.title());
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if shown == Some(index) {
                    button::secondary
                } else {
                    button::text
                };
            let tab = row![
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
            ];
            let tab = container(tab).padding(1).style(style::tab_outline(&self.current_theme().ui, shown == Some(index)));
            // Middle click closes, as in browsers.
            let tab = mouse_area(tab).on_middle_release(Message::CloseTab(index));
            context_area(tab, move |point| Message::ContextMenu(ContextTarget::Tab(index), point)).into()
        });
        let mut tabs: Vec<Element<'_, Message>> = tabs.collect();
        if let Some(page) = self.class_ref.as_ref().filter(|_| has_class_ref) {
            let label = match page.page.as_ref() {
                Some(page) => format!("Docs: {}", page.name),
                None => "Class Reference".to_string(),
            };
            let style: fn(&iced::Theme, button::Status) -> button::Style =
                if class_ref_shown { button::secondary } else { button::text };
            let tab = row![
                button(text(label).size(13).font(iced::Font { style: iced::font::Style::Italic, ..iced::Font::default() }))
                    .on_press(Message::ShowClassRef)
                    .style(style)
                    .padding([4, 10]),
                style::tip(
                    button(text("×").size(13))
                        .on_press(Message::ClassRef(ClassRefMessage::Close))
                        .style(button::text)
                        .padding([4, 6]),
                    "Close (Ctrl+W)",
                    tooltip::Position::Bottom,
                    &self.current_theme().ui,
                ),
            ];
            let tab = container(tab).padding(1).style(style::tab_outline(&self.current_theme().ui, class_ref_shown));
            let tab = mouse_area(tab).on_middle_release(Message::ClassRef(ClassRefMessage::Close));
            tabs.push(context_area(tab, |point| Message::ContextMenu(ContextTarget::ClassRef, point)).into());
        }

        // Room around the tabs so their outlines aren't cut by the bar's edges.
        container(Row::with_children(tabs).spacing(4))
            .padding([3, 4])
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
                let label = button(text(format!("Godot {version}")).size(12))
                    .on_press(Message::ChooseGodot)
                    .style(button::text)
                    .padding([0, 4]);
                let tip = match &self.godot_choice {
                    Some(path) => format!("Saved with Godot {version}; using {}. Click to choose another", path.display()),
                    None => format!("Saved with Godot {version}. Click to choose which installed Godot to use"),
                };
                left.push(style::tip(label, tip, tooltip::Position::Top, ui));
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
            let editor = button(text("Godot").size(12)).on_press(Message::OpenInGodot).style(button::text).padding([0, 4]);
            let description = "Open the project in the Godot editor, at the current scene";
            left.push(style::tip(editor, description, tooltip::Position::Top, ui));
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


/// How often the session is saved while working (when it changed).
const AUTOSAVE_SECONDS: u64 = 30;

fn autosave_later() -> Task<Message> {
    Task::perform(sleep(std::time::Duration::from_secs(AUTOSAVE_SECONDS)), |()| Message::AutosaveSession)
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

/// UTF-8 without NUL bytes: worth opening in the editor.
fn is_text_file(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| !text.contains('\0'))
}

/// What a key-mapped command does (ids from `keymap::COMMANDS`).
fn command_message(id: &str) -> Option<Message> {
    let edit = |command: Command| Message::Edit(EditorEvent::Command(command));
    Some(match id {
        "file.save" => Message::Save,
        "file.openFolder" => Message::OpenFolderDialog,
        "file.closeTab" => Message::CloseActiveTab,
        "file.quickOpen" => Message::OpenPalette(String::new()),
        "palette.commands" => Message::OpenPalette(">".to_string()),
        "go.line" => Message::OpenPalette(":".to_string()),
        "go.symbol" => Message::OpenPalette("@".to_string()),
        "go.projectSymbol" => Message::OpenPalette("#".to_string()),
        "go.definition" => Message::GotoDefinition,
        "go.references" => Message::FindReferences,
        "refactor.rename" => Message::StartRename,
        "find.find" => Message::OpenFind { replace: false },
        "find.replace" => Message::OpenFind { replace: true },
        "find.next" => Message::Find(FindMessage::Next),
        "find.previous" => Message::Find(FindMessage::Previous),
        "search.project" => Message::OpenSearch,
        "run.project" => Message::RunProject,
        "run.scene" => Message::RunScene,
        "run.stop" => Message::StopRun,
        "debug.start" => Message::StartDebug { scene: false },
        "debug.startScene" => Message::StartDebug { scene: true },
        "debug.toggleBreakpoint" => Message::ToggleBreakpoint,
        "debug.stepOver" => Message::Debug(debugger::DebugMessage::Step("next")),
        "debug.stepInto" => Message::Debug(debugger::DebugMessage::Step("stepIn")),
        "debug.stepOut" => Message::Debug(debugger::DebugMessage::Step("stepOut")),
        "view.terminal" => Message::ToggleTerminal,
        "view.split" => Message::SplitMove,
        "view.shortcuts" => Message::OpenShortcuts,
        "help.classReference" => Message::OpenClassRef(None, None),
        "view.zoomIn" => Message::Zoom(1),
        "view.zoomOut" => Message::Zoom(-1),
        "view.zoomReset" => Message::ZoomReset,
        "edit.undo" => edit(Command::Undo),
        "edit.redo" => edit(Command::Redo),
        "edit.selectAll" => edit(Command::SelectAll),
        "edit.toggleComment" => edit(Command::ToggleComment),
        "edit.deleteLine" => edit(Command::DeleteLines),
        "edit.moveLineUp" => edit(Command::MoveLinesUp),
        "edit.moveLineDown" => edit(Command::MoveLinesDown),
        "edit.duplicateLine" => edit(Command::DuplicateLines),
        "edit.cursorAbove" => edit(Command::AddCursorAbove),
        "edit.cursorBelow" => edit(Command::AddCursorBelow),
        "edit.selectNextOccurrence" => edit(Command::SelectNextOccurrence),
        "edit.completion" => Message::Edit(EditorEvent::TriggerCompletion),
        "edit.fold" => edit(Command::ToggleFoldAtCursor),
        "edit.format" => Message::Edit(EditorEvent::FormatDocument),
        _ => return None,
    })
}

/// A theme from a file: a VS Code theme (`.json`), Godot's editor settings
/// (`editor_settings-4.x.tres`) or one of ours (`.toml`).
fn read_theme_file(path: &Path) -> Result<theme::Theme, String> {
    let vscode = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("jsonc"));
    if vscode {
        theme::vscode::import(path)
    } else if theme::godot_editor::version_of(path).is_some() {
        theme::godot_editor::import(path)
    } else {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        theme::Theme::from_toml(&text).map_err(|e| e.to_string())
    }
}

/// Shows a file (selected) or folder in the system file manager.
fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        if path.is_file() {
            // `/select,` must be glued to the path in one argument.
            c.arg(format!("/select,{}", path.display()));
        } else {
            c.arg(path);
        }
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if path.is_file() { path.parent().unwrap_or(path) } else { path });
        c
    };
    command.spawn().map(drop)
}

/// Opens a file with the operating system's default application.
fn open_external(path: &Path) -> std::io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(path).spawn().map(drop)
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

#[cfg(test)]
mod terminal_tab_tests {
    use super::Terminals;

    fn ids(terminals: &Terminals) -> Vec<u64> {
        terminals.tabs.iter().map(|t| t.id).collect()
    }

    #[test]
    fn closing_a_tab_keeps_a_sensible_one_active() {
        let mut terminals = Terminals::default();
        for _ in 0..3 {
            terminals.add();
        }
        assert_eq!((ids(&terminals), terminals.active), (vec![1, 2, 3], 2));
        // Closing one before the active tab keeps showing the same shell.
        terminals.active = 2;
        terminals.remove(1);
        assert_eq!(terminals.active().map(|t| t.id), Some(3));
        // Closing the active last tab shows the one before it.
        terminals.remove(3);
        assert_eq!(terminals.active().map(|t| t.id), Some(2));
        terminals.remove(2);
        assert!(terminals.active().is_none());
        // Ids are never reused.
        terminals.add();
        assert_eq!(ids(&terminals), [4]);
    }
}

/// The theme to show with the OS in `kind` mode: the one chosen for it,
/// else Godot Dark / Godot Light, else any theme of that kind.
fn theme_for_mode(themes: &[theme::Theme], kind: theme::ThemeKind, chosen: Option<&str>) -> Option<usize> {
    let default = match kind {
        theme::ThemeKind::Dark => "Godot Dark",
        theme::ThemeKind::Light => "Godot Light",
    };
    let named = |name: &str| themes.iter().position(|t| t.name == name);
    chosen.and_then(named).or_else(|| named(default)).or_else(|| themes.iter().position(|t| t.kind == kind))
}

#[cfg(test)]
mod os_theme_tests {
    use super::theme_for_mode;
    use theme::ThemeKind;

    #[test]
    fn the_chosen_theme_wins_then_the_builtin_of_that_mode() {
        let mut themes = theme::builtin();
        let mut mine = themes[0].clone();
        mine.name = "Mine".to_string();
        mine.kind = ThemeKind::Light;
        themes.push(mine);
        let name = |index: Option<usize>| index.map(|i| themes[i].name.clone());
        assert_eq!(name(theme_for_mode(&themes, ThemeKind::Light, Some("Mine"))).as_deref(), Some("Mine"));
        assert_eq!(name(theme_for_mode(&themes, ThemeKind::Light, None)).as_deref(), Some("Godot Light"));
        // A chosen theme that was deleted: back to the built-in one.
        assert_eq!(name(theme_for_mode(&themes, ThemeKind::Dark, Some("Gone"))).as_deref(), Some("Godot Dark"));
        // Without the built-in: any theme of that kind.
        themes.retain(|t| t.name != "Godot Light");
        let found = theme_for_mode(&themes, ThemeKind::Light, None).map(|i| themes[i].name.as_str());
        assert_eq!(found, Some("Mine"));
    }
}
