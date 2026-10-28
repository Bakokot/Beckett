//! Code editor widget: draws an `editor_core::Buffer` on a monospace grid
//! and turns input into [`EditorEvent`]s. The buffer and popup contents live
//! in app state; this widget only keeps view state (focus, scroll, drag,
//! hover timing).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use editor_core::buffer::{display_width, TAB_WIDTH};
use editor_core::gdscript::{Token, TokenKind};
use editor_core::folding::LineMap;
use editor_core::diff::{ChangeKind, LineChange};
use editor_core::{Buffer, Command, Motion, Pos};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self, Paragraph as _, Renderer as _};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::input_method::{self, InputMethod};
use iced::advanced::{clipboard, mouse, Clipboard, Renderer as _, Shell};
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::time::{Duration, Instant};
use iced::{
    alignment, window, Border, Color, Element, Event, Font, Length, Pixels, Point, Rectangle,
    Size, Theme, Vector,
};

use crate::docs::Doc;
use crate::keymap::{Chord, Keymap};
use crate::style::to_color;

const BLINK_MILLIS: u128 = 500;
const COMPLETION_ROWS: usize = 10;
const DOC_FONT_SIZE: f32 = 13.0;
const POPUP_PADDING: f32 = 8.0;
/// Width of the kind-tag column in the completion list (`const` + a space).
const TAG_COLUMNS: usize = 6;

/// Behaviors the user can turn off or tune (settings).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditorOptions {
    pub cursor_blink: bool,
    /// Diagnostic messages written at the end of their line.
    pub inline_diagnostics: bool,
    pub indent_guides: bool,
    /// How long the mouse rests on a word before its documentation shows.
    pub hover_delay: Duration,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self { cursor_blink: true, inline_diagnostics: true, indent_guides: true, hover_delay: Duration::from_millis(500) }
    }
}

#[derive(Debug, Clone)]
pub enum EditorEvent {
    Command(Command),
    Completion(CompletionAction),
    /// Ctrl+Space.
    TriggerCompletion,
    /// The mouse rested on the word at this position.
    Hover(Pos),
    /// The mouse left the hovered word.
    HoverEnd,
    /// Clicked while another panel had keyboard focus.
    Focused,
    /// Right-click at this window position: open the editor menu.
    ContextMenu(Point),
    /// A key bound to this command in the key map was pressed.
    Shortcut(&'static str),
    /// Shift+Alt+F.
    FormatDocument,
    /// Ctrl+click: go to the definition (or open the path) under the click.
    GotoDefinition,
    /// Ctrl is held with the mouse over this char (`None`: released or moved
    /// away); the app answers with the link range to underline, if any.
    LinkHover(Option<usize>),
    /// Tab / Shift+Tab while a snippet is being filled in.
    NextTabStop,
    PreviousTabStop,
    /// A file dragged from the explorer was dropped at this position (with
    /// Ctrl held or not).
    DropFile { path: PathBuf, pos: Pos, ctrl: bool },
    /// A git change marker in the gutter was clicked.
    GitMarker,
    /// A line number was clicked: add or remove a breakpoint on that line.
    ToggleBreakpoint(usize),
}

#[derive(Debug, Clone, Copy)]
pub enum CompletionAction {
    Move(isize),
    Accept,
    /// Clicked row (index into the visible list).
    AcceptIndex(usize),
    Dismiss,
}

/// An underline over a char-column range (e.g. a diagnostic), with an
/// optional message shown faded after the end of the line.
#[derive(Debug, Clone)]
pub struct Mark {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub color: Color,
    pub message: Option<String>,
}

/// Opacity of inline diagnostic messages.
const INLINE_MESSAGE_ALPHA: f32 = 0.55;
/// Columns between the end of a line and its inline message.
const INLINE_MESSAGE_GAP: usize = 4;

#[derive(Debug, Clone)]
pub struct CompletionRow {
    pub label: String,
    /// Short kind tag, e.g. `fn`, `var`.
    pub tag: &'static str,
    pub color: Color,
}

pub struct CompletionPopup<'a> {
    pub rows: &'a [CompletionRow],
    pub selected: usize,
    /// Where the completed word starts.
    pub anchor: Pos,
    pub doc: Option<&'a Doc>,
}

pub struct HoverPopup<'a> {
    pub pos: Pos,
    pub doc: &'a Doc,
}

pub struct SignaturePopup<'a> {
    pub label: &'a str,
    /// Byte range of the active parameter in `label`.
    pub active: Option<Range<usize>>,
    pub anchor: Pos,
}

#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub background: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub gutter: Color,
    pub gutter_active: Color,
    pub current_line: Color,
    pub selection: Color,
    pub cursor: Color,
    pub bracket: Color,
    pub popup: Color,
    pub popup_border: Color,
    pub find_match: Color,
    pub find_current: Color,
    pub indent_guide: Color,
    /// Gutter markers of lines changed since the staged text.
    pub git_added: Color,
    pub git_modified: Color,
    pub git_deleted: Color,
    pub breakpoint: Color,
    /// Background of the line the debugger is paused on, and its marker.
    pub paused_line: Color,
    pub paused_marker: Color,
    pub syntax: theme::SyntaxColors,
}

impl Colors {
    pub fn from_theme(theme: &theme::Theme) -> Self {
        let ui = &theme.ui;
        let text = to_color(ui.text);
        let accent = to_color(ui.accent);
        Self {
            background: to_color(ui.background),
            text: to_color(theme.syntax.text),
            muted: to_color(ui.text_muted),
            accent,
            gutter: to_color(ui.text_muted).scale_alpha(0.7),
            gutter_active: text,
            current_line: text.scale_alpha(0.05),
            selection: accent.scale_alpha(0.35),
            cursor: text,
            bracket: text.scale_alpha(0.5),
            popup: to_color(ui.surface),
            popup_border: to_color(ui.border),
            find_match: to_color(ui.warning).scale_alpha(0.22),
            find_current: to_color(ui.warning),
            indent_guide: text.scale_alpha(0.12),
            git_added: to_color(ui.success),
            git_modified: accent,
            git_deleted: to_color(ui.danger),
            breakpoint: to_color(ui.danger),
            paused_line: to_color(ui.warning).scale_alpha(0.22),
            paused_marker: to_color(ui.warning),
            syntax: theme.syntax,
        }
    }

    pub fn token(&self, kind: Option<TokenKind>) -> Color {
        let s = &self.syntax;
        to_color(match kind {
            None => s.text,
            Some(TokenKind::Keyword) => s.keyword,
            Some(TokenKind::ControlFlow) => s.control_flow,
            Some(TokenKind::Type) => s.r#type,
            Some(TokenKind::Function) => s.function,
            Some(TokenKind::Annotation) => s.annotation,
            Some(TokenKind::String) => s.string,
            Some(TokenKind::StringName) => s.string_name,
            Some(TokenKind::NodePath) => s.node_path,
            Some(TokenKind::Number) => s.number,
            Some(TokenKind::Comment) => s.comment,
            Some(TokenKind::Constant) => s.constant,
            Some(TokenKind::Symbol) => s.symbol,
        })
    }
}

pub struct CodeEditor<'a, Message> {
    buffer: &'a Buffer,
    on_event: Box<dyn Fn(EditorEvent) -> Message + 'a>,
    marks: &'a [Mark],
    colors: Colors,
    font: CodeFont,
    completion: Option<CompletionPopup<'a>>,
    hover: Option<HoverPopup<'a>>,
    signature: Option<SignaturePopup<'a>>,
    /// Sorted char ranges to highlight (search matches) and the current one.
    highlights: &'a [(usize, usize)],
    current_highlight: Option<usize>,
    keymap: Option<&'a Keymap>,
    /// Whether this editor owns keyboard focus in the app.
    focused: bool,
    /// A snippet is being filled in: Tab moves between its stops.
    tab_stops: bool,
    /// Char range underlined while Ctrl is held: a Ctrl+click target.
    link: Option<(usize, usize)>,
    /// A file being dragged over the app, which can be dropped here.
    dropping: Option<&'a Path>,
    /// Lines changed since the staged text, marked in the gutter.
    git_changes: Rc<Vec<LineChange>>,
    /// Lines (zero-based, in order) with a breakpoint; `None` in editors
    /// that take none.
    breakpoints: Option<Vec<usize>>,
    /// The line the debugger is paused on.
    paused_line: Option<usize>,
    options: EditorOptions,
}

