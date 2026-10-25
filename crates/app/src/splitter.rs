//! A thin horizontal bar that resizes the panel below it: dragging it
//! reports how far the mouse moved vertically.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{mouse, Clipboard, Renderer as _, Shell};
use iced::{Color, Element, Event, Length, Rectangle, Size, Theme};

/// Height of the grab area; only a 1px line is drawn in it.
const HEIGHT: f32 = 5.0;

pub struct Splitter<'a, Message> {
    on_drag: Box<dyn Fn(f32) -> Message + 'a>,
    line: Color,
    active: Color,
}

/// `on_drag` gets the vertical movement in pixels (negative is up).
pub fn splitter<'a, Message>(line: Color, active: Color, on_drag: impl Fn(f32) -> Message + 'a) -> Splitter<'a, Message> {
    Splitter { on_drag: Box::new(on_drag), line, active }
}

#[derive(Default)]
struct State {
    /// Mouse y where the last reported drag step ended.
    dragging_from: Option<f32>,
}

impl<'a, Message> Widget<Message, Theme, iced::Renderer> for Splitter<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(HEIGHT))
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.resolve(Length::Fill, Length::Fixed(HEIGHT), Size::ZERO))
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
                    state.dragging_from = Some(point.y);
                    shell.capture_event();
                }
            }
            // Followed anywhere in the window, not just over the bar.
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(from) = state.dragging_from {
                    let delta = position.y - from;
                    if delta != 0.0 {
                        state.dragging_from = Some(position.y);
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
        let (color, height) = if hot { (self.active, 2.0) } else { (self.line, 1.0) };
        let line = Rectangle { y: bounds.y + (HEIGHT - height) / 2.0, height, ..bounds };
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
        if dragging || cursor.is_over(layout.bounds()) {
            mouse::Interaction::ResizingVertically
        } else {
            mouse::Interaction::None
        }
    }
}

impl<'a, Message: 'a> From<Splitter<'a, Message>> for Element<'a, Message> {
    fn from(splitter: Splitter<'a, Message>) -> Self {
        Element::new(splitter)
    }
}
