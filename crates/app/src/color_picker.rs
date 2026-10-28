//! A color palette to click or drag in: a saturation / brightness square for
//! the current hue, a hue bar under it, and an opacity bar.

use std::f32::consts::{FRAC_PI_2, PI};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{mouse, Clipboard, Renderer as _, Shell};
use iced::gradient::Linear;
use iced::{Background, Border, Color, Element, Event, Gradient, Length, Point, Radians, Rectangle, Size, Theme};

use crate::theme_editor::Hsv;

const SQUARE_HEIGHT: f32 = 170.0;
const BAR_HEIGHT: f32 = 14.0;
const GAP: f32 = 8.0;
const HEIGHT: f32 = SQUARE_HEIGHT + 2.0 * (GAP + BAR_HEIGHT);

pub struct ColorPicker<'a, Message> {
    hsv: Hsv,
    on_change: Box<dyn Fn(Hsv) -> Message + 'a>,
}

pub fn color_picker<'a, Message>(hsv: Hsv, on_change: impl Fn(Hsv) -> Message + 'a) -> ColorPicker<'a, Message> {
    ColorPicker { hsv, on_change: Box::new(on_change) }
}

/// The part being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Square,
    Hue,
    Alpha,
}

#[derive(Default)]
struct State {
    dragging: Option<Part>,
}

/// Where each part is drawn inside `bounds`.
struct Parts {
    square: Rectangle,
    hue: Rectangle,
    alpha: Rectangle,
}

impl Parts {
    fn new(bounds: Rectangle) -> Self {
        let square = Rectangle { height: SQUARE_HEIGHT, ..bounds };
        let hue = Rectangle { y: square.y + SQUARE_HEIGHT + GAP, height: BAR_HEIGHT, ..bounds };
        let alpha = Rectangle { y: hue.y + BAR_HEIGHT + GAP, height: BAR_HEIGHT, ..bounds };
        Self { square, hue, alpha }
    }

    fn at(&self, point: Point) -> Option<Part> {
        [(self.square, Part::Square), (self.hue, Part::Hue), (self.alpha, Part::Alpha)]
            .into_iter()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, part)| part)
    }
}

/// The color `part` gives at `point` (clamped to the part), from `hsv`.
fn pick(parts: &Parts, part: Part, point: Point, hsv: Hsv) -> Hsv {
    let fraction = |value: f32, start: f32, size: f32| ((value - start) / size.max(1.0)).clamp(0.0, 1.0);
    match part {
        Part::Square => Hsv {
            s: (fraction(point.x, parts.square.x, parts.square.width) * 100.0).round(),
            v: ((1.0 - fraction(point.y, parts.square.y, parts.square.height)) * 100.0).round(),
            ..hsv
        },
        // 359 at the far end: 360 would wrap to red at the start.
        Part::Hue => Hsv { h: (fraction(point.x, parts.hue.x, parts.hue.width) * 359.0).round(), ..hsv },
        Part::Alpha => Hsv { a: (fraction(point.x, parts.alpha.x, parts.alpha.width) * 255.0).round(), ..hsv },
    }
}

fn to_iced(color: theme::Color) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, f32::from(color.a) / 255.0)
}