impl<'a, Message> CodeEditor<'a, Message> {
    pub fn new(
        buffer: &'a Buffer,
        colors: Colors,
        on_event: impl Fn(EditorEvent) -> Message + 'a,
    ) -> Self {
        Self {
            buffer,
            on_event: Box::new(on_event),
            marks: &[],
            colors,
            font: CodeFont::default(),
            completion: None,
            hover: None,
            signature: None,
            highlights: &[],
            current_highlight: None,
            focused: true,
            tab_stops: false,
            keymap: None,
            link: None,
            dropping: None,
            git_changes: Rc::default(),
            breakpoints: None,
            paused_line: None,
            options: EditorOptions::default(),
        }
    }

    pub fn breakpoints(mut self, lines: Option<Vec<usize>>, paused_line: Option<usize>) -> Self {
        self.breakpoints = lines;
        self.paused_line = paused_line;
        self
    }

    pub fn git_changes(mut self, changes: Rc<Vec<LineChange>>) -> Self {
        self.git_changes = changes;
        self
    }

    pub fn dropping(mut self, path: Option<&'a Path>) -> Self {
        self.dropping = path;
        self
    }

    /// Key shortcuts the editor turns into `Shortcut` events (none: the
    /// editor ignores shortcut keys, as in the theme preview).
    pub fn keymap(mut self, keymap: &'a Keymap) -> Self {
        self.keymap = Some(keymap);
        self
    }

    pub fn tab_stops(mut self, active: bool) -> Self {
        self.tab_stops = active;
        self
    }

    pub fn link(mut self, range: Option<(usize, usize)>) -> Self {
        self.link = range;
        self
    }

    pub fn highlights(mut self, ranges: &'a [(usize, usize)], current: Option<usize>) -> Self {
        self.highlights = ranges;
        self.current_highlight = current;
        self
    }

    pub fn font(mut self, font: CodeFont) -> Self {
        self.font = font;
        self
    }

    pub fn options(mut self, options: EditorOptions) -> Self {
        self.options = options;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn marks(mut self, marks: &'a [Mark]) -> Self {
        self.marks = marks;
        self
    }

    pub fn completion(mut self, popup: Option<CompletionPopup<'a>>) -> Self {
        self.completion = popup.filter(|p| !p.rows.is_empty());
        self
    }

    pub fn hover(mut self, popup: Option<HoverPopup<'a>>) -> Self {
        self.hover = popup.filter(|p| !p.doc.is_empty());
        self
    }

    pub fn signature(mut self, popup: Option<SignaturePopup<'a>>) -> Self {
        self.signature = popup;
        self
    }

    fn emit(&self, shell: &mut Shell<'_, Message>, event: EditorEvent) {
        shell.publish((self.on_event)(event));
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Metrics {
    pub(crate) font: Font,
    pub(crate) font_size: f32,
    pub(crate) char_width: f32,
    pub(crate) line_height: f32,
    /// Shape all text, so the font's ligatures apply.
    pub(crate) ligatures: bool,
}

/// The code font as the editor and the terminal draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodeFont {
    pub font: Font,
    pub size: f32,
    /// Line height as a multiple of `size`.
    pub line_height: f32,
    pub ligatures: bool,
}

impl CodeFont {
    pub fn from_settings(settings: &theme::FontSettings) -> Self {
        let settings = settings.sanitized();
        Self {
            font: settings.family.as_deref().map_or(Font::MONOSPACE, font_family),
            size: settings.size,
            line_height: settings.line_height,
            ligatures: settings.ligatures,
        }
    }
}

impl Default for CodeFont {
    fn default() -> Self {
        Self::from_settings(&theme::FontSettings::default())
    }
}

/// A font by family name. iced wants `&'static str` names, so each distinct
/// name is leaked once and reused.
fn font_family(name: &str) -> Font {
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let name = match names.iter().find(|n| **n == name) {
        Some(known) => *known,
        None => {
            let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
            names.push(leaked);
            leaked
        }
    };
    Font { family: iced::font::Family::Name(name), ..Font::MONOSPACE }
}

/// Tracks the mouse resting over text, for hover requests.
#[derive(Debug, Clone, Copy)]
struct HoverTrack {
    point: Point,
    since: Instant,
    /// Word range (char indices) a hover was requested for.
    sent: Option<(usize, usize)>,
}

struct State {
    focused: bool,
    window_focused: bool,
    focused_at: Instant,
    now: Instant,
    scroll: Vector,
    dragging: bool,
    last_click: Option<mouse::Click>,
    modifiers: Modifiers,
    seen_reveal: u64,
    metrics: Option<Metrics>,
    hover: Option<HoverTrack>,
    /// Font the metrics were measured for.
    font: Option<CodeFont>,
    /// Mouse position over the editor, for Ctrl+hover links.
    mouse: Option<Point>,
    /// Word range (char indices) last reported with `LinkHover`.
    link_sent: Option<(usize, usize)>,
    /// Input method composition in progress: text and its highlighted range.
    preedit: Option<(String, Option<std::ops::Range<usize>>)>,
    /// Alt+drag: the (line, visual column) where the box started.
    box_anchor: Option<(usize, usize)>,
    /// The Alt+drag left its starting cell, so it is a box, not a click.
    box_dragged: bool,
}

impl State {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            focused: true,
            window_focused: true,
            focused_at: now,
            now,
            scroll: Vector::ZERO,
            dragging: false,
            last_click: None,
            modifiers: Modifiers::default(),
            seen_reveal: u64::MAX,
            metrics: None,
            hover: None,
            font: None,
            mouse: None,
            link_sent: None,
            box_anchor: None,
            preedit: None,
            box_dragged: false,
        }
    }

    fn cursor_visible(&self, blink: bool) -> bool {
        self.focused
            && self.window_focused
            && (!blink || (self.now.saturating_duration_since(self.focused_at).as_millis() / BLINK_MILLIS).is_multiple_of(2))
    }
}

/// Screen-space layout of the editor for one frame.
struct Geometry {
    bounds: Rectangle,
    text_area: Rectangle,
    /// Screen x of visual column 0 (before horizontal scroll).
    text_x: f32,
    char_width: f32,
    line_height: f32,
    /// Which lines are shown (folded ones are not).
    map: LineMap,
}

/// Columns of the gutter taken by the fold markers, right of the numbers.
const FOLD_COLUMNS: f32 = 2.0;
/// Columns left of the numbers, for breakpoint dots.
const BREAKPOINT_COLUMNS: f32 = 1.5;

impl Geometry {
    fn new(buffer: &Buffer, bounds: Rectangle, metrics: Metrics) -> Self {
        let digits = buffer.len_lines().to_string().len().max(3) as f32;
        let gutter = (BREAKPOINT_COLUMNS + digits + 1.0 + FOLD_COLUMNS) * metrics.char_width;
        let text_area = Rectangle {
            x: bounds.x + gutter,
            width: (bounds.width - gutter).max(0.0),
            ..bounds
        };
        Self {
            bounds,
            text_area,
            text_x: text_area.x + metrics.char_width * 0.5,
            char_width: metrics.char_width,
            line_height: metrics.line_height,
            map: buffer.line_map(),
        }
    }

    fn line_y(&self, line: usize, scroll: Vector) -> f32 {
        self.bounds.y + self.map.row_of(line) as f32 * self.line_height - scroll.y
    }

    /// The line shown at screen `y`.
    fn line_at_y(&self, y: f32, scroll: Vector) -> usize {
        let row = ((y - self.bounds.y + scroll.y) / self.line_height).max(0.0) as usize;
        self.map.line_at(row)
    }

    /// The fold marker column, left of the text.
    fn fold_column(&self) -> Rectangle {
        let width = FOLD_COLUMNS * self.char_width;
        Rectangle { x: self.text_area.x - width, width, ..self.bounds }
    }

