//! Conversions from our theme format to iced styles.

use iced::widget::{container, text, tooltip};
use iced::{Background, Border, Color};

pub fn to_color(c: theme::Color) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, f32::from(c.a) / 255.0)
}

pub fn iced_theme(theme: &theme::Theme) -> iced::Theme {
    let ui = &theme.ui;
    iced::Theme::custom(
        theme.name.clone(),
        iced::theme::Palette {
            background: to_color(ui.background),
            text: to_color(ui.text),
            primary: to_color(ui.accent),
            success: to_color(ui.success),
            warning: to_color(ui.warning),
            danger: to_color(ui.danger),
        },
    )
}

/// How long the mouse must rest on a control before its tooltip shows.
const TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// Wraps `content` with a tooltip that appears after a short hover.
pub fn tip<'a, Message: 'a>(
    content: impl Into<iced::Element<'a, Message>>,
    label: impl Into<String>,
    position: tooltip::Position,
    ui: &theme::UiColors,
) -> iced::Element<'a, Message> {
    let (background, border, color) = (to_color(ui.surface), to_color(ui.border), to_color(ui.text));
    let label = container(text(label.into()).size(12).color(color))
        .padding([4, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(background)),
            border: Border { color: border, width: 1.0, radius: 4.0.into() },
            ..container::Style::default()
        });
    tooltip(content, label, position).gap(4).delay(TOOLTIP_DELAY).into()
}

/// The outline around one editor tab: the border color, or the accent for
/// the tab shown.
pub fn tab_outline(ui: &theme::UiColors, selected: bool) -> impl Fn(&iced::Theme) -> container::Style {
    let color = to_color(if selected { ui.accent } else { ui.border });
    move |_| container::Style {
        border: Border { color, width: 1.0, radius: 4.0.into() },
        ..container::Style::default()
    }
}

/// A panel filled with the theme's surface color (sidebar, bars).
pub fn surface(ui: &theme::UiColors) -> impl Fn(&iced::Theme) -> container::Style {
    let background = to_color(ui.surface);
    let border = to_color(ui.border);
    move |_| container::Style {
        background: Some(Background::Color(background)),
        border: Border {
            color: border,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}
