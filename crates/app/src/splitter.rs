//! A thin bar that resizes what is beside it: dragging it reports how far
//! the mouse moved across the bar (vertically for a horizontal bar, such as
//! the one above the bottom panel; horizontally for the one between the two
//! sides of the split editor).

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{mouse, Clipboard, Renderer as _, Shell};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Theme};

/// Thickness of the grab area; only a 1px line is drawn in it.
pub const THICKNESS: f32 = 5.0;

pub struct Splitter<'a, Message> {
    on_drag: Box<dyn Fn(f32) -> Message + 'a>,
    line: Color,
    active: Color,
    /// A vertical bar, dragged left and right.
    vertical: bool,
    /// Sent on a double-click (e.g. back to the default size).
    on_reset: Option<Message>,
}

/// A horizontal bar: `on_drag` gets the vertical movement in pixels
/// (negative is up).
pub fn splitter<'a, Message>(line: Color, active: Color, on_drag: impl Fn(f32) -> Message + 'a) -> Splitter<'a, Message> {
    Splitter { on_drag: Box::new(on_drag), line, active, vertical: false, on_reset: None }
}

/// A vertical bar: `on_drag` gets the horizontal movement in pixels
/// (negative is left).
pub fn vertical<'a, Message>(line: Color, active: Color, on_drag: impl Fn(f32) -> Message + 'a) -> Splitter<'a, Message> {
    Splitter { on_drag: Box::new(on_drag), line, active, vertical: true, on_reset: None }
}

#[derive(Default)]
struct State {
    /// Mouse position (across the bar) where the last reported drag step ended.
    dragging_from: Option<f32>,
    last_click: Option<mouse::Click>,
}

impl<Message> Splitter<'_, Message> {
    /// What a double-click on the bar sends.
    pub fn on_reset(mut self, message: Message) -> Self {
        self.on_reset = Some(message);
        self
    }

    fn size(&self) -> Size<Length> {
        if self.vertical {
            Size::new(Length::Fixed(THICKNESS), Length::Fill)
        } else {
            Size::new(Length::Fill, Length::Fixed(THICKNESS))
        }
    }

    /// The coordinate the bar moves along.
    fn across(&self, point: Point) -> f32 {
        if self.vertical { point.x } else { point.y }
    }
}

impl<'a, Message: Clone> Widget<Message, Theme, iced::Renderer> for Splitter<'a, Message> {
    fn size(&self) -> Size<Length> {
        Splitter::size(self)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let size = Splitter::size(self);
        layout::Node::new(limits.resolve(size.width, size.height, Size::ZERO))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(point) = cursor.position_over(layout.bounds()) {
                    let click = mouse::Click::new(point, mouse::Button::Left, state.last_click);
                    state.last_click = Some(click);
                    if let (mouse::click::Kind::Double, Some(reset)) = (click.kind(), &self.on_reset) {
                        shell.publish(reset.clone());
                        state.dragging_from = None;
                        shell.capture_event();
                        return;
                    }
                    state.dragging_from = Some(self.across(point));
                    shell.capture_event();
                }
            }
            // Followed anywhere in the window, not just over the bar.
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(from) = state.dragging_from {
                    let delta = self.across(*position) - from;
                    if delta != 0.0 {
                        state.dragging_from = Some(self.across(*position));
                        shell.publish((self.on_drag)(delta));
                    }
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging_from = None;
                shell.request_redraw();
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
        let bounds = layout.bounds();
        let hot = state.dragging_from.is_some() || cursor.is_over(bounds);
        let (color, thickness) = if hot { (self.active, 2.0) } else { (self.line, 1.0) };
        let offset = (THICKNESS - thickness) / 2.0;
        let line = if self.vertical {
            Rectangle { x: bounds.x + offset, width: thickness, ..bounds }
        } else {
            Rectangle { y: bounds.y + offset, height: thickness, ..bounds }
        };
        renderer.fill_quad(Quad { bounds: line, ..Quad::default() }, color);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let dragging = tree.state.downcast_ref::<State>().dragging_from.is_some();
        match dragging || cursor.is_over(layout.bounds()) {
            false => mouse::Interaction::None,
            true if self.vertical => mouse::Interaction::ResizingHorizontally,
            true => mouse::Interaction::ResizingVertically,
        }
    }
}

impl<'a, Message: Clone + 'a> From<Splitter<'a, Message>> for Element<'a, Message> {
    fn from(splitter: Splitter<'a, Message>) -> Self {
        Element::new(splitter)
    }
}
