//! Terminal widget: draws a `terminal::Terminal` grid and forwards input to
//! the shell. Writes go straight to the pty; redraws are driven by the
//! terminal's wakeup events.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self, Renderer as _};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{clipboard, mouse, Clipboard, Renderer as _, Shell};
use iced::keyboard::{self, key::Named, Modifiers};
use iced::time::{Duration, Instant};
use iced::{
    alignment, font, window, Border, Color, Element, Event, Font, Length, Pixels, Point, Rectangle,
    Size, Theme,
};
use terminal::{
    encode_key, encode_mouse, CellView, Key, Mods, MouseAction, MouseButton, MouseTracking, Palette, Rgb,
    SelectionKind, Terminal,
};

use crate::code_editor::{measure, CodeFont, Metrics};
use crate::keymap::Chord;
use crate::terminal_links::{self, TermLink};

const PADDING: f32 = 6.0;

pub struct TerminalView<'a, Message> {
    terminal: &'a Terminal,
    palette: Palette,
    selection: Color,
    focused: bool,
    on_focus: Message,
    font: CodeFont,
    passthrough: Vec<Chord>,
    /// Ctrl+click on a link (path or URL) in the output.
    on_link: Option<Box<dyn Fn(TermLink) -> Message + 'a>>,
    link_color: Color,
}

impl<'a, Message: Clone> TerminalView<'a, Message> {
    pub fn new(terminal: &'a Terminal, palette: Palette, on_focus: Message) -> Self {
        Self {
            terminal,
            palette,
            selection: Color::from_rgba8(0x47, 0x8c, 0xbf, 0.4),
            focused: false,
            on_focus,
            font: CodeFont::default(),
            passthrough: Vec::new(),
            on_link: None,
            link_color: Color::from_rgb8(0x47, 0x8c, 0xbf),
        }
    }

    /// Links under Ctrl+click, underlined in `color` while Ctrl is held.
    pub fn on_link(mut self, color: Color, on_link: impl Fn(TermLink) -> Message + 'a) -> Self {
        self.link_color = color;
        self.on_link = Some(Box::new(on_link));
        self
    }

    /// Key presses left to the app instead of sent to the shell.
    pub fn passthrough(mut self, chords: Vec<Chord>) -> Self {
        self.passthrough = chords;
        self
    }

    pub fn font(mut self, font: CodeFont) -> Self {
        self.font = font;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn selection_color(mut self, color: Color) -> Self {
        self.selection = color;
        self
    }
}

#[derive(Default)]
struct State {
    metrics: Option<Metrics>,
    /// Font the metrics were measured for.
    font: Option<CodeFont>,
    modifiers: Modifiers,
    dragging: bool,
    moved: bool,
    last_click: Option<mouse::Click>,
    /// Button held down while an app tracks the mouse, and the last cell
    /// reported, so moves within one cell aren't sent again.
    reported_button: Option<MouseButton>,
    reported_cell: Option<(usize, usize)>,
    /// Cursor blink phase starts here (reset on focus and on typing).
    blink_from: Option<Instant>,
    now: Option<Instant>,
    /// Where the mouse is over the terminal.
    mouse: Option<Point>,
    /// The link under the mouse while Ctrl is held: row and columns.
    link: Option<(usize, std::ops::Range<usize>)>,
}

const BLINK_MILLIS: u128 = 500;

impl State {
    fn cursor_visible(&self) -> bool {
        match (self.blink_from, self.now) {
            (Some(from), Some(now)) => (now.saturating_duration_since(from).as_millis() / BLINK_MILLIS).is_multiple_of(2),
            _ => true,
        }
    }
}

fn color(rgb: Rgb) -> Color {
    Color::from_rgb8(rgb.r, rgb.g, rgb.b)
}

fn to_mouse_button(button: mouse::Button) -> Option<MouseButton> {
    match button {
        mouse::Button::Left => Some(MouseButton::Left),
        mouse::Button::Middle => Some(MouseButton::Middle),
        mouse::Button::Right => Some(MouseButton::Right),
        _ => None,
    }
}

impl<'a, Message> TerminalView<'a, Message> {
    /// The link at `point` (row, columns, link), when links are wanted.
    fn link_at(&self, bounds: Rectangle, metrics: Metrics, point: Point) -> Option<(usize, std::ops::Range<usize>, TermLink)> {
        self.on_link.as_ref()?;
        let (row, column) = cell_at(bounds, metrics, point);
        let snapshot = self.terminal.snapshot(&self.palette);
        let cells = snapshot.rows.get(row)?;
        // One char per column; a wide char's second column ends a word.
        let mut line = vec![' '; snapshot.columns];
        for (index, cell) in cells {
            if let Some(slot) = line.get_mut(*index) {
                *slot = if cell.c == '\0' { ' ' } else { cell.c };
            }
            if cell.wide {
                if let Some(slot) = line.get_mut(index + 1) {
                    *slot = '\0';
                }
            }
        }
        let (span, link) = terminal_links::link_at(&line, column)?;
        Some((row, span, link))
    }