    /// Where the "..." marker of a folded line is drawn.
    fn fold_marker(&self, buffer: &Buffer, line: usize, scroll: Vector) -> Rectangle {
        let end = buffer.visual_col(line, buffer.line_len(line));
        let x = self.col_x(end + 1, scroll);
        Rectangle::new(Point::new(x, self.line_y(line, scroll) + 2.0), Size::new(self.char_width * 3.0 + 4.0, self.line_height - 4.0))
    }

    fn col_x(&self, visual: usize, scroll: Vector) -> f32 {
        self.text_x + visual as f32 * self.char_width - scroll.x
    }

    fn page_lines(&self) -> usize {
        ((self.bounds.height / self.line_height) as usize).saturating_sub(1).max(1)
    }

    /// Places a popup of `size` below the line at `anchor` (or above it when
    /// there is no room), kept inside the editor horizontally.
    fn popup_at(&self, anchor: Point, size: Size, above_first: bool) -> Rectangle {
        let below = anchor.y + self.line_height + 2.0;
        let above = anchor.y - size.height - 2.0;
        let fits_below = below + size.height <= self.bounds.y + self.bounds.height;
        let fits_above = above >= self.bounds.y;
        let y = match (above_first, fits_above, fits_below) {
            (true, true, _) | (false, true, false) => above,
            _ => below,
        };
        let max_x = self.bounds.x + self.bounds.width - size.width;
        Rectangle::new(Point::new(anchor.x.min(max_x).max(self.bounds.x), y), size)
    }
}

/// Completion list layout: the list box, the first visible row and the
/// optional docs box beside it.
struct CompletionLayout {
    list: Rectangle,
    first: usize,
    docs: Option<Rectangle>,
}

enum KeyAction {
    Event(EditorEvent),
    Copy,
    Cut,
    Paste,
}

impl<'a, Message> CodeEditor<'a, Message> {
    /// Whether `point` (on `line`) is on a git change marker: the strip at
    /// the gutter's right edge, beside a changed line or a removal wedge.
    /// Whether `point` is on the line numbers, where a click toggles a
    /// breakpoint (in editors that take breakpoints).
    fn on_line_numbers(&self, geometry: &Geometry, point: Point) -> bool {
        self.breakpoints.is_some() && geometry.bounds.contains(point) && point.x < geometry.fold_column().x
    }

    fn on_git_marker(&self, geometry: &Geometry, point: Point, line: usize) -> bool {
        let strip = point.x >= geometry.text_area.x - geometry.char_width * 0.8 && point.x < geometry.text_area.x;
        strip
            && self.git_changes.iter().any(|c| match c.kind {
                ChangeKind::Deleted => c.lines.start == line || c.lines.start == line + 1,
                ChangeKind::Added | ChangeKind::Modified => c.lines.contains(&line),
            })
    }

    fn pos_at(&self, geometry: &Geometry, scroll: Vector, point: Point) -> Pos {
        let line = geometry.line_at_y(point.y, scroll);
        let visual = ((point.x - geometry.text_x + scroll.x) / geometry.char_width).round().max(0.0);
        Pos { line, column: self.buffer.column_for_visual(line, visual as usize) }
    }

    /// (line, visual column) of the gap nearest to `point`, past line ends
    /// too, for box selection.
    fn grid_at(&self, geometry: &Geometry, scroll: Vector, point: Point) -> (usize, usize) {
        let line = geometry.line_at_y(point.y, scroll);
        let visual = ((point.x - geometry.text_x + scroll.x) / geometry.char_width).round().max(0.0);
        (line, visual as usize)
    }

    /// Char index of the character under `point` (not rounded to the nearest gap).
    fn char_under(&self, geometry: &Geometry, scroll: Vector, point: Point) -> usize {
        let line = geometry.line_at_y(point.y, scroll);
        let visual = ((point.x - geometry.text_x + scroll.x) / geometry.char_width).floor().max(0.0);
        let column = self.buffer.column_for_visual(line, visual as usize);
        self.buffer.pos_to_char(Pos { line, column })
    }

    fn screen_point(&self, geometry: &Geometry, scroll: Vector, pos: Pos) -> Point {
        let visual = self.buffer.visual_col(pos.line, pos.column);
        Point::new(geometry.col_x(visual, scroll), geometry.line_y(pos.line, scroll))
    }

    /// Rectangles covering a char range, one per visible line; a range that
    /// continues past a line end also covers the line break.
    fn range_rects(
        &self,
        g: &Geometry,
        scroll: Vector,
        start: usize,
        end: usize,
        first: usize,
        last: usize,
    ) -> Vec<Rectangle> {
        let buffer = self.buffer;
        let start = buffer.char_to_pos(start);
        let end = buffer.char_to_pos(end);
        (start.line.max(first)..=end.line.min(last.saturating_sub(1)))
            .filter(|&line| !g.map.is_hidden(line))
            .map(|line| {
                let from = if line == start.line { buffer.visual_col(line, start.column) } else { 0 };
                let to = if line == end.line {
                    buffer.visual_col(line, end.column)
                } else {
                    buffer.visual_col(line, buffer.line_len(line)) + 1
                };
                Rectangle::new(
                    Point::new(g.col_x(from, scroll), g.line_y(line, scroll)),
                    Size::new(to.saturating_sub(from) as f32 * g.char_width, g.line_height),
                )
            })
            .collect()
    }

    /// Scrolls so the primary cursor is visible.
    fn reveal(&self, state: &mut State, geometry: &Geometry) {
        let pos = self.buffer.char_to_pos(self.buffer.primary().head);
        let lh = geometry.line_height;
        let y = geometry.map.row_of(pos.line) as f32 * lh;
        if y < state.scroll.y {
            state.scroll.y = y;
        } else if y + lh > state.scroll.y + geometry.bounds.height {
            state.scroll.y = y + lh - geometry.bounds.height;
        }
        let margin = geometry.char_width * 2.0;
        let x = self.buffer.visual_col(pos.line, pos.column) as f32 * geometry.char_width;
        let width = geometry.text_area.width - geometry.char_width;
        if x - margin < state.scroll.x {
            state.scroll.x = (x - margin).max(0.0);
        } else if x + margin > state.scroll.x + width {
            state.scroll.x = x + margin - width;
        }
        self.clamp_scroll(state, geometry);
    }

    fn clamp_scroll(&self, state: &mut State, geometry: &Geometry) {
        let max_y = geometry.map.rows().saturating_sub(1) as f32 * geometry.line_height;
        state.scroll.y = state.scroll.y.clamp(0.0, max_y);
        state.scroll.x = state.scroll.x.max(0.0);
    }

    fn completion_layout(
        &self,
        popup: &CompletionPopup<'_>,
        geometry: &Geometry,
        scroll: Vector,
        metrics: Metrics,
    ) -> CompletionLayout {
        let cw = geometry.char_width;
        let visible = popup.rows.len().min(COMPLETION_ROWS);
        let widest = popup.rows.iter().map(|r| r.label.chars().count()).max().unwrap_or(0);
        let width = ((widest + TAG_COLUMNS + 2).clamp(24, 64)) as f32 * cw;
        let size = Size::new(width, visible as f32 * geometry.line_height + 4.0);
        let anchor = self.screen_point(geometry, scroll, popup.anchor);
        let list = geometry.popup_at(Point::new(anchor.x - cw * (TAG_COLUMNS as f32 + 0.5), anchor.y), size, false);
        let first = popup.selected.saturating_sub(visible - 1);

        let docs = popup.doc.filter(|d| !d.is_empty()).and_then(|doc| {
            let right = geometry.bounds.x + geometry.bounds.width;
            let width = (right - list.x - list.width - 4.0).min(440.0);
            (width >= 220.0).then(|| {
                let height = doc_height(doc, width, metrics);
                Rectangle::new(Point::new(list.x + list.width + 4.0, list.y), Size::new(width, height))
            })
        });
        CompletionLayout { list, first, docs }
    }

