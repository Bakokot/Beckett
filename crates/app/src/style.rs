//! Conversions from our theme format to iced styles.

use iced::widget::container;
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
            danger: to_color(ui.danger),
        },
    )
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
