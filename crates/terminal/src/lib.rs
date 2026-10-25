//! Integrated terminal: a shell on a pseudo-terminal (ConPTY on Windows),
//! parsed by `alacritty_terminal`, exposed as a cell grid snapshot.

mod keys;
mod mouse;

pub use keys::{encode_key, Key, Mods};
pub use mouse::{encode_mouse, MouseAction, MouseButton, MouseTracking};

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb as AnsiRgb};

/// An sRGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    fn dim(self) -> Self {
        let f = |c: u8| (u16::from(c) * 2 / 3) as u8;
        Self::new(f(self.r), f(self.g), f(self.b))
    }
}

/// Colors used to resolve the terminal's palette references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Black, red, green, yellow, blue, magenta, cyan, white, then bright.
    pub ansi: [Rgb; 16],
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// New output; redraw.
    Wakeup,
    Title(String),
    /// The shell exited.
    Exit,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub working_directory: Option<PathBuf>,
    /// Program and arguments; the platform default shell when `None`.
    pub shell: Option<(String, Vec<String>)>,
    pub env: HashMap<String, String>,
}

/// One rendered cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellView {
    pub c: char,
    pub fg: Rgb,
    /// `None` for the default background.
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Occupies two columns.
    pub wide: bool,
    pub selected: bool,
}

/// The visible screen at one instant.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub columns: usize,
    /// Visible rows, top to bottom; wide-char spacer cells are omitted.
    pub rows: Vec<Vec<(usize, CellView)>>,
    /// Row and column of the cursor, when shown.
    pub cursor: Option<(usize, usize)>,
    /// Lines scrolled back from the bottom.
    pub display_offset: usize,
}

/// Terminal-side modes that change how input is encoded.
#[derive(Debug, Clone, Copy, Default)]
pub struct InputModes {
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub mouse: MouseTracking,
    /// Report mouse events with the SGR encoding (DECSET 1006).
    pub sgr_mouse: bool,
    /// A full-screen app is showing (`less`, `vim`…): no scrollback.
    pub alt_screen: bool,
    /// With `alt_screen`, the wheel should send arrow keys (DECSET 1007).
    pub alternate_scroll: bool,
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    size: Mutex<(usize, usize)>,
}

impl fmt::Debug for Terminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Terminal").field("size", &self.size).finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct Listener {
    notify: Arc<dyn Fn(Event) + Send + Sync>,
    /// Set once the event loop exists; used to answer terminal queries.
    sender: Arc<Mutex<Option<EventLoopSender>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::Wakeup => (self.notify)(Event::Wakeup),
            TermEvent::Title(title) => (self.notify)(Event::Title(title)),
            TermEvent::Exit | TermEvent::ChildExit(_) => (self.notify)(Event::Exit),
            // Replies to queries such as cursor position reports.
            TermEvent::PtyWrite(text) => {
                let sender = self.sender.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(sender) = sender.as_ref() {
                    let _ = sender.send(Msg::Input(Cow::Owned(text.into_bytes())));
                }
            }
            _ => {}
        }
    }
}

struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

fn window_size(columns: usize, lines: usize, cell: (u16, u16)) -> WindowSize {
    WindowSize {
        num_lines: lines as u16,
        num_cols: columns as u16,
        cell_width: cell.0,
        cell_height: cell.1,
    }
}