    /// Recomputes the underlined link: the one under the mouse with Ctrl held.
    fn update_link(&self, state: &mut State, bounds: Rectangle, metrics: Metrics) -> bool {
        let link = match state.mouse {
            Some(point) if state.modifiers.command() => self.link_at(bounds, metrics, point).map(|(row, span, _)| (row, span)),
            _ => None,
        };
        let changed = link != state.link;
        state.link = link;
        changed
    }
}

/// Grid cell under `point`.
fn cell_at(bounds: Rectangle, metrics: Metrics, point: Point) -> (usize, usize) {
    let column = ((point.x - bounds.x - PADDING) / metrics.char_width).max(0.0) as usize;
    let row = ((point.y - bounds.y - PADDING) / metrics.line_height).max(0.0) as usize;
    (row, column)
}

fn to_key(key: &keyboard::Key, text: Option<&str>, ctrl: bool) -> Option<Key> {
    Some(match key.as_ref() {
        keyboard::Key::Named(named) => match named {
            Named::Enter => Key::Enter,
            Named::Backspace => Key::Backspace,
            Named::Tab => Key::Tab,
            Named::Escape => Key::Escape,
            Named::ArrowUp => Key::Up,
            Named::ArrowDown => Key::Down,
            Named::ArrowLeft => Key::Left,
            Named::ArrowRight => Key::Right,
            Named::Home => Key::Home,
            Named::End => Key::End,
            Named::PageUp => Key::PageUp,
            Named::PageDown => Key::PageDown,
            Named::Insert => Key::Insert,
            Named::Delete => Key::Delete,
            Named::Space => Key::Char(" ".to_string()),
            Named::F1 => Key::F(1),
            Named::F2 => Key::F(2),
            Named::F3 => Key::F(3),
            Named::F4 => Key::F(4),
            Named::F5 => Key::F(5),
            Named::F6 => Key::F(6),
            Named::F7 => Key::F(7),
            Named::F8 => Key::F(8),
            Named::F9 => Key::F(9),
            Named::F10 => Key::F(10),
            Named::F11 => Key::F(11),
            Named::F12 => Key::F(12),
            _ => return None,
        },
        // With Ctrl the produced text is a control code; use the key itself.
        keyboard::Key::Character(c) if ctrl => Key::Char(c.to_string()),
        keyboard::Key::Character(c) => Key::Char(text.unwrap_or(c).to_string()),
        keyboard::Key::Unidentified => Key::Char(text?.to_string()),
    })
}

impl<'a, Message: Clone> Widget<Message, Theme, iced::Renderer> for TerminalView<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
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
        let Some(metrics) = state.metrics else {
            return layout::Node::new(Size::ZERO);
        };
        let size = limits.resolve(Length::Fill, Length::Fill, Size::ZERO);
        let columns = ((size.width - PADDING * 2.0) / metrics.char_width).floor().max(2.0) as usize;
        let lines = ((size.height - PADDING * 2.0) / metrics.line_height).floor().max(1.0) as usize;
        let cell = (metrics.char_width.round() as u16, metrics.line_height.round() as u16);
        self.terminal.resize(columns, lines, cell);
        layout::Node::new(size)
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
        let bounds = layout.bounds();

