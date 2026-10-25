//! The rename bar (F2): asks for the new name of the symbol under the
//! cursor; the language server computes the edits.

use std::path::PathBuf;

use editor_core::Pos;
use iced::widget::tooltip::Position;
use iced::widget::{button, container, row, space, text, text_input};
use iced::{Alignment, Element, Length};

use crate::style;

pub const INPUT: &str = "rename-input";

#[derive(Debug, Clone)]
pub enum RenameMessage {
    Input(String),
    Submit,
    Close,
}

pub struct Rename {
    pub path: PathBuf,
    pub pos: Pos,
    /// The symbol's current name.
    pub original: String,
    pub name: String,
}

impl Rename {
    pub fn new(path: PathBuf, pos: Pos, original: String) -> Self {
        Self { path, pos, name: original.clone(), original }
    }

    /// The new name, if it is a valid identifier different from the old one.
    pub fn valid_name(&self) -> Option<&str> {
        let name = self.name.trim();
        let mut chars = name.chars();
        let starts_ok = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_');
        (starts_ok && chars.all(|c| c.is_alphanumeric() || c == '_') && name != self.original).then_some(name)
    }

    pub fn view<'a>(&'a self, ui: &theme::UiColors) -> Element<'a, RenameMessage> {
        let muted = style::to_color(ui.text_muted);
        let hint = if self.valid_name().is_some() || self.name.trim() == self.original {
            text("Enter to rename in every file, Escape to cancel").size(12).color(muted)
        } else {
            text("Not a valid name").size(12).color(style::to_color(ui.danger))
        };
        let close = button(text("x").size(12)).on_press(RenameMessage::Close).style(button::text).padding([2, 6]);
        let content = row![
            text(format!("Rename {}", self.original)).size(12),
            text_input("New name", &self.name)
                .id(INPUT)
                .on_input(RenameMessage::Input)
                .on_submit(RenameMessage::Submit)
                .size(13)
                .padding([3, 6])
                .width(280),
            hint,
            space::horizontal(),
            style::tip(close, "Cancel (Escape)", Position::Bottom, ui),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        container(content).padding([4, 8]).width(Length::Fill).style(style::surface(ui)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        let mut rename = Rename::new(PathBuf::new(), Pos { line: 0, column: 0 }, "speed".to_string());
        assert_eq!(rename.valid_name(), None);
        rename.name = " max_speed ".to_string();
        assert_eq!(rename.valid_name(), Some("max_speed"));
        for bad in ["", "2fast", "max-speed", "a b"] {
            rename.name = bad.to_string();
            assert_eq!(rename.valid_name(), None, "{bad:?}");
        }
    }
}
