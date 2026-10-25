//! Right-click menus: a wrapper reporting right-clicks with their position,
//! and the menu drawn there over everything else.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, Clipboard, Shell};
use iced::widget::{button, column, container, row, space, text, Column};
use iced::{Background, Border, Element, Event, Length, Point, Rectangle, Shadow, Size, Theme, Vector};

use crate::style;

const MENU_WIDTH: f32 = 260.0;
const ITEM_HEIGHT: f32 = 26.0;

/// One line of a menu.
#[derive(Debug, Clone)]
pub enum Entry<M> {
    /// Label, shortcut hint, and what it does (`None`: shown disabled).
    Item(String, String, Option<M>),
    Separator,
}

impl<M> Entry<M> {
    pub fn item(label: impl Into<String>, message: M) -> Self {
        Entry::Item(label.into(), String::new(), Some(message))
    }

    pub fn with_shortcut(label: impl Into<String>, shortcut: impl Into<String>, message: Option<M>) -> Self {
        Entry::Item(label.into(), shortcut.into(), message)
    }
}

/// An open menu at a window position.
pub struct Menu<M> {
    pub position: Point,
    pub entries: Vec<Entry<M>>,
}

impl<M: Clone + 'static> Menu<M> {
    /// The menu, placed at its position but kept inside a window of `size`.
    /// `on_pick` wraps the chosen item's message (to close the menu first).
    pub fn view<'a>(&'a self, ui: &theme::UiColors, window: Size, on_pick: impl Fn(M) -> M + 'a) -> Element<'a, M> {
        let muted = style::to_color(ui.text_muted);
        let rows = self.entries.iter().map(|entry| match entry {
            Entry::Separator => {
                let border = style::to_color(ui.border);
                let line = container(space()).width(Length::Fill).height(1).style(move |_| container::Style {
                    background: Some(border.into()),
                    ..container::Style::default()
                });
                container(line).padding([4, 6]).into()
            }
            Entry::Item(label, shortcut, message) => {
                let label_color = if message.is_some() { style::to_color(ui.text) } else { muted };
                let content = row![
                    text(label.as_str()).size(13).color(label_color),
                    space::horizontal(),
                    text(shortcut.as_str()).size(12).color(muted),
                ]
                .spacing(16);
                button(content)
                    .on_press_maybe(message.clone().map(&on_pick))
                    .style(button::text)
                    .width(Length::Fill)
                    .padding([4, 10])
                    .into()
            }
        });
        let height = self.entries.iter().map(|e| if matches!(e, Entry::Separator) { 9.0 } else { ITEM_HEIGHT }).sum::<f32>() + 12.0;
        let x = self.position.x.min(window.width - MENU_WIDTH - 4.0).max(0.0);
        let y = if self.position.y + height > window.height { (self.position.y - height).max(0.0) } else { self.position.y };

        let (background, border) = (style::to_color(ui.surface), style::to_color(ui.border));
        let menu = container(Column::with_children(rows).width(Length::Fill))
            .width(MENU_WIDTH)
            .padding(6)
            .style(move |_| container::Style {
                background: Some(Background::Color(background)),
                border: Border { color: border, width: 1.0, radius: 6.0.into() },
                shadow: Shadow { color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35), offset: Vector::new(0.0, 3.0), blur_radius: 12.0 },
                ..container::Style::default()
            });
        column![container(menu).padding(iced::Padding { left: x, top: y, ..iced::Padding::ZERO })].into()
    }
}

/// Wraps an element so a right-click on it reports where it happened.
pub struct ContextArea<'a, Message> {
    child: Element<'a, Message>,
    on_right_click: Box<dyn Fn(Point) -> Message + 'a>,
}

pub fn context_area<'a, Message>(
    child: impl Into<Element<'a, Message>>,
    on_right_click: impl Fn(Point) -> Message + 'a,
) -> ContextArea<'a, Message> {
    ContextArea { child: child.into(), on_right_click: Box::new(on_right_click) }
}

impl<'a, Message> Widget<Message, Theme, iced::Renderer> for ContextArea<'a, Message> {
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
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event {
            if let Some(point) = cursor.position_over(layout.bounds()) {
                shell.publish((self.on_right_click)(point));
                shell.capture_event();
                return;
            }
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

impl<'a, Message: 'a> From<ContextArea<'a, Message>> for Element<'a, Message> {
    fn from(area: ContextArea<'a, Message>) -> Self {
        Element::new(area)
    }
}