        // Mouse reporting: when the app tracks the mouse (and Shift isn't
        // held, which keeps selecting text), clicks, moves and the wheel go
        // to it instead.
        let modes = self.terminal.input_modes();
        let tracking = modes.mouse != MouseTracking::Off && !state.modifiers.shift();
        let mods = Mods { ctrl: state.modifiers.control(), alt: state.modifiers.alt(), shift: state.modifiers.shift() };
        let report = |state: &mut State, action: MouseAction, cell: (usize, usize)| {
            if modes.mouse.reports(action) {
                if let Some(bytes) = encode_mouse(action, cell.0, cell.1, mods, modes.sgr_mouse) {
                    self.terminal.write(bytes);
                }
                state.reported_cell = Some(cell);
            }
        };

        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                state.now = Some(*now);
                if self.focused {
                    let from = *state.blink_from.get_or_insert(*now);
                    let elapsed = now.saturating_duration_since(from).as_millis();
                    let wait = BLINK_MILLIS - elapsed % BLINK_MILLIS;
                    shell.request_redraw_at(*now + Duration::from_millis(wait as u64));
                } else {
                    // Blinking restarts, cursor shown, when focus comes back.
                    state.blink_from = None;
                }
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                if self.update_link(state, bounds, metrics) {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) if tracking => {
                let (Some(point), Some(button)) = (cursor.position_over(bounds), to_mouse_button(*button)) else {
                    return;
                };
                if !self.focused {
                    shell.publish(self.on_focus.clone());
                }
                state.reported_button = Some(button);
                report(state, MouseAction::Press(button), cell_at(bounds, metrics, point));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(button)) if state.reported_button.is_some() => {
                if let (Some(pressed), Some(point)) = (state.reported_button, cursor.position()) {
                    if Some(pressed) == to_mouse_button(*button) {
                        state.reported_button = None;
                        report(state, MouseAction::Release(pressed), cell_at(bounds, metrics, point));
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position })
                if tracking && (state.reported_button.is_some() || cursor.is_over(bounds)) =>
            {
                let cell = cell_at(bounds, metrics, *position);
                if state.reported_cell != Some(cell) {
                    report(state, MouseAction::Move(state.reported_button), cell);
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) && (tracking || modes.alt_screen) => {
                let lines = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => y / metrics.line_height,
                };
                let steps = (lines.round() as i32).clamp(-10, 10);
                let cell = cursor.position().map_or((0, 0), |p| cell_at(bounds, metrics, p));
                for _ in 0..steps.unsigned_abs() {
                    if tracking {
                        report(state, if steps > 0 { MouseAction::WheelUp } else { MouseAction::WheelDown }, cell);
                    } else if modes.alternate_scroll {
                        // A full-screen app without mouse tracking (less, man):
                        // the wheel scrolls it with arrow keys.
                        let key = if steps > 0 { Key::Up } else { Key::Down };
                        if let Some(bytes) = encode_key(&key, Mods::default(), modes.app_cursor) {
                            self.terminal.write(bytes);
                        }
                    }
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(point) = cursor.position_over(bounds) else {
                    return;
                };
                // Ctrl+click on a link opens it instead of selecting.
                if state.modifiers.command() {
                    if let (Some((_, _, link)), Some(on_link)) = (self.link_at(bounds, metrics, point), &self.on_link) {
                        shell.publish(on_link(link));
                        shell.capture_event();
                        return;
                    }
                }
                if !self.focused {
                    shell.publish(self.on_focus.clone());
                }
                let click = mouse::Click::new(point, mouse::Button::Left, state.last_click);
                state.last_click = Some(click);
                let (row, column) = cell_at(bounds, metrics, point);
                let kind = match click.kind() {
                    mouse::click::Kind::Single => SelectionKind::Simple,
                    mouse::click::Kind::Double => SelectionKind::Word,
                    mouse::click::Kind::Triple => SelectionKind::Line,
                };
                self.terminal.start_selection(row, column, kind);
                state.dragging = true;
                state.moved = kind != SelectionKind::Simple;
                shell.request_redraw();
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let (row, column) = cell_at(bounds, metrics, *position);
                self.terminal.extend_selection(row, column);
                state.moved = true;
                shell.request_redraw();
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                // A plain click selects nothing.
                if !state.moved {
                    self.terminal.clear_selection();
                }
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let lines = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => y / metrics.line_height,
                };
                self.terminal.scroll(lines.round() as i32);
                self.update_link(state, bounds, metrics);
                shell.request_redraw();
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                state.mouse = cursor.position_over(bounds);
                if self.update_link(state, bounds, metrics) {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                state.mouse = None;
                if self.update_link(state, bounds, metrics) {
                    shell.request_redraw();
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. })
                if self.focused =>
            {
                // IDE-wide shortcuts (palette, terminal toggle, run…) go to the app.
                if Chord::from_key(key, *modifiers).is_some_and(|chord| self.passthrough.contains(&chord)) {
                    return;
                }
                let mods = Mods { ctrl: modifiers.control(), alt: modifiers.alt(), shift: modifiers.shift() };
                if mods.ctrl && !mods.alt {
                    if let keyboard::Key::Character(c) = key.as_ref() {
                        match c.to_lowercase().as_str() {
                            // Copy when something is selected, else send ^C.
                            "c" if mods.shift || self.terminal.selected_text().is_some() => {
                                if let Some(selected) = self.terminal.selected_text() {
                                    clipboard.write(clipboard::Kind::Standard, selected);
                                }
                                self.terminal.clear_selection();
                                shell.capture_event();
                                return;
                            }
                            "v" => {
                                if let Some(pasted) = clipboard.read(clipboard::Kind::Standard) {
                                    self.terminal.paste(&pasted);
                                }
                                shell.capture_event();
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                let Some(key) = to_key(key, text.as_deref(), mods.ctrl && !mods.alt) else {
                    return;
                };
                // Keep the cursor visible while typing.
                state.blink_from = state.now;
                let app_cursor = self.terminal.input_modes().app_cursor;
                if let Some(bytes) = encode_key(&key, mods, app_cursor) {
                    self.terminal.write(bytes);
                    self.terminal.clear_selection();
                    self.terminal.scroll_to_bottom();
                }
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
        let bounds = layout.bounds();
        let (cw, lh) = (metrics.char_width, metrics.line_height);
        let snapshot = self.terminal.snapshot(&self.palette);
        let origin = Point::new(bounds.x + PADDING, bounds.y + PADDING);
        let cell_rect = |row: usize, column: usize, width: usize| {
            Rectangle::new(
                Point::new(origin.x + column as f32 * cw, origin.y + row as f32 * lh),
                Size::new(width as f32 * cw, lh),
            )
        };

        renderer.with_layer(bounds, |r| {
            fill(r, bounds, color(self.palette.background));

            for (row, cells) in snapshot.rows.iter().enumerate() {
                // Backgrounds and selection.
                for (column, cell) in cells {
                    let width = if cell.wide { 2 } else { 1 };
                    if let Some(bg) = cell.bg {
                        fill(r, cell_rect(row, *column, width), color(bg));
                    }
                    if cell.selected {
                        fill(r, cell_rect(row, *column, width), self.selection);
                    }
                }

                // Text, in runs of cells that share a style.
                let mut run = String::new();
                let mut run_start = 0;
                let mut run_style: Option<(Rgb, bool, bool)> = None;
                let mut next_column = 0;
                let flush = |r: &mut iced::Renderer, run: &mut String, start: usize, style: Option<(Rgb, bool, bool)>| {
                    if let Some((fg, bold, italic)) = style {
                        if !run.trim().is_empty() {
                            draw_text(r, std::mem::take(run), cell_rect(row, start, 1).position(), color(fg), bold, italic, metrics, bounds);
                        }
                    }
                    run.clear();
                };
                for (column, cell) in cells {
                    let style = Some((cell.fg, cell.bold, cell.italic));
                    let contiguous = *column == next_column && !cell.wide;
                    if style != run_style || !contiguous {
                        flush(r, &mut run, run_start, run_style);
                        run_start = *column;
                        run_style = style;
                    }
                    if cell.wide {
                        // Wide glyphs are drawn alone at their own column.
                        let point = cell_rect(row, *column, 2).position();
                        draw_text(r, cell.c.to_string(), point, color(cell.fg), cell.bold, cell.italic, metrics, bounds);
                        run_style = None;
                        next_column = column + 2;
                        continue;
                    }
                    run.push(if cell.c == '\0' { ' ' } else { cell.c });
                    next_column = column + 1;
                    if cell.underline {
                        let rect = cell_rect(row, *column, 1);
                        fill(r, Rectangle { y: rect.y + lh - 2.0, height: 1.0, ..rect }, color(cell.fg));
                    }
                }
                flush(r, &mut run, run_start, run_style);
            }

            // The link Ctrl+click would open.
            if let Some((row, span)) = &state.link {
                let rect = cell_rect(*row, span.start, span.len());
                fill(r, Rectangle { y: rect.y + lh - 1.5, height: 1.0, ..rect }, self.link_color);
            }

            // Cursor: a blinking block when focused, an outline otherwise.
            if let Some((row, column)) = snapshot.cursor {
                let rect = cell_rect(row, column, 1);
                let cursor = color(self.palette.cursor);
                if self.focused && !state.cursor_visible() {
                    // Blinked off.
                } else if self.focused {
                    fill(r, rect, cursor);
                    let under: Option<&CellView> = snapshot.rows[row]
                        .iter()
                        .find(|(c, _)| *c == column)
                        .map(|(_, cell)| cell);
                    if let Some(cell) = under.filter(|c| !c.c.is_whitespace()) {
                        draw_text(r, cell.c.to_string(), rect.position(), color(self.palette.background), cell.bold, cell.italic, metrics, bounds);
                    }
                } else {
                    r.fill_quad(
                        Quad { bounds: rect, border: Border { color: cursor, width: 1.0, radius: 0.0.into() }, ..Quad::default() },
                        Color::TRANSPARENT,
                    );
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
        let tracking = self.terminal.input_modes().mouse != MouseTracking::Off;
        if cursor.is_over(layout.bounds()) && tree.state.downcast_ref::<State>().link.is_some() {
            mouse::Interaction::Pointer
        } else if cursor.is_over(layout.bounds()) && tracking {
            mouse::Interaction::Idle
        } else if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

impl<'a, Message: Clone + 'a> From<TerminalView<'a, Message>> for Element<'a, Message> {
    fn from(view: TerminalView<'a, Message>) -> Self {
        Element::new(view)
    }
}

fn fill(renderer: &mut iced::Renderer, bounds: Rectangle, color: Color) {
    renderer.fill_quad(Quad { bounds, ..Quad::default() }, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    renderer: &mut iced::Renderer,
    content: String,
    position: Point,
    color: Color,
    bold: bool,
    italic: bool,
    metrics: Metrics,
    clip: Rectangle,
) {
    let font = Font {
        weight: if bold { font::Weight::Bold } else { font::Weight::Normal },
        style: if italic { font::Style::Italic } else { font::Style::Normal },
        ..metrics.font
    };
    let basic = content.is_ascii() && !metrics.ligatures;
    let shaping = if basic { text::Shaping::Basic } else { text::Shaping::Advanced };
    renderer.fill_text(
        text::Text {
            content,
            bounds: Size::new(f32::INFINITY, metrics.line_height),
            size: Pixels(metrics.font_size),
            line_height: text::LineHeight::Absolute(Pixels(metrics.line_height)),
            font,
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
