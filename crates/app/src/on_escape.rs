//! Wraps an element so Escape produces a message before the child sees it
//! (iced's text input swallows Escape, which should close the find bar).

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, Clipboard, Shell};
use iced::keyboard::{self, key::Named};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector};

pub struct OnEscape<'a, Message> {
    child: Element<'a, Message>,
    message: Message,
}

pub fn on_escape<'a, Message>(child: impl Into<Element<'a, Message>>, message: Message) -> OnEscape<'a, Message> {
    OnEscape { child: child.into(), message }
}

impl<'a, Message: Clone> Widget<Message, Theme, iced::Renderer> for OnEscape<'a, Message> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::stateless()
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

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
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
        if let Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(Named::Escape), .. }) = event {
            shell.publish(self.message.clone());
            shell.capture_event();
            return;
        }
        self.child
            .as_widget_mut()
            .update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
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
        self.child
            .as_widget()
            .draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.child
            .as_widget()
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.child
            .as_widget_mut()
            .overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: Clone + 'a> From<OnEscape<'a, Message>> for Element<'a, Message> {
    fn from(widget: OnEscape<'a, Message>) -> Self {
        Element::new(widget)
    }
}
