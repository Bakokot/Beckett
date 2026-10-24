//! Code editor widget: draws an `editor_core::Buffer` on a monospace grid
//! and turns input into `Command`s. The buffer lives in app state; this
//! widget only keeps view state (focus, scroll, drag).

use editor_core::buffer::TAB_WIDTH;
use editor_core::gdscript::{Token, TokenKind};
use editor_core::{Buffer, Command, Motion, Pos};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self, Paragraph as _, Renderer as _};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{clipboard, mouse, Clipboard, Renderer as _, Shell};
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::time::{Duration, Instant};
use iced::{
    alignment, window, Border, Color, Element, Event, Font, Length, Pixels, Point, Rectangle,
    Size, Theme, Vector,
};

use crate::style::to_color;

const BLINK_MILLIS: u128 = 500;
const LINE_HEIGHT_FACTOR: f32 = 1.4;

/// An underline over a char-column range (e.g. a diagnostic).
#[derive(Debug, Clone, Copy)]
pub struct Mark {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub color: Color,
}

#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub background: Color,
    pub text: Color,
    pub gutter: Color,
    pub gutter_active: Color,
    pub current_line: Color,
    pub selection: Color,
    pub cursor: Color,
    pub bracket: Color,
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
            gutter: to_color(ui.text_muted).scale_alpha(0.7),
            gutter_active: text,
            current_line: text.scale_alpha(0.05),
            selection: accent.scale_alpha(0.35),
            cursor: text,
            bracket: text.scale_alpha(0.5),
            syntax: theme.syntax,
        }
    }

    fn token(&self, kind: Option<TokenKind>) -> Color {
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
    on_command: Box<dyn Fn(Command) -> Message + 'a>,
    marks: &'a [Mark],
    colors: Colors,
    font_size: f32,
}

impl<'a, Message> CodeEditor<'a, Message> {
    pub fn new(
        buffer: &'a Buffer,
        colors: Colors,
        on_command: impl Fn(Command) -> Message + 'a,
    ) -> Self {
        Self {
            buffer,
            on_command: Box::new(on_command),
            marks: &[],
            colors,
            font_size: 14.0,
        }
    }

    pub fn marks(mut self, marks: &'a [Mark]) -> Self {
        self.marks = marks;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Metrics {
    font_size: f32,
    char_width: f32,
    line_height: f32,
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
        }
    }

    fn cursor_visible(&self) -> bool {
        self.focused
            && self.window_focused
            && (self.now.saturating_duration_since(self.focused_at).as_millis() / BLINK_MILLIS) % 2 == 0
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
}

impl Geometry {
    fn new(buffer: &Buffer, bounds: Rectangle, metrics: Metrics) -> Self {
        let digits = buffer.len_lines().to_string().len().max(3) as f32;
        let gutter = (digits + 2.0) * metrics.char_width;
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
        }
    }

    fn line_y(&self, line: usize, scroll: Vector) -> f32 {
        self.bounds.y + line as f32 * self.line_height - scroll.y
    }

    fn col_x(&self, visual: usize, scroll: Vector) -> f32 {
        self.text_x + visual as f32 * self.char_width - scroll.x
    }

    fn page_lines(&self) -> usize {
        ((self.bounds.height / self.line_height) as usize).saturating_sub(1).max(1)
    }
}

enum KeyAction {
    Command(Command),
    Copy,
    Cut,
    Paste,
}

impl<'a, Message> CodeEditor<'a, Message> {
    fn pos_at(&self, geometry: &Geometry, scroll: Vector, point: Point) -> Pos {
        let line = ((point.y - geometry.bounds.y + scroll.y) / geometry.line_height).max(0.0) as usize;
        let line = line.min(self.buffer.len_lines() - 1);
        let visual = ((point.x - geometry.text_x + scroll.x) / geometry.char_width).round().max(0.0);
        Pos { line, column: self.buffer.column_for_visual(line, visual as usize) }
    }

    /// Scrolls so the primary cursor is visible.
    fn reveal(&self, state: &mut State, geometry: &Geometry) {
        let pos = self.buffer.char_to_pos(self.buffer.primary().head);
        let lh = geometry.line_height;
        let y = pos.line as f32 * lh;
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
        let max_y = (self.buffer.len_lines() - 1) as f32 * geometry.line_height;
        state.scroll.y = state.scroll.y.clamp(0.0, max_y);
        state.scroll.x = state.scroll.x.max(0.0);
    }