    fn key_action(
        &self,
        key: &Key,
        modifiers: Modifiers,
        text: Option<&str>,
        page: usize,
    ) -> Option<KeyAction> {
        let (ctrl, shift, alt) = (modifiers.command(), modifiers.shift(), modifiers.alt());
        let event = |event| Some(KeyAction::Event(event));
        let command = |command| Some(KeyAction::Event(EditorEvent::Command(command)));
        let movement = |motion| command(Command::Move { motion, extend: shift });

        // Popup navigation takes precedence while one is open.
        if let Key::Named(named) = key.as_ref() {
            if self.completion.is_some() {
                let action = match named {
                    Named::ArrowUp => Some(CompletionAction::Move(-1)),
                    Named::ArrowDown => Some(CompletionAction::Move(1)),
                    Named::PageUp => Some(CompletionAction::Move(-(COMPLETION_ROWS as isize))),
                    Named::PageDown => Some(CompletionAction::Move(COMPLETION_ROWS as isize)),
                    Named::Enter | Named::Tab => Some(CompletionAction::Accept),
                    Named::Escape => Some(CompletionAction::Dismiss),
                    _ => None,
                };
                if let Some(action) = action {
                    return event(EditorEvent::Completion(action));
                }
            }
            if named == Named::Escape && (self.signature.is_some() || self.hover.is_some() || self.tab_stops) {
                return event(EditorEvent::Completion(CompletionAction::Dismiss));
            }
            if named == Named::Tab && self.tab_stops {
                return event(if modifiers.shift() { EditorEvent::PreviousTabStop } else { EditorEvent::NextTabStop });
            }
        }

        // Clipboard keys are fixed (they need the clipboard here).
        if let Key::Character(c) = key.as_ref() {
            if ctrl && !alt {
                match c.to_lowercase().as_str() {
                    "c" => return Some(KeyAction::Copy),
                    "x" => return Some(KeyAction::Cut),
                    "v" => return Some(KeyAction::Paste),
                    _ => {}
                }
            }
        }
        // Everything else bound in the (customizable) key map.
        if let (Some(keymap), Some(chord)) = (self.keymap, Chord::from_key(key, modifiers)) {
            if let Some(id) = keymap.lookup(&chord) {
                return event(EditorEvent::Shortcut(id));
            }
        }

        match key.as_ref() {
            Key::Named(named) => match named {
                Named::ArrowLeft => movement(if ctrl { Motion::WordLeft } else { Motion::Left }),
                Named::ArrowRight => movement(if ctrl { Motion::WordRight } else { Motion::Right }),
                Named::ArrowUp => movement(Motion::Up),
                Named::ArrowDown => movement(Motion::Down),
                Named::Home => movement(if ctrl { Motion::DocumentStart } else { Motion::Home }),
                Named::End => movement(if ctrl { Motion::DocumentEnd } else { Motion::End }),
                Named::PageUp => movement(Motion::PageUp(page)),
                Named::PageDown => movement(Motion::PageDown(page)),
                Named::Backspace => command(Command::Backspace { word: ctrl }),
                Named::Delete => command(Command::Delete { word: ctrl }),
                Named::Enter => command(Command::Newline),
                Named::Tab if shift => command(Command::Outdent),
                Named::Tab => command(Command::Indent),
                Named::Escape if self.buffer.selections().len() > 1 => {
                    command(Command::CollapseSelections)
                }
                Named::Space => command(Command::Insert(" ".to_string())),
                _ => None,
            },
            // An unbound Ctrl+key or Alt+key types nothing; Ctrl+Alt is AltGr
            // on many layouts, which does type.
            Key::Character(_) if ctrl != alt => None,
            _ => {
                let text = text?.chars().filter(|c| !c.is_control()).collect::<String>();
                (!text.is_empty()).then_some(KeyAction::Event(EditorEvent::Command(Command::Insert(text))))
            }
        }
    }

    /// Reports the char under the mouse while Ctrl is held (`None` when it
    /// isn't), once per word, so the app can find out whether it is a link.
    fn update_link(&self, state: &mut State, geometry: &Geometry, shell: &mut Shell<'_, Message>) {
        let index = state
            .mouse
            .filter(|p| state.modifiers.command() && !state.dragging && geometry.text_area.contains(*p))
            .map(|p| self.char_under(geometry, state.scroll, p));
        let word = index.map(|i| match self.buffer.word_range(i) {
            (start, end) if start < end => (start, end),
            _ => (i, i + 1),
        });
        if word != state.link_sent {
            state.link_sent = word;
            self.emit(shell, EditorEvent::LinkHover(index));
        }
    }

    /// Whether `point` is over the underlined link.
    fn over_link(&self, state: &State, geometry: &Geometry, point: Point) -> bool {
        let Some((start, end)) = self.link else {
            return false;
        };
        let index = self.char_under(geometry, state.scroll, point);
        state.modifiers.command() && geometry.text_area.contains(point) && (start..end).contains(&index)
    }

    fn update_hover(
        &self,
        state: &mut State,
        geometry: &Geometry,
        shell: &mut Shell<'_, Message>,
        point: Option<Point>,
    ) {
        let now = Instant::now();
        let sent = state.hover.and_then(|h| h.sent);
        let Some(point) = point.filter(|p| geometry.text_area.contains(*p)) else {
            if sent.is_some() {
                self.emit(shell, EditorEvent::HoverEnd);
            }
            state.hover = None;
            return;
        };
        let index = self.char_under(geometry, state.scroll, point);
        let word = self.buffer.word_range(index);
        match sent {
            Some(range) if range == word && word.0 != word.1 => {}
            _ => {
                if sent.is_some() {
                    self.emit(shell, EditorEvent::HoverEnd);
                }
                state.hover = Some(HoverTrack { point, since: now, sent: None });
                shell.request_redraw_at(now + self.options.hover_delay);
            }
        }
    }
}

