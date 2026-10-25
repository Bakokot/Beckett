//! Makes the numeric keypad type digits again.
//!
//! winit reports a numpad key's `key` as if NumLock were off (Numpad1 is
//! End, Numpad4 is Left, Numpad. is Delete…), so widgets that look at `key`
//! move the cursor instead of typing. `modified_key` follows NumLock; for
//! numpad keys it replaces `key` before any widget sees the event.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, Clipboard, Shell};
use iced::keyboard::{self, Location};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector};

/// The event with the NumLock-aware key for numpad keys.
pub fn fix_key_event(event: keyboard::Event) -> keyboard::Event {
    match event {
        keyboard::Event::KeyPressed { modified_key, physical_key, modifiers, location: Location::Numpad, text, repeat, .. } => {
            keyboard::Event::KeyPressed {
                key: modified_key.clone(),
                modified_key,
                physical_key,
                modifiers,
                location: Location::Numpad,
                text,
                repeat,
            }
        }
        keyboard::Event::KeyReleased { modified_key, physical_key, modifiers, location: Location::Numpad, .. } => {
            keyboard::Event::KeyReleased { key: modified_key.clone(), modified_key, physical_key, modifiers, location: Location::Numpad }
        }
        other => other,
    }
}

/// Wraps the whole interface so every widget gets fixed numpad events.
pub struct NumpadFix<'a, Message> {
    child: Element<'a, Message>,
}

pub fn numpad_fix<'a, Message>(child: impl Into<Element<'a, Message>>) -> NumpadFix<'a, Message> {
    NumpadFix { child: child.into() }
}

impl<'a, Message> Widget<Message, Theme, iced::Renderer> for NumpadFix<'a, Message> {
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
        let fixed;
        let event = match event {
            Event::Keyboard(key_event) => {
                fixed = Event::Keyboard(fix_key_event(key_event.clone()));
                &fixed
            }
            other => other,
        };
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

impl<'a, Message: 'a> From<NumpadFix<'a, Message>> for Element<'a, Message> {
    fn from(widget: NumpadFix<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::{Key, Modifiers};

    fn press(key: Key, modified_key: Key, location: Location) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key: Physical::Code(Code::Numpad1),
            modifiers: Modifiers::default(),
            location,
            text: Some("1".into()),
            repeat: false,
        }
    }

    #[test]
    fn numpad_keys_follow_numlock() {
        // NumLock on: winit says End, the NumLock-aware key says "1".
        let fixed = fix_key_event(press(Key::Named(Named::End), Key::Character("1".into()), Location::Numpad));
        assert!(matches!(fixed, keyboard::Event::KeyPressed { key: Key::Character(ref c), .. } if c == "1"));
        // NumLock off: both say End, and End it stays.
        let fixed = fix_key_event(press(Key::Named(Named::End), Key::Named(Named::End), Location::Numpad));
        assert!(matches!(fixed, keyboard::Event::KeyPressed { key: Key::Named(Named::End), .. }));
        // Keys elsewhere on the keyboard are untouched.
        let fixed = fix_key_event(press(Key::Named(Named::End), Key::Character("x".into()), Location::Standard));
        assert!(matches!(fixed, keyboard::Event::KeyPressed { key: Key::Named(Named::End), .. }));
    }
}
