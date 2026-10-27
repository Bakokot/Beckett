//! Dragging files out of the explorer: a wrapper that reports when a press
//! on it turns into a drag, and when that drag ends (anywhere). Clicks are
//! left to the wrapped element.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, Clipboard, Shell};
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, Vector};

/// How far the mouse must move with the button down to start a drag.
const THRESHOLD: f32 = 6.0;

pub struct DragSource<'a, Message> {
    child: Element<'a, Message>,
    on_start: Message,
    on_end: Message,
}

pub fn drag_source<'a, Message>(child: impl Into<Element<'a, Message>>, on_start: Message, on_end: Message) -> DragSource<'a, Message> {
    DragSource { child: child.into(), on_start, on_end }
}

#[derive(Default)]
struct State {
    pressed_at: Option<Point>,
    dragging: bool,
}

impl<'a, Message: Clone> Widget<Message, Theme, iced::Renderer> for DragSource<'a, Message> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.child)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.child));
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.child.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        self.child.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // Watched, never captured: the wrapped button still gets its clicks.
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                state.pressed_at = cursor.position_over(layout.bounds());
            }
            // Through the cursor, which a scrollable parent offsets like the press.
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some(start), Some(position)) = (state.pressed_at, cursor.position()) {
                    if !state.dragging && start.distance(position) > THRESHOLD {
                        state.dragging = true;
                        shell.publish(self.on_start.clone());
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.pressed_at = None;
                if std::mem::take(&mut state.dragging) {
                    shell.publish(self.on_end.clone());
                }
            }
            _ => {}
        }
        self.child.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.child.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<State>().dragging {
            return mouse::Interaction::Grabbing;
        }
        self.child.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.child.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: Clone + 'a> From<DragSource<'a, Message>> for Element<'a, Message> {
    fn from(source: DragSource<'a, Message>) -> Self {
        Element::new(source)
    }
}