impl<'a, Message> Widget<Message, Theme, iced::Renderer> for CodeEditor<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new())
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        if state.font != Some(self.font) {
            state.font = Some(self.font);
            state.metrics = Some(measure(self.font));
        }
        layout::Node::new(limits.resolve(Length::Fill, Length::Fill, Size::ZERO))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let Some(metrics) = state.metrics else {
            return;
        };
        let geometry = Geometry::new(self.buffer, layout.bounds(), metrics);

        if state.seen_reveal != self.buffer.reveal_version() {
            state.seen_reveal = self.buffer.reveal_version();
            self.reveal(state, &geometry);
            shell.request_redraw();
        }

        match event {
            Event::Window(window::Event::Unfocused) => state.window_focused = false,
            Event::Window(window::Event::Focused) => {
                state.window_focused = true;
                state.focused_at = Instant::now();
                shell.request_redraw();
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                state.now = *now;
                if state.focused && state.window_focused && self.options.cursor_blink {
                    let elapsed = now.saturating_duration_since(state.focused_at).as_millis();
                    let wait = BLINK_MILLIS - elapsed % BLINK_MILLIS;
                    shell.request_redraw_at(*now + Duration::from_millis(wait as u64));
                }
                // Input methods (CJK…): tell the system where the cursor is so
                // its candidate window sits next to it. The composition itself
                // is drawn by the editor, at the cursor.
                if state.focused && self.focused {
                    let pos = self.buffer.char_to_pos(self.buffer.primary().head);
                    let point = self.screen_point(&geometry, state.scroll, pos);
                    let cursor = Rectangle::new(point, Size::new(1.0, geometry.line_height));
                    shell.request_input_method(&InputMethod::<&str>::Enabled {
                        cursor,
                        purpose: input_method::Purpose::Normal,
                        preedit: None,
                    });
                }
                if let Some(track) = &mut state.hover {
                    if track.sent.is_none() && now.saturating_duration_since(track.since) >= self.options.hover_delay {
                        let index = self.char_under(&geometry, state.scroll, track.point);
                        let word = self.buffer.word_range(index);
                        track.sent = Some(word);
                        if word.0 != word.1 && !state.dragging {
                            let pos = self.buffer.char_to_pos(index);
                            shell.publish((self.on_event)(EditorEvent::Hover(pos)));
                        }
                    }
                }
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                // Pressing or releasing Ctrl shows or hides the link under the mouse.
                self.update_link(state, &geometry, shell);
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(point) = cursor.position_over(geometry.bounds) else {
                    state.focused = false;
                    return;
                };
                state.focused = true;
                state.focused_at = Instant::now();
                if !self.focused {
                    self.emit(shell, EditorEvent::Focused);
                }

                // Fold markers in the gutter, and the "..." of a folded line.
                let line = geometry.line_at_y(point.y, state.scroll);

                // Git change markers, at the gutter's right edge.
                if self.on_git_marker(&geometry, point, line) {
                    self.emit(shell, EditorEvent::GitMarker);
                    shell.capture_event();
                    return;
                }

                // Line numbers (and the margin left of them): a breakpoint.
                if self.on_line_numbers(&geometry, point) && line < self.buffer.len_lines() {
                    self.emit(shell, EditorEvent::ToggleBreakpoint(line));
                    shell.capture_event();
                    return;
                }
                let on_marker = self.buffer.is_folded(line)
                    && geometry.fold_marker(self.buffer, line, state.scroll).contains(point);
                if (geometry.fold_column().contains(point) && self.buffer.fold_range_at(line).is_some()) || on_marker {
                    self.emit(shell, EditorEvent::Command(Command::ToggleFold(line)));
                    shell.capture_event();
                    return;
                }

                // A click on the completion list accepts that row.
                if let Some(popup) = &self.completion {
                    let layout = self.completion_layout(popup, &geometry, state.scroll, metrics);
                    if layout.list.contains(point) {
                        let row = ((point.y - layout.list.y - 2.0) / geometry.line_height) as usize;
                        let index = (layout.first + row).min(popup.rows.len() - 1);
                        self.emit(shell, EditorEvent::Completion(CompletionAction::AcceptIndex(index)));
                        shell.capture_event();
                        return;
                    }
                }

                let click = mouse::Click::new(point, mouse::Button::Left, state.last_click);
                state.last_click = Some(click);
                let pos = self.pos_at(&geometry, state.scroll, point);
                let alt = state.modifiers.alt();
                let command = match click.kind() {
                    mouse::click::Kind::Single if alt => {
                        // Alt+click adds a cursor; dragging on turns it into a box.
                        state.box_anchor = Some(self.grid_at(&geometry, state.scroll, point));
                        Command::AddCursorAt(pos)
                    }
                    mouse::click::Kind::Single if state.modifiers.command() => {
                        self.emit(shell, EditorEvent::Command(Command::Click { pos, extend: false }));
                        self.emit(shell, EditorEvent::GotoDefinition);
                        shell.capture_event();
                        return;
                    }
                    mouse::click::Kind::Single => {
                        state.dragging = true;
                        Command::Click { pos, extend: state.modifiers.shift() }
                    }
                    mouse::click::Kind::Double => Command::SelectWordAt(pos),
                    mouse::click::Kind::Triple => Command::SelectLineAt(pos),
                };
                self.emit(shell, EditorEvent::Command(command));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(anchor) = state.box_anchor {
                    let head = self.grid_at(&geometry, state.scroll, *position);
                    if head != anchor || state.box_dragged {
                        state.box_dragged = true;
                        self.emit(shell, EditorEvent::Command(Command::BoxSelect { anchor, head }));
                    }
                    shell.capture_event();
                } else if state.dragging {
                    let pos = self.pos_at(&geometry, state.scroll, *position);
                    self.emit(shell, EditorEvent::Command(Command::Drag(pos)));
                    shell.capture_event();
                } else if self.dropping.is_some() {
                    // The drop caret follows the mouse.
                    shell.request_redraw();
                } else {
                    self.update_hover(state, &geometry, shell, cursor.position_over(geometry.bounds));
                }
                state.mouse = cursor.position_over(geometry.bounds);
                self.update_link(state, &geometry, shell);
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                self.update_hover(state, &geometry, shell, None);
                state.mouse = None;
                self.update_link(state, &geometry, shell);
            }
            // Right-click: move the cursor there (unless inside the selection,
            // which the menu acts on), then ask for the menu.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let Some(point) = cursor.position_over(geometry.text_area) else {
                    return;
                };
                let index = self.char_under(&geometry, state.scroll, point);
                let inside = self.buffer.selections().iter().any(|s| !s.is_empty() && (s.start()..=s.end()).contains(&index));
                if !inside {
                    let pos = self.pos_at(&geometry, state.scroll, point);
                    self.emit(shell, EditorEvent::Command(Command::Click { pos, extend: false }));
                }
                self.emit(shell, EditorEvent::ContextMenu(point));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let (Some(path), Some(point)) = (self.dropping, cursor.position_over(geometry.text_area)) {
                    let pos = self.pos_at(&geometry, state.scroll, point);
                    let ctrl = state.modifiers.command();
                    self.emit(shell, EditorEvent::DropFile { path: path.to_path_buf(), pos, ctrl });
                }
                state.dragging = false;
                state.box_anchor = None;
                state.box_dragged = false;
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(geometry.bounds) => {
                if let Some(popup) = &self.completion {
                    let layout = self.completion_layout(popup, &geometry, state.scroll, metrics);
                    if cursor.is_over(layout.list) {
                        let lines = match *delta {
                            mouse::ScrollDelta::Lines { y, .. } => y,
                            mouse::ScrollDelta::Pixels { y, .. } => y / geometry.line_height,
                        };
                        let step = if lines > 0.0 { -1 } else { 1 };
                        self.emit(shell, EditorEvent::Completion(CompletionAction::Move(step)));
                        shell.capture_event();
                        return;
                    }
                }
                let (mut dx, mut dy) = match *delta {
                    mouse::ScrollDelta::Lines { x, y } => {
                        (x * 3.0 * geometry.char_width, y * 3.0 * geometry.line_height)
                    }
                    mouse::ScrollDelta::Pixels { x, y } => (x, y),
                };
                if state.modifiers.shift() && dx == 0.0 {
                    (dx, dy) = (dy, 0.0);
                }
                state.scroll = state.scroll - Vector::new(dx, dy);
                self.clamp_scroll(state, &geometry);
                self.update_hover(state, &geometry, shell, None);
                self.update_link(state, &geometry, shell);
                shell.request_redraw();
                shell.capture_event();
            }
            Event::InputMethod(event) if state.focused && self.focused => {
                match event {
                    input_method::Event::Preedit(content, selection) if !content.is_empty() => {
                        state.preedit = Some((content.clone(), selection.clone()));
                    }
                    input_method::Event::Commit(content) => {
                        state.preedit = None;
                        if !content.is_empty() {
                            self.emit(shell, EditorEvent::Command(Command::Insert(content.clone())));
                        }
                    }
                    input_method::Event::Preedit(..) | input_method::Event::Opened | input_method::Event::Closed => {
                        state.preedit = None;
                    }
                }
                shell.request_redraw();
                shell.capture_event();
            }
            // Keys pressed while composing belong to the input method.
            Event::Keyboard(keyboard::Event::KeyPressed { .. }) if state.preedit.is_some() => {
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. })
                if state.focused && self.focused =>
            {
                let page = geometry.page_lines();
                let Some(action) = self.key_action(key, *modifiers, text.as_deref(), page) else {
                    return;
                };
                let event = match action {
                    KeyAction::Event(event) => Some(event),
                    KeyAction::Copy | KeyAction::Cut => {
                        let selected = self.buffer.selected_text();
                        if !selected.is_empty() {
                            clipboard.write(clipboard::Kind::Standard, selected);
                        }
                        matches!(action, KeyAction::Cut)
                            .then_some(EditorEvent::Command(Command::DeleteSelection))
                    }
                    KeyAction::Paste => clipboard
                        .read(clipboard::Kind::Standard)
                        .map(|text| EditorEvent::Command(Command::Paste(text))),
                };
                if let Some(event) = event {
                    self.emit(shell, event);
                }
                if state.hover.is_some_and(|h| h.sent.is_some()) {
                    self.emit(shell, EditorEvent::HoverEnd);
                }
                state.hover = None;
                state.focused_at = Instant::now();
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let Some(metrics) = state.metrics else {
            return;
        };
        let buffer = self.buffer;
        let colors = &self.colors;
        let g = Geometry::new(buffer, layout.bounds(), metrics);
        // The buffer may have shrunk since the last scroll update.
        let max_y = g.map.rows().saturating_sub(1) as f32 * g.line_height;
        let scroll = Vector::new(state.scroll.x, state.scroll.y.min(max_y));
        let lh = g.line_height;
        let cw = g.char_width;

        fill(renderer, g.bounds, colors.background);

        // Lines `first..last` cover the visible rows; folded ones among them
        // are skipped with `shown`.
        let first_row = (scroll.y / lh).floor() as usize;
        let visible = (g.bounds.height / lh).ceil() as usize + 1;
        let first = g.map.line_at(first_row);
        let last = (g.map.line_at(first_row + visible) + 1).min(buffer.len_lines());
        let shown = |line: usize| !g.map.is_hidden(line);
        let primary = buffer.primary();
        let primary_pos = buffer.char_to_pos(primary.head);
        let folded: Vec<usize> = buffer.folded().iter().map(|&(header, _)| header).collect();
        let mouse_in_gutter = state.mouse.is_some_and(|p| p.x < g.text_area.x && g.bounds.contains(p));

        // Gutter: line numbers, and fold markers (folded blocks always,
        // foldable ones while the mouse is over the gutter).
        renderer.with_layer(g.bounds, |r| {
            let fold_x = g.fold_column().x + cw * 0.4;
            for line in (first..last).filter(|&l| shown(l)) {
                let number = (line + 1).to_string();
                let x = fold_x - (number.len() as f32 + 0.6) * cw;
                let color = if line == primary_pos.line { colors.gutter_active } else { colors.gutter };
                let y = g.line_y(line, scroll);
                draw_text(r, number, Point::new(x, y), color, metrics, g.bounds);
                if folded.contains(&line) {
                    draw_text(r, ">".to_string(), Point::new(fold_x, y), colors.accent, metrics, g.bounds);
                } else if mouse_in_gutter && buffer.fold_range_at(line).is_some() {
                    draw_text(r, "v".to_string(), Point::new(fold_x, y), colors.gutter, metrics, g.bounds);
                }
            }

            // Breakpoints: red dots left of the numbers; the paused line's
            // marker over them.
            let dot = |r: &mut iced::Renderer, line: usize, size: Size, color: Color, radius: f32| {
                let center = Point::new(g.bounds.x + cw * BREAKPOINT_COLUMNS * 0.5, g.line_y(line, scroll) + lh / 2.0);
                let bounds = Rectangle::new(Point::new(center.x - size.width / 2.0, center.y - size.height / 2.0), size);
                r.fill_quad(Quad { bounds, border: Border { radius: radius.into(), ..Border::default() }, ..Quad::default() }, color);
            };
            let radius = (lh * 0.3).min(cw * 0.6);
            for &line in self.breakpoints.iter().flatten().filter(|&&l| (first..last).contains(&l) && shown(l)) {
                dot(r, line, Size::new(radius * 2.0, radius * 2.0), colors.breakpoint, radius);
            }
            if let Some(line) = self.paused_line.filter(|&l| (first..last).contains(&l) && shown(l)) {
                dot(r, line, Size::new(cw * 1.2, lh * 0.45), colors.paused_marker, lh * 0.12);
            }

            // Git change markers, at the gutter's right edge: a bar beside
            // added or modified lines, a wedge where lines were removed.
            let bar_x = g.text_area.x - cw * 0.45;
            for change in self.git_changes.iter() {
                match change.kind {
                    ChangeKind::Added | ChangeKind::Modified => {
                        let color = if change.kind == ChangeKind::Added { colors.git_added } else { colors.git_modified };
                        let lines = change.lines.start.max(first)..change.lines.end.min(last);
                        for line in lines.filter(|&l| shown(l)) {
                            let y = g.line_y(line, scroll);
                            fill(r, Rectangle::new(Point::new(bar_x, y), Size::new(3.0, lh)), color);
                        }
                    }
                    ChangeKind::Deleted => {
                        // Between the line before and `lines.start` (past the
                        // end: below the last line).
                        let at = change.lines.start;
                        let y = if at < buffer.len_lines() {
                            g.line_y(at, scroll)
                        } else {
                            g.line_y(buffer.len_lines().saturating_sub(1), scroll) + lh
                        };
                        if (first..=last).contains(&at) && (at >= buffer.len_lines() || shown(at)) {
                            fill(r, Rectangle::new(Point::new(bar_x - 2.0, y - 2.0), Size::new(7.0, 4.0)), colors.git_deleted);
                        }
                    }
                }
            }
        });

        renderer.with_layer(g.text_area, |r| {
            // Current line.
            if primary.is_empty() {
                let y = g.line_y(primary_pos.line, scroll);
                fill(r, Rectangle { y, height: lh, ..g.text_area }, colors.current_line);
            }
            // The line the debugger is paused on.
            if let Some(line) = self.paused_line.filter(|&l| shown(l)) {
                let y = g.line_y(line, scroll);
                fill(r, Rectangle { y, height: lh, ..g.text_area }, colors.paused_line);
            }

            // Indentation guides.
            let guides = if self.options.indent_guides { indent_levels(buffer, first, last) } else { Vec::new() };
            for (line, levels) in (first..last).zip(guides).filter(|&(l, _)| shown(l)) {
                let y = g.line_y(line, scroll);
                for level in 0..levels {
                    let x = g.col_x(level * TAB_WIDTH, scroll).round();
                    fill(r, Rectangle::new(Point::new(x, y), Size::new(1.0, lh)), colors.indent_guide);
                }
            }

            // Search matches (sorted), only those on visible lines.
            let visible_start = buffer.pos_to_char(Pos { line: first, column: 0 });
            let visible_end = buffer.pos_to_char(Pos { line: last, column: 0 });
            let skip = self.highlights.partition_point(|&(_, end)| end < visible_start);
            for (index, &(start, end)) in self.highlights.iter().enumerate().skip(skip) {
                if start > visible_end {
                    break;
                }
                for rect in self.range_rects(&g, scroll, start, end, first, last) {
                    fill(r, rect, colors.find_match);
                    if self.current_highlight == Some(index) {
                        outline(r, rect, colors.find_current);
                    }
                }
            }

            // Selections.
            for sel in buffer.selections().iter().filter(|s| !s.is_empty()) {
                for rect in self.range_rects(&g, scroll, sel.start(), sel.end(), first, last) {
                    fill(r, rect, colors.selection);
                }
            }

            // Diagnostics underlines.
            for mark in self.marks.iter().filter(|m| (first..last).contains(&m.line) && shown(m.line)) {
                let from = buffer.visual_col(mark.line, mark.start);
                let to = buffer.visual_col(mark.line, mark.end).max(from + 1);
                let rect = Rectangle::new(
                    Point::new(g.col_x(from, scroll), g.line_y(mark.line, scroll) + lh - 2.0),
                    Size::new((to - from) as f32 * cw, 1.5),
                );
                fill(r, rect, mark.color);
            }

            // Matching brackets.
            if state.focused && self.focused && primary.is_empty() {
                if let Some((open, close)) = buffer.matching_bracket(primary.head) {
                    for index in [open, close] {
                        let pos = buffer.char_to_pos(index);
                        let rect = Rectangle::new(self.screen_point(&g, scroll, pos), Size::new(cw, lh));
                        outline(r, rect, colors.bracket);
                    }
                }
            }

            // Text.
            for line in (first..last).filter(|&l| shown(l)) {
                let text = buffer.line_text(line);
                let tokens = buffer.tokens(line);
                let y = g.line_y(line, scroll);
                for (visual, segment, kind) in segments(&text, &tokens) {
                    if segment.trim().is_empty() {
                        continue;
                    }
                    let point = Point::new(g.col_x(visual, scroll), y);
                    draw_text(r, segment, point, colors.token(kind), metrics, g.text_area);
                }
            }

            // "..." after a folded header (click it, or the gutter, to unfold).
            for &line in folded.iter().filter(|&&l| (first..last).contains(&l)) {
                let marker = g.fold_marker(buffer, line, scroll);
                r.fill_quad(
                    Quad { bounds: marker, border: Border { color: colors.muted, width: 1.0, radius: 3.0.into() }, ..Quad::default() },
                    colors.current_line,
                );
                let text = Point::new(marker.x + 2.0, g.line_y(line, scroll));
                draw_text(r, "...".to_string(), text, colors.muted, metrics, g.text_area);
            }

            // Ctrl+hover link.
            if let Some((start, end)) = self.link.filter(|_| state.modifiers.command()) {
                for rect in self.range_rects(&g, scroll, start, end, first, last) {
                    let underline = Rectangle::new(Point::new(rect.x, rect.y + lh - 3.0), Size::new(rect.width, 1.0));
                    fill(r, underline, colors.accent);
                }
            }

            // Inline messages: one per line, the first mark's (marks come
            // sorted most severe first).
            let mut with_message = std::collections::HashSet::new();
            for mark in self.marks.iter().filter(|m| self.options.inline_diagnostics && (first..last).contains(&m.line) && shown(m.line)) {
                let Some(message) = &mark.message else {
                    continue;
                };
                if !with_message.insert(mark.line) {
                    continue;
                }
                let end = buffer.visual_col(mark.line, buffer.line_len(mark.line));
                let point = Point::new(g.col_x(end + INLINE_MESSAGE_GAP, scroll), g.line_y(mark.line, scroll));
                let color = mark.color.scale_alpha(INLINE_MESSAGE_ALPHA);
                draw_text(r, message.clone(), point, color, metrics, g.text_area);
            }

            // Input method composition, drawn over the text at the cursor:
            // underlined, with the part the IME highlights selected.
            if let (Some((content, selection)), true) = (&state.preedit, self.focused) {
                let point = self.screen_point(&g, scroll, buffer.char_to_pos(primary.head));
                let width = |s: &str| s.chars().map(display_width).sum::<usize>() as f32 * cw;
                let rect = Rectangle::new(point, Size::new(width(content), lh));
                fill(r, rect, colors.background);
                if let Some(range) = selection.clone().filter(|r| content.is_char_boundary(r.start) && content.is_char_boundary(r.end)) {
                    let x = point.x + width(&content[..range.start]);
                    fill(r, Rectangle::new(Point::new(x, point.y), Size::new(width(&content[range]), lh)), colors.selection);
                }
                draw_text(r, content.clone(), point, colors.text, metrics, g.text_area);
                fill(r, Rectangle::new(Point::new(point.x, point.y + lh - 2.0), Size::new(rect.width, 1.0)), colors.accent);
            }

            // Cursors.
            if self.focused && state.cursor_visible(self.options.cursor_blink) && state.preedit.is_none() {
                for sel in buffer.selections() {
                    let pos = buffer.char_to_pos(sel.head);
                    if !(first..last).contains(&pos.line) {
                        continue;
                    }
                    let point = self.screen_point(&g, scroll, pos);
                    let rect = Rectangle::new(Point::new(point.x - 1.0, point.y), Size::new(2.0, lh));
                    fill(r, rect, colors.cursor);
                }
            }

            // Where a dragged file would be dropped.
            if let (Some(_), Some(mouse)) = (self.dropping, cursor.position_over(g.text_area)) {
                let point = self.screen_point(&g, scroll, self.pos_at(&g, scroll, mouse));
                let rect = Rectangle::new(Point::new(point.x - 1.0, point.y), Size::new(2.0, lh));
                fill(r, rect, colors.accent);
            }
        });

        // Popups, on top of everything.
        renderer.with_layer(g.bounds, |r| {
            if let Some(popup) = &self.hover {
                let width = 520.0_f32.min(g.bounds.width - 16.0);
                let size = Size::new(width, doc_height(popup.doc, width, metrics));
                let anchor = self.screen_point(&g, scroll, popup.pos);
                let rect = g.popup_at(anchor, size, false);
                draw_doc(r, popup.doc, rect, colors, metrics);
            }

            if let Some(popup) = &self.signature {
                let chars = popup.label.chars().count() as f32;
                let size = Size::new(chars * cw + POPUP_PADDING * 2.0, lh + 8.0);
                let anchor = self.screen_point(&g, scroll, popup.anchor);
                let rect = g.popup_at(Point::new(anchor.x - cw, anchor.y), size, true);
                panel(r, rect, colors);
                let origin = Point::new(rect.x + POPUP_PADDING, rect.y + 4.0);
                let label = popup.label;
                let active = popup.active.clone().unwrap_or(0..0);
                let parts = [
                    (&label[..active.start], colors.text),
                    (&label[active.clone()], colors.accent),
                    (&label[active.end..], colors.text),
                ];
                let mut x = origin.x;
                for (part, color) in parts {
                    if !part.is_empty() {
                        draw_text(r, part.to_string(), Point::new(x, origin.y), color, metrics, rect);
                    }
                    x += part.chars().count() as f32 * cw;
                }
            }

            if let Some(popup) = &self.completion {
                let layout = self.completion_layout(popup, &g, scroll, metrics);
                panel(r, layout.list, colors);
                let visible = popup.rows.len().min(COMPLETION_ROWS);
                for (offset, row) in popup.rows[layout.first..layout.first + visible].iter().enumerate() {
                    let index = layout.first + offset;
                    let y = layout.list.y + 2.0 + offset as f32 * lh;
                    if index == popup.selected {
                        let bounds = Rectangle::new(Point::new(layout.list.x + 2.0, y), Size::new(layout.list.width - 4.0, lh));
                        fill(r, bounds, colors.selection);
                    }
                    let tag = Point::new(layout.list.x + cw * 0.5, y);
                    draw_text(r, row.tag.to_string(), tag, row.color, metrics, layout.list);
                    let label = Point::new(layout.list.x + cw * (TAG_COLUMNS as f32 + 0.5), y);
                    draw_text(r, row.label.clone(), label, colors.text, metrics, layout.list);
                }
                if let (Some(rect), Some(doc)) = (layout.docs, popup.doc) {
                    draw_doc(r, doc, rect, colors, metrics);
                }
            }
        });
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        let Some(metrics) = state.metrics else {
            return mouse::Interaction::None;
        };
        let geometry = Geometry::new(self.buffer, layout.bounds(), metrics);
        if self.dropping.is_some() && cursor.is_over(geometry.text_area) {
            return mouse::Interaction::Grabbing;
        }
        if let Some(point) = cursor.position_over(geometry.bounds) {
            if self.on_git_marker(&geometry, point, geometry.line_at_y(point.y, state.scroll)) || self.on_line_numbers(&geometry, point) {
                return mouse::Interaction::Pointer;
            }
        }
        if let Some(popup) = &self.completion {
            let layout = self.completion_layout(popup, &geometry, state.scroll, metrics);
            if cursor.is_over(layout.list) {
                return mouse::Interaction::Pointer;
            }
        }
        if cursor.position().is_some_and(|p| self.over_link(state, &geometry, p)) {
            mouse::Interaction::Pointer
        } else if cursor.is_over(geometry.text_area) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

impl<'a, Message: 'a> From<CodeEditor<'a, Message>> for Element<'a, Message> {
    fn from(editor: CodeEditor<'a, Message>) -> Self {
        Element::new(editor)
    }
}

pub(crate) fn measure(code: CodeFont) -> Metrics {
    const SAMPLE: &str = "MMMMMMMMMMMMMMMMMMMM";
    let line_height = (code.size * code.line_height).round();
    let paragraph = paragraph(SAMPLE, code.font, code.size, line_height, f32::INFINITY);
    Metrics {
        font: code.font,
        font_size: code.size,
        char_width: paragraph.min_width() / SAMPLE.len() as f32,
        line_height,
        ligatures: code.ligatures,
    }
}

type Paragraph = <iced::Renderer as text::Renderer>::Paragraph;

fn paragraph(content: &str, font: Font, size: f32, line_height: f32, width: f32) -> Paragraph {
    Paragraph::with_text(text::Text {
        content,
        bounds: Size::new(width, f32::INFINITY),
        size: Pixels(size),
        line_height: text::LineHeight::Absolute(Pixels(line_height)),
        font,
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::WordOrGlyph,
    })
}

fn doc_line_height() -> f32 {
    (DOC_FONT_SIZE * 1.35).round()
}

/// Height of a docs panel of `width`: signature (monospace) plus prose.
fn doc_height(doc: &Doc, width: f32, metrics: Metrics) -> f32 {
    let inner = width - POPUP_PADDING * 2.0;
    let mut height = POPUP_PADDING * 2.0;
    if let Some(signature) = &doc.signature {
        height += paragraph(signature, metrics.font, metrics.font_size, metrics.line_height, inner).min_height();
    }
    if !doc.body.is_empty() {
        if doc.signature.is_some() {
            height += 6.0;
        }
        height += paragraph(&doc.body, Font::DEFAULT, DOC_FONT_SIZE, doc_line_height(), inner).min_height();
    }
    height.min(360.0)
}

/// Draws a docs panel. Uses `fill_text` (owned, cached by the renderer):
/// `fill_paragraph` only keeps a weak reference, so a paragraph built
/// during `draw` would be gone before the frame renders. The bounds must be
/// finite: with an infinite height the cached text wraps after every glyph.
fn draw_doc(renderer: &mut iced::Renderer, doc: &Doc, rect: Rectangle, colors: &Colors, metrics: Metrics) {
    panel(renderer, rect, colors);
    let inner = rect.width - POPUP_PADDING * 2.0;
    let x = rect.x + POPUP_PADDING;
    let mut y = rect.y + POPUP_PADDING;
    let clip = Rectangle { height: rect.height - POPUP_PADDING, ..rect };
    let wrapped = |renderer: &mut iced::Renderer, content: &str, font, size, line_height, color, y| {
        renderer.fill_text(
            text::Text {
                content: content.to_string(),
                bounds: Size::new(inner, rect.height),
                size: Pixels(size),
                line_height: text::LineHeight::Absolute(Pixels(line_height)),
                font,
                align_x: text::Alignment::Left,
                align_y: alignment::Vertical::Top,
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::WordOrGlyph,
            },
            Point::new(x, y),
            color,
            clip,
        );
    };
    if let Some(signature) = &doc.signature {
        let function = to_color(colors.syntax.function);
        wrapped(renderer, signature, metrics.font, metrics.font_size, metrics.line_height, function, y);
        y += paragraph(signature, metrics.font, metrics.font_size, metrics.line_height, inner).min_height() + 6.0;
    }
    if !doc.body.is_empty() {
        wrapped(renderer, &doc.body, Font::DEFAULT, DOC_FONT_SIZE, doc_line_height(), colors.text, y);
    }
}

fn panel(renderer: &mut iced::Renderer, bounds: Rectangle, colors: &Colors) {
    renderer.fill_quad(
        Quad {
            bounds,
            border: Border { color: colors.popup_border, width: 1.0, radius: 4.0.into() },
            ..Quad::default()
        },
        colors.popup,
    );
}

fn outline(renderer: &mut iced::Renderer, bounds: Rectangle, color: Color) {
    renderer.fill_quad(
        Quad {
            bounds,
            border: Border { color, width: 1.0, radius: 2.0.into() },
            ..Quad::default()
        },
        Color::TRANSPARENT,
    );
}

fn fill(renderer: &mut iced::Renderer, bounds: Rectangle, color: Color) {
    renderer.fill_quad(Quad { bounds, ..Quad::default() }, color);
}

fn draw_text(
    renderer: &mut iced::Renderer,
    content: String,
    position: Point,
    color: Color,
    metrics: Metrics,
    clip: Rectangle,
) {
    // Basic shaping is faster but skips ligatures.
    let basic = content.is_ascii() && !metrics.ligatures;
    let shaping = if basic { text::Shaping::Basic } else { text::Shaping::Advanced };
    renderer.fill_text(
        text::Text {
            content,
            bounds: Size::new(f32::INFINITY, metrics.line_height),
            size: Pixels(metrics.font_size),
            line_height: text::LineHeight::Absolute(Pixels(metrics.line_height)),
            font: metrics.font,
            align_x: text::Alignment::Left,
            align_y: alignment::Vertical::Top,
            shaping,
            wrapping: text::Wrapping::None,
        },
        position,
        color,
        clip,
    );
}

/// How far a blank line looks up and down for the indentation it sits in.
const GUIDE_SCAN_LINES: usize = 200;

/// Indent levels of `first..last` for indentation guides. A blank line
/// takes the smaller level of the nearest non-blank lines around it, so
/// guides run through blank lines inside a block but not between blocks.
fn indent_levels(buffer: &Buffer, first: usize, last: usize) -> Vec<usize> {
    let level = |line: usize| -> Option<usize> {
        let text = buffer.line_text(line);
        let indent = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        (indent < text.trim_end().chars().count()).then(|| buffer.visual_col(line, indent) / TAB_WIDTH)
    };
    let len = buffer.len_lines();
    (first..last)
        .map(|line| {
            level(line).unwrap_or_else(|| {
                let above = (line.saturating_sub(GUIDE_SCAN_LINES)..line).rev().find_map(level);
                let below = (line + 1..len.min(line + GUIDE_SCAN_LINES)).find_map(level);
                above.unwrap_or(0).min(below.unwrap_or(0))
            })
        })
        .collect()
}

/// Splits a line into same-color runs: (visual column, text with tabs
/// expanded to spaces, token kind).
fn segments(line: &str, tokens: &[Token]) -> Vec<(usize, String, Option<TokenKind>)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_start = 0;
    let mut current_kind = None;
    let mut visual = 0;
    let mut token = 0;
    for (byte, c) in line.char_indices() {
        while token < tokens.len() && tokens[token].range.end <= byte {
            token += 1;
        }
        let kind = tokens
            .get(token)
            .filter(|t| t.range.start <= byte)
            .map(|t| t.kind);
        if kind != current_kind && !current.is_empty() {
            out.push((current_start, std::mem::take(&mut current), current_kind));
        }
        if current.is_empty() {
            current_start = visual;
            current_kind = kind;
        }
        if c == '\t' {
            let width = TAB_WIDTH - visual % TAB_WIDTH;
            current.extend(std::iter::repeat_n(' ', width));
            visual += width;
        } else if display_width(c) > 1 {
            // A wide glyph is drawn alone at its own cells: fonts don't size
            // it to exactly two cells, which would push the rest off the grid.
            if !current.is_empty() {
                out.push((current_start, std::mem::take(&mut current), current_kind));
            }
            out.push((visual, c.to_string(), kind));
            visual += display_width(c);
        } else {
            current.push(c);
            visual += 1;
        }
    }
    if !current.is_empty() {
        out.push((current_start, current, current_kind));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::gdscript::{lex_line, LineState};

    #[test]
    fn segments_expand_tabs_and_split_by_token() {
        let line = "\tvar x = 1";
        let tokens = lex_line(line, LineState::Normal).0;
        let segs = segments(line, &tokens);
        assert_eq!(segs[0], (0, "    ".to_string(), None));
        assert_eq!(segs[1], (4, "var".to_string(), Some(TokenKind::Keyword)));
        let last = segs.last().unwrap();
        assert_eq!((last.0, last.1.as_str(), last.2), (12, "1", Some(TokenKind::Number)));
    }

    #[test]
    fn wide_characters_are_their_own_two_cell_segments() {
        let line = "a日本b";
        let segs = segments(line, &[]);
        let cells: Vec<(usize, &str)> = segs.iter().map(|(v, s, _)| (*v, s.as_str())).collect();
        assert_eq!(cells, [(0, "a"), (1, "日"), (3, "本"), (5, "b")]);
    }

    #[test]
    fn indent_levels_bridge_blank_lines_inside_blocks() {
        let text = "func a():\n\tif x:\n\t\tpass\n\n\t\tpass\n\n    return\n\nfunc b():\n";
        let buffer = Buffer::new(text, editor_core::Language::GDScript);
        let levels = indent_levels(&buffer, 0, buffer.len_lines());
        // Blank line 3 sits inside the `if` body; line 7 between functions.
        assert_eq!(levels, [0, 1, 2, 2, 2, 1, 1, 0, 0, 0]);
    }
}