impl Terminal {
    /// Starts the shell. `notify` is called from the terminal's I/O thread.
    pub fn spawn(
        options: Options,
        columns: usize,
        lines: usize,
        notify: impl Fn(Event) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = Listener { notify: Arc::new(notify), sender: Arc::new(Mutex::new(None)) };
        let config = term::Config::default();
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &Size { columns, lines },
            listener.clone(),
        )));

        let pty_options = tty::Options {
            shell: options.shell.map(|(program, args)| tty::Shell::new(program, args)),
            working_directory: options.working_directory,
            drain_on_exit: false,
            env: options.env,
            #[cfg(windows)]
            escape_args: false,
        };
        let pty = tty::new(&pty_options, window_size(columns, lines, (8, 16)), 0)?;
        let event_loop = EventLoop::new(Arc::clone(&term), listener.clone(), pty, false, false)?;
        let sender = event_loop.channel();
        *listener.sender.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender.clone());
        event_loop.spawn();

        Ok(Self { term, sender, size: Mutex::new((columns, lines)) })
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    /// Sends pasted text, wrapped for bracketed paste when the shell wants it.
    pub fn paste(&self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.input_modes().bracketed_paste {
            self.write(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.write(text.into_bytes());
        }
        self.scroll_to_bottom();
    }

    pub fn size(&self) -> (usize, usize) {
        *self.size.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Resizes the grid and the pty, if the size changed.
    pub fn resize(&self, columns: usize, lines: usize, cell: (u16, u16)) {
        let (columns, lines) = (columns.max(2), lines.max(1));
        let mut size = self.size.lock().unwrap_or_else(PoisonError::into_inner);
        if *size == (columns, lines) {
            return;
        }
        *size = (columns, lines);
        self.term.lock().resize(Size { columns, lines });
        let _ = self.sender.send(Msg::Resize(window_size(columns, lines, cell)));
    }

    /// Scrolls through history; positive is up.
    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    pub fn input_modes(&self) -> InputModes {
        let mode = *self.term.lock().mode();
        let mouse = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseTracking::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseTracking::Drags
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseTracking::Clicks
        } else {
            MouseTracking::Off
        };
        InputModes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse,
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        }
    }

    /// Starts a selection at a visible cell (`row` counted from the top).
    pub fn start_selection(&self, row: usize, column: usize, kind: SelectionKind) {
        let mut term = self.term.lock();
        let point = visible_point(&term, row, column);
        let kind = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        term.selection = Some(Selection::new(kind, point, Side::Left));
    }

    pub fn extend_selection(&self, row: usize, column: usize) {
        let mut term = self.term.lock();
        let point = visible_point(&term, row, column);
        if let Some(selection) = &mut term.selection {
            selection.update(point, Side::Right);
        }
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    pub fn snapshot(&self, palette: &Palette) -> Snapshot {
        let term = self.term.lock();
        let content = term.renderable_content();
        let columns = term.columns();
        let screen_lines = term.screen_lines();
        let offset = content.display_offset as i32;
        let overrides = content.colors;
        let resolve = |color: Color| resolve_color(color, palette, |i| overrides[i]);

        let mut rows: Vec<Vec<(usize, CellView)>> = vec![Vec::new(); screen_lines];
        let selection = content.selection;
        for indexed in content.display_iter {
            let row = (indexed.point.line.0 + offset) as usize;
            if row >= screen_lines {
                continue;
            }
            let cell = &indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let mut fg = cell.fg;
            // Bold text in a base color is shown in its bright variant.
            if cell.flags.contains(Flags::BOLD) {
                if let Color::Named(named) = fg {
                    if (named as usize) < 8 {
                        fg = Color::Indexed(named as u8 + 8);
                    }
                }
            }
            let mut fg = resolve(fg);
            let mut bg = match cell.bg {
                Color::Named(NamedColor::Background) => None,
                other => Some(resolve(other)),
            };
            if cell.flags.contains(Flags::DIM) {
                fg = fg.dim();
            }
            if cell.flags.contains(Flags::INVERSE) {
                let old_bg = bg.unwrap_or(palette.background);
                bg = Some(fg);
                fg = old_bg;
            }
            if cell.flags.contains(Flags::HIDDEN) {
                fg = bg.unwrap_or(palette.background);
            }
            let selected = selection.is_some_and(|s| s.contains(indexed.point));
            rows[row].push((
                indexed.point.column.0,
                CellView {
                    c: cell.c,
                    fg,
                    bg,
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                    underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                    wide: cell.flags.contains(Flags::WIDE_CHAR),
                    selected,
                },
            ));
        }

        let cursor_point = content.cursor.point;
        let cursor_row = cursor_point.line.0 + offset;
        let cursor = (content.mode.contains(TermMode::SHOW_CURSOR)
            && (0..screen_lines as i32).contains(&cursor_row))
        .then_some((cursor_row as usize, cursor_point.column.0));

        Snapshot { columns, rows, cursor, display_offset: content.display_offset }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    Simple,
    Word,
    Line,
}

fn visible_point<T>(term: &Term<T>, row: usize, column: usize) -> Point {
    let offset = term.grid().display_offset() as i32;
    let row = row.min(term.screen_lines().saturating_sub(1)) as i32;
    let column = column.min(term.columns().saturating_sub(1));
    Point::new(Line(row - offset), Column(column))
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

/// Resolves a cell color: OSC overrides first, then the palette, then the
/// standard 256-color cube and grayscale ramp.
fn resolve_color(color: Color, palette: &Palette, overrides: impl Fn(usize) -> Option<AnsiRgb>) -> Rgb {
    let from_ansi = |rgb: AnsiRgb| Rgb::new(rgb.r, rgb.g, rgb.b);
    match color {
        Color::Spec(rgb) => from_ansi(rgb),
        Color::Indexed(index) => overrides(index as usize)
            .map(from_ansi)
            .unwrap_or_else(|| indexed_color(index, palette)),
        Color::Named(named) => {
            if let Some(rgb) = overrides(named as usize) {
                return from_ansi(rgb);
            }
            let index = named as usize;
            match named {
                NamedColor::Foreground | NamedColor::BrightForeground => palette.foreground,
                NamedColor::Background => palette.background,
                NamedColor::Cursor => palette.cursor,
                NamedColor::DimForeground => palette.foreground.dim(),
                _ if index < 16 => palette.ansi[index],
                // DimBlack..DimWhite follow the named colors in order.
                _ => {
                    let dim = index.saturating_sub(NamedColor::DimBlack as usize);
                    palette.ansi.get(dim).copied().unwrap_or(palette.foreground).dim()
                }
            }
        }
    }
}

fn indexed_color(index: u8, palette: &Palette) -> Rgb {
    match index {
        0..=15 => palette.ansi[index as usize],
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Rgb::new(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            Rgb::new(v, v, v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        let mut ansi = [Rgb::new(0, 0, 0); 16];
        ansi[1] = Rgb::new(200, 0, 0);
        ansi[9] = Rgb::new(255, 80, 80);
        Palette {
            ansi,
            foreground: Rgb::new(220, 220, 220),
            background: Rgb::new(20, 20, 20),
            cursor: Rgb::new(255, 255, 255),
        }
    }

    #[test]
    fn resolves_palette_cube_and_overrides() {
        let p = palette();
        let none = |_| None;
        assert_eq!(resolve_color(Color::Named(NamedColor::Red), &p, none), Rgb::new(200, 0, 0));
        assert_eq!(resolve_color(Color::Named(NamedColor::Foreground), &p, none), p.foreground);
        assert_eq!(resolve_color(Color::Indexed(9), &p, none), Rgb::new(255, 80, 80));
        assert_eq!(resolve_color(Color::Indexed(196), &p, none), Rgb::new(255, 0, 0));
        assert_eq!(resolve_color(Color::Indexed(232), &p, none), Rgb::new(8, 8, 8));
        let spec = Color::Spec(AnsiRgb { r: 1, g: 2, b: 3 });
        assert_eq!(resolve_color(spec, &p, none), Rgb::new(1, 2, 3));
        let red_override = |i| (i == 1).then_some(AnsiRgb { r: 9, g: 9, b: 9 });
        assert_eq!(resolve_color(Color::Named(NamedColor::Red), &p, red_override), Rgb::new(9, 9, 9));
    }
}