    fn key_action(
        &self,
        key: &Key,
        modifiers: Modifiers,
        text: Option<&str>,
        page: usize,
    ) -> Option<KeyAction> {
        let (ctrl, shift, alt) = (modifiers.command(), modifiers.shift(), modifiers.alt());
        let movement = |motion| Some(KeyAction::Command(Command::Move { motion, extend: shift }));
        let command = |command| Some(KeyAction::Command(command));
        match key.as_ref() {
            Key::Named(named) => match named {
                Named::ArrowLeft => movement(if ctrl { Motion::WordLeft } else { Motion::Left }),
                Named::ArrowRight => movement(if ctrl { Motion::WordRight } else { Motion::Right }),
                Named::ArrowUp if ctrl && alt => command(Command::AddCursorAbove),
                Named::ArrowDown if ctrl && alt => command(Command::AddCursorBelow),
                Named::ArrowUp | Named::ArrowDown if shift && alt => command(Command::DuplicateLines),
                Named::ArrowUp if alt => command(Command::MoveLinesUp),
                Named::ArrowDown if alt => command(Command::MoveLinesDown),
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
                _ => None,
            },
            // Ctrl+Alt is AltGr on many layouts: treat it as typing.
            Key::Character(c) if ctrl && !alt => match c.to_lowercase().as_str() {
                "a" => command(Command::SelectAll),
                "c" => Some(KeyAction::Copy),
                "x" => Some(KeyAction::Cut),
                "v" => Some(KeyAction::Paste),
                "z" if shift => command(Command::Redo),
                "z" => command(Command::Undo),
                "y" => command(Command::Redo),
                "d" => command(Command::SelectNextOccurrence),
                "k" if shift => command(Command::DeleteLines),
                // Ctrl+K is Godot's binding, Ctrl+/ VS Code's.
                "k" | "/" => command(Command::ToggleComment),
                _ => None,
            },
            _ => {
                let text = text?.chars().filter(|c| !c.is_control()).collect::<String>();
                (!text.is_empty()).then_some(KeyAction::Command(Command::Insert(text)))
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
        if state.metrics.is_none_or(|m| m.font_size != self.font_size) {
            state.metrics = Some(measure(self.font_size));
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
                if state.focused && state.window_focused {
                    let elapsed = now.saturating_duration_since(state.focused_at).as_millis();
                    let wait = BLINK_MILLIS - elapsed % BLINK_MILLIS;
                    shell.request_redraw_at(*now + Duration::from_millis(wait as u64));
                }
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(point) = cursor.position_over(geometry.bounds) else {
                    state.focused = false;
                    return;
                };
                let click = mouse::Click::new(point, mouse::Button::Left, state.last_click);
                state.last_click = Some(click);
                state.focused = true;
                state.focused_at = Instant::now();

                let pos = self.pos_at(&geometry, state.scroll, point);
                let alt = state.modifiers.alt();
                let command = match click.kind() {
                    mouse::click::Kind::Single if alt => Command::AddCursorAt(pos),
                    mouse::click::Kind::Single => {
                        state.dragging = true;
                        Command::Click { pos, extend: state.modifiers.shift() }
                    }
                    mouse::click::Kind::Double => Command::SelectWordAt(pos),
                    mouse::click::Kind::Triple => Command::SelectLineAt(pos),
                };
                shell.publish((self.on_command)(command));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let pos = self.pos_at(&geometry, state.scroll, *position);
                shell.publish((self.on_command)(Command::Drag(pos)));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(geometry.bounds) => {
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
                shell.request_redraw();
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. })
                if state.focused =>
            {
                let page = geometry.page_lines();
                let Some(action) = self.key_action(key, *modifiers, text.as_deref(), page) else {
                    return;
                };
                let command = match action {
                    KeyAction::Command(command) => Some(command),
                    KeyAction::Copy | KeyAction::Cut => {
                        let selected = self.buffer.selected_text();
                        if !selected.is_empty() {
                            clipboard.write(clipboard::Kind::Standard, selected);
                        }
                        matches!(action, KeyAction::Cut).then_some(Command::DeleteSelection)
                    }
                    KeyAction::Paste => clipboard
                        .read(clipboard::Kind::Standard)
                        .map(Command::Paste),
                };
                if let Some(command) = command {
                    shell.publish((self.on_command)(command));
                }
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
        _cursor: mouse::Cursor,
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
        let max_y = (buffer.len_lines() - 1) as f32 * g.line_height;
        let scroll = Vector::new(state.scroll.x, state.scroll.y.min(max_y));
        let lh = g.line_height;
        let cw = g.char_width;

        fill(renderer, g.bounds, colors.background);

        let first = (scroll.y / lh).floor() as usize;
        let visible = (g.bounds.height / lh).ceil() as usize + 1;
        let last = (first + visible).min(buffer.len_lines());
        let primary = buffer.primary();
        let primary_pos = buffer.char_to_pos(primary.head);

        // Gutter: line numbers.
        renderer.with_layer(g.bounds, |r| {
            for line in first..last {
                let number = (line + 1).to_string();
                let x = g.text_area.x - (number.len() as f32 + 1.0) * cw;
                let color = if line == primary_pos.line { colors.gutter_active } else { colors.gutter };
                draw_text(r, number, Point::new(x, g.line_y(line, scroll)), color, metrics, g.bounds);
            }
        });

        renderer.with_layer(g.text_area, |r| {
            // Current line.
            if primary.is_empty() {
                let y = g.line_y(primary_pos.line, scroll);
                fill(r, Rectangle { y, height: lh, ..g.text_area }, colors.current_line);
            }

            // Selections.
            for sel in buffer.selections().iter().filter(|s| !s.is_empty()) {
                let start = buffer.char_to_pos(sel.start());
                let end = buffer.char_to_pos(sel.end());
                for line in start.line.max(first)..=end.line.min(last.saturating_sub(1)) {
                    let from = if line == start.line { buffer.visual_col(line, start.column) } else { 0 };
                    let to = if line == end.line {
                        buffer.visual_col(line, end.column)
                    } else {
                        buffer.visual_col(line, buffer.line_len(line)) + 1
                    };
                    let x = g.col_x(from, scroll);
                    let rect = Rectangle::new(
                        Point::new(x, g.line_y(line, scroll)),
                        Size::new((to - from) as f32 * cw, lh),
                    );
                    fill(r, rect, colors.selection);
                }
            }

            // Diagnostics underlines.
            for mark in self.marks.iter().filter(|m| (first..last).contains(&m.line)) {
                let from = buffer.visual_col(mark.line, mark.start);
                let to = buffer.visual_col(mark.line, mark.end).max(from + 1);
                let rect = Rectangle::new(
                    Point::new(g.col_x(from, scroll), g.line_y(mark.line, scroll) + lh - 2.0),
                    Size::new((to - from) as f32 * cw, 1.5),
                );
                fill(r, rect, mark.color);
            }

            // Matching brackets.
            if state.focused && primary.is_empty() {
                if let Some((open, close)) = buffer.matching_bracket(primary.head) {
                    for index in [open, close] {
                        let pos = buffer.char_to_pos(index);
                        let visual = buffer.visual_col(pos.line, pos.column);
                        let rect = Rectangle::new(
                            Point::new(g.col_x(visual, scroll), g.line_y(pos.line, scroll)),
                            Size::new(cw, lh),
                        );
                        r.fill_quad(
                            Quad {
                                bounds: rect,
                                border: Border { color: colors.bracket, width: 1.0, radius: 2.0.into() },
                                ..Quad::default()
                            },
                            Color::TRANSPARENT,
                        );
                    }
                }
            }

            // Text.
            for line in first..last {
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

            // Cursors.
            if state.cursor_visible() {
                for sel in buffer.selections() {
                    let pos = buffer.char_to_pos(sel.head);
                    if !(first..last).contains(&pos.line) {
                        continue;
                    }
                    let visual = buffer.visual_col(pos.line, pos.column);
                    let rect = Rectangle::new(
                        Point::new(g.col_x(visual, scroll) - 1.0, g.line_y(pos.line, scroll)),
                        Size::new(2.0, lh),
                    );
                    fill(r, rect, colors.cursor);
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
        if cursor.is_over(geometry.text_area) {
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

fn measure(font_size: f32) -> Metrics {
    const SAMPLE: &str = "MMMMMMMMMMMMMMMMMMMM";
    let line_height = (font_size * LINE_HEIGHT_FACTOR).round();
    let paragraph = <iced::Renderer as text::Renderer>::Paragraph::with_text(text::Text {
        content: SAMPLE,
        bounds: Size::INFINITE,
        size: Pixels(font_size),
        line_height: text::LineHeight::Absolute(Pixels(line_height)),
        font: Font::MONOSPACE,
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Basic,
        wrapping: text::Wrapping::None,
    });
    Metrics {
        font_size,
        char_width: paragraph.min_width() / SAMPLE.len() as f32,
        line_height,
    }
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
    let shaping = if content.is_ascii() { text::Shaping::Basic } else { text::Shaping::Advanced };
    renderer.fill_text(
        text::Text {
            content,
            bounds: Size::new(f32::INFINITY, metrics.line_height),
            size: Pixels(metrics.font_size),
            line_height: text::LineHeight::Absolute(Pixels(metrics.line_height)),
            font: Font::MONOSPACE,
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
}