impl<'a, Message> Widget<Message, Theme, iced::Renderer> for ColorPicker<'a, Message> {
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
        let parts = Parts::new(layout.bounds());
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some((point, part)) = cursor.position().and_then(|p| parts.at(p).map(|part| (p, part))) else {
                    return;
                };
                state.dragging = Some(part);
                shell.publish((self.on_change)(pick(&parts, part, point, self.hsv)));
                shell.capture_event();
            }
            // Dragging keeps following the mouse, even outside the palette.
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(part) = state.dragging {
                    shell.publish((self.on_change)(pick(&parts, part, *position, self.hsv)));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => state.dragging = None,
            _ => {}
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let parts = Parts::new(layout.bounds());
        let hsv = self.hsv;
        let quad = |bounds: Rectangle, radius: f32| Quad { bounds, border: Border { radius: radius.into(), ..Border::default() }, ..Quad::default() };
        let gradient = |angle: f32, from: Color, to: Color| Background::Gradient(Gradient::Linear(Linear::new(Radians(angle)).add_stop(0.0, from).add_stop(1.0, to)));
        let left_to_right = FRAC_PI_2;
        let top_to_bottom = PI;

        // Saturation (left to right) and brightness (top to bottom) of the hue.
        let pure = to_iced(Hsv { s: 100.0, v: 100.0, a: 255.0, ..hsv }.to_color());
        renderer.fill_quad(quad(parts.square, 4.0), pure);
        renderer.fill_quad(quad(parts.square, 4.0), gradient(left_to_right, Color::WHITE, Color::WHITE.scale_alpha(0.0)));
        renderer.fill_quad(quad(parts.square, 4.0), gradient(top_to_bottom, Color::BLACK.scale_alpha(0.0), Color::BLACK));

        // Hue: the color wheel unrolled.
        let mut hues = Linear::new(Radians(left_to_right));
        for step in 0..=6 {
            let color = Hsv { h: step as f32 * 60.0 % 360.0, s: 100.0, v: 100.0, a: 255.0 }.to_color();
            hues = hues.add_stop(step as f32 / 6.0, to_iced(color));
        }
        renderer.fill_quad(quad(parts.hue, 3.0), Background::Gradient(Gradient::Linear(hues)));

        // Opacity over a checkerboard, so transparency shows.
        let cell = BAR_HEIGHT / 2.0;
        renderer.fill_quad(quad(parts.alpha, 3.0), Color::from_rgb8(0xee, 0xee, 0xee));
        let columns = (parts.alpha.width / cell).ceil() as usize;
        for column in 0..columns {
            for row in 0..2 {
                if (column + row) % 2 == 0 {
                    let x = parts.alpha.x + column as f32 * cell;
                    let width = cell.min(parts.alpha.x + parts.alpha.width - x);
                    renderer.fill_quad(quad(Rectangle::new(Point::new(x, parts.alpha.y + row as f32 * cell), Size::new(width, cell)), 0.0), Color::from_rgb8(0xbb, 0xbb, 0xbb));
                }
            }
        }
        let opaque = to_iced(Hsv { a: 255.0, ..hsv }.to_color());
        renderer.fill_quad(quad(parts.alpha, 3.0), gradient(left_to_right, opaque.scale_alpha(0.0), opaque));

        // Markers: a ring on the square, a handle on each bar.
        let ring = |center: Point, color: Color| Quad {
            bounds: Rectangle::new(Point::new(center.x - 6.0, center.y - 6.0), Size::new(12.0, 12.0)),
            border: Border { color, width: 2.0, radius: 6.0.into() },
            ..Quad::default()
        };
        let square_point = Point::new(
            parts.square.x + hsv.s / 100.0 * parts.square.width,
            parts.square.y + (1.0 - hsv.v / 100.0) * parts.square.height,
        );
        // Dark ring on light colors, light ring on dark ones.
        let ring_color = if hsv.v > 60.0 && hsv.s < 40.0 { Color::BLACK } else { Color::WHITE };
        renderer.fill_quad(ring(square_point, ring_color), Color::TRANSPARENT);
        let handle = |bar: Rectangle, fraction: f32| Quad {
            bounds: Rectangle::new(Point::new(bar.x + fraction * bar.width - 3.0, bar.y - 2.0), Size::new(6.0, bar.height + 4.0)),
            border: Border { color: Color::BLACK, width: 1.0, radius: 2.0.into() },
            ..Quad::default()
        };
        renderer.fill_quad(handle(parts.hue, hsv.h / 359.0), Color::WHITE);
        renderer.fill_quad(handle(parts.alpha, hsv.a / 255.0), Color::WHITE);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let dragging = tree.state.downcast_ref::<State>().dragging.is_some();
        if dragging || cursor.position().is_some_and(|p| Parts::new(layout.bounds()).at(p).is_some()) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::None
        }
    }
}

impl<'a, Message: 'a> From<ColorPicker<'a, Message>> for Element<'a, Message> {
    fn from(picker: ColorPicker<'a, Message>) -> Self {
        Element::new(picker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_map_to_hsv() {
        let parts = Parts::new(Rectangle::new(Point::new(10.0, 20.0), Size::new(200.0, HEIGHT)));
        let start = Hsv { h: 200.0, s: 50.0, v: 50.0, a: 255.0 };
        // Top right of the square: full saturation and brightness, same hue.
        let top_right = pick(&parts, Part::Square, Point::new(210.0, 20.0), start);
        assert_eq!((top_right.h, top_right.s, top_right.v), (200.0, 100.0, 100.0));
        // Bottom left, and beyond it (clamped while dragging outside).
        let bottom_left = pick(&parts, Part::Square, Point::new(0.0, 500.0), start);
        assert_eq!((bottom_left.s, bottom_left.v), (0.0, 0.0));
        // Middle of the hue bar: cyan-ish; start of the opacity bar: clear.
        assert_eq!(pick(&parts, Part::Hue, Point::new(110.0, parts.hue.y + 5.0), start).h, 180.0);
        assert_eq!(pick(&parts, Part::Alpha, Point::new(10.0, parts.alpha.y + 5.0), start).a, 0.0);
        assert_eq!(parts.at(Point::new(50.0, parts.hue.y + 2.0)), Some(Part::Hue));
        assert_eq!(parts.at(Point::new(50.0, parts.square.y + SQUARE_HEIGHT + 2.0)), None);
    }
}
